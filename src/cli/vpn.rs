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
    use akon_core::vpn::backend::VpnBackend;

    let mut backend = native_connect_once(config, state_path).await?;

    println!(
        "\n   {} {} to disconnect",
        "Press".dimmed(),
        "Ctrl-C".bright_cyan()
    );

    // Ctrl-C MUST always win, even mid-reconnect, so race the whole supervision
    // future against the signal at the top level.
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            println!("\n{} Disconnecting (Ctrl-C)...", "[..]".bright_yellow());
        }
        _ = async {
            if let Some(policy) = reconnection {
                native_supervise(config, state_path, &policy, &mut backend).await;
            } else {
                std::future::pending::<()>().await;
            }
        } => {}
    }

    let _ = backend.disconnect();
    // Give the in-process data-plane task a moment to drop the TUN + restore
    // routes before the process exits.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let _ = fs::remove_file(state_path);
    println!("{} VPN disconnected", "[OK]".bright_green().bold());
    Ok(())
}

/// Connect the native backend once and drive it to `Connected`, persisting state.
#[cfg(target_os = "linux")]
async fn native_connect_once(
    config: &akon_core::config::VpnConfig,
    state_path: &std::path::Path,
) -> Result<akon_core::vpn::f5::NativeF5Backend, AkonError> {
    use akon_core::vpn::backend::{Credentials, LifecycleEvent, VpnBackend};
    use akon_core::vpn::f5::NativeF5Backend;

    println!(
        "{} {} {}",
        ">>".bright_cyan(),
        "Connecting to VPN server (native F5):"
            .bright_white()
            .bold(),
        config.server.bright_yellow()
    );

    // Fresh PIN+OTP password. Prefer a pre-generated value passed via
    // AKON_VPN_PASSWORD (so a privileged run can use a credential generated by
    // the unprivileged user). Falls back to the keyring when the env var is
    // absent (the normal rootless path: running as the user with a
    // capability-granted binary). Never logged.
    let password: String = match std::env::var("AKON_VPN_PASSWORD") {
        Ok(p) if !p.trim().is_empty() => p,
        _ => generate_password(&config.username)?.expose().to_string(),
    };

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

    while let Some(event) = events.recv().await {
        info!("native lifecycle: {:?}", event);
        match event {
            LifecycleEvent::Authenticating => {
                println!("{} Authenticating...", "[AUTH]".bright_magenta());
            }
            LifecycleEvent::Connected { ip, device } => {
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
                // Persist the host-teardown plan so `akon vpn off` can fully
                // restore the host even if this process is later killed.
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
                return Ok(backend);
            }
            LifecycleEvent::Failed { kind, detail } => {
                error!("native F5 connection failed: {:?}: {}", kind, detail);
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
/// Periodically runs an HTTP health check; after `consecutive_failures_threshold`
/// failures it tears down and re-establishes the connection (up to `max_attempts`
/// with exponential backoff). Exits cleanly on Ctrl-C.
#[cfg(target_os = "linux")]
async fn native_supervise(
    config: &akon_core::config::VpnConfig,
    state_path: &std::path::Path,
    policy: &akon_core::vpn::reconnection::ReconnectionPolicy,
    backend: &mut akon_core::vpn::f5::NativeF5Backend,
) {
    use akon_core::vpn::backend::VpnBackend;
    use akon_core::vpn::health_check::HealthChecker;

    let checker = match HealthChecker::new(
        policy.health_check_endpoint.clone(),
        Duration::from_secs(10),
    ) {
        Ok(c) => c,
        Err(e) => {
            warn!("invalid health-check endpoint, supervision disabled: {e}");
            let _ = tokio::signal::ctrl_c().await;
            return;
        }
    };

    let interval = Duration::from_secs(policy.health_check_interval_secs.max(1));
    let mut consecutive_failures = 0u32;

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("Ctrl-C received, stopping native supervision");
                return;
            }
            _ = tokio::time::sleep(interval) => {}
        }

        let result = checker.check().await;
        if result.is_success() {
            consecutive_failures = 0;
            debug!("native health check OK");
            continue;
        }

        consecutive_failures += 1;
        warn!(
            "native health check failed ({}/{})",
            consecutive_failures, policy.consecutive_failures_threshold
        );
        if consecutive_failures < policy.consecutive_failures_threshold {
            continue;
        }

        // Reconnect with exponential backoff.
        println!(
            "{} {}",
            "[RECONNECT]".bright_yellow(),
            "Connection unhealthy, reconnecting...".bright_yellow()
        );
        let _ = backend.disconnect();

        let mut delay: u64 = policy.base_interval_secs.max(1) as u64;
        let max_delay: u64 = policy.max_interval_secs.max(1) as u64;
        let multiplier: u64 = policy.backoff_multiplier.max(1) as u64;
        let mut reconnected = false;
        for attempt in 1..=policy.max_attempts {
            tokio::time::sleep(Duration::from_secs(delay)).await;
            match native_connect_once(config, state_path).await {
                Ok(new_backend) => {
                    *backend = new_backend;
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
            return;
        }
    }
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
pub async fn run_vpn_on(force: bool) -> Result<(), AkonError> {
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

    if let Err(e) = run_vpn_on_native(&config, &state_path, reconnection_policy).await {
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
