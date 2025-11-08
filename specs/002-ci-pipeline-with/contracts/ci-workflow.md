# CI Workflow Contract

**Feature**: 002-ci-pipeline-with
**Type**: GitHub Actions Workflow Configuration
**Version**: 1.0

## Overview

This document defines the contract for the CI workflow that validates code quality, runs tests, and verifies builds for the akon project.

## Workflow Specification

### Triggers

**Events**:
- `push` to any branch
- `pull_request` to any branch

**Branch Filter**: All branches (`**`)

**Rationale**: Validate all code changes regardless of branch to catch issues early.

---

## Job Contracts

### Job 1: Lint

**Purpose**: Validate code formatting and linting rules.

**Runner**: `ubuntu-latest`

**Prerequisites**: None (can run in parallel with other jobs)

**Steps**:

1. **Checkout Repository**
   - Action: `actions/checkout@v4`
   - Purpose: Clone repository code
   - Inputs: Default (full checkout)
   - Outputs: Repository files available in workspace
   - Expected Duration: 5-10 seconds

2. **Setup Rust Toolchain**
   - Action: `actions-rust-lang/setup-rust-toolchain@v1`
   - Purpose: Install Rust with formatting and linting tools
   - Inputs:
     - `toolchain`: `stable`
     - `components`: `rustfmt, clippy`
   - Outputs: Rust toolchain with cargo, rustfmt, clippy available
   - Expected Duration: 30-60 seconds (cold), 5-10 seconds (cached)

3. **Check Code Formatting**
   - Command: `cargo fmt --all --check`
   - Purpose: Verify code follows formatting rules from rustfmt.toml
   - Exit Code: 0 if formatted correctly, non-zero if formatting issues exist
   - Outputs: List of files needing formatting (if any)
   - Expected Duration: 2-5 seconds

4. **Run Clippy Linter**
   - Command: `cargo clippy --workspace --all-targets -- -D warnings`
   - Purpose: Catch code quality issues and treat warnings as errors
   - Exit Code: 0 if no issues, non-zero if warnings/errors exist
   - Outputs: List of clippy warnings/errors with file locations
   - Expected Duration: 30-60 seconds

**Success Criteria**:
- All steps complete with exit code 0
- No formatting violations
- No clippy warnings or errors

**Failure Scenarios**:
- Formatting check fails → Clear message showing which files need formatting
- Clippy fails → Detailed warnings/errors with file:line locations
- Compilation fails → Rust compiler errors

**Total Expected Duration**: 1-2 minutes

---

### Job 2: Test

**Purpose**: Execute all unit and integration tests across workspace.

**Runner**: `ubuntu-latest`

**Prerequisites**: None (can run in parallel with other jobs)

**Steps**:

1. **Checkout Repository**
   - Action: `actions/checkout@v4`
   - Purpose: Clone repository code
   - Expected Duration: 5-10 seconds

2. **Install System Dependencies**
   - Command:
     ```bash
     sudo apt-get update
     sudo apt-get install -y libopenconnect-dev libdbus-1-dev pkg-config
     ```
   - Purpose: Install native libraries required for linking
   - Exit Code: 0 if successful
   - Expected Duration: 30-60 seconds

3. **Setup Rust Toolchain**
   - Action: `actions-rust-lang/setup-rust-toolchain@v1`
   - Purpose: Install Rust toolchain
   - Inputs:
     - `toolchain`: `stable`
   - Expected Duration: 30-60 seconds (cold), 5-10 seconds (cached)

4. **Run Tests**
   - Command: `cargo test --workspace --verbose`
   - Purpose: Execute all tests in workspace members (akon, akon-core)
   - Exit Code: 0 if all tests pass, non-zero if any test fails
   - Outputs: Test results with pass/fail for each test, detailed failure messages
   - Expected Duration: 1-3 minutes

**Success Criteria**:
- All system dependencies install successfully
- All tests pass across workspace
- No test panics or assertion failures

**Failure Scenarios**:
- System dependency installation fails → apt error messages
- Test fails → Test name, expected vs actual values, stack trace
- Compilation fails before tests run → Rust compiler errors

**Total Expected Duration**: 2-4 minutes

---

### Job 3: Build

**Purpose**: Verify successful compilation in release mode.

**Runner**: `ubuntu-latest`

**Prerequisites**: None (can run in parallel with other jobs)

**Steps**:

1. **Checkout Repository**
   - Action: `actions/checkout@v4`
   - Purpose: Clone repository code
   - Expected Duration: 5-10 seconds

2. **Install System Dependencies**
   - Command:
     ```bash
     sudo apt-get update
     sudo apt-get install -y libopenconnect-dev libdbus-1-dev pkg-config
     ```
   - Purpose: Install native libraries required for linking
   - Expected Duration: 30-60 seconds

3. **Setup Rust Toolchain**
   - Action: `actions-rust-lang/setup-rust-toolchain@v1`
   - Purpose: Install Rust toolchain
   - Inputs:
     - `toolchain`: `stable`
   - Expected Duration: 30-60 seconds (cold), 5-10 seconds (cached)

4. **Build Release Binary**
   - Command: `cargo build --release --workspace --verbose`
   - Purpose: Compile all workspace members in optimized release mode
   - Exit Code: 0 if build succeeds, non-zero if compilation fails
   - Outputs: Compiled binary at `target/release/akon`, detailed build logs
   - Expected Duration: 2-4 minutes

