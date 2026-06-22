//! `NativeF5Backend` — orchestrates the native F5 layers and implements
//! [`VpnBackend`].
//!
//! Flow (per openconnect `f5.c`, validated by the test actors framework):
//! 1. **Auth**: GET `/` → parse `auth_form` → POST `username`/`password` →
//!    collect `MRHSession` + `F5_ST` cookies.
//! 2. **Config**: GET profile XML → `<params>`; GET options XML → session id,
//!    `ur_Z`, ipv4/ipv6/hdlc, DNS, routes.
//! 3. **Tunnel upgrade**: GET `/myvpn?sess=&hdlc_framing=&ipv4=&ipv6=&Z=&hostname=`
//!    (no Cookie) → expect 200/201, read `X-VPN-client-IP`.
//! 4. **PPP**: run LCP then IPCP to "network up" using the negotiated IP/DNS.
//!
//! All socket I/O goes through the [`Transport`] seam, so the entire flow is
//! exercised offline against the fake F5 server actor.

use crate::vpn::backend::{
    BackendError, ConnectionHandle, Credentials, FailureKind, LifecycleEvent, VpnBackend,
};
use crate::vpn::f5::auth::{F5AuthForm, F5CookieJar};
use crate::vpn::f5::config::{parse_options, parse_profile, F5Options};
use crate::vpn::f5::dns::{DnsApplier, NoopDns};
use crate::vpn::f5::framing::{f5_decap, f5_encap};
use crate::vpn::f5::http::{send_request, HttpRequest, HttpResponse};
use crate::vpn::f5::ppp::{lcp_terminate_request, PppNegotiator, PppPhase};
use crate::vpn::f5::F5Error;
use crate::vpn::transport::{NoopTun, Transport, TransportFactory, TunConfig, TunDevice};
use data_encoding::BASE64;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::sync::Notify;

static HANDLE_SEQ: AtomicU64 = AtomicU64::new(5000);

/// Shared, observable state of a native F5 connection.
#[derive(Default)]
struct Shared {
    alive: bool,
    handle: Option<ConnectionHandle>,
    /// Cookie header + host captured during the handshake, needed for the
    /// HTTP logout during teardown.
    logout_cookie: Option<String>,
    /// Negotiated DNS servers (dotted), exposed via [`NativeF5Backend::negotiated_dns`]
    /// so callers can resolve VPN-only names through the tunnel.
    dns: Vec<String>,
    /// Host mutations made by the TUN's `configure`, exposed via
    /// [`NativeF5Backend::teardown_plan`] so the CLI can persist them for an
    /// out-of-process `akon vpn off`.
    teardown_plan: crate::vpn::f5::HostTeardownPlan,
}

/// Native, pure-Rust F5 BIG-IP SSL VPN backend.
///
/// Replaces the openconnect delegation for the F5 protocol. The transport is
/// injected (a TLS socket in production; the in-memory duplex in tests), which
/// is what makes the whole flow validatable by the test actors framework. A
/// [`TunDevice`] seam carries the user data plane (a real `/dev/net/tun` in
/// production; a fake in tests).
pub struct NativeF5Backend {
    transport: Option<Box<dyn Transport>>,
    /// Optional connection factory for the HTTP (auth/config) phase. When set,
    /// the handshake reconnects per request as the server closes connections
    /// (real F5 behaviour). When `None`, the single `transport` is reused for
    /// the whole exchange (in-memory test transport never closes mid-exchange).
    factory: Option<Box<dyn TransportFactory>>,
    tun: Option<Box<dyn TunDevice>>,
    dns: Option<Box<dyn DnsApplier>>,
    host: String,
    shared: Arc<Mutex<Shared>>,
    /// Signalled by [`disconnect`] to stop the data-plane pump and trigger
    /// graceful teardown.
    shutdown: Arc<Notify>,
}

impl NativeF5Backend {
    /// Create a backend over `transport` for `host`, with a no-op TUN device.
    ///
    /// The no-op TUN lets the full control plane (auth → config → tunnel → PPP →
    /// teardown) be tested without root; the data-plane pump runs but moves no
    /// real OS packets. Use [`with_transport_and_tun`](Self::with_transport_and_tun)
    /// to attach a real or fake TUN that actually carries packets.
    pub fn with_transport(transport: Box<dyn Transport>, host: impl Into<String>) -> Self {
        Self::with_transport_and_tun(transport, Box::new(NoopTun::default()), host)
    }

    /// Create a backend over `transport` and `tun` for `host` (no-op DNS).
    pub fn with_transport_and_tun(
        transport: Box<dyn Transport>,
        tun: Box<dyn TunDevice>,
        host: impl Into<String>,
    ) -> Self {
        Self::with_parts(transport, tun, Box::new(NoopDns), host)
    }

    /// Create a backend with explicit transport, TUN, and DNS applier.
    pub fn with_parts(
        transport: Box<dyn Transport>,
        tun: Box<dyn TunDevice>,
        dns: Box<dyn DnsApplier>,
        host: impl Into<String>,
    ) -> Self {
        Self {
            transport: Some(transport),
            factory: None,
            tun: Some(tun),
            dns: Some(dns),
            host: host.into(),
            shared: Arc::new(Mutex::new(Shared::default())),
            shutdown: Arc::new(Notify::new()),
        }
    }

    /// Create a backend whose HTTP (auth/config) phase reconnects via `factory`
    /// (so it survives servers that close the connection between requests), with
    /// the given TUN and DNS appliers.
    pub fn with_factory_and_parts(
        factory: Box<dyn TransportFactory>,
        tun: Box<dyn TunDevice>,
        dns: Box<dyn DnsApplier>,
        host: impl Into<String>,
    ) -> Self {
        Self {
            transport: None,
            factory: Some(factory),
            tun: Some(tun),
            dns: Some(dns),
            host: host.into(),
            shared: Arc::new(Mutex::new(Shared::default())),
            shutdown: Arc::new(Notify::new()),
        }
    }

