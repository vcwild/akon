//! CLI-based OpenConnect connection manager
//!
//! Manages OpenConnect CLI process lifecycle from spawn to termination

use crate::config::VpnConfig;
use crate::error::{AkonError, VpnError};
use crate::vpn::{ConnectionEvent, ConnectionState, DisconnectReason, OutputParser};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{mpsc, Mutex};
use std::process::Stdio;
use std::time::Duration;

/// CLI-based OpenConnect connection manager
#[allow(dead_code)]
pub struct CliConnector {
    /// Current connection state
    state: Arc<Mutex<ConnectionState>>,

    /// Optional handle to running OpenConnect process (may be sudo wrapper)
    child_process: Arc<Mutex<Option<Child>>>,

    /// Actual OpenConnect process PID (not the sudo wrapper)
    openconnect_pid: Arc<Mutex<Option<u32>>>,

    /// OpenConnect stdin - kept alive to prevent process termination
    process_stdin: Arc<Mutex<Option<ChildStdin>>>,

    /// Channel for receiving connection events
    event_receiver: mpsc::UnboundedReceiver<ConnectionEvent>,

    /// Channel sender (kept for cloning to monitor tasks)
    event_sender: mpsc::UnboundedSender<ConnectionEvent>,

    /// Parser for OpenConnect output
    parser: Arc<OutputParser>,

    /// Configuration (server URL, protocol)
    config: VpnConfig,
}

impl CliConnector {
    /// Create new connector with configuration
    pub fn new(config: VpnConfig) -> Result<Self, AkonError> {
        let (event_sender, event_receiver) = mpsc::unbounded_channel();

        Ok(Self {
            state: Arc::new(Mutex::new(ConnectionState::Idle)),
            child_process: Arc::new(Mutex::new(None)),
            openconnect_pid: Arc::new(Mutex::new(None)),
            process_stdin: Arc::new(Mutex::new(None)),
            event_receiver,
            event_sender,
            parser: Arc::new(OutputParser::new()),
            config,
        })
    }

    /// Get current connection state
    pub fn state(&self) -> ConnectionState {
        // This is a synchronous method, but we need to handle the async Mutex
        // For now, we'll use try_lock which is available
        self.state.try_lock()
            .map(|guard| guard.clone())
            .unwrap_or(ConnectionState::Idle)
    }

    /// Check if currently connected
    pub fn is_connected(&self) -> bool {
        matches!(self.state(), ConnectionState::Established { .. })
    }

    /// Get the process ID of the running OpenConnect process
    ///
    /// Returns the actual openconnect PID, not the sudo wrapper PID
    pub fn get_pid(&self) -> Option<u32> {
        self.openconnect_pid
            .try_lock()
            .ok()
            .and_then(|guard| *guard)
    }

    /// Find the OpenConnect daemon process PID
    ///
    /// When openconnect uses --background, it daemonizes and we need to find
    /// it by process name and command line matching our server
    async fn find_openconnect_daemon_pid(server: &str) -> Option<u32> {
        // Wait a bit for daemon to start
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Try multiple times in case daemon hasn't started yet
        for attempt in 0..15 {
            // Use pgrep to find openconnect processes matching our server
            let output = tokio::process::Command::new("pgrep")
                .args(["-f", &format!("openconnect.*{}", server)])
                .output()
                .await;

            if let Ok(output) = output {
                if output.status.success() {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    // Parse PID (take the first one if multiple)
                    for line in stdout.lines() {
                        if let Ok(pid) = line.trim().parse::<u32>() {
                            tracing::debug!(
                                "Found OpenConnect daemon PID {} for server {}",
                                pid,
                                server
                            );
                            return Some(pid);
                        }
                    }
                }
            }

            // Wait a bit and retry
            if attempt < 14 {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }

        tracing::warn!(
            "Could not find OpenConnect daemon process for server {}",
            server
        );
        None
    }

    /// Spawn OpenConnect process with credentials
    ///
    /// Returns the spawned child process
    async fn spawn_process(&self) -> Result<Child, VpnError> {
        // Use sudo to run openconnect since it requires root privileges for network configuration
        let mut cmd = Command::new("sudo");
        cmd.arg("openconnect")
            .arg("--protocol")
            .arg(self.config.protocol.as_str())
            .arg("--user")
            .arg(&self.config.username)
            .arg("--passwd-on-stdin")
            .arg("--background"); // Daemonize to stay running

        // Add --no-dtls flag if configured
        if self.config.no_dtls {
            cmd.arg("--no-dtls");
            tracing::debug!("DTLS disabled per configuration");
        }

        // Add server (without explicit port, let openconnect use default)
        cmd.arg(&self.config.server)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Spawn the process
        let child = cmd.spawn().map_err(|e| VpnError::ProcessSpawnError {
            reason: format!("Failed to spawn openconnect: {}", e),
        })?;

        tracing::debug!("OpenConnect process spawned with PID: {:?}", child.id());
        Ok(child)
    }

    /// Send password to OpenConnect via stdin
    ///
    /// Writes password and keeps stdin open (closing it would terminate openconnect)
    async fn send_password(&self, child: &mut Child, password: &str) -> Result<(), VpnError> {
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(password.as_bytes())
                .await
                .map_err(|e| VpnError::ProcessSpawnError {
                    reason: format!("Failed to write password to stdin: {}", e),
                })?;

            stdin
                .write_all(b"\n")
                .await
                .map_err(|e| VpnError::ProcessSpawnError {
                    reason: format!("Failed to write newline to stdin: {}", e),
                })?;

            stdin
                .flush()
                .await
                .map_err(|e| VpnError::ProcessSpawnError {
                    reason: format!("Failed to flush stdin: {}", e),
                })?;

            // Store stdin to keep it alive - closing it would terminate openconnect
            {
                let mut stdin_lock = self.process_stdin.lock().await;
                *stdin_lock = Some(stdin);
            }
            tracing::debug!("Password sent to OpenConnect, stdin kept alive");
        }
        Ok(())
    }

