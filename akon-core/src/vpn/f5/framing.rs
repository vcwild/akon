//! F5 wire-framing codec (pure Rust).
//!
//! Implements the two F5 PPP encapsulations used by the BIG-IP SSL VPN:
//!
//! - **F5 non-HDLC** (`PPP_ENCAP_F5`, `encap_len = 4`): each PPP frame is
//!   prefixed by a 4-byte header — big-endian magic `0xf500` followed by the
//!   big-endian length of the PPP payload. Multiple frames may be concatenated
//!   in a single buffer; the next frame starts at `4 + len`.
//! - **HDLC variant** (`PPP_ENCAP_F5_HDLC`): RFC1662 async-HDLC framing with
//!   `0x7e` flag delimiters, `0x7d` escaping, an ASYNCMAP for control chars,
//!   and a trailing little-endian 16-bit PPP FCS (FCS16).
//!
//! Protocol ground truth: openconnect `ppp.c` (`hdlc_into_new_pkt`,
//! `unhdlc_in_place`) and `ppp.h` (FCS constants).

use crate::vpn::f5::F5Error;

/// F5 non-HDLC pre-PPP encapsulation magic (big-endian on the wire).
pub const F5_ENCAP_MAGIC: u16 = 0xf500;

/// Length in bytes of the F5 non-HDLC pre-PPP header (`magic` + `len`).
pub const F5_ENCAP_LEN: usize = 4;

/// HDLC frame delimiter / flag byte (RFC1662).
pub const HDLC_FLAG: u8 = 0x7e;

/// HDLC escape byte (RFC1662). The following byte is the original XOR `0x20`.
pub const HDLC_ESCAPE: u8 = 0x7d;

/// XOR value applied to an escaped HDLC byte.
pub const HDLC_XOR: u8 = 0x20;

/// Initial FCS16 value (RFC1662 `PPPINITFCS16`).
pub const PPPINITFCS16: u16 = 0xffff;

/// Expected FCS16 over `payload || fcs` for a valid frame (RFC1662
/// `PPPGOODFCS16`).
pub const PPPGOODFCS16: u16 = 0xf0b8;

/// ASYNCMAP that escapes every control char `< 0x20` (RFC1662 `ASYNCMAP_LCP`).
pub const ASYNCMAP_LCP: u32 = 0xffff_ffff;

/// Reflected FCS16 polynomial (RFC1662). Used to fold each byte into the FCS.
const FCS16_POLY: u16 = 0x8408;

/// Fold a single byte into a running FCS16 value.
///
/// This is the bit-by-bit equivalent of openconnect's `foldfcs` table lookup
/// (`(fcs >> 8) ^ fcstab[(fcs ^ c) & 0xff]`), using the reflected polynomial
/// `0x8408`.
#[inline]
fn fcs16_fold(mut fcs: u16, byte: u8) -> u16 {
    fcs ^= byte as u16;
    for _ in 0..8 {
        if fcs & 1 != 0 {
            fcs = (fcs >> 1) ^ FCS16_POLY;
        } else {
            fcs >>= 1;
        }
    }
    fcs
}

/// Compute the PPP FCS16 over `data`, initialised to [`PPPINITFCS16`].
///
/// The returned value is the *running* FCS (not yet complemented). To obtain
/// the bytes appended to a frame, complement it (`fcs ^ 0xffff`) and emit
/// little-endian. Exposed for tests and protocol vectors.
pub fn fcs16(data: &[u8]) -> u16 {
    let mut fcs = PPPINITFCS16;
    for &b in data {
        fcs = fcs16_fold(fcs, b);
    }
    fcs
}

/// Whether a byte must be HDLC-escaped given an `asyncmap`.
///
/// `0x7e` and `0x7d` are always escaped; a control char `< 0x20` is escaped
/// when its corresponding bit is set in `asyncmap`.
#[inline]
fn needs_escape(c: u8, asyncmap: u32) -> bool {
    c == HDLC_FLAG || c == HDLC_ESCAPE || (c < 0x20 && (asyncmap & (1u32 << c)) != 0)
}

