//! OpenConnect FFI bindings and safe wrappers
//!
//! This module provides safe Rust wrappers around the OpenConnect C library
//! for establishing VPN connections with TOTP authentication.

use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_void};
use std::ptr;

use crate::auth::totp::generate_totp_default;
use crate::error::{AkonError, VpnError};
use crate::types::OtpSecret;

// Include the generated bindings from build.rs
#[allow(non_camel_case_types)]
#[allow(non_upper_case_globals)]
#[allow(unused)]
#[allow(non_snake_case)]
mod bindings {
    #[allow(non_camel_case_types)]
    #[allow(non_upper_case_globals)]
    #[allow(unused)]
    #[allow(non_snake_case)]
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

pub use bindings::*;

/// Safe wrapper for OpenConnect VPN connection
pub struct OpenConnectConnection {
    vpninfo: *mut openconnect_info,
    otp_secret: Option<Box<OtpSecret>>,
}

impl OpenConnectConnection {
    /// Create a new VPN connection
    pub fn new() -> Result<Self, AkonError> {
        // Initialize SSL (required before creating vpninfo)
        let ret = unsafe { bindings::openconnect_init_ssl() };
        if ret != 0 {
            return Err(AkonError::Vpn(VpnError::ConnectionFailed {
                reason: "Failed to initialize SSL".to_string(),
            }));
        }

        // Create VPN info structure with null callbacks for now
        let vpninfo = unsafe {
            bindings::openconnect_vpninfo_new(
                ptr::null(),     // useragent (null = default)
                None,            // validate_peer_cert
                None,            // write_new_config
                None,            // process_auth_form
                None,            // progress
                ptr::null_mut(), // privdata
            )
        };

        if vpninfo.is_null() {
            return Err(AkonError::Vpn(VpnError::ConnectionFailed {
                reason: "Failed to create VPN info structure".to_string(),
            }));
        }

        Ok(Self {
            vpninfo,
            otp_secret: None,
        })
    }

    /// Set VPN protocol
    pub fn set_protocol(&mut self, protocol: &str) -> Result<(), AkonError> {
        let protocol_c = CString::new(protocol).map_err(|_| {
            AkonError::Vpn(VpnError::ConnectionFailed {
                reason: "Invalid protocol name".to_string(),
            })
        })?;

        let ret = unsafe {
            bindings::openconnect_set_protocol(self.vpninfo, protocol_c.as_ptr())
        };

        if ret != 0 {
            return Err(AkonError::Vpn(VpnError::ConnectionFailed {
                reason: format!("Failed to set protocol: {}", protocol),
            }));
        }

        Ok(())
    }

    /// Set OTP secret for TOTP authentication
    pub fn set_otp_secret(&mut self, secret: OtpSecret) {
        let secret_box = Box::new(secret);
        let secret_ptr = Box::into_raw(secret_box);

        // Set up token callbacks
        unsafe {
            bindings::openconnect_set_token_callbacks(
                self.vpninfo,
                secret_ptr as *mut c_void,
                Some(Self::lock_token_callback),
                Some(Self::unlock_token_callback),
            );
        }

        // Store the box (will be cleaned up in drop)
        self.otp_secret = Some(unsafe { Box::from_raw(secret_ptr) });
    }

