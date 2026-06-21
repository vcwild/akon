//! F5 HTTP auth logic for the native backend (pure Rust).
//!
//! F5 BIG-IP SSL VPN authenticates over HTTPS. The login form (`id="auth_form"`)
//! POSTs `username`/`password` as `application/x-www-form-urlencoded`. Auth
//! success is signalled not by a single cookie but by the **combination** of two
//! `Set-Cookie` values: `MRHSession` (often re-set repeatedly before auth
//! completes) and `F5_ST` (the "session timeout" cookie). Only when both are
//! present is the session established, and the subsequent requests carry the
//! combined `Cookie: MRHSession=<v>; F5_ST=<v>` header.
//!
//! Protocol ground truth: openconnect `f5.c` (`check_cookie_success`) — both
//! cookies required, combined header formatted as
//! `"MRHSession=%s; F5_ST=%s"`.

use std::collections::HashMap;

/// The F5 session cookie. Set repeatedly during the exchange; not sufficient on
/// its own to indicate auth success.
pub const COOKIE_MRHSESSION: &str = "MRHSession";

/// The F5 "session timeout" cookie. Its presence (together with [`COOKIE_MRHSESSION`])
/// indicates that authentication has completed.
pub const COOKIE_F5_ST: &str = "F5_ST";

/// Accumulates `Set-Cookie` values seen during the auth exchange and reports
/// when the F5 session is established (both [`COOKIE_MRHSESSION`] and
/// [`COOKIE_F5_ST`] present).
#[derive(Debug, Default, Clone)]
pub struct F5CookieJar {
    cookies: HashMap<String, String>,
}

impl F5CookieJar {
    /// Create an empty cookie jar.
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a raw `Set-Cookie` header value (e.g. `"MRHSession=abc; path=/; secure"`).
    ///
    /// Only the first `name=value` pair is significant; cookie attributes
    /// (`path`, `secure`, `HttpOnly`, ...) after the first `;` are ignored. An
    /// empty value clears the cookie (servers delete cookies by re-setting an
    /// empty value); anything else stores/overwrites it.
    pub fn ingest_set_cookie(&mut self, header_value: &str) {
        let trimmed = header_value.trim_start();
        let pair = trimmed.split(';').next().unwrap_or("").trim();
        let Some((name, value)) = pair.split_once('=') else {
            return;
        };
        let name = name.trim();
        let value = value.trim();
        if name.is_empty() {
            return;
        }
        if value.is_empty() {
            self.cookies.remove(name);
        } else {
            self.cookies.insert(name.to_string(), value.to_string());
        }
    }

    /// Get the stored value of cookie `name`, if present.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.cookies.get(name).map(String::as_str)
    }

    /// True iff both `MRHSession` and `F5_ST` are present (auth success).
    pub fn is_authenticated(&self) -> bool {
        self.cookies.contains_key(COOKIE_MRHSESSION) && self.cookies.contains_key(COOKIE_F5_ST)
    }

    /// The combined `Cookie` header value `"MRHSession=..; F5_ST=.."`, or `None`
    /// if not yet authenticated.
    pub fn cookie_header(&self) -> Option<String> {
        let session = self.cookies.get(COOKIE_MRHSESSION)?;
        let f5_st = self.cookies.get(COOKIE_F5_ST)?;
        Some(format!(
            "{COOKIE_MRHSESSION}={session}; {COOKIE_F5_ST}={f5_st}"
        ))
    }

    /// A `Cookie` header echoing **all** currently-held cookies (joined by
    /// `"; "`), or `None` if the jar is empty.
    ///
    /// openconnect re-sends every cookie on every request during the auth/redirect
    /// chain; the F5 frontend often sets intermediate session cookies (e.g. a
    /// `LastMRH_Session`, `MRHSession`, policy cookies) that must be echoed back
    /// for the next step to succeed. This returns them all, sorted for
    /// determinism.
    pub fn cookie_header_all(&self) -> Option<String> {
        if self.cookies.is_empty() {
            return None;
        }
        let mut pairs: Vec<(&String, &String)> = self.cookies.iter().collect();
        pairs.sort_by(|a, b| a.0.cmp(b.0));
        Some(
            pairs
                .into_iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("; "),
        )
    }
}

/// A field of an F5 login form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormField {
    /// The `name` attribute.
    pub name: String,
    /// Field kind, lowercased (`text`, `password`, `hidden`, ...).
    pub kind: String,
    /// Any prefilled `value` attribute.
    pub value: String,
}

