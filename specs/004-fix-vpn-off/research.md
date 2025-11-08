# Research: Fix VPN Off Command Cleanup

**Feature**: 004-fix-vpn-off
**Date**: 2025-11-08
**Phase**: 0 - Research & Discovery

## Overview

This document consolidates research findings for merging the cleanup functionality into the `vpn off` command. Since this is a bug fix leveraging existing, tested code, research focuses on understanding current implementation patterns and ensuring no regressions.

## Technical Decisions

### Decision 1: Reuse Existing `cleanup_orphaned_processes()` Function

**Context**: The `src/daemon/process.rs` module already contains a well-tested `cleanup_orphaned_processes()` function that:
- Uses `pgrep -x openconnect` to find all OpenConnect processes
- Sends SIGTERM for graceful shutdown
- Waits 5 seconds for process termination
- Escalates to SIGKILL for unresponsive processes
- Handles permission errors (EPERM) gracefully
- Returns count of terminated processes

**Decision**: Call this existing function from `run_vpn_off()` after attempting to terminate the tracked PID.

**Rationale**:
- DRY principle - avoid duplicating process cleanup logic
- Function is already tested and proven reliable
- Maintains consistent cleanup behavior across codebase
- Reduces implementation risk and testing burden

**Alternatives Considered**:
1. **Inline the cleanup logic in `run_vpn_off()`**: Rejected because it duplicates code and increases maintenance burden
2. **Create a new cleanup function**: Rejected because existing function already handles all edge cases
3. **Use `sudo pkill` approach from `perform_reconnection()`**: Rejected because it's less robust and doesn't handle permission errors as gracefully

### Decision 2: Call Cleanup After Tracked PID Termination

**Context**: Current `run_vpn_off()` implementation:
1. Reads state file to get tracked PID
2. Checks if process exists
3. Sends SIGTERM, waits 5 seconds
4. Sends SIGKILL if still running
5. Removes state file

**Decision**: Insert `cleanup_orphaned_processes()` call after step 5 (or immediately if no state file exists).

**Rationale**:
- Ensures tracked process is handled first with user feedback
- Orphaned process cleanup is then comprehensive (catches any leftovers)
- Maintains existing user experience while adding reliability
- If no tracked PID, cleanup still runs to catch orphaned processes

**Alternatives Considered**:
1. **Call cleanup before handling tracked PID**: Rejected because it could terminate the tracked process before we report its specific PID to the user
2. **Only call cleanup if tracked PID fails**: Rejected because we want comprehensive cleanup even on successful disconnect
3. **Call cleanup in parallel**: Rejected because serial execution is simpler and timeout is acceptable

### Decision 3: Maintain Separate State File Management

**Context**: State file at `/tmp/akon_vpn_state.json` tracks:
- Connection IP
- Network device
- Connection timestamp
- OpenConnect PID

**Decision**: Keep existing state file removal in `run_vpn_off()`. The `cleanup_orphaned_processes()` function does not manage state files.

**Rationale**:
- Clear separation of concerns: `run_vpn_off()` owns state file lifecycle
- `cleanup_orphaned_processes()` is a pure process termination utility
- Allows cleanup function to be called from other contexts without side effects
- State file removal already works correctly

**Alternatives Considered**:
1. **Move state file removal into cleanup function**: Rejected because it couples process cleanup to state management
2. **Update state file after each process termination**: Rejected as unnecessary complexity

### Decision 4: Deprecate `run_vpn_cleanup()` Command

**Context**: The `akon vpn cleanup` command currently calls `cleanup_orphaned_processes()` and removes the state file. With comprehensive cleanup in `vpn off`, this command becomes redundant.

**Decision**:
- Phase 1: Keep `run_vpn_cleanup()` but make it call `run_vpn_off()` internally with a deprecation warning
- Phase 2 (future): Remove the command entirely after user migration period

**Rationale**:
- Gradual deprecation prevents breaking existing user workflows/scripts
- Warning message educates users about the new simplified approach
- Allows safe migration path

**Alternatives Considered**:
1. **Remove command immediately**: Rejected due to potential breaking changes for users
2. **Keep both commands indefinitely**: Rejected because it confuses the interface
3. **Make cleanup a no-op**: Rejected because users might have valid use cases during migration

