//! F5 profile/options XML parsing (pure Rust, dependency-free).
//!
//! F5 BIG-IP returns flat XML for both the VPN profile and the tunnel options.
//! The profile (`/vdesk/vpn/index.php3?outform=xml`) looks like:
//!
//! ```xml
//! <favorites type="VPN" limited="YES">
//!   <favorite id="/Common/demo">
//!     <params>resourcename=/Common/demo</params>
//!   </favorite>
//! </favorites>
//! ```
//!
//! The options XML has root `<favorite><object>...</object></favorite>` whose
//! many flat children carry the per-tunnel settings as element text, e.g.
//! `<Session_ID>SID</Session_ID>`, `<IPV4_0>1</IPV4_0>`, `<DNS0>8.8.8.8</DNS0>`,
//! `<LAN0>10.0.0.0/8</LAN0>`.
//!
//! Rather than pull in an XML crate, this module ships a tiny tolerant scanner
//! sufficient for this flat structure: it walks `<tag ...>text</tag>` pairs
//! (and bare self-closing tags), unescaping the five predefined XML entities.
//!
//! Protocol ground truth: openconnect `f5.c` (`parse_profile`, `parse_options`)
//! and `auth-common.c` (`xmlnode_bool_or_int_value`).

use crate::vpn::f5::F5Error;

/// Parsed F5 VPN options (the data needed to bring up the tunnel).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct F5Options {
    /// `Session_ID` — the `/myvpn` `sess=` parameter.
    pub session_id: Option<String>,
    /// `ur_Z` — the `/myvpn` `Z=` parameter.
    pub ur_z: Option<String>,
    /// `IPV4_0` — whether IPv4 transport is enabled.
    pub ipv4: bool,
    /// `IPV6_0` — whether IPv6 transport is enabled.
    pub ipv6: bool,
    /// `hdlc_framing` — whether RFC1662 HDLC-like framing is used.
    pub hdlc_framing: bool,
    /// `idle_session_timeout` — idle timeout in seconds.
    pub idle_timeout: Option<u32>,
    /// `tunnel_dtls` — whether DTLS transport is offered.
    pub dtls: bool,
    /// `tunnel_port_dtls` — the UDP port for DTLS, when enabled.
    pub dtls_port: Option<u16>,
    /// `DNS0`..`DNS2` — DNS servers, in document order.
    pub dns: Vec<String>,
    /// `DNSSuffix0`.. — DNS search domains, in document order.
    pub domains: Vec<String>,
    /// `LAN0`.. — split-include routes (one tag may hold several whitespace-
    /// separated routes).
    pub routes: Vec<String>,
    /// `UseDefaultGateway0` — whether the default route should be installed.
    pub default_gateway: bool,
}

/// A single flat XML element discovered by the [scanner](scan_elements):
/// its tag `name` and decoded text `content` (empty for self-closing tags).
#[derive(Debug, Clone, PartialEq, Eq)]
struct XmlElement {
    name: String,
    content: String,
}

/// Extract the resource params from the profile XML (first `<params>` text
/// inside a `<favorites type="VPN">`).
///
/// Returns [`F5Error::InvalidConfig`] if there is no `<favorites type="VPN">`
/// containing a non-self-closing `<params>` element.
pub fn parse_profile(xml: &str) -> Result<String, F5Error> {
    // Find a <favorites ...> open tag whose type attribute is "VPN", then take
    // the first <params>..</params> text within that favorites block.
    let mut rest = xml;
    while let Some(open) = find_open_tag(rest, "favorites") {
        let attrs = &rest[open.attrs_start..open.tag_end];
        let block = &rest[open.tag_end..];
        // Bound the search to this favorites block (up to its closing tag, if any).
        let block = match find_close_tag(block, "favorites") {
            Some(end) => &block[..end],
            None => block,
        };

        if tag_attr(attrs, "type").as_deref() == Some("VPN") {
            if let Some(params) = first_element_text(block, "params") {
                return Ok(params);
            }
        }
        rest = &rest[open.tag_end..];
    }

    Err(F5Error::InvalidConfig(
        "no <favorites type=\"VPN\"> with a <params> element".to_string(),
    ))
}

