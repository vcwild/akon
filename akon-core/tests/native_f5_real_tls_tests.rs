//! REAL end-to-end test for the native F5 backend over a genuine TLS-over-TCP
//! connection.
//!
//! Unlike `native_f5_backend_tests.rs` (which uses an in-memory transport), this
//! drives [`NativeF5Backend`] through its **production** [`TlsTransport`] against
//! a **real** local TLS server (real `TcpListener` + rustls handshake) that runs
//! the [`F5ServerActor`] protocol logic. This is the test that acknowledges the
//! openconnect replacement: it exercises the actual socket I/O path — real TLS
//! records, coalesced reads, real handshake — not an emulation of it.
//!
//! It uses a self-signed certificate trusted only by this test's client config,
//! so it needs no external server, no root, and does not touch the host network
//! beyond loopback. Every wait is bounded, so it cannot hang.
#![cfg(feature = "test-actors")]

use std::sync::Arc;
use std::time::Duration;

use akon_core::vpn::backend::{Credentials, LifecycleEvent, VpnBackend};
use akon_core::vpn::f5::tls_transport::TlsTransport;
use akon_core::vpn::f5::NativeF5Backend;
use akon_core::vpn::testkit::f5_server_actor::{F5ServerActor, F5ServerScript};
use akon_core::vpn::transport::Transport;
use async_trait::async_trait;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::rustls::{ClientConfig, RootCertStore, ServerConfig};
use tokio_rustls::TlsAcceptor;

/// Adapter so the server side of a real TLS stream satisfies the `Transport`
/// trait, letting the existing `F5ServerActor` drive it unchanged.
struct ServerTlsTransport {
    stream: tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
}

#[async_trait]
impl Transport for ServerTlsTransport {
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

/// A self-signed cert + key plus a client config that trusts it.
struct TestPki {
    server_config: Arc<ServerConfig>,
    client_config: Arc<ClientConfig>,
}

fn make_pki(ip_literal: &str) -> TestPki {
    // Generate a self-signed certificate with an IP-address SAN matching the
    // loopback literal we dial, so the TCP destination and TLS server name agree.
    use std::net::IpAddr;
    let ip: IpAddr = ip_literal.parse().expect("valid IP literal");
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).expect("cert params");
    params.subject_alt_names.push(rcgen::SanType::IpAddress(ip));
    let key_pair = rcgen::KeyPair::generate().expect("keypair");
    let cert = params.self_signed(&key_pair).expect("self-signed cert");

    let cert_der = CertificateDer::from(cert.der().to_vec());
    let key_der = PrivateKeyDer::try_from(key_pair.serialize_der()).expect("serialize private key");

    let server_config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der.clone()], key_der)
        .expect("server config");

    // Client trusts exactly this cert.
    let mut roots = RootCertStore::empty();
    roots.add(cert_der).expect("add root");
    let client_config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();

    TestPki {
        server_config: Arc::new(server_config),
        client_config: Arc::new(client_config),
    }
}

/// Start a real TLS server on loopback that serves one F5 session, returning the
/// bound port. The server runs the `F5ServerActor` over the accepted TLS stream.
async fn spawn_real_f5_server(pki: &TestPki, script: F5ServerScript) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let acceptor = TlsAcceptor::from(Arc::clone(&pki.server_config));

    tokio::spawn(async move {
        if let Ok((tcp, _)) = listener.accept().await {
            if let Ok(tls) = acceptor.accept(tcp).await {
                let mut transport = ServerTlsTransport { stream: tls };
                F5ServerActor::new(script).run(&mut transport).await;
            }
        }
    });

    port
}

async fn collect(
    mut rx: tokio::sync::mpsc::UnboundedReceiver<LifecycleEvent>,
) -> Vec<LifecycleEvent> {
    let mut events = Vec::new();
    loop {
        match tokio::time::timeout(Duration::from_secs(10), rx.recv()).await {
            Ok(Some(e)) => {
                let terminal = matches!(
                    e,
                    LifecycleEvent::Connected { .. }
                        | LifecycleEvent::Failed { .. }
                        | LifecycleEvent::Disconnected { .. }
                );
                events.push(e);
                if terminal {
                    break;
                }
            }
            Ok(None) => break,
            Err(_) => break, // bounded: never hangs
        }
    }
    events
}

/// We sign the cert for the loopback IP and dial the same literal, so the TCP
/// destination and the TLS server name match (rustls supports IP server names).
const TEST_HOST: &str = "127.0.0.1";