    /// Build a production backend from a [`VpnConfig`]: connect a real TLS
    /// transport to the configured server (default port 443) and attach a real
    /// Linux TUN device. Linux-only; requires `CAP_NET_ADMIN` for the TUN.
    ///
    /// This is the constructor the CLI uses for `protocol = f5`.
    #[cfg(target_os = "linux")]
    pub async fn connect_from_config(
        config: &crate::config::VpnConfig,
    ) -> Result<Self, BackendError> {
        use crate::vpn::f5::tls_transport::TlsTransportFactory;
        use crate::vpn::f5::tun::LinuxTun;

        // Split "host" or "host:port" from the configured server.
        let (host, port) = split_host_port(&config.server, 443);

        // Validate connectivity eagerly so the caller gets an immediate error on
        // an unreachable/bad server; the handshake itself reconnects via factory.
        {
            use crate::vpn::f5::tls_transport::TlsTransport;
            let _probe = TlsTransport::connect(&host, port).await.map_err(|e| {
                BackendError::StartFailed(format!("TLS connect to {host}:{port}: {e}"))
            })?;
        }

        let factory = TlsTransportFactory::new(host.clone(), port);

        let tun = LinuxTun::open("")
            .map_err(|e| BackendError::StartFailed(format!("open TUN device: {e}")))?;

        let dns = crate::vpn::f5::dns::SystemDnsApplier::detect();

        Ok(Self::with_factory_and_parts(
            Box::new(factory),
            Box::new(tun),
            Box::new(dns),
            host,
        ))
    }

    /// Build a **control-plane-only** backend from a [`VpnConfig`]: connect a
    /// real TLS transport to the configured server, but attach a **no-op TUN and
    /// no-op DNS** so the full handshake (auth → config → tunnel upgrade → PPP →
    /// `Connected`) is validated against the real appliance **without taking over
    /// the host's networking** (no TUN device created, no routes, no DNS changes).
    ///
    /// This is the minimal, low-footprint path used by the production sign-off
    /// test: it proves end-to-end reachability and protocol correctness against
    /// the live server while leaving the developer's connectivity untouched. It
    /// needs no `CAP_NET_ADMIN` because it creates no TUN device.
    pub async fn connect_control_plane_only(
        config: &crate::config::VpnConfig,
    ) -> Result<Self, BackendError> {
        use crate::vpn::f5::tls_transport::{TlsTransport, TlsTransportFactory};
        use crate::vpn::transport::NoopTun;

        let (host, port) = split_host_port(&config.server, 443);

        // Eager connectivity probe for a fast, clear error on an unreachable host.
        {
            let _probe = TlsTransport::connect(&host, port).await.map_err(|e| {
                BackendError::StartFailed(format!("TLS connect to {host}:{port}: {e}"))
            })?;
        }

        let factory = TlsTransportFactory::new(host.clone(), port);

        Ok(Self::with_factory_and_parts(
            Box::new(factory),
            Box::new(NoopTun::default()),
            Box::new(NoopDns),
            host,
        ))
    }

    /// The DNS servers negotiated for the tunnel (dotted IPv4), available after
    /// the connection reaches `Connected`. Lets callers resolve VPN-only names
    /// through the tunnel.
    pub fn negotiated_dns(&self) -> Vec<String> {
        self.shared.lock().expect("poisoned").dns.clone()
    }

    /// The host-teardown plan recording every networking mutation made to bring
    /// up the tunnel (tun device, server-pin route, rp_filter originals, DNS
    /// interface). Available once the connection reaches `Connected`. Persist it
    /// (e.g. to the VPN state file) so `akon vpn off` can fully restore the host
    /// even if this process is later killed. See
    /// [`crate::vpn::f5::teardown::teardown_host`].
    pub fn teardown_plan(&self) -> crate::vpn::f5::HostTeardownPlan {
        self.shared.lock().expect("poisoned").teardown_plan.clone()
    }
}

/// Resolve a host (or `host:port`) to its first IPv4 address (dotted string).
/// Returns `None` if it can't be resolved.
fn resolve_host_ipv4(host: &str) -> Option<String> {
    use std::net::ToSocketAddrs;
    let (h, _) = split_host_port(host, 443);
    if let Ok(ip) = h.parse::<std::net::Ipv4Addr>() {
        return Some(ip.to_string());
    }
    (h.as_str(), 443u16)
        .to_socket_addrs()
        .ok()?
        .find_map(|sa| match sa.ip() {
            std::net::IpAddr::V4(v4) => Some(v4.to_string()),
            _ => None,
        })
}

/// Split a `host` or `host:port` string, applying `default_port` when absent.
fn split_host_port(server: &str, default_port: u16) -> (String, u16) {
    // Strip a leading scheme if present.
    let s = server
        .strip_prefix("https://")
        .or_else(|| server.strip_prefix("http://"))
        .unwrap_or(server);
    // Strip any trailing path.
    let s = s.split('/').next().unwrap_or(s);
    if let Some((h, p)) = s.rsplit_once(':') {
        if let Ok(port) = p.parse::<u16>() {
            return (h.to_string(), port);
        }
    }
    (s.to_string(), default_port)
}

