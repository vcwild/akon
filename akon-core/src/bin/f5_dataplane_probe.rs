//! Containerized data-plane round-trip probe.
//!
//! Runs the REAL native F5 data plane inside a container/netns to reproduce the
//! production "reply loops / not delivered locally" symptom deterministically:
//!
//! 1. Spawns an in-process fake F5 server (`F5ServerActor`) that completes the
//!    handshake and, in the data phase, **echoes IP packets with src/dst
//!    swapped** (a faithful echo responder).
//! 2. Brings up `NativeF5Backend` with a **real `LinuxTun`** over an in-memory
//!    transport to that server, so the actual TUN + routing code runs.
//! 3. Binds a UDP socket to the assigned tunnel IP, sends a datagram to a target
//!    IP that is routed through the tunnel, and checks the **echo comes back to
//!    the local socket**.
//!
//! Prints `RESULT: ok` (round-trip delivered) or `RESULT: fail <why>` and exits
//! accordingly. Needs `CAP_NET_ADMIN` (run in a container with --cap-add
//! NET_ADMIN --device /dev/net/tun, or as root). Only built with `test-actors`.

use std::time::Duration;

use akon_core::vpn::backend::{Credentials, LifecycleEvent, VpnBackend};
use akon_core::vpn::f5::dns::NoopDns;
use akon_core::vpn::f5::tun::LinuxTun;
use akon_core::vpn::f5::NativeF5Backend;
use akon_core::vpn::testkit::{F5ServerActor, F5ServerScript, MemoryTransport};

#[tokio::main]
async fn main() {
    match run().await {
        Ok(()) => {
            println!("RESULT: ok");
            std::process::exit(0);
        }
        Err(e) => {
            println!("RESULT: fail {e}");
            std::process::exit(1);
        }
    }
}

async fn run() -> Result<(), String> {
    // HOST-SAFETY GUARD: this probe creates a real TUN and installs full-tunnel
    // routes, which would hijack the host's networking. Refuse to run unless we
    // are in an ISOLATED network namespace (not the host's init netns), so it can
    // never disrupt a developer's or production host. Run it via `unshare -rn`
    // (the netns regression test does this) or inside a container.
    require_isolated_netns()?;

    // Open a real TUN early to fail fast without privileges.
    let tun = LinuxTun::open("").map_err(|e| format!("open TUN (need CAP_NET_ADMIN): {e}"))?;

    // In-memory transport pair: one end drives the fake F5 server (echo mode),
    // the other is the backend's tunnel transport.
    let (client, mut server) = MemoryTransport::pair();
    let script = F5ServerScript {
        // assigned tunnel IP for the client
        assigned_ip: [10, 10, 99, 2],
        ..F5ServerScript::default()
    };
    tokio::spawn(async move {
        F5ServerActor::new(script).run(&mut server).await;
    });

    let mut backend = NativeF5Backend::with_parts(
        Box::new(client),
        Box::new(tun),
        Box::new(NoopDns),
        "f5.local",
    );

    let mut rx = backend
        .connect(Credentials::new("probe", "1234567890"))
        .map_err(|e| format!("connect start: {e}"))?;

    let mut tun_ip = None;
    while let Ok(Some(ev)) = tokio::time::timeout(Duration::from_secs(15), rx.recv()).await {
        if let LifecycleEvent::Connected { ip, .. } = ev {
            tun_ip = Some(ip);
            break;
        }
        if matches!(ev, LifecycleEvent::Failed { .. }) {
            return Err(format!("connect failed: {ev:?}"));
        }
    }
    let tun_ip = tun_ip.ok_or("never reached Connected")?;
    eprintln!("probe: connected, tunnel ip {tun_ip}");

    // Route a target IP through the tunnel. The echo server swaps src/dst, so a
    // packet we send to `target` returns as `target -> tun_ip`, which must be
    // delivered to our local socket.
    use akon_core::vpn::f5::netlink::{if_nametoindex, NetlinkSocket};
    use std::net::{Ipv4Addr, SocketAddrV4};
    let target: Ipv4Addr = "10.10.99.50".parse().expect("valid ipv4");
    let dst = SocketAddrV4::new(target, 7777);
    let dev = "tun0";
    // Route the probe target through the tun via NETLINK (not a child `ip`), so
    // the probe itself is rootless-capable under a `cap_net_admin+ep` file
    // capability — a spawned `ip` would not inherit the capability.
    let ifindex = if_nametoindex(dev).map_err(|e| format!("if_nametoindex({dev}): {e}"))?;
    let mut nl = NetlinkSocket::open().map_err(|e| format!("netlink open: {e}"))?;
    nl.route_add_dev(target, 32, ifindex, true)
        .map_err(|e| format!("failed to route {target}/32 via {dev}: {e}"))?;
    eprintln!("probe: routed {target}/32 via {dev} (netlink)");

    // Bind a UDP socket to the tunnel IP.
    let bind_addr = SocketAddrV4::new(tun_ip_v4(tun_ip)?, 0);
    let sock = tokio::net::UdpSocket::bind(bind_addr)
        .await
        .map_err(|e| format!("bind udp on {bind_addr}: {e}"))?;
    let local = sock.local_addr().map_err(|e| format!("local_addr: {e}"))?;
    eprintln!("probe: udp socket bound to {local}");
    let payload = b"AKON_DATAPLANE_PROBE";

    // Send a few datagrams (one may be lost while the tun settles) and wait for
    // the swapped echo to be delivered back to our local socket.
    let mut buf = [0u8; 256];
    let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
    let mut next_send = tokio::time::Instant::now();
    loop {
        if tokio::time::Instant::now() >= deadline {
            let _ = backend.disconnect();
            return Err(
                "no echo received through tunnel within 6s (data-plane round-trip failed)".into(),
            );
        }
        if tokio::time::Instant::now() >= next_send {
            match sock.send_to(payload, dst).await {
                Ok(n) => eprintln!("probe: sent {n} bytes to {dst} via tunnel"),
                Err(e) => eprintln!("probe: send error: {e}"),
            }
            next_send = tokio::time::Instant::now() + Duration::from_millis(750);
        }
        match tokio::time::timeout(Duration::from_millis(500), sock.recv_from(&mut buf)).await {
            Ok(Ok((n, from))) => {
                eprintln!("probe: received {n} bytes from {from}");
                if &buf[..n] != payload {
                    let _ = backend.disconnect();
                    return Err("echo payload mismatch".into());
                }
                // Round-trip proven. Now exercise the teardown reconciler: it
                // must remove every host mutation (interface + routes) so a real
                // host can't be left black-holed after `akon vpn off`.
                verify_teardown(&mut backend).await?;
                return Ok(());
            }
            Ok(Err(e)) => eprintln!("probe: recv error: {e}"),
            Err(_) => {} // timeout slice; loop and maybe re-send
        }
    }
}

