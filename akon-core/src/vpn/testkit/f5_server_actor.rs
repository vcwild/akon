//! Fake F5 BIG-IP server actor — the **ground-truth oracle** for the native F5
//! backend.
//!
//! This actor speaks the real F5 wire protocol over a [`MemoryTransport`]:
//!
//! 1. Serves the HTTP auth form, sets `MRHSession` + `F5_ST` on credential POST.
//! 2. Serves the profile and options XML.
//! 3. Accepts the `GET /myvpn?...` tunnel upgrade with `200` + `X-VPN-client-IP`.
//! 4. Acts as the PPP peer: ACKs the client's LCP Config-Request and NAKs its
//!    IPCP request with a concrete assigned IP + DNS, driving the negotiator to
//!    "network up" — using the *real* [`crate::vpn::f5::framing`] and
//!    [`crate::vpn::f5::ppp`] code so the test exercises the genuine codec.
//!
//! It performs no real I/O and requires no root or network. Drive it by
//! spawning [`F5ServerActor::run`] on a tokio task connected to the backend's
//! transport peer.

use crate::vpn::f5::framing::{f5_decap, f5_encap};
use crate::vpn::f5::ppp::{
    self, build_ncp_frame, parse_ppp_frame, NcpOption, NcpPacket, CONFACK, CONFNAK, CONFREQ,
    PPP_IP6CP, PPP_IPCP, PPP_LCP,
};
use crate::vpn::transport::Transport;

/// Script controlling how the fake server behaves for a session.
#[derive(Debug, Clone)]
pub struct F5ServerScript {
    /// Whether credentials should be accepted (sets both cookies) or rejected.
    pub accept_auth: bool,
    /// HTTP status returned for the `/myvpn` tunnel upgrade (200/201 = success).
    pub tunnel_status: u16,
    /// IPv4 address assigned to the client (dotted), reported via header and
    /// offered in IPCP NAK.
    pub assigned_ip: [u8; 4],
    /// DNS server offered in IPCP NAK.
    pub dns: [u8; 4],
    /// Whether HDLC framing is advertised in the options XML.
    pub hdlc: bool,
    /// **Realistic mode**: emulate a real F5 frontend that closes the connection
    /// after every HTTP response (`Connection: close`), redirects the initial
    /// `GET /` to the logon page, and sets an intermediate session cookie before
    /// the credential POST. Requires a *listener-based* harness (a real local
    /// TLS server) so the client can reconnect per request.
    pub realistic: bool,
}

impl Default for F5ServerScript {
    fn default() -> Self {
        Self {
            accept_auth: true,
            tunnel_status: 200,
            assigned_ip: [10, 20, 30, 40],
            dns: [8, 8, 8, 8],
            hdlc: false,
            realistic: false,
        }
    }
}

impl F5ServerScript {
    /// A script that rejects authentication.
    pub fn auth_failure() -> Self {
        Self {
            accept_auth: false,
            ..Self::default()
        }
    }

    /// A script that rejects the tunnel upgrade with the given status.
    pub fn tunnel_rejected(status: u16) -> Self {
        Self {
            tunnel_status: status,
            ..Self::default()
        }
    }

    /// A script emulating real F5 frontend behavior (connection-close, initial
    /// redirect, intermediate cookies). Use with a listener-based harness.
    pub fn realistic() -> Self {
        Self {
            realistic: true,
            ..Self::default()
        }
    }
}

/// The fake F5 server actor.
pub struct F5ServerActor {
    script: F5ServerScript,
}

impl F5ServerActor {
    /// Create a server actor with the given script.
    pub fn new(script: F5ServerScript) -> Self {
        Self { script }
    }

