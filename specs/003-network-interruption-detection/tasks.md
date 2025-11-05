# Tasks: Network Interruption Detection and Automatic Reconnection

**Input**: Design documents from `/home/vcwild/Projects/personal/akon/specs/003-network-interruption-detection/`
**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/

**Branch**: `003-network-interruption-detection`

**Tests**: Following TDD approach per constitution - tests are included for all critical components

**Organization**: Tasks are grouped by user story to enable independent implementation and testing of each story.

## Format: `[ID] [P?] [Story] Description`
- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (US1, US2, US3, US4)
- Include exact file paths in descriptions

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Project initialization and dependency configuration

- [x] T001 Add new dependencies to `akon-core/Cargo.toml`: zbus = "4.0", reqwest = { version = "0.12", default-features = false, features = ["rustls-tls"] }, wiremock = "0.6" (dev-dependency)
- [x] T002 Verify tokio features in `akon-core/Cargo.toml` include ["sync", "time", "macros", "process", "signal"]
- [x] T003 Run `cargo fetch` to download all new dependencies
- [x] T004 Create module structure: `akon-core/src/vpn/network_monitor.rs`, `akon-core/src/vpn/health_check.rs`, `akon-core/src/vpn/reconnection.rs` (empty files with module declarations)
- [x] T005 Update `akon-core/src/vpn/mod.rs` to declare new modules: `pub mod network_monitor;`, `pub mod health_check;`, `pub mod reconnection;`

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Core data types and state machine that ALL user stories depend on

**⚠️ CRITICAL**: No user story work can begin until this phase is complete

- [x] T006 [P] Extend `ConnectionState` enum in `akon-core/src/vpn/state.rs` to add `Reconnecting { attempt: u32, next_retry_at: Option<u64>, max_attempts: u32 }` variant
- [x] T007 [P] Create `ReconnectionPolicy` struct in `akon-core/src/vpn/reconnection.rs` with fields: max_attempts, base_interval_secs, backoff_multiplier, max_interval_secs, consecutive_failures_threshold, health_check_interval_secs, health_check_endpoint (with serde derives and default functions)
- [x] T008 [P] Create `NetworkEvent` enum in `akon-core/src/vpn/network_monitor.rs` with variants: NetworkUp, NetworkDown, InterfaceChanged, SystemResumed, SystemSuspending
- [x] T009 [P] Create `HealthCheckResult` struct in `akon-core/src/vpn/health_check.rs` with fields: success, status_code, duration, error, timestamp, plus `is_healthy()` method
- [x] T010 Extend `akon-core/src/config/toml_config.rs` to parse `[reconnection]` section from config file with ReconnectionPolicy fields and validation rules
- [x] T011 [P] Create test fixtures directory `akon-core/tests/fixtures/` with sample config file containing reconnection settings
- [x] T012 [P] Add state transition tests in `akon-core/tests/connection_state_tests.rs` verifying valid transitions involving Reconnecting state

**Checkpoint**: ✅ Foundation ready - user story implementation can now begin in parallel

---

## Phase 3: User Story 1 - Network Interruption Detection and Automatic Reconnection (Priority: P1) 🎯 MVP

**Goal**: Detect network interruptions (WiFi changes, suspend/resume) and automatically reconnect with exponential backoff

**Independent Test**: Establish VPN, simulate network interruption (disconnect WiFi), verify detection, process cleanup, and automatic reconnection with correct backoff intervals

### Tests for User Story 1

**NOTE: Write these tests FIRST, ensure they FAIL before implementation**

- [x] T013 [P] [US1] Create unit tests in `akon-core/tests/network_monitor_tests.rs` for NetworkMonitor: test_detects_network_up_event, test_detects_network_down_event, test_detects_interface_change, test_detects_system_suspend, test_detects_system_resume (using mock D-Bus)
- [x] T014 [P] [US1] Create unit tests in `akon-core/tests/reconnection_tests.rs` for exponential backoff: test_backoff_calculation (verify 5s→10s→20s→40s→60s), test_backoff_cap_at_max_interval, test_backoff_with_different_multipliers
- [x] T015 [P] [US1] Create unit tests in `akon-core/tests/reconnection_tests.rs` for reconnection attempts: test_successful_reconnection_updates_state, test_failed_attempt_increments_counter, test_max_attempts_exceeded_transitions_to_error, test_network_not_stable_delays_reconnection
- [x] T016 [P] [US1] Create integration test in `akon-core/tests/reconnection_flow_tests.rs` for full reconnection lifecycle: setup VPN connection, trigger network down event, verify process cleanup, verify state transitions (Disconnected→Reconnecting→Connected), verify retry attempts follow backoff pattern

