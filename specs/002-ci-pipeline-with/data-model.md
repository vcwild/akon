# Data Model: CI Pipeline Implementation

**Feature**: 002-ci-pipeline-with
**Phase**: 1 (Design)
**Date**: 2025-11-06

## Overview

This document defines the entities and their relationships for the CI pipeline implementation. Since this is a CI/CD configuration feature, the "data model" consists of the GitHub Actions workflow structure and its components rather than traditional application data entities.

## Entities

### 1. Workflow

**Description**: Top-level GitHub Actions workflow configuration that orchestrates all CI jobs.

**Attributes**:
- `name`: string - Display name shown in GitHub Actions UI ("CI")
- `on`: trigger configuration - Events that trigger the workflow
  - `push`: trigger on any branch push
  - `pull_request`: trigger on any PR
- `jobs`: map of job definitions - Collection of parallel jobs to execute

**Validation Rules**:
- Must have at least one job defined
- Must have at least one trigger event
- Trigger events must be valid GitHub Actions events

**State Transitions**:
- Queued → Running → Success/Failure/Cancelled

**Relationships**:
- Contains multiple Jobs (1:N)

---

### 2. Job

**Description**: Independent unit of work that runs on a GitHub Actions runner. Jobs run in parallel unless explicitly sequenced with dependencies.

**Attributes**:
- `name`: string - Human-readable job identifier
- `runs-on`: string - Runner specification (e.g., "ubuntu-latest")
- `steps`: array of Step definitions - Sequential actions to execute
- `strategy`: optional matrix configuration - For parameterized job execution

**Validation Rules**:
- Must specify valid runner (ubuntu-latest, ubuntu-22.04, etc.)
- Must have at least one step
- Step order matters (sequential execution within job)

**State Transitions**:
- Pending → In Progress → Success/Failure/Cancelled

**Relationships**:
- Belongs to Workflow (N:1)
- Contains multiple Steps (1:N)
- May depend on other Jobs (N:N via `needs` keyword)

**Instances in This Feature**:

1. **Lint Job**
   - Purpose: Validate code formatting and linting rules
   - Runner: ubuntu-latest
   - Key Steps: checkout, setup-rust, fmt check, clippy check

2. **Test Job**
   - Purpose: Execute all unit and integration tests
   - Runner: ubuntu-latest
   - Key Steps: checkout, install system deps, setup-rust, run tests

3. **Build Job**
   - Purpose: Verify successful compilation in release mode
   - Runner: ubuntu-latest
   - Key Steps: checkout, install system deps, setup-rust, build release

---

### 3. Step

**Description**: Individual action or command executed within a job. Steps run sequentially.

**Attributes**:
- `name`: string - Descriptive step name for logs
- `uses`: optional string - Reference to reusable action (e.g., "actions/checkout@v4")
- `run`: optional string - Shell command to execute
- `with`: optional map - Parameters passed to action
- `env`: optional map - Environment variables for this step

**Validation Rules**:
- Must have either `uses` OR `run`, not both
- If `uses`, must be valid action reference (org/repo@version)
- If `run`, must be valid shell command
- Name should clearly describe step purpose

**State Transitions**:
- Queued → Running → Success/Failure/Skipped

**Relationships**:
- Belongs to Job (N:1)
- May use Action (N:1)

**Common Step Patterns**:

1. **Checkout Step**
   ```yaml
   - name: Checkout code
     uses: actions/checkout@v4
   ```

2. **System Dependency Step**
   ```yaml
   - name: Install system dependencies
     run: |
       sudo apt-get update
       sudo apt-get install -y libopenconnect-dev libdbus-1-dev pkg-config
   ```

3. **Rust Toolchain Step**
   ```yaml
   - name: Setup Rust toolchain
     uses: actions-rust-lang/setup-rust-toolchain@v1
     with:
       toolchain: stable
       components: rustfmt, clippy
   ```

4. **Command Execution Step**
   ```yaml
   - name: Run tests
     run: cargo test --workspace --verbose
   ```

---

### 4. Action (External Dependency)

**Description**: Reusable GitHub Actions maintained by third parties. Not defined by this feature but consumed by it.

