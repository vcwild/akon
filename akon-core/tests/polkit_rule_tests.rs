//! Static-content checks for the shipped polkit rule (spec 009).
//!
//! These lock the rule's scope so it can never silently widen: it must grant
//! exactly the four resolve1 DNS actions akon needs, only for local active
//! sessions, and nothing else.

use std::path::PathBuf;

fn rule_source() -> String {
    // The rule lives at the repo root under packaging/polkit/.
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .join("packaging/polkit/49-akon-resolved-dns.rules");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

#[test]
fn rule_grants_exactly_the_four_resolve1_dns_actions() {
    let src = rule_source();
    for action in &[
        "org.freedesktop.resolve1.set-dns-servers",
        "org.freedesktop.resolve1.set-domains",
        "org.freedesktop.resolve1.set-default-route",
        "org.freedesktop.resolve1.revert",
    ] {
        assert!(src.contains(action), "polkit rule must reference {action}");
    }
}

#[test]
fn rule_is_scoped_to_local_active_sessions() {
    let src = rule_source();
    assert!(
        src.contains("subject.local"),
        "rule must be limited to local sessions"
    );
    assert!(
        src.contains("subject.active"),
        "rule must be limited to active sessions"
    );
}

#[test]
fn rule_does_not_grant_unrelated_or_blanket_actions() {
    let src = rule_source();
    // No blanket admin / wildcard grant.
    assert!(
        !src.contains("org.freedesktop.resolve1.*"),
        "rule must not use a wildcard action"
    );
    // Only resolve1 actions are granted — guard against accidentally adding
    // login1/systemd1/NetworkManager actions to the YES branch.
    for forbidden in &[
        "org.freedesktop.systemd1",
        "org.freedesktop.login1",
        "org.freedesktop.NetworkManager",
        "org.freedesktop.resolve1.set-dnssec",
        "org.freedesktop.resolve1.register-service",
    ] {
        assert!(
            !src.contains(forbidden),
            "rule must not reference {forbidden}"
        );
    }
    // Exactly one YES result (a single grant branch).
    assert_eq!(
        src.matches("polkit.Result.YES").count(),
        1,
        "rule should contain exactly one YES grant"
    );
}
