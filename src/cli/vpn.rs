//! VPN connection management commands
//!
//! Native, in-process F5 BIG-IP SSL VPN client. akon runs as the user (keyring
//! intact); the only privilege needed is `CAP_NET_ADMIN` for the TUN device and
//! in-process netlink route configuration, granted via a file capability
//! (`setcap cap_net_admin+ep <akon>`). No `openconnect`, no `sudo`-spawned child.

use akon_core::auth::password::generate_password;
use akon_core::config::toml_config::{get_config_path, TomlConfig};
use akon_core::error::{AkonError, VpnError};
use colored::Colorize;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use tracing::{debug, error, info, warn};

/// State file for tracking VPN connection
fn state_file_path() -> PathBuf {
    std::env::var("AKON_STATE_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp/akon_vpn_state.json"))
}

/// Print actionable suggestions based on VPN error type.
fn print_error_suggestions(error: &VpnError) {
    match error {
        VpnError::AuthenticationFailed => {
            eprintln!(
                "\n{} {}",
                "[TIP]".bright_yellow(),
                "Suggestions:".bright_white().bold()
            );
            eprintln!("   {} Verify your PIN is correct", "-".bright_blue());
            eprintln!(
                "   {} Check if your TOTP secret is valid",
                "-".bright_blue()
            );
            eprintln!(
                "   {} Run {} to reconfigure credentials",
                "-".bright_blue(),
                "akon setup".bright_cyan()
            );
            eprintln!("   {} Ensure your account is not locked", "-".bright_blue());
        }
        VpnError::NetworkError { reason } if reason.contains("SSL") || reason.contains("TLS") => {
            eprintln!(
                "\n{} {}",
                "[TIP]".bright_yellow(),
                "Suggestions:".bright_white().bold()
            );
            eprintln!("   - Check your internet connection");
            eprintln!("   - Verify the VPN server address is correct");
            eprintln!("   - The server may be experiencing issues");
            eprintln!("   - Try again in a few moments");
        }
        VpnError::NetworkError { reason } if reason.contains("Certificate") => {
            eprintln!(
                "\n{} {}",
                "[TIP]".bright_yellow(),
                "Suggestions:".bright_white().bold()
            );
            eprintln!("   - The server certificate may be self-signed");
            eprintln!("   - Contact your VPN administrator for certificate details");
            eprintln!("   - You may need to add the certificate to your trusted store");
        }
        VpnError::NetworkError { reason } if reason.contains("DNS") => {
            eprintln!(
                "\n{} {}",
                "[TIP]".bright_yellow(),
                "Suggestions:".bright_white().bold()
            );
            eprintln!("   - Check your DNS configuration");
            eprintln!("   - Verify the VPN server hostname in config.toml");
            eprintln!("   - Try using the server's IP address instead");
        }
        VpnError::ConnectionFailed { reason }
            if reason.contains("CAP_NET_ADMIN")
                || reason.contains("TUN")
                || reason.contains("Permission") =>
        {
            eprintln!(
                "\n{} {}",
                "[TIP]".bright_yellow(),
                "Suggestions:".bright_white().bold()
            );
            eprintln!(
                "   {} Creating the TUN device needs CAP_NET_ADMIN. Grant it once with:",
                "-".bright_blue()
            );
            eprintln!(
                "       {}",
                "sudo setcap cap_net_admin+ep $(command -v akon)".bright_cyan()
            );
            eprintln!(
                "   {} Then run akon as your normal user (no sudo) so the keyring stays accessible",
                "-".bright_blue()
            );
            eprintln!(
                "   {} Ensure the 'tun' kernel module is loaded: lsmod | grep tun",
                "-".bright_blue()
            );
        }
        _ => {
            eprintln!(
                "\n{} {}",
                "[TIP]".bright_yellow(),
                "Suggestions:".bright_white().bold()
            );
            eprintln!(
                "   {} Check system logs: {}",
                "-".bright_blue(),
                "journalctl -xe".bright_cyan()
            );
            eprintln!(
                "   {} Verify configuration: {}",
                "-".bright_blue(),
                "cat ~/.config/akon/config.toml".bright_cyan()
            );
            eprintln!(
                "   {} Try reconnecting: {}",
                "-".bright_blue(),
                "akon vpn on".bright_cyan()
            );
        }
    }
}

