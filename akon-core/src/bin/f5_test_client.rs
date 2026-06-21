//! Standalone native-F5 client — run **inside** a Fedora/Ubuntu container to
//! validate the native backend (and especially the distro-specific DNS
//! application) against real distro userland, with no host side effects.
//!
//! It:
//! 1. Connects the native F5 backend to the F5 test server over real TLS
//!    (trusting the server cert at `AKON_F5_CA`), driving to `Connected`.
//! 2. Exercises the real [`akon_core::vpn::f5::dns::SystemDnsApplier`] against
//!    the container's resolver (systemd-resolved/`resolvectl` on Fedora/Ubuntu,
//!    with `resolvconf`/`/etc/resolv.conf` fallbacks), printing the detected
//!    backend and applying a sample DNS config to a dummy interface.
//!
//! Prints `RESULT: ok backend=<dns-backend>` on success and exits 0; prints
//! `RESULT: fail ...` and exits non-zero otherwise. Only compiled with the
//! `test-actors` feature.
//!
//! Env vars:
//! - `AKON_F5_HOST`  — F5 server host (default `f5server`)
//! - `AKON_F5_PORT`  — F5 server TLS port (default `8443`)
//! - `AKON_F5_CA`    — path to the server cert PEM to trust (default `/certs/server.pem`)
//! - `AKON_DNS_IFACE`— interface name to apply DNS to (default `lo`)

use std::sync::Arc;
use std::time::Duration;

use akon_core::vpn::backend::{Credentials, LifecycleEvent, VpnBackend};
use akon_core::vpn::f5::dns::{DnsApplier, SystemDnsApplier};
use akon_core::vpn::f5::tls_transport::TlsTransport;
use akon_core::vpn::f5::NativeF5Backend;
use akon_core::vpn::transport::TunConfig;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn client_config_trusting(ca_path: &str) -> Result<Arc<ClientConfig>, String> {
    let pem = std::fs::read(ca_path).map_err(|e| format!("read CA {ca_path}: {e}"))?;
    let mut reader = std::io::BufReader::new(&pem[..]);
    let mut roots = RootCertStore::empty();
    for item in rustls_pemfile::certs(&mut reader).flatten() {
        let _ = roots.add(item);
    }
    Ok(Arc::new(
        ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    ))
}

#[tokio::main]
async fn main() {
    match run().await {
        Ok(backend) => {
            println!("RESULT: ok backend={backend:?}");
            std::process::exit(0);
        }
        Err(e) => {
            println!("RESULT: fail {e}");
            std::process::exit(1);
        }
    }
}

async fn run() -> Result<akon_core::vpn::f5::dns::DnsBackend, String> {
    let host = env_or("AKON_F5_HOST", "f5server");
    let port: u16 = env_or("AKON_F5_PORT", "8443").parse().unwrap_or(8443);
    let ca = env_or("AKON_F5_CA", "/certs/server.pem");
    let iface = env_or("AKON_DNS_IFACE", "lo");

    // --- 1. Connect over real TLS to the F5 server and reach Connected ---
    let config = client_config_trusting(&ca)?;
    let transport = TlsTransport::connect_with_config(&host, port, config)
        .await
        .map_err(|e| format!("TLS connect {host}:{port}: {e}"))?;

    let mut backend = NativeF5Backend::with_transport(Box::new(transport), host.clone());
    let mut rx = backend
        .connect(Credentials::new("testuser", "1234567890"))
        .map_err(|e| format!("connect start: {e}"))?;

    let mut connected = false;
    while let Ok(Some(ev)) = tokio::time::timeout(Duration::from_secs(20), rx.recv()).await {
        match ev {
            LifecycleEvent::Connected { ip, .. } => {
                eprintln!("client: connected, assigned ip {ip}");
                connected = true;
                break;
            }
            LifecycleEvent::Failed { kind, detail } => {
                return Err(format!("connection failed: {kind:?}: {detail}"));
            }
            _ => {}
        }
    }
    if !connected {
        return Err("did not reach Connected".to_string());
    }

    // --- 2. Exercise the real distro DNS applier ---
    let mut dns = SystemDnsApplier::detect();
    let backend_kind = dns.backend();
    eprintln!("client: detected DNS backend = {backend_kind:?}");

    let dns_config = TunConfig {
        ipv4: Some("10.20.30.40".into()),
        mtu: Some(1400),
        dns: vec!["10.20.30.53".into()],
        domains: vec!["corp.example.com".into()],
        routes: vec![],
        ..Default::default()
    };

    dns.apply(&iface, &dns_config)
        .map_err(|e| format!("dns apply on {iface}: {e}"))?;
    eprintln!("client: DNS applied on {iface}");

    // Best-effort revert so we leave the container resolver as we found it.
    let _ = dns.revert(&iface);

    Ok(backend_kind)
}