    /// Run the full server session over `transport` until the tunnel is up and
    /// PPP has reached the network phase (or auth/tunnel fails). Returns when
    /// the exchange completes or the transport closes.
    pub async fn run<T: Transport + ?Sized>(&self, transport: &mut T) {
        // --- HTTP phase: handle requests until the /myvpn upgrade ---
        loop {
            let request = match read_http_request(transport).await {
                Some(r) => r,
                None => return, // transport closed
            };

            let (method, path) = request_line(&request);

            if path.starts_with("/myvpn") {
                self.handle_tunnel_upgrade(transport).await;
                if self.script.tunnel_status == 200 || self.script.tunnel_status == 201 {
                    break; // proceed to PPP
                } else {
                    return; // rejected; no tunnel
                }
            } else if method == "POST" {
                // Credential submission.
                self.handle_auth_post(transport).await;
            } else if path.contains("index.php3") {
                self.respond(transport, 200, &[], profile_xml().as_bytes())
                    .await;
            } else if path.contains("connect.php3") {
                self.respond(transport, 200, &[], self.options_xml().as_bytes())
                    .await;
            } else {
                // Initial GET "/" -> login form (no cookies yet).
                self.respond(transport, 200, &[], login_form_html().as_bytes())
                    .await;
            }
        }

        // --- PPP phase: act as the peer ---
        self.run_ppp_peer(transport).await;
    }

    /// Serve a **single** connection in realistic mode: read one HTTP request,
    /// respond with `Connection: close`, and close (return `false`). For the
    /// `/myvpn` request, instead keep the connection open, run the PPP peer, and
    /// return `true` to signal the session is complete.
    ///
    /// A listener-based harness calls this once per accepted TLS connection, so
    /// the reconnecting client experiences the same connection-close behavior a
    /// real F5 frontend exhibits.
    ///
    /// Returns `true` when the tunnel session completed (no more connections
    /// expected), `false` to keep accepting.
    pub async fn serve_one_connection<T: Transport + ?Sized>(&self, transport: &mut T) -> bool {
        let request = match read_http_request(transport).await {
            Some(r) => r,
            None => return false,
        };
        let (method, path) = request_line(&request);
        let has_cookie = request_has_cookie(&request);

        if path.starts_with("/myvpn") {
            self.handle_tunnel_upgrade(transport).await;
            if self.script.tunnel_status == 200 || self.script.tunnel_status == 201 {
                self.run_ppp_peer(transport).await;
            }
            return true; // session done
        }

        if method == "POST" {
            // Credential POST: succeed (set both session cookies) or re-serve form.
            self.handle_auth_post_close(transport).await;
        } else if path.contains("index.php3") {
            self.respond_close(transport, 200, &[], profile_xml().as_bytes())
                .await;
        } else if path.contains("connect.php3") {
            self.respond_close(transport, 200, &[], self.options_xml().as_bytes())
                .await;
        } else if !has_cookie {
            // Initial GET with no session cookie: redirect to the logon page and
            // set an intermediate (insufficient) MRHSession cookie — exactly the
            // kind of behavior that broke the naive client.
            let headers = [
                ("Location", "/my.logon.php3?outform=xml"),
                ("Set-Cookie", "MRHSession=preauth123; path=/; secure"),
            ];
            self.respond_close(transport, 302, &headers, b"").await;
        } else {
            // Logon page (we have the preauth cookie now): serve the auth form.
            self.respond_close(transport, 200, &[], login_form_html().as_bytes())
                .await;
        }
        false // connection closed; expect a reconnect
    }

    async fn handle_auth_post_close<T: Transport + ?Sized>(&self, transport: &mut T) {
        if self.script.accept_auth {
            let cookies = [
                ("Set-Cookie", "MRHSession=fakesession; path=/; secure"),
                ("Set-Cookie", "F5_ST=1z1z1z1700000000z3600; path=/"),
            ];
            self.respond_close(transport, 200, &cookies, b"<html>ok</html>")
                .await;
        } else {
            self.respond_close(transport, 200, &[], login_form_html().as_bytes())
                .await;
        }
    }

    async fn handle_auth_post<T: Transport + ?Sized>(&self, transport: &mut T) {
        if self.script.accept_auth {
            let cookies = [
                ("Set-Cookie", "MRHSession=fakesession; path=/; secure"),
                ("Set-Cookie", "F5_ST=1z1z1z1700000000z3600; path=/"),
            ];
            self.respond(transport, 200, &cookies, b"<html>ok</html>")
                .await;
        } else {
            // No cookies set -> client never authenticates.
            self.respond(transport, 200, &[], login_form_html().as_bytes())
                .await;
        }
    }