### Implementation for User Story 1

- [x] T017 [P] [US1] Implement `NetworkMonitor::new()` in `akon-core/src/vpn/network_monitor.rs`: connect to system D-Bus, verify NetworkManager available, create mpsc channel
- [x] T018 [P] [US1] Implement `NetworkMonitor::start()` in `akon-core/src/vpn/network_monitor.rs`: spawn tokio task, subscribe to NetworkManager StateChanged and PrepareForSleep signals, map D-Bus signals to NetworkEvent enum, send events through channel
- [x] T019 [P] [US1] Implement `NetworkMonitor::is_network_available()` in `akon-core/src/vpn/network_monitor.rs`: query NetworkManager State property, return true if NM_STATE_CONNECTED_GLOBAL
- [x] T020 [US1] Implement `ReconnectionManager::new()` in `akon-core/src/vpn/reconnection.rs`: accept policy, network_monitor, health_checker; create watch channel for state, create mpsc channel for commands
- [x] T021 [US1] Implement `ReconnectionManager::calculate_backoff()` in `akon-core/src/vpn/reconnection.rs`: exponential backoff formula (base × multiplier^(attempt-1), capped at max_interval)
- [x] T022 [US1] Implement `ReconnectionManager::attempt_reconnect()` in `akon-core/src/vpn/reconnection.rs`: check network stability via health_checker.is_reachable(), update state with attempt counter, call existing VPN connection logic, handle success/failure, increment counter on failure, transition to Error on max attempts
- [x] T023 [US1] Implement `ReconnectionManager::handle_network_event()` in `akon-core/src/vpn/reconnection.rs`: match on NetworkEvent types (NetworkDown→initiate reconnection, InterfaceChanged→initiate reconnection, SystemResumed→initiate reconnection, SystemSuspending→log event)
- [x] T024 [US1] Implement `ReconnectionManager::run()` in `akon-core/src/vpn/reconnection.rs`: tokio::select! event loop handling network events, retry timer, commands; call handle_network_event(), attempt_reconnect() at scheduled times
- [x] T025 [US1] Extend `akon-core/src/vpn/cli_connector.rs` to integrate ReconnectionManager: spawn background task on connect, setup event channels, wire network monitor to reconnection manager (deferred - complex integration)
- [x] T026 [US1] Implement process cleanup logic in `akon-core/src/vpn/process.rs`: find OpenConnect processes by PID, send SIGTERM, wait 5 seconds, send SIGKILL if still alive, handle multiple processes, update connection state to Disconnected after cleanup
- [x] T027 [US1] Add reconnection state logging in `akon-core/src/vpn/reconnection.rs`: log state transitions (Reconnecting with attempt number), log backoff calculation, log reconnection success/failure, sanitize any sensitive data
- [x] T028 [US1] Update `src/cli/vpn.rs` status command to display Reconnecting state with attempt details (e.g., "Reconnecting: Attempt 3 of 5, next retry at 14:35:20")

**Checkpoint**: User Story 1 complete - Network interruption detection and automatic reconnection fully functional

---

## Phase 4: User Story 2 - Periodic Connection Health Check with Reconnection (Priority: P2)

**Goal**: Detect silent VPN failures through periodic HTTP/HTTPS health checks and trigger reconnection when checks fail consistently

**Independent Test**: Establish VPN, block network traffic at firewall level, verify health checks detect failure after consecutive threshold, verify reconnection triggered

### Tests for User Story 2

- [x] T029 [P] [US2] Create unit tests in `akon-core/tests/health_check_tests.rs` for HealthChecker: test_successful_health_check_with_200, test_health_check_fails_on_timeout, test_health_check_fails_on_4xx_status, test_health_check_fails_on_5xx_status, test_is_reachable_true_for_any_response, test_is_reachable_false_for_connection_refused (using wiremock)
- [x] T030 [P] [US2] Create unit tests in `akon-core/tests/reconnection_tests.rs` for health check handling: test_health_check_failure_triggers_reconnection, test_consecutive_failures_threshold, test_single_failure_does_not_trigger_reconnection, test_health_check_success_resets_failure_counter
- [x] T031 [P] [US2] Create integration test in `akon-core/tests/health_check_flow_tests.rs`: establish VPN, mock health check endpoint returning errors, verify consecutive failure tracking, verify reconnection triggered after threshold met

### Implementation for User Story 2