/// Connect using the native, in-process F5 backend.
///
/// The akon process *is* the VPN client: it drives the connection lifecycle,
/// persists connection state, and stays alive carrying the data plane until
/// interrupted (Ctrl-C) or the tunnel ends. Reconnection is supervised
/// **in-process** (no spawned daemon).
#[cfg(target_os = "linux")]
async fn run_vpn_on_native(
    config: &akon_core::config::VpnConfig,
    state_path: &std::path::Path,
    reconnection: Option<akon_core::vpn::reconnection::ReconnectionPolicy>,
) -> Result<(), AkonError> {
    let (mut backend, mut events) = native_connect_once(config, state_path, true).await?;

    println!(
        "\n   {} {} to disconnect",
        "Press".dimmed(),
        "Ctrl-C".bright_cyan()
    );

    // Supervise the connection. With a reconnection policy we run the full
    // event-driven supervisor (reacts to drops immediately + health checks).
    // Without a policy we still watch the event stream so we exit promptly when
    // the tunnel drops (rather than hanging on a dead tunnel). Ctrl-C always wins.
    if let Some(policy) = reconnection {
        native_supervise(config, state_path, &policy, &mut backend, &mut events).await;
    } else {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                println!("\n{} Disconnecting (Ctrl-C)...", "[..]".bright_yellow());
            }
            _ = async {
                // Drain events; exit when the backend disconnects/fails or the
                // stream closes (no reconnection configured).
                loop {
                    match events.recv().await {
                        Some(akon_core::vpn::backend::LifecycleEvent::Disconnected { .. })
                        | Some(akon_core::vpn::backend::LifecycleEvent::Failed { .. })
                        | None => break,
                        Some(_) => {}
                    }
                }
            } => {
                println!("\n{} VPN connection ended", "[..]".bright_yellow());
            }
        }
    }

    let _ = akon_core::vpn::backend::VpnBackend::disconnect(&mut backend);
    // Give the in-process data-plane task a moment to drop the TUN + restore
    // routes before the process exits.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let _ = fs::remove_file(state_path);
    println!("{} VPN disconnected", "[OK]".bright_green().bold());
    Ok(())
}

/// Choose the password for a connect attempt.
///
/// The `AKON_VPN_PASSWORD` env value is a **one-time** PIN+OTP handed off by the
/// parent at startup (background mode). It is valid only for the **initial**
/// connect — TOTP codes expire within 30–60 s, so reusing it for a reconnect
/// minutes later would authenticate with an expired OTP and fail (the cause of
/// the ~3-minute stale-tunnel bug). Therefore: use the env value for the initial
/// connect only; every reconnect regenerates a fresh credential.
fn select_password(
    initial: bool,
    env_password: Option<String>,
    gen: impl FnOnce() -> Result<String, AkonError>,
) -> Result<String, AkonError> {
    if initial {
        if let Some(p) = env_password.filter(|p| !p.trim().is_empty()) {
            return Ok(p);
        }
    }
    // Reconnect, or no initial env value: always generate a fresh PIN+OTP.
    gen()
}

/// Connect the native backend once and drive it to `Connected`, persisting state.
///
/// `initial` MUST be true only for the first connect of a session; reconnection
/// attempts pass `false` so they authenticate with a freshly generated OTP.
#[cfg(target_os = "linux")]
async fn native_connect_once(
    config: &akon_core::config::VpnConfig,
    state_path: &std::path::Path,
    initial: bool,
) -> Result<
    (
        akon_core::vpn::f5::NativeF5Backend,
        tokio::sync::mpsc::UnboundedReceiver<akon_core::vpn::backend::LifecycleEvent>,
    ),
    AkonError,
