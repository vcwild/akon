//! Minimal HTTP/1.1 client over a [`Transport`].
//!
//! The F5 auth/config phase is a small, well-defined sequence of HTTP requests.
//! Rather than depend on a full HTTP stack (which would also pull in its own TLS
//! and connection management), this module implements just enough HTTP/1.1 to
//! drive that exchange over the abstract [`Transport`] seam. In production the
//! transport is TLS-over-TCP; in tests it is the in-memory duplex driven by the
//! fake F5 server actor.

use crate::vpn::f5::F5Error;
use crate::vpn::transport::Transport;

/// A parsed HTTP response.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    /// Status code (e.g. 200).
    pub status: u16,
    /// Header (name, value) pairs, in order; names lowercased.
    pub headers: Vec<(String, String)>,
    /// Response body bytes (exactly Content-Length when present).
    pub body: Vec<u8>,
    /// Bytes read from the transport that lie **beyond** this response's body.
    ///
    /// On a real (coalescing) TLS stream the server can pack the start of the
    /// next protocol phase — notably the first PPP frame after the `/myvpn`
    /// upgrade — into the same read as the HTTP response. Those bytes MUST NOT
    /// be discarded; they are surfaced here so the caller can feed them into the
    /// PPP layer. (The in-memory transport rarely coalesces, but production TLS
    /// routinely does — this is the field that makes the real path correct.)
    pub leftover: Vec<u8>,

    /// True when the server signalled the connection will close after this
    /// response (`Connection: close` or an HTTP/1.0 response). The caller must
    /// then reconnect (a fresh TLS connection) for the next request — real F5
    /// frontends routinely do this between auth-phase requests.
    pub wants_close: bool,
}

impl HttpResponse {
    /// All values of a (case-insensitive) header name, in order. Useful for
    /// `set-cookie`, which may appear multiple times.
    pub fn header_all(&self, name: &str) -> Vec<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .filter(|(k, _)| *k == name)
            .map(|(_, v)| v.as_str())
            .collect()
    }

    /// First value of a (case-insensitive) header name.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.header_all(name).into_iter().next()
    }
}

/// An HTTP request to send.
pub struct HttpRequest<'a> {
    /// Method, e.g. "GET" or "POST".
    pub method: &'a str,
    /// Request target, e.g. "/vdesk/vpn/index.php3?...".
    pub path: &'a str,
    /// Host header value.
    pub host: &'a str,
    /// Extra headers (name, value).
    pub headers: Vec<(String, String)>,
    /// Optional body; when present a Content-Length is added automatically.
    pub body: Option<Vec<u8>>,
}

impl<'a> HttpRequest<'a> {
    /// Create a GET request.
    pub fn get(path: &'a str, host: &'a str) -> Self {
        Self {
            method: "GET",
            path,
            host,
            headers: Vec::new(),
            body: None,
        }
    }

    /// Create a POST request with a url-encoded form body.
    pub fn post_form(path: &'a str, host: &'a str, body: String) -> Self {
        Self {
            method: "POST",
            path,
            host,
            headers: vec![(
                "Content-Type".to_string(),
                "application/x-www-form-urlencoded".to_string(),
            )],
            body: Some(body.into_bytes()),
        }
    }

