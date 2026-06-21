//! Data-plane and teardown tests for the native F5 backend.
//!
//! These prove the parts that make `Connected` more than cosmetic: a real
//! bidirectional packet pump between a (fake) TUN device and the tunnel, and a
//! graceful teardown (PPP Terminate-Request + HTTP logout). All offline against
//! the fake F5 server actor — no root, no network, hang-proof.
#![cfg(feature = "test-actors")]

use std::time::Duration;

use akon_core::vpn::backend::{Credentials, LifecycleEvent, VpnBackend};
use akon_core::vpn::f5::NativeF5Backend;
use akon_core::vpn::testkit::f5_server_actor::{F5ServerActor, F5ServerScript};
use akon_core::vpn::testkit::fake_dns::{FakeDns, FakeDnsHandle};
use akon_core::vpn::testkit::fake_tun::{FakeTun, FakeTunHandle};
use akon_core::vpn::testkit::transport::MemoryTransport;
use tokio::sync::mpsc::UnboundedReceiver;

/// Wire a native backend (with a fake TUN) to a fake F5 server. Returns the
/// backend and the TUN handle for driving/inspecting the data plane.
fn wire(script: F5ServerScript) -> (NativeF5Backend, FakeTunHandle) {
    let (client, mut server) = MemoryTransport::pair();
    tokio::spawn(async move {
        F5ServerActor::new(script).run(&mut server).await;
    });
    let (tun, handle) = FakeTun::new();
    let backend =
        NativeF5Backend::with_transport_and_tun(Box::new(client), Box::new(tun), "vpn.example.com");
    (backend, handle)
}

/// Wire a native backend with fake TUN + fake DNS, returning the DNS handle too.
fn wire_with_dns(script: F5ServerScript) -> (NativeF5Backend, FakeTunHandle, FakeDnsHandle) {
    let (client, mut server) = MemoryTransport::pair();
    tokio::spawn(async move {
        F5ServerActor::new(script).run(&mut server).await;
    });
    let (tun, tun_handle) = FakeTun::new();
    let (dns, dns_handle) = FakeDns::new();
    let backend = NativeF5Backend::with_parts(
        Box::new(client),
        Box::new(tun),
        Box::new(dns),
        "vpn.example.com",
    );
    (backend, tun_handle, dns_handle)
}

/// Wait until a specific lifecycle label is observed (bounded).
async fn wait_for(rx: &mut UnboundedReceiver<LifecycleEvent>, label: &str) -> bool {
    loop {
        match tokio::time::timeout(Duration::from_secs(8), rx.recv()).await {
            Ok(Some(e)) => {
                if e.label() == label {
                    return true;
                }
            }
            _ => return false,
        }
    }
}

/// A minimal well-formed IPv4 packet (header only) for round-trip testing.
fn sample_ipv4_packet() -> Vec<u8> {
    // Version=4, IHL=5 -> 0x45; rest arbitrary but plausible. 20-byte header.
    let mut p = vec![0x45, 0x00, 0x00, 0x14];
    p.extend_from_slice(&[0x00, 0x01, 0x00, 0x00]); // id, flags
    p.extend_from_slice(&[0x40, 0x01, 0x00, 0x00]); // ttl, proto=ICMP, csum
    p.extend_from_slice(&[10, 20, 30, 40]); // src
    p.extend_from_slice(&[8, 8, 8, 8]); // dst
    p
}

/// The reply the fake F5 echo server produces for an IPv4 packet: swap the
/// source/destination addresses and recompute the IPv4 header checksum. (This
/// sample is ICMP, so there are no ports to swap.)
fn swapped_reply(packet: &[u8]) -> Vec<u8> {
    let mut p = packet.to_vec();
    for k in 0..4 {
        p.swap(12 + k, 16 + k);
    }
    let ihl = ((p[0] & 0x0f) as usize) * 4;
    p[10] = 0;
    p[11] = 0;
    let mut sum: u32 = 0;
    let mut i = 0;
    while i + 1 < ihl {
        sum += u16::from_be_bytes([p[i], p[i + 1]]) as u32;
        i += 2;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    let csum = !(sum as u16);
    p[10..12].copy_from_slice(&csum.to_be_bytes());
    p
}

#[tokio::test]
async fn native_f5_data_plane_round_trips_a_packet() {
    let (mut backend, tun) = wire(F5ServerScript::default());
    let mut rx = backend
        .connect(Credentials::new("alice", "pin123456"))
        .expect("connect starts");

    // Wait until the tunnel is up before sending data.
    assert!(wait_for(&mut rx, "Connected").await, "never connected");

    // Inject an OS-originated packet; the fake server echoes it back through the
    // tunnel as a faithful reply — it swaps the IPv4 source/destination (and, for
    // UDP, the ports) and fixes the checksums, so the reply is addressed back to
    // the sender. The reply therefore has src/dst swapped relative to what we
    // sent (this is exactly what exposed the real-TUN read-back loop bug).
    let packet = sample_ipv4_packet();
    tun.inject_from_os(packet.clone());
    let expected = swapped_reply(&packet);

    // Poll (bounded) for the echoed reply to be delivered to the OS.
    let mut got = None;
    for _ in 0..50 {
        let to_os = tun.packets_to_os();
        if let Some(p) = to_os.into_iter().find(|p| *p == expected) {
            got = Some(p);
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        got.as_ref(),
        Some(&expected),
        "echoed reply (src/dst swapped) did not round-trip through the data plane"
    );

    // The TUN was configured with the negotiated address AND the MTU derived
    // from the server's advertised MRU (1411), not the old hardcoded 1400.
    let cfg = tun.applied_config().expect("tun configured");
    assert_eq!(cfg.ipv4.as_deref(), Some("10.20.30.40"));
    assert_eq!(
        cfg.mtu,
        Some(1411),
        "MTU should be derived from negotiated MRU"
    );
}

#[tokio::test]
async fn native_f5_applies_negotiated_dns() {
    let (mut backend, _tun, dns) = wire_with_dns(F5ServerScript::default());
    let mut rx = backend
        .connect(Credentials::new("testuser", "1234567890"))
        .expect("connect starts");

    assert!(wait_for(&mut rx, "Connected").await, "never connected");

    // The fake server's options XML advertises DNS 8.8.8.8; the backend must
    // apply it to the host resolver (recorded by the fake DNS applier).
    let mut applied = Vec::new();
    for _ in 0..50 {
        applied = dns.applied_servers();
        if !applied.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        applied.contains(&"8.8.8.8".to_string()),
        "negotiated DNS was not applied: {applied:?}"
    );
}

#[tokio::test]
async fn native_f5_disconnect_tears_down_gracefully() {
    let (mut backend, _tun) = wire(F5ServerScript::default());
    let mut rx = backend
        .connect(Credentials::new("alice", "pin123456"))
        .expect("connect starts");

    assert!(wait_for(&mut rx, "Connected").await, "never connected");
    assert!(backend.is_alive());

    // Request disconnect; the session must stop pumping, tear down, and emit
    // Disconnected — all bounded, no hang.
    backend.disconnect().expect("disconnect");

    assert!(
        wait_for(&mut rx, "Disconnected").await,
        "never emitted Disconnected after disconnect"
    );
    assert!(!backend.is_alive());
}
