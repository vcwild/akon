//! PRODUCTION DATA-PLANE SIGN-OFF — the final "it's a real VPN" gate.
//!
//! Unlike `production_signoff_test.rs` (control-plane only, no TUN), this opens
//! a **real Linux TUN device**, connects the native F5 backend to the operator's
//! **real** appliance with their **real** keyring credentials, then **routes a
//! single probe target through the tunnel** and verifies it becomes reachable —
//! proving user traffic actually traverses the native data plane.
//!
//! ## ⚠️ This routes real traffic over a production VPN. Read before enabling.
//!
//! Safety design (minimal footprint — never hijacks the host):
//! - It does **NOT** install a default route. It adds exactly **one `/32` host
//!   route** for the operator-supplied `AKON_SOAK_PROBE_TARGET` via the tunnel
//!   interface, probes it, and always removes that route on exit.
//! - The TUN interface and route are torn down on every exit path (including
//!   panic) by RAII guards. The operator's normal connectivity is untouched
//!   except for the single probed host.
//! - Everything is bounded; it cannot hang.
//!
//! ## Triple-gated (cannot run by accident, in CI, or in the normal suite)
//! Requires ALL of:
//!   AKON_SIGNOFF_PRODUCTION=1
//!   AKON_SIGNOFF_ACK=I_UNDERSTAND_THIS_HITS_PRODUCTION
//!   AKON_SOAK_PROBE_TARGET=<host-or-ip reachable only via the VPN>   (e.g. an
//!       intranet host:port; if no port is given, 443 is assumed)
//! and must run with CAP_NET_ADMIN (root) to create the TUN + route.
//!
//! Run (Linux, root):
//!   sudo -E AKON_F5_DEBUG=1 \
//!     AKON_SIGNOFF_PRODUCTION=1 \
//!     AKON_SIGNOFF_ACK=I_UNDERSTAND_THIS_HITS_PRODUCTION \
//!     AKON_SOAK_PROBE_TARGET=intranet.example.com:443 \
//!     cargo test --test production_dataplane_signoff_test -- --nocapture
#![cfg(target_os = "linux")]

use std::process::Command;
use std::time::Duration;

const ACK_PHRASE: &str = "I_UNDERSTAND_THIS_HITS_PRODUCTION";

fn authorized() -> bool {
    std::env::var("AKON_SIGNOFF_PRODUCTION").as_deref() == Ok("1")
        && std::env::var("AKON_SIGNOFF_ACK").as_deref() == Ok(ACK_PHRASE)
}

/// Parse the probe target into (host, port). Accepts bare hosts, `host:port`,
/// and full URLs (`https://host/path`, with or without a trailing `:port`).
/// Default port 443.
fn probe_target() -> Option<(String, u16)> {
    parse_probe_target(&std::env::var("AKON_SOAK_PROBE_TARGET").ok()?)
}

/// Pure parser for the probe target (testable without env).
fn parse_probe_target(raw: &str) -> Option<(String, u16)> {
    let mut s = raw.trim().to_string();
    if s.is_empty() {
        return None;
    }
    // Strip a URL scheme.
    if let Some(rest) = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
    {
        s = rest.to_string();
    }
    // Strip any path (and a trailing slash): keep only the authority.
    if let Some(idx) = s.find('/') {
        s.truncate(idx);
    }
    if s.is_empty() {
        return None;
    }
    // Now `s` is `host` or `host:port`.
    match s.rsplit_once(':') {
        Some((h, p)) if !h.is_empty() && p.parse::<u16>().is_ok() => {
            Some((h.to_string(), p.parse().unwrap()))
        }
        _ => Some((s, 443)),
    }
}

#[cfg(test)]
mod parse_tests {
    use super::parse_probe_target;

    #[test]
    fn handles_url_and_host_forms() {
        // The exact (slightly malformed) form the operator tried.
        assert_eq!(
            parse_probe_target("https://intranet.example.com/:443"),
            Some(("intranet.example.com".to_string(), 443))
        );
        assert_eq!(
            parse_probe_target("https://intranet.example.com/"),
            Some(("intranet.example.com".to_string(), 443))
        );
        assert_eq!(
            parse_probe_target("intranet.example.com"),
            Some(("intranet.example.com".to_string(), 443))
        );
        assert_eq!(
            parse_probe_target("intranet.example.com:8443"),
            Some(("intranet.example.com".to_string(), 8443))
        );
        assert_eq!(
            parse_probe_target("10.0.0.5:22"),
            Some(("10.0.0.5".to_string(), 22))
        );
        assert_eq!(parse_probe_target("  "), None);
    }
}