    /// Monitor stdout for connection events
    ///
    /// Runs in background task, parsing output and sending events
    #[allow(dead_code)]
    async fn monitor_stdout(
        parser: Arc<OutputParser>,
        stdout: tokio::process::ChildStdout,
        event_sender: mpsc::UnboundedSender<ConnectionEvent>,
    ) {
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();

        while let Ok(Some(line)) = lines.next_line().await {
            tracing::debug!("OpenConnect stdout: {}", line);
            let event = parser.parse_line(&line);

            if event_sender.send(event.clone()).is_err() {
                tracing::warn!("Failed to send event, receiver dropped");
                break;
            }

            // Stop monitoring if we hit certain terminal events
            if matches!(event, ConnectionEvent::Error { .. }) {
                break;
            }
        }
    }

    /// Monitor stderr for errors
    ///
    /// Runs in background task, parsing error output
    #[allow(dead_code)]
    async fn monitor_stderr(
        parser: Arc<OutputParser>,
        stderr: tokio::process::ChildStderr,
        event_sender: mpsc::UnboundedSender<ConnectionEvent>,
    ) {
        let reader = BufReader::new(stderr);
        let mut lines = reader.lines();

        while let Ok(Some(line)) = lines.next_line().await {
            tracing::debug!("OpenConnect stderr: {}", line);
            let event = parser.parse_error(&line);

            if event_sender.send(event).is_err() {
                tracing::warn!("Failed to send error event, receiver dropped");
                break;
            }
        }
    }

    /// Connect to VPN
    ///
    /// Spawns OpenConnect, sends credentials, waits for connection, then detaches
    pub async fn connect(&mut self, password: String) -> Result<(), VpnError> {
        // Update state to Connecting
        {
            let mut state = self.state.lock().await;
            *state = ConnectionState::Connecting;
        }

        // Spawn OpenConnect process (via sudo wrapper with --background flag)
        let mut child = self.spawn_process().await?;
        let sudo_pid = child.id().unwrap_or(0);

        tracing::info!("Spawned sudo wrapper with PID {}", sudo_pid);

        // Send password via stdin (do this immediately while sudo is running)
        self.send_password(&mut child, &password).await?;

        // Take stdout for monitoring connection status
        let stdout = child.stdout.take().ok_or_else(|| VpnError::ProcessSpawnError {
            reason: "Failed to capture stdout".to_string(),
        })?;

        // Monitor stdout until we see connection success, then stop
        let parser = Arc::clone(&self.parser);
        let event_sender = self.event_sender.clone();

        let mut reader = BufReader::new(stdout).lines();
        let mut connected = false;
        let mut ip_address = None;
        let mut device = None;

        // Read output until connection is established or error occurs
        while let Ok(Some(line)) = reader.next_line().await {
            tracing::debug!("OpenConnect: {}", line);

            // Parse the line for connection events
            let event = parser.parse_line(&line);
            match &event {
                ConnectionEvent::Connected { ip, device: dev } => {
                    connected = true;
                    ip_address = Some(ip.to_string());
                    device = Some(dev.clone());
                    let _ = event_sender.send(event.clone());
                    break; // Stop monitoring once connected
                }
                ConnectionEvent::Error { kind, raw_output } => {
                    let error_msg = format!("{:?}: {}", kind, raw_output);
                    let _ = event_sender.send(event.clone());
                    return Err(VpnError::ConnectionFailed {
                        reason: error_msg,
                    });
                }
                _ => {
                    let _ = event_sender.send(event.clone());
                }
            }
        }

        if !connected {
            return Err(VpnError::ConnectionFailed {
                reason: "Connection established message not received".to_string(),
            });
        }

        // Find the daemonized OpenConnect process PID
        let daemon_pid = Self::find_openconnect_daemon_pid(&self.config.server).await;

        // Store the daemon PID
        let final_pid = daemon_pid.ok_or_else(|| VpnError::ProcessSpawnError {
            reason: "Could not find openconnect daemon process".to_string(),
        })?;

        {
            let mut pid_lock = self.openconnect_pid.lock().await;
            *pid_lock = Some(final_pid);
        }

        tracing::info!("OpenConnect daemonized with PID {}", final_pid);

        // Send ProcessStarted event with the actual PID
        let _ = event_sender.send(ConnectionEvent::ProcessStarted { pid: final_pid });

        // Update state to Established
        {
            let mut state = self.state.lock().await;
            *state = ConnectionState::Established {
                ip: ip_address.unwrap_or_default().parse().unwrap_or("0.0.0.0".parse().unwrap()),
                device: device.unwrap_or_default(),
            };
        }

        // Drop child handle - let openconnect run independently as a daemon
        // We only keep the PID for status checks and disconnect operations
        drop(child);
        tracing::info!("Detached from OpenConnect daemon, returning control to user");

        Ok(())
    }