#[tokio::test]
async fn native_f5_connects_over_real_tls() {
    let pki = make_pki(TEST_HOST);
    let port = spawn_real_f5_server(&pki, F5ServerScript::default()).await;

    // Connect the production TLS transport to the real local server, trusting
    // the test cert via the client config seam.
    let transport =
        TlsTransport::connect_with_config(TEST_HOST, port, Arc::clone(&pki.client_config))
            .await
            .expect("real TLS connect");

    let mut backend = NativeF5Backend::with_transport(Box::new(transport), TEST_HOST);
    let rx = backend
        .connect(Credentials::new("alice", "pin123456"))
        .expect("connect starts");
    let events = collect(rx).await;

    let labels: Vec<&str> = events.iter().map(|e| e.label()).collect();
    assert!(
        labels.contains(&"Connected"),
        "native F5 did not reach Connected over real TLS: {:?}",
        labels
    );
    let ip = events.iter().find_map(|e| match e {
        LifecycleEvent::Connected { ip, .. } => Some(ip.to_string()),
        _ => None,
    });
    assert_eq!(ip.as_deref(), Some("10.20.30.40"));
    assert!(backend.is_alive());
}

#[tokio::test]
async fn native_f5_auth_failure_over_real_tls() {
    let pki = make_pki(TEST_HOST);
    let port = spawn_real_f5_server(&pki, F5ServerScript::auth_failure()).await;

    let transport =
        TlsTransport::connect_with_config(TEST_HOST, port, Arc::clone(&pki.client_config))
            .await
            .expect("real TLS connect");
    let mut backend = NativeF5Backend::with_transport(Box::new(transport), TEST_HOST);
    let events = collect(
        backend
            .connect(Credentials::new("alice", "wrong"))
            .expect("connect starts"),
    )
    .await;

    let labels: Vec<&str> = events.iter().map(|e| e.label()).collect();
    assert!(
        !labels.contains(&"Connected"),
        "should not connect: {:?}",
        labels
    );
    assert!(labels.contains(&"Failed"), "expected failure: {:?}", labels);
    assert!(!backend.is_alive());
}

/// Start a **realistic** F5 server: it closes the connection after every HTTP
/// response (`Connection: close`), redirects the initial `GET /`, and sets an
/// intermediate cookie — exactly the behaviors a real F5 frontend exhibits
/// (and which broke the naive single-connection client). Accepts many
/// connections until the tunnel session completes.
async fn spawn_realistic_f5_server(pki: &TestPki) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let acceptor = TlsAcceptor::from(Arc::clone(&pki.server_config));

    tokio::spawn(async move {
        let actor = F5ServerActor::new(F5ServerScript::realistic());
        // Serve connection-by-connection until the session completes (or we hit
        // a safety cap so the test can never hang).
        for _ in 0..40 {
            match listener.accept().await {
                Ok((tcp, _)) => {
                    if let Ok(tls) = acceptor.accept(tcp).await {
                        let mut transport = ServerTlsTransport { stream: tls };
                        let done = actor.serve_one_connection(&mut transport).await;
                        if done {
                            break;
                        }
                    }
                }
                Err(_) => break,
            }
        }
    });

    port
}

/// THE KEY REGRESSION TEST for the real-appliance failure: the native backend
/// must complete the full handshake against a server that closes the connection
/// between requests and uses a redirect + intermediate cookie. This reproduces
/// the production `peer closed connection` failure offline and proves the
/// reconnecting HTTP client + redirect/cookie handling fix it.
#[tokio::test]
async fn native_f5_connects_against_realistic_closing_server() {
    use akon_core::vpn::f5::tls_transport::TlsTransportFactory;
    use akon_core::vpn::f5::NativeF5Backend;
    use akon_core::vpn::transport::{NoopTun, TransportFactory};

    let pki = make_pki(TEST_HOST);
    let port = spawn_realistic_f5_server(&pki).await;

    // Build a backend whose HTTP phase reconnects via a factory (production path).
    let factory: Box<dyn TransportFactory> = Box::new(TlsTransportFactory::with_config(
        TEST_HOST,
        port,
        Arc::clone(&pki.client_config),
    ));
    let mut backend = NativeF5Backend::with_factory_and_parts(
        factory,
        Box::new(NoopTun::default()),
        Box::new(akon_core::vpn::f5::dns::NoopDns),
        TEST_HOST,
    );

    let events = collect(
        backend
            .connect(Credentials::new("alice", "pin123456"))
            .expect("connect starts"),
    )
    .await;

    let labels: Vec<&str> = events.iter().map(|e| e.label()).collect();
    assert!(
        labels.contains(&"Connected"),
        "native F5 did not reach Connected against a realistic closing server: {:?}",
        labels
    );
    let ip = events.iter().find_map(|e| match e {
        LifecycleEvent::Connected { ip, .. } => Some(ip.to_string()),
        _ => None,
    });
    assert_eq!(ip.as_deref(), Some("10.20.30.40"));
}