> {
    use akon_core::vpn::backend::{Credentials, LifecycleEvent, VpnBackend};
    use akon_core::vpn::f5::NativeF5Backend;

    // Only print the connecting banner when running in the foreground; when we
    // are the background child the parent already printed it.
    if std::env::var_os("AKON_BACKGROUND_READY_FILE").is_none() {
        println!(
            "{} {} {}",
            ">>".bright_cyan(),
            "Connecting to VPN server (native F5):"
                .bright_white()
                .bold(),
            config.server.bright_yellow()
        );
    }

    // PIN+OTP password. The one-time `AKON_VPN_PASSWORD` env value is used for
    // the INITIAL connect only; reconnects regenerate a fresh OTP (a reused OTP
    // expires within ~60 s and would fail auth — the ~3-minute stale-tunnel bug).
    let env_password = std::env::var("AKON_VPN_PASSWORD").ok();
    let password = select_password(initial, env_password, || {
        Ok(generate_password(&config.username)?.expose().to_string())
    })?;

    let mut backend = NativeF5Backend::connect_from_config(config)
        .await
        .map_err(|e| {
            error!("native F5 connect failed: {e}");
            AkonError::Vpn(VpnError::ConnectionFailed {
                reason: e.to_string(),
            })
        })?;

    let credentials = Credentials::new(config.username.clone(), password.clone());
    let mut events = backend.connect(credentials).map_err(|e| {
        AkonError::Vpn(VpnError::ConnectionFailed {
            reason: e.to_string(),
        })
    })?;

    loop {
        let Some(event) = events.recv().await else {
            break;
        };
        info!("native lifecycle: {:?}", event);
        match event {
            LifecycleEvent::Authenticating => {
                println!("{} Authenticating...", "[AUTH]".bright_magenta());
            }
            LifecycleEvent::Connected { ip, device } => {
                // Persist state first so `vpn status`/`vpn off` see the session
                // immediately (important: before signalling the parent).
                let teardown_plan = backend.teardown_plan();
                let state = serde_json::json!({
                    "ip": ip.to_string(),
                    "device": device,
                    "connected_at": chrono::Utc::now().to_rfc3339(),
                    "pid": std::process::id(),
                    "backend": "native-f5",
                    "server": config.server,
                    "teardown_plan": teardown_plan,
                });
                let _ = fs::write(state_path, state.to_string());

                // Signal the parent (if we are running as a background child)
                // so it can print the summary and return the terminal.
                crate::cli::background::signal_ready(
                    crate::cli::background::BackgroundReady::Connected {
                        ip: ip.to_string(),
                        device: device.clone(),
                    },
                );

                // Print only when running in the foreground (parent's stdout is
                // the log file when we are the background child).
                if std::env::var_os("AKON_BACKGROUND_READY_FILE").is_none() {
                    println!(
                        "{} {}",
                        "[OK]".bright_green().bold(),
                        "VPN connection established".bright_green().bold()
                    );
                    println!(
                        "   {} {}",
                        "IP address:".bright_white(),
                        ip.to_string().bright_cyan().bold()
                    );
                }

                // Hand back the live event receiver so the supervisor can react
                // to Disconnected/Failed events immediately (event-driven, not
                // polling).
                return Ok((backend, events));
            }
            LifecycleEvent::Failed { kind, detail } => {
                error!("native F5 connection failed: {:?}: {}", kind, detail);
                // Signal the parent so it can report the failure.
                crate::cli::background::signal_ready(
                    crate::cli::background::BackgroundReady::Failed {
                        message: detail.clone(),
                    },
                );
                eprintln!(
                    "{} {}",
                    "[ERROR]".bright_red().bold(),
                    format!("Connection failed: {detail}").bright_red()
                );
                return Err(AkonError::Vpn(VpnError::ConnectionFailed {
                    reason: detail,
                }));
            }
            _ => {}
        }
    }

    Err(AkonError::Vpn(VpnError::ConnectionFailed {
        reason: "connection ended before established".to_string(),
    }))
}

