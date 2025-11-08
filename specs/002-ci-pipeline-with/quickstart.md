# CI Pipeline Quickstart

**Feature**: 002-ci-pipeline-with
**Last Updated**: 2025-11-06

## Overview

This quickstart guide helps you understand and work with the CI pipeline for the akon project. The CI automatically validates code quality, runs tests, and verifies builds on every push and pull request.

## What Gets Checked

Every time you push code or open a PR, three parallel checks run:

1. **Lint** - Code formatting and quality (cargo fmt + clippy)
2. **Test** - All unit and integration tests
3. **Build** - Release mode compilation

All three must pass for the CI to succeed.

## Quick Reference

### Before You Push

Run what CI will check (use cargo commands directly):

```bash
# Validation checks (what CI runs)
cargo fmt --all --check              # ~2 seconds - catches formatting issues
cargo clippy --workspace --all-targets -- -D warnings  # ~30 seconds - catches code quality issues
cargo test --workspace               # ~1 minute - catches test failures
cargo build --release --workspace    # ~2 minutes - catches build failures

# Or use Makefile for building only (no lint/format)
make all                             # Equivalent to: cargo build --release
make install                         # Builds + installs to /usr/local/bin with sudo config
```

### Fix Common Issues

**Formatting violations**:
```bash
cargo fmt --all      # Auto-fixes formatting
```

**Clippy warnings**:
```bash
# See warnings in detail:
cargo clippy --workspace --all-targets -- -D warnings

# Fix issues manually based on clippy suggestions
```

**Test failures**:
```bash
# Run tests with output:
cargo test --workspace --verbose

# Run specific test:
cargo test test_name

# Run tests in specific crate:
cd akon-core && cargo test
```

**Build failures**:
```bash
# Build with verbose output:
cargo build --release --workspace --verbose

# Check for linking issues:
cargo build --workspace 2>&1 | grep -i "error"
```

## Viewing CI Results

### On GitHub

1. Go to your PR or commit
2. Scroll to bottom for "Checks" section
3. Click on failed check name to see logs
4. Look for red ❌ to find specific failure

### Understanding Failure Messages

**Formatting Check Failed**:
```
error: Diff in /home/runner/work/akon/akon/src/main.rs at line 42:
-   let x  =   5;
+   let x = 5;
```
→ Fix: Run `cargo fmt --all`

**Clippy Failed**:
```
warning: this expression creates a reference which is immediately dereferenced by the compiler
  --> src/main.rs:15:20
   |
15 |     some_function(&x);
   |                    ^^ help: change this to: `x`
```
→ Fix: Apply clippy suggestion

**Test Failed**:
```
test auth::tests::test_totp_generation ... FAILED
assertion `left == right` failed
  left: "123456"
 right: "654321"
```
→ Fix: Update test or fix implementation

## CI Configuration

### Where Is It?

`.github/workflows/ci.yml` - Single workflow file

### What Does It Do?

```yaml
on:
  push:              # Runs on every push to any branch
  pull_request:      # Runs on every PR

jobs:
  lint:              # Validates formatting + clippy
  test:              # Runs all tests
  build:             # Compiles release binary
```

### How Long Does It Take?

**First run** (cold cache): 3-5 minutes
**Subsequent runs** (warm cache): 2-3 minutes

Cache automatically updates when dependencies change (Cargo.lock).

## Local Development Workflow

### Recommended: Pre-Push Checks

Add to your local workflow before pushing:

```bash
# Quick check (30 seconds)
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings

# Full validation (2-3 minutes, same as CI)
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --release --workspace
```

### Optional: Git Pre-Push Hook

Create `.git/hooks/pre-push`:

```bash
#!/bin/bash
echo "Running pre-push checks..."
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings
```

Make it executable:
```bash
chmod +x .git/hooks/pre-push
```

## Troubleshooting

### CI Passes Locally But Fails on GitHub

**System Dependencies**: CI uses Ubuntu package names. Check `.github/workflows/ci.yml` for correct packages:
- Ubuntu: `libopenconnect-dev`, `libdbus-1-dev`, `pkg-config`
- Your system (Fedora): `openconnect-devel`, `dbus-devel`, `pkgconf-pkg-config`

**Rust Version**: CI uses stable. Check your version:
```bash
rustc --version
rustup update stable
```

**Cache Issues**: Rare, but cache can become stale. GitHub auto-manages this.

### CI Takes Too Long

**Expected times**:
- Cold cache: 3-5 minutes (normal for first run after dependency changes)
- Warm cache: 2-3 minutes (normal for typical commits)

**If consistently slow**:
- Check if Cargo.lock changed (forces rebuild)
- Check if many files changed (more to compile)

### Fork PRs From External Contributors

External contributors' PRs run CI with read-only permissions (safe by default). CI should work identically to internal branches.

## Making Changes to CI

### Adding a New Job

Edit `.github/workflows/ci.yml`:

```yaml
jobs:
  lint:
    # ... existing ...

  test:
    # ... existing ...

  build:
    # ... existing ...

  new-job:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions-rust-lang/setup-rust-toolchain@v1
      - run: your-command-here
```

### Adding System Dependencies

Add to the "Install system dependencies" step:

```yaml
- name: Install system dependencies
  run: |
    sudo apt-get update
    sudo apt-get install -y libopenconnect-dev libdbus-1-dev pkg-config your-new-package
```

### Testing CI Changes

1. Push to feature branch
2. Open draft PR
3. Watch CI run
4. Iterate on workflow file
5. Mark PR ready when CI passes

## Integration with Development

### Branch Protection (Optional)

Repository maintainers can require CI to pass before merging:

1. Settings → Branches → Add rule
2. Branch name pattern: `main`
3. ✅ Require status checks to pass
4. Select: `lint`, `test`, `build`

### Status Badges (Optional)

Add to README.md:

```markdown
[![CI](https://github.com/vcwild/akon/actions/workflows/ci.yml/badge.svg)](https://github.com/vcwild/akon/actions/workflows/ci.yml)
```

## Getting Help

### CI Logs

Click on failed job in GitHub → Expand step → See detailed output

### Common Commands

```bash
# Same as CI lint job:
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings

# Same as CI test job:
cargo test --workspace --verbose

# Same as CI build job:
cargo build --release --workspace --verbose
```

### Related Files

- `.github/workflows/ci.yml` - CI workflow configuration (uses cargo commands directly)
- `clippy.toml` - Linting rules
- `rustfmt.toml` - Formatting rules
- `Makefile` - Local development targets (`make all` for build, `make install` for system installation)
- `Cargo.toml` - Workspace configuration

## Next Steps

After CI is working:

**Priority 2 Features** (not in initial implementation):
- Add MSRV (1.70) testing to verify Rust 1.70 compatibility
- Add code coverage reporting
- Add security audit job (cargo audit)
- Add release automation

**Current Focus**: Get basic CI working reliably with lint, test, and build.