    /// Add a header.
    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    /// Serialize the request to wire bytes.
    ///
    /// This deliberately mirrors openconnect's `http_common_headers` wire profile
    /// (the AnyConnect-compatible client real F5 appliances expect):
    /// - HTTP/1.1, `Host` (without `:443`), the exact AnyConnect `User-Agent`.
    /// - **No** `Connection`, `Accept`, or `Accept-Encoding` headers (openconnect
    ///   omits all of these; sending them can trip strict F5/WAF frontends).
    /// - On POSTs: an `X-Pad` header padding the body length to a multiple of 64
    ///   (openconnect emits this to avoid leaking password length), then
    ///   `Content-Type` and `Content-Length`.
    pub fn to_bytes(&self) -> Vec<u8> {
        let body = self.body.clone().unwrap_or_default();

        let mut out = String::new();
        out.push_str(&format!("{} {} HTTP/1.1\r\n", self.method, self.path));
        // Host: drop the implicit :443.
        let host_header = match self.host.rsplit_once(':') {
            Some((h, "443")) => h.to_string(),
            _ => self.host.to_string(),
        };
        out.push_str(&format!("Host: {host_header}\r\n"));
        out.push_str(&format!("User-Agent: {USER_AGENT}\r\n"));

        // Caller-supplied headers (Cookie, Content-Type for forms, ...). We do
        // NOT emit Connection/Accept/Accept-Encoding to match openconnect.
        for (k, v) in &self.headers {
            // Content-Type/Content-Length are emitted in the body block below to
            // preserve openconnect's X-Pad → Content-Type → Content-Length order.
            if k.eq_ignore_ascii_case("content-type") || k.eq_ignore_ascii_case("content-length") {
                continue;
            }
            out.push_str(&format!("{k}: {v}\r\n"));
        }

        if self.body.is_some() {
            // X-Pad: pad the body length up to a multiple of 64 (openconnect
            // http.c). The value is that many '0' characters.
            let rlen = body.len();
            let pad = 64 * (1 + rlen / 64) - rlen;
            out.push_str(&format!("X-Pad: {}\r\n", "0".repeat(pad)));
            let content_type = self
                .headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
                .map(|(_, v)| v.clone())
                .unwrap_or_else(|| "application/x-www-form-urlencoded".to_string());
            out.push_str(&format!("Content-Type: {content_type}\r\n"));
            out.push_str(&format!("Content-Length: {rlen}\r\n"));
        }

        out.push_str("\r\n");
        let mut bytes = out.into_bytes();
        bytes.extend_from_slice(&body);
        bytes
    }
}

/// The exact AnyConnect-compatible User-Agent openconnect sends. Real F5
/// appliances frequently key on this; matching it maximizes compatibility.
pub const USER_AGENT: &str = "AnyConnect-compatible OpenConnect VPN Agent v9.12";

/// Send an HTTP request over the transport and read the full response.
///
/// Supports `Content-Length`-delimited bodies (sufficient for the F5 exchange).
/// The response parsing stops once the declared body is read, leaving any
/// trailing bytes (e.g. the start of the PPP stream after `/myvpn`) buffered in
/// the returned [`HttpResponse::body`] only up to Content-Length.
pub async fn send_request<T: Transport + ?Sized>(
    transport: &mut T,
    request: &HttpRequest<'_>,
) -> Result<HttpResponse, F5Error> {
    if debug_enabled() {
        debug_log!(
            "[f5-http] >>> {} {} (host {}){}",
            request.method,
            request.path,
            request.host,
            if request.body.is_some() {
                " [+body]"
            } else {
                ""
            }
        );
    }
    transport
        .send(&request.to_bytes())
        .await
        .map_err(|e| F5Error::MalformedHttp(format!("send failed: {}", e)))?;
    let resp = read_response(transport).await;
    if debug_enabled() {
        match &resp {
            Ok(r) => {
                debug_log!(
                    "[f5-http] <<< {} ({} body bytes, {} leftover){}",
                    r.status,
                    r.body.len(),
                    r.leftover.len(),
                    if r.wants_close {
                        " [Connection: close]"
                    } else {
                        ""
                    }
                );
                for (k, v) in &r.headers {
                    if matches!(
                        k.as_str(),
                        "location" | "connection" | "content-length" | "set-cookie"
                    ) {
                        debug_log!("[f5-http]     {k}: {v}");
                    }
                }
            }
            Err(e) => debug_log!("[f5-http] <<< ERROR {e}"),
        }
    }
    resp
}

/// Whether verbose F5 HTTP debug logging is enabled (`AKON_F5_DEBUG=1`).
pub fn debug_enabled() -> bool {
    std::env::var("AKON_F5_DEBUG").as_deref() == Ok("1")
}

/// A wall-clock timestamp (`HH:MM:SS.mmm`) for prefixing debug log lines, so a
/// soak-test log shows *when* each event (keepalive, drop, packet) happened.
pub fn debug_ts() -> String {
    chrono::Local::now().format("%H:%M:%S%.3f").to_string()
}