    /// Connect to VPN server
    pub fn connect(
        &mut self,
        server: &str,
        _username: &str,
        _password: &str,
        no_dtls: bool,
    ) -> Result<(), AkonError> {
        let server_c = CString::new(server).map_err(|_| {
            AkonError::Vpn(VpnError::ConnectionFailed {
                reason: "Invalid server URL".to_string(),
            })
        })?;

        // Disable DTLS if requested
        if no_dtls {
            let ret = unsafe { bindings::openconnect_disable_dtls(self.vpninfo) };
            if ret != 0 {
                return Err(AkonError::Vpn(VpnError::ConnectionFailed {
                    reason: "Failed to disable DTLS".to_string(),
                }));
            }
        }

        // Parse the server URL
        let ret = unsafe { bindings::openconnect_parse_url(self.vpninfo, server_c.as_ptr()) };
        if ret != 0 {
            return Err(AkonError::Vpn(VpnError::ConnectionFailed {
                reason: format!("Failed to parse URL: {}", server),
            }));
        }

        // Obtain cookie (authenticate)
        let ret = unsafe { bindings::openconnect_obtain_cookie(self.vpninfo) };
        if ret != 0 {
            return Err(AkonError::Vpn(VpnError::AuthenticationFailed));
        }

        // Make CSTP connection
        let ret = unsafe { bindings::openconnect_make_cstp_connection(self.vpninfo) };
        if ret != 0 {
            return Err(AkonError::Vpn(VpnError::ConnectionFailed {
                reason: "Failed to establish CSTP connection".to_string(),
            }));
        }

        // Setup TUN device
        let ret = unsafe {
            bindings::openconnect_setup_tun_device(
                self.vpninfo,
                ptr::null(), // vpnc_script (null = default)
                ptr::null(), // ifname (null = auto)
            )
        };
        if ret != 0 {
            return Err(AkonError::Vpn(VpnError::ConnectionFailed {
                reason: "Failed to setup TUN device".to_string(),
            }));
        }

        // Setup DTLS (optional, but recommended) - only if not disabled
        if !no_dtls {
            let _ = unsafe { bindings::openconnect_setup_dtls(self.vpninfo, 60) };
        }

        Ok(())
    }

    /// Run the main connection loop
    pub fn run_mainloop(&mut self) -> Result<(), AkonError> {
        // Run main loop with reconnection settings
        let ret = unsafe {
            bindings::openconnect_mainloop(
                self.vpninfo,
                300, // reconnect_timeout (5 minutes)
                30,  // reconnect_interval (30 seconds)
            )
        };

        if ret != 0 {
            return Err(AkonError::Vpn(VpnError::ConnectionFailed {
                reason: "Main loop exited with error".to_string(),
            }));
        }

        Ok(())
    }

    /// Disconnect from VPN
    pub fn disconnect(&mut self) -> Result<(), AkonError> {
        // OpenConnect doesn't have a direct disconnect function
        // The main loop will exit when cancelled
        Ok(())
    }

    /// Token lock callback (called when OpenConnect needs to prepare for token use)
    unsafe extern "C" fn lock_token_callback(_tokdata: *mut c_void) -> c_int {
        // For TOTP, we don't need to "lock" anything special
        // This callback is mainly for hardware tokens
        0 // Success
    }

    /// Token unlock callback (called when OpenConnect needs a new token)
    unsafe extern "C" fn unlock_token_callback(
        tokdata: *mut c_void,
        new_tok: *const c_char,
    ) -> c_int {
        if tokdata.is_null() {
            return -1; // Error
        }

        // Get the OTP secret from context
        let secret = &*(tokdata as *const OtpSecret);

        // Generate TOTP token
        match generate_totp_default(secret.expose()) {
            Ok(token) => {
                // Copy token to the provided buffer
                let token_str = token.expose();
                let token_cstr = match CString::new(token_str) {
                    Ok(cstr) => cstr,
                    Err(_) => return -1,
                };

                // Copy to OpenConnect's buffer (assuming it has enough space)
                if !new_tok.is_null() {
                    std::ptr::copy_nonoverlapping(
                        token_cstr.as_ptr(),
                        new_tok as *mut c_char,
                        token_cstr.as_bytes().len() + 1, // Include null terminator
                    );
                }

                0 // Success
            }
            Err(_) => -1, // Error
        }
    }
}

impl Drop for OpenConnectConnection {
    fn drop(&mut self) {
        if !self.vpninfo.is_null() {
            unsafe {
                bindings::openconnect_vpninfo_free(self.vpninfo);
            }
        }
    }
}
