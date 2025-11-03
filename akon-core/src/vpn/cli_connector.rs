//! CLI-based OpenConnect connection manager
//!
//! Manages OpenConnect CLI process lifecycle from spawn to termination

use crate::config::VpnConfig;
use crate::error::{AkonError, VpnError};
use crate::vpn::{ConnectionEvent, ConnectionState, DisconnectReason, OutputParser};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, Mutex};
use std::process::Stdio;

/// CLI-based OpenConnect connection manager
#[allow(dead_code)]
pub struct CliConnector {
    /// Current connection state
    state: Arc<Mutex<ConnectionState>>,

    /// Optional handle to running OpenConnect process
    child_process: Arc<Mutex<Option<Child>>>,

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

    /// Spawn OpenConnect process with credentials
    ///
    /// Returns the spawned child process
    async fn spawn_process(&self) -> Result<Child, VpnError> {
        // Build OpenConnect command
        let server_url = format!("https://{}:{}", self.config.server, self.config.port);

        let mut cmd = Command::new("openconnect");
        cmd.arg("--protocol")
            .arg(self.config.protocol.as_str())
            .arg("--user")
            .arg(&self.config.username)
            .arg("--passwd-on-stdin")
            .arg(&server_url)
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
    /// Writes password and immediately closes stdin for security
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

            drop(stdin); // Close stdin
            tracing::debug!("Password sent to OpenConnect, stdin closed");
        }
        Ok(())
    }

    /// Monitor stdout for connection events
    ///
    /// Runs in background task, parsing output and sending events
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
    /// Spawns OpenConnect, sends credentials, and starts monitoring
    pub async fn connect(&mut self, password: String) -> Result<(), VpnError> {
        // Update state to Connecting
        {
            let mut state = self.state.lock().await;
            *state = ConnectionState::Connecting;
        }

        // Spawn OpenConnect process
        let mut child = self.spawn_process().await?;
        let pid = child.id().unwrap_or(0);

        // Send ProcessStarted event
        let _ = self.event_sender.send(ConnectionEvent::ProcessStarted { pid });

        // Send password via stdin
        self.send_password(&mut child, &password).await?;

        // Take stdout and stderr for monitoring
        let stdout = child.stdout.take().ok_or_else(|| VpnError::ProcessSpawnError {
            reason: "Failed to capture stdout".to_string(),
        })?;

        let stderr = child.stderr.take().ok_or_else(|| VpnError::ProcessSpawnError {
            reason: "Failed to capture stderr".to_string(),
        })?;

        // Store child process
        {
            let mut child_lock = self.child_process.lock().await;
            *child_lock = Some(child);
        }

        // Spawn monitoring tasks
        let parser_clone = Arc::clone(&self.parser);
        let sender_clone = self.event_sender.clone();
        tokio::spawn(async move {
            Self::monitor_stdout(parser_clone, stdout, sender_clone).await;
        });

        let parser_clone = Arc::clone(&self.parser);
        let sender_clone = self.event_sender.clone();
        tokio::spawn(async move {
            Self::monitor_stderr(parser_clone, stderr, sender_clone).await;
        });

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
        // Update state
        {
            let mut state = self.state.lock().await;
            *state = ConnectionState::Disconnecting;
        }

        let mut child_lock = self.child_process.lock().await;
        if let Some(child) = child_lock.as_mut() {
            // Try graceful termination with SIGTERM (kill() sends SIGTERM on Unix)
            child.kill().await.map_err(|_| VpnError::TerminationError)?;

            // Wait with timeout
            let wait_result = tokio::time::timeout(
                tokio::time::Duration::from_secs(5),
                child.wait()
            ).await;

            match wait_result {
                Ok(Ok(_status)) => {
                    tracing::info!("OpenConnect process terminated gracefully");
                }
                Ok(Err(e)) => {
                    tracing::error!("Error waiting for process: {}", e);
                    return Err(VpnError::TerminationError);
                }
                Err(_) => {
                    // Timeout - need force kill
                    tracing::warn!("Graceful shutdown timed out, force killing");
                    self.force_kill_internal(child).await?;
                }
            }

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