/// Print a timestamped debug line to stderr (only meaningful when
/// [`debug_enabled`] is true; callers gate on it). Mirrors `eprintln!` args but
/// prepends `[HH:MM:SS.mmm] `.
macro_rules! debug_log {
    ($($arg:tt)*) => {
        eprintln!("[{}] {}", $crate::vpn::f5::http::debug_ts(), format_args!($($arg)*))
    };
}
pub(crate) use debug_log;

/// Read and parse an HTTP/1.1 response from the transport.
pub async fn read_response<T: Transport + ?Sized>(
    transport: &mut T,
) -> Result<HttpResponse, F5Error> {
    let mut acc: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];

    // Read until we have the full header block (terminated by CRLFCRLF).
    let header_end = loop {
        if let Some(pos) = find_subslice(&acc, b"\r\n\r\n") {
            break pos;
        }
        let n = transport
            .recv(&mut chunk)
            .await
            .map_err(|e| F5Error::MalformedHttp(format!("recv failed: {}", e)))?;
        if n == 0 {
            return Err(F5Error::MalformedHttp(
                "connection closed before headers complete".to_string(),
            ));
        }
        acc.extend_from_slice(&chunk[..n]);
    };

    let header_block = String::from_utf8_lossy(&acc[..header_end]).to_string();
    let (version, status, headers) = parse_head(&header_block)?;

    // Determine body length.
    let content_length = headers
        .iter()
        .find(|(k, _)| k == "content-length")
        .and_then(|(_, v)| v.trim().parse::<usize>().ok());

    // Detect whether the server will close the connection after this response:
    // HTTP/1.0 defaults to close, and any `Connection: close` forces it.
    let connection_hdr = headers
        .iter()
        .find(|(k, _)| k == "connection")
        .map(|(_, v)| v.to_ascii_lowercase())
        .unwrap_or_default();
    let is_http10 = version.starts_with("HTTP/1.0");
    let mut wants_close =
        connection_hdr.contains("close") || (is_http10 && !connection_hdr.contains("keep-alive"));

    let is_chunked = headers
        .iter()
        .any(|(k, v)| k == "transfer-encoding" && v.to_ascii_lowercase().contains("chunked"));

    let body_start = header_end + 4;
    let mut body: Vec<u8> = acc[body_start..].to_vec();

    if let Some(len) = content_length {
        while body.len() < len {
            let n = transport
                .recv(&mut chunk)
                .await
                .map_err(|e| F5Error::MalformedHttp(format!("recv body failed: {}", e)))?;
            if n == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..n]);
        }
    } else if !is_chunked && body_carrying_status(status) {
        // No Content-Length and not chunked: the body runs until the server
        // closes the connection (common for HTTP/1.0-style F5 login pages). Read
        // to EOF and treat the close as the end of the body, not an error.
        loop {
            match transport.recv(&mut chunk).await {
                Ok(0) => {
                    wants_close = true;
                    break;
                }
                Ok(n) => body.extend_from_slice(&chunk[..n]),
                // An unexpected TLS EOF here just means the body ended at close.
                Err(_) => {
                    wants_close = true;
                    break;
                }
            }
        }
    }

    // Split the accumulated post-header bytes into the body (Content-Length) and
    // any leftover bytes belonging to the next protocol phase. Never discard the
    // leftover — on a real TLS stream it carries the first PPP frame.
    let leftover = match content_length {
        Some(len) if body.len() > len => body.split_off(len),
        _ => Vec::new(),
    };

    Ok(HttpResponse {
        status,
        headers,
        body,
        leftover,
        wants_close,
    })
}

/// Whether a status code is allowed to carry a message body (used to decide
/// whether to read-to-EOF when no Content-Length is present). 1xx/204/304 never
/// carry a body.
fn body_carrying_status(status: u16) -> bool {
    !(matches!(status, 204 | 304) || (100..200).contains(&status))
}

/// Parsed HTTP response head: `(version, status, headers)`.
type ParsedHead = (String, u16, Vec<(String, String)>);