## Implementation Patterns

### Pattern 1: Error Handling for Permission Issues

**Current Pattern**: `cleanup_orphaned_processes()` handles `nix::errno::Errno::EPERM` gracefully by logging a warning and continuing.

**Application**: No changes needed. The function already handles cases where:
- OpenConnect processes are owned by different users
- Current user lacks permission to terminate processes
- sudo is required but not available

### Pattern 2: Async/Await in Command Functions

**Current Pattern**: `run_vpn_off()` is already async and uses `tokio::time::sleep()` for timeout handling.

**Application**: No changes needed. Call to `cleanup_orphaned_processes()` is synchronous (it uses `std::thread::sleep()`), which is fine from an async context.

### Pattern 3: User Feedback During Operations

**Current Pattern**: `run_vpn_off()` provides rich user feedback with:
- Emoji indicators (🔌 for disconnect, ✓ for success, ⚠ for warnings)
- Colored output using `colored` crate
- Progress messages during operations

**Application**: Add similar feedback messages after cleanup phase:
```rust
println!("🧹 Cleaning up any orphaned OpenConnect processes...");
// ... call cleanup ...
println!("✓ Cleanup complete: terminated N process(es)");
```

## Best Practices Applied

### Rust Process Management

**Research Findings**:
- `nix::sys::signal::kill()` is the idiomatic Rust way to send signals
- Checking for process existence: `kill(pid, None)` returns `ESRCH` if process doesn't exist
- SIGTERM should be preferred for graceful shutdown before SIGKILL
- Timeout patterns prevent indefinite hangs

**Application**: Existing code already follows these practices. No changes needed.

### CLI Design Patterns

**Research Findings**:
- Commands should be composable and have clear single responsibilities
- Redundant commands confuse users and increase maintenance burden
- Deprecation warnings help migrate users smoothly

**Application**: Consolidating cleanup into `vpn off` follows CLI best practices for simplicity and clarity.

### Testing Strategy for Process Management

**Research Findings**:
- Process termination tests require careful setup/teardown
- Mock processes or use actual test processes that can be safely killed
- Integration tests verify end-to-end behavior
- Unit tests verify logic in isolation

**Application**:
- Extend existing `tests/vpn_disconnect_tests.rs`
- Add tests that spawn test processes before running cleanup
- Verify zero processes remain after cleanup
- Test permission error handling paths

## Dependencies

### Existing Dependencies (No Changes)

- `nix` v0.27+ - Signal handling APIs
- `tokio` v1.35+ - Async runtime with time utilities
- `serde_json` v1.0+ - State file serialization
- `tracing` v0.1+ - Structured logging
- `colored` v2.1+ - Terminal output formatting

### New Dependencies

**None required**. All functionality uses existing dependencies.

## Risk Assessment

### Low Risk Areas

✅ **Reusing tested code**: `cleanup_orphaned_processes()` is already proven
✅ **No new dependencies**: Zero supply chain risk
✅ **Isolated changes**: Only modifying one function in one file
✅ **Backward compatible**: State file format unchanged

### Medium Risk Areas

⚠️ **Timing changes**: Adding cleanup extends command duration by 5-10 seconds
⚠️ **User expectations**: Users might expect instant disconnect
⚠️ **Sudo requirements**: Cleanup might fail without proper permissions

**Mitigation**:
- Add progress messages so users know cleanup is in progress
- Log clear warnings when permission errors occur
- Document sudo requirements in error messages and help text

### Negligible Risk Areas

✓ **Breaking changes**: Minimal - only removing redundant command (with deprecation)
✓ **Security**: No credential handling or sensitive data involved
✓ **Performance**: 5-10 second cleanup is acceptable for disconnect operation

## Open Questions

**All research questions resolved.** No NEEDS CLARIFICATION items remain.

## References

- Existing implementation: `src/daemon/process.rs:192-303`
- Current disconnect logic: `src/cli/vpn.rs:804-950`
- Signal handling docs: https://docs.rs/nix/latest/nix/sys/signal/
- Process management best practices: Rust cookbook - process execution patterns

## Next Steps

Proceed to **Phase 1: Design & Contracts** with:
- High confidence in implementation approach
- No blocking technical questions
- Clear path forward using existing, tested components