**Attributes**:
- `reference`: string - Full action reference (e.g., "actions/checkout@v4")
- `inputs`: map - Configuration parameters
- `outputs`: map - Data produced by action

**Validation Rules**:
- Must use trusted sources (github.com official actions or rust-lang organization)
- Should pin to major version (@v1, @v4) for auto-updates with stability
- Must be publicly accessible

**Relationships**:
- Used by Steps (N:N)

**Actions Used in This Feature**:

1. **actions/checkout@v4**
   - Purpose: Clone repository code
   - Inputs: None (defaults sufficient)
   - Outputs: Repository files in workspace

2. **actions-rust-lang/setup-rust-toolchain@v1**
   - Purpose: Install Rust toolchain with automatic caching
   - Inputs:
     - `toolchain`: "stable" (default) or "1.70" (MSRV)
     - `components`: "rustfmt, clippy"
   - Outputs: Configured Rust environment with cargo, rustc, rustfmt, clippy

---

### 5. Cache (Implicit Entity)

**Description**: GitHub Actions cache managed automatically by setup-rust-toolchain action. Not explicitly configured but important for performance.

**Attributes**:
- `key`: auto-generated hash based on Cargo.lock and rust-toolchain
- `paths`: automatically includes cargo registry, cargo index, target directory
- `size`: approximately 500MB for typical Rust project

**Validation Rules**:
- Cache automatically invalidated when Cargo.lock changes
- Cache shared across jobs on same runner
- Cache persists across workflow runs

**State Transitions**:
- Miss → Populated → Hit → Stale → Evicted

**Relationships**:
- Associated with Workflow (via cache action)
- Shared by all Jobs using Rust toolchain

---

## Entity Relationship Diagram

```plaintext
Workflow (1)
  │
  ├─ on: Triggers (push, pull_request)
  │
  └─ jobs: (3)
       │
       ├─ Lint Job (1)
       │    └─ steps: (4)
       │         ├─ Checkout (uses actions/checkout@v4)
       │         ├─ Setup Rust (uses setup-rust-toolchain@v1)
       │         ├─ Format Check (run: cargo fmt)
       │         └─ Clippy (run: cargo clippy)
       │
       ├─ Test Job (1)
       │    └─ steps: (4)
       │         ├─ Checkout (uses actions/checkout@v4)
       │         ├─ Install System Deps (run: apt-get)
       │         ├─ Setup Rust (uses setup-rust-toolchain@v1)
       │         └─ Run Tests (run: cargo test)
       │
       └─ Build Job (1)
            └─ steps: (4)
                 ├─ Checkout (uses actions/checkout@v4)
                 ├─ Install System Deps (run: apt-get)
                 ├─ Setup Rust (uses setup-rust-toolchain@v1)
                 └─ Build Release (run: cargo build)
```

## Configuration Schema

The workflow configuration maps to GitHub Actions YAML schema:

```yaml
# Workflow Entity
name: string
on:
  push:
    branches: [string]
  pull_request:
    branches: [string]

jobs:
  # Job Entity (repeated)
  <job_id>:
    runs-on: string
    steps:
      # Step Entity (repeated)
      - name: string
        uses: string?
        run: string?
        with:
          <key>: <value>
```

## Validation Strategy

Since this is configuration-as-code, validation occurs at multiple levels:

1. **Syntax Validation**: GitHub validates YAML syntax on push
2. **Schema Validation**: GitHub validates against workflow schema
3. **Runtime Validation**: Failures in steps indicate configuration errors
4. **Manual Validation**: Review workflow logs for expected behavior

## Non-Functional Considerations

### Performance
- Jobs run in parallel by default (no `needs` dependencies)
- First run: ~5 minutes (cold cache)
- Subsequent runs: ~2 minutes (warm cache)
- Cache hit rate: ~90% for typical development workflow

### Reliability
- Each job is idempotent (can be safely retried)
- System dependency installation uses stable package repositories
- Action versions pinned to major versions for stability with security updates

### Security
- No secrets required in workflow
- Fork PRs run with read-only permissions by default
- No credential storage or handling in CI configuration

### Maintainability
- Clear step names for debugging
- Verbose output flags for troubleshooting
- Reusable patterns across jobs (DRY principle applied)