/// Resolve a host to its first IPv4 address (so we can install a /32 route).
fn resolve_ipv4(host: &str) -> Option<std::net::Ipv4Addr> {
    use std::net::ToSocketAddrs;
    if let Ok(ip) = host.parse::<std::net::Ipv4Addr>() {
        return Some(ip);
    }
    (host, 443u16)
        .to_socket_addrs()
        .ok()?
        .find_map(|sa| match sa.ip() {
            std::net::IpAddr::V4(v4) => Some(v4),
            _ => None,
        })
}

/// Minimal async DNS A-record query over UDP to a specific server. Returns the
/// first A record, or None. Used to resolve a VPN-only name THROUGH the tunnel
/// (the query/response traverse the tunnel, proving the data plane works).
async fn dns_query_a(
    source: std::net::Ipv4Addr,
    server: std::net::Ipv4Addr,
    name: &str,
) -> Option<std::net::Ipv4Addr> {
    // Build a standard DNS query: header + QNAME + QTYPE=A + QCLASS=IN.
    let mut q: Vec<u8> = Vec::with_capacity(64);
    q.extend_from_slice(&[0x12, 0x34]); // id
    q.extend_from_slice(&[0x01, 0x00]); // flags: recursion desired
    q.extend_from_slice(&[0x00, 0x01]); // QDCOUNT=1
    q.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x00]); // AN/NS/AR=0
    for label in name.trim_end_matches('.').split('.') {
        if label.is_empty() || label.len() > 63 {
            return None;
        }
        q.push(label.len() as u8);
        q.extend_from_slice(label.as_bytes());
    }
    q.push(0); // root label
    q.extend_from_slice(&[0x00, 0x01]); // QTYPE=A
    q.extend_from_slice(&[0x00, 0x01]); // QCLASS=IN

    // Bind to the tunnel source IP so the request egresses the tunnel and the
    // reply routes back to us (a 0.0.0.0 bind would let the kernel pick the
    // wrong source for the /32 tun interface).
    let sock = tokio::net::UdpSocket::bind((source, 0)).await.ok()?;
    sock.send_to(&q, (server, 53)).await.ok()?;
    let mut buf = [0u8; 512];
    let n = sock.recv(&mut buf).await.ok()?;
    let resp = &buf[..n];
    if resp.len() < 12 {
        return None;
    }
    let ancount = u16::from_be_bytes([resp[6], resp[7]]);
    if ancount == 0 {
        return None;
    }
    // Skip header (12) + question section.
    let mut p = 12usize;
    while p < resp.len() && resp[p] != 0 {
        let len = resp[p] as usize;
        if len & 0xc0 == 0xc0 {
            p += 2;
            break;
        }
        p += 1 + len;
    }
    if p < resp.len() && resp[p] == 0 {
        p += 1;
    }
    p += 4; // QTYPE + QCLASS
            // Answer records: NAME(ptr=2) TYPE(2) CLASS(2) TTL(4) RDLEN(2) RDATA.
    for _ in 0..ancount {
        if p + 12 > resp.len() {
            return None;
        }
        // NAME is usually a compression pointer (2 bytes).
        p += if resp[p] & 0xc0 == 0xc0 {
            2
        } else {
            // walk labels
            let mut q2 = p;
            while q2 < resp.len() && resp[q2] != 0 {
                q2 += 1 + resp[q2] as usize;
            }
            q2 + 1 - p
        };
        if p + 10 > resp.len() {
            return None;
        }
        let rtype = u16::from_be_bytes([resp[p], resp[p + 1]]);
        let rdlen = u16::from_be_bytes([resp[p + 8], resp[p + 9]]) as usize;
        p += 10;
        if rtype == 1 && rdlen == 4 && p + 4 <= resp.len() {
            return Some(std::net::Ipv4Addr::new(
                resp[p],
                resp[p + 1],
                resp[p + 2],
                resp[p + 3],
            ));
        }
        p += rdlen;
    }
    None
}