/// Parse the options XML into [`F5Options`].
///
/// Requires at least one of `ipv4`/`ipv6` to be enabled **and** both `ur_z`
/// and `session_id` to be present, mirroring openconnect's
/// `(*ipv4 < 1 && *ipv6 < 1) || !*ur_z || !*session_id` failure check.
/// Otherwise returns [`F5Error::InvalidConfig`].
pub fn parse_options(xml: &str) -> Result<F5Options, F5Error> {
    let mut opts = F5Options::default();

    for el in scan_elements(xml) {
        let name = el.name.as_str();
        let text = el.content.trim();

        match name {
            "Session_ID" => set_nonempty(&mut opts.session_id, text),
            "ur_Z" => set_nonempty(&mut opts.ur_z, text),
            "IPV4_0" => opts.ipv4 = bool_or_int_value(text).unwrap_or(false),
            "IPV6_0" => opts.ipv6 = bool_or_int_value(text).unwrap_or(false),
            "hdlc_framing" => opts.hdlc_framing = bool_or_int_value(text).unwrap_or(false),
            "idle_session_timeout" => {
                if let Ok(n) = text.parse::<u32>() {
                    opts.idle_timeout = Some(n);
                }
            }
            "tunnel_dtls" => opts.dtls = bool_or_int_value(text).unwrap_or(false),
            "tunnel_port_dtls" => {
                if let Ok(p) = text.parse::<u16>() {
                    opts.dtls_port = Some(p);
                }
            }
            "UseDefaultGateway0" => opts.default_gateway = bool_or_int_value(text).unwrap_or(false),
            _ => {
                // The flat, numbered families: DNS<n>, DNSSuffix<n>, LAN<n>.
                if let Some(rest) = name.strip_prefix("DNSSuffix") {
                    if is_all_digits(rest) && !text.is_empty() {
                        opts.domains.push(text.to_string());
                    }
                } else if let Some(rest) = name.strip_prefix("DNS") {
                    if is_all_digits(rest) && !text.is_empty() {
                        opts.dns.push(text.to_string());
                    }
                } else if let Some(rest) = name.strip_prefix("LAN") {
                    if is_all_digits(rest) {
                        // One LAN tag may carry several whitespace-separated routes.
                        for route in text.split_whitespace() {
                            opts.routes.push(route.to_string());
                        }
                    }
                }
            }
        }
    }

    if (!opts.ipv4 && !opts.ipv6) || opts.ur_z.is_none() || opts.session_id.is_none() {
        return Err(F5Error::InvalidConfig(
            "options XML missing ur_Z, Session_ID, or any of IPV4_0/IPV6_0".to_string(),
        ));
    }

    Ok(opts)
}

/// Store `text` into `slot` when non-empty (mirrors the openconnect behaviour
/// where an empty element does not satisfy the `!*x` presence checks).
fn set_nonempty(slot: &mut Option<String>, text: &str) {
    if !text.is_empty() {
        *slot = Some(text.to_string());
    }
}

