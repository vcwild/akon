//! Unix socket IPC for daemon communication
//!
//! Handles communication between parent and daemon processes using Unix domain sockets.

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::thread;

use akon_core::error::{AkonError, VpnError};
use akon_core::vpn::state::{ConnectionState, SharedConnectionState};

/// IPC message types
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum IpcMessage {
    /// Request current connection status
    StatusRequest,
    /// Response with current connection status
    StatusResponse(ConnectionState),
    /// Request to disconnect
    DisconnectRequest,
    /// Response to disconnect request
    DisconnectResponse(Result<(), String>),
    /// Connection state change notification
    StateChange(ConnectionState),
}

/// IPC client for communicating with daemon
pub struct IpcClient {
    socket_path: PathBuf,
}

impl IpcClient {
    /// Create a new IPC client
    pub fn new(socket_path: PathBuf) -> Self {
        Self { socket_path }
    }

    /// Send a message and receive a response
    pub fn send_message(&self, message: &IpcMessage) -> Result<IpcMessage, AkonError> {
        let mut stream = UnixStream::connect(&self.socket_path)
            .map_err(|e| AkonError::Vpn(VpnError::ConnectionFailed {
                reason: format!("Failed to connect to daemon socket: {}", e),
            }))?;

        // Serialize and send message
        let message_data = serde_json::to_vec(message)
            .map_err(|e| AkonError::Vpn(VpnError::ConnectionFailed {
                reason: format!("Failed to serialize message: {}", e),
            }))?;

        stream.write_all(&message_data)
            .map_err(|e| AkonError::Vpn(VpnError::ConnectionFailed {
                reason: format!("Failed to send message: {}", e),
            }))?;

        stream.flush()
            .map_err(|e| AkonError::Vpn(VpnError::ConnectionFailed {
                reason: format!("Failed to flush message: {}", e),
            }))?;

        // Read response
        let mut buffer = Vec::new();
        stream.read_to_end(&mut buffer)
            .map_err(|e| AkonError::Vpn(VpnError::ConnectionFailed {
                reason: format!("Failed to read response: {}", e),
            }))?;

        let response: IpcMessage = serde_json::from_slice(&buffer)
            .map_err(|e| AkonError::Vpn(VpnError::ConnectionFailed {
                reason: format!("Failed to deserialize response: {}", e),
            }))?;

        Ok(response)
    }

    /// Get current connection status
    pub fn get_status(&self) -> Result<ConnectionState, AkonError> {
        match self.send_message(&IpcMessage::StatusRequest)? {
            IpcMessage::StatusResponse(state) => Ok(state),
            _ => Err(AkonError::Vpn(VpnError::ConnectionFailed {
                reason: "Unexpected response to status request".to_string(),
            })),
        }
    }

    /// Request disconnection
    pub fn disconnect(&self) -> Result<(), AkonError> {
        match self.send_message(&IpcMessage::DisconnectRequest)? {
            IpcMessage::DisconnectResponse(result) => result
                .map_err(|e| AkonError::Vpn(VpnError::ConnectionFailed {
                    reason: format!("Disconnect failed: {}", e),
                })),
            _ => Err(AkonError::Vpn(VpnError::ConnectionFailed {
                reason: "Unexpected response to disconnect request".to_string(),
            })),
        }
    }
}

/// IPC server for daemon to listen for commands
pub struct IpcServer {
    listener: UnixListener,
    connection_state: SharedConnectionState,
}

impl IpcServer {
    /// Create a new IPC server
    pub fn new(socket_path: PathBuf, connection_state: SharedConnectionState) -> Result<Self, AkonError> {
        // Clean up any existing socket
        let _ = std::fs::remove_file(&socket_path);

        let listener = UnixListener::bind(&socket_path)
            .map_err(|e| AkonError::Vpn(VpnError::ConnectionFailed {
                reason: format!("Failed to bind IPC socket: {}", e),
            }))?;

        Ok(Self {
            listener,
            connection_state,
        })
    }

    /// Run the IPC server (blocking)
    pub fn run(&self) -> Result<(), AkonError> {
        for stream in self.listener.incoming() {
            match stream {
                Ok(mut stream) => {
                    let connection_state = self.connection_state.clone();
                    thread::spawn(move || {
                        if let Err(e) = Self::handle_connection(&mut stream, &connection_state) {
                            eprintln!("IPC connection error: {}", e);
                        }
                    });
                }
                Err(e) => {
                    eprintln!("IPC accept error: {}", e);
                    // Continue listening
                }
            }
        }

        Ok(())
    }

    /// Handle a single IPC connection
    fn handle_connection(stream: &mut UnixStream, connection_state: &SharedConnectionState) -> Result<(), AkonError> {
        let mut buffer = Vec::new();
        stream.read_to_end(&mut buffer)
            .map_err(|e| AkonError::Vpn(VpnError::ConnectionFailed {
                reason: format!("Failed to read IPC message: {}", e),
            }))?;

        let message: IpcMessage = serde_json::from_slice(&buffer)
            .map_err(|e| AkonError::Vpn(VpnError::ConnectionFailed {
                reason: format!("Failed to deserialize IPC message: {}", e),
            }))?;

        let response = match message {
            IpcMessage::StatusRequest => {
                let state = connection_state.get();
                IpcMessage::StatusResponse(state)
            }
            IpcMessage::DisconnectRequest => {
                // Set state to disconnecting
                connection_state.set(ConnectionState::Disconnecting);
                IpcMessage::DisconnectResponse(Ok(()))
            }
            _ => {
                return Err(AkonError::Vpn(VpnError::ConnectionFailed {
                    reason: "Unknown IPC message type".to_string(),
                }));
            }
        };

        let response_data = serde_json::to_vec(&response)
            .map_err(|e| AkonError::Vpn(VpnError::ConnectionFailed {
                reason: format!("Failed to serialize response: {}", e),
            }))?;

        stream.write_all(&response_data)
            .map_err(|e| AkonError::Vpn(VpnError::ConnectionFailed {
                reason: format!("Failed to send response: {}", e),
            }))?;

        stream.flush()
            .map_err(|e| AkonError::Vpn(VpnError::ConnectionFailed {
                reason: format!("Failed to flush response: {}", e),
            }))?;

        Ok(())
    }
}

/// Get the default socket path
pub fn get_default_socket_path() -> PathBuf {
    // Use XDG_RUNTIME_DIR if available, otherwise /tmp
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        Path::new(&runtime_dir).join("akon.sock")
    } else {
        Path::new("/tmp").join(format!("akon-{}.sock", nix::unistd::getuid()))
    }
}
