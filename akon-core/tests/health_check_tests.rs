use akon_core::vpn::health_check::HealthChecker;
use std::time::Duration;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

/// Test successful health check with HTTP 200 response
#[tokio::test]
async fn test_successful_health_check_with_200() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200).set_body_string("OK"))
        .mount(&mock_server)
        .await;

    let endpoint = format!("{}/health", mock_server.uri());
    let health_checker = HealthChecker::new(endpoint, Duration::from_secs(5)).unwrap();

    let result = health_checker.check().await;

    assert!(result.is_success());
    assert!(result.error().is_none());
}

/// Test health check fails on timeout
#[tokio::test]
async fn test_health_check_fails_on_timeout() {
    let mock_server = MockServer::start().await;

    // Respond with a delay longer than the timeout
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(10)))
        .mount(&mock_server)
        .await;

    let endpoint = format!("{}/health", mock_server.uri());
    let health_checker = HealthChecker::new(endpoint, Duration::from_secs(1)).unwrap();

    let result = health_checker.check().await;

    assert!(!result.is_success());
    assert!(result.error().is_some());
    assert!(result
        .error()
        .unwrap()
        .to_string()
        .to_lowercase()
        .contains("timeout"));
}

/// Test health check fails on 4xx status codes
#[tokio::test]
async fn test_health_check_fails_on_4xx_status() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&mock_server)
        .await;

    let endpoint = format!("{}/health", mock_server.uri());
    let health_checker = HealthChecker::new(endpoint, Duration::from_secs(5)).unwrap();

    let result = health_checker.check().await;

    assert!(!result.is_success());
}

/// Test health check fails on 5xx status codes
#[tokio::test]
async fn test_health_check_fails_on_5xx_status() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&mock_server)
        .await;

    let endpoint = format!("{}/health", mock_server.uri());
    let health_checker = HealthChecker::new(endpoint, Duration::from_secs(5)).unwrap();

    let result = health_checker.check().await;

    assert!(!result.is_success());
}

/// Test is_reachable returns true for any response (even error status codes)
#[tokio::test]
async fn test_is_reachable_true_for_any_response() {
    let mock_server = MockServer::start().await;

    // Even with 500 status, the endpoint is "reachable"
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock_server)
        .await;

    let endpoint = format!("{}/health", mock_server.uri());
    let health_checker = HealthChecker::new(endpoint, Duration::from_secs(5)).unwrap();

    let is_reachable = health_checker.is_reachable().await;

    assert!(is_reachable);
}

/// Test is_reachable returns false for connection refused
#[tokio::test]
async fn test_is_reachable_false_for_connection_refused() {
    // Use an endpoint that doesn't exist (nothing listening on this port)
    let endpoint = "http://127.0.0.1:59999/health".to_string();
    let health_checker = HealthChecker::new(endpoint, Duration::from_secs(1)).unwrap();

    let is_reachable = health_checker.is_reachable().await;

    assert!(!is_reachable);
}
