# Tasks: CI Pipeline Implementation

**Input**: Design documents from `/specs/002-ci-pipeline-with/`
**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/ci-workflow.md

**Feature**: Implement GitHub Actions CI pipeline with linting, testing, and building

**Organization**: Tasks are grouped by user story to enable independent implementation and testing of each story.

## Format: `[ID] [P?] [Story] Description`
- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (e.g., US1, US2, US3, US4)
- Include exact file paths in descriptions

## Implementation Strategy

This feature implements CI/CD infrastructure as a series of independent, incrementally deliverable user stories:

1. **MVP (US1 only)**: Lint job - immediate value, catches format/quality issues
2. **US1+US2**: Add test execution - prevents regressions
3. **US1+US2+US3**: Add build verification - complete CI validation
4. **All stories**: Add MSRV testing - full compatibility verification
5. **Branch Protection**: Enforce CI checks before merging - **CRITICAL for production use** (achieves SC-001: zero broken code merged to main)

Each story can be implemented, tested, and merged independently.

**Total**: 54 tasks across 8 phases

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Create workflow file structure and basic configuration

- [ ] T001 Create `.github/workflows/` directory in repository root
- [ ] T002 Create `.github/workflows/ci.yml` file with basic workflow skeleton (name, triggers)
- [ ] T003 Configure workflow triggers for push and pull_request events on all branches

**Expected Output**:
- Directory: `.github/workflows/`
- File: `.github/workflows/ci.yml` with workflow name and trigger configuration

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Shared job steps that ALL user stories depend on

**⚠️ CRITICAL**: These steps are reused across all jobs - must be defined before implementing user stories

- [ ] T004 [P] Define checkout step using `actions/checkout@v4` (reusable in all jobs)
- [ ] T005 [P] Define Rust toolchain setup step using `actions-rust-lang/setup-rust-toolchain@v1` with stable channel (reusable in all jobs)
- [ ] T006 [P] Define system dependencies installation step for Ubuntu (apt-get install libopenconnect-dev libdbus-1-dev pkg-config)

**Checkpoint**: Foundation steps defined - user story jobs can now reference these patterns

---

## Phase 3: User Story 1 - Code Quality Validation on Every Push (Priority: P1) 🎯 MVP

**Goal**: Automatically validate code formatting and linting rules on every push

**Independent Test**: Push code to any branch and verify lint job runs with `cargo fmt --check` and `cargo clippy -- -D warnings`

**Acceptance Criteria**:
- Properly formatted code → lint job passes with green checkmark
- Code with formatting issues → lint job fails with specific violations listed
- Code with clippy warnings → lint job fails with specific warnings listed

### Implementation for User Story 1

- [ ] T007 [US1] Define `lint` job in `.github/workflows/ci.yml` with `runs-on: ubuntu-latest`
- [ ] T008 [US1] Add checkout step to lint job using `actions/checkout@v4`
- [ ] T009 [US1] Add Rust toolchain setup to lint job using `actions-rust-lang/setup-rust-toolchain@v1` with components `rustfmt, clippy`
- [ ] T010 [US1] Add formatting check step running `cargo fmt --all --check` in lint job
- [ ] T011 [US1] Add clippy check step running `cargo clippy --workspace --all-targets -- -D warnings` in lint job
- [ ] T012 [US1] Test lint job by pushing formatted code (should pass)
- [ ] T013 [US1] Test lint job by pushing unformatted code (should fail with clear message)
- [ ] T014 [US1] Test lint job by pushing code with clippy warnings (should fail with clear warnings)

**Checkpoint**: Lint job is fully functional and catches formatting/linting issues independently

---

## Phase 4: User Story 2 - Automated Test Execution (Priority: P1)

**Goal**: Automatically run all unit and integration tests on every push

**Independent Test**: Push code with passing tests and verify test job executes successfully; push code with failing test and verify CI catches it

**Acceptance Criteria**:
- All tests passing locally → test job executes all tests successfully
- Failing test → test job fails with clear indication of which test failed and why
- Tests for both workspace crates → tests run for all workspace members

### Implementation for User Story 2

- [ ] T015 [US2] Define `test` job in `.github/workflows/ci.yml` with `runs-on: ubuntu-latest` (parallel to lint job)
- [ ] T016 [US2] Add checkout step to test job using `actions/checkout@v4`
- [ ] T017 [US2] Add system dependencies installation step to test job (apt-get install libopenconnect-dev libdbus-1-dev pkg-config)
- [ ] T018 [US2] Add Rust toolchain setup to test job using `actions-rust-lang/setup-rust-toolchain@v1`
- [ ] T019 [US2] Add test execution step running `cargo test --workspace --verbose` in test job
- [ ] T020 [US2] Test test job by pushing code with all tests passing (should pass)
- [ ] T021 [US2] Test test job by introducing a failing test (should fail with test name and reason)
- [ ] T022 [US2] Verify tests run for both akon and akon-core workspace members

**Checkpoint**: Test job is fully functional and catches test failures independently

---

