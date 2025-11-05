# HealthChecker Interface Contract

**Module**: `akon-core/src/vpn/health_check.rs`
**Purpose**: Verify VPN connectivity via HTTP/HTTPS requests

## Interface

```rust
use std::time::Duration;
use reqwest::Client;

/// Performs HTTP/HTTPS health checks to verify VPN connectivity
pub struct HealthChecker {
    client: Client,
    endpoint: String,
    timeout: Duration,
}

impl HealthChecker {
    /// Create a new health checker
    ///
    /// # Arguments
    /// * `endpoint` - HTTP/HTTPS URL to check (e.g., "https://vpn.example.com/healthz")
    /// * `timeout` - Maximum time to wait for response (default: 5 seconds)
    ///
    /// # Errors
    /// Returns error if endpoint URL is invalid
    pub fn new(endpoint: String, timeout: Option<Duration>) -> Result<Self, HealthCheckError>;

    /// Perform a single health check
    ///
    /// Sends GET request to configured endpoint and evaluates response.
    /// Check is considered successful if:
    /// - Request completes within timeout
    /// - HTTP status code is 2xx or 3xx
    /// - No network errors occur
    ///
    /// # Returns
    /// Result with health check details (success, status, duration, error)
    pub async fn check(&self) -> HealthCheckResult;

    /// Check if endpoint is reachable (for network stability detection)
    ///
    /// Similar to check() but only cares about reachability, not response details.
    /// Used to determine if network is stable enough to attempt VPN reconnection.
    ///
    /// # Returns
    /// true if endpoint responds with any HTTP status, false if unreachable
    pub async fn is_reachable(&self) -> bool;
}

#[derive(Debug, thiserror::Error)]
pub enum HealthCheckError {
    #[error("Invalid endpoint URL: {0}")]
    InvalidUrl(String),

    #[error("HTTP client creation failed: {0}")]
    ClientCreationFailed(#[from] reqwest::Error),
}
```

## Behavior Specification

### Success Criteria

A health check is considered **successful** if:

1. **Request completes**: Response received within timeout period
2. **Status code**: HTTP 2xx (200-299) or 3xx (300-399)
3. **No errors**: No connection refused, timeout, DNS failure, TLS errors

### Failure Criteria

A health check **fails** if:

1. **Timeout**: No response within configured timeout (5s)
2. **Connection error**: TCP connection refused, network unreachable
3. **DNS error**: Cannot resolve endpoint hostname
4. **TLS error**: Certificate validation failure (if HTTPS)
5. **HTTP error**: Status code 4xx or 5xx

### Edge Cases

- **Redirects**: Follow redirects (up to 3) - final status code determines success
- **Empty response body**: Acceptable - only status code matters
- **Chunked encoding**: Accept but don't read full body (HEAD request would be ideal)
- **Slow headers**: Count against timeout - don't wait forever for headers

## Testing Contract

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{MockServer, Mock, ResponseTemplate};
    use wiremock::matchers::method;

    #[tokio::test]
    async fn test_successful_health_check_with_200() {
        // Given: Mock server returning 200 OK
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        let checker = HealthChecker::new(
            mock_server.uri(),
            Some(Duration::from_secs(5))
        ).unwrap();

        // When: Health check performed
        let result = checker.check().await;

        // Then: Check succeeds with 200 status
        assert!(result.is_healthy());
        assert_eq!(result.status_code, Some(200));
        assert!(result.duration < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn test_health_check_fails_on_timeout() {
        // Given: Mock server with 10s delay
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(10)))
            .mount(&mock_server)
            .await;

        let checker = HealthChecker::new(
            mock_server.uri(),
            Some(Duration::from_secs(2))
        ).unwrap();

        // When: Health check performed
        let result = checker.check().await;

        // Then: Check fails due to timeout
        assert!(!result.success);
        assert!(result.error.is_some());
        assert!(result.error.unwrap().contains("timeout"));
    }

    #[tokio::test]
    async fn test_health_check_fails_on_4xx_status() {
        // Given: Mock server returning 404
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;

        let checker = HealthChecker::new(mock_server.uri(), None).unwrap();

        // When: Health check performed
        let result = checker.check().await;

        // Then: Check fails with 404 status
        assert!(!result.is_healthy());
        assert_eq!(result.status_code, Some(404));
    }

    #[tokio::test]
    async fn test_is_reachable_true_for_any_response() {
        // Given: Mock server returning 500 (error but reachable)
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&mock_server)
            .await;

        let checker = HealthChecker::new(mock_server.uri(), None).unwrap();

        // When: Checking reachability
        let reachable = checker.is_reachable().await;

        // Then: Endpoint is reachable (got response even if error)
        assert!(reachable);
    }

    #[tokio::test]
    async fn test_is_reachable_false_for_connection_refused() {
        // Given: Endpoint that doesn't exist
        let checker = HealthChecker::new(
            "http://127.0.0.1:1".to_string(), // Nothing listening here
            Some(Duration::from_millis(100))
        ).unwrap();

        // When: Checking reachability
        let reachable = checker.is_reachable().await;

        // Then: Endpoint is not reachable
        assert!(!reachable);
    }
}
```

### Integration Tests

- Test against real HTTPS endpoints
- Verify TLS certificate validation
- Test behavior behind actual VPN connection

## Dependencies

```toml
[dependencies]
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls"] }
tokio = { version = "1.35", features = ["time"] }
thiserror = "1.0"
tracing = "0.1"

[dev-dependencies]
wiremock = "0.6"
```

## Usage Example

```rust
use akon_core::vpn::HealthChecker;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let checker = HealthChecker::new(
        "https://vpn.example.com/healthz".to_string(),
        Some(Duration::from_secs(5))
    )?;

    // Check if VPN is healthy
    let result = checker.check().await;
    if result.is_healthy() {
        tracing::info!(
            "Health check passed: HTTP {} in {:?}",
            result.status_code.unwrap(),
            result.duration
        );
    } else {
        tracing::warn!(
            "Health check failed: {:?}",
            result.error
        );
    }

    // Check network stability before reconnection
    if checker.is_reachable().await {
        tracing::info!("Network stable, can attempt reconnection");
    } else {
        tracing::info!("Network not stable, waiting...");
    }

    Ok(())
}
```

## Performance Requirements

- **Check duration**: Complete within 5 seconds (including timeout)
- **CPU overhead**: Minimal - one HTTP client per checker instance
- **Memory overhead**: < 100KB per checker instance
- **Concurrent checks**: Support multiple checkers in parallel without blocking

## Security Considerations

- **TLS verification**: Always validate certificates (no insecure mode)
- **Timeout enforcement**: Prevent hanging on slow/malicious endpoints
- **URL validation**: Reject non-HTTP(S) schemes (file://, ftp://, etc.)
- **No credential leaking**: Never log full URLs if they contain auth tokens
- **User-Agent**: Set generic user agent to avoid fingerprinting

## Configuration Integration

```toml
# ~/.config/akon/config.toml
[reconnection]
health_check_endpoint = "https://vpn.example.com/healthz"
health_check_interval_secs = 60
health_check_timeout_secs = 5
```

## Logging

```rust
// Success
tracing::debug!(
    endpoint = %self.endpoint,
    status = result.status_code,
    duration_ms = result.duration.as_millis(),
    "Health check successful"
);

// Failure
tracing::warn!(
    endpoint = %self.endpoint,
    error = %result.error.as_ref().unwrap(),
    "Health check failed"
);

// Reachability check
tracing::trace!(
    endpoint = %self.endpoint,
    reachable = reachable,
    "Network reachability check"
);
```
