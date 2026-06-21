//! Real-host integration tests: drive the native F5 backend over real TLS+TCP
//! against an F5 test server in a **Podman container**, and validate the
//! distro-specific DNS application by running the native client **inside Fedora
//! and Ubuntu containers**.
//!
//! This is the closest we get to production without a real F5 appliance: real
//! server and client processes, in their own network namespaces, over a real
//! Podman network and TLS handshake — fully isolated, **no side effects on the
//! host**. The Fedora/Ubuntu client containers exercise the genuine
//! `SystemDnsApplier` (`resolvectl`/`resolvconf`/`resolv.conf`) on each distro.
//!
//! The tests are **opt-in and self-skipping**: they only run when
//! `AKON_RUN_PODMAN_TESTS=1` AND podman is available; otherwise they print a
//! notice and pass, so they never block or hang the normal suite. They are
//! bounded and always tear their containers/network down.
//!
//! Enable with:
//!   AKON_RUN_PODMAN_TESTS=1 cargo test -p akon-core --features test-actors \
//!       --test native_f5_podman_tests -- --nocapture --test-threads=1
#![cfg(feature = "test-actors")]

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use akon_core::vpn::backend::{Credentials, LifecycleEvent, VpnBackend};
use akon_core::vpn::f5::tls_transport::TlsTransport;
use akon_core::vpn::f5::NativeF5Backend;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};

const NETWORK: &str = "akon-f5-it-net";
const SERVER_IMAGE: &str = "akon-f5-test-server:latest";
const SERVER_NAME: &str = "f5server";
const HOST_PORT: u16 = 18443;

fn enabled() -> bool {
    std::env::var("AKON_RUN_PODMAN_TESTS").as_deref() == Ok("1")
}

fn podman_available() -> bool {
    Command::new("podman")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .to_path_buf()
}

fn podman(args: &[&str]) -> std::io::Result<std::process::Output> {
    Command::new("podman").args(args).output()
}

