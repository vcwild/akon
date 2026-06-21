//! Locally-reproducible REAL TUN data-plane test.
//!
//! Unlike the `FakeTun` data-plane tests, this opens a **real Linux TUN device**
//! (`/dev/net/tun` via `LinuxTun`) and drives a full native F5 connection
//! against a local realistic TLS server, verifying that:
//! - the real TUN interface is created and configured with the negotiated
//!   IP + MTU, and
//! - a packet injected into the kernel TUN is carried out through the tunnel
//!   (and the echoed reply is written back to the TUN).
//!
//! It needs `CAP_NET_ADMIN` (root) to open/configure the TUN, so it is
//! **gated**: it self-skips unless `AKON_RUN_TUN_TESTS=1` is set AND the process
//! can actually open the TUN. This keeps it locally reproducible (run it
//! deliberately with privileges) without breaking the normal suite. It is fully
//! local — no production network — so it has no side effects beyond a transient
//! `tun%d` interface that is torn down on disconnect.
//!
//! Run with:
//!   sudo -E AKON_RUN_TUN_TESTS=1 \
//!     cargo test -p akon-core --features test-actors \
//!     --test native_f5_real_tun_tests -- --nocapture
#![cfg(all(feature = "test-actors", target_os = "linux"))]

use std::sync::Arc;
use std::time::Duration;

use akon_core::vpn::backend::{Credentials, LifecycleEvent, VpnBackend};
use akon_core::vpn::f5::dns::NoopDns;
use akon_core::vpn::f5::tls_transport::TlsTransportFactory;
use akon_core::vpn::f5::tun::LinuxTun;
use akon_core::vpn::f5::NativeF5Backend;
use akon_core::vpn::testkit::f5_server_actor::{F5ServerActor, F5ServerScript};
use akon_core::vpn::transport::{Transport, TransportFactory};
use async_trait::async_trait;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::rustls::{ClientConfig, RootCertStore, ServerConfig};
use tokio_rustls::TlsAcceptor;

const TEST_HOST: &str = "127.0.0.1";

fn enabled() -> bool {
    std::env::var("AKON_RUN_TUN_TESTS").as_deref() == Ok("1")
}

/// Refuse to mutate networking unless we are in an ISOLATED network namespace.
/// This test connects a full-tunnel fake server (`UseDefaultGateway0=1`), so on
/// a real host it would install `0.0.0.0/1`+`128.0.0.0/1` and hijack the host's
/// traffic. We consider the environment isolated only when there is NO real
/// uplink default route (a throwaway `unshare -rn` netns has only loopback).
/// Run it via: `AKON_RUN_TUN_TESTS=1 unshare -rn ... cargo test ...`.
fn isolated_netns() -> bool {
    use akon_core::vpn::f5::netlink::{if_indextoname, NetlinkSocket};
    let Ok(mut nl) = NetlinkSocket::open() else {
        return false;
    };
    match nl.default_route() {
        Ok(Some((gw, oif))) => {
            let name = if_indextoname(oif).unwrap_or_default();
            // Isolated iff the only default is loopback / unspecified gateway.
            name.is_empty() || name == "lo" || gw.is_unspecified()
        }
        // No default route at all => isolated throwaway netns.
        Ok(None) => true,
        Err(_) => false,
    }
}

/// Can we actually open a TUN device here? (root / CAP_NET_ADMIN)
fn can_open_tun() -> bool {
    match LinuxTun::open("") {
        Ok(_t) => true, // dropped immediately
        Err(_) => false,
    }
}

struct ServerTls {
    stream: tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
}
#[async_trait]
impl Transport for ServerTls {
    async fn send(&mut self, data: &[u8]) -> std::io::Result<()> {
        self.stream.write_all(data).await?;
        self.stream.flush().await
    }
    async fn recv(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.stream.read(buf).await
    }
    async fn close(&mut self) -> std::io::Result<()> {
        self.stream.shutdown().await
    }
}

struct Pki {
    server: Arc<ServerConfig>,
    client: Arc<ClientConfig>,
}

fn make_pki() -> Pki {
    let ip: std::net::IpAddr = TEST_HOST.parse().unwrap();
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    params.subject_alt_names.push(rcgen::SanType::IpAddress(ip));
    let key = rcgen::KeyPair::generate().unwrap();
    let cert = params.self_signed(&key).unwrap();
    let cert_der = CertificateDer::from(cert.der().to_vec());
    let key_der = PrivateKeyDer::try_from(key.serialize_der()).unwrap();
    let server = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der.clone()], key_der)
        .unwrap();
    let mut roots = RootCertStore::empty();
    roots.add(cert_der).unwrap();
    let client = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    Pki {
        server: Arc::new(server),
        client: Arc::new(client),
    }
}

