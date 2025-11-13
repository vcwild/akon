use akon_core::auth::keyring;
use akon_core::types::Pin;

// This integration test requires the mock keyring to be enabled via the
// `mock-keyring` feature. Run with:
//
// cargo test -p akon-core --test integration_keyring_tests --features mock-keyring
//
#[cfg(feature = "mock-keyring")]
#[test]
fn integration_long_pin_truncation() {
    let username = "integration_long_pin_user";

    // Clean up any existing entries
    let _ = keyring::delete_pin(username);
    let _ = keyring::delete_otp_secret(username);

    // Create and store a long PIN (>30 chars)
    let long_pin = "abcdefghijklmnopqrstuvwxyz0123456789".to_string(); // 36 chars
    let pin = Pin::from_unchecked(long_pin.clone());
    keyring::store_pin(username, &pin).expect("Failed to store long PIN");

    // Store a valid OTP secret
    let otp = "JBSWY3DPEHPK3PXP"; // valid base32
    keyring::store_otp_secret(username, otp).expect("Failed to store OTP secret");

    // Generate password
    let res = akon_core::auth::password::generate_password(username);
    assert!(res.is_ok(), "generate_password failed: {:?}", res.err());

    let password = res.unwrap();
    let pwd = password.expose();

    // Expect the password to start with the first 30 chars of the stored PIN
    let expected_prefix: String = long_pin.chars().take(30).collect();
    assert!(
        pwd.starts_with(&expected_prefix),
        "Password prefix mismatch: {} vs {}",
        pwd,
        expected_prefix
    );

    // Clean up
    let _ = keyring::delete_pin(username);
    let _ = keyring::delete_otp_secret(username);
}