impl VpnBackend for NativeF5Backend {
    fn connect(
        &mut self,
        credentials: Credentials,
    ) -> Result<UnboundedReceiver<LifecycleEvent>, BackendError> {
        let initial_transport = self.transport.take();
        let factory = self.factory.take();
        if initial_transport.is_none() && factory.is_none() {
            return Err(BackendError::StartFailed(
                "no transport or factory available".into(),
            ));
        }
        let mut tun = self
            .tun
            .take()
            .ok_or_else(|| BackendError::StartFailed("tun already consumed".into()))?;
        let mut dns = self
            .dns
            .take()
            .ok_or_else(|| BackendError::StartFailed("dns already consumed".into()))?;
        let host = self.host.clone();
        // The actual OS interface name (kernel-assigned for the real TUN).
        let device = tun.name();
        let shared = Arc::clone(&self.shared);
        let shutdown = Arc::clone(&self.shutdown);
        let (tx, rx) = mpsc::unbounded_channel();

        tokio::spawn(async move {
            let _ = tx.send(LifecycleEvent::Connecting);

            // The HTTP phase uses a connection manager that reconnects when the
            // server closes the connection between requests (real F5 behaviour).
            let mut conn = HttpConn::new(initial_transport, factory);

            // Bound only the handshake so a misbehaving peer can't hang setup.
            let handshake = tokio::time::timeout(
                Duration::from_secs(20),
                run_handshake(&mut conn, &host, &device, &credentials, &tx, &shared),
            )
            .await;

            let session = match handshake {
                Ok(Ok(session)) => session,
                Ok(Err(e)) => {
                    let _ = tx.send(failure_event(&e));
                    shared.lock().expect("poisoned").alive = false;
                    return;
                }
                Err(_) => {
                    let _ = tx.send(LifecycleEvent::Failed {
                        kind: FailureKind::Network,
                        detail: "handshake timed out".into(),
                    });
                    shared.lock().expect("poisoned").alive = false;
                    return;
                }
            };

            // The tunnel transport is the connection left open after `/myvpn`.
            let mut transport = match conn.into_transport() {
                Some(t) => t,
                None => {
                    let _ = tx.send(LifecycleEvent::Failed {
                        kind: FailureKind::Network,
                        detail: "no tunnel transport after handshake".into(),
                    });
                    shared.lock().expect("poisoned").alive = false;
                    return;
                }
            };

            // --- Data plane: pump packets until disconnect or transport EOF ---
            let reason = run_data_plane(
                transport.as_mut(),
                tun.as_mut(),
                dns.as_mut(),
                &session,
                &tx,
                &shared,
                &shutdown,
            )
            .await;

            // --- Teardown: PPP Terminate-Request + HTTP logout + close ---
            graceful_teardown(transport.as_mut(), &host, &session).await;

            shared.lock().expect("poisoned").alive = false;
            // Emit the ACTUAL reason: UserRequested (we called disconnect) vs
            // ServerClosed (the tunnel dropped on its own). The supervisor uses
            // this to decide whether to reconnect.
            let _ = tx.send(LifecycleEvent::Disconnected { reason });
        });

        Ok(rx)
    }

    fn disconnect(&mut self) -> Result<(), BackendError> {
        // Signal the running session to stop pumping and tear down gracefully.
        // `notify_one` stores a permit if the pump is not currently parked on
        // `notified()` (e.g. mid-loop or still starting up), so the shutdown is
        // never lost — unlike `notify_waiters`, which only wakes current waiters.
        self.shutdown.notify_one();
        // Reflect intent immediately for observers; the session task clears the
        // handle once teardown completes.
        self.shared.lock().expect("poisoned").alive = false;
        Ok(())
    }

    fn is_alive(&self) -> bool {
        self.shared.lock().expect("poisoned").alive
    }

    fn handle(&self) -> Option<ConnectionHandle> {
        self.shared.lock().expect("poisoned").handle
    }
}

/// Map an [`F5Error`] to a terminal lifecycle failure.
fn failure_event(e: &F5Error) -> LifecycleEvent {
    let kind = match e {
        F5Error::AuthFailed(_) => FailureKind::Authentication,
        F5Error::TunnelUpgradeRejected(_)
        | F5Error::MalformedHttp(_)
        | F5Error::BadEncapMagic(_)
        | F5Error::TruncatedFrame { .. }
        | F5Error::HdlcFcsInvalid
        | F5Error::MalformedPpp(_) => FailureKind::Network,
        F5Error::InvalidConfig(_) => FailureKind::Backend,
    };
    LifecycleEvent::Failed {
        kind,
        detail: e.to_string(),
    }
}

/// Manages the HTTP-phase connection, reconnecting when the server closes it
/// between requests (real F5 frontends do this routinely).
///
/// `request` sends one HTTP request, transparently (re)connecting first if no
/// live connection is held. If the response says the server will close
/// (`wants_close`), the current connection is dropped so the next request opens
/// a fresh one. The connection that survives the final `/myvpn` request is the
/// tunnel transport, retrieved via [`HttpConn::into_transport`].
struct HttpConn {
    current: Option<Box<dyn Transport>>,
    factory: Option<Box<dyn TransportFactory>>,
}

impl HttpConn {
    fn new(
        initial: Option<Box<dyn Transport>>,
        factory: Option<Box<dyn TransportFactory>>,
    ) -> Self {
        Self {
            current: initial,
            factory,
        }
    }

    /// Ensure a live connection exists (reconnecting via the factory if needed).
    async fn ensure_connected(&mut self) -> Result<(), F5Error> {
        if self.current.is_some() {
            return Ok(());
        }
        let factory = self.factory.as_ref().ok_or_else(|| {
            F5Error::MalformedHttp("connection closed, no factory to reconnect".into())
        })?;
        let t = factory
            .connect()
            .await
            .map_err(|e| F5Error::MalformedHttp(format!("reconnect failed: {e}")))?;
        self.current = Some(t);
        Ok(())
    }

    /// Send a request, (re)connecting as needed and dropping the connection when
    /// the server signals close.
    async fn request(&mut self, req: &HttpRequest<'_>) -> Result<HttpResponse, F5Error> {
        self.ensure_connected().await?;
        let transport = self.current.as_mut().expect("connected");
        let result = send_request(transport.as_mut(), req).await;

        match result {
            Ok(resp) => {
                if resp.wants_close {
                    // Drop the connection; next request reconnects.
                    self.current = None;
                }
                Ok(resp)
            }
            Err(e) => {
                // The connection is unusable; drop it so a retry can reconnect.
                self.current = None;
                Err(e)
            }
        }
    }

    /// Take the open connection (the tunnel transport after `/myvpn`).
    fn into_transport(self) -> Option<Box<dyn Transport>> {
        self.current
    }
}