/// True iff `s` is non-empty and consists solely of ASCII digits (used to gate
/// the numbered tag families like `DNS0`, `LAN12`).
fn is_all_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// Interpret a flat F5 boolean/int value.
///
/// Mirrors openconnect's `xmlnode_bool_or_int_value`: a leading digit means the
/// value is an integer (non-zero ⇒ `true`); otherwise `"yes"`/`"on"` ⇒ `true`
/// and `"no"`/`"off"` ⇒ `false` (case-insensitive). Anything else ⇒ `None`.
fn bool_or_int_value(text: &str) -> Option<bool> {
    let t = text.trim();
    let first = t.bytes().next()?;
    if first.is_ascii_digit() {
        // atoi-style: parse the leading integer run.
        let digits: String = t
            .bytes()
            .take_while(u8::is_ascii_digit)
            .map(char::from)
            .collect();
        return digits.parse::<i64>().ok().map(|n| n != 0);
    }
    if t.eq_ignore_ascii_case("yes") || t.eq_ignore_ascii_case("on") {
        Some(true)
    } else if t.eq_ignore_ascii_case("no") || t.eq_ignore_ascii_case("off") {
        Some(false)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Minimal flat-XML scanner
// ---------------------------------------------------------------------------

/// A located `<name ...>` open tag.
struct OpenTag {
    /// Byte offset where the tag's attribute span begins (just after the name).
    attrs_start: usize,
    /// Byte offset just past the closing `>` of the open tag.
    tag_end: usize,
}

/// Find the first `<name ...>` open tag in `haystack` (ignoring self-closing
/// `<name/>` forms). Returns its attribute span and end offset.
fn find_open_tag(haystack: &str, name: &str) -> Option<OpenTag> {
    let bytes = haystack.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        // Skip declarations/comments/processing-instructions/closing tags.
        if matches!(bytes.get(i + 1), Some(b'/') | Some(b'!') | Some(b'?')) {
            i += 1;
            continue;
        }
        let after = i + 1;
        if haystack[after..].starts_with(name) {
            let next = after + name.len();
            // The char after the name must delimit the tag name.
            let delim = bytes.get(next).copied();
            if matches!(
                delim,
                Some(b'>') | Some(b'/') | Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r')
            ) {
                if let Some(close_rel) = haystack[next..].find('>') {
                    let tag_end = next + close_rel + 1;
                    // Ignore self-closing tags ("<name/>").
                    if bytes[tag_end - 2] != b'/' {
                        return Some(OpenTag {
                            attrs_start: next,
                            tag_end,
                        });
                    }
                }
            }
        }
        i += 1;
    }
    None
}

/// Find the byte offset (relative to `haystack`) of the start of the first
/// `</name>` closing tag.
fn find_close_tag(haystack: &str, name: &str) -> Option<usize> {
    let needle = format!("</{name}");
    let pos = haystack.find(&needle)?;
    // Ensure the char after the name terminates it (avoid "</favorites2>").
    let after = pos + needle.len();
    let delim = haystack.as_bytes().get(after).copied();
    if matches!(
        delim,
        Some(b'>') | Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r')
    ) {
        Some(pos)
    } else {
        None
    }
}

/// Return the decoded text of the first `<name>..</name>` element in `haystack`,
/// or `None` if absent or self-closing.
fn first_element_text(haystack: &str, name: &str) -> Option<String> {
    let open = find_open_tag(haystack, name)?;
    let body = &haystack[open.tag_end..];
    let end = find_close_tag(body, name)?;
    Some(decode_entities(&body[..end]))
}

/// Walk every simple `<tag ...>text</tag>` (and self-closing `<tag/>`) element
/// in `xml`, returning each with its decoded text content in document order.
///
/// This is intentionally shallow: it does not build a tree. For the flat F5
/// options document that is exactly what is needed — every leaf element under
/// `<object>` is yielded once. Nested elements are still yielded individually.
fn scan_elements(xml: &str) -> Vec<XmlElement> {
    let bytes = xml.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        // Skip comments.
        if xml[i..].starts_with("<!--") {
            match xml[i..].find("-->") {
                Some(rel) => i += rel + 3,
                None => break,
            }
            continue;
        }
        // Skip declarations / PIs / closing tags — not element openers we emit.
        if matches!(bytes.get(i + 1), Some(b'/') | Some(b'!') | Some(b'?')) {
            i += 1;
            continue;
        }

        // Parse a tag name starting at i+1.
        let name_start = i + 1;
        let mut j = name_start;
        while j < bytes.len() && !matches!(bytes[j], b' ' | b'\t' | b'\n' | b'\r' | b'>' | b'/') {
            j += 1;
        }
        if j == name_start {
            i += 1;
            continue;
        }
        let name = &xml[name_start..j];

        // Find the end of the open tag.
        let Some(gt_rel) = xml[j..].find('>') else {
            break;
        };
        let tag_end = j + gt_rel + 1;
        let self_closing = bytes[tag_end - 2] == b'/';

        if self_closing {
            out.push(XmlElement {
                name: name.to_string(),
                content: String::new(),
            });
            i = tag_end;
            continue;
        }

        // Non-self-closing: capture text up to the matching close tag.
        let body = &xml[tag_end..];
        match find_close_tag(body, name) {
            Some(close_off) => {
                let content = decode_entities(&body[..close_off]);
                out.push(XmlElement {
                    name: name.to_string(),
                    content,
                });
                // Advance just past the open tag so nested elements are also
                // scanned (the F5 docs are flat, but this keeps the scan robust).
                i = tag_end;
            }
            None => {
                // No matching close tag; treat as empty and move on.
                out.push(XmlElement {
                    name: name.to_string(),
                    content: String::new(),
                });
                i = tag_end;
            }
        }
    }

    out
}

