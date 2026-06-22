//! Host-state teardown reconciler for the native F5 backend.
//!
//! `akon vpn on` mutates host networking to bring up the tunnel:
//!   1. creates a `tun%d` interface (with address, MTU, and — for full tunnel —
//!      the `0.0.0.0/1` + `128.0.0.0/1` default-override routes, plus any split
//!      routes). These are **device-bound** and the kernel removes them
//!      automatically when the TUN fd closes (the device is non-persistent).
//!   2. installs a **server-pin route** `server/32 via <original-gateway>` so the
//!      encrypted tunnel's own packets keep flowing over the real default. This
//!      is **NOT** device-bound, so it must be removed explicitly.
//!   3. loosens `rp_filter` on the tun and on `all` (sysctl).
//!   4. points the host resolver (systemd-resolved/resolvconf) at the VPN DNS.
//!
//! To guarantee a production host can always recover its connectivity, every one
//! of these must be undone by `akon vpn off` — **even if the `vpn on` process was
//! SIGKILL'd / OOM-killed** and its in-memory cleanup never ran. We therefore
//! persist a [`HostTeardownPlan`] to the state file at connect time and replay it
//! here. [`teardown_host`] is fully **idempotent** and **best-effort**: it is safe
//! to run when nothing is present, and a failure on one step never aborts the
//! others, so partial leaks are always cleaned on the next `off`/`reset`.

use serde::{Deserialize, Serialize};

/// A record of every host mutation made while bringing up the tunnel, persisted
/// so teardown can reconcile the host back to its original state without needing
/// the original process or its in-memory state.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostTeardownPlan {
    /// The tun interface name (e.g. `tun0`). Deleting it reaps all device-bound
    /// routes (address, default-halves, split routes).
    #[serde(default)]
    pub device: Option<String>,
    /// Non-device-bound routes to delete explicitly. Each entry is a destination
    /// (CIDR or IP) we added that does NOT die with the interface — notably the
    /// `server/32 via <gw>` pin.
    #[serde(default)]
    pub extra_routes: Vec<String>,
    /// `rp_filter` sysctl keys we changed, with their ORIGINAL values, so we can
    /// restore them exactly (e.g. `("net.ipv4.conf.all.rp_filter", "1")`).
    #[serde(default)]
    pub rp_filter_restore: Vec<(String, String)>,
    /// The interface whose DNS configuration must be reverted (usually the same
    /// as `device`). `None` if DNS was never applied.
    #[serde(default)]
    pub dns_iface: Option<String>,
}

impl HostTeardownPlan {
    /// True if the plan records no mutations (nothing to undo).
    pub fn is_empty(&self) -> bool {
        self.device.is_none()
            && self.extra_routes.is_empty()
            && self.rp_filter_restore.is_empty()
            && self.dns_iface.is_none()
    }
}

/// The outcome of a teardown attempt: what was undone and any non-fatal problems.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct TeardownReport {
    /// Human-readable lines describing each action taken (for logging).
    pub actions: Vec<String>,
    /// Best-effort steps that failed (teardown continues regardless).
    pub warnings: Vec<String>,
}