    async fn handle_tunnel_upgrade<T: Transport + ?Sized>(&self, transport: &mut T) {
        let ip = self.script.assigned_ip;
        let ip_str = format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]);
        let headers: [(&str, &str); 1] = [("X-VPN-client-IP", &ip_str)];
        self.respond(transport, self.script.tunnel_status, &headers, b"")
            .await;
    }

    /// PPP peer loop: read F5-encapsulated PPP frames, ACK LCP, NAK then ACK
    /// IPCP, using the real framing/ppp codec.
    async fn run_ppp_peer<T: Transport + ?Sized>(&self, transport: &mut T) {
        let mut buf = [0u8; 4096];
        let mut naked_ipcp = false;

        loop {
            let n = match transport.recv(&mut buf).await {
                Ok(0) | Err(_) => return,
                Ok(n) => n,
            };

            let frames = match f5_decap(&buf[..n]) {
                Ok(f) => f,
                Err(_) => continue,
            };

            for ppp_frame in frames {
                // Data-plane: if this is an IP packet, echo it back as a proper
                // reply — SWAP source/destination so the reply is addressed to
                // the client's tunnel IP (otherwise a verbatim echo would arrive
                // with the wrong destination and the client's kernel would not
                // deliver it locally). This makes the round-trip a faithful test
                // of the data plane AND local delivery.
                if is_ppp_ip_frame(&ppp_frame) {
                    let reply = swap_ip_src_dst(&ppp_frame);
                    let wire = f5_encap(&reply);
                    if transport.send(&wire).await.is_err() {
                        return;
                    }
                    continue;
                }

                let pkt = match parse_ppp_frame(&ppp_frame) {
                    Ok(p) => p,
                    Err(_) => continue,
                };

                // An LCP Terminate-Request means the client is tearing down.
                if pkt.proto == PPP_LCP && pkt.code == ppp::TERMREQ {
                    return;
                }

                let replies = self.respond_ppp(&pkt, &mut naked_ipcp);
                for reply in replies {
                    let wire = f5_encap(&reply);
                    if transport.send(&wire).await.is_err() {
                        return;
                    }
                }
            }
        }
    }

    /// Produce PPP replies to a client packet (peer behavior).
    fn respond_ppp(&self, pkt: &NcpPacket, naked_ipcp: &mut bool) -> Vec<Vec<u8>> {
        let mut out = Vec::new();

        match (pkt.proto, pkt.code) {
            // Client's LCP Config-Request -> ACK it; also send OUR Config-Request
            // (which the client will ACK) so LCP fully opens.
            (PPP_LCP, CONFREQ) => {
                let ack = NcpPacket {
                    proto: PPP_LCP,
                    code: CONFACK,
                    id: pkt.id,
                    options: pkt.options.clone(),
                };
                out.push(build_ncp_frame(&ack));

                // Our own LCP Config-Request. In realistic mode, include an
                // unknown/proprietary option (tag 0xDF) like a real F5 server,
                // so the client must ACK options it doesn't recognize for LCP to
                // open (the exact path that previously broke parsing).
                // Advertise MRU 1411 (0x0583) like the real appliance, so the
                // client's MTU-from-MRU derivation is exercised end-to-end.
                let mut our_req = ppp::lcp_config_request(200, 0x99887766, 1411);
                if self.script.realistic {
                    our_req.options.push(NcpOption {
                        tag: 0xdf,
                        data: vec![0x11, 0x22],
                    });
                }
                out.push(build_ncp_frame(&our_req));

                // In realistic mode, also run IP6CP in parallel like a real F5,
                // so the client's IP6CP-reject path is exercised end-to-end.
                if self.script.realistic {
                    let ip6cp_req = NcpPacket {
                        proto: PPP_IP6CP,
                        code: CONFREQ,
                        id: 210,
                        options: vec![NcpOption {
                            tag: 1, // interface identifier
                            data: vec![0x28, 0xf8, 0xd2, 0x5d, 0x63, 0x37, 0xb3, 0xc5],
                        }],
                    };
                    out.push(build_ncp_frame(&ip6cp_req));
                }
            }
            // Client ACKs our LCP request -> nothing further for LCP.
            (PPP_LCP, CONFACK) => {}
            // Client's IPCP Config-Request.
            (PPP_IPCP, CONFREQ) => {
                // Whether the client's request already carries the assigned IP
                // AND (in realistic mode) the offered DNS — i.e. it adopted our
                // NAK. Only then do we ACK; otherwise we NAK again. This is what
                // makes the test catch a client that fails to echo NAKed DNS
                // (the real-appliance bug).
                let ip_ok = pkt
                    .option(ppp::IPCP_IPADDR)
                    .map(|o| o.data == self.script.assigned_ip)
                    .unwrap_or(false);
                let dns_ok = !self.script.realistic
                    || pkt
                        .option(ppp::IPCP_DNS1)
                        .map(|o| o.data == self.script.dns)
                        .unwrap_or(false);

                if !*naked_ipcp || !ip_ok || !dns_ok {
                    // NAK with the assigned IP + DNS(1/2) to force adoption.
                    *naked_ipcp = true;
                    let mut options = vec![NcpOption {
                        tag: ppp::IPCP_IPADDR,
                        data: self.script.assigned_ip.to_vec(),
                    }];
                    options.push(NcpOption {
                        tag: ppp::IPCP_DNS1,
                        data: self.script.dns.to_vec(),
                    });
                    if self.script.realistic {
                        // A secondary DNS too, exercising DNS2 adoption.
                        options.push(NcpOption {
                            tag: ppp::IPCP_DNS2,
                            data: vec![
                                self.script.dns[0],
                                self.script.dns[1],
                                self.script.dns[2],
                                self.script.dns[3].wrapping_add(1),
                            ],
                        });
                    }
                    let nak = NcpPacket {
                        proto: PPP_IPCP,
                        code: CONFNAK,
                        id: pkt.id,
                        options,
                    };
                    out.push(build_ncp_frame(&nak));
                } else {
                    // Request carries the assigned IP + DNS -> ACK.
                    let ack = NcpPacket {
                        proto: PPP_IPCP,
                        code: CONFACK,
                        id: pkt.id,
                        options: pkt.options.clone(),
                    };
                    out.push(build_ncp_frame(&ack));

                    // Also send OUR IPCP Config-Request (with the server's
                    // gateway address) so the client ACKs it and both
                    // directions of IPCP complete, bringing the network up.
                    let our_req = NcpPacket {
                        proto: PPP_IPCP,
                        code: CONFREQ,
                        id: 201,
                        options: vec![NcpOption {
                            tag: ppp::IPCP_IPADDR,
                            // Server-side gateway address (last octet .1).
                            data: vec![
                                self.script.assigned_ip[0],
                                self.script.assigned_ip[1],
                                self.script.assigned_ip[2],
                                1,
                            ],
                        }],
                    };
                    out.push(build_ncp_frame(&our_req));
                }
            }
            // Client ACKs our IPCP request — both directions complete.
            (PPP_IPCP, CONFACK) => {}
            _ => {}
        }

        out
    }

    async fn respond<T: Transport + ?Sized>(
        &self,
        transport: &mut T,
        status: u16,
        headers: &[(&str, &str)],
        body: &[u8],
    ) {
        let reason = match status {
            200 => "OK",
            201 => "Created",
            403 => "Forbidden",
            502 => "Bad Gateway",
            504 => "Gateway Timeout",
            _ => "Status",
        };
        let mut head = format!("HTTP/1.1 {} {}\r\n", status, reason);
        for (k, v) in headers {
            head.push_str(&format!("{}: {}\r\n", k, v));
        }
        head.push_str(&format!("Content-Length: {}\r\n", body.len()));
        head.push_str("\r\n");
        let mut bytes = head.into_bytes();
        bytes.extend_from_slice(body);
        let _ = transport.send(&bytes).await;
    }

    /// Like [`respond`](Self::respond) but adds `Connection: close` (the realistic
    /// F5-frontend behavior). The caller closes the transport afterwards.
    async fn respond_close<T: Transport + ?Sized>(
        &self,
        transport: &mut T,
        status: u16,
        headers: &[(&str, &str)],
        body: &[u8],
    ) {
        let reason = match status {
            200 => "OK",
            201 => "Created",
            302 => "Found",
            403 => "Forbidden",
            502 => "Bad Gateway",
            504 => "Gateway Timeout",
            _ => "Status",
        };
        let mut head = format!("HTTP/1.1 {} {}\r\n", status, reason);
        for (k, v) in headers {
            head.push_str(&format!("{}: {}\r\n", k, v));
        }
        head.push_str(&format!("Content-Length: {}\r\n", body.len()));
        head.push_str("Connection: close\r\n");
        head.push_str("\r\n");
        let mut bytes = head.into_bytes();
        bytes.extend_from_slice(body);
        let _ = transport.send(&bytes).await;
        let _ = transport.close().await;
    }

    fn options_xml(&self) -> String {
        let hdlc = if self.script.hdlc { "yes" } else { "no" };
        format!(
            "<favorite><object>\
<Session_ID>FAKE_SID</Session_ID>\
<ur_Z>FAKE_URZ</ur_Z>\
<IPV4_0>1</IPV4_0>\
<IPV6_0>0</IPV6_0>\
<hdlc_framing>{}</hdlc_framing>\
<DNS0>{}.{}.{}.{}</DNS0>\
<UseDefaultGateway0>1</UseDefaultGateway0>\
</object></favorite>",
            hdlc, self.script.dns[0], self.script.dns[1], self.script.dns[2], self.script.dns[3]
        )
    }
}

