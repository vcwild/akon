//! Real Linux TUN device implementation (production data plane).
//!
//! Opens `/dev/net/tun`, creates a TUN interface via `ioctl(TUNSETIFF)`, applies
//! the negotiated [`TunConfig`] (address, MTU, routes, DNS) using the `ip`
//! tooling, and exposes async packet read/write through the [`TunDevice`] seam.
//!
//! Requires `CAP_NET_ADMIN` (root). It is intentionally a thin adapter: all
//! protocol logic lives above the seam and is validated offline by the test
//! actors framework, so this module is the small piece that must run on a real
//! kernel. It is Linux-only (gated at the module declaration in `f5/mod.rs`).

use crate::vpn::f5::netlink::{if_indextoname, if_nametoindex, NetlinkSocket};
use crate::vpn::f5::teardown::HostTeardownPlan;
use crate::vpn::transport::{TunConfig, TunDevice};
use async_trait::async_trait;
use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use tokio::io::unix::AsyncFd;
use tokio::io::Interest;

const TUN_PATH: &str = "/dev/net/tun";

// From <linux/if_tun.h>.
const IFF_TUN: i16 = 0x0001;
const IFF_NO_PI: i16 = 0x1000;
// _IOW('T', 202, int) — TUNSETIFF.
const TUNSETIFF: libc::c_ulong = 0x4004_54ca;
const IFNAMSIZ: usize = 16;

#[repr(C)]
struct IfReq {
    ifr_name: [libc::c_char; IFNAMSIZ],
    ifr_flags: i16,
    _pad: [u8; 22],
}

/// A real Linux TUN device.
///
/// I/O goes through [`AsyncFd`] doing **raw `read(2)`/`write(2)` syscalls** on
/// the TUN fd. A TUN is a packet (datagram) device, not a regular file: using a
/// buffered, offset-tracking abstraction like `tokio::fs::File` causes packets
/// just written to be read straight back (an echo/loop), so we must talk to the
/// fd directly with each syscall transferring exactly one packet.
pub struct LinuxTun {
    fd: AsyncFd<OwnedFd>,
    name: String,
    /// A persistable record of every host mutation made in `configure`, so an
    /// out-of-process `akon vpn off` can reconcile the host even after a
    /// SIGKILL. Built up during `configure`. Drop replays it (best-effort) for
    /// the in-process exit path.
    plan: HostTeardownPlan,
}