/// Run the F5 HTML-form authentication loop until both session cookies appear.
///
/// Mirrors openconnect `f5_obtain_cookie`: GET the login page, parse the
/// `<form>` (the first must be `auth_form`), fill username + password (akon's
/// pre-composed PIN+OTP — a single string that satisfies the common single-step
/// F5 login), POST `application/x-www-form-urlencoded` to the form action,
/// follow it, and re-check for `MRHSession` + `F5_ST`. Supports multi-step
/// servers (a second form gets the same submission). Bounded iterations so a
/// misbehaving server cannot loop forever.
async fn authenticate(
    conn: &mut HttpConn,
    host: &str,
    credentials: &Credentials,
) -> Result<String, F5Error> {
    let mut jar = F5CookieJar::new();
    // Current request path (no leading-slash assumptions; we keep it as a full
    // request-target string starting with `/`).
    let mut next_path = "/".to_string();
    let mut pending_post: Option<String> = None;
    // Number of HTML forms we have actually parsed (openconnect's form_order).
    let mut form_order = 0u32;

    // Generous bound: redirect chains + multi-step auth still terminate.
    for _step in 0..16 {
        // Build the request. Echo ALL accumulated cookies on every request
        // (openconnect re-sends the full Cookie header each time).
        let cookie_header = jar.cookie_header_all();
        let resp = if let Some(body) = pending_post.take() {
            let mut req = HttpRequest::post_form(&next_path, host, body);
            if let Some(ch) = &cookie_header {
                req = req.with_header("Cookie", ch);
            }
            conn.request(&req).await?
        } else {
            let mut req = HttpRequest::get(&next_path, host);
            if let Some(ch) = &cookie_header {
                req = req.with_header("Cookie", ch);
            }
            conn.request(&req).await?
        };

        // Harvest cookies from this response.
        for sc in resp.header_all("set-cookie") {
            jar.ingest_set_cookie(sc);
        }

        // Success = both MRHSession and F5_ST present.
        if jar.is_authenticated() {
            return jar
                .cookie_header()
                .ok_or_else(|| F5Error::AuthFailed("inconsistent cookie state".into()));
        }

        // Redirect: openconnect follows ANY non-200 response that carries a
        // Location header, converting the method to GET (HTTP_REDIRECT_TO_GET).
        if resp.status != 200 {
            if let Some(location) = resp.header("location") {
                next_path = resolve_target(&next_path, location);
                pending_post = None; // POST -> GET on redirect
                continue;
            }
        }

        // Otherwise parse the next form and submit it.
        let html = String::from_utf8_lossy(&resp.body);
        let form = match F5AuthForm::parse(&html) {
            Some(f) => f,
            None => {
                return Err(F5Error::AuthFailed(format!(
                    "no login form found (HTTP {}); the server may present a SAML/JS login \
                     not yet supported, or credentials are wrong",
                    resp.status
                )));
            }
        };
        form_order += 1;

        // openconnect: the FIRST parsed form must be `auth_form`.
        if form_order == 1 && !form.id.is_empty() && form.id != "auth_form" {
            return Err(F5Error::AuthFailed(format!(
                "unexpected first form id '{}' (expected 'auth_form') — likely not an F5 VPN",
                form.id
            )));
        }

        let body = form.build_submission(&credentials.username, &credentials.password);
        // POST to the form action (resolved against the current path), or the
        // same path if the form has no action.
        next_path = if form.action.is_empty() {
            next_path.clone()
        } else {
            resolve_target(&next_path, &form.action)
        };
        pending_post = Some(body);
    }

    Err(F5Error::AuthFailed(
        "authentication did not complete (no MRHSession/F5_ST after multiple steps)".into(),
    ))
}

/// Resolve a redirect/form-action `location` against the `current` request path
/// into a new request target (path + query). Mirrors openconnect's
/// `handle_redirect`:
/// - absolute `https://host/path` → the `/path` portion (same-host assumption;
///   a different host would need a reconnect, handled by the factory),
/// - absolute path `/foo` → used as-is,
/// - relative `foo` → resolved against the directory of the current path.
fn resolve_target(current: &str, location: &str) -> String {
    let loc = location.trim();
    if loc.is_empty() || loc.starts_with('#') {
        return current.to_string();
    }

    // Absolute URL: take the path component (drop scheme + authority).
    if let Some(rest) = loc
        .strip_prefix("https://")
        .or_else(|| loc.strip_prefix("http://"))
    {
        return match rest.find('/') {
            Some(i) => rest[i..].to_string(),
            None => "/".to_string(),
        };
    }

    // Absolute path.
    if loc.starts_with('/') {
        return loc.to_string();
    }

    // Relative path: resolve against the directory of the current path.
    // Strip the current query string first.
    let current_path = current.split('?').next().unwrap_or("/");
    match current_path.rfind('/') {
        Some(i) => format!("{}/{}", &current_path[..i], loc),
        None => format!("/{loc}"),
    }
}

/// The negotiated session state needed for the data plane and teardown.
struct Session {
    /// `Cookie` header value (`MRHSession=..; F5_ST=..`) for the logout request.
    cookie_header: String,
    /// PPP magic number (for LCP terminate framing).
    #[allow(dead_code)]
    magic: u32,
    /// The tunnel interface name (e.g. `tun0`).
    device: String,
    /// The assigned tunnel IP, parsed (for the `Connected`/`LinkUp` events).
    parsed_ip: std::net::IpAddr,
    /// Negotiated TUN configuration.
    tun_config: TunConfig,
}