/// Parse the status line + headers of an HTTP response head.
fn parse_head(head: &str) -> Result<ParsedHead, F5Error> {
    let mut lines = head.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| F5Error::MalformedHttp("empty response".to_string()))?;

    // "HTTP/1.1 200 OK"
    let mut parts = status_line.split_whitespace();
    let version = parts
        .next()
        .ok_or_else(|| F5Error::MalformedHttp("missing version".to_string()))?
        .to_string();
    let status: u16 = parts
        .next()
        .ok_or_else(|| F5Error::MalformedHttp("missing status code".to_string()))?
        .parse()
        .map_err(|_| F5Error::MalformedHttp("bad status code".to_string()))?;

    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if let Some(idx) = line.find(':') {
            let name = line[..idx].trim().to_ascii_lowercase();
            let value = line[idx + 1..].trim().to_string();
            headers.push((name, value));
        }
    }

    Ok((version, status, headers))
}

/// Find the first index of `needle` within `haystack`.
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vpn::testkit::transport::MemoryTransport;

    #[test]
    fn debug_ts_is_hh_mm_ss_millis() {
        // Soak logs depend on each debug line carrying a wall-clock timestamp.
        let ts = debug_ts();
        // Shape: HH:MM:SS.mmm — 12 chars, colons at 2/5, dot at 8, all else digits.
        assert_eq!(ts.len(), 12, "unexpected timestamp: {ts}");
        let b = ts.as_bytes();
        assert_eq!((b[2], b[5], b[8]), (b':', b':', b'.'), "separators: {ts}");
        assert!(
            ts.chars()
                .enumerate()
                .all(|(i, c)| matches!(i, 2 | 5 | 8) || c.is_ascii_digit()),
            "non-digit in timestamp: {ts}"
        );
    }

    #[test]
    fn request_serializes_get() {
        let req = HttpRequest::get("/", "vpn.example.com");
        let s = String::from_utf8(req.to_bytes()).unwrap();
        assert!(s.starts_with("GET / HTTP/1.1\r\n"));
        assert!(s.contains("Host: vpn.example.com\r\n"));
        assert!(s.ends_with("\r\n\r\n"));
    }

    #[test]
    fn request_serializes_post_with_length() {
        let body = "username=a&password=b";
        let req = HttpRequest::post_form("/login", "h", body.to_string());
        let s = String::from_utf8(req.to_bytes()).unwrap();
        assert!(s.contains("Content-Type: application/x-www-form-urlencoded\r\n"));
        assert!(s.contains(&format!("Content-Length: {}\r\n", body.len())));
        assert!(s.ends_with(body));
    }

    #[tokio::test]
    async fn reads_response_with_body_and_multiple_set_cookie() {
        let (mut client, mut server) = MemoryTransport::pair();
        // Server writes a canned response.
        let resp = "HTTP/1.1 200 OK\r\nSet-Cookie: MRHSession=abc; path=/\r\nSet-Cookie: F5_ST=1z2z3z4z5; path=/\r\nContent-Length: 5\r\n\r\nhello";
        server.send(resp.as_bytes()).await.unwrap();
        let parsed = read_response(&mut client).await.unwrap();
        assert_eq!(parsed.status, 200);
        assert_eq!(parsed.body, b"hello");
        let cookies = parsed.header_all("set-cookie");
        assert_eq!(cookies.len(), 2);
        assert!(cookies[0].contains("MRHSession=abc"));
        assert!(cookies[1].contains("F5_ST="));
    }

    #[tokio::test]
    async fn reads_response_split_across_reads() {
        let (mut client, mut server) = MemoryTransport::pair();
        server
            .send(b"HTTP/1.1 201 Created\r\nX-VPN-client-IP: 10.0.")
            .await
            .unwrap();
        server
            .send(b"0.7\r\nContent-Length: 0\r\n\r\n")
            .await
            .unwrap();
        let parsed = read_response(&mut client).await.unwrap();
        assert_eq!(parsed.status, 201);
        assert_eq!(parsed.header("x-vpn-client-ip"), Some("10.0.0.7"));
    }
}
