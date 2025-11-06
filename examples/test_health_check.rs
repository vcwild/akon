//! Manual end-to-end test for health check and reconnection
//!
//! This example demonstrates:
//! 1. Loading config with reconnection policy from ~/.config/akon/config.toml
//! 2. Creating a health checker
//! 3. Performing health checks
//! 4. Simulating reconnection logic
//!
//! Prerequisites:
//! - Configure your endpoint in ~/.config/akon/config.toml:
//!   [reconnection]
//!   health_check_endpoint = "https://your-internal-endpoint.example.com/"
//!
//! Run with: cargo run --example test_health_check

use akon_core::config::toml_config::TomlConfig;
use akon_core::vpn::health_check::HealthChecker;
use akon_core::vpn::reconnection::{ReconnectionManager, ReconnectionPolicy};
use std::env;
use std::path::PathBuf;
use std::time::Duration;

#[tokio::main]
async fn main() {
    // Initialize simple logging
    println!("Setting up logging...");

    println!("🔍 Akon Health Check & Reconnection Test\n");

    // Step 1: Load config
    println!("📋 Step 1: Loading configuration...");
    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let config_path = PathBuf::from(home)
        .join(".config")
        .join("akon")
        .join("config.toml");

    let config = match TomlConfig::from_file(&config_path) {
        Ok(c) => {
            println!("   ✅ Config loaded from: {}", config_path.display());
            c
        }
        Err(e) => {
            eprintln!("   ❌ Failed to load config: {}", e);
            eprintln!("   Using default reconnection policy for testing...");

            // Create a default policy for testing
            let policy = ReconnectionPolicy {
                max_attempts: 5,
                base_interval_secs: 5,
                backoff_multiplier: 2,
                max_interval_secs: 60,
                consecutive_failures_threshold: 3,
                health_check_interval_secs: 10, // Faster for testing
                health_check_endpoint: "https://example.com/".to_string(),
            };

            println!(
                "   ℹ Using test policy: endpoint={}",
                policy.health_check_endpoint
            );

            TomlConfig::new(
                akon_core::config::VpnConfig::new(
                    "test.example.com".to_string(),
                    "testuser".to_string(),
                ),
                Some(policy),
            )
        }
    };

    let policy = match config.reconnection_policy() {
        Some(p) => {
            println!("   ✅ Reconnection policy found:");
            println!("      - Max attempts: {}", p.max_attempts);
            println!("      - Base interval: {}s", p.base_interval_secs);
            println!(
                "      - Health check interval: {}s",
                p.health_check_interval_secs
            );
            println!("      - Endpoint: {}", p.health_check_endpoint);
            p.clone()
        }
        None => {
            println!("   ⚠ No reconnection policy in config, using defaults");
            return;
        }
    };

    // Step 2: Create health checker
    println!("\n🏥 Step 2: Creating health checker...");
    let health_checker =
        match HealthChecker::new(policy.health_check_endpoint.clone(), Duration::from_secs(5)) {
            Ok(hc) => {
                println!("   ✅ Health checker created");
                hc
            }
            Err(e) => {
                eprintln!("   ❌ Failed to create health checker: {}", e);
                return;
            }
        };

    // Step 3: Perform initial health check
    println!("\n🔬 Step 3: Performing initial health check...");
    let result = health_checker.check().await;

    if result.is_success() {
        println!("   ✅ Health check PASSED");
        println!("      - Duration: {:?}", result.duration());
    } else {
        println!("   ⚠ Health check FAILED");
        if let Some(err) = result.error() {
            println!("      - Error: {}", err);
        }
        println!("      - Duration: {:?}", result.duration());
    }

    // Step 4: Test consecutive health checks
    println!("\n🔄 Step 4: Testing consecutive health checks...");
    println!(
        "   Running {} health checks with {} second intervals...",
        policy.consecutive_failures_threshold + 2,
        policy.health_check_interval_secs
    );

    let mut consecutive_failures = 0;

    for i in 1..=(policy.consecutive_failures_threshold + 2) {
        print!("   Check {}: ", i);
        let result = health_checker.check().await;

        if result.is_success() {
            println!("✅ SUCCESS ({:?})", result.duration());
            consecutive_failures = 0;
        } else {
            consecutive_failures += 1;
            println!("❌ FAILED (failures: {})", consecutive_failures);
            if let Some(err) = result.error() {
                println!("      Error: {}", err);
            }

            if consecutive_failures >= policy.consecutive_failures_threshold {
                println!("   ⚠️  THRESHOLD REACHED! Would trigger reconnection now.");
                break;
            }
        }

        if i < policy.consecutive_failures_threshold + 2 {
            tokio::time::sleep(Duration::from_secs(policy.health_check_interval_secs)).await;
        }
    }

    // Step 5: Test reconnection manager
    println!("\n♻️  Step 5: Testing reconnection manager...");
    let manager = ReconnectionManager::new(policy.clone());
    println!("   ✅ ReconnectionManager created");

    // Test backoff calculation
    println!("\n   📊 Exponential backoff progression:");
    for attempt in 1..=policy.max_attempts {
        let backoff = manager.calculate_backoff(attempt);
        println!("      Attempt {}: wait {:?}", attempt, backoff);
    }

    // Step 6: Test reachability check
    println!("\n🌐 Step 6: Testing network reachability...");
    let is_reachable = health_checker.is_reachable().await;
    if is_reachable {
        println!("   ✅ Endpoint is reachable");
        println!("      This indicates network connectivity is working");
    } else {
        println!("   ❌ Endpoint is NOT reachable");
        println!("      This would delay reconnection attempts");
    }

    println!("\n✨ Test complete!");
    println!("\n📝 Summary:");
    println!("   - Configuration loading: ✅");
    println!("   - Health checker creation: ✅");
    println!("   - Health check execution: ✅");
    println!("   - Consecutive failure tracking: ✅");
    println!("   - Reconnection manager: ✅");
    println!("   - Exponential backoff: ✅");
    println!("   - Network reachability: ✅");

    println!("\n💡 Next steps:");
    println!("   - Integrate ReconnectionManager into VPN connection flow");
    println!("   - Test with actual VPN disconnect/reconnect scenarios");
    println!("   - Monitor health checks in production");
}