/// Read an attribute value (`name="..."` or `name='...'`) out of an open-tag
/// attribute span. Returns the raw (entity-decoded) value if present.
fn tag_attr(attrs: &str, name: &str) -> Option<String> {
    let bytes = attrs.as_bytes();
    let mut i = 0;
    while i + name.len() <= bytes.len() {
        if attrs[i..].starts_with(name) {
            // Must be a standalone attribute name: preceded by start/space, and
            // followed (after optional spaces) by '='.
            let prev_ok = i == 0 || matches!(bytes[i - 1], b' ' | b'\t' | b'\n' | b'\r');
            let mut k = i + name.len();
            while k < bytes.len() && matches!(bytes[k], b' ' | b'\t' | b'\n' | b'\r') {
                k += 1;
            }
            if prev_ok && bytes.get(k) == Some(&b'=') {
                k += 1;
                while k < bytes.len() && matches!(bytes[k], b' ' | b'\t' | b'\n' | b'\r') {
                    k += 1;
                }
                let quote = bytes.get(k).copied();
                if matches!(quote, Some(b'"') | Some(b'\'')) {
                    let q = quote.unwrap();
                    let val_start = k + 1;
                    if let Some(end_rel) = attrs[val_start..].find(q as char) {
                        return Some(decode_entities(&attrs[val_start..val_start + end_rel]));
                    }
                }
            }
        }
        i += 1;
    }
    None
}