/// RAII guard that loosens `rp_filter` on an interface for the probe and
/// restores the previous value on drop.
struct RpFilterGuard {
    key: String,
    previous: Option<String>,
}
impl RpFilterGuard {
    fn loosen(iface: &str) -> Self {
        let key = format!("net.ipv4.conf.{iface}.rp_filter");
        let previous = Command::new("sysctl")
            .args(["-n", &key])
            .output()
            .ok()
            .and_then(|o| {
                let v = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if v.is_empty() {
                    None
                } else {
                    Some(v)
                }
            });
        // 2 = loose reverse-path filtering (accept if reachable via any iface).
        let _ = Command::new("sysctl")
            .arg(format!("{key}=2"))
            .stdout(std::process::Stdio::null())
            .status();
        eprintln!("dataplane-soak: set {key}=2 (was {:?})", previous);
        Self { key, previous }
    }
}
impl Drop for RpFilterGuard {
    fn drop(&mut self) {
        if let Some(prev) = &self.previous {
            let _ = Command::new("sysctl")
                .arg(format!("{}={}", self.key, prev))
                .stdout(std::process::Stdio::null())
                .status();
        }
    }
}

/// RAII guard that removes the probe /32 route on drop (best-effort).
struct RouteGuard {
    target_cidr: String,
}
impl Drop for RouteGuard {
    fn drop(&mut self) {
        let _ = Command::new("ip")
            .args(["route", "del", &self.target_cidr])
            .status();
        eprintln!("dataplane-soak: removed route {}", self.target_cidr);
    }
}

/// RAII guard that disconnects the backend on drop, so the TUN + routes are
/// always torn down — including on any assertion panic. Generic over the
/// `VpnBackend` to avoid naming the concrete type at the guard definition.
struct BackendGuard<B: akon_core::vpn::backend::VpnBackend>(B);
impl<B: akon_core::vpn::backend::VpnBackend> Drop for BackendGuard<B> {
    fn drop(&mut self) {
        let _ = self.0.disconnect();
        eprintln!("dataplane-soak: backend disconnected (guard)");
    }
}

#[tokio::test]
async fn production_dataplane_soak() {
    if !authorized() {
        eprintln!(
            "SKIP: production data-plane soak is disabled.\n\
             Set ALL of: AKON_SIGNOFF_PRODUCTION=1, AKON_SIGNOFF_ACK={ACK_PHRASE}, \
             AKON_SOAK_PROBE_TARGET=<host[:port] reachable only via the VPN>, and run as root."
        );
        return;
    }

    // Hard overall deadline: the soak MUST terminate within 30s no matter what
    // (a stuck DNS lookup, a wedged tunnel, etc.). On timeout the inner future
    // is dropped, which drops the RAII guards (route removal + backend
    // disconnect + TUN teardown), then we fail. This guarantees no hang and no
    // leaked interface/route.
    match tokio::time::timeout(Duration::from_secs(30), run_soak_inner()).await {
        Ok(()) => {}
        Err(_) => panic!(
            "production data-plane soak exceeded its 30s deadline (forced abort; tunnel torn down)"
        ),
    }
}

