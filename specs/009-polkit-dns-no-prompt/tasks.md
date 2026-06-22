---
description: "Task list for 009-polkit-dns-no-prompt"
---

# Tasks: No password prompts during `akon vpn on` (DNS via polkit)

**Branch**: `009-polkit-dns-no-prompt`

## Format: `[ID] [P?] [Story] Description`

---

## Phase 1: The polkit rule (Story 1 + 2)

- [x] T001 [US1/US2] Create `packaging/polkit/49-akon-resolved-dns.rules` — a
      polkit JS rule returning `polkit.Result.YES` for
      `org.freedesktop.resolve1.{set-dns-servers,set-domains,set-default-route,revert}`
      ONLY when `subject.local && subject.active`.
- [x] T002 [P] [US2] Add a unit test (in `akon-core/tests/polkit_rule_tests.rs`)
      that reads the shipped rule file and asserts: it contains exactly the four
      intended action ids, contains the `local` and `active` guards, and does NOT
      contain `Result.YES` for any other action / a blanket grant.

**Checkpoint**: the rule exists and its content is locked by a test.

---

## Phase 2: Packaging install/uninstall (Story 1)

- [x] T003 [US1] Add the rule to the deb assets (`Cargo.toml`
      `[package.metadata.deb] assets`) → install to
      `/usr/share/polkit-1/rules.d/49-akon-resolved-dns.rules`.
- [x] T004 [P] [US1] Add the rule to the rpm assets
      (`[package.metadata.generate-rpm] assets`) → same path.
- [x] T005 [P] [US1] `make install`: copy the rule to the polkit rules dir;
      `make` notes it requires root (like setcap). Document.
- [x] T006 [US1] Removal on uninstall: deb `postrm` and rpm `post-uninstall.sh`
      remove `/usr/share/polkit-1/rules.d/49-akon-resolved-dns.rules`.

**Checkpoint**: package install/uninstall manages the rule.

---

## Phase 3: Best-effort, non-blocking DNS apply (Story 1 hardening, FR-005/006)

- [x] T007 [US1] In `akon-core/src/vpn/f5/dns.rs`, downgrade the primary
      `set-dns` failure from `Err(...)` (which aborts) to a WARN + `Ok(())` so a
      DNS failure never tears down a working tunnel; the connection proceeds with
      a visible warning.
- [x] T008 [P] [US1] Ensure resolvectl invocations cannot hang on an interactive
      agent (document/verify `resolvectl` returns promptly when polkit denies in
      a non-interactive context; no code change if it already fails fast).

---

## Phase 4: Docs + verification

- [x] T009 Add `quickstart.md`: how to verify no prompts (with the rule) and the
      manual install command for from-source users
      (`sudo install -m 644 packaging/polkit/49-akon-resolved-dns.rules
      /usr/share/polkit-1/rules.d/`).
- [x] T010 Update README requirements/notes: akon ships a polkit rule so DNS
      applies without prompting; from-source users run `make install` (or copy
      the rule). Capture an ADR for the polkit-rule decision.
- [x] T011 `cargo fmt --check` + `cargo clippy --workspace --all-targets
      --features test-actors -- -D warnings` clean (1.96); full CI-equivalent
      `cargo test --workspace --features test-actors` green.
- [x] T012 Manual: on a real desktop, install the rule, run `akon vpn on` →
      confirm ZERO password prompts and that VPN-only names resolve.

---

## Dependencies

T001 → T002 (test reads the rule), T003/T004/T005 (package the rule), T006.
T007 independent (can run in parallel with packaging). T009/T010 after T001.
T011/T012 last.

## Parallel opportunities

T002 ∥ T007. T003 ∥ T004 ∥ T005 once T001 exists.