impl LinuxTun {
    /// Open `/dev/net/tun` and create a TUN interface (kernel-assigned name when
    /// `requested_name` is empty, e.g. `tun0`).
    pub fn open(requested_name: &str) -> io::Result<Self> {
        // Open the clone device.
        let std_file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(TUN_PATH)?;
        let fd = std_file.as_raw_fd();

        // Build the ifreq.
        let mut ifr = IfReq {
            ifr_name: [0; IFNAMSIZ],
            ifr_flags: IFF_TUN | IFF_NO_PI,
            _pad: [0; 22],
        };
        for (i, b) in requested_name.bytes().take(IFNAMSIZ - 1).enumerate() {
            ifr.ifr_name[i] = b as libc::c_char;
        }

        // SAFETY: fd is a valid open file; ifr is a correctly-sized ifreq.
        let rc = unsafe { libc::ioctl(fd, TUNSETIFF, &mut ifr as *mut _) };
        if rc < 0 {
            let err = io::Error::last_os_error();
            // EPERM means we lack CAP_NET_ADMIN — the rootless path is to grant
            // the capability to the akon binary once, so akon can run as the
            // user (keyring intact) while still creating the TUN.
            if err.raw_os_error() == Some(libc::EPERM) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "creating the TUN device requires CAP_NET_ADMIN. Grant it once with: \
                     `sudo setcap cap_net_admin+ep <path-to-akon>` and then run akon as your \
                     normal user (no sudo) so the keyring stays accessible",
                ));
            }
            return Err(err);
        }

        // Recover the (possibly kernel-assigned) interface name.
        let name = ifr
            .ifr_name
            .iter()
            .take_while(|&&c| c != 0)
            .map(|&c| c as u8 as char)
            .collect::<String>();

        // The fd must be non-blocking for AsyncFd readiness-based I/O.
        let owned: OwnedFd = std_file.into();
        set_nonblocking(owned.as_raw_fd())?;
        let fd = AsyncFd::new(owned)?;

        Ok(Self {
            fd,
            name,
            plan: HostTeardownPlan::default(),
        })
    }

    /// The interface name (e.g. `tun0`).
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Put a file descriptor into non-blocking mode (required for `AsyncFd`).
fn set_nonblocking(fd: std::os::fd::RawFd) -> io::Result<()> {
    // SAFETY: fd is a valid open descriptor we own.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: same fd; setting O_NONBLOCK.
    let rc = unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// The `/proc/sys` path for a dotted sysctl key (`net.ipv4.conf.all.rp_filter`
/// -> `/proc/sys/net/ipv4/conf/all/rp_filter`).
fn sysctl_proc_path(key: &str) -> String {
    format!("/proc/sys/{}", key.replace('.', "/"))
}

/// Read a sysctl value via `/proc/sys` directly (no child process, so it works
/// under a file capability). Returns the trimmed value, or `None`.
fn read_sysctl(key: &str) -> Option<String> {
    let val = std::fs::read_to_string(sysctl_proc_path(key)).ok()?;
    let val = val.trim().to_string();
    (!val.is_empty()).then_some(val)
}

/// Write a sysctl value via `/proc/sys` directly (in-process, capability-safe).
fn write_sysctl(key: &str, value: &str) -> io::Result<()> {
    std::fs::write(sysctl_proc_path(key), value)
}

/// Parse a `dest/prefix` CIDR (or a bare IP, treated as /32) into
/// `(Ipv4Addr, prefix)`. Returns `None` if unparseable.
fn parse_cidr(s: &str) -> Option<(std::net::Ipv4Addr, u8)> {
    let (ip_part, prefix) = match s.split_once('/') {
        Some((ip, pfx)) => (ip, pfx.parse::<u8>().ok()?),
        None => (s, 32),
    };
    let ip = ip_part.parse::<std::net::Ipv4Addr>().ok()?;
    (prefix <= 32).then_some((ip, prefix))
}

/// Discover the host's current IPv4 default route as `(gateway, oif_index)` via
/// netlink. Skips any default already pointing at a `tun*` interface (a stale
/// akon route). Used by full-tunnel mode to pin the VPN server's own packets to
/// the real gateway.
fn original_default_route() -> Option<(std::net::Ipv4Addr, u32)> {
    let mut nl = NetlinkSocket::open().ok()?;
    let (gw, oif) = nl.default_route().ok()??;
    // Skip a default that already points at a tun (stale akon route): resolve
    // the oif's name and reject tun*.
    if let Some(name) = if_indextoname(oif) {
        if name.starts_with("tun") {
            return None;
        }
    }
    Some((gw, oif))
}

/// Convert a dotted netmask to a CIDR prefix length (e.g. 255.255.0.0 -> 16).
fn netmask_to_prefix(mask: &str) -> Option<u8> {
    let octets: Vec<u8> = mask.split('.').filter_map(|o| o.parse().ok()).collect();
    if octets.len() != 4 {
        return None;
    }
    let bits = u32::from_be_bytes([octets[0], octets[1], octets[2], octets[3]]);
    // Must be contiguous 1s then 0s.
    let ones = bits.leading_ones();
    if bits == (!0u32).checked_shl(32 - ones).unwrap_or(0) {
        Some(ones as u8)
    } else {
        None
    }
}

/// Normalize an F5 route string to a form `ip route` accepts. Converts
/// `network/dotted-mask` to `network/prefix`; passes through CIDR and bare
/// networks unchanged.
fn normalize_route(route: &str) -> String {
    if let Some((net, mask)) = route.split_once('/') {
        // Already a prefix (e.g. "10.0.0.0/8")?
        if mask.parse::<u8>().is_ok() {
            return route.to_string();
        }
        // Dotted mask -> prefix.
        if let Some(prefix) = netmask_to_prefix(mask) {
            return format!("{net}/{prefix}");
        }
    }
    route.to_string()
}

/// Whether a (normalized) route is the IPv4 default route.
fn is_default_route(route: &str) -> bool {
    matches!(route, "default" | "0.0.0.0/0")
        || route.starts_with("0.0.0.0/0.0.0.0")
        || route == "0.0.0.0/0"
}

#[async_trait]
impl TunDevice for LinuxTun {
    fn name(&self) -> String {
        self.name.clone()
    }

    /// The teardown plan describing every host mutation made by `configure`.
    /// Persist this (e.g. into the VPN state file) so `akon vpn off` can undo the
    /// changes even if this process is killed before its `Drop` runs.
    fn teardown_plan(&self) -> HostTeardownPlan {
        self.plan.clone()
    }

    async fn configure(&mut self, config: &TunConfig) -> io::Result<()> {
        let dev = self.name.clone();
        let debug = crate::vpn::f5::http::debug_enabled();
        if debug {
            eprintln!(
                "[tun-cfg] dev={dev} ipv4={:?} mtu={:?} default_gateway={} routes={:?} dns={:?} domains={:?} server_ip={:?}",
                config.ipv4, config.mtu, config.default_gateway, config.routes,
                config.dns, config.domains, config.server_ip
            );
        }

        // Record the device in the teardown plan up front: deleting it reaps all
        // device-bound routes (address, default-halves, split routes).
        self.plan.device = Some(dev.clone());

        // Open an in-process netlink socket. ALL link/address/route operations go
        // through it (NOT `ip`), so they run under akon's own capability and work
        // rootless via `setcap cap_net_admin+ep` (a spawned `ip` would not inherit
        // the file capability). See ADR 0001.
        let mut nl = NetlinkSocket::open()?;
        let ifindex = if_nametoindex(&dev)?;

        // MTU.
        if let Some(mtu) = config.mtu {
            nl.link_set_mtu(ifindex, mtu)?;
        }

        // Address. Use /32 (F5 assigns a host address). Log success so a silent
        // failure (which would break local delivery of replies) is visible.
        if let Some(addr) = &config.ipv4 {
            if let Ok(ip4) = addr.parse::<std::net::Ipv4Addr>() {
                match nl.addr_add(ifindex, ip4, 32) {
                    Ok(()) => {
                        if debug {
                            eprintln!("[tun-cfg] added address {addr}/32 dev {dev}");
                        }
                    }
                    Err(e) => eprintln!("[tun-cfg] WARN add address {addr}/32 failed: {e}"),
                }
            } else {
                eprintln!("[tun-cfg] WARN: assigned IP {addr} is not valid IPv4");
            }
        }

        // Bring the link up.
        nl.link_up(ifindex)?;

        // Normalize and classify routes. The F5 server may express the default
        // route as `UseDefaultGateway0` OR as a split route `0.0.0.0/0` /
        // `0.0.0.0/0.0.0.0`. Either means FULL TUNNEL.
        let mut split_routes: Vec<String> = Vec::new();
        let mut full_tunnel = config.default_gateway;
        for raw in &config.routes {
            let norm = normalize_route(raw);
            if is_default_route(&norm) {
                full_tunnel = true;
            } else {
                split_routes.push(norm);
            }
        }
        if debug {
            eprintln!("[tun-cfg] full_tunnel={full_tunnel}; split_routes={split_routes:?}");
        }

        // --- Full-tunnel: route everything via the tun, but keep the encrypted
        //     tunnel's own packets to the VPN server on the ORIGINAL default
        //     gateway (otherwise they'd loop into the tun and the tunnel
        //     collapses). Mirrors openconnect's vpnc-script. The 0/1 + 128/1
        //     split-default trick overrides the default without deleting it.
        if full_tunnel {
            // 1) Pin the VPN server to the original gateway FIRST (before the
            //    default is overridden) so the encrypted tunnel keeps flowing.
            match original_default_route() {
                Some((orig_gw, orig_oif)) => {
                    if debug {
                        eprintln!("[tun-cfg] original default: via {orig_gw} oif {orig_oif}");
                    }
                    if let Some(server) = &config.server_ip {
                        if let Ok(server_ip) = server.parse::<std::net::Ipv4Addr>() {
                            match nl.route_add_via(server_ip, 32, orig_gw, orig_oif, true) {
                                Ok(()) => {
                                    if debug {
                                        eprintln!("[tun-cfg] pinned VPN server {server}/32 via original gw {orig_gw}");
                                    }
                                    // Persist for out-of-process teardown: this route
                                    // is NOT device-bound and won't die with the tun.
                                    self.plan.extra_routes.push(format!("{server}/32"));
                                }
                                Err(e) => eprintln!("[tun-cfg] WARN pin server route failed: {e}"),
                            }
                        }
                    } else {
                        eprintln!("[tun-cfg] WARN: no server_ip to pin; tunnel packets may loop");
                    }
                }
                None => eprintln!("[tun-cfg] WARN: no original default route; cannot pin server"),
            }
            // 2) Override the default with two /1 routes via the tun.
            for (dest, prefix) in [
                (std::net::Ipv4Addr::new(0, 0, 0, 0), 1u8),
                (std::net::Ipv4Addr::new(128, 0, 0, 0), 1u8),
            ] {
                match nl.route_add_dev(dest, prefix, ifindex, true) {
                    Ok(()) => {
                        if debug {
                            eprintln!("[tun-cfg] default-half {dest}/{prefix} via {dev}");
                        }
                    }
                    Err(e) => eprintln!("[tun-cfg] WARN default-half {dest}/{prefix} failed: {e}"),
                }
            }
        }

        // --- Split-include routes (non-default). ---
        let mut installed = 0usize;
        for route in &split_routes {
            match parse_cidr(route) {
                Some((dest, prefix)) => match nl.route_add_dev(dest, prefix, ifindex, true) {
                    Ok(()) => {
                        installed += 1;
                        if debug {
                            eprintln!("[tun-cfg] installed split route {route} via {dev}");
                        }
                    }
                    Err(e) => eprintln!("[tun-cfg] WARN split route {route} failed: {e}"),
                },
                None => eprintln!("[tun-cfg] WARN unparseable split route {route}"),
            }
        }
        if debug {
            eprintln!(
                "[tun-cfg] routes done: {installed}/{} split installed; full_tunnel={full_tunnel}",
                split_routes.len()
            );
        }

        // Loosen reverse-path filtering on the tun so replies arriving on it are
        // not silently dropped when the kernel computes an asymmetric return
        // path (a very common cause of "routes look right but traffic hangs").
        // Written via /proc/sys directly (no child process) so it is capability
        // -safe. Best-effort; also set 'all' to loose for the same reason.
        for key in [
            format!("net.ipv4.conf.{dev}.rp_filter"),
            "net.ipv4.conf.all.rp_filter".to_string(),
        ] {
            // Record the ORIGINAL value first so teardown can restore it exactly
            // (otherwise `all.rp_filter` would be left loosened forever).
            if let Some(orig) = read_sysctl(&key) {
                self.plan.rp_filter_restore.push((key.clone(), orig));
            }
            let _ = write_sysctl(&key, "2");
        }

        // NOTE: DNS is applied by the DnsApplier seam in `run_data_plane`, and
        // `dns_iface` (the teardown's DNS-revert target) is recorded THERE — only
        // when a host-mutating applier actually applies DNS. Recording it here
        // (based merely on the negotiated config) would make a NoopDns/test run
        // schedule a `resolvectl` call against the un-namespaced host resolver.
        Ok(())
    }

    async fn write_packet(&mut self, packet: &[u8]) -> io::Result<()> {
        // One write(2) == one packet on a TUN. Wait for writability, then issue
        // the raw syscall via the guard, retrying on spurious wakeups.
        loop {
            let mut guard = self.fd.writable().await?;
            match guard.try_io(|inner| {
                let fd = inner.get_ref().as_raw_fd();
                // SAFETY: fd is valid; packet is a readable slice.
                let rc = unsafe {
                    libc::write(fd, packet.as_ptr() as *const libc::c_void, packet.len())
                };
                if rc < 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(rc as usize)
                }
            }) {
                Ok(res) => return res.map(|_| ()),
                Err(_would_block) => continue,
            }
        }
    }

    async fn read_packet(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // One read(2) == one packet on a TUN. Wait for readability, then issue
        // the raw syscall via the guard, retrying on spurious wakeups.
        let n = loop {
            let mut guard = self.fd.ready(Interest::READABLE).await?;
            match guard.try_io(|inner| {
                let fd = inner.get_ref().as_raw_fd();
                // SAFETY: fd is valid; buf is a writable slice.
                let rc =
                    unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
                if rc < 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(rc as usize)
                }
            }) {
                Ok(res) => break res?,
                Err(_would_block) => continue,
            }
        };
        Ok(n)
    }
}