/// Append `c` to `out`, escaping it per [`needs_escape`] if required.
#[inline]
fn hdlc_push(out: &mut Vec<u8>, c: u8, asyncmap: u32) {
    if needs_escape(c, asyncmap) {
        out.push(HDLC_ESCAPE);
        out.push(c ^ HDLC_XOR);
    } else {
        out.push(c);
    }
}

/// Encode a PPP payload into an F5 non-HDLC frame (`0xf500 | len | payload`).
///
/// The returned buffer is `4 + payload.len()` bytes.
pub fn f5_encap(ppp_payload: &[u8]) -> Vec<u8> {
    let len = ppp_payload.len() as u16;
    let mut out = Vec::with_capacity(F5_ENCAP_LEN + ppp_payload.len());
    out.extend_from_slice(&F5_ENCAP_MAGIC.to_be_bytes());
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(ppp_payload);
    out
}

/// Decode zero or more concatenated F5 non-HDLC frames from a buffer.
///
/// Returns the recovered PPP payloads in order. An empty buffer yields an empty
/// vector.
///
/// # Errors
///
/// - [`F5Error::BadEncapMagic`] if a frame header magic is not `0xf500`.
/// - [`F5Error::TruncatedFrame`] if a declared length exceeds the remaining
///   buffer, or a partial header is present.
pub fn f5_decap(buf: &[u8]) -> Result<Vec<Vec<u8>>, F5Error> {
    let mut out = Vec::new();
    let mut offset = 0usize;

    while offset < buf.len() {
        let remaining = buf.len() - offset;
        if remaining < F5_ENCAP_LEN {
            return Err(F5Error::TruncatedFrame {
                needed: F5_ENCAP_LEN,
                have: remaining,
            });
        }

        let magic = u16::from_be_bytes([buf[offset], buf[offset + 1]]);
        if magic != F5_ENCAP_MAGIC {
            return Err(F5Error::BadEncapMagic(magic));
        }

        let payload_len = u16::from_be_bytes([buf[offset + 2], buf[offset + 3]]) as usize;
        let frame_end = F5_ENCAP_LEN + payload_len;
        if remaining < frame_end {
            return Err(F5Error::TruncatedFrame {
                needed: frame_end,
                have: remaining,
            });
        }

        let start = offset + F5_ENCAP_LEN;
        out.push(buf[start..start + payload_len].to_vec());
        offset += frame_end;
    }

    Ok(out)
}

/// HDLC-frame a payload: compute the FCS16, escape, and wrap in `0x7e` flags.
///
/// The FCS is computed over the *unescaped* payload, complemented, appended
/// little-endian (low byte first), and the whole frame — payload + FCS — is
/// escaped and bracketed by `0x7e` flag bytes.
///
/// `asyncmap` controls which control chars `< 0x20` are escaped; pass
/// [`ASYNCMAP_LCP`] to escape all of them.
pub fn hdlc_frame(payload: &[u8], asyncmap: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() * 2 + 4);
    out.push(HDLC_FLAG);

    for &b in payload {
        hdlc_push(&mut out, b, asyncmap);
    }

    let fcs = fcs16(payload) ^ 0xffff;
    hdlc_push(&mut out, (fcs & 0xff) as u8, asyncmap);
    hdlc_push(&mut out, (fcs >> 8) as u8, asyncmap);

    out.push(HDLC_FLAG);
    out
}