/// In-process health-monitored supervision loop.
///
/// **Event-driven** supervision. Reacts immediately to the backend's lifecycle
/// events — a `Disconnected { ServerClosed }` or `Failed` (the tunnel dropped on
/// its own) triggers an instant reconnect, rather than waiting for a polling
/// tick. A periodic HTTP health check is an additional safety net for *silent*
/// failures (a tunnel that's up but not carrying traffic). Reconnects use a
/// fresh OTP and swap in the new backend + its event stream. Exits on Ctrl-C or
/// a user-requested disconnect.
#[cfg(target_os = "linux")]
async fn native_supervise(
    config: &akon_core::config::VpnConfig,
    state_path: &std::path::Path,
    policy: &akon_core::vpn::reconnection::ReconnectionPolicy,
    backend: &mut akon_core::vpn::f5::NativeF5Backend,
    events: &mut tokio::sync::mpsc::UnboundedReceiver<akon_core::vpn::backend::LifecycleEvent>,
) {
    use akon_core::vpn::backend::LifecycleEvent;
    use akon_core::vpn::health_check::HealthChecker;

    let checker = HealthChecker::new(
        policy.health_check_endpoint.clone(),
        Duration::from_secs(10),
    )
    .ok();
    if checker.is_none() {
        warn!("invalid health-check endpoint; relying on lifecycle events only");
    }

    let interval = Duration::from_secs(policy.health_check_interval_secs.max(1));
    let mut consecutive_failures = 0u32;

    loop {
        // Wait for the FIRST of: a lifecycle event (immediate reaction), the
        // health-check interval elapsing, or Ctrl-C.
        let trigger = tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("Ctrl-C received, stopping native supervision");
                return;
            }
            ev = events.recv() => SuperviseTrigger::Event(ev),
            _ = tokio::time::sleep(interval) => SuperviseTrigger::HealthTick,
        };

        let need_reconnect = match trigger {
            // --- React to a lifecycle event from the backend ---
            SuperviseTrigger::Event(Some(LifecycleEvent::Disconnected { reason })) => {
                if reason.is_user_requested() {
                    info!("backend disconnected at user request; stopping supervision");
                    return;
                }
                warn!("tunnel dropped (server closed); reconnecting immediately");
                println!(
                    "{} {}",
                    "[RECONNECT]".bright_yellow(),
                    "Tunnel dropped, reconnecting...".bright_yellow()
                );
                true
            }
            SuperviseTrigger::Event(Some(LifecycleEvent::Failed { kind, detail })) => {
                warn!("backend failed ({kind:?}: {detail}); reconnecting");
                println!(
                    "{} {}",
                    "[RECONNECT]".bright_yellow(),
                    "Connection failed, reconnecting...".bright_yellow()
                );
                true
            }
            // The event stream closed (sender dropped) → backend is gone.
            SuperviseTrigger::Event(None) => {
                warn!("backend event stream closed; reconnecting");
                true
            }
            // Other events (LinkUp, etc.) — keep watching.
            SuperviseTrigger::Event(Some(_)) => false,

            // --- Periodic health check (safety net for silent failures) ---
            SuperviseTrigger::HealthTick => {
                if let Some(checker) = &checker {
                    if checker.check().await.is_success() {
                        consecutive_failures = 0;
                        debug!("native health check OK");
                        false
                    } else {
                        consecutive_failures += 1;
                        warn!(
                            "native health check failed ({}/{})",
                            consecutive_failures, policy.consecutive_failures_threshold
                        );
                        if consecutive_failures >= policy.consecutive_failures_threshold {
                            println!(
                                "{} {}",
                                "[RECONNECT]".bright_yellow(),
                                "Connection unhealthy, reconnecting...".bright_yellow()
                            );
                            true
                        } else {
                            false
                        }
                    }
                } else {
                    false
                }
            }
        };

        if !need_reconnect {
            continue;
        }

        // --- Reconnect with exponential backoff, fresh OTP, new event stream ---
        let _ = akon_core::vpn::backend::VpnBackend::disconnect(backend);

        let mut delay: u64 = policy.base_interval_secs.max(1) as u64;
        let max_delay: u64 = policy.max_interval_secs.max(1) as u64;
        let multiplier: u64 = policy.backoff_multiplier.max(1) as u64;
        let mut reconnected = false;
        for attempt in 1..=policy.max_attempts {
            // Allow Ctrl-C to interrupt the backoff wait.
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    info!("Ctrl-C during reconnect backoff; stopping");
                    return;
                }
                _ = tokio::time::sleep(Duration::from_secs(delay)) => {}
            }
            // initial = false: regenerate a fresh OTP for every reconnect.
            match native_connect_once(config, state_path, false).await {
                Ok((new_backend, new_events)) => {
                    *backend = new_backend;
                    *events = new_events;
                    consecutive_failures = 0;
                    reconnected = true;
                    info!("native reconnection succeeded on attempt {attempt}");
                    break;
                }
                Err(e) => {
                    warn!("native reconnection attempt {attempt} failed: {e}");
                    delay = (delay * multiplier).min(max_delay);
                }
            }
        }

        if !reconnected {
            error!("native reconnection exhausted all attempts; giving up");
            eprintln!(
                "{} {}",
                "[ERROR]".bright_red().bold(),
                "Reconnection failed after all attempts".bright_red()
            );
            // Fail-safe: leave no half-configured tunnel and make status honest.
            let _ = std::fs::remove_file(state_path);
            return;
        }
    }
}

/// What woke the supervisor's select loop.
#[cfg(target_os = "linux")]
enum SuperviseTrigger {
    Event(Option<akon_core::vpn::backend::LifecycleEvent>),
    HealthTick,
}

#[cfg(not(target_os = "linux"))]
async fn run_vpn_on_native(
    _config: &akon_core::config::VpnConfig,
    _state_path: &std::path::Path,
    _reconnection: Option<akon_core::vpn::reconnection::ReconnectionPolicy>,
) -> Result<(), AkonError> {
    Err(AkonError::Vpn(VpnError::ConnectionFailed {
        reason: "the native F5 backend is only supported on Linux".to_string(),
    }))
}