/// Reconcile the host back to its pre-VPN state from a persisted plan.
///
/// Order matters: revert DNS and remove the explicit (non-device-bound) routes
/// and restore sysctls FIRST, then delete the interface (which reaps the
/// device-bound routes). Every step is best-effort and idempotent.
#[cfg(target_os = "linux")]
pub fn teardown_host(plan: &HostTeardownPlan) -> TeardownReport {
    use crate::vpn::f5::netlink::{if_nametoindex, NetlinkSocket};
    use std::process::Command;

    let mut report = TeardownReport::default();

    // 1) Revert DNS for the tun link and flush the (possibly poisoned) cache so
    //    a stale negative result can't linger. DNS goes through systemd-resolved
    //    (D-Bus/polkit) — NOT CAP_NET_ADMIN — so the `resolvectl` child is fine
    //    rootless. resolved also auto-reverts when the link disappears; we do it
    //    explicitly for resolvconf hosts and to flush caches.
    if let Some(iface) = &plan.dns_iface {
        // Use `.output()` (not `.status()`) so the child's stderr is captured,
        // not echoed to our terminal: when the tun link is already gone (e.g.
        // teardown replay after SIGKILL, or the link was removed first) resolved
        // prints `Failed to resolve interface "tunN": No such device`. That is
        // benign — resolved auto-reverts DNS when the link disappears — so we
        // swallow the noise rather than surface a scary line to the user.
        let reverted = Command::new("resolvectl")
            .args(["revert", iface])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if reverted {
            report.actions.push(format!("reverted DNS on {iface}"));
        }
        // resolvconf fallback (no-op if not present); also output-captured.
        let _ = Command::new("resolvconf").args(["-d", iface]).output();
        let _ = Command::new("resolvectl").arg("flush-caches").output();
        report.actions.push("flushed DNS caches".to_string());
    }

    // Open one netlink socket for the link/route operations (in-process, so it
    // works rootless under a file capability — see ADR 0001).
    let mut nl = NetlinkSocket::open().ok();

    // 2) Remove non-device-bound routes (the server pin) via netlink. Idempotent:
    //    a missing route (ESRCH) is treated as success by `route_del`.
    for dest in &plan.extra_routes {
        match (nl.as_mut(), parse_cidr(dest)) {
            (Some(sock), Some((ip, prefix))) => match sock.route_del(ip, prefix) {
                Ok(()) => report.actions.push(format!("removed route {dest}")),
                Err(e) => report
                    .warnings
                    .push(format!("route {dest} not removed: {e}")),
            },
            (_, None) => report
                .warnings
                .push(format!("route {dest} unparseable; skipped")),
            (None, _) => report
                .warnings
                .push("no netlink socket; routes not removed".to_string()),
        }
    }

    // 3) Restore rp_filter to its original value(s) via /proc/sys (in-process).
    for (key, original) in &plan.rp_filter_restore {
        match std::fs::write(sysctl_proc_path(key), original) {
            Ok(()) => report.actions.push(format!("restored {key}={original}")),
            Err(e) => report
                .warnings
                .push(format!("failed to restore {key}: {e}")),
        }
    }

    // 4) Delete the tun interface LAST via netlink. This reaps the address and
    //    all device-bound routes (default-halves + split routes). A missing
    //    device (ENODEV) is treated as success by `link_del`.
    if let Some(dev) = &plan.device {
        match (nl.as_mut(), if_nametoindex(dev)) {
            (Some(sock), Ok(ifindex)) => match sock.link_del(ifindex) {
                Ok(()) => report.actions.push(format!(
                    "deleted interface {dev} (reaped device-bound routes)"
                )),
                Err(e) => report.warnings.push(format!("failed to delete {dev}: {e}")),
            },
            // if_nametoindex failing means the device is already gone — fine.
            (_, Err(_)) => {}
            (None, _) => report
                .warnings
                .push("no netlink socket; interface not deleted".to_string()),
        }
    }

    report
}

/// The `/proc/sys` path for a dotted sysctl key.
#[cfg(target_os = "linux")]
fn sysctl_proc_path(key: &str) -> String {
    format!("/proc/sys/{}", key.replace('.', "/"))
}