/// Capture the backend's teardown plan, disconnect, run the host reconciler, and
/// assert the interface and the default-override routes are gone — proving
/// `akon vpn off` restores host networking. Prints `TEARDOWN: ok` on success.
async fn verify_teardown(backend: &mut akon_core::vpn::f5::NativeF5Backend) -> Result<(), String> {
    use akon_core::vpn::backend::VpnBackend;
    use akon_core::vpn::f5::teardown::teardown_host;

    let plan = backend.teardown_plan();
    let dev = plan.device.clone().ok_or("teardown plan has no device")?;
    eprintln!("probe: teardown plan = {plan:?}");

    let _ = backend.disconnect();
    tokio::time::sleep(Duration::from_millis(300)).await;

    let report = teardown_host(&plan);
    for a in &report.actions {
        eprintln!("probe: teardown action: {a}");
    }

    // The interface must be gone.
    let dev_present = std::process::Command::new("ip")
        .args(["link", "show", &dev])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if dev_present {
        return Err(format!("interface {dev} still present after teardown"));
    }

    // The default-override routes must be gone (they die with the interface).
    let routes = std::process::Command::new("ip")
        .args(["route", "show"])
        .output()
        .map_err(|e| format!("ip route show: {e}"))?;
    let routes = String::from_utf8_lossy(&routes.stdout);
    if routes.contains(&dev) {
        return Err(format!("routes via {dev} still present after teardown"));
    }

    eprintln!("TEARDOWN: ok");
    Ok(())
}

/// Refuse to run unless the caller has explicitly placed us in an isolated
/// network namespace (or container) AND verified the host is unreachable. This
/// probe creates a real TUN and installs **full-tunnel** routes, so running it
/// in the host netns would hijack the operator's networking. Rather than rely on
/// fragile auto-detection (which breaks under user namespaces), we require an
/// explicit handshake token that ONLY the isolation wrapper sets:
///
///   `AKON_PROBE_ISOLATED=1`
///
/// As an additional safety net, we also confirm there is **no real default
/// route off a physical interface** — i.e. the netns is the throwaway kind with
/// only loopback/tun. If a real uplink default is visible, we refuse even with
/// the token set, so the probe can never black-hole a host's connectivity.
fn require_isolated_netns() -> Result<(), String> {
    if std::env::var("AKON_PROBE_ISOLATED").as_deref() != Ok("1") {
        return Err(
            "refusing to run: this probe creates a real TUN + full-tunnel \
                    routes and must run ONLY inside an isolated network namespace \
                    or container. The isolation wrapper must set \
                    AKON_PROBE_ISOLATED=1 (see native_f5_netns_roundtrip_tests / \
                    the container harness). Never run it directly on a host."
                .to_string(),
        );
    }
    // Belt-and-suspenders: ensure no real uplink default route exists in this
    // namespace (a throwaway netns has only lo / a lo-default, not a physical
    // uplink). This blocks accidentally setting the token on a real host.
    if let Ok(mut nl) = akon_core::vpn::f5::netlink::NetlinkSocket::open() {
        if let Ok(Some((gw, oif))) = nl.default_route() {
            let name = akon_core::vpn::f5::netlink::if_indextoname(oif).unwrap_or_default();
            // A real uplink default has a non-loopback device and a real gateway.
            if !name.is_empty() && name != "lo" && !gw.is_unspecified() {
                return Err(format!(
                    "refusing to run: a real default route (via {gw} dev {name}) is \
                     visible — this looks like a real host, not an isolated netns. \
                     Run inside `unshare -rn` (loopback only) or a container."
                ));
            }
        }
    }
    Ok(())
}

/// Extract the IPv4 form of the assigned tunnel address.
fn tun_ip_v4(ip: std::net::IpAddr) -> Result<std::net::Ipv4Addr, String> {
    match ip {
        std::net::IpAddr::V4(v4) => Ok(v4),
        std::net::IpAddr::V6(_) => Err("tunnel IP is not IPv4".into()),
    }
}