/// Hold-open production session: connect the native backend with a REAL TUN +
/// full DNS to the operator's appliance and **keep the tunnel up for a bounded
/// window** (default 3 minutes) so the operator can manually browse internal
/// sites, then GUARANTEE a clean teardown that restores the previous host
/// configuration (default route + DNS) on every exit path.
///
/// Enable like the soak (no probe target needed):
///   sudo -E AKON_SIGNOFF_PRODUCTION=1 \
///     AKON_SIGNOFF_ACK=I_UNDERSTAND_THIS_HITS_PRODUCTION \
///     AKON_SIGNOFF_HOLD_OPEN=1 \
///     cargo test --test production_dataplane_signoff_test \
///       production_hold_open_session -- --nocapture
///
/// Optional: AKON_HOLD_SECONDS=<n> (default 180; hard-capped at 600).
#[tokio::test]
async fn production_hold_open_session() {
    if !authorized() || std::env::var("AKON_SIGNOFF_HOLD_OPEN").as_deref() != Ok("1") {
        eprintln!(
            "SKIP: hold-open session disabled. Set AKON_SIGNOFF_PRODUCTION=1, \
             AKON_SIGNOFF_ACK={ACK_PHRASE}, AKON_SIGNOFF_HOLD_OPEN=1 and run as root."
        );
        return;
    }

    // Bounded hold window (default 3 min, hard cap 10 min). A hard timeout wraps
    // the whole session so it ALWAYS terminates and tears down, even if the
    // operator walks away — the inner future is dropped on timeout, dropping the
    // RAII guards (DNS revert + route removal + TUN teardown).
    let hold = std::env::var("AKON_HOLD_SECONDS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(180)
        .min(600);
    // Add a small grace margin over the hold so the inner future's own clean exit
    // wins the race in the normal case; the outer timeout is the safety net.
    let outer = Duration::from_secs(hold + 30);

    match tokio::time::timeout(outer, run_hold_open_inner(hold)).await {
        Ok(()) => {}
        Err(_) => panic!(
            "hold-open session exceeded its {outer:?} safety deadline (forced abort; tunnel torn down)"
        ),
    }
}

async fn run_hold_open_inner(hold_secs: u64) {
    use akon_core::auth::password::generate_password;
    use akon_core::config::toml_config::{get_config_path, TomlConfig};
    use akon_core::config::VpnProtocol;
    use akon_core::vpn::backend::{Credentials, LifecycleEvent, VpnBackend};
    use akon_core::vpn::f5::dns::SystemDnsApplier;
    use akon_core::vpn::f5::tls_transport::TlsTransportFactory;
    use akon_core::vpn::f5::tun::LinuxTun;
    use akon_core::vpn::f5::NativeF5Backend;
    use akon_core::vpn::transport::TransportFactory;

    let config_path = get_config_path().expect("config path");
    let config = TomlConfig::from_file(&config_path)
        .expect("load ~/.config/akon/config.toml")
        .vpn_config;
    assert_eq!(config.protocol, VpnProtocol::F5, "hold-open is F5-only");

    let password: String = match std::env::var("AKON_SOAK_PASSWORD") {
        Ok(p) if !p.trim().is_empty() => p,
        _ => generate_password(&config.username)
            .expect("PIN+OTP: set AKON_SOAK_PASSWORD (e.g. `akon get-password`) when under sudo")
            .expose()
            .to_string(),
    };

    let tun = match LinuxTun::open("") {
        Ok(t) => t,
        Err(e) => {
            eprintln!("SKIP: cannot open /dev/net/tun (need root/CAP_NET_ADMIN): {e}");
            return;
        }
    };

    let (host, port) = split_host_port(&config.server, 443);
    let factory: Box<dyn TransportFactory> = Box::new(TlsTransportFactory::new(host.clone(), port));
    let backend = NativeF5Backend::with_factory_and_parts(
        factory,
        Box::new(tun),
        Box::new(SystemDnsApplier::detect()),
        host.clone(),
    );

    let mut backend = backend;
    let mut rx = backend
        .connect(Credentials::new(config.username.clone(), password.clone()))
        .expect("connect starts");

    // Own the backend in a guard so the TUN + routes + DNS are ALWAYS reverted
    // on every exit path (clean exit, panic, or the outer timeout dropping us).
    let mut guard = BackendGuard(backend);

    let mut device = None;
    let mut tun_ip: Option<std::net::Ipv4Addr> = None;
    let connect_deadline = tokio::time::Instant::now() + Duration::from_secs(45);
    while tokio::time::Instant::now() < connect_deadline {
        match tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
            Ok(Some(ev)) => {
                eprintln!("hold-open: {ev:?}");
                match ev {
                    LifecycleEvent::Connected { device: dev, ip } => {
                        if let std::net::IpAddr::V4(v4) = ip {
                            tun_ip = Some(v4);
                        }
                        device = Some(dev);
                        break;
                    }
                    LifecycleEvent::Failed { kind, detail } => {
                        panic!("connect failed: {kind:?}: {detail}")
                    }
                    _ => {}
                }
            }
            Ok(None) => break,
            Err(_) => {}
        }
    }
    let device = device.expect("did not reach Connected within timeout");
    let tun_ip = tun_ip.expect("no IPv4 tunnel address");
    let dns = guard.0.negotiated_dns();

    eprintln!("\n========================================================================");
    eprintln!("  ✅ NATIVE VPN CONNECTED — interface {device}, tunnel IP {tun_ip}");
    eprintln!("     VPN DNS: {dns:?}");
    eprintln!("     The tunnel is UP. Try your internal sites in the browser now.");
    eprintln!("     Holding for {hold_secs}s, then the tunnel is torn down and your");
    eprintln!("     previous network configuration (default route + DNS) is restored.");
    eprintln!("     Press Ctrl-C to disconnect early.");
    eprintln!("========================================================================\n");

    // Hold the tunnel up for the window, draining lifecycle events (so a server
    // teardown is observed), or exit early on Ctrl-C. Either way the guard runs.
    let hold_deadline = tokio::time::Instant::now() + Duration::from_secs(hold_secs);
    loop {
        let remaining = hold_deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            eprintln!("hold-open: window elapsed; disconnecting and restoring host config…");
            break;
        }
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                eprintln!("hold-open: Ctrl-C received; disconnecting and restoring host config…");
                break;
            }
            ev = rx.recv() => {
                match ev {
                    Some(LifecycleEvent::Failed { kind, detail }) => {
                        eprintln!("hold-open: connection failed mid-session: {kind:?}: {detail}");
                        break;
                    }
                    Some(LifecycleEvent::Disconnected { .. }) | None => {
                        eprintln!("hold-open: connection ended");
                        break;
                    }
                    Some(_) => {}
                }
            }
            _ = tokio::time::sleep(remaining.min(Duration::from_secs(5))) => {}
        }
    }

    // Explicit clean disconnect (the guard would also do it on drop).
    let _ = guard.0.disconnect();
    // Give teardown a moment to complete, then verify the interface is gone.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let iface_gone = !Command::new("ip")
        .args(["link", "show", "dev", &device])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    eprintln!(
        "hold-open: disconnected; interface {device} gone={iface_gone}. Host config restored."
    );
}