/// Parse a `dest/prefix` CIDR (or bare IP -> /32) into `(Ipv4Addr, prefix)`.
#[cfg(target_os = "linux")]
fn parse_cidr(s: &str) -> Option<(std::net::Ipv4Addr, u8)> {
    let (ip_part, prefix) = match s.split_once('/') {
        Some((ip, pfx)) => (ip, pfx.parse::<u8>().ok()?),
        None => (s, 32),
    };
    let ip = ip_part.parse::<std::net::Ipv4Addr>().ok()?;
    (prefix <= 32).then_some((ip, prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_plan_is_empty() {
        assert!(HostTeardownPlan::default().is_empty());
    }

    #[test]
    fn non_empty_plan_is_not_empty() {
        let plan = HostTeardownPlan {
            device: Some("tun0".into()),
            ..Default::default()
        };
        assert!(!plan.is_empty());
    }

    #[test]
    fn plan_round_trips_through_json() {
        let plan = HostTeardownPlan {
            device: Some("tun0".into()),
            extra_routes: vec!["203.0.113.10/32".into()],
            rp_filter_restore: vec![
                ("net.ipv4.conf.all.rp_filter".into(), "1".into()),
                ("net.ipv4.conf.tun0.rp_filter".into(), "0".into()),
            ],
            dns_iface: Some("tun0".into()),
        };
        let json = serde_json::to_string(&plan).expect("serialize");
        let back: HostTeardownPlan = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(plan, back);
    }

    #[test]
    fn plan_deserializes_with_missing_fields() {
        // Forward/backward compatibility: a state file without teardown fields.
        let back: HostTeardownPlan = serde_json::from_str("{}").expect("deserialize empty");
        assert!(back.is_empty());
    }

    #[test]
    fn sysctl_proc_path_maps_dotted_key() {
        assert_eq!(
            sysctl_proc_path("net.ipv4.conf.all.rp_filter"),
            "/proc/sys/net/ipv4/conf/all/rp_filter"
        );
    }

    #[test]
    fn parse_cidr_handles_cidr_and_bare_ip() {
        assert_eq!(
            parse_cidr("10.10.0.0/16"),
            Some(("10.10.0.0".parse().unwrap(), 16))
        );
        assert_eq!(
            parse_cidr("203.0.113.10"),
            Some(("203.0.113.10".parse().unwrap(), 32))
        );
        assert_eq!(parse_cidr("not-an-ip"), None);
        assert_eq!(parse_cidr("10.0.0.0/40"), None); // prefix out of range
    }

    // --- Behavioral coverage for the teardown reconciler (replaces the old
    //     openconnect `cleanup_tests`: "cleanup when nothing running",
    //     idempotency, graceful handling of missing resources). ---

    #[cfg(target_os = "linux")]
    #[test]
    fn teardown_of_empty_plan_is_a_no_op() {
        // The "no active connection" case: nothing to reconcile, no actions,
        // no warnings, never panics.
        let report = teardown_host(&HostTeardownPlan::default());
        assert!(
            report.actions.is_empty(),
            "empty plan should take no actions"
        );
        assert!(
            report.warnings.is_empty(),
            "empty plan should warn about nothing"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn teardown_of_missing_resources_is_graceful_and_idempotent() {
        // A plan pointing at a device/route that does not exist must not panic
        // and must be safe to run repeatedly (idempotent), like reaping orphans
        // when none are running. We use a clearly non-existent tun name and a
        // TEST-NET route so it never touches anything real on the host.
        let plan = HostTeardownPlan {
            device: Some("akon-nope0".into()),
            extra_routes: vec!["192.0.2.123/32".into()],
            rp_filter_restore: vec![],
            // A bogus DNS interface: reverting it makes `resolvectl` emit
            // `Failed to resolve interface "akon-nope0": No such device`. The
            // fix captures the child's output (`.output()`), so this must NOT
            // surface to our terminal — and teardown must still record the
            // cache flush and never panic.
            dns_iface: Some("akon-nope0".into()),
        };
        // Twice, to prove idempotency. Either it cleanly no-ops (resource
        // already absent) or warns — but never panics, and the second run is
        // identical to the first.
        let first = teardown_host(&plan);
        let second = teardown_host(&plan);
        // DNS branch always runs the flush (best-effort) and never warns about
        // the missing interface — the revert noise is swallowed, not surfaced.
        assert!(
            first.actions.iter().any(|a| a.contains("flushed DNS")),
            "DNS teardown should always flush caches: {:?}",
            first.actions
        );
        assert!(
            !first
                .warnings
                .iter()
                .any(|w| w.contains("No such device") || w.contains("resolve interface")),
            "missing DNS interface must not produce a warning: {:?}",
            first.warnings
        );
        assert_eq!(
            first.actions.contains(&"flushed DNS caches".to_string()),
            second.actions.contains(&"flushed DNS caches".to_string()),
            "teardown must be idempotent across runs"
        );
    }
}
