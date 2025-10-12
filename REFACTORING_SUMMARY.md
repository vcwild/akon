# OpenConnect Refactoring Summary

## Current Status

We've successfully refactored the OpenConnect VPN implementation following **Option A: Password Callback Pattern** for safe, leak-free credential handling.

## Key Improvements

### 1. Removed Global State
- **Before**: Used `static GLOBAL_CREDENTIALS: Mutex<Option<VpnCredentials>>`
- **After**: Credentials stored in struct fields, no global state

### 2. Fixed Memory Leaks
- **Before**: CStrings were leaked via `.into_raw()` with unclear ownership
- **After**: CStrings owned by struct, automatically freed in `Drop`

### 3. Password Callback Pattern
```rust
// Password stored in struct as CString
password_cstr: Option<CString>

// In connect():
// Use OpenConnect API functions instead of direct struct access
openconnect_passphrase_from_fsid(vpn)  // Use API functions

// In Drop:
// CStrings automatically dropped, no manual cleanup needed
```

### 4. Current Issue: Opaque Struct

The `openconnect_info` struct is opaque - bindgen doesn't generate field accessors. We need to:

**Solution**: Use OpenConnect's public API functions exclusively:
- `openconnect_set_passwd_cb()` - Set password callback
- `openconnect_passphrase_from_fsid()` - Use for password handling
- `openconnect_disable_dtls()` - Already available
- `openconnect_set_reported_os()` - For configuration

## Implementation Strategy

Based on OpenConnect source code patterns:

1. **Don't access struct fields directly** - use API functions
2. **Use `openconnect_set_passwd_cb()`** to register password callback
3. **Keep credentials in Rust struct** - pass via userdata/privdata mechanism
4. **Let OpenConnect call our callback** when it needs credentials

## Next Steps

1. Update `openconnect.rs` to use only API functions (no struct field access)
2. Implement proper password callback registration via API
3. Test with actual F5 VPN connection
4. Document the final safe pattern

## References

- OpenConnect GitLab: https://gitlab.com/openconnect/openconnect
- API Documentation: `/usr/include/openconnect.h`
- Our implementation: `akon-core/src/vpn/openconnect.rs`