/// Connect to the VPN (`akon vpn on`).
pub async fn run_vpn_on(force: bool, foreground: bool) -> Result<(), AkonError> {
    let state_path = state_file_path();

    // Handle an existing connection: if a live akon VPN process is recorded,
    // either refuse (already connected) or, with --force, tear it down first.
    if state_path.exists() {
        if let Ok(state_content) = fs::read_to_string(&state_path) {
            if let Ok(state) = serde_json::from_str::<serde_json::Value>(&state_content) {
                if let Some(pid) = state.get("pid").and_then(|p| p.as_u64()) {
                    let process_running = std::process::Command::new("ps")
                        .args(["-p", &pid.to_string()])
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .status()
                        .map(|s| s.success())
                        .unwrap_or(false);

                    if process_running {
                        if force {
                            info!(
                                "Force flag set, disconnecting existing connection and resetting"
                            );
                            println!(
                                "{} {}",
                                "[FORCE]".bright_yellow(),
                                "Force reconnection requested - disconnecting first..."
                                    .bright_yellow()
                            );
                            // The supervising process owns the TUN; signal it to
                            // stop so its Drop reverts host config, then clear state.
                            let _ = run_vpn_off().await;
                        } else {
                            println!(
                                "{} {}",
                                "[OK]".bright_green().bold(),
                                "VPN is already connected".bright_green()
                            );
                            if let Some(ip) = state.get("ip") {
                                println!(
                                    "   {} {}",
                                    "IP address:".bright_white(),
                                    ip.as_str().unwrap_or("unknown").bright_cyan().bold()
                                );
                            }
                            println!(
                                "\n   {} {} to see full status",
                                "Run".dimmed(),
                                "akon vpn status".bright_cyan()
                            );
                            return Ok(());
                        }
                    } else {
                        info!("Found stale connection state (PID {pid}), cleaning up");
                        println!(
                            "{} {}",
                            "[WARN]".bright_yellow(),
                            "Cleaning up stale connection...".dimmed()
                        );
                        let _ = fs::remove_file(&state_path);
                    }
                }
            }
        }
    }

    // Load configuration and connect via the native backend.
    let config_path = get_config_path()?;
    let toml_config = TomlConfig::from_file(&config_path)?;
    let reconnection_policy = toml_config.reconnection.clone();
    let config = toml_config.vpn_config;
    info!("Loaded configuration for server: {}", config.server);

    let result = if !foreground {
        #[cfg(target_os = "linux")]
        {
            crate::cli::background::run_vpn_on_background(&config, &state_path, reconnection_policy)
                .await
        }
        #[cfg(not(target_os = "linux"))]
        {
            run_vpn_on_native(&config, &state_path, reconnection_policy).await
        }
    } else {
        run_vpn_on_native(&config, &state_path, reconnection_policy).await
    };

    if let Err(e) = result {
        if let AkonError::Vpn(ve) = &e {
            print_error_suggestions(ve);
        }
        return Err(e);
    }
    Ok(())
}

/// Disconnect and reconcile ALL host networking changes.
///
/// The native backend mutates the host in-process (TUN, routes, rp_filter, DNS).
/// To guarantee a host always recovers connectivity, `vpn off` signals the
/// supervising process (if alive) and replays the persisted [`HostTeardownPlan`]
/// — which works even if the `vpn on` process was SIGKILL'd and never ran its own
/// cleanup. The teardown is idempotent and best-effort, so it is always safe.
pub async fn run_vpn_off() -> Result<(), AkonError> {
    let state_path = state_file_path();

    if !state_path.exists() {
        println!(
            "{} {}",
            "[WARN]".bright_yellow(),
            "No active VPN connection found".bright_white()
        );
        return Ok(());
    }

    let state_content = fs::read_to_string(&state_path).map_err(|e| {
        AkonError::Vpn(VpnError::ConnectionFailed {
            reason: format!("Failed to read state file: {}", e),
        })
    })?;
    let state: serde_json::Value = serde_json::from_str(&state_content).map_err(|e| {
        AkonError::Vpn(VpnError::ConnectionFailed {
            reason: format!("Failed to parse state file: {}", e),
        })
    })?;

    #[cfg(target_os = "linux")]
    {
        run_vpn_off_native(&state, &state_path).await
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = state;
        let _ = fs::remove_file(&state_path);
        println!("{} VPN disconnected", "[OK]".bright_green().bold());
        Ok(())
    }
}

