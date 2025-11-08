# Quickstart: Fix VPN Off Command Cleanup

**Feature**: 004-fix-vpn-off
**Date**: 2025-11-08
**For**: Developers implementing this bug fix

## 🎯 What You're Building

Fix the `akon vpn off` command to ensure it cleans up **all** OpenConnect processes, not just the tracked one. This eliminates residual VPN connections that can cause network conflicts and confusion about connection state.

## 📋 Quick Context

**Problem**: Currently, `akon vpn off` only terminates the OpenConnect process whose PID is stored in the state file. If orphaned processes exist from previous sessions (crashes, forced kills, etc.), they remain running and maintain VPN connections.

**Solution**: After terminating the tracked process, call the existing `cleanup_orphaned_processes()` function to comprehensively clean up all OpenConnect processes.

**Impact**: Users get reliable disconnection with a single command. No more `akon vpn cleanup` needed.

## 🛠️ Implementation Steps

### Step 1: Modify `run_vpn_off()` Function

**File**: `src/cli/vpn.rs`

**Location**: Lines ~804-920 (current implementation)

**Changes**:

1. After the existing disconnect logic (SIGTERM → wait → SIGKILL → remove state file)
2. Add a call to `cleanup_orphaned_processes()` with user feedback
3. Handle the result appropriately (don't fail on partial cleanup)

**Pseudocode**:

```rust
pub async fn run_vpn_off() -> Result<(), AkonError> {
    // [EXISTING CODE] - Handle tracked PID termination
    // ... read state file, send SIGTERM, wait, send SIGKILL, remove state file ...

    // [NEW CODE] - Comprehensive cleanup of orphaned processes
    println!(
        "{} {}",
        "🧹".bright_yellow(),
        "Cleaning up any orphaned OpenConnect processes...".bright_white()
    );

    match cleanup_orphaned_processes() {
        Ok(0) => {
            println!("  {} No orphaned processes found", "✓".bright_green());
        }
        Ok(count) => {
            println!(
                "  {} Terminated {} orphaned process(es)",
                "✓".bright_green(),
                count.to_string().bright_yellow()
            );
        }
        Err(e) => {
            warn!("Orphan cleanup failed: {}", e);
            println!(
                "  {} Warning: Could not verify all processes cleaned up",
                "⚠".bright_yellow()
            );
        }
    }

    println!(
        "{} {}",
        "✓".bright_green(),
        "Disconnect complete".bright_green().bold()
    );

    Ok(())
}
```

**Import Needed** (add to top of file if not present):

```rust
use crate::daemon::process::cleanup_orphaned_processes;
```

### Step 2: Handle Edge Case - No Active Connection

**Scenario**: User runs `akon vpn off` when no state file exists (no tracked connection).

**Current Behavior**: Prints "No active connection" and exits early.

**New Behavior**: Still run orphan cleanup to catch any lingering processes.

**Code Change**:

```rust
// Around line 815 in current implementation
if !state_path.exists() {
    println!("No active VPN connection found");

    // [NEW CODE] - Still clean up orphans
    println!(
        "{} {}",
        "🧹".bright_yellow(),
        "Checking for orphaned OpenConnect processes...".bright_white()
    );

    match cleanup_orphaned_processes() {
        Ok(0) => {
            println!("  {} No orphaned processes found", "✓".bright_green());
        }
        Ok(count) => {
            println!(
                "  {} Terminated {} orphaned process(es)",
                "✓".bright_green(),
                count
            );
        }
        Err(e) => {
            warn!("Orphan cleanup failed: {}", e);
        }
    }

    return Ok(());
}
```

### Step 3: Deprecate `run_vpn_cleanup()` Function

**File**: `src/cli/vpn.rs`

**Location**: Lines ~1184-1230 (current implementation)

**Changes**: Replace entire function body with deprecation warning and delegation.

**New Implementation**:

```rust
/// Run the VPN cleanup command (DEPRECATED - use `akon vpn off` instead)
///
/// This command is deprecated. Use `akon vpn off` which now includes comprehensive cleanup.
pub async fn run_vpn_cleanup() -> Result<(), AkonError> {
    println!(
        "{} {}",
        "⚠".bright_yellow(),
        "DEPRECATED: 'akon vpn cleanup' is deprecated.".bright_yellow().bold()
    );
    println!(
        "  Use {} instead.",
        "akon vpn off".bright_cyan()
    );
    println!("  This command will be removed in a future version.");
    println!();

    // Delegate to run_vpn_off for now
    run_vpn_off().await
}
```

### Step 4: Add Tests

**File**: `tests/vpn_disconnect_tests.rs` (extend existing file)

**New Test 1**: Verify comprehensive cleanup

```rust
#[tokio::test]
async fn test_vpn_off_cleans_up_orphaned_processes() {
    // Setup: Spawn 3 test processes, track one
    // Execute: run_vpn_off()
    // Assert: All 3 processes terminated
    // Teardown: Clean up any remaining processes
}
```

**New Test 2**: Verify cleanup runs even with no active connection

```rust
#[tokio::test]
async fn test_vpn_off_cleans_orphans_when_no_active_connection() {
    // Setup: No state file, 2 orphaned processes
    // Execute: run_vpn_off()
    // Assert: Both orphaned processes terminated
    // Teardown: Clean up
}
```

**New Test 3**: Verify permission errors don't fail command

```rust
#[tokio::test]
async fn test_vpn_off_handles_permission_errors_gracefully() {
    // Setup: Process owned by different user
    // Execute: run_vpn_off() without sudo
    // Assert: Returns Ok(()), logs warning
    // Teardown: sudo cleanup
}
```

## ✅ Definition of Done

Before marking this feature complete, verify:

- [ ] `run_vpn_off()` calls `cleanup_orphaned_processes()` after handling tracked PID
- [ ] Cleanup runs even when no state file exists (no tracked connection)
- [ ] User sees clear progress messages during cleanup
- [ ] Command succeeds (returns `Ok(())`) even if some processes can't be terminated
- [ ] Warnings are logged for permission errors
- [ ] `run_vpn_cleanup()` shows deprecation warning and delegates to `run_vpn_off()`
- [ ] All new tests pass
- [ ] Existing tests still pass (no regressions)
- [ ] Manual test: Run `akon vpn off` after starting multiple `openconnect` processes
- [ ] Manual test: Verify `pgrep -x openconnect` returns no results after disconnect

## 🧪 Manual Testing Script

```bash
#!/bin/bash
# Test script for comprehensive cleanup

# 1. Start VPN normally
sudo akon vpn on

# 2. In another terminal, spawn orphaned openconnect process (will fail auth but stays running briefly)
sudo openconnect vpn.example.com &

# 3. Check running processes (should see 2+)
pgrep -x openconnect | wc -l

# 4. Disconnect
sudo akon vpn off

# 5. Verify all processes cleaned up
if [ $(pgrep -x openconnect | wc -l) -eq 0 ]; then
    echo "✅ SUCCESS: All OpenConnect processes cleaned up"
else
    echo "❌ FAILURE: Residual OpenConnect processes remain"
    pgrep -x openconnect
fi
```

## 📚 Key Files Reference

| File | Purpose | Changes |
|------|---------|---------|
| `src/cli/vpn.rs` | CLI command implementations | Modify `run_vpn_off()`, deprecate `run_vpn_cleanup()` |
| `src/daemon/process.rs` | Process management utilities | Reference only, no changes |
| `tests/vpn_disconnect_tests.rs` | Disconnect integration tests | Add comprehensive cleanup tests |
| `/tmp/akon_vpn_state.json` | VPN state tracking | Read by `run_vpn_off()`, no format changes |

## 🚀 Estimated Effort

- **Implementation**: 1-2 hours (straightforward code changes)
- **Testing**: 2-3 hours (writing and running comprehensive tests)
- **Documentation**: 30 minutes (update help text, comments)
- **Total**: 4-5 hours

## 💡 Tips & Gotchas

1. **Import Statement**: Don't forget to add `use crate::daemon::process::cleanup_orphaned_processes;` at the top of `vpn.rs`

2. **Async Context**: The cleanup function is synchronous (uses `std::thread::sleep`), but it's safe to call from async context

3. **Sudo Requirements**: Tests involving process termination may require sudo. Consider using mock processes or test doubles.

4. **Error Handling**: The cleanup should be "best effort" - log warnings but don't fail the entire command if some processes can't be terminated

5. **User Feedback**: The emoji and colored output are important for UX - maintain the pattern from existing code

6. **State File Handling**: Remove state file **before** calling cleanup to ensure consistent state

## 🔗 Related Documentation

- [Feature Spec](./spec.md) - Full requirements and acceptance criteria
- [Research Notes](./research.md) - Design decisions and alternatives considered
- [Data Model](./data-model.md) - State file format and process lifecycle
- [Function Contracts](./contracts/function-contracts.md) - API contracts and testing requirements

## 🆘 Need Help?

- **Question about existing code**: Check `src/daemon/process.rs` for the `cleanup_orphaned_processes()` reference implementation
- **Testing questions**: Review existing tests in `tests/vpn_disconnect_tests.rs` for patterns
- **Constitution compliance**: See [Constitution Check](./plan.md#constitution-check) section in plan.md

---

**Ready to implement?** Start with Step 1, test incrementally, and refer back to this guide as needed. Good luck! 🚀
