# Research: CI Pipeline Implementation

**Feature**: 002-ci-pipeline-with
**Phase**: 0 (Research & Investigation)
**Date**: 2025-11-06

## Overview

This document consolidates research findings for implementing a GitHub Actions CI pipeline for the akon Rust project. The pipeline must handle linting, testing, and building for a workspace project with system-level dependencies.

## Research Questions & Findings

### 1. GitHub Actions for Rust Projects

**Question**: What is the recommended GitHub Actions setup for Rust projects with workspace support?

**Decision**: Use `actions-rust-lang/setup-rust-toolchain@v1` action

**Rationale**:
- Official rust-lang GitHub organization action (trustworthy, well-maintained)
- Automatic Rust toolchain installation with caching
- Built-in support for `rust-toolchain.toml` files
- Handles cargo registry and target directory caching automatically
- Respects MSRV and custom toolchain specifications
- More efficient than manual cargo cache actions

**Alternatives Considered**:
- `actions-rs/toolchain@v1` - Deprecated, no longer maintained
- Manual cargo caching with `actions/cache@v3` - More complex, requires manual cache key management
- `Swatinem/rust-cache@v2` - Good alternative, but setup-rust-toolchain provides integrated solution

**Implementation Notes**:
```yaml
- uses: actions-rust-lang/setup-rust-toolchain@v1
  with:
    toolchain: stable
    components: rustfmt, clippy
```

### 2. System Dependencies in GitHub Actions

**Question**: How to install system dependencies (openconnect-devel, dbus-devel, pkgconf-pkg-config) on Ubuntu runners?

**Decision**: Use `apt-get` with Ubuntu package names, installed before Rust setup

**Rationale**:
- GitHub Actions uses Ubuntu runners by default
- Ubuntu package names differ from Fedora (dnf) names used in Makefile
- Must install before any cargo commands that link against these libraries
- apt-get is faster than dnf on Ubuntu

**Package Mapping** (Fedora → Ubuntu):
- `openconnect-devel` → `libopenconnect-dev`
- `dbus-devel` → `libdbus-1-dev`
- `pkgconf-pkg-config` → `pkg-config`

**Alternatives Considered**:
- Docker container with Fedora - Slower, unnecessary complexity
- Using GitHub's container images - Overkill for simple dependency installation
- Skip system deps and use mocks - Won't catch real linking issues

**Implementation Notes**:
```yaml
- name: Install system dependencies
  run: |
    sudo apt-get update
    sudo apt-get install -y libopenconnect-dev libdbus-1-dev pkg-config
```

### 3. Cargo Workspace Testing Strategy

**Question**: What is the best practice for running tests in a Cargo workspace with multiple crates?

**Decision**: Use `cargo test --workspace` as single command

**Rationale**:
- Runs tests for all workspace members in correct dependency order
- Shares build artifacts between crates efficiently
- Single command simplifies CI configuration
- Catches integration issues between workspace crates
- Respects workspace-level features and dependencies

**Alternatives Considered**:
- Run `cargo test` separately in each crate directory - Redundant builds, misses cross-crate issues
- Test only the binary crate - Misses library crate tests
- Parallel test jobs per crate - Unnecessary complexity, slower due to cold caches

**Implementation Notes**:
```yaml
- name: Run tests
  run: cargo test --workspace --verbose
```

### 4. Linting Configuration

**Question**: How to enforce both formatting and clippy checks in CI?

**Decision**: Run `cargo fmt --check` and `cargo clippy -- -D warnings` as separate steps in single job

**Rationale**:
- `--check` flag makes fmt non-destructive (fails without modifying files)
- `-D warnings` treats all clippy warnings as errors
- Separate steps provide clearer error reporting in CI logs
- Single job reduces overhead (shared dependency installation)
- Respects existing clippy.toml and rustfmt.toml configuration

**Alternatives Considered**:
- Separate jobs for fmt and clippy - More parallel, but slower overall due to setup duplication
- Allow clippy warnings - Defeats purpose of linting
- Use `cargo fix` automatically - Should be manual developer action, not CI automation

**Implementation Notes**:
```yaml
- name: Check formatting
  run: cargo fmt --all --check

- name: Run clippy
  run: cargo clippy --workspace --all-targets -- -D warnings
```

### 5. Build Matrix Strategy

**Question**: Should CI test multiple Rust versions or platforms?

**Decision**: Start with stable Rust on Ubuntu only, add MSRV (1.70) verification as P2

**Rationale**:
- Priority 1: Get basic CI working (stable Rust)
- Priority 2: Add MSRV verification to catch compatibility issues
- Project declares MSRV in clippy.toml (1.70), should verify it works
- Single platform sufficient for initial implementation (Linux-only project per Makefile)
- Beta/nightly testing not needed (project uses stable features)