**Success Criteria**:
- All system dependencies install successfully
- All workspace members compile without errors
- Release binary produced at expected location

**Failure Scenarios**:
- System dependency installation fails → apt error messages
- Compilation fails → Rust compiler errors with file:line locations
- Linker fails → Missing library errors

**Total Expected Duration**: 3-5 minutes

---

## Overall Workflow Contract

### Parallelism

All three jobs (lint, test, build) run in parallel with no dependencies between them.

**Rationale**: Faster feedback - developers see all failure types simultaneously.

### Caching Strategy

**Cached Data**:
- Cargo registry index (~10MB)
- Cargo registry cache (~200MB, downloaded crates)
- Target directory (~300MB, compiled artifacts)

**Cache Management**: Automatic via `actions-rust-lang/setup-rust-toolchain@v1`

**Cache Invalidation**: Automatic when Cargo.lock changes

**Expected Cache Hit Rate**: ~90% for typical development workflow

### Success Criteria (Overall)

Workflow succeeds if and only if ALL three jobs succeed.

### Failure Handling

- Any job failure → Workflow marked as failed
- Failed status visible on commit/PR in GitHub UI
- Detailed logs available for each failed job
- No automatic retry (developer must fix and push)

### Performance Targets

**Cold Cache** (first run or after Cargo.lock change):
- Lint: 1-2 minutes
- Test: 2-4 minutes
- Build: 3-5 minutes
- **Total**: 3-5 minutes (parallel execution)

**Warm Cache** (typical subsequent runs):
- Lint: 30-60 seconds
- Test: 1-2 minutes
- Build: 2-3 minutes
- **Total**: 2-3 minutes (parallel execution)

---

## Configuration Contract

### External Dependencies

**GitHub Actions**:
- `actions/checkout@v4` - Official action, stable, well-maintained
- `actions-rust-lang/setup-rust-toolchain@v1` - Official rust-lang action, provides caching

**System Packages** (Ubuntu):
- `libopenconnect-dev` - OpenConnect development libraries
- `libdbus-1-dev` - D-Bus development libraries
- `pkg-config` - Package configuration tool

**Version Pinning Strategy**:
- Actions: Pin to major version (@v1, @v4) for stability with auto-updates
- System packages: Use latest from Ubuntu repositories (stable LTS packages)
- Rust toolchain: Use `stable` channel, MSRV validation deferred to P2

### Configuration Files

Workflow respects existing project configuration:

- `clippy.toml`: Linting rules (MSRV 1.70, test-specific allowances)
- `rustfmt.toml`: Formatting rules (edition 2021, max_width 100, etc.)
- `Cargo.toml`: Workspace configuration and dependencies

No workflow-specific Rust configuration required.

---

## Security Contract

### Permissions

**Default Permissions**: Read-only repository access

**No Elevated Permissions Required**: Workflow does not write to repository, deploy artifacts, or access secrets.

**Fork PR Handling**: External forks run with same read-only permissions (safe by default).

### Secrets

**No Secrets Required**: Workflow does not access:
- API tokens
- Deployment credentials
- Container registries
- External services

### Sensitive Data

**No Sensitive Data Logged**:
- No credentials in build output
- No user data in test fixtures
- No secrets in configuration files

**Public Logs**: All workflow logs are publicly visible (appropriate for open source project).

---

## Maintenance Contract

### Action Updates

**Automatic Updates**: GitHub automatically updates pinned major versions (@v1, @v4) with security patches.

**Breaking Changes**: Major version changes require manual workflow update.

**Monitoring**: GitHub sends notifications for deprecated actions.

### System Dependencies

**Stability**: Ubuntu LTS package repositories provide stable versions.

**Breaking Changes**: Rare; Ubuntu LTS guarantees package API stability.

### Rust Toolchain

**Stable Channel**: Automatically updates to latest stable Rust (backward compatible).

**MSRV Testing**: Deferred to Priority 2 (not in this contract).

---

## Testing the Contract

### Validation Scenarios

1. **Happy Path**: Clean commit on main branch
   - Expected: All three jobs pass in parallel
   - Duration: 2-3 minutes with warm cache

2. **Formatting Violation**: Commit with unformatted code
   - Expected: Lint job fails with specific file list
   - Other jobs may pass independently

3. **Clippy Warning**: Commit with code quality issue
   - Expected: Lint job fails with warning details
   - Other jobs may pass independently

4. **Failing Test**: Commit that breaks existing test
   - Expected: Test job fails with test name and assertion details
   - Other jobs may pass independently

5. **Build Failure**: Commit with compilation error
   - Expected: Build job (and likely test job) fail with compiler errors
   - Lint job may pass if syntax is valid

6. **Fork PR**: External contributor opens PR from fork
   - Expected: All jobs run with read-only permissions
   - Same validation as internal branches

### Contract Verification Checklist

- [ ] Workflow triggers on push to any branch
- [ ] Workflow triggers on pull requests
- [ ] All three jobs run in parallel
- [ ] Lint job validates formatting and clippy rules
- [ ] Test job installs system dependencies correctly
- [ ] Test job executes all workspace tests
- [ ] Build job produces release binary
- [ ] Cache improves performance on subsequent runs
- [ ] Fork PRs run successfully with limited permissions
- [ ] Failure messages are clear and actionable
- [ ] Logs are publicly visible (no secrets exposed)