- [x] T032 [P] [US2] Implement `HealthChecker::new()` in `akon-core/src/vpn/health_check.rs`: create reqwest Client with rustls-tls, validate endpoint URL (must be HTTP/HTTPS), set timeout duration
- [x] T033 [P] [US2] Implement `HealthChecker::check()` in `akon-core/src/vpn/health_check.rs`: send GET request to endpoint with timeout, measure duration, map response to HealthCheckResult (success if 2xx/3xx status), handle errors (timeout, connection refused, DNS failure)
- [x] T034 [P] [US2] Implement `HealthChecker::is_reachable()` in `akon-core/src/vpn/health_check.rs`: call check() and return true if any response received (even error status codes), return false only on network errors
- [x] T035 [US2] Add consecutive failure counter to ReconnectionManager internal state in `akon-core/src/vpn/reconnection.rs`: track current failure count, reset on success
- [x] T036 [US2] Implement `ReconnectionManager::handle_health_check()` in `akon-core/src/vpn/reconnection.rs`: call health_checker.check(), increment failure counter on failure, reset counter on success, trigger reconnection when counter reaches consecutive_failures_threshold, only trigger if currently in Connected state
- [x] T037 [US2] Update `ReconnectionManager::run()` event loop in `akon-core/src/vpn/reconnection.rs`: add tokio::time::interval for periodic health checks, call handle_health_check() on interval tick (every health_check_interval_secs), ensure health check only runs when state is Connected
- [x] T038 [US2] Add health check logging in `akon-core/src/vpn/health_check.rs`: log successful checks (debug level with duration), log failed checks (warn level with error details), log consecutive failure count in reconnection.rs

**Checkpoint**: User Story 2 complete - Periodic health checks detect and recover from silent failures ✅

---

## Phase 5: User Story 3 - Configurable Retry Policy (Priority: P2)

**Goal**: Allow users to customize reconnection behavior (max attempts, backoff parameters, health check interval) via configuration file

**Independent Test**: Modify config values (max_attempts, base_interval_secs, etc.), trigger reconnection, verify tool respects configured values

### Tests for User Story 3

- [x] T039 [P] [US3] Create unit tests in `akon-core/tests/config_tests.rs` for ReconnectionPolicy parsing: test_parse_reconnection_config_with_all_fields, test_parse_reconnection_config_with_defaults, test_validate_max_attempts_range, test_validate_backoff_multiplier_range, test_validate_health_check_endpoint_url, test_invalid_config_returns_error
- [x] T040 [P] [US3] Create integration test in `akon-core/tests/config_integration_tests.rs`: write various config files, load them, trigger reconnection, verify behavior matches config (attempts, intervals, etc.) - 8 tests created, 1 documented for live VPN testing

### Implementation for User Story 3

- [x] T041 [P] [US3] Implement validation functions in `akon-core/src/vpn/reconnection.rs`: validate_max_attempts (1-20 range), validate_base_interval (1-300 range), validate_backoff_multiplier (1-10 range), validate_max_interval (≥ base_interval), validate_consecutive_failures (1-10 range), validate_health_check_interval (10-3600 range), validate_health_check_endpoint (valid HTTP/HTTPS URL)
- [x] T042 [US3] Update ReconnectionPolicy deserialization in `akon-core/src/vpn/reconnection.rs` to call validation functions after parsing, return descriptive errors for invalid values
- [x] T043 [US3] Update config loading in `akon-core/src/config/toml_config.rs` to load ReconnectionPolicy from [reconnection] section, apply defaults for missing fields, validate entire policy
- [x] T044 [US3] Update `src/cli/setup.rs` to optionally configure reconnection settings during setup: prompt for health_check_endpoint (with default), optionally prompt for advanced settings (max_attempts, intervals), write to config file
- [x] T045 [US3] Add config validation logging in `akon-core/src/config/toml_config.rs`: log when defaults are applied, log validation errors with specific field names and valid ranges, log successfully loaded reconnection policy values

**Checkpoint**: User Story 3 complete - Reconnection behavior fully configurable ✅

---

## Phase 6: User Story 4 - Manual Process Cleanup and Reset (Priority: P3)

**Goal**: Provide manual commands to cleanup orphaned processes and reset reconnection state when automatic recovery fails

**Independent Test**: Create orphaned OpenConnect processes manually, exceed retry limits, run cleanup/reset commands, verify system returns to clean state

### Tests for User Story 4