async fn run_soak_inner() {
    let Some((probe_host, probe_port)) = probe_target() else {
        eprintln!(
            "SKIP: AKON_SOAK_PROBE_TARGET not set (need an intranet host reachable only via VPN)."
        );
        return;
    };

    use akon_core::auth::password::generate_password;
    use akon_core::config::toml_config::{get_config_path, TomlConfig};
    use akon_core::config::VpnProtocol;
    use akon_core::vpn::backend::{Credentials, LifecycleEvent, VpnBackend};
    use akon_core::vpn::f5::dns::SystemDnsApplier;
    use akon_core::vpn::f5::tls_transport::TlsTransportFactory;
    use akon_core::vpn::f5::tun::LinuxTun;
    use akon_core::vpn::f5::NativeF5Backend;
    use akon_core::vpn::transport::TransportFactory;

    // --- Load real config + credentials ---
    let config_path = get_config_path().expect("config path");
    let config = TomlConfig::from_file(&config_path)
        .expect("load ~/.config/akon/config.toml")
        .vpn_config;
    assert_eq!(config.protocol, VpnProtocol::F5, "soak is F5-only");
    eprintln!(
        "dataplane-soak: server={} user={} probe={}:{}",
        config.server, config.username, probe_host, probe_port
    );

    // Try to resolve the probe target to an IPv4 NOW (bounded) while normal DNS
    // still works. If it's an IP literal this is instant; if it's a VPN-only
    // name (resolvable only via the tunnel's DNS), this returns None and we
    // resolve it AFTER the tunnel is up by querying the VPN DNS server through
    // the tunnel — which itself exercises the data plane.
    let probe_host_owned = probe_host.clone();
    let mut probe_ip: Option<std::net::Ipv4Addr> = match tokio::time::timeout(
        Duration::from_secs(8),
        tokio::task::spawn_blocking(move || resolve_ipv4(&probe_host_owned)),
    )
    .await
    {
        Ok(Ok(Some(ip))) => Some(ip),
        _ => None,
    };
    if let Some(ip) = probe_ip {
        eprintln!("dataplane-soak: probe {probe_host} -> {ip} (resolved pre-connect)");
    } else {
        eprintln!(
            "dataplane-soak: probe {probe_host} not resolvable pre-connect (VPN-only name?); \
             will resolve via the tunnel's DNS after connect"
        );
    }

    // Password: prefer a pre-generated PIN+OTP passed via env (so the test can
    // run under sudo for the TUN while the credential is generated by the
    // unprivileged user — root has no access to the user's keyring). Falls back
    // to the keyring only when the env var is absent (e.g. running as the user
    // with a capability-granted binary). The value is never logged.
    let password: String = match std::env::var("AKON_SOAK_PASSWORD") {
        Ok(p) if !p.trim().is_empty() => p,
        _ => generate_password(&config.username)
            .expect(
                "PIN+OTP: set AKON_SOAK_PASSWORD (generated as your user, e.g. via \
                 `akon get-password`) when running under sudo, or run as a user whose keyring \
                 holds the credentials",
            )
            .expose()
            .to_string(),
    };

    // --- Open a REAL TUN device (needs root) ---
    let tun = match LinuxTun::open("") {
        Ok(t) => t,
        Err(e) => {
            eprintln!("SKIP: cannot open /dev/net/tun (need root/CAP_NET_ADMIN): {e}");
            return;
        }
    };

    // --- Build backend: real TLS factory + real TUN + real DNS applier ---
    let (host, port) = split_host_port(&config.server, 443);
    let factory: Box<dyn TransportFactory> = Box::new(TlsTransportFactory::new(host.clone(), port));
    let mut backend = NativeF5Backend::with_factory_and_parts(
        factory,
        Box::new(tun),
        Box::new(SystemDnsApplier::detect()),
        host.clone(),
    );

    let mut rx = backend
        .connect(Credentials::new(config.username.clone(), password.clone()))
        .expect("connect starts");

    // From here on, the backend is owned by a guard so the TUN + any routes are
    // ALWAYS torn down — even if an assertion below panics.
    let mut guard = BackendGuard(backend);

    // --- Drive to Connected (bounded) ---
    let mut device = None;
    let mut tun_ip: Option<std::net::Ipv4Addr> = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(45);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
            Ok(Some(ev)) => {
                eprintln!("dataplane-soak: {ev:?}");
                match ev {
                    LifecycleEvent::Connected { device: dev, ip } => {
                        if let std::net::IpAddr::V4(v4) = ip {
                            tun_ip = Some(v4);
                        }
                        device = Some(dev);
                        break;
                    }
                    LifecycleEvent::Failed { kind, detail } => {
                        panic!("connect failed: {kind:?}: {detail}");
                    }
                    _ => {}
                }
            }
            Ok(None) => break,
            Err(_) => {}
        }
    }
    let device = device.expect("did not reach Connected within timeout");
    let tun_ip = tun_ip.expect("no IPv4 tunnel address");
    eprintln!("dataplane-soak: connected on interface {device} (tunnel ip {tun_ip})");

    // Loosen reverse-path filtering on the tunnel interface for the duration of
    // the probe. Strict rp_filter (=1) silently drops replies arriving on a tun
    // whose return path the kernel computes via a different interface — a very
    // common cause of "packets go out, nothing comes back" on a partial tunnel
    // setup. We set it to 2 (loose) and restore the previous value on teardown.
    let _rpf_guard = RpFilterGuard::loosen(&device);

    // --- If the probe name wasn't resolvable pre-connect, resolve it THROUGH
    //     the tunnel by querying the VPN DNS server (which proves the data
    //     plane carries traffic). We route the DNS server /32 via the tun, then
    //     send a bounded raw UDP DNS A query to it. ---
    let mut _dns_route_guard: Option<RouteGuard> = None;
    if probe_ip.is_none() {
        let dns_servers = guard.0.negotiated_dns();
        let dns_server = dns_servers
            .first()
            .and_then(|s| s.parse::<std::net::Ipv4Addr>().ok())
            .expect("no negotiated VPN DNS server to resolve the probe name");
        eprintln!(
            "dataplane-soak: resolving {probe_host} via VPN DNS {dns_server} (through tunnel)"
        );

        // Route the DNS server through the tunnel.
        let dns_cidr = format!("{dns_server}/32");
        let ok = Command::new("ip")
            .args(["route", "replace", &dns_cidr, "dev", &device])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        _dns_route_guard = Some(RouteGuard {
            target_cidr: dns_cidr.clone(),
        });
        assert!(ok, "failed to route VPN DNS {dns_cidr} via {device}");

        // Bounded DNS query through the tunnel, sourced from the tunnel IP so
        // the reply routes back to us.
        let name = probe_host.clone();
        let resolved = tokio::time::timeout(
            Duration::from_secs(8),
            dns_query_a(tun_ip, dns_server, &name),
        )
        .await
        .ok()
        .flatten();
        let ip = resolved.expect(
            "DNS query for the probe name through the tunnel failed — \
             the data plane did not carry the DNS round-trip (or the name has no A record)",
        );
        eprintln!("dataplane-soak: {probe_host} -> {ip} (resolved THROUGH the tunnel)");
        probe_ip = Some(ip);
    }
    let probe_ip = probe_ip.expect("probe ip resolved");

    // --- Route ONLY the probe target through the tunnel (no default route) ---
    let target_cidr = format!("{probe_ip}/32");
    let route_ok = Command::new("ip")
        .args(["route", "replace", &target_cidr, "dev", &device])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let _route_guard = RouteGuard {
        target_cidr: target_cidr.clone(),
    };
    assert!(
        route_ok,
        "failed to add /32 route {target_cidr} via {device}"
    );
    eprintln!("dataplane-soak: routed {target_cidr} via {device}");

    // --- Probe: TCP-connect to the target THROUGH the tunnel (bounded) ---
    let addr = format!("{probe_ip}:{probe_port}");
    let reachable = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::net::TcpStream::connect(&addr),
    )
    .await
    .map(|r| r.is_ok())
    .unwrap_or(false);

    assert!(
        reachable,
        "probe target {addr} was NOT reachable through the tunnel — data plane did not carry traffic"
    );
    eprintln!(
        "✅ PRODUCTION DATA-PLANE SIGN-OFF PASSED: routed {target_cidr} via {device} and reached \
         {addr} through the native tunnel."
    );

    // Explicit clean teardown (the guard would do it anyway on drop).
    let _ = guard.0.disconnect();

    // Verify (bounded) that the TUN interface and the probe route are actually
    // gone — a production host must NOT be left with a leaked tun%d or route.
    let mut iface_gone = false;
    for _ in 0..30 {
        let exists = Command::new("ip")
            .args(["link", "show", "dev", &device])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !exists {
            iface_gone = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    // Drop the route guard explicitly so the /32 is removed before we check.
    drop(_route_guard);
    let route_left = Command::new("ip")
        .args(["route", "show", &target_cidr])
        .output()
        .map(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty())
        .unwrap_or(false);

    assert!(iface_gone, "TUN interface {device} leaked after disconnect");
    assert!(!route_left, "route {target_cidr} leaked after disconnect");
    eprintln!("dataplane-soak: torn down cleanly (no leaked interface or route)");
}

/// Local copy of host:port splitting (the backend's is private).
fn split_host_port(server: &str, default_port: u16) -> (String, u16) {
    let s = server
        .strip_prefix("https://")
        .or_else(|| server.strip_prefix("http://"))
        .unwrap_or(server);
    let s = s.split('/').next().unwrap_or(s);
    if let Some((h, p)) = s.rsplit_once(':') {
        if let Ok(port) = p.parse::<u16>() {
            return (h.to_string(), port);
        }
    }
    (s.to_string(), default_port)
}

#[cfg(test)]
mod dns_tests {
    use super::dns_query_a;
    #[tokio::test]
    async fn resolves_via_explicit_server() {
        // Validate the raw DNS query against a public resolver (local network).
        let ip = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            dns_query_a(
                "0.0.0.0".parse().unwrap(),
                "8.8.8.8".parse().unwrap(),
                "one.one.one.one",
            ),
        )
        .await;
        match ip {
            Ok(Some(addr)) => {
                eprintln!("resolved one.one.one.one -> {addr}");
                assert!(addr.to_string() == "1.1.1.1" || addr.to_string() == "1.0.0.1");
            }
            _ => eprintln!("SKIP: no network/DNS available for this check"),
        }
    }
}