/// Tear down the native session and restore host networking from the persisted
/// plan (works even after a SIGKILL of the supervising process).
#[cfg(target_os = "linux")]
async fn run_vpn_off_native(
    state: &serde_json::Value,
    state_path: &std::path::Path,
) -> Result<(), AkonError> {
    use akon_core::vpn::f5::teardown::{teardown_host, HostTeardownPlan};
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;

    println!(
        "{} {}",
        ">>".bright_cyan(),
        "Disconnecting VPN and restoring host networking..."
            .bright_white()
            .bold()
    );

    // 1) Ask the supervising process (if still alive) to stop, so it isn't
    //    racing us re-installing routes while we tear them down. Best-effort.
    if let Some(pid) = state.get("pid").and_then(|p| p.as_u64()) {
        let pid = Pid::from_raw(pid as i32);
        let alive = std::process::Command::new("ps")
            .args(["-p", &pid.as_raw().to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if alive {
            info!(
                pid = pid.as_raw(),
                "signalling native VPN supervisor to stop"
            );
            let _ = kill(pid, Signal::SIGTERM);
            for _ in 0..10 {
                tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
                let still = std::process::Command::new("ps")
                    .args(["-p", &pid.as_raw().to_string()])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false);
                if !still {
                    break;
                }
            }
        }
    }

    // 2) Replay the persisted teardown plan to reconcile the host. This is the
    //    authoritative cleanup and is idempotent / safe even if step 1 already
    //    reverted some of it, or the supervisor was killed long ago.
    let plan: HostTeardownPlan = state
        .get("teardown_plan")
        .and_then(|p| serde_json::from_value(p.clone()).ok())
        .unwrap_or_default();

    if plan.is_empty() {
        println!(
            "{} {}",
            "[WARN]".bright_yellow(),
            "No teardown plan recorded; nothing host-level to reconcile".dimmed()
        );
    } else {
        let report = teardown_host(&plan);
        for action in &report.actions {
            println!("   {} {}", "[CLEAN]".bright_green(), action);
            info!(action = %action, "native teardown");
        }
        for warning in &report.warnings {
            warn!(warning = %warning, "native teardown warning");
        }
    }

    if let Err(e) = fs::remove_file(state_path) {
        warn!("failed to remove state file: {e}");
    }

    println!(
        "{} {}",
        "[OK]".bright_green().bold(),
        "VPN disconnected; host networking restored"
            .bright_green()
            .bold()
    );
    Ok(())
}

/// The session metadata read from the state file (a snapshot of the connection
/// state machine). The tunnel interface is the authoritative "connected" signal;
/// these are the supporting details for display.
#[derive(Debug, Default, Clone)]
struct StatusRecord {
    device: Option<String>,
    ip: Option<String>,
    connected_at: Option<String>,
    pid: Option<u64>,
}

impl StatusRecord {
    fn from_json(v: &serde_json::Value) -> Self {
        Self {
            device: v.get("device").and_then(|d| d.as_str()).map(String::from),
            ip: v.get("ip").and_then(|d| d.as_str()).map(String::from),
            connected_at: v
                .get("connected_at")
                .and_then(|d| d.as_str())
                .map(String::from),
            pid: v.get("pid").and_then(|p| p.as_u64()),
        }
    }
}

/// The verdict produced by reconciling the session record against ground truth.
#[derive(Debug, PartialEq, Eq)]
enum StatusVerdict {
    /// A tunnel for the recorded session exists. `ip` is the live address when
    /// available, else the recorded one.
    Connected {
        device: String,
        ip: Option<String>,
        connected_at: Option<String>,
    },
    /// A session is recorded but its tunnel interface is gone.
    Stale {
        reason: &'static str,
        ip: Option<String>,
    },
    /// No session recorded.
    NotConnected,
}

/// Pure status decision: reconcile the (optional) session record against the
/// ground-truth `interface_present` and a live interface IP. The supervising PID
/// is intentionally NOT an input — the tunnel interface is authoritative.
fn evaluate_status(
    record: Option<&StatusRecord>,
    interface_present: bool,
    live_ip: Option<String>,
) -> StatusVerdict {
    let Some(rec) = record else {
        return StatusVerdict::NotConnected;
    };
    match &rec.device {
        None => StatusVerdict::Stale {
            reason: "no tunnel device recorded",
            ip: rec.ip.clone(),
        },
        Some(_) if !interface_present => StatusVerdict::Stale {
            reason: "tunnel interface no longer present",
            ip: rec.ip.clone(),
        },
        Some(device) => StatusVerdict::Connected {
            device: device.clone(),
            ip: live_ip.or_else(|| rec.ip.clone()),
            connected_at: rec.connected_at.clone(),
        },
    }
}

/// Whether the recorded tunnel interface currently exists (ground truth).
fn tunnel_interface_present(device: &str) -> bool {
    #[cfg(target_os = "linux")]
    {
        akon_core::vpn::f5::netlink::interface_exists(device)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = device;
        false
    }
}

/// The live IPv4 currently assigned to the recorded tunnel interface, if any.
fn tunnel_interface_ipv4(device: &str) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        akon_core::vpn::f5::netlink::interface_ipv4(device).map(|ip| ip.to_string())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = device;
        None
    }
}