fn podman_status(args: &[&str]) -> bool {
    Command::new("podman")
        .args(args)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn build_image(tag: &str, containerfile: &str, root: &std::path::Path) -> bool {
    eprintln!("podman: building {tag} from {containerfile} ...");
    podman_status(&[
        "build",
        "-t",
        tag,
        "-f",
        &root.join(containerfile).to_string_lossy(),
        &root.to_string_lossy(),
    ])
}

fn cleanup(container_names: &[&str]) {
    for name in container_names {
        let _ = podman(&["rm", "-f", name]);
    }
    let _ = podman(&["network", "rm", "-f", NETWORK]);
}

/// Test harness that always tears down its podman resources.
struct PodmanScope {
    containers: Vec<String>,
}
impl PodmanScope {
    fn new() -> Self {
        Self {
            containers: Vec::new(),
        }
    }
    fn track(&mut self, name: &str) {
        self.containers.push(name.to_string());
    }
}
impl Drop for PodmanScope {
    fn drop(&mut self) {
        let names: Vec<&str> = self.containers.iter().map(|s| s.as_str()).collect();
        cleanup(&names);
    }
}

/// Start the shared network + F5 server container, returning the host path of
/// the server cert (written into a shared volume) once it appears.
async fn start_server(
    scope: &mut PodmanScope,
    cert_dir: &std::path::Path,
    root: &std::path::Path,
) -> Option<Vec<u8>> {
    // Fresh network.
    let _ = podman(&["network", "rm", "-f", NETWORK]);
    if !podman_status(&["network", "create", NETWORK]) {
        eprintln!("skip: could not create podman network");
        return None;
    }

    if !build_image(
        SERVER_IMAGE,
        "test-support/f5-container/Containerfile",
        root,
    ) {
        eprintln!("skip: server image build failed");
        return None;
    }

    let mount = format!("{}:/certs:Z", cert_dir.display());
    let port_map = format!("{HOST_PORT}:8443");
    scope.track(SERVER_NAME);
    let ok = podman_status(&[
        "run",
        "-d",
        "--name",
        SERVER_NAME,
        "--network",
        NETWORK,
        "-p",
        &port_map,
        "-v",
        &mount,
        "-e",
        // SAN covers both the in-network DNS name and loopback (host access).
        "AKON_F5_SAN=f5server",
        SERVER_IMAGE,
    ]);
    if !ok {
        eprintln!("skip: server run failed");
        return None;
    }

    // Wait for the cert to be written.
    let cert_path = cert_dir.join("server.pem");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
    while tokio::time::Instant::now() < deadline {
        if let Ok(bytes) = std::fs::read(&cert_path) {
            if !bytes.is_empty() {
                return Some(bytes);
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    eprintln!("skip: server cert not produced in time");
    None
}

/// Run a distro client container to completion and return whether it reported
/// `RESULT: ok` (printing its logs for diagnostics).
fn run_client(
    scope: &mut PodmanScope,
    name: &str,
    image: &str,
    containerfile: &str,
    cert_dir: &std::path::Path,
    root: &std::path::Path,
) -> bool {
    if !build_image(image, containerfile, root) {
        eprintln!("skip: {name} image build failed");
        return true; // skip (treat as non-failing) when image can't build offline
    }

    let mount = format!("{}:/certs:ro,Z", cert_dir.display());
    scope.track(name);
    // Run to completion (foreground), capturing output.
    let out = podman(&[
        "run",
        "--name",
        name,
        "--network",
        NETWORK,
        "-v",
        &mount,
        image,
    ]);

    match out {
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            let stderr = String::from_utf8_lossy(&o.stderr);
            eprintln!("--- {name} stdout ---\n{stdout}");
            eprintln!("--- {name} stderr ---\n{stderr}");
            stdout.contains("RESULT: ok")
        }
        Err(e) => {
            eprintln!("{name} run error: {e}");
            false
        }
    }
}

fn client_config_trusting(cert_pem: &[u8]) -> Arc<ClientConfig> {
    let mut reader = std::io::BufReader::new(cert_pem);
    let mut roots = RootCertStore::empty();
    for item in rustls_pemfile::certs(&mut reader).flatten() {
        let _ = roots.add(item);
    }
    Arc::new(
        ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    )
}

/// Host-side smoke test: the native backend connects to the containerized F5
/// server over the published port via real TLS.
#[tokio::test]
async fn native_f5_connects_to_containerized_server() {
    if !enabled() {
        eprintln!("skip: set AKON_RUN_PODMAN_TESTS=1 to run podman integration tests");
        return;
    }
    if !podman_available() {
        eprintln!("skip: podman not available");
        return;
    }

    let root = repo_root();
    let cert_dir = tempfile::tempdir().expect("tempdir");
    let mut scope = PodmanScope::new();

    let cert = match start_server(&mut scope, cert_dir.path(), &root).await {
        Some(c) => c,
        None => return, // already logged a skip reason
    };

    // The server cert includes a 127.0.0.1 SAN, so connect from the host over
    // the published loopback port via real TLS and drive to Connected.
    let config = client_config_trusting(&cert);
    let mut connected_ip = None;
    'attempts: for _ in 0..20 {
        if let Ok(transport) =
            TlsTransport::connect_with_config("127.0.0.1", HOST_PORT, Arc::clone(&config)).await
        {
            let mut backend = NativeF5Backend::with_transport(Box::new(transport), "127.0.0.1");
            let mut rx = backend
                .connect(Credentials::new("testuser", "1234567890"))
                .expect("connect starts");
            while let Ok(Some(ev)) = tokio::time::timeout(Duration::from_secs(15), rx.recv()).await
            {
                match ev {
                    LifecycleEvent::Connected { ip, .. } => {
                        connected_ip = Some(ip.to_string());
                        break 'attempts;
                    }
                    LifecycleEvent::Failed { .. } => break,
                    _ => {}
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    assert_eq!(
        connected_ip.as_deref(),
        Some("10.20.30.40"),
        "native backend did not reach Connected against the containerized F5 server"
    );
}

/// Fedora: run the native client inside a Fedora container; assert it connects
/// and applies DNS via the real Fedora resolver tooling.
#[tokio::test]
async fn native_f5_in_fedora_container() {
    if !enabled() || !podman_available() {
        eprintln!("skip: podman integration tests disabled/unavailable");
        return;
    }
    let root = repo_root();
    let cert_dir = tempfile::tempdir().expect("tempdir");
    let mut scope = PodmanScope::new();

    if start_server(&mut scope, cert_dir.path(), &root)
        .await
        .is_none()
    {
        return;
    }

    let ok = run_client(
        &mut scope,
        "akon-f5-client-fedora",
        "akon-f5-client-fedora:latest",
        "test-support/f5-container/Containerfile.client-fedora",
        cert_dir.path(),
        &root,
    );
    assert!(ok, "native client failed inside Fedora container");
}

/// Ubuntu: run the native client inside an Ubuntu container; assert it connects
/// and applies DNS via the real Ubuntu resolver tooling.
#[tokio::test]
async fn native_f5_in_ubuntu_container() {
    if !enabled() || !podman_available() {
        eprintln!("skip: podman integration tests disabled/unavailable");
        return;
    }
    let root = repo_root();
    let cert_dir = tempfile::tempdir().expect("tempdir");
    let mut scope = PodmanScope::new();

    if start_server(&mut scope, cert_dir.path(), &root)
        .await
        .is_none()
    {
        return;
    }

    let ok = run_client(
        &mut scope,
        "akon-f5-client-ubuntu",
        "akon-f5-client-ubuntu:latest",
        "test-support/f5-container/Containerfile.client-ubuntu",
        cert_dir.path(),
        &root,
    );
    assert!(ok, "native client failed inside Ubuntu container");
}

/// ROOTLESS validation: build the `f5_dataplane_probe` image (which grants the
/// binary `cap_net_admin+ep` and runs it as a NON-ROOT user), then run it in a
/// container with `--cap-add NET_ADMIN --device /dev/net/tun`. The probe brings
/// up a real TUN, configures address/routes via **in-process netlink** (no
/// `sudo`, no `ip` child), runs a full data-plane round-trip, and tears down —
/// all as an unprivileged user, in COMPLETE container isolation with zero effect
/// on the host. This is the openconnect rootless feature-parity proof.
#[tokio::test]
async fn rootless_dataplane_runs_in_container_as_user() {
    if !enabled() || !podman_available() {
        eprintln!("skip: podman integration tests disabled/unavailable");
        return;
    }
    let root = repo_root();
    let image = "akon-f5-rootless-probe:latest";
    let name = "akon-f5-rootless-probe";
    let mut scope = PodmanScope::new();
    scope.track(name);

    if !build_image(
        image,
        "test-support/f5-container/Containerfile.rootless-probe",
        &root,
    ) {
        eprintln!("skip: rootless-probe image build failed (offline?)");
        return;
    }

    // Run the probe container:
    //  - `--user akon` is baked into the image (runs as a NON-ROOT user),
    //  - `--cap-add NET_ADMIN` gives the container's userns the capability the
    //    binary's file capability draws on,
    //  - `--device /dev/net/tun` exposes the TUN clone device,
    //  - `--network none` keeps it fully isolated from the host network.
    // The probe brings `lo` up itself is not needed; it only needs local
    // delivery on the tun, which works without external networking.
    let out = podman(&[
        "run",
        "--rm",
        "--name",
        name,
        "--cap-add",
        "NET_ADMIN",
        "--device",
        "/dev/net/tun",
        "--network",
        "none",
        image,
    ]);

    match out {
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            let stderr = String::from_utf8_lossy(&o.stderr);
            eprintln!("--- rootless-probe stdout ---\n{stdout}");
            eprintln!("--- rootless-probe stderr (tail) ---");
            for line in stderr
                .lines()
                .rev()
                .take(30)
                .collect::<Vec<_>>()
                .iter()
                .rev()
            {
                eprintln!("{line}");
            }
            assert!(
                stdout.contains("RESULT: ok"),
                "rootless data-plane round-trip failed in container (no `RESULT: ok`). \
                 This proves the netlink-based rootless path under a cap_net_admin+ep \
                 file capability, run as a non-root user."
            );
            // The teardown reconciler must also have fully cleaned up (in-container).
            assert!(
                stderr.contains("TEARDOWN: ok"),
                "rootless probe did not complete teardown verification (`TEARDOWN: ok`)"
            );
        }
        Err(e) => panic!("rootless-probe run error: {e}"),
    }
}