impl FormField {
    /// Whether this is a password field (the slot for PIN+OTP).
    pub fn is_password(&self) -> bool {
        self.kind == "password"
    }

    /// Whether this is a free-text field that should receive the username
    /// (a text/username/email input).
    pub fn is_username(&self) -> bool {
        matches!(self.kind.as_str(), "text" | "username" | "email")
    }
}

/// A parsed F5 HTML login form (`<form id="auth_form" method="post" action="...">`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct F5AuthForm {
    /// The form `id` attribute (the first form must be `auth_form`).
    pub id: String,
    /// The form `action` (becomes the next POST target). Empty = post to same URL.
    pub action: String,
    /// The parsed input fields, in document order.
    pub fields: Vec<FormField>,
}

impl F5AuthForm {
    /// Parse the first `<form>...</form>` from an HTML document.
    ///
    /// Returns `None` if no form is found. Tolerant of attribute ordering,
    /// single/double quotes, and self-closing `<input/>` tags. This is a
    /// purpose-built scanner for the flat F5 login page, not a general HTML
    /// parser (mirroring the dependency-light approach used for the F5 XML).
    pub fn parse(html: &str) -> Option<F5AuthForm> {
        let lower = html.to_ascii_lowercase();
        let form_start = lower.find("<form")?;
        let after_form_tag = form_start + lower[form_start..].find('>')? + 1;
        let form_open_tag = &html[form_start..after_form_tag];

        let id = tag_attr(form_open_tag, "id").unwrap_or_default();
        let action = tag_attr(form_open_tag, "action").unwrap_or_default();

        // Body of the form up to </form> (or end of document).
        let form_end = lower[after_form_tag..]
            .find("</form>")
            .map(|i| after_form_tag + i)
            .unwrap_or(html.len());
        let body = &html[after_form_tag..form_end];

        let mut fields = Vec::new();
        let lower_body = body.to_ascii_lowercase();
        let mut cursor = 0;
        while let Some(rel) = lower_body[cursor..].find("<input") {
            let start = cursor + rel;
            let end = body[start..]
                .find('>')
                .map(|i| start + i + 1)
                .unwrap_or(body.len());
            let tag = &body[start..end];
            let name = tag_attr(tag, "name").unwrap_or_default();
            if !name.is_empty() {
                fields.push(FormField {
                    name,
                    kind: tag_attr(tag, "type").unwrap_or_else(|| "text".to_string()),
                    value: tag_attr(tag, "value").unwrap_or_default(),
                });
            }
            cursor = end;
        }

        Some(F5AuthForm { id, action, fields })
    }

    /// Build the urlencoded POST body for this form, filling the username and
    /// password slots and preserving all other fields (including hidden ones)
    /// with their existing values.
    ///
    /// `password` is akon's pre-composed PIN+OTP string. For a single-step F5
    /// login this is the complete OTP-inclusive credential.
    pub fn build_submission(&self, username: &str, password: &str) -> String {
        let mut parts: Vec<String> = Vec::new();
        for field in &self.fields {
            let value = if field.is_password() {
                password.to_string()
            } else if field.is_username() {
                username.to_string()
            } else {
                field.value.clone()
            };
            parts.push(format!(
                "{}={}",
                percent_encode(&field.name),
                percent_encode(&value)
            ));
        }
        // Fallback to the canonical fields if the form had no parsed inputs.
        if parts.is_empty() {
            return build_login_body(username, password);
        }
        parts.join("&")
    }
}

/// Extract an attribute value from a tag string (`<tag a="x" b='y'>`), tolerant
/// of single/double quotes and surrounding whitespace. Case-insensitive name.
fn tag_attr(tag: &str, attr: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let needle = format!("{}=", attr);
    let mut search = 0;
    while let Some(rel) = lower[search..].find(&needle) {
        let at = search + rel;
        // Ensure the char before the attr name is a word boundary (space, quote,
        // or tag start) to avoid matching substrings like "xid=".
        let prev_ok = at == 0
            || lower.as_bytes()[at - 1].is_ascii_whitespace()
            || lower.as_bytes()[at - 1] == b'<';
        let val_start = at + needle.len();
        if !prev_ok || val_start >= tag.len() {
            search = at + needle.len();
            continue;
        }
        let bytes = tag.as_bytes();
        let (quote, content_start) = match bytes[val_start] {
            b'"' => (Some(b'"'), val_start + 1),
            b'\'' => (Some(b'\''), val_start + 1),
            _ => (None, val_start),
        };
        let content = &tag[content_start..];
        let end = match quote {
            Some(q) => content.find(q as char).unwrap_or(content.len()),
            None => content
                .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
                .unwrap_or(content.len()),
        };
        return Some(content[..end].to_string());
    }
    None
}

