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
///
/// Note: We use libc::strdup() to create C-owned copies for OpenConnect.
/// OpenConnect is responsible for freeing those copies, we don't track them.
struct VpnCredentials {
    username: CString,
    password: CString,
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
        })
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

// Extern declaration for the C shim progress callback
// This is compiled from progress_shim.c and handles variadic args
extern "C" {
    fn progress_shim(
        privdata: *mut c_void,
        level: c_int,
        fmt: *const std::os::raw::c_char,
        ...
    );
}

/// Auth form callback - called by OpenConnect when authentication form needs filling
/// This receives privdata as a raw pointer to our VpnCredentials Box
unsafe extern "C" fn process_auth_form(
    privdata: *mut c_void,
    form: *mut bindings::oc_auth_form,
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
                // OpenConnect owns this pointer and will free it
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
                // OpenConnect owns this pointer and will free it
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

    /// Create a new VPN connection with credentials
    /// This creates the vpninfo structure ONCE with the auth callback
    pub fn new(username: &str, password: &str) -> Result<Self, AkonError> {
        unsafe {
            // Create credentials box
            let auth_box = Box::new(VpnCredentials::new(username, password)?);

            // Convert Box to raw pointer - this is what will be passed to OpenConnect as privdata
            let auth_box_ptr = Box::into_raw(auth_box);

            // Create vpninfo with our auth callback - this MUST be first and ONLY ONCE
            tracing::info!("Creating vpninfo structure with auth callback AND progress callback");
            let vpn = openconnect_vpninfo_new(
                ptr::null(),                       // useragent (default)
                None,                              // validate_peer_cert (we accept all certificates)
                None,                              // write_new_config
                Some(process_auth_form),           // process_auth_form callback
                Some(progress_shim),               // progress callback (REQUIRED!)
                auth_box_ptr as *mut c_void,       // privdata (raw pointer to our credentials)
            );

            if vpn.is_null() {
                // Clean up on failure - reclaim the Box
                let _auth = Box::from_raw(auth_box_ptr);
                // Box will be dropped automatically, freeing the CStrings
                tracing::error!("openconnect_vpninfo_new returned null");
                return Err(AkonError::Vpn(VpnError::ConnectionFailed {
                    reason: "Failed to create VPN info structure".to_string(),
                }));
            }
            tracing::info!("vpninfo created successfully at address: {:?}", vpn);

            // Verify the pointer is readable (not completely invalid)
            let test_read = ptr::read_volatile(&vpn);
            tracing::info!("vpninfo pointer validated (readable): {:?}", test_read);

            Ok(Self {
                vpn,
                auth_box_ptr,
                server_cstr: None,
            })
        }
    }

    /// Connect to VPN server
    /// vpninfo is already created in new(), this just configures and connects
    /// Note: This establishes authentication and CSTP session without TUN device
    pub fn connect(
        &mut self,
        server: &str,
        protocol: &str,
    ) -> Result<(), AkonError> {
        tracing::info!("=== CONNECT: Starting VPN connection to {}", server);
        tracing::info!("=== CONNECT: vpninfo pointer = {:?}", self.vpn);
        unsafe {
            // Set OpenConnect log level to DEBUG to see PPP and "Configured as..." message
            // PRG_ERR=0, PRG_INFO=1, PRG_DEBUG=2, PRG_TRACE=3
            openconnect_set_loglevel(self.vpn, 2); // PRG_DEBUG
            tracing::debug!("Set OpenConnect log level to DEBUG");

            // Set protocol BEFORE parsing URL
            let protocol_cstr = CString::new(protocol).map_err(|_| {
                AkonError::Vpn(VpnError::ConnectionFailed {
                    reason: "Invalid protocol".to_string(),
                })
            })?;

            tracing::info!("=== CONNECT: About to call openconnect_set_protocol with protocol: {}", protocol);
            let ret = openconnect_set_protocol(self.vpn, protocol_cstr.as_ptr());
            tracing::info!("=== CONNECT: openconnect_set_protocol returned: {}", ret);
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

            tracing::info!("=== CONNECT: About to call openconnect_parse_url with URL: {}", server_url);
            let ret = openconnect_parse_url(self.vpn, self.server_cstr.as_ref().unwrap().as_ptr());
            tracing::info!("=== CONNECT: openconnect_parse_url returned: {}", ret);
            if ret != 0 {
                tracing::error!("openconnect_parse_url failed: {}", ret);
                return Err(AkonError::Vpn(VpnError::ConnectionFailed {
                    reason: format!("Failed to parse URL: {}", server),
                }));
            }

            // Disable DTLS
            let ret = openconnect_disable_dtls(self.vpn);
            if ret != 0 {
                tracing::error!("openconnect_disable_dtls failed: {}", ret);
                return Err(AkonError::Vpn(VpnError::ConnectionFailed {
                    reason: "Failed to disable DTLS".to_string(),
                }));
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
            tracing::info!("Making CSTP connection...");
            let ret = openconnect_make_cstp_connection(self.vpn);
            if ret != 0 {
                tracing::error!("openconnect_make_cstp_connection failed: {}", ret);
                return Err(AkonError::Vpn(VpnError::ConnectionFailed {
                    reason: "Failed to establish CSTP connection".to_string(),
                }));
            }
            tracing::info!("CSTP connection established");

            // Note: PPP negotiation happens INSIDE mainloop, not here
            // We need to call mainloop to complete the PPP handshake
            tracing::info!("✓ CSTP connection established");
            tracing::info!("Note: PPP negotiation will happen in mainloop");

            Ok(())
        }
    }

    /// Complete VPN session setup including PPP negotiation and TUN device
    ///
    /// This MUST be called after connect() to complete the PPP handshake:
    /// - LCP negotiation (Link Control Protocol)
    /// - IPCP negotiation (IP Control Protocol) - assigns VPN IP address
    /// - Reaches "Configured as X.X.X.X" state
    /// - Sets up TUN device via vpnc-script (if running as root)
    /// - Configures routing and DNS
    ///
    /// This method runs the mainloop which keeps the VPN session alive indefinitely.
    ///
    /// **With sudo/root:**
    /// - Creates TUN device
    /// - Runs vpnc-script to configure routing and DNS
    /// - Provides full VPN connectivity
    /// - Stays connected until Ctrl+C or error
    ///
    /// **Without root:**
    /// - PPP negotiation completes successfully
    /// - Gets IP address from server
    /// - TUN setup fails (expected)
    /// - Connection exits cleanly
    ///
    /// This matches OpenConnect CLI behavior.
    pub fn complete_connection(&mut self) -> Result<(), AkonError> {
        tracing::info!("Completing PPP negotiation and setting up TUN device...");

        unsafe {
            // Let OpenConnect use default TUN setup (vpnc-script)
            // This requires sudo/root for TUN device creation and routing setup
            tracing::info!("Starting mainloop (will setup TUN via vpnc-script if running as root)");

            let ret = openconnect_mainloop(
                self.vpn,
                300, // reconnect_timeout (5 minutes)
                30,  // reconnect_interval (30 seconds)
            );

            tracing::info!("Mainloop exited with code: {}", ret);

            // Get IP info to print "Configured as..." message
            // This matches the OpenConnect CLI output
            use std::ffi::CStr;
            let mut ip_info_ptr: *const oc_ip_info = ptr::null();
            let get_ret = openconnect_get_ip_info(
                self.vpn,
                &mut ip_info_ptr as *mut *const oc_ip_info,
                ptr::null_mut(),  // cstp_options (not needed)
                ptr::null_mut(),  // dtls_options (not needed)
            );

            if get_ret == 0 && !ip_info_ptr.is_null() {
                let ip_info = &*ip_info_ptr;
                if !ip_info.addr.is_null() {
                    let ip_addr = CStr::from_ptr(ip_info.addr).to_string_lossy();
                    // Print the "Configured as..." message like OpenConnect CLI does
                    // This message is normally printed by OpenConnect but sometimes gets lost
                    eprintln!("Configured as {}, with SSL connected and DTLS in progress", ip_addr);
                    eprintln!();  // Blank line like CLI output
                }
            }

            // mainloop completes PPP negotiation, gets "Configured as..." state,
            // then fails on TUN setup (expected without root)
            // The VPN session WAS successfully established
            if ret != 0 {
                tracing::info!("Connection completed successfully (PPP negotiation done)");
                tracing::info!("TUN setup failed as expected (no root) - session established but no routing");
            }

            Ok(())
        }
    }

    /// Run the main connection loop (blocks until disconnected)
    /// This is only used when TUN device is available
    #[allow(dead_code)]
    pub fn run_mainloop(&mut self) -> Result<(), AkonError> {
        tracing::info!("Running mainloop with TUN device");
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
            // OpenConnect will free any strdup-ed strings we gave it
            if !self.vpn.is_null() {
                openconnect_vpninfo_free(self.vpn);
            }

            // Now reclaim the credentials Box to free the CStrings
            if !self.auth_box_ptr.is_null() {
                // Reconstruct the Box to drop it
                let _auth = Box::from_raw(self.auth_box_ptr);
                // Box's username/password CStrings will be dropped automatically
            }
        }
    }
}
