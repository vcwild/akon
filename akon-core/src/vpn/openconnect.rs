//! OpenConnect VPN connection handling
//!
//! This module provides safe Rust wrappers around the OpenConnect C library
//! for establishing VPN connections with PIN + OTP authentication.
//!
//! Uses auth form callback pattern for secure, leak-free credential handling.

use std::ffi::{CStr, CString};
use std::os::raw::{c_int, c_void};
use std::ptr;

use crate::error::{AkonError, VpnError};

// Include the generated bindings from build.rs
#[allow(non_camel_case_types)]
#[allow(non_upper_case_globals)]
#[allow(unused)]
#[allow(non_snake_case)]
#[allow(dead_code)]
mod bindings {
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

use bindings::*;

/// Credentials stored in a Box and passed via privdata
/// This avoids global state while keeping credentials accessible to callbacks
struct VpnCredentials {
    username: CString,
    password: CString,
    /// Track allocated pointers that need to be freed
    allocated: Vec<*mut std::os::raw::c_char>,
}

impl VpnCredentials {
    fn new(username: &str, password: &str) -> Result<Self, AkonError> {
        Ok(Self {
            username: CString::new(username).map_err(|_| {
                AkonError::Vpn(VpnError::ConnectionFailed {
                    reason: "Invalid username".to_string(),
                })
            })?,
            password: CString::new(password).map_err(|_| {
                AkonError::Vpn(VpnError::ConnectionFailed {
                    reason: "Invalid password".to_string(),
                })
            })?,
            allocated: Vec::new(),
        })
    }

    unsafe fn record_alloc(&mut self, p: *mut std::os::raw::c_char) {
        if !p.is_null() {
            self.allocated.push(p);
        }
    }
}

/// Safe wrapper for OpenConnect VPN connection
///
/// This implementation uses auth form callback with proper ownership:
/// - No global state or Mutex
/// - Credentials stored in Box, passed via privdata
/// - No CString leaks - all memory properly managed
pub struct OpenConnectConnection {
    vpn: *mut openconnect_info,
    /// Raw pointer to credentials (leaked from Box, must be reclaimed in Drop)
    auth_box_ptr: *mut VpnCredentials,
    /// Keep server CString alive
    server_cstr: Option<CString>,
}

/// Auth form callback for OpenConnect
/// This is called when OpenConnect needs to fill in authentication forms.
/// We get credentials from privdata (VpnCredentials Box) and fill the form.
unsafe extern "C" fn process_auth_form(
    privdata: *mut c_void,
    form: *mut oc_auth_form,
) -> c_int {
    if form.is_null() || privdata.is_null() {
        tracing::error!("process_auth_form: null privdata or form");
        return -1;
    }

    // Cast privdata back to our credentials (mutable so we can track allocations)
    let creds = &mut *(privdata as *mut VpnCredentials);

    tracing::debug!("Processing auth form");

    // Iterate through form options and fill them
    let mut opt = (*form).opts;
    while !opt.is_null() {
        let name_ptr = (*opt).name;
        if !name_ptr.is_null() {
            let name_str = CStr::from_ptr(name_ptr).to_string_lossy();
            tracing::debug!("Auth form field: {}", name_str);

            // Fill username fields
            if name_str.contains("user") || name_str.contains("name") {
                // Allocate with libc::strdup so OpenConnect can manage it
                let dup = libc::strdup(creds.username.as_ptr());
                if dup.is_null() {
                    tracing::error!("strdup username failed");
                    return -1;
                }
                (*opt)._value = dup;
                creds.record_alloc(dup);
                tracing::debug!("Set username field");
            }
            // Fill password fields
            else if name_str.contains("pass") || name_str.contains("secret") {
                // Allocate with libc::strdup so OpenConnect can manage it
                let dup = libc::strdup(creds.password.as_ptr());
                if dup.is_null() {
                    tracing::error!("strdup password failed");
                    return -1;
                }
                (*opt)._value = dup;
                creds.record_alloc(dup);
                tracing::debug!("Set password field");
            }
        }
        opt = (*opt).next;
    }

    tracing::info!("Auth form processed successfully");
    0 // Success (OC_FORM_RESULT_OK)
}

impl OpenConnectConnection {
    /// Initialize OpenConnect SSL library globally
    /// This MUST be called before forking any daemon processes
    pub fn init_ssl() -> Result<(), AkonError> {
        unsafe {
            let ret = openconnect_init_ssl();
            if ret != 0 {
                return Err(AkonError::Vpn(VpnError::ConnectionFailed {
                    reason: "Failed to initialize OpenConnect SSL".to_string(),
                }));
            }
            Ok(())
        }
    }