/// Whether a given PID is currently running (advisory only).
fn pid_running(pid: u64) -> bool {
    std::process::Command::new("ps")
        .args(["-p", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn print_status_header() {
    println!(
        "{} {} - {}",
        "●".bright_green(),
        "akon-vpn".bright_white().bold(),
        "Akon VPN Connection".bright_white()
    );
}

/// Show VPN connection status (`akon vpn status`).
///
/// The native backend runs the VPN **in-process**, so "connected" is decided by
/// the existence of the session's **tunnel interface** (the kernel's truth), not
/// by whether a recorded PID is alive. The state file is a snapshot of the
/// connection state machine used to look up the device and display metadata.
pub fn run_vpn_status() -> Result<(), AkonError> {
    use chrono::{DateTime, Utc};

    let state_path = state_file_path();

    // No state file -> NotConnected (exit 1).
    if !state_path.exists() {
        print_status_header();
        println!(
            "    {} {} ({})",
            "Active:".bright_white(),
            "inactive (dead)".bright_red(),
            "not connected".dimmed()
        );
        std::process::exit(1);
    }

    let state_content = fs::read_to_string(&state_path).map_err(|e| {
        AkonError::Vpn(VpnError::ConnectionFailed {
            reason: format!("Failed to read state file: {}", e),
        })
    })?;
    let state: serde_json::Value = serde_json::from_str(&state_content).map_err(|e| {
        AkonError::Vpn(VpnError::ConnectionFailed {
            reason: format!("Failed to parse state file: {}", e),
        })
    })?;

    let record = StatusRecord::from_json(&state);
    let present = record
        .device
        .as_deref()
        .map(tunnel_interface_present)
        .unwrap_or(false);
    let live_ip = record.device.as_deref().and_then(tunnel_interface_ipv4);

    match evaluate_status(Some(&record), present, live_ip) {
        StatusVerdict::NotConnected => {
            print_status_header();
            println!(
                "    {} {} ({})",
                "Active:".bright_white(),
                "inactive (dead)".bright_red(),
                "not connected".dimmed()
            );
            std::process::exit(1);
        }
        StatusVerdict::Stale { reason, ip } => {
            print_status_header();
            println!(
                "    {} {} ({})",
                "Active:".bright_white(),
                "inactive (stale)".bright_yellow().bold(),
                reason.dimmed()
            );
            if let Some(ip) = ip {
                println!("   {} {}", "Last IP:".dimmed(), ip.bright_cyan());
            }
            println!();
            println!(
                "  {} Run {} to clean up stale state",
                "[TIP]".bright_yellow(),
                "akon vpn off".bright_cyan()
            );
            std::process::exit(2);
        }
        StatusVerdict::Connected {
            device,
            ip,
            connected_at,
        } => {
            let connected_at_info = connected_at.and_then(|s| s.parse::<DateTime<Utc>>().ok());

            let duration_str = connected_at_info.map(|connected_at| {
                let d = Utc::now().signed_duration_since(connected_at);
                if d.num_days() > 0 {
                    format!("{} days", d.num_days())
                } else if d.num_hours() > 0 {
                    format!("{}h {}min", d.num_hours(), d.num_minutes() % 60)
                } else if d.num_minutes() > 0 {
                    format!("{}min {}s", d.num_minutes(), d.num_seconds() % 60)
                } else {
                    format!("{}s", d.num_seconds())
                }
            });
            let active_since = connected_at_info
                .map(|dt| dt.with_timezone(&chrono::Local))
                .map(|dt| dt.format("%a %Y-%m-%d %H:%M:%S %Z").to_string())
                .unwrap_or_else(|| "unknown".to_string());

            print_status_header();
            if let Some(dur) = &duration_str {
                println!(
                    "    {} {} since {}; {} ago",
                    "Active:".bright_white(),
                    "active (running)".bright_green().bold(),
                    active_since.bright_white(),
                    dur.bright_magenta()
                );
            } else {
                println!(
                    "    {} {}",
                    "Active:".bright_white(),
                    "active (running)".bright_green().bold()
                );
            }

            // The PID is advisory; note when the recorded owner is gone (the
            // tunnel still exists, so the verdict remains Connected).
            if let Some(pid_num) = record.pid {
                let suffix = if pid_running(pid_num) {
                    String::new()
                } else {
                    " (not running)".to_string()
                };
                println!(
                    "  {} {} (akon native F5){}",
                    "Main PID:".bright_white(),
                    pid_num.to_string().bright_yellow(),
                    suffix.dimmed()
                );
            }

            if let Some(ip) = ip {
                println!(
                    "        {} {}",
                    "IP:".bright_white(),
                    ip.bright_cyan().bold()
                );
            }
            println!("    {} {}", "Device:".bright_white(), device.bright_cyan());

            Ok(())
        }
    }
}

#[cfg(test)]
mod password_tests {
    use super::select_password;
    use akon_core::error::AkonError;

    #[test]
    fn initial_connect_uses_env_password_when_present() {
        let p = select_password(true, Some("PIN123456".into()), || {
            panic!("must not regenerate on initial when env is present")
        })
        .unwrap();
        assert_eq!(p, "PIN123456");
    }

    #[test]
    fn initial_connect_regenerates_when_env_absent_or_empty() {
        let p = select_password(true, None, || Ok("FRESH".into())).unwrap();
        assert_eq!(p, "FRESH");
        let p2 = select_password(true, Some("   ".into()), || Ok("FRESH".into())).unwrap();
        assert_eq!(p2, "FRESH");
    }

    #[test]
    fn reconnect_always_regenerates_ignoring_env() {
        // The crux of the fix: a reconnect must NOT reuse the stale one-time OTP.
        let p = select_password(false, Some("STALE_OTP".into()), || Ok("FRESH".into())).unwrap();
        assert_eq!(p, "FRESH", "reconnect must use a freshly generated OTP");
    }

    #[test]
    fn regeneration_error_propagates() {
        let r = select_password(false, Some("x".into()), || {
            Err(AkonError::Vpn(
                akon_core::error::VpnError::AuthenticationFailed,
            ))
        });
        assert!(r.is_err());
    }
}

#[cfg(test)]
mod status_tests {
    use super::{evaluate_status, StatusRecord, StatusVerdict};

    fn record(device: Option<&str>, ip: Option<&str>, pid: Option<u64>) -> StatusRecord {
        StatusRecord {
            device: device.map(String::from),
            ip: ip.map(String::from),
            connected_at: Some("2026-01-01T00:00:00Z".into()),
            pid,
        }
    }

    #[test]
    fn no_record_is_not_connected() {
        assert_eq!(
            evaluate_status(None, false, None),
            StatusVerdict::NotConnected
        );
        // Even if some interface happens to be present, no record => not connected.
        assert_eq!(
            evaluate_status(None, true, Some("1.2.3.4".into())),
            StatusVerdict::NotConnected
        );
    }

    #[test]
    fn record_with_present_interface_is_connected_live_ip_preferred() {
        let rec = record(Some("tun0"), Some("10.0.0.1"), Some(1234));
        let v = evaluate_status(Some(&rec), true, Some("10.20.30.40".into()));
        assert_eq!(
            v,
            StatusVerdict::Connected {
                device: "tun0".into(),
                ip: Some("10.20.30.40".into()), // live IP preferred over recorded
                connected_at: Some("2026-01-01T00:00:00Z".into()),
            }
        );
    }

    #[test]
    fn record_with_present_interface_falls_back_to_recorded_ip() {
        let rec = record(Some("tun0"), Some("10.0.0.1"), None);
        let v = evaluate_status(Some(&rec), true, None);
        match v {
            StatusVerdict::Connected { ip, .. } => assert_eq!(ip, Some("10.0.0.1".into())),
            other => panic!("expected Connected, got {other:?}"),
        }
    }

    #[test]
    fn record_with_absent_interface_is_stale() {
        let rec = record(Some("tun0"), Some("10.0.0.1"), Some(1234));
        assert_eq!(
            evaluate_status(Some(&rec), false, None),
            StatusVerdict::Stale {
                reason: "tunnel interface no longer present",
                ip: Some("10.0.0.1".into()),
            }
        );
    }

    #[test]
    fn record_without_device_is_stale() {
        let rec = record(None, Some("10.0.0.1"), Some(1234));
        assert_eq!(
            evaluate_status(Some(&rec), false, None),
            StatusVerdict::Stale {
                reason: "no tunnel device recorded",
                ip: Some("10.0.0.1".into()),
            }
        );
    }

    // FR-005: the verdict must be independent of the PID.
    #[test]
    fn verdict_is_pid_independent() {
        // Interface present + (any) pid => Connected.
        let with_dead_pid = record(Some("tun0"), Some("10.0.0.1"), Some(999999));
        assert!(matches!(
            evaluate_status(Some(&with_dead_pid), true, None),
            StatusVerdict::Connected { .. }
        ));
        // Interface absent + (any) pid => Stale.
        let with_alive_pid = record(Some("tun0"), Some("10.0.0.1"), Some(1));
        assert!(matches!(
            evaluate_status(Some(&with_alive_pid), false, None),
            StatusVerdict::Stale { .. }
        ));
    }
}