/// Read a single HTTP request (head + any Content-Length body) from transport.
/// Returns the full request bytes, or None if the transport closed.
async fn read_http_request<T: Transport + ?Sized>(transport: &mut T) -> Option<Vec<u8>> {
    let mut acc: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 2048];

    let header_end = loop {
        if let Some(pos) = find_subslice(&acc, b"\r\n\r\n") {
            break pos;
        }
        match transport.recv(&mut chunk).await {
            Ok(0) | Err(_) => return None,
            Ok(n) => acc.extend_from_slice(&chunk[..n]),
        }
    };

    let head = String::from_utf8_lossy(&acc[..header_end]).to_string();
    let content_length = head
        .lines()
        .find_map(|l| {
            let lower = l.to_ascii_lowercase();
            lower
                .strip_prefix("content-length:")
                .map(|v| v.trim().parse::<usize>().unwrap_or(0))
        })
        .unwrap_or(0);

    let body_start = header_end + 4;
    while acc.len() < body_start + content_length {
        match transport.recv(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => acc.extend_from_slice(&chunk[..n]),
        }
    }

    Some(acc)
}

fn request_line(request: &[u8]) -> (String, String) {
    let text = String::from_utf8_lossy(request);
    let first = text.lines().next().unwrap_or("");
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();
    (method, path)
}