## Phase 5: User Story 3 - Build Verification (Priority: P1)

**Goal**: Verify successful compilation in release mode on every push

**Independent Test**: Push code and verify `cargo build --release` completes successfully, producing the akon binary

**Acceptance Criteria**:
- Valid Rust code → build job compiles all workspace members successfully in release mode
- Code with compilation errors → build job fails with clear compiler error messages
- Successful build → build artifacts are available for inspection

### Implementation for User Story 3

- [ ] T023 [US3] Define `build` job in `.github/workflows/ci.yml` with `runs-on: ubuntu-latest` (parallel to lint and test jobs)
- [ ] T024 [US3] Add checkout step to build job using `actions/checkout@v4`
- [ ] T025 [US3] Add system dependencies installation step to build job (apt-get install libopenconnect-dev libdbus-1-dev pkg-config)
- [ ] T026 [US3] Add Rust toolchain setup to build job using `actions-rust-lang/setup-rust-toolchain@v1`
- [ ] T027 [US3] Add release build step running `cargo build --release --workspace --verbose` in build job
- [ ] T028 [US3] Test build job by pushing valid code (should compile successfully)
- [ ] T029 [US3] Test build job by introducing compilation error (should fail with clear compiler error)
- [ ] T030 [US3] Verify build artifacts are produced at `target/release/akon`

**Checkpoint**: Build job is fully functional and catches compilation errors independently. All three P1 jobs (lint, test, build) now run in parallel.

---

## Phase 6: User Story 4 - Multi-Platform Build Matrix (Priority: P2)

**Goal**: Verify builds on multiple Rust versions (stable and MSRV 1.70)

**Independent Test**: Configure matrix builds and verify successful compilation on Rust stable and MSRV (1.70)

**Acceptance Criteria**:
- Code using Rust 1.70 compatible features → builds succeed on both stable and MSRV toolchains
- Code requiring newer Rust features → build fails on MSRV with clear feature compatibility error

### Implementation for User Story 4

- [ ] T031 [US4] Add strategy matrix to test job in `.github/workflows/ci.yml` with rust versions: `[stable, "1.70"]`
- [ ] T032 [US4] Update Rust toolchain setup in test job to use `${{ matrix.rust }}` toolchain
- [ ] T033 [US4] Test matrix builds by pushing code compatible with Rust 1.70 (both should pass)
- [ ] T034 [US4] Test MSRV enforcement by using a newer Rust feature (MSRV build should fail with clear error)
- [ ] T035 [US4] Optionally add strategy matrix to build job for comprehensive MSRV verification

**Checkpoint**: CI now validates compatibility with declared MSRV (1.70) in addition to stable Rust

---

## Phase 7: Branch Protection Configuration

**Purpose**: Configure GitHub branch protection to enforce CI checks before merging to main

**⚠️ IMPORTANT**: These tasks enforce the primary success criterion: "Zero broken code merged to main branch (100% CI-gated merges)"

- [ ] T036 Navigate to repository Settings → Branches on GitHub
- [ ] T037 Click "Add rule" or "Add branch protection rule"
- [ ] T038 Set branch name pattern to `main` (or your default branch name)
- [ ] T039 Enable "Require status checks to pass before merging"
- [ ] T040 Select required status checks: `lint`, `test`, `build` (all three P1 jobs)
- [ ] T041 Enable "Require branches to be up to date before merging" (recommended)
- [ ] T042 Optionally enable "Require approvals" (1 approval recommended for teams)
- [ ] T043 Optionally enable "Dismiss stale pull request approvals when new commits are pushed"
- [ ] T044 Click "Create" or "Save changes" to apply branch protection rules
- [ ] T045 Test branch protection by creating a PR with failing CI (should block merge)
- [ ] T046 Test branch protection by creating a PR with passing CI (should allow merge)
- [ ] T047 Document branch protection configuration in `quickstart.md` with screenshots or step-by-step guide

**Checkpoint**: Main branch is now protected - only code that passes all CI checks can be merged ✅

---

## Phase 8: Polish & Cross-Cutting Concerns

**Purpose**: Documentation, optimization, and refinements that affect the overall CI experience

- [ ] T048 [P] Add workflow name and description comments in `.github/workflows/ci.yml`
- [ ] T049 [P] Verify caching is working correctly by checking CI run times (cold vs warm cache)
- [ ] T050 [P] Update `quickstart.md` with CI status badge and actual observed run times
- [ ] T051 [P] Add comment in workflow file about system dependencies Ubuntu package names vs Fedora
- [ ] T052 [P] Test fork PR workflow by creating a fork and opening PR (verify read-only permissions work)
- [ ] T053 [P] Document troubleshooting steps in `quickstart.md` based on any issues encountered
- [ ] T054 [P] Add monitoring recommendations for CI performance degradation alerts

**Checkpoint**: CI pipeline is fully documented, optimized, and ready for team use

---

## Dependencies

### Story Completion Order

