# Safe OpenConnect Refactoring - Complete

## Summary

Successfully refactored the OpenConnect FFI implementation from an unsafe global-state pattern to a safe, leak-free design using the auth form callback pattern.

## Build & Test Status

✅ **Build**: Success
✅ **Tests**: 39/39 passed

```bash
cargo check
# Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.16s

cargo test -p akon-core
# test result: ok. 39 passed; 0 failed; 0 ignored
```

## The Problem

The original implementation had several safety issues:

1. **Global Mutex**: `static GLOBAL_CREDENTIALS: Mutex<Option<VpnCredentials>>`
   - Not thread-safe across multiple VPN connections
   - Shared mutable state

2. **Memory Leaks**: `CString::into_raw()` leaked pointers
   - Unclear ownership
   - OpenConnect managing Rust-allocated memory

3. **Unsafe FFI**: Direct struct field access
   - Attempting to access `(*vpn).username`, `(*vpn).privdata`
   - These fields don't exist - `openconnect_info` is opaque

## The Solution

### Safe Pattern: Box + Privdata + Auth Form Callback

```rust
// 1. Credentials in a Box
struct VpnCredentials {
    username: CString,
    password: CString,
}

// 2. Pass via privdata to OpenConnect
let creds = Box::new(VpnCredentials { ... });
let creds_ptr = Box::into_raw(creds);

let vpn = openconnect_vpninfo_new(
    ptr::null(),
    None,
    None,
    Some(process_auth_form),      // Callback receives privdata
    None,
    creds_ptr as *mut c_void,     // Our credentials
);

// 3. Store Box for Drop
self.credentials = Some(Box::from_raw(creds_ptr));

// 4. Callback uses privdata
unsafe extern "C" fn process_auth_form(
    privdata: *mut c_void,
    form: *mut oc_auth_form,
) -> c_int {
    let creds = &*(privdata as *const VpnCredentials);
    // Use libc::strdup so OpenConnect can free
    (*opt)._value = libc::strdup(creds.username.as_ptr());
    0
}

// 5. Clean Drop
impl Drop {
    fn drop(&mut self) {
        openconnect_vpninfo_free(self.vpn);
        // credentials Box auto-dropped
    }
}
```

## Why This Works

1. **No Global State**: Each `OpenConnectConnection` owns its credentials
2. **Clear Ownership**:
   - Rust owns `Box<VpnCredentials>`
   - OpenConnect owns `libc::strdup()` results
3. **Type Safety**: Compiler ensures credentials live long enough
4. **No Leaks**: All allocations tracked and freed properly

## Key Insights

- `openconnect_info` is **opaque** - can't access fields directly
- Must use OpenConnect API functions exclusively
- `privdata` parameter is the bridge between Rust and C
- `libc::strdup()` creates C-owned copies for OpenConnect

## Architecture

```
┌─────────────────────────────────────┐
│   OpenConnectConnection (Rust)     │
│  ┌────────────────────────────────┐ │
│  │ Box<VpnCredentials>            │ │
│  │  ├─ username: CString          │ │
│  │  └─ password: CString          │ │
│  └────────────────────────────────┘ │
│           │                          │
│           │ Box::into_raw()          │
│           ↓                          │
│      *mut c_void                     │
│           │                          │
└───────────┼──────────────────────────┘
            │
            │ privdata parameter
            ↓
┌─────────────────────────────────────┐
│   openconnect_info (C Library)      │
│  ┌────────────────────────────────┐ │
│  │ Opaque struct                  │ │
│  │ (fields not accessible)        │ │
│  └────────────────────────────────┘ │
│           │                          │
│           │ When auth needed         │
│           ↓                          │
│   process_auth_form(privdata, form) │
└─────────────────────────────────────┘
            │
            │ Cast back
            ↓
    &VpnCredentials (read-only access)
            │
            │ libc::strdup()
            ↓
    C-owned string → OpenConnect frees
```

## Files Changed

- `akon-core/src/vpn/openconnect.rs` - Safe implementation
- `akon-core/Cargo.toml` - Added `libc` dependency
- `akon-core/build.rs` - Unchanged (still using bindgen)
- Documentation files created

## Next Steps

- [x] Implement safe pattern
- [x] Fix compilation errors
- [x] Pass all tests
- [ ] Test with actual VPN connection
- [ ] Add integration tests
- [ ] Consider adding callback logging

## References

- Implementation: `akon-core/src/vpn/openconnect.rs`
- Documentation: `REFACTORING_COMPLETE.md`
- Setup guide: `OPENCONNECT_SETUP.md`