/// Whether the request carries a `Cookie:` header (case-insensitive).
fn request_has_cookie(request: &[u8]) -> bool {
    let text = String::from_utf8_lossy(request);
    text.lines()
        .take_while(|l| !l.is_empty())
        .any(|l| l.to_ascii_lowercase().starts_with("cookie:"))
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Swap the IPv4 source and destination of the IP packet inside a PPP frame
/// (`FF 03 00 21 <ip>`), recomputing the IPv4 header checksum, so an echo reply
/// is addressed back to the original sender (the client's tunnel IP). Returns
/// the frame unchanged if it isn't a parseable IPv4 packet.
///
/// Because the UDP/TCP checksum covers a pseudo-header that includes the IP
/// src/dst, swapping the addresses invalidates the transport checksum and the
/// receiver's kernel would silently drop the datagram. For UDP we set the
/// checksum to 0 ("no checksum", valid for IPv4 UDP) so the echo is accepted;
/// other protocols are left as-is (the round-trip test uses UDP).
fn swap_ip_src_dst(frame: &[u8]) -> Vec<u8> {
    let mut out = frame.to_vec();
    let mut p = 0usize;
    if out.len() >= 2 && out[0] == 0xff && out[1] == 0x03 {
        p += 2;
    }
    if p >= out.len() {
        return out;
    }
    p += if out[p] & 0x01 == 1 { 1 } else { 2 };
    let ip = p;
    if out.len() < ip + 20 || (out[ip] >> 4) != 4 {
        return out;
    }
    for k in 0..4 {
        out.swap(ip + 12 + k, ip + 16 + k);
    }
    let ihl = ((out[ip] & 0x0f) as usize) * 4;
    if out.len() >= ip + ihl && ihl >= 20 {
        // Recompute the IPv4 header checksum.
        out[ip + 10] = 0;
        out[ip + 11] = 0;
        let mut sum: u32 = 0;
        let mut i = ip;
        while i + 1 < ip + ihl {
            sum += u16::from_be_bytes([out[i], out[i + 1]]) as u32;
            i += 2;
        }
        while sum >> 16 != 0 {
            sum = (sum & 0xffff) + (sum >> 16);
        }
        let csum = !(sum as u16);
        out[ip + 10..ip + 12].copy_from_slice(&csum.to_be_bytes());

        // For UDP (proto 17): also swap the source/destination PORTS so the echo
        // is addressed back to the sender's socket (src_port<->dst_port), and
        // zero the UDP checksum ("no checksum", valid for IPv4 UDP) since the
        // pseudo-header it covered now has swapped addresses. UDP header layout
        // from `ip+ihl`: sport(2) dport(2) len(2) csum(2).
        let proto = out[ip + 9];
        if proto == 17 && out.len() >= ip + ihl + 8 {
            out.swap(ip + ihl, ip + ihl + 2); // sport[0] <-> dport[0]
            out.swap(ip + ihl + 1, ip + ihl + 3); // sport[1] <-> dport[1]
            out[ip + ihl + 6] = 0;
            out[ip + ihl + 7] = 0;
        }
    }
    out
}

/// True if a PPP frame carries an IP (0x21) or IPv6 (0x57) payload, i.e. it is a
/// data-plane packet rather than an NCP control frame.
fn is_ppp_ip_frame(frame: &[u8]) -> bool {
    let rest = if frame.len() >= 2 && frame[0] == 0xff && frame[1] == 0x03 {
        &frame[2..]
    } else {
        frame
    };
    if rest.is_empty() {
        return false;
    }
    let proto = if rest[0] & 0x01 == 1 {
        rest[0] as u16
    } else if rest.len() >= 2 {
        u16::from_be_bytes([rest[0], rest[1]])
    } else {
        return false;
    };
    matches!(proto, 0x0021 | 0x0057)
}

fn login_form_html() -> String {
    "<html><body><form id=\"auth_form\" method=\"post\" action=\"/my.policy\">\
<input type=\"text\" name=\"username\"/>\
<input type=\"password\" name=\"password\"/>\
</form></body></html>"
        .to_string()
}

fn profile_xml() -> String {
    "<favorites type=\"VPN\" limited=\"YES\">\
<favorite id=\"/Common/akon_vpn\"><params>resourcename=/Common/akon_vpn</params></favorite>\
</favorites>"
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vpn::testkit::transport::MemoryTransport;

    #[test]
    fn script_defaults_are_successful() {
        let s = F5ServerScript::default();
        assert!(s.accept_auth);
        assert_eq!(s.tunnel_status, 200);
    }

    #[tokio::test]
    async fn serves_login_form_on_root_get() {
        let (mut client, mut server) = MemoryTransport::pair();
        let actor = F5ServerActor::new(F5ServerScript::default());
        let handle = tokio::spawn(async move {
            actor.run(&mut server).await;
        });

        // Client sends GET / and should receive the auth_form.
        use crate::vpn::f5::http::{send_request, HttpRequest};
        let resp = send_request(&mut client, &HttpRequest::get("/", "h"))
            .await
            .unwrap();
        assert_eq!(resp.status, 200);
        assert!(String::from_utf8_lossy(&resp.body).contains("auth_form"));

        drop(client); // closes transport, ends the actor
        let _ = handle.await;
    }
}
