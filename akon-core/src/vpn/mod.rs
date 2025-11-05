//! VPN connection module
//!
//! Handles OpenConnect CLI integration and connection state management.

pub mod cli_connector;
pub mod connection_event;
pub mod output_parser;
pub mod state;

// Network interruption detection and automatic reconnection
pub mod health_check;
pub mod network_monitor;
pub mod process;
pub mod reconnection;

// Public re-exports
pub use cli_connector::CliConnector;
pub use connection_event::{ConnectionEvent, ConnectionState, DisconnectReason};
pub use output_parser::OutputParser;