    /// Create a new VPN connection
    /// Note: This only creates the struct, actual initialization happens in connect()
    pub fn new() -> Result<Self, AkonError> {
        Ok(Self {
            vpn: ptr::null_mut(),
            auth_box_ptr: ptr::null_mut(),
            server_cstr: None,
        })
    }

    /// Connect to VPN server
    pub fn connect(
        &mut self,
        server: &str,
        username: &str,
        password: &str,
        protocol: &str,
        no_dtls: bool,
    ) -> Result<(), AkonError> {
        tracing::info!("Starting VPN connection to {}", server);
        unsafe {
            // Create credentials box
            let auth_box = Box::new(VpnCredentials::new(username, password)?);

            // Convert Box to raw pointer - this is what will be passed to OpenConnect as privdata
            // We'll store this pointer and reclaim it in Drop
            let auth_box_ptr = Box::into_raw(auth_box);

            // Free the old vpninfo if it exists (it won't on first call after new())
            if !self.vpn.is_null() {
                openconnect_vpninfo_free(self.vpn);
            }

            // Free old credentials if they exist
            if !self.auth_box_ptr.is_null() {
                // Reconstruct Box and let it drop to free allocations
                let mut old_auth = Box::from_raw(self.auth_box_ptr);
                for p in old_auth.allocated.drain(..) {
                    if !p.is_null() {
                        libc::free(p as *mut c_void);
                    }
                }
            }

            // Store the new auth_box_ptr
            self.auth_box_ptr = auth_box_ptr;

            // Create vpninfo with our auth callback - this MUST be first
            tracing::info!("Creating vpninfo structure with auth callback");
            self.vpn = openconnect_vpninfo_new(
                ptr::null(),                       // useragent (default)
                None,                              // validate_peer_cert (we accept all certificates)
                None,                              // write_new_config
                Some(process_auth_form),           // process_auth_form callback
                None,                              // progress
                auth_box_ptr as *mut c_void,       // privdata (raw pointer to our AuthPriv)
            );

            if self.vpn.is_null() {
                tracing::error!("openconnect_vpninfo_new returned null");
                return Err(AkonError::Vpn(VpnError::ConnectionFailed {
                    reason: "Failed to create VPN info structure".to_string(),
                }));
            }
            tracing::info!("vpninfo created successfully");

            // Set protocol BEFORE parsing URL
            let protocol_cstr = CString::new(protocol).map_err(|_| {
                AkonError::Vpn(VpnError::ConnectionFailed {
                    reason: "Invalid protocol".to_string(),
                })
            })?;

            tracing::info!("Setting protocol: {}", protocol);
            let ret = openconnect_set_protocol(self.vpn, protocol_cstr.as_ptr());
            if ret != 0 {
                tracing::error!("openconnect_set_protocol failed: {}", ret);
                return Err(AkonError::Vpn(VpnError::ConnectionFailed {
                    reason: format!("Failed to set protocol: {}", protocol),
                }));
            }

            // Parse server URL - this sets up hostname, port, and URL path
            // For F5, just pass the hostname, OpenConnect will add https:// if needed
            let server_url = if server.starts_with("http://") || server.starts_with("https://") {
                server.to_string()
            } else {
                format!("https://{}", server)
            };

            self.server_cstr = Some(CString::new(server_url.as_str()).map_err(|_| {
                AkonError::Vpn(VpnError::ConnectionFailed {
                    reason: "Invalid server URL".to_string(),
                })
            })?);

            tracing::info!("Parsing server URL: {}", server_url);
            let ret = openconnect_parse_url(self.vpn, self.server_cstr.as_ref().unwrap().as_ptr());
            if ret != 0 {
                tracing::error!("openconnect_parse_url failed: {}", ret);
                return Err(AkonError::Vpn(VpnError::ConnectionFailed {
                    reason: format!("Failed to parse URL: {}", server),
                }));
            }

            // Disable DTLS if requested
            if no_dtls {
                tracing::info!("Disabling DTLS");
                let ret = openconnect_disable_dtls(self.vpn);
                if ret != 0 {
                    tracing::warn!("openconnect_disable_dtls failed: {}", ret);
                }
            }

            // Set local hostname for the connection
            let hostname_cstr = CString::new("akon-client").unwrap();
            let ret = openconnect_set_localname(self.vpn, hostname_cstr.as_ptr());
            if ret != 0 {
                tracing::warn!("openconnect_set_localname failed: {}", ret);
            }

            // Obtain cookie (authenticate) - this will trigger our auth callback
            tracing::info!("Obtaining authentication cookie (this will make HTTPS requests)");
            let ret = openconnect_obtain_cookie(self.vpn);
            if ret != 0 {
                tracing::error!("openconnect_obtain_cookie failed: {}", ret);
                return Err(AkonError::Vpn(VpnError::AuthenticationFailed));
            }
            tracing::info!("Authentication successful");

            // Make CSTP connection
            let ret = openconnect_make_cstp_connection(self.vpn);
            if ret != 0 {
                return Err(AkonError::Vpn(VpnError::ConnectionFailed {
                    reason: "Failed to establish CSTP connection".to_string(),
                }));
            }

            // Setup TUN device
            let ret = openconnect_setup_tun_device(
                self.vpn,
                ptr::null(), // vpnc_script (default)
                ptr::null(), // ifname (auto)
            );
            if ret != 0 {
                return Err(AkonError::Vpn(VpnError::ConnectionFailed {
                    reason: "Failed to setup TUN device".to_string(),
                }));
            }

            // Setup DTLS if not disabled
            if !no_dtls {
                let _ = openconnect_setup_dtls(self.vpn, 60);
            }

            Ok(())
        }
    }