/// Run the F5 control-plane handshake, emitting lifecycle events and returning
/// the [`Session`] needed to run the data plane and tear down.
async fn run_handshake(
    conn: &mut HttpConn,
    host: &str,
    device: &str,
    credentials: &Credentials,
    tx: &UnboundedSender<LifecycleEvent>,
    shared: &Arc<Mutex<Shared>>,
) -> Result<Session, F5Error> {
    // --- 1. Authenticate ---
    let _ = tx.send(LifecycleEvent::Authenticating);
    let cookie_header = authenticate(conn, host, credentials).await?;
    let _ = tx.send(LifecycleEvent::SessionEstablished);

    // --- 2. Fetch config ---
    let profile_resp = conn
        .request(
            &HttpRequest::get("/vdesk/vpn/index.php3?outform=xml&client_version=2.0", host)
                .with_header("Cookie", &cookie_header),
        )
        .await?;
    let params = parse_profile(&String::from_utf8_lossy(&profile_resp.body))?;

    let options_path = format!(
        "/vdesk/vpn/connect.php3?{}&outform=xml&client_version=2.0",
        params
    );
    let options_resp = conn
        .request(&HttpRequest::get(&options_path, host).with_header("Cookie", &cookie_header))
        .await?;
    let opts = parse_options(&String::from_utf8_lossy(&options_resp.body))?;

    // --- 3. Tunnel upgrade (no Cookie; auth via sess+Z query params) ---
    // This connection must stay OPEN for PPP, so we ensure a live connection and
    // send the request directly (not through `request`, which may drop on close).
    let myvpn = build_myvpn_path(&opts);
    conn.ensure_connected().await?;
    let transport = conn.current.as_mut().expect("connected for /myvpn");
    let upgrade = send_request(transport.as_mut(), &HttpRequest::get(&myvpn, host)).await?;
    if upgrade.status != 200 && upgrade.status != 201 {
        return Err(F5Error::TunnelUpgradeRejected(upgrade.status));
    }
    let assigned_ip = upgrade
        .header("x-vpn-client-ip")
        .map(|s| s.to_string())
        .unwrap_or_default();

    // --- 4. PPP negotiation to network up (over the now-open tunnel transport) ---
    let device = device.to_string();
    let negotiator = run_ppp(transport.as_mut(), &upgrade.leftover).await?;

    // Resolve the final IP: prefer the PPP-negotiated address; fall back to the
    // header-assigned one.
    let ip = negotiator
        .negotiated_ipv4()
        .map(|o| std::net::Ipv4Addr::from(o).to_string())
        .or(if assigned_ip.is_empty() {
            None
        } else {
            Some(assigned_ip.clone())
        })
        .unwrap_or_else(|| "0.0.0.0".to_string());

    let parsed_ip = ip
        .parse()
        .unwrap_or_else(|_| "0.0.0.0".parse().expect("valid"));

    // Build the TUN configuration from what was negotiated.
    // Resolve the VPN server to an IP so full-tunnel mode can pin its packets to
    // the original gateway (keeping the encrypted tunnel off the tunnel).
    let server_ip = resolve_host_ipv4(host);

    let tun_config = TunConfig {
        ipv4: Some(ip.clone()),
        // Derive the MTU from the negotiated MRU (was a fixed 1400).
        mtu: Some(negotiator.negotiated_mtu()),
        dns: negotiator
            .dns_servers()
            .into_iter()
            .map(|o| std::net::Ipv4Addr::from(o).to_string())
            .collect(),
        domains: opts.domains.clone(),
        routes: opts.routes.clone(),
        default_gateway: opts.default_gateway,
        server_ip,
    };

    {
        let mut g = shared.lock().expect("poisoned");
        g.alive = true;
        g.handle = Some(ConnectionHandle(HANDLE_SEQ.fetch_add(1, Ordering::SeqCst)));
        g.logout_cookie = Some(cookie_header.clone());
        g.dns = tun_config.dns.clone();
    }

    // NOTE: `LinkUp`/`Connected` are intentionally NOT emitted here. The
    // handshake only proves the control plane + PPP negotiation succeeded; the
    // OS interface is not configured and no packets flow yet. Emitting
    // `Connected` now would lie to the user when `configure()` later fails (the
    // production "looks connected but everything hangs" bug). These events are
    // emitted from `run_data_plane` once the TUN is actually configured.
    Ok(Session {
        cookie_header,
        magic: negotiator.magic(),
        device,
        parsed_ip,
        tun_config,
    })
}

/// The bidirectional data-plane pump. Runs until [`disconnect`](NativeF5Backend::disconnect)
/// signals `shutdown`, the transport closes, or the TUN device closes.
///
/// - OS → tunnel: read an IP packet from the TUN device, F5-encapsulate it, send
///   it over the transport.
/// - tunnel → OS: read from the transport, F5-decapsulate, and write each IP
///   packet to the TUN device (ignoring any residual PPP control frames).
async fn run_data_plane(
    transport: &mut dyn Transport,
    tun: &mut dyn TunDevice,
    dns: &mut dyn DnsApplier,
    session: &Session,
    tx: &UnboundedSender<LifecycleEvent>,
    shared: &Arc<Mutex<Shared>>,
    shutdown: &Arc<Notify>,
) -> crate::vpn::backend::DisconnectReason {
    // Configure the OS interface with the negotiated parameters. This is the
    // step that actually makes the tunnel usable (address, MTU, routes). If it
    // fails we must NOT pretend to be connected: surface a `Failed` event so
    // the supervisor/CLI reacts, instead of silently leaving a dead tunnel
    // (the production "looks connected but everything hangs" bug).
    if let Err(e) = tun.configure(&session.tun_config).await {
        crate::vpn::f5::http::debug_log!("[tun-cfg] ERROR: interface configuration failed: {e}");
        let _ = tx.send(LifecycleEvent::Failed {
            kind: FailureKind::Network,
            detail: format!("failed to configure tunnel interface: {e}"),
        });
        return crate::vpn::backend::DisconnectReason::ServerClosed;
    }

    // Capture the host-teardown plan now that `configure` has recorded the
    // link/route/rp_filter mutations, so the CLI can persist it for an
    // out-of-process `akon vpn off` (works even if this process is SIGKILL'd).
    let mut plan = tun.teardown_plan();

    // Apply the negotiated DNS servers/search domains to the host resolver
    // (systemd-resolved on Fedora/Ubuntu, with fallbacks). Log failures — a
    // working data plane is useless if names don't resolve via the VPN DNS.
    // Only record a DNS-revert in the teardown plan when the applier ACTUALLY
    // mutates the host resolver (the real SystemDnsApplier) AND the apply
    // succeeded — so a NoopDns / test / container run never schedules a
    // `resolvectl` call against the un-namespaced host resolver.
    if !session.tun_config.dns.is_empty() {
        match dns.apply(&session.device, &session.tun_config) {
            Ok(()) => {
                if crate::vpn::f5::http::debug_enabled() {
                    crate::vpn::f5::http::debug_log!(
                        "[dns] applied: servers={:?} domains={:?} on {}",
                        session.tun_config.dns,
                        session.tun_config.domains,
                        session.device
                    );
                }
                if dns.mutates_host() {
                    plan.dns_iface = Some(session.device.clone());
                }
            }
            Err(e) => {
                // Always visible: DNS failure means VPN-only names won't resolve.
                crate::vpn::f5::http::debug_log!(
                    "[dns] WARNING: failed to apply VPN DNS: {e} — names may not resolve"
                )
            }
        }
    }

    // Publish the finalized plan (now including DNS revert if applicable).
    {
        let mut g = shared.lock().expect("poisoned");
        g.teardown_plan = plan;
    }

    // The OS interface is now configured and packets can flow: announce
    // `LinkUp` then `Connected`. This is the first point at which the tunnel is
    // genuinely usable.
    let _ = tx.send(LifecycleEvent::LinkUp {
        ip: session.parsed_ip,
        device: session.device.clone(),
    });
    let _ = tx.send(LifecycleEvent::Connected {
        ip: session.parsed_ip,
        device: session.device.clone(),
    });

    // Run the pump until it exits, then always revert DNS. The exit reason tells
    // the caller whether WE requested the disconnect or the tunnel dropped.
    let reason = pump_packets(transport, tun, shutdown, session.magic).await;
    let _ = dns.revert(&session.device);
    reason
}