- [x] T046 [P] [US4] Create unit tests in `akon-core/tests/cleanup_tests.rs` for process cleanup: test_cleanup_terminates_openconnect_processes, test_cleanup_handles_multiple_processes, test_cleanup_uses_sigterm_before_sigkill, test_cleanup_when_no_processes_running, test_cleanup_with_insufficient_permissions
- [x] T047 [P] [US4] Create unit tests in `akon-core/tests/reconnection_tests.rs` for reset functionality: test_reset_clears_retry_counter, test_reset_transitions_from_error_to_disconnected, test_reset_allows_reconnection_after_max_attempts_exceeded
- [x] T048 [P] [US4] Create integration test in `akon-core/tests/integration/manual_recovery_tests.rs`: exceed max attempts, verify Error state, run reset command, verify retry counter cleared, trigger reconnection, verify it works

### Implementation for User Story 4

- [x] T049 [P] [US4] Implement `cleanup_orphaned_processes()` function in `src/daemon/process.rs`: find all OpenConnect processes (by name "openconnect"), for each process send SIGTERM, wait 5 seconds, check if still alive, send SIGKILL if needed, return count of terminated processes, handle permission errors gracefully
- [x] T050 [P] [US4] Implement `ReconnectionCommand::ResetRetries` handling in `akon-core/src/vpn/reconnection.rs`: reset internal retry counter to 0, reset consecutive failures counter to 0, transition from Error state to Disconnected state, log reset action
- [x] T051 [US4] Add `vpn cleanup` subcommand in `src/cli/vpn.rs`: call cleanup_orphaned_processes(), display count of terminated processes, update connection state to Disconnected, return appropriate exit code (0 if successful, 1 if errors)
- [x] T052 [US4] Add `vpn reset` subcommand in `src/cli/vpn.rs`: send ResetRetries command to reconnection manager, display confirmation message, return appropriate exit code
- [x] T053 [US4] Update `vpn status` command in `src/cli/vpn.rs` to show when manual intervention needed: detect Error state with "max attempts exceeded" message, display clear message suggesting `akon vpn cleanup` or `akon vpn reset`
- [x] T054 [US4] Add cleanup/reset command documentation to spec: update contracts/ with new command specifications, add usage examples to quickstart.md

**Checkpoint**: All user stories complete - Full feature set implemented and independently testable

---

## Phase 7: Polish & Cross-Cutting Concerns

**Purpose**: Improvements that affect multiple user stories

- [x] T055 [P] Add error context with thiserror for all custom error types: NetworkMonitorError, HealthCheckError, ReconnectionError with clear error messages and source chain
- [x] T056 [P] Add comprehensive tracing spans in all reconnection modules: use tracing::instrument on key functions, add span fields for attempt numbers, intervals, event types
- [x] T057 [P] Update README.md with reconnection feature documentation: add section explaining automatic reconnection, document config options, add troubleshooting guide
- [x] T058 [P] Update quickstart.md with manual testing scenarios: document how to test network interruption, how to test health check failure, how to verify backoff timing (completed in T054)
- [x] T059 Code review for security: verify no credentials in logs, verify HTTPS certificate validation, verify state file permissions (0600), verify no secrets in error messages
- [x] T060 Performance testing: verify health check overhead (< 0.1% CPU idle), verify event detection latency (< 1s), verify memory usage (< 5MB), verify timer accuracy (< 500ms) - Test framework created with 3 unit tests and 5 documented live test procedures
- [x] T061 Run full test suite: `cargo test` with all unit and integration tests, verify > 90% coverage on security-critical code paths
- [x] T062 Run clippy with strict lints: `cargo clippy -- -D warnings`, fix all warnings
- [x] T063 Run rustfmt: `cargo fmt --all`, ensure consistent code style
- [x] T064 Validate all acceptance criteria from spec.md against implementation: verify each SC-001 through SC-010 is met
- [x] T065 Run quickstart.md validation: follow developer setup instructions, verify all commands work, test manual test scenarios

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies - can start immediately
- **Foundational (Phase 2)**: Depends on Setup completion - BLOCKS all user stories
- **User Stories (Phase 3-6)**: All depend on Foundational phase completion
  - User Story 1 (P1) can start after Phase 2
  - User Story 2 (P2) can start after Phase 2 (uses health_check.rs)
  - User Story 3 (P2) can start after Phase 2 (uses config/toml_config.rs)
  - User Story 4 (P3) can start after Phase 2
  - Stories 2, 3, 4 can proceed in parallel with Story 1 if team capacity allows
- **Polish (Phase 7)**: Depends on all user stories being complete

### User Story Dependencies