    /// Get next connection event
    ///
    /// Returns None if event channel is closed
    pub async fn next_event(&mut self) -> Option<ConnectionEvent> {
        self.event_receiver.recv().await
    }

    /// Gracefully disconnect VPN
    ///
    /// Sends SIGTERM and waits up to 5 seconds before force-killing
    pub async fn disconnect(&mut self) -> Result<(), VpnError> {
        use nix::sys::signal::{kill, Signal};
        use nix::unistd::Pid;

        // Update state
        {
            let mut state = self.state.lock().await;
            *state = ConnectionState::Disconnecting;
        }

        // Get the actual OpenConnect PID
        let pid_opt = {
            let pid_lock = self.openconnect_pid.lock().await;
            *pid_lock
        };

        if let Some(pid_num) = pid_opt {
            let pid = Pid::from_raw(pid_num as i32);

            // Check if process exists
            if kill(pid, None).is_err() {
                tracing::info!("OpenConnect process {} already terminated", pid);

            // Clean up state
            {
                let mut pid_lock = self.openconnect_pid.lock().await;
                *pid_lock = None;
            }
            {
                let mut child_lock = self.child_process.lock().await;
                *child_lock = None;
            }
            {
                let mut stdin_lock = self.process_stdin.lock().await;
                *stdin_lock = None; // Close stdin
            }                return Ok(());
            }

            tracing::info!("Sending SIGTERM to OpenConnect process {}", pid);

            // Try graceful termination with SIGTERM
            if let Err(e) = kill(pid, Signal::SIGTERM) {
                tracing::error!("Failed to send SIGTERM: {}", e);
                return Err(VpnError::TerminationError);
            }

            // Wait with timeout for process to exit
            let mut attempts = 0;
            let max_attempts = 10; // 5 seconds (500ms * 10)

            loop {
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                attempts += 1;

                // Check if process still exists
                match kill(pid, None) {
                    Err(_) => {
                        // Process no longer exists
                        tracing::info!("OpenConnect process terminated gracefully");
                        break;
                    }
                    Ok(_) if attempts >= max_attempts => {
                        // Timeout - force kill
                        tracing::warn!("Graceful shutdown timed out, sending SIGKILL");
                        if let Err(e) = kill(pid, Signal::SIGKILL) {
                            tracing::error!("Failed to send SIGKILL: {}", e);
                            return Err(VpnError::TerminationError);
                        }

                        // Wait a bit for SIGKILL to take effect
                        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                        tracing::warn!("Sent SIGKILL to process {}", pid);
                        break;
                    }
                    _ => {
                        // Still running, continue waiting
                        continue;
                    }
                }
            }

            // Clean up state
            {
                let mut pid_lock = self.openconnect_pid.lock().await;
                *pid_lock = None;
            }
        }

        // Clean up child process handle
        {
            let mut child_lock = self.child_process.lock().await;
            *child_lock = None;
        }

        // Update state to Idle
        {
            let mut state = self.state.lock().await;
            *state = ConnectionState::Idle;
        }

        // Send disconnect event
        let _ = self.event_sender.send(ConnectionEvent::Disconnected {
            reason: DisconnectReason::UserRequested,
        });

        Ok(())
    }

    /// Force kill the process with SIGKILL
    async fn force_kill_internal(&self, child: &mut Child) -> Result<(), VpnError> {
        if let Some(pid) = child.id() {
            use nix::sys::signal::{kill, Signal};
            use nix::unistd::Pid;

            let pid = Pid::from_raw(pid as i32);
            kill(pid, Signal::SIGKILL).map_err(|_| VpnError::TerminationError)?;
            tracing::warn!("Sent SIGKILL to process {}", pid);
        }
        Ok(())
    }

    /// Force kill VPN connection
    pub async fn force_kill(&mut self) -> Result<(), VpnError> {
        let mut child_lock = self.child_process.lock().await;
        if let Some(child) = child_lock.as_mut() {
            self.force_kill_internal(child).await?;
            *child_lock = None;
        }

        // Update state to Idle
        {
            let mut state = self.state.lock().await;
            *state = ConnectionState::Idle;
        }

        Ok(())
    }
}