/// Decode the five predefined XML entities and trim no whitespace (callers trim
/// as needed). Unknown entities are left verbatim.
fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let tail = &rest[amp..];
        if let Some(semi) = tail.find(';') {
            let entity = &tail[..=semi];
            match entity {
                "&amp;" => out.push('&'),
                "&lt;" => out.push('<'),
                "&gt;" => out.push('>'),
                "&quot;" => out.push('"'),
                "&apos;" => out.push('\''),
                other => out.push_str(other),
            }
            rest = &tail[semi + 1..];
        } else {
            out.push('&');
            rest = &tail[1..];
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_profile_extracts_first_params() {
        let xml = r#"<favorites type="VPN" limited="YES"><favorite id="/Common/demo"><params>resourcename=/Common/demo</params></favorite></favorites>"#;
        assert_eq!(
            parse_profile(xml).unwrap(),
            "resourcename=/Common/demo".to_string()
        );
    }

    #[test]
    fn parse_profile_with_declaration_and_whitespace() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
            <favorites type="VPN" limited="YES">
              <favorite id="/Common/demo">
                <caption>demo</caption>
                <name>/Common/demo</name>
                <params>resourcename=/Common/demo</params>
              </favorite>
            </favorites>"#;
        assert_eq!(parse_profile(xml).unwrap(), "resourcename=/Common/demo");
    }

    #[test]
    fn parse_profile_skips_non_vpn_favorites() {
        let xml = r#"<root><favorites type="OTHER"><favorite><params>nope</params></favorite></favorites><favorites type="VPN"><favorite><params>yes=1</params></favorite></favorites></root>"#;
        assert_eq!(parse_profile(xml).unwrap(), "yes=1");
    }

    #[test]
    fn parse_profile_no_vpn_favorites_errors() {
        let xml =
            r#"<favorites type="OTHER"><favorite><params>nope</params></favorite></favorites>"#;
        assert!(matches!(parse_profile(xml), Err(F5Error::InvalidConfig(_))));
    }

    #[test]
    fn parse_profile_decodes_entities() {
        let xml = r#"<favorites type="VPN"><favorite><params>a=1&amp;b=2</params></favorite></favorites>"#;
        assert_eq!(parse_profile(xml).unwrap(), "a=1&b=2");
    }

    #[test]
    fn parse_options_full_document() {
        let xml = r#"<favorite><object><Session_ID>SID123</Session_ID><ur_Z>URZ456</ur_Z><IPV4_0>1</IPV4_0><IPV6_0>0</IPV6_0><hdlc_framing>no</hdlc_framing><DNS0>8.8.8.8</DNS0><DNS1>1.1.1.1</DNS1><LAN0>10.0.0.0/8</LAN0><UseDefaultGateway0>1</UseDefaultGateway0></object></favorite>"#;
        let opts = parse_options(xml).unwrap();

        assert_eq!(opts.session_id.as_deref(), Some("SID123"));
        assert_eq!(opts.ur_z.as_deref(), Some("URZ456"));
        assert!(opts.ipv4);
        assert!(!opts.ipv6);
        assert!(!opts.hdlc_framing);
        assert_eq!(opts.dns, vec!["8.8.8.8".to_string(), "1.1.1.1".to_string()]);
        assert_eq!(opts.routes, vec!["10.0.0.0/8".to_string()]);
        assert!(opts.default_gateway);
    }

    #[test]
    fn parse_options_collects_domains_and_multi_route_lan() {
        let xml = r#"<favorite><object><Session_ID>S</Session_ID><ur_Z>Z</ur_Z><IPV6_0>1</IPV6_0><DNSSuffix0>corp.example</DNSSuffix0><DNSSuffix1>example.com</DNSSuffix1><LAN0>10.0.0.0/8 192.168.0.0/16</LAN0><LAN1>172.16.0.0/12</LAN1></object></favorite>"#;
        let opts = parse_options(xml).unwrap();
        assert!(opts.ipv6);
        assert_eq!(
            opts.domains,
            vec!["corp.example".to_string(), "example.com".to_string()]
        );
        assert_eq!(
            opts.routes,
            vec![
                "10.0.0.0/8".to_string(),
                "192.168.0.0/16".to_string(),
                "172.16.0.0/12".to_string(),
            ]
        );
    }

    #[test]
    fn parse_options_idle_timeout_and_dtls() {
        let xml = r#"<favorite><object><Session_ID>S</Session_ID><ur_Z>Z</ur_Z><IPV4_0>1</IPV4_0><idle_session_timeout>1800</idle_session_timeout><tunnel_dtls>yes</tunnel_dtls><tunnel_port_dtls>4433</tunnel_port_dtls></object></favorite>"#;
        let opts = parse_options(xml).unwrap();
        assert_eq!(opts.idle_timeout, Some(1800));
        assert!(opts.dtls);
        assert_eq!(opts.dtls_port, Some(4433));
    }

    #[test]
    fn parse_options_missing_ur_z_errors() {
        let xml =
            r#"<favorite><object><Session_ID>S</Session_ID><IPV4_0>1</IPV4_0></object></favorite>"#;
        assert!(matches!(parse_options(xml), Err(F5Error::InvalidConfig(_))));
    }

    #[test]
    fn parse_options_missing_session_id_errors() {
        let xml = r#"<favorite><object><ur_Z>Z</ur_Z><IPV4_0>1</IPV4_0></object></favorite>"#;
        assert!(matches!(parse_options(xml), Err(F5Error::InvalidConfig(_))));
    }

    #[test]
    fn parse_options_no_ip_family_errors() {
        let xml = r#"<favorite><object><Session_ID>S</Session_ID><ur_Z>Z</ur_Z><IPV4_0>0</IPV4_0><IPV6_0>0</IPV6_0></object></favorite>"#;
        assert!(matches!(parse_options(xml), Err(F5Error::InvalidConfig(_))));
    }

    #[test]
    fn bool_parsing_truthy_and_falsy_forms() {
        assert_eq!(bool_or_int_value("yes"), Some(true));
        assert_eq!(bool_or_int_value("on"), Some(true));
        assert_eq!(bool_or_int_value("1"), Some(true));
        assert_eq!(bool_or_int_value("YES"), Some(true));
        assert_eq!(bool_or_int_value("On"), Some(true));
        assert_eq!(bool_or_int_value("42"), Some(true));

        assert_eq!(bool_or_int_value("no"), Some(false));
        assert_eq!(bool_or_int_value("off"), Some(false));
        assert_eq!(bool_or_int_value("0"), Some(false));
        assert_eq!(bool_or_int_value("OFF"), Some(false));

        assert_eq!(bool_or_int_value("maybe"), None);
        assert_eq!(bool_or_int_value(""), None);
    }

    #[test]
    fn scanner_handles_self_closing_and_whitespace() {
        let xml = "<favorite>\n  <object>\n    <Session_ID>S</Session_ID>\n    <ur_Z>Z</ur_Z>\n    <IPV4_0>1</IPV4_0>\n    <empty/>\n  </object>\n</favorite>";
        let opts = parse_options(xml).unwrap();
        assert_eq!(opts.session_id.as_deref(), Some("S"));
        assert_eq!(opts.ur_z.as_deref(), Some("Z"));
        assert!(opts.ipv4);
    }

    #[test]
    fn dns_suffix_not_confused_with_dns() {
        // DNSSuffix must not be captured as a DNS server.
        let xml = r#"<favorite><object><Session_ID>S</Session_ID><ur_Z>Z</ur_Z><IPV4_0>1</IPV4_0><DNS0>8.8.8.8</DNS0><DNSSuffix0>corp</DNSSuffix0></object></favorite>"#;
        let opts = parse_options(xml).unwrap();
        assert_eq!(opts.dns, vec!["8.8.8.8".to_string()]);
        assert_eq!(opts.domains, vec!["corp".to_string()]);
    }
}