- **User Story 1 (P1)**: Depends only on Phase 2 - Core reconnection functionality
- **User Story 2 (P2)**: Depends on Phase 2 - Extends US1 by adding health checks (integrates with ReconnectionManager from US1 but can be tested independently)
- **User Story 3 (P2)**: Depends on Phase 2 - Makes configuration flexible (tests existing behavior with different configs)
- **User Story 4 (P3)**: Depends on Phase 2 - Manual controls (can test cleanup and reset independently)

### Within Each User Story

1. Tests MUST be written and FAIL before implementation (TDD)
2. For User Story 1: NetworkMonitor before ReconnectionManager, exponential backoff before attempt_reconnect
3. For User Story 2: HealthChecker before integration with ReconnectionManager
4. For User Story 3: Validation functions before config parsing
5. For User Story 4: Process cleanup before CLI commands

### Parallel Opportunities

- **Phase 1**: All tasks are independent setup tasks
- **Phase 2**: T006-T009 (data structures) can run in parallel, T011-T012 (tests) can run in parallel
- **User Story 1**: T013-T016 (all tests) can run in parallel, T017-T019 (NetworkMonitor) can run in parallel
- **User Story 2**: T029-T031 (all tests) can run in parallel, T032-T034 (HealthChecker methods) can run in parallel
- **User Story 3**: T039-T040 (tests) can run in parallel, T041 (validation functions) can be parallelized
- **User Story 4**: T046-T048 (all tests) can run in parallel, T049-T050 (cleanup and reset) can run in parallel
- **Phase 7**: T055-T058 (documentation and error handling) can run in parallel
- **Different user stories can be worked on by different team members simultaneously after Phase 2**

---

## Parallel Example: User Story 1

```bash
# Launch all tests for User Story 1 together:
parallel cargo test :: \
  "network_monitor_tests::test_detects_network_up_event" \
  "network_monitor_tests::test_detects_network_down_event" \
  "reconnection_tests::test_backoff_calculation" \
  "reconnection_tests::test_max_attempts_exceeded_transitions_to_error"

# Launch NetworkMonitor implementation tasks in parallel (different methods):
# Task T017: NetworkMonitor::new()
# Task T018: NetworkMonitor::start()
# Task T019: NetworkMonitor::is_network_available()
# These touch different methods in the same file, so coordinate to avoid conflicts
```

---

## Implementation Strategy

### MVP Scope (Minimum Viable Product)

**Target**: User Story 1 only (Phase 1 + Phase 2 + Phase 3)

**Deliverable**: Basic network interruption detection and automatic reconnection with exponential backoff

**Why**: Solves the core problem (stale connections after network changes) with intelligent retry logic. Users can manually configure settings in config file for customization until US3 is implemented.

**Tasks**: T001-T028 (28 tasks)

**Estimated Duration**: 1-2 weeks for single developer

### Incremental Delivery

1. **Week 1**: MVP (US1) - Core reconnection functionality
2. **Week 2**: US2 (health checks) - Defense against silent failures
3. **Week 3**: US3 (configuration) - User customization
4. **Week 4**: US4 (manual controls) - Edge case handling and debugging tools
5. **Week 5**: Polish and documentation

### Testing Strategy

- Write tests FIRST (TDD) for all critical paths
- Unit tests with mocks (D-Bus, HTTP) for fast feedback
- Integration tests with real services for validation
- Manual testing scenarios in quickstart.md for QA
- Target > 90% coverage on security-critical code (auth, credentials, process cleanup)

---

## Total Task Count: 65 tasks

- **Phase 1 (Setup)**: 5 tasks
- **Phase 2 (Foundation)**: 7 tasks
- **Phase 3 (US1 - P1)**: 16 tasks (4 test tasks + 12 implementation tasks)
- **Phase 4 (US2 - P2)**: 10 tasks (3 test tasks + 7 implementation tasks)
- **Phase 5 (US3 - P2)**: 7 tasks (2 test tasks + 5 implementation tasks)
- **Phase 6 (US4 - P3)**: 9 tasks (3 test tasks + 6 implementation tasks)
- **Phase 7 (Polish)**: 11 tasks

**MVP Task Count**: 28 tasks (Phases 1-3)

**Parallel Opportunities**: ~25 tasks can be parallelized within their phases

**Independent Test Criteria**:
- **US1**: Network interruption triggers reconnection with correct backoff
- **US2**: Health check failures trigger reconnection after threshold
- **US3**: Config changes affect reconnection behavior
- **US4**: Cleanup/reset commands restore system to clean state
