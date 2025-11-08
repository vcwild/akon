# Implementation Plan: CI Pipeline Implementation

**Branch**: `002-ci-pipeline-with` | **Date**: 2025-11-06 | **Spec**: [spec.md](./spec.md)
**Input**: Feature specification from `/specs/002-ci-pipeline-with/spec.md`

**Note**: This template is filled in by the `/speckit.plan` command. See `.specify/templates/commands/plan.md` for the execution workflow.

## Summary

Implement a GitHub Actions CI pipeline that automatically validates code quality, runs tests, and verifies builds on every push and pull request. The pipeline will execute three primary jobs: (1) linting with `cargo fmt` and `cargo clippy`, (2) testing with `cargo test --workspace`, and (3) building with `cargo build --release --workspace`. This ensures code quality standards are enforced before code review and prevents regressions from being merged.

## Technical Context

**Language/Version**: Rust 1.70+ (stable channel, MSRV 1.70, edition 2021)
**Primary Dependencies**:
  - GitHub Actions for CI/CD platform
  - actions/checkout@v4 for repository access
  - actions-rust-lang/setup-rust-toolchain@v1 for Rust environment
  - Cargo for build, test, and lint orchestration
**Storage**: N/A (CI/CD configuration only)
**Testing**: cargo test (with workspace support), existing test suites in tests/ directory
**Target Platform**: GitHub Actions runners (Ubuntu latest)
**Project Type**: Rust workspace with binary crate (akon) and library crate (akon-core)
**Performance Goals**:
  - CI pipeline completes in <5 minutes for typical commits
  - Dependency caching reduces subsequent builds to <2 minutes
**Constraints**:
  - Must support existing project structure (workspace with members)
  - Must install system dependencies: openconnect-devel, dbus-devel, pkgconf-pkg-config
  - Must respect existing configuration: clippy.toml (MSRV 1.70), rustfmt.toml
  - Must work with fork-based pull requests (limited permissions)
**Scale/Scope**:
  - Single GitHub Actions workflow file
  - Three parallel jobs (lint, test, build)
  - Approximately 50-100 lines of YAML configuration

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

Verify compliance with Auto-OpenConnect Constitution v1.0.0:

- [x] **Security-First**: N/A - This feature adds CI/CD infrastructure and does not handle credentials or secrets. CI workflow will NOT log or expose any secrets.
- [x] **Modular Architecture**: N/A - This feature adds CI infrastructure, not application modules. However, CI will validate existing modular architecture through testing.
- [x] **Test-Driven Development**: CI will enforce TDD by running all tests automatically. The CI configuration itself is declarative YAML, not test-driven code.
- [x] **Observability**: CI will provide observability into build health through GitHub Actions logs and status checks. This complements, but does not replace, application-level systemd logging.
- [x] **CLI-First Interface**: N/A - This feature does not add CLI commands. However, CI will validate existing CLI functionality through automated testing.

**Security-Critical Changes** (require extra scrutiny):

This feature does NOT involve security-critical changes. CI pipeline validation criteria:

- [x] No secrets in workflow YAML files
- [x] No credentials passed to build/test commands
- [x] Workflow permissions follow least-privilege principle
- [x] Fork PRs run with restricted permissions (read-only by default)

## Project Structure

### Documentation (this feature)

```
specs/[###-feature]/
├── plan.md              # This file (/speckit.plan command output)
├── research.md          # Phase 0 output (/speckit.plan command)
├── data-model.md        # Phase 1 output (/speckit.plan command)
├── quickstart.md        # Phase 1 output (/speckit.plan command)
├── contracts/           # Phase 1 output (/speckit.plan command)
└── tasks.md             # Phase 2 output (/speckit.tasks command - NOT created by /speckit.plan)
```

### Source Code (repository root)

```plaintext
.github/
└── workflows/
    └── ci.yml           # NEW: GitHub Actions CI workflow

# Existing project structure (unchanged)
src/
├── main.rs
├── cli/
│   ├── mod.rs
│   ├── setup.rs
│   ├── vpn.rs
│   └── get_password.rs
└── daemon/
    ├── mod.rs
    ├── process.rs
    └── ipc.rs

akon-core/
├── Cargo.toml
├── src/
│   ├── lib.rs
│   ├── error.rs
│   ├── types.rs
│   ├── auth/
│   ├── config/
│   └── vpn/
└── tests/
    ├── auth_tests.rs
    ├── config_tests.rs
    ├── types_tests.rs
    └── error_tests.rs

tests/
├── integration/
│   ├── get_password_tests.rs
│   ├── keyring_tests.rs
│   ├── setup_tests.rs
│   └── vpn_status_tests.rs
└── unit/

# Configuration files used by CI
clippy.toml              # Existing: Clippy linting rules (MSRV 1.70)
rustfmt.toml             # Existing: Code formatting rules
Cargo.toml               # Existing: Workspace configuration
```

**Structure Decision**: This is a Rust workspace project with a binary crate (akon) and library crate (akon-core). The CI workflow will be added as a single file `.github/workflows/ci.yml` which will orchestrate linting, testing, and building across the entire workspace. All existing source and configuration files remain unchanged.

## Complexity Tracking

No constitution violations. This feature adds declarative CI configuration only.

## Phase Completion Status

### Phase 0: Research ✅ COMPLETE

**Output**: `research.md`

**Key Decisions**:
- Use `actions-rust-lang/setup-rust-toolchain@v1` for Rust environment
- Install Ubuntu packages: `libopenconnect-dev`, `libdbus-1-dev`, `pkg-config`
- Run `cargo test --workspace` for comprehensive testing
- Use `cargo fmt --check` and `cargo clippy -- -D warnings` for linting
- Start with stable Rust only (MSRV testing deferred to P2)
- Run on all pushes and PRs with no branch restrictions
- Parallel job execution (lint, test, build independent)

### Phase 1: Design ✅ COMPLETE

**Outputs**:
- `data-model.md` - Workflow/job/step entity definitions
- `contracts/ci-workflow.md` - CI workflow contract specification
- `quickstart.md` - Developer guide for working with CI
- `.github/copilot-instructions.md` - Updated agent context

**Design Summary**:
- Single workflow file: `.github/workflows/ci.yml`
- Three parallel jobs: lint, test, build
- Each job runs on ubuntu-latest
- System dependencies installed before Rust setup
- Automatic caching via setup-rust-toolchain action
- No secrets required, read-only permissions

**Constitution Re-Check**: ✅ PASSED
- No security-critical changes
- Enforces TDD through automated test execution
- Provides observability via GitHub Actions logs
- No credential handling in CI

### Phase 2: Tasks (NOT STARTED)

**Note**: Per instructions, `/speckit.plan` command stops after Phase 1. Phase 2 (tasks breakdown) is handled by `/speckit.tasks` command, which is not part of this execution.

## Next Steps

To continue implementation:

1. Run `/speckit.tasks` command to generate `tasks.md` with implementation breakdown
2. Create `.github/workflows/ci.yml` based on contracts/ci-workflow.md
3. Test workflow on feature branch
4. Open PR to merge CI pipeline