/// Build the urlencoded body for the credential POST: `"username=..&password=.."`.
///
/// Encoding: application/x-www-form-urlencoded with strict percent-encoding.
/// Only the unreserved set `A-Z a-z 0-9 - _ . ~` is left literal; every other
/// byte (including space, `&`, `=`, `+`, `%`, `@`) is percent-encoded as
/// `%XX` with upper-case hex. (Space is encoded as `%20`, not `+`, so the body
/// round-trips unambiguously.)
pub fn build_login_body(username: &str, password: &str) -> String {
    format!(
        "username={}&password={}",
        percent_encode(username),
        percent_encode(password)
    )
}

/// Percent-encode a string per the strict `application/x-www-form-urlencoded`
/// rules used by [`build_login_body`]: unreserved chars literal, everything
/// else `%XX` (upper-case hex).
fn percent_encode(input: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(input.len());
    for &byte in input.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push('%');
            out.push(HEX[(byte >> 4) as usize] as char);
            out.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
    out
}

/// Parse the F5 `F5_ST` cookie value.
///
/// The value is a `z`-separated record (openconnect format `"%dz%dz%dz%lldz%lld"`).
/// The 4th field is the session `start` time and the 5th is the `dur`ation.
/// Returns `Some((start, dur))` when at least five `z`-separated integer fields
/// are present, else `None`.
pub fn parse_f5_st(value: &str) -> Option<(i64, i64)> {
    let mut fields = value.split('z');
    let _f0 = fields.next()?.parse::<i64>().ok()?;
    let _f1 = fields.next()?.parse::<i64>().ok()?;
    let _f2 = fields.next()?.parse::<i64>().ok()?;
    let start = fields.next()?.parse::<i64>().ok()?;
    let dur = fields.next()?.parse::<i64>().ok()?;
    Some((start, dur))
}