```
Setup (Phase 1)
    ↓
Foundational (Phase 2)
    ↓
┌─────────────┬─────────────┬─────────────┐
│   US1 (P1)  │   US2 (P1)  │   US3 (P1)  │  ← Can implement in parallel
│   (Lint)    │   (Test)    │   (Build)   │     (different jobs in same workflow)
└─────────────┴─────────────┴─────────────┘
    ↓
US4 (P2) - Builds on test/build jobs
    ↓
Polish (Phase 7)
```

### Inter-Story Dependencies

- **US1 (Lint)**: No dependencies - fully independent ✅
- **US2 (Test)**: No dependencies - fully independent ✅
- **US3 (Build)**: No dependencies - fully independent ✅
- **US4 (MSRV)**: Extends US2 and US3 with matrix strategy

**Parallel Implementation**: US1, US2, and US3 can be implemented simultaneously by different developers or in rapid succession, as they modify different sections of the same workflow file.

---

## Parallel Execution Examples

### Within Each User Story

**User Story 1 (Lint)**:
- T007, T008, T009 can be done sequentially (defining job structure)
- T010, T011 can be written in parallel (different steps)
- T012, T013, T014 are sequential test validations

**User Story 2 (Test)**:
- T015, T016, T017, T018, T019 can be done sequentially (defining job structure)
- T020, T021, T022 are sequential test validations

**User Story 3 (Build)**:
- T023, T024, T025, T026, T027 can be done sequentially (defining job structure)
- T028, T029, T030 are sequential test validations

**User Story 4 (MSRV)**:
- T031, T032 must be sequential (modifying matrix configuration)
- T033, T034, T035 are sequential test validations

### Across User Stories

**Maximum Parallelization**:
```
Developer A: Implements US1 (T007-T014) → Lint job complete
Developer B: Implements US2 (T015-T022) → Test job complete  } Can work simultaneously
Developer C: Implements US3 (T023-T030) → Build job complete
```

After P1 stories complete, any developer can add US4 (T031-T035).

---

## Task Summary

**Total Tasks**: 42

### By User Story:
- **Setup (Phase 1)**: 3 tasks
- **Foundational (Phase 2)**: 3 tasks
- **US1 - Code Quality Validation (P1)**: 8 tasks
- **US2 - Automated Test Execution (P1)**: 8 tasks
- **US3 - Build Verification (P1)**: 8 tasks
- **US4 - Multi-Platform Build Matrix (P2)**: 5 tasks
- **Polish (Phase 7)**: 7 tasks

### Parallelization:
- **6 tasks** marked [P] for parallel execution (foundation definitions and polish tasks)
- **User story implementations** can proceed in parallel (different jobs in workflow file)
- **Test validations** within each story are sequential

### MVP Definition:
- **Minimum**: US1 only (8 tasks) - Lint job provides immediate value
- **Recommended**: US1 + US2 + US3 (24 tasks) - Complete CI validation (lint + test + build)
- **Full Feature**: All 42 tasks including MSRV testing and polish

---

## Implementation Notes

### File Modifications

**Single File**: `.github/workflows/ci.yml`

All user stories modify the same workflow file but in different sections (different jobs), enabling parallel development:
- US1 modifies `jobs.lint` section
- US2 modifies `jobs.test` section
- US3 modifies `jobs.build` section
- US4 adds `strategy.matrix` to test/build jobs

**Directory Creation**: `.github/workflows/` (Phase 1)

### Testing Strategy

Each user story includes validation tasks that verify:
1. **Happy path**: Feature works correctly with valid input
2. **Failure path**: Feature catches errors and provides clear messages
3. **Integration**: Feature works with existing project structure (workspace, dependencies)

### Incremental Delivery

Each user story checkpoint represents a shippable increment:
- **After US1**: Team gets automatic linting feedback
- **After US2**: Team gets automatic test execution
- **After US3**: Team gets complete CI validation (ready to enable branch protection)
- **After US4**: Team gets MSRV compatibility verification

### Configuration Management

- Workflow respects existing project configuration:
  - `clippy.toml` for linting rules
  - `rustfmt.toml` for formatting rules
  - `Cargo.toml` for workspace structure
- No new Makefile targets required (uses cargo commands directly per FR-015)

---

## Success Criteria

CI implementation is complete when:

- ✅ All three P1 jobs (lint, test, build) run automatically on every push and PR
- ✅ Jobs run in parallel with independent failure reporting
- ✅ Clear, actionable error messages for each failure type
- ✅ CI status visible on commits and PRs via GitHub status checks
- ✅ Fork PRs from external contributors work correctly with read-only permissions
- ✅ Caching reduces subsequent build times to ~2-3 minutes
- ✅ Zero broken code merged to main branch (when branch protection is enabled)

**MVP Success** (US1 only):
- ✅ Lint job runs on every push
- ✅ Formatting and clippy violations are caught automatically
- ✅ Developers receive clear feedback on code quality issues

**Full Success** (All stories):
- ✅ All P1 jobs (lint, test, build) working
- ✅ MSRV (1.70) compatibility verified
- ✅ Complete documentation and troubleshooting guides available