/// How often we send a client-originated PPP keepalive (LCP Echo-Request / DPD).
/// Must be comfortably below the F5 server's DPD tolerance (observed ~150 s);
/// 20 s matches openconnect's keepalive range.
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(20);

/// The inner packet-forwarding loop (separated so DNS revert always runs on
/// exit). Returns the reason the pump stopped: `UserRequested` if `disconnect`
/// signalled shutdown, otherwise `ServerClosed` (transport/TUN ended).
///
/// Sends a periodic PPP keepalive (Echo-Request carrying `magic`) so the F5
/// server's DPD does not expire and tear down the tunnel. The keepalive is
/// skipped for any interval in which real outbound data was sent (data already
/// refreshes the server's peer-liveness), mirroring openconnect.
async fn pump_packets(
    transport: &mut dyn Transport,
    tun: &mut dyn TunDevice,
    shutdown: &Arc<Notify>,
    magic: u32,
) -> crate::vpn::backend::DisconnectReason {
    use crate::vpn::backend::DisconnectReason;
    use crate::vpn::f5::http::debug_log;
    let mut tun_buf = vec![0u8; 4096];
    let mut net_buf = vec![0u8; 4096];
    let debug = crate::vpn::f5::http::debug_enabled();
    let (mut out_pkts, mut in_pkts) = (0u64, 0u64);

    // Keepalive state: a timer plus the out_pkts value at the last tick, so we
    // can skip the keepalive when data is already flowing.
    let mut keepalive = tokio::time::interval(KEEPALIVE_INTERVAL);
    keepalive.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // Skip the immediate first tick (interval fires once right away).
    keepalive.tick().await;
    let mut out_pkts_at_last_ka = 0u64;
    let mut ka_id: u8 = 0;

    loop {
        tokio::select! {
            _ = shutdown.notified() => return DisconnectReason::UserRequested,

            // Periodic keepalive / DPD.
            _ = keepalive.tick() => {
                if out_pkts == out_pkts_at_last_ka {
                    // No data sent since the last tick: send an explicit DPD.
                    ka_id = ka_id.wrapping_add(1);
                    let echo = crate::vpn::f5::ppp::lcp_echo_request(
                        ka_id, magic, &magic.to_be_bytes(),
                    );
                    let wire = f5_encap(&crate::vpn::f5::ppp::build_ncp_frame(&echo));
                    if debug {
                        debug_log!("[f5-data] keepalive: sent LCP Echo-Request #{ka_id}");
                    }
                    if transport.send(&wire).await.is_err() {
                        debug_log!("[f5-data] tunnel ended: keepalive send failed");
                        return DisconnectReason::ServerClosed;
                    }
                }
                out_pkts_at_last_ka = out_pkts;
            }

            // OS -> tunnel
            r = tun.read_packet(&mut tun_buf) => {
                match r {
                    Ok(0) | Err(_) => {
                        debug_log!("[f5-data] tunnel ended: TUN device closed");
                        return DisconnectReason::ServerClosed;
                    }
                    Ok(n) => {
                        out_pkts += 1;
                        if debug {
                            debug_log!(
                                "[f5-data] OS->tun #{out_pkts}: {n} bytes {}",
                                hex_preview(&tun_buf[..n], 20)
                            );
                        }
                        let ppp_frame = wrap_ip_in_ppp(&tun_buf[..n]);
                        let wire = f5_encap(&ppp_frame);
                        if transport.send(&wire).await.is_err() {
                            debug_log!("[f5-data] tunnel ended: transport send failed");
                            return DisconnectReason::ServerClosed;
                        }
                    }
                }
            }

            // tunnel -> OS
            r = transport.recv(&mut net_buf) => {
                match r {
                    Ok(0) | Err(_) => {
                        // The server closed the tunnel TLS connection (or it
                        // errored). This is the common "server dropped the
                        // tunnel" case; log it so the reconnect cause is visible.
                        debug_log!("[f5-data] tunnel ended: server closed the connection");
                        return DisconnectReason::ServerClosed;
                    }
                    Ok(n) => {
                        if let Ok(frames) = f5_decap(&net_buf[..n]) {
                            for ppp in frames {
                                // Strip the PPP header and forward only IP packets;
                                // residual LCP/IPCP control frames are ignored.
                                if let Some(ip_packet) = ppp_payload_if_ip(&ppp) {
                                    in_pkts += 1;
                                    if debug {
                                        eprintln!(
                                            "[f5-data] tun<-net #{in_pkts}: {} bytes {}",
                                            ip_packet.len(),
                                            hex_preview(ip_packet, 20)
                                        );
                                    }
                                    let _ = tun.write_packet(ip_packet).await;
                                } else if debug {
                                    eprintln!(
                                        "[f5-data] tun<-net non-IP ctrl frame: {}",
                                        hex_preview(&ppp, 16)
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Wrap a raw IP packet in a PPP IP frame (`FF 03` + proto + payload). Selects
/// IPv6 proto (0x57) when the first nibble is 6, else IPv4 (0x21), matching
/// openconnect's `ppp.c` send path.
fn wrap_ip_in_ppp(ip_packet: &[u8]) -> Vec<u8> {
    let proto: u16 = if ip_packet.first().map(|b| b >> 4) == Some(6) {
        0x0057
    } else {
        0x0021
    };
    let mut frame = Vec::with_capacity(ip_packet.len() + 4);
    frame.push(0xff);
    frame.push(0x03);
    frame.extend_from_slice(&proto.to_be_bytes());
    frame.extend_from_slice(ip_packet);
    frame
}

/// If a PPP frame carries an IP (0x21) or IPv6 (0x57) payload, return the inner
/// IP packet (after the `FF 03 proto` header). Otherwise `None` (control frame).
fn ppp_payload_if_ip(frame: &[u8]) -> Option<&[u8]> {
    // Tolerate optional FF 03 prefix.
    let rest = if frame.len() >= 2 && frame[0] == 0xff && frame[1] == 0x03 {
        &frame[2..]
    } else {
        frame
    };
    // Protocol is 1 byte if the low bit is set (PFC), else 2 bytes.
    if rest.is_empty() {
        return None;
    }
    let (proto, payload) = if rest[0] & 0x01 == 1 {
        (rest[0] as u16, &rest[1..])
    } else if rest.len() >= 2 {
        (u16::from_be_bytes([rest[0], rest[1]]), &rest[2..])
    } else {
        return None;
    };
    match proto {
        0x0021 | 0x0057 => Some(payload), // IPv4 / IPv6
        _ => None,
    }
}

/// Gracefully tear down the session: send an LCP Terminate-Request, then the F5
/// HTTP logout, then close the transport. Best-effort and idempotent — failures
/// are ignored since we are shutting down anyway.
async fn graceful_teardown(transport: &mut dyn Transport, host: &str, session: &Session) {
    // 1. PPP LCP Terminate-Request.
    let term = lcp_terminate_request(0xfe);
    let wire = f5_encap(&crate::vpn::f5::ppp::build_ncp_frame(&term));
    let _ = transport.send(&wire).await;

    // 2. F5 HTTP logout (best-effort, short timeout so teardown can't hang).
    let logout = HttpRequest::get("/vdesk/hangup.php3?hangup_error=1", host)
        .with_header("Cookie", &session.cookie_header);
    let _ = tokio::time::timeout(Duration::from_secs(3), send_request(transport, &logout)).await;

    // 3. Close the transport.
    let _ = transport.close().await;
}

/// Build the `/myvpn` tunnel-upgrade path. No cookies; auth via `sess` + `Z`.
fn build_myvpn_path(opts: &F5Options) -> String {
    let sid = opts.session_id.clone().unwrap_or_default();
    let urz = opts.ur_z.clone().unwrap_or_default();
    let hostname_b64 = BASE64.encode(b"localhost");
    format!(
        "/myvpn?sess={}&hdlc_framing={}&ipv4={}&ipv6={}&Z={}&hostname={}",
        sid,
        if opts.hdlc_framing { "yes" } else { "no" },
        if opts.ipv4 { "yes" } else { "no" },
        if opts.ipv6 { "yes" } else { "no" },
        urz,
        hostname_b64,
    )
}

/// Run PPP LCP+IPCP negotiation over the (now raw) transport until "network up",
/// returning the negotiator (carrying the negotiated IP/DNS/magic).
///
/// `prebuffered` carries any bytes the server coalesced after the `/myvpn`
/// response (the start of the PPP stream on a real TLS connection); they are
/// processed before reading more from the transport.
async fn run_ppp(
    transport: &mut dyn Transport,
    prebuffered: &[u8],
) -> Result<PppNegotiator, F5Error> {
    let mut negotiator = PppNegotiator::new();

    // Send the initial LCP Config-Request(s).
    for frame in negotiator.start() {
        let wire = f5_encap(&frame);
        send_all(transport, &wire).await?;
    }

    // Process any pre-buffered PPP bytes first.
    if !prebuffered.is_empty() {
        drive_ppp_bytes(transport, &mut negotiator, prebuffered).await?;
    }

    let mut buf = [0u8; 4096];
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);

    loop {
        if matches!(negotiator.phase(), PppPhase::Up) {
            return Ok(negotiator);
        }
        if matches!(negotiator.phase(), PppPhase::Terminated) {
            return Err(F5Error::MalformedPpp("PPP terminated during setup".into()));
        }

        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(F5Error::MalformedPpp("PPP negotiation timed out".into()));
        }

        let n = match tokio::time::timeout(remaining, transport.recv(&mut buf)).await {
            Ok(Ok(0)) => return Err(F5Error::MalformedPpp("transport closed during PPP".into())),
            Ok(Ok(n)) => n,
            Ok(Err(e)) => return Err(F5Error::MalformedHttp(format!("recv: {}", e))),
            Err(_) => return Err(F5Error::MalformedPpp("PPP negotiation timed out".into())),
        };

        drive_ppp_bytes(transport, &mut negotiator, &buf[..n]).await?;
    }
}

/// Decode F5 frames from `bytes` and feed each through the negotiator, sending
/// any replies it produces.
///
/// Tolerant by design (matching openconnect): a frame that fails to decap or
/// parse is logged and skipped rather than failing the whole session. Only a
/// genuinely fatal transport condition aborts PPP.
async fn drive_ppp_bytes(
    transport: &mut dyn Transport,
    negotiator: &mut PppNegotiator,
    bytes: &[u8],
) -> Result<(), F5Error> {
    if crate::vpn::f5::http::debug_enabled() {
        eprintln!(
            "[f5-ppp] <<< {} raw bytes: {}",
            bytes.len(),
            hex_preview(bytes, 64)
        );
    }

    let frames = match f5_decap(bytes) {
        Ok(f) => f,
        Err(e) => {
            if crate::vpn::f5::http::debug_enabled() {
                eprintln!("[f5-ppp] decap error (skipping): {e}");
            }
            return Ok(());
        }
    };

    for ppp_frame in frames {
        if crate::vpn::f5::http::debug_enabled() {
            eprintln!(
                "[f5-ppp]   frame {} bytes: {}",
                ppp_frame.len(),
                hex_preview(&ppp_frame, 48)
            );
        }
        match negotiator.on_frame(&ppp_frame) {
            Ok(replies) => {
                for reply in replies {
                    let wire = f5_encap(&reply);
                    send_all(transport, &wire).await?;
                }
            }
            Err(e) => {
                // A single unparseable frame must not kill the session.
                if crate::vpn::f5::http::debug_enabled() {
                    eprintln!("[f5-ppp]   frame parse error (skipping): {e}");
                }
            }
        }
    }
    Ok(())
}

/// Render up to `max` bytes of `data` as space-separated hex for diagnostics.
fn hex_preview(data: &[u8], max: usize) -> String {
    let shown = data.len().min(max);
    let mut s: String = data[..shown].iter().map(|b| format!("{b:02x} ")).collect();
    if data.len() > max {
        s.push_str(&format!("... (+{} more)", data.len() - max));
    }
    s.trim_end().to_string()
}

async fn send_all(transport: &mut dyn Transport, data: &[u8]) -> Result<(), F5Error> {
    transport
        .send(data)
        .await
        .map_err(|e| F5Error::MalformedHttp(format!("send: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vpn::f5::config::F5Options;

    #[test]
    fn myvpn_path_has_required_params_and_no_cookie_semantics() {
        let opts = F5Options {
            session_id: Some("SID".into()),
            ur_z: Some("URZ".into()),
            ipv4: true,
            ipv6: false,
            hdlc_framing: false,
            ..Default::default()
        };
        let path = build_myvpn_path(&opts);
        assert!(path.contains("sess=SID"));
        assert!(path.contains("Z=URZ"));
        assert!(path.contains("ipv4=yes"));
        assert!(path.contains("ipv6=no"));
        assert!(path.contains("hdlc_framing=no"));
        assert!(path.contains("hostname="));
    }

    #[test]
    fn failure_mapping() {
        assert_eq!(
            failure_event(&F5Error::AuthFailed("x".into())),
            LifecycleEvent::Failed {
                kind: FailureKind::Authentication,
                detail: "authentication failed: x".into()
            }
        );
        assert!(matches!(
            failure_event(&F5Error::TunnelUpgradeRejected(403)),
            LifecycleEvent::Failed {
                kind: FailureKind::Network,
                ..
            }
        ));
    }

    #[test]
    fn split_host_port_variants() {
        assert_eq!(
            split_host_port("vpn.example.com", 443),
            ("vpn.example.com".into(), 443)
        );
        assert_eq!(
            split_host_port("vpn.example.com:8443", 443),
            ("vpn.example.com".into(), 8443)
        );
        assert_eq!(
            split_host_port("https://vpn.example.com/path", 443),
            ("vpn.example.com".into(), 443)
        );
        assert_eq!(
            split_host_port("10.0.0.1:444", 443),
            ("10.0.0.1".into(), 444)
        );
    }

    #[test]
    fn wrap_ip_selects_proto_by_version() {
        let v4 = wrap_ip_in_ppp(&[0x45, 0, 0, 0]);
        assert_eq!(&v4[..4], &[0xff, 0x03, 0x00, 0x21]);
        let v6 = wrap_ip_in_ppp(&[0x60, 0, 0, 0]);
        assert_eq!(&v6[..4], &[0xff, 0x03, 0x00, 0x57]);
    }

    #[test]
    fn ppp_payload_extracts_ip() {
        // FF 03 0021 <ip>
        let frame = [0xff, 0x03, 0x00, 0x21, 0xde, 0xad];
        assert_eq!(ppp_payload_if_ip(&frame), Some(&[0xde, 0xad][..]));
        // LCP control frame -> not IP
        let lcp = [0xff, 0x03, 0xc0, 0x21, 0x01];
        assert_eq!(ppp_payload_if_ip(&lcp), None);
    }

    // The idle pump must emit a PPP keepalive (LCP Echo-Request) so the F5
    // server's DPD does not expire and drop the tunnel. Uses paused tokio time
    // to fast-forward the keepalive interval deterministically (no real wait).
    #[cfg(feature = "test-actors")]
    #[tokio::test(start_paused = true)]
    async fn idle_pump_sends_keepalive_echo_request() {
        use crate::vpn::f5::framing::f5_decap;
        use crate::vpn::f5::ppp::{parse_ppp_frame, ECHOREQ, PPP_LCP};
        use crate::vpn::testkit::transport::MemoryTransport;
        use crate::vpn::transport::{NoopTun, Transport};

        let (mut client, mut server) = MemoryTransport::pair();
        let mut tun = NoopTun::default();
        let shutdown = std::sync::Arc::new(Notify::new());
        let magic = 0x0a0b0c0du32;

        // Run the pump in the background (NoopTun never yields packets -> idle).
        let pump =
            tokio::spawn(
                async move { pump_packets(&mut client, &mut tun, &shutdown, magic).await },
            );

        // Advance past one keepalive interval, then yield so the spawned pump
        // task wakes on its timer and writes the keepalive into the in-memory
        // channel. (Under paused time a real `timeout` would auto-advance and
        // race the pump, so we drive scheduling explicitly instead.)
        tokio::time::advance(KEEPALIVE_INTERVAL + Duration::from_secs(1)).await;
        for _ in 0..16 {
            tokio::task::yield_now().await;
        }

        let mut buf = vec![0u8; 4096];
        let n = server.recv(&mut buf).await.expect("recv ok");
        let frames = f5_decap(&buf[..n]).expect("valid f5 frame");
        let pkt = parse_ppp_frame(&frames[0]).expect("valid ppp frame");
        assert_eq!(pkt.proto, PPP_LCP);
        assert_eq!(pkt.code, ECHOREQ, "idle pump must send an LCP Echo-Request");

        pump.abort();
    }
}