/// Extract the value of `name` from the first `name=value` pair of a
/// `Cookie`/`Set-Cookie` style string (stops at the first `;`).
///
/// Returns the trimmed value, or `None` if the leading pair's name does not
/// match `name` or there is no `=`.
pub fn extract_cookie_pair(header_value: &str, name: &str) -> Option<String> {
    let pair = header_value.split(';').next()?.trim();
    let (k, v) = pair.split_once('=')?;
    if k.trim() == name {
        Some(v.trim().to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_cookies_authenticate_and_build_combined_header() {
        let mut jar = F5CookieJar::new();
        jar.ingest_set_cookie("MRHSession=abc; path=/; secure");
        jar.ingest_set_cookie("F5_ST=1z2z3z100z200; path=/");

        assert!(jar.is_authenticated());
        assert_eq!(jar.get("MRHSession"), Some("abc"));
        assert_eq!(jar.get("F5_ST"), Some("1z2z3z100z200"));
        assert_eq!(
            jar.cookie_header().as_deref(),
            Some("MRHSession=abc; F5_ST=1z2z3z100z200")
        );
    }

    #[test]
    fn only_mrhsession_is_not_authenticated() {
        let mut jar = F5CookieJar::new();
        jar.ingest_set_cookie("MRHSession=abc; path=/; secure");

        assert!(!jar.is_authenticated());
        assert_eq!(jar.cookie_header(), None);
    }

    #[test]
    fn only_f5_st_is_not_authenticated() {
        let mut jar = F5CookieJar::new();
        jar.ingest_set_cookie("F5_ST=1z2z3z100z200");

        assert!(!jar.is_authenticated());
        assert_eq!(jar.cookie_header(), None);
    }

    #[test]
    fn mrhsession_can_be_re_set_before_auth_completes() {
        let mut jar = F5CookieJar::new();
        jar.ingest_set_cookie("MRHSession=first; path=/");
        jar.ingest_set_cookie("MRHSession=second; path=/");
        assert_eq!(jar.get("MRHSession"), Some("second"));
        assert!(!jar.is_authenticated());
    }

    #[test]
    fn empty_value_clears_cookie() {
        let mut jar = F5CookieJar::new();
        jar.ingest_set_cookie("MRHSession=abc");
        jar.ingest_set_cookie("F5_ST=xyz");
        assert!(jar.is_authenticated());
        // Server deletes the cookie by re-setting an empty value.
        jar.ingest_set_cookie("F5_ST=; path=/; expires=Thu, 01 Jan 1970 00:00:00 GMT");
        assert!(!jar.is_authenticated());
        assert_eq!(jar.get("F5_ST"), None);
    }

    #[test]
    fn login_body_percent_encodes_reserved_chars() {
        let body = build_login_body("user@x", "p&ss word");
        assert!(
            body.contains("username=user%40x"),
            "body did not encode @: {body}"
        );
        assert!(
            body.contains("password=p%26ss%20word"),
            "body did not encode & and space: {body}"
        );
        assert_eq!(body, "username=user%40x&password=p%26ss%20word");
    }

    #[test]
    fn login_body_leaves_unreserved_literal() {
        let body = build_login_body("a-b_c.d~e", "AZaz09");
        assert_eq!(body, "username=a-b_c.d~e&password=AZaz09");
    }

    #[test]
    fn login_body_encodes_plus_equals_percent() {
        let body = build_login_body("a+b", "x=y%z");
        assert_eq!(body, "username=a%2Bb&password=x%3Dy%25z");
    }

    #[test]
    fn parse_f5_st_extracts_start_and_dur() {
        assert_eq!(
            parse_f5_st("0z0z0z1700000000z3600"),
            Some((1700000000, 3600))
        );
    }

    #[test]
    fn parse_f5_st_rejects_garbage() {
        assert_eq!(parse_f5_st("garbage"), None);
        assert_eq!(parse_f5_st("1z2z3z4"), None); // too few fields
        assert_eq!(parse_f5_st("1z2z3zNOTINTz5"), None); // non-integer field
    }

    #[test]
    fn extract_cookie_pair_matches_leading_name() {
        assert_eq!(
            extract_cookie_pair("MRHSession=abc; path=/; secure", "MRHSession").as_deref(),
            Some("abc")
        );
        assert_eq!(extract_cookie_pair("MRHSession=abc", "F5_ST"), None);
        assert_eq!(extract_cookie_pair("novalue", "novalue"), None);
    }

    #[test]
    fn parse_auth_form_basic() {
        let html = "<html><body>\
<form id=\"auth_form\" method=\"post\" action=\"/my.policy\">\
<input type=\"text\" name=\"username\"/>\
<input type=\"password\" name=\"password\"/>\
</form></body></html>";
        let form = F5AuthForm::parse(html).expect("form parsed");
        assert_eq!(form.id, "auth_form");
        assert_eq!(form.action, "/my.policy");
        assert_eq!(form.fields.len(), 2);
        assert!(form.fields[0].is_username());
        assert!(form.fields[1].is_password());
    }

    #[test]
    fn build_submission_fills_user_password_and_preserves_hidden() {
        let html = "<form id=\"auth_form\" method=\"post\" action=\"/my.policy\">\
<input type=\"hidden\" name=\"vhost\" value=\"standard\"/>\
<input type=\"text\" name=\"username\"/>\
<input type=\"password\" name=\"password\"/>\
</form>";
        let form = F5AuthForm::parse(html).unwrap();
        // password carries akon's PIN+OTP (single string).
        let body = form.build_submission("testuser", "1234567890");
        assert!(
            body.contains("vhost=standard"),
            "hidden not preserved: {body}"
        );
        assert!(
            body.contains("username=testuser"),
            "username missing: {body}"
        );
        assert!(
            body.contains("password=1234567890"),
            "password missing: {body}"
        );
    }

    #[test]
    fn parse_auth_form_tolerates_single_quotes_and_attr_order() {
        let html = "<form action='/step2' id='auth_form' method='post'>\
<input name='username' type='text'>\
<input value='' name='password' type='password'>\
</form>";
        let form = F5AuthForm::parse(html).unwrap();
        assert_eq!(form.id, "auth_form");
        assert_eq!(form.action, "/step2");
        assert_eq!(form.fields.len(), 2);
    }

    #[test]
    fn parse_returns_none_without_form() {
        assert!(F5AuthForm::parse("<html>no form here</html>").is_none());
    }

    #[test]
    fn tag_attr_avoids_substring_false_match() {
        // "xid" must not match "id".
        let tag = "<form xid=\"nope\" id=\"real\">";
        assert_eq!(tag_attr(tag, "id").as_deref(), Some("real"));
    }
}