    /// Run the main connection loop (blocks until disconnected)
    pub fn run_mainloop(&mut self) -> Result<(), AkonError> {
        unsafe {
            let ret = openconnect_mainloop(
                self.vpn,
                300, // reconnect_timeout (5 minutes)
                30,  // reconnect_interval (30 seconds)
            );

            if ret != 0 {
                return Err(AkonError::Vpn(VpnError::ConnectionFailed {
                    reason: "Main loop exited with error".to_string(),
                }));
            }

            Ok(())
        }
    }

    /// Disconnect from VPN
    pub fn disconnect(&mut self) -> Result<(), AkonError> {
        // Disconnect is handled automatically in Drop
        Ok(())
    }
}

impl Drop for OpenConnectConnection {
    fn drop(&mut self) {
        unsafe {
            // Free vpninfo first (so it stops using privdata)
            if !self.vpn.is_null() {
                openconnect_vpninfo_free(self.vpn);
            }

            // Now free any strdup-allocated pointers and reclaim the Box
            if !self.auth_box_ptr.is_null() {
                // Reconstruct the Box to drop it
                let mut auth = Box::from_raw(self.auth_box_ptr);
                // Free strdup-ed pointers
                for p in auth.allocated.drain(..) {
                    if !p.is_null() {
                        libc::free(p as *mut c_void);
                    }
                }
                // auth's username/password CString will be dropped automatically
            }
        }
    }
}