impl Drop for LinuxTun {
    /// Guarantee the interface is removed from a production host.
    ///
    /// The TUN was created **without** `IFF_PERSIST`, so the kernel removes the
    /// interface (and any routes bound to it) automatically when the underlying
    /// fd is closed — which happens when `self.fd` (the `AsyncFd<OwnedFd>`) is
    /// dropped here. As an explicit belt-and-suspenders safety net we also delete
    /// the link **via netlink** (best-effort, in-process so it is capability-safe
    /// and ignored if the kernel already reaped it). Together these ensure no
    /// `tun%d` device or device-bound route is ever left behind, on any exit path
    /// (normal disconnect, error, panic, or process teardown). Non-device-bound
    /// routes (the server pin) and rp_filter are restored by the persisted
    /// [`HostTeardownPlan`] reconciler (`teardown_host`), which `Drop` also runs
    /// here for the in-process exit path.
    fn drop(&mut self) {
        // Reconcile non-device-bound state (server-pin route, rp_filter) via the
        // recorded plan — these do NOT die with the interface. Best-effort.
        let _ = crate::vpn::f5::teardown::teardown_host(&self.plan);
    }
}

#[cfg(test)]
mod tests {
    use super::{is_default_route, netmask_to_prefix, normalize_route};

    #[test]
    fn netmask_conversions() {
        assert_eq!(netmask_to_prefix("255.255.255.255"), Some(32));
        assert_eq!(netmask_to_prefix("255.255.0.0"), Some(16));
        assert_eq!(netmask_to_prefix("255.0.0.0"), Some(8));
        assert_eq!(netmask_to_prefix("0.0.0.0"), Some(0));
        assert_eq!(netmask_to_prefix("255.0.255.0"), None); // non-contiguous
    }

    #[test]
    fn normalize_handles_mask_and_cidr_forms() {
        assert_eq!(normalize_route("10.0.0.0/255.0.0.0"), "10.0.0.0/8");
        assert_eq!(normalize_route("10.0.0.0/8"), "10.0.0.0/8");
        assert_eq!(normalize_route("0.0.0.0/0.0.0.0"), "0.0.0.0/0");
        assert_eq!(normalize_route("10.10.0.0/255.255.0.0"), "10.10.0.0/16");
    }

    #[test]
    fn detects_default_route_in_all_forms() {
        // The exact form the real F5 sent.
        assert!(is_default_route(&normalize_route("0.0.0.0/0.0.0.0")));
        assert!(is_default_route("0.0.0.0/0"));
        assert!(is_default_route("default"));
        assert!(!is_default_route("10.0.0.0/8"));
        assert!(!is_default_route(&normalize_route("10.10.0.0/255.255.0.0")));
    }
}