**Alternatives Considered**:
- Full matrix (stable, beta, nightly, MSRV) immediately - Overkill, slower CI
- Multiple OS (Ubuntu, macOS, Windows) - Project is Linux-specific (NetworkManager, D-Bus dependencies)
- No MSRV testing - Risks breaking declared compatibility

**Implementation Notes** (P1 - stable only):
```yaml
jobs:
  lint:
    runs-on: ubuntu-latest
  test:
    runs-on: ubuntu-latest
  build:
    runs-on: ubuntu-latest
```

**Implementation Notes** (P2 - add MSRV):
```yaml
test:
  strategy:
    matrix:
      rust: [stable, "1.70"]
  runs-on: ubuntu-latest
  steps:
    - uses: actions-rust-lang/setup-rust-toolchain@v1
      with:
        toolchain: ${{ matrix.rust }}
```

### 6. CI Trigger Strategy

**Question**: When should CI run (branches, PRs, tags)?

**Decision**: Run on all pushes and pull requests, no branch restrictions

**Rationale**:
- Validates code quality on every branch (feature branches, main, etc.)
- Prevents broken code from reaching PR stage
- Pull request triggers catch fork-based contributions
- No tag triggers needed (no release automation yet)
- Simple, predictable behavior

**Alternatives Considered**:
- Only on PRs to main - Misses issues on feature branches before PR
- Only on main and release branches - Too late to catch issues
- Include tag triggers - Not needed without release automation

**Implementation Notes**:
```yaml
on:
  push:
    branches: ["**"]
  pull_request:
    branches: ["**"]
```

### 7. Caching Strategy

**Question**: What should be cached to optimize CI performance?

**Decision**: Use built-in caching from `actions-rust-lang/setup-rust-toolchain@v1`

**Rationale**:
- Action automatically caches:
  - Cargo registry index
  - Cargo registry cache (crates.io downloads)
  - Target directory (compiled artifacts)
- Zero configuration required
- Handles cache invalidation automatically based on Cargo.lock and rust-toolchain
- Proven strategy used by rust-lang organization

**Alternatives Considered**:
- Manual cache with `actions/cache@v3` - More control but requires manual key management
- No caching - Each build would take 5-10 minutes
- Sccache for shared compilation cache - Overkill for small project

**Performance Expectations**:
- First run (cold cache): ~5 minutes
- Subsequent runs (warm cache): ~2 minutes
- Cache size: ~500MB (typical Rust project)

### 8. Job Parallelization

**Question**: Should lint, test, and build run in parallel or sequentially?

**Decision**: Run as parallel jobs with no dependencies

**Rationale**:
- Faster feedback (all three complete in parallel)
- Independent failures (can see lint issues without waiting for tests)
- Each job installs dependencies independently (idempotent, cached)
- GitHub Actions free tier provides sufficient parallel runners

**Alternatives Considered**:
- Sequential with dependencies (lint → test → build) - Slower, 3x total time
- Single job with multiple steps - Less clear error reporting, no parallelism

**Implementation Notes**:
```yaml
jobs:
  lint:
    # ...
  test:
    # ...
  build:
    # ...
  # No "needs:" clauses = parallel execution
```

## Best Practices Applied

### From Rust Community
1. Use `--workspace` flag for workspace-aware operations
2. Use `--all-targets` with clippy to check tests, benches, examples
3. Use `--verbose` flag for better CI debugging
4. Pin action versions to major version with auto-updates (`@v1`, `@v4`)

### From GitHub Actions Community
1. Install system dependencies before language setup
2. Use official actions from trusted organizations
3. Name steps clearly for readable logs
4. Use `runs-on: ubuntu-latest` for automatic Ubuntu version updates

### From Constitution
1. No secrets in workflow files (verified: none needed)
2. Logs are public (verified: no sensitive data logged)
3. Test-driven development enforced (automatic test execution)

## Implementation Checklist

- [ ] Create `.github/workflows/ci.yml`
- [ ] Define workflow triggers (push, pull_request)
- [ ] Implement lint job (fmt + clippy)
- [ ] Implement test job (cargo test --workspace)
- [ ] Implement build job (cargo build --release --workspace)
- [ ] Add system dependency installation
- [ ] Configure Rust toolchain setup with caching
- [ ] Test workflow on feature branch
- [ ] Verify all jobs pass on clean main branch
- [ ] Verify failure scenarios (bad format, failing test, build error)

## References

- [actions-rust-lang/setup-rust-toolchain](https://github.com/actions-rust-lang/setup-rust-toolchain)
- [GitHub Actions workflow syntax](https://docs.github.com/en/actions/using-workflows/workflow-syntax-for-github-actions)
- [Cargo workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html)
- [Clippy documentation](https://github.com/rust-lang/rust-clippy)
- Project files: `clippy.toml`, `rustfmt.toml`, `Cargo.toml`, `Makefile`