/// De-frame a single HDLC frame: strip flags, unescape, verify FCS16, and
/// return the payload (with the trailing FCS removed).
///
/// A leading `0x7e` flag is optional (mirroring openconnect's tolerance); the
/// frame is read up to the next `0x7e`.
///
/// # Errors
///
/// - [`F5Error::HdlcFcsInvalid`] if the frame is too short to contain a FCS or
///   the FCS check (`fcs == PPPGOODFCS16`) fails.
pub fn hdlc_deframe(frame: &[u8]) -> Result<Vec<u8>, F5Error> {
    let mut inp = frame;

    // Optional leading flag.
    if let Some((&first, rest)) = inp.split_first() {
        if first == HDLC_FLAG {
            inp = rest;
        }
    }

    let mut unescaped = Vec::with_capacity(inp.len());
    let mut escape = false;
    for &c in inp {
        if c == HDLC_FLAG {
            // Trailing flag: end of frame.
            break;
        } else if escape {
            unescaped.push(c ^ HDLC_XOR);
            escape = false;
        } else if c == HDLC_ESCAPE {
            escape = true;
        } else {
            unescaped.push(c);
        }
    }

    // Must contain at least the 2-byte FCS.
    if unescaped.len() < 2 {
        return Err(F5Error::HdlcFcsInvalid);
    }

    // FCS over (payload || received FCS) must equal PPPGOODFCS16.
    if fcs16(&unescaped) != PPPGOODFCS16 {
        return Err(F5Error::HdlcFcsInvalid);
    }

    unescaped.truncate(unescaped.len() - 2);
    Ok(unescaped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f5_encap_byte_exact() {
        let frame = f5_encap(&[0x21, 0xAA, 0xBB]);
        assert_eq!(frame, vec![0xF5, 0x00, 0x00, 0x03, 0x21, 0xAA, 0xBB]);
    }

    #[test]
    fn f5_encap_empty_payload() {
        assert_eq!(f5_encap(&[]), vec![0xF5, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn f5_round_trip_single() {
        let x = vec![0x80, 0x21, 0x01, 0x00, 0xDE, 0xAD, 0xBE, 0xEF];
        let decoded = f5_decap(&f5_encap(&x)).unwrap();
        assert_eq!(decoded, vec![x]);
    }

    #[test]
    fn f5_decap_empty_buffer() {
        assert_eq!(f5_decap(&[]).unwrap(), Vec::<Vec<u8>>::new());
    }

    #[test]
    fn f5_decap_two_concatenated_frames() {
        let a = vec![0x21, 0x01];
        let b = vec![0x57, 0x02, 0x03];
        let mut buf = f5_encap(&a);
        buf.extend_from_slice(&f5_encap(&b));

        let decoded = f5_decap(&buf).unwrap();
        assert_eq!(decoded, vec![a, b]);
    }

    #[test]
    fn f5_decap_bad_magic() {
        // 0xf400 magic instead of 0xf500.
        let buf = [0xF4, 0x00, 0x00, 0x01, 0xAA];
        assert_eq!(f5_decap(&buf), Err(F5Error::BadEncapMagic(0xf400)));
    }

    #[test]
    fn f5_decap_truncated_payload() {
        // Declares 5 bytes of payload but only provides 2.
        let buf = [0xF5, 0x00, 0x00, 0x05, 0xAA, 0xBB];
        match f5_decap(&buf) {
            Err(F5Error::TruncatedFrame { needed, have }) => {
                assert_eq!(needed, 9);
                assert_eq!(have, 6);
            }
            other => panic!("expected TruncatedFrame, got {other:?}"),
        }
    }

    #[test]
    fn f5_decap_truncated_header() {
        // Only 3 bytes: not even a full 4-byte header.
        let buf = [0xF5, 0x00, 0x00];
        match f5_decap(&buf) {
            Err(F5Error::TruncatedFrame { needed, have }) => {
                assert_eq!(needed, 4);
                assert_eq!(have, 3);
            }
            other => panic!("expected TruncatedFrame, got {other:?}"),
        }
    }

    #[test]
    fn hdlc_round_trip_simple() {
        for payload in [
            vec![0x21u8],
            vec![0xc0, 0x21, 0x01, 0x00, 0x00, 0x04],
            vec![0x00, 0x01, 0x02, 0x1f, 0x20, 0x80, 0xff],
        ] {
            let framed = hdlc_frame(&payload, ASYNCMAP_LCP);
            assert_eq!(framed.first(), Some(&HDLC_FLAG));
            assert_eq!(framed.last(), Some(&HDLC_FLAG));
            let recovered = hdlc_deframe(&framed).unwrap();
            assert_eq!(recovered, payload, "round-trip mismatch");
        }
    }

    #[test]
    fn hdlc_escapes_flag_and_escape_bytes() {
        // Payload containing both 0x7e and 0x7d must be escaped.
        let payload = vec![0x7e, 0x7d, 0xAB];
        let framed = hdlc_frame(&payload, ASYNCMAP_LCP);

        // The framed bytes must contain an escape byte 0x7d.
        assert!(framed.contains(&HDLC_ESCAPE), "escape byte 0x7d missing");

        // The literal 0x7e payload byte must have been escaped, so the only
        // remaining 0x7e bytes are the two flag delimiters.
        let flag_count = framed.iter().filter(|&&b| b == HDLC_FLAG).count();
        assert_eq!(flag_count, 2, "0x7e must only appear as the two delimiters");

        // Escaped representations must be present: 0x7e -> 0x7d 0x5e,
        // 0x7d -> 0x7d 0x5d.
        assert!(
            framed.windows(2).any(|w| w == [0x7d, 0x5e]),
            "0x7e not escaped as 0x7d 0x5e"
        );
        assert!(
            framed.windows(2).any(|w| w == [0x7d, 0x5d]),
            "0x7d not escaped as 0x7d 0x5d"
        );

        // And it still round-trips.
        assert_eq!(hdlc_deframe(&framed).unwrap(), payload);
    }

    #[test]
    fn hdlc_asyncmap_zero_does_not_escape_control_chars() {
        // With asyncmap 0, control chars < 0x20 (other than 0x7e/0x7d) are not
        // escaped, but 0x7e/0x7d still are.
        let payload = vec![0x01, 0x02, 0x1f];
        let framed = hdlc_frame(&payload, 0);
        // No escape bytes expected for these control chars.
        assert!(!framed.contains(&HDLC_ESCAPE));
        assert_eq!(hdlc_deframe(&framed).unwrap(), payload);
    }

    #[test]
    fn fcs16_known_vector_and_good_fcs() {
        // FCS over a known input. Computed with the RFC1662 algorithm.
        let payload = [0x21u8, 0xAA, 0xBB];
        let running = fcs16(&payload);
        let appended = running ^ 0xffff;
        let fcs_le = [(appended & 0xff) as u8, (appended >> 8) as u8];

        // Running FCS over (payload || fcs_le) must equal PPPGOODFCS16.
        let mut full = payload.to_vec();
        full.extend_from_slice(&fcs_le);
        assert_eq!(fcs16(&full), PPPGOODFCS16);

        // PPPGOODFCS16 constant sanity check.
        assert_eq!(PPPGOODFCS16, 0xf0b8);
        assert_eq!(PPPINITFCS16, 0xffff);

        // Stable byte-exact vector for this payload (regression guard).
        assert_eq!(fcs_le, [0xfc, 0xc6]);
    }

    #[test]
    fn fcs16_empty_input() {
        // FCS of nothing is the init value; complement is the well-known
        // 0xffff -> appended 0x0000... actually init un-folded.
        assert_eq!(fcs16(&[]), PPPINITFCS16);
    }

    #[test]
    fn hdlc_deframe_corrupted_fcs() {
        let payload = vec![0x21, 0x01, 0x02, 0x03];
        let mut framed = hdlc_frame(&payload, ASYNCMAP_LCP);

        // Corrupt a payload byte just after the leading flag (index 1).
        framed[1] ^= 0xff;

        assert_eq!(hdlc_deframe(&framed), Err(F5Error::HdlcFcsInvalid));
    }

    #[test]
    fn hdlc_deframe_too_short() {
        // Just two flags: no payload/FCS.
        let framed = vec![HDLC_FLAG, HDLC_FLAG];
        assert_eq!(hdlc_deframe(&framed), Err(F5Error::HdlcFcsInvalid));
    }
}
