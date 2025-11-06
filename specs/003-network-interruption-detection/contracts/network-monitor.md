# NetworkMonitor Interface Contract

**Module**: `akon-core/src/vpn/network_monitor.rs`
**Purpose**: Monitor system network state changes via D-Bus

## Interface

```rust
use tokio::sync::mpsc;
use zbus::Connection;

/// Monitors network state changes from NetworkManager
pub struct NetworkMonitor {
    connection: Connection,
    event_tx: mpsc::UnboundedSender<NetworkEvent>,
}

impl NetworkMonitor {
    /// Create a new network monitor connected to system D-Bus
    ///
    /// # Errors
    /// Returns error if D-Bus connection fails or NetworkManager unavailable
    pub async fn new() -> Result<Self, NetworkMonitorError>;

    /// Start monitoring network events
    ///
    /// Spawns background task that listens for D-Bus signals and sends
    /// NetworkEvent through the channel. Task continues until monitor dropped.
    ///
    /// # Returns
    /// Receiver end of channel for consuming network events
    pub fn start(self) -> mpsc::UnboundedReceiver<NetworkEvent>;

    /// Check if network is currently available
    ///
    /// Queries NetworkManager for current connectivity state
    pub async fn is_network_available(&self) -> Result<bool, NetworkMonitorError>;
}

#[derive(Debug, thiserror::Error)]
pub enum NetworkMonitorError {
    #[error("D-Bus connection failed: {0}")]
    DBusConnectionFailed(#[from] zbus::Error),

    #[error("NetworkManager not available")]
    NetworkManagerUnavailable,

    #[error("Failed to query network state: {0}")]
    QueryFailed(String),
}
```

## Behavior Specification

### Startup

- **Precondition**: System has D-Bus and NetworkManager running
- **Postcondition**: Monitor connected to `org.freedesktop.NetworkManager` on system bus
- **Side Effects**: Subscribes to NetworkManager signals

### Event Detection

**Network Up/Down**:
- **Trigger**: `StateChanged` signal from NetworkManager
- **Condition**: State transitions to/from `NM_STATE_CONNECTED_GLOBAL`
- **Output**: `NetworkEvent::NetworkUp` or `NetworkEvent::NetworkDown`

**Interface Changed**:
- **Trigger**: `PropertiesChanged` signal on `org.freedesktop.NetworkManager.Connection.Active`
- **Condition**: `Devices` property changes while connected
- **Output**: `NetworkEvent::InterfaceChanged { old, new }`

**System Suspend/Resume**:
- **Trigger**: `PrepareForSleep` signal from `org.freedesktop.login1.Manager`
- **Condition**: Signal argument is true (suspending) or false (resumed)
- **Output**: `NetworkEvent::SystemSuspending` or `NetworkEvent::SystemResumed`

### Error Handling

- **D-Bus disconnection**: Attempt reconnect with exponential backoff (separate from VPN reconnection)
- **Signal parsing errors**: Log warning and continue (don't crash monitor)
- **Channel send errors**: Receiver dropped, monitor should shutdown gracefully

## Testing Contract

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_detects_network_up_event() {
        // Given: Mock D-Bus with NetworkManager
        let mock_dbus = MockDBus::new();
        mock_dbus.register_network_manager();

        // When: NetworkManager emits StateChanged to connected
        let monitor = NetworkMonitor::new().await.unwrap();
        let mut events = monitor.start();
        mock_dbus.emit_state_changed(NM_STATE_CONNECTED_GLOBAL);

        // Then: NetworkUp event received
        let event = events.recv().await.unwrap();
        assert_matches!(event, NetworkEvent::NetworkUp { .. });
    }

    #[tokio::test]
    async fn test_detects_interface_change() {
        // Given: Connected to WiFi interface wlan0
        let monitor = setup_connected_monitor("wlan0").await;
        let mut events = monitor.start();

        // When: Interface switches to wlan1
        mock_interface_change("wlan0", "wlan1").await;

        // Then: InterfaceChanged event received
        let event = events.recv().await.unwrap();
        assert_matches!(
            event,
            NetworkEvent::InterfaceChanged {
                old_interface, new_interface
            } if old_interface == "wlan0" && new_interface == "wlan1"
        );
    }

    #[tokio::test]
    async fn test_handles_dbus_disconnection() {
        // Given: Monitor connected to D-Bus
        let monitor = NetworkMonitor::new().await.unwrap();

        // When: D-Bus connection dropped
        drop_dbus_connection();

        // Then: Monitor attempts reconnection (logs warning)
        // And: Eventually reconnects or returns error
        assert!(monitor.is_network_available().await.is_err());
    }
}
```

### Integration Tests

- Test against real NetworkManager on test system
- Verify all signal types are correctly parsed
- Test behavior during actual suspend/resume cycle

## Dependencies

```toml
[dependencies]
zbus = "4.0"
tokio = { version = "1.35", features = ["sync"] }
thiserror = "1.0"
tracing = "0.1"
```

## Usage Example

```rust
use akon_core::vpn::{NetworkMonitor, NetworkEvent};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let monitor = NetworkMonitor::new().await?;
    let mut events = monitor.start();

    while let Some(event) = events.recv().await {
        match event {
            NetworkEvent::NetworkDown { interface } => {
                tracing::info!("Network down on {interface}");
                // Trigger stale connection detection
            }
            NetworkEvent::SystemResumed => {
                tracing::info!("System resumed from suspend");
                // Check VPN connection status
            }
            _ => {}
        }
    }

    Ok(())
}
```

## Performance Requirements

- **Event latency**: < 1 second from D-Bus signal to NetworkEvent emission
- **CPU overhead**: < 0.1% when idle (no events)
- **Memory overhead**: < 1MB for monitor task
- **Reconnection time**: < 5 seconds after D-Bus disconnection

## Security Considerations

- **D-Bus permissions**: Requires read access to system bus (standard for user applications)
- **Signal validation**: Validate signal source before processing (prevent spoofing)
- **No credential exposure**: Monitor never accesses VPN credentials