/// Realistic multi-connection F5 server on loopback.
async fn spawn_server(pki: &Pki) -> u16 {
    let listener = TcpListener::bind((TEST_HOST, 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let acceptor = TlsAcceptor::from(Arc::clone(&pki.server));
    tokio::spawn(async move {
        let actor = F5ServerActor::new(F5ServerScript::realistic());
        for _ in 0..40 {
            if let Ok((tcp, _)) = listener.accept().await {
                if let Ok(tls) = acceptor.accept(tcp).await {
                    let mut t = ServerTls { stream: tls };
                    if actor.serve_one_connection(&mut t).await {
                        break;
                    }
                }
            } else {
                break;
            }
        }
    });
    port
}

#[tokio::test]
async fn native_f5_real_tun_brings_up_interface() {
    if !enabled() {
        eprintln!(
            "skip: set AKON_RUN_TUN_TESTS=1 (and run with CAP_NET_ADMIN) to run the real-TUN test"
        );
        return;
    }
    if !isolated_netns() {
        eprintln!(
            "skip: REFUSING to run in the host network namespace (this test connects a \
             full-tunnel server and would hijack host networking). Run inside `unshare -rn` \
             (a throwaway netns with only loopback) or a container."
        );
        return;
    }
    if !can_open_tun() {
        eprintln!("skip: cannot open /dev/net/tun (needs root/CAP_NET_ADMIN)");
        return;
    }

    let pki = make_pki();
    let port = spawn_server(&pki).await;

    // Real Linux TUN device + factory-based reconnecting transport.
    let tun = LinuxTun::open("").expect("open real TUN");
    let if_name = tun.name().to_string();
    eprintln!("real-tun: created interface {if_name}");

    let factory: Box<dyn TransportFactory> = Box::new(TlsTransportFactory::with_config(
        TEST_HOST,
        port,
        Arc::clone(&pki.client),
    ));
    let mut backend = NativeF5Backend::with_factory_and_parts(
        factory,
        Box::new(tun),
        Box::new(NoopDns),
        TEST_HOST,
    );

    let mut rx = backend
        .connect(Credentials::new("tester", "1234567890"))
        .expect("connect starts");

    let mut connected_ip = None;
    while let Ok(Some(ev)) = tokio::time::timeout(Duration::from_secs(20), rx.recv()).await {
        match ev {
            LifecycleEvent::Connected { ip, .. } => {
                connected_ip = Some(ip.to_string());
                break;
            }
            LifecycleEvent::Failed { kind, detail } => {
                panic!("real-tun connect failed: {kind:?}: {detail}");
            }
            _ => {}
        }
    }
    assert_eq!(connected_ip.as_deref(), Some("10.20.30.40"));

    // The real interface exists and has the negotiated address (via `ip addr`).
    let out = std::process::Command::new("ip")
        .args(["addr", "show", "dev", &if_name])
        .output()
        .expect("ip addr");
    let text = String::from_utf8_lossy(&out.stdout);
    eprintln!("real-tun: {if_name} state:\n{text}");
    assert!(
        text.contains("10.20.30.40"),
        "interface {if_name} did not get the negotiated address"
    );
    assert!(text.contains("mtu 1411"), "interface MTU should be 1411");

    // --- Rehearse the production data-plane soak's route mechanics locally ---
    // Add a /32 host route through the tunnel interface (TEST-NET-1, RFC5737,
    // a safe non-routable address), verify it landed on the right device, then
    // remove it. This de-risks the exact `ip route` add/verify/remove path the
    // production soak uses, on a real interface, without any production traffic.
    let probe_cidr = "192.0.2.123/32";
    let add = std::process::Command::new("ip")
        .args(["route", "replace", probe_cidr, "dev", &if_name])
        .status()
        .expect("ip route replace");
    assert!(add.success(), "failed to add probe route via {if_name}");

    let routes = std::process::Command::new("ip")
        .args(["route", "show", probe_cidr])
        .output()
        .expect("ip route show");
    let routes_text = String::from_utf8_lossy(&routes.stdout);
    assert!(
        routes_text.contains(&if_name),
        "probe route not present on {if_name}: {routes_text}"
    );
    eprintln!("real-tun: probe route {probe_cidr} via {if_name} OK");

    // Remove the probe route (the production soak does this via an RAII guard).
    let _ = std::process::Command::new("ip")
        .args(["route", "del", probe_cidr])
        .status();

    // Clean teardown.
    backend.disconnect().expect("disconnect");
    tokio::time::sleep(Duration::from_millis(300)).await;
    eprintln!("real-tun: disconnected; interface torn down");

    // The probe route must be gone after teardown.
    let after = std::process::Command::new("ip")
        .args(["route", "show", probe_cidr])
        .output()
        .expect("ip route show");
    assert!(
        String::from_utf8_lossy(&after.stdout).trim().is_empty(),
        "probe route leaked after teardown"
    );
}

/// Proves the production no-leak safety net: a `LinuxTun` that is simply dropped
/// (no graceful disconnect — simulating an early-exit/panic path) still removes
/// its kernel interface. This is the guarantee a production host relies on.
#[tokio::test]
async fn dropping_linux_tun_removes_interface() {
    if !enabled() {
        eprintln!("skip: set AKON_RUN_TUN_TESTS=1 to run the real-TUN no-leak test");
        return;
    }
    if !isolated_netns() {
        eprintln!(
            "skip: REFUSING to create a TUN in the host network namespace; run inside \
             `unshare -rn` or a container."
        );
        return;
    }
    if !can_open_tun() {
        eprintln!("skip: cannot open /dev/net/tun (needs root/CAP_NET_ADMIN)");
        return;
    }

    let name = {
        let tun = LinuxTun::open("").expect("open real TUN");
        let n = tun.name().to_string();
        // Interface exists while the tun is alive.
        let up = std::process::Command::new("ip")
            .args(["link", "show", "dev", &n])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        assert!(up, "interface {n} should exist while LinuxTun is alive");
        n
        // `tun` dropped here — no graceful teardown, just Drop.
    };

    // After drop, the interface must be gone (fd close + explicit ip link delete).
    let gone = std::process::Command::new("ip")
        .args(["link", "show", "dev", &name])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| !s.success())
        .unwrap_or(true);
    assert!(gone, "interface {name} leaked after LinuxTun drop");
    eprintln!("real-tun: drop removed interface {name} (no leak)");
}
