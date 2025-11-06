//! VPN connectivity health checking via HTTP/HTTPS
//!
//! This module provides HealthChecker for verifying VPN connectivity
//! through periodic HTTP/HTTPS requests to a configured endpoint.

use reqwest::Client;
use std::time::{Duration, Instant};
use tracing::{debug, warn};
use url::Url;

/// Result of a health check attempt
#[derive(Debug, Clone)]
pub struct HealthCheckResult {
    success: bool,
    duration: Duration,
    error: Option<String>,
}

impl HealthCheckResult {
    /// Create a successful health check result
    pub fn success(duration: Duration) -> Self {
        Self {
            success: true,
            duration,
            error: None,
        }
    }

    /// Create a failed health check result
    pub fn failure(duration: Duration, error: String) -> Self {
        Self {
            success: false,
            duration,
            error: Some(error),
        }
    }

    /// Check if the health check was successful
    pub fn is_success(&self) -> bool {
        self.success
    }

    /// Get the duration of the health check
    pub fn duration(&self) -> Duration {
        self.duration
    }

    /// Get the error message if the check failed
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

/// Performs HTTP/HTTPS health checks to verify VPN connectivity
#[derive(Debug)]
pub struct HealthChecker {
    client: Client,
    endpoint: String,
    timeout: Duration,
}

/// Errors that can occur during health check operations
#[derive(Debug, thiserror::Error)]
pub enum HealthCheckError {
    #[error("Invalid endpoint URL: {0}")]
    InvalidUrl(String),

    #[error("HTTP client creation failed: {0}")]
    ClientCreationFailed(#[from] reqwest::Error),
}

impl HealthChecker {
    /// Create a new health checker
    ///
    /// # Arguments
    /// * `endpoint` - HTTP/HTTPS URL to check (must use http:// or https:// scheme)
    /// * `timeout` - Maximum duration to wait for a response
    ///
    /// # Returns
    /// * `Ok(HealthChecker)` if the endpoint URL is valid
    /// * `Err(HealthCheckError)` if the URL is invalid or doesn't use HTTP/HTTPS
    #[tracing::instrument(skip(timeout), fields(endpoint = %endpoint, timeout_ms = timeout.as_millis()))]
    pub fn new(endpoint: String, timeout: Duration) -> Result<Self, HealthCheckError> {
        // Validate endpoint URL
        let url = Url::parse(&endpoint)
            .map_err(|e| HealthCheckError::InvalidUrl(format!("Failed to parse URL: {}", e)))?;

        // Ensure scheme is HTTP or HTTPS
        match url.scheme() {
            "http" | "https" => {}
            scheme => {
                return Err(HealthCheckError::InvalidUrl(format!(
                    "Only HTTP/HTTPS schemes are supported, got: {}",
                    scheme
                )));
            }
        }

        // Create HTTP client with rustls-tls
        let client = Client::builder()
            .timeout(timeout)
            .use_rustls_tls()
            .build()
            .map_err(|e| {
                HealthCheckError::InvalidUrl(format!("Failed to create HTTP client: {}", e))
            })?;

        Ok(Self {
            client,
            endpoint,
            timeout,
        })
    }

    /// Perform a health check
    ///
    /// Sends a GET request to the configured endpoint and measures the response time.
    /// A check is considered successful if:
    /// - The endpoint responds within the timeout
    /// - The response status code is 2xx or 3xx
    ///
    /// # Returns
    /// * `HealthCheckResult` containing success status, duration, and any error
    #[tracing::instrument(skip(self), fields(endpoint = %self.endpoint))]
    pub async fn check(&self) -> HealthCheckResult {
        let start = Instant::now();

        match self.client.get(&self.endpoint).send().await {
            Ok(response) => {
                let duration = start.elapsed();
                let status = response.status();

                if status.is_success() || status.is_redirection() {
                    debug!(
                        endpoint = %self.endpoint,
                        status = %status,
                        duration_ms = duration.as_millis(),
                        "Health check succeeded"
                    );
                    HealthCheckResult::success(duration)
                } else {
                    warn!(
                        endpoint = %self.endpoint,
                        status = %status,
                        duration_ms = duration.as_millis(),
                        "Health check failed with error status"
                    );
                    HealthCheckResult::failure(
                        duration,
                        format!("Unhealthy status code: {}", status),
                    )
                }
            }
            Err(e) => {
                let duration = start.elapsed();
                let error_msg = if e.is_timeout() {
                    format!("Request timeout after {:?}", self.timeout)
                } else if e.is_connect() {
                    "Connection refused or unreachable".to_string()
                } else {
                    format!("Request failed: {}", e)
                };

                warn!(
                    endpoint = %self.endpoint,
                    error = %error_msg,
                    duration_ms = duration.as_millis(),
                    "Health check failed"
                );

                HealthCheckResult::failure(duration, error_msg)
            }
        }
    }

    /// Check if the endpoint is reachable
    ///
    /// This is a lighter check that only verifies network connectivity.
    /// Returns `true` if the endpoint responds with ANY status code (even errors),
    /// and `false` only if there's a network-level failure (connection refused, timeout, DNS failure).
    ///
    /// Use this to determine if the network is stable enough to attempt reconnection.
    ///
    /// # Returns
    /// * `true` if the endpoint is reachable (any HTTP response received)
    /// * `false` if there's a network-level failure
    #[tracing::instrument(skip(self), fields(endpoint = %self.endpoint))]
    pub async fn is_reachable(&self) -> bool {
        match self.client.get(&self.endpoint).send().await {
            Ok(_) => {
                // Any response means the endpoint is reachable
                true
            }
            Err(e) => {
                // Network-level errors mean unreachable
                if e.is_timeout() || e.is_connect() {
                    false
                } else {
                    // Other errors (like invalid response) still mean reachable
                    true
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_health_checker_new_valid_http() {
        let result = HealthChecker::new(
            "http://example.com/health".to_string(),
            Duration::from_secs(5),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_health_checker_new_valid_https() {
        let result = HealthChecker::new(
            "https://example.com/health".to_string(),
            Duration::from_secs(5),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_health_checker_new_invalid_scheme() {
        let result = HealthChecker::new(
            "ftp://example.com/health".to_string(),
            Duration::from_secs(5),
        );
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Only HTTP/HTTPS schemes"));
    }

    #[test]
    fn test_health_checker_new_invalid_url() {
        let result = HealthChecker::new("not a url".to_string(), Duration::from_secs(5));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("parse URL"));
    }

    #[test]
    fn test_health_check_result_success() {
        let result = HealthCheckResult::success(Duration::from_millis(123));
        assert!(result.is_success());
        assert_eq!(result.duration(), Duration::from_millis(123));
        assert!(result.error().is_none());
    }

    #[test]
    fn test_health_check_result_failure() {
        let result = HealthCheckResult::failure(Duration::from_millis(456), "timeout".to_string());
        assert!(!result.is_success());
        assert_eq!(result.duration(), Duration::from_millis(456));
        assert_eq!(result.error(), Some("timeout"));
    }
}
