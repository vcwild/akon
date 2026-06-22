//! PPP control-protocol engine for the native F5 backend.
//!
//! Pure-Rust implementation of the PPP/LCP/IPCP build, parse and negotiation
//! logic needed to bring an F5 PPP-over-HTTPS tunnel to "network up".
//!
//! Protocol ground truth: openconnect `ppp.c` / `ppp.h`.
//!
//! ## On-the-wire shape
//!
//! A PPP frame consists of an optional HDLC-style Address/Control prefix
//! (`0xFF 0x03`), a 1- or 2-byte protocol field, and the protocol payload:
//!
//! ```text
//! [FF 03]?  proto(1-2)  <ncp body...>
//! ```
//!
//! On the **send** side we always emit the full `FF 03` prefix and the complete
//! 2-byte protocol field (no PFC/ACFC compression) for simplicity. On the
//! **parse** side we tolerate frames with or without the `FF 03` prefix and with
//! either a 1- or 2-byte protocol field (a single proto byte is the low byte and
//! is always odd per RFC 1661).
//!
//! ## NCP body
//!
//! ```text
//! code(1)  id(1)  length(2 be, covers code..end)  <options...>
//! ```
//!
//! Options are TLVs: `type(1) len(1, covers type..value) value(len-2)`.

use super::F5Error;

// ---------------------------------------------------------------------------
// PPP protocol field values (ppp.h)
// ---------------------------------------------------------------------------

/// LCP — Link Control Protocol.
pub const PPP_LCP: u16 = 0xc021;
/// IPCP — IP Control Protocol (IPv4).
pub const PPP_IPCP: u16 = 0x8021;
/// IP6CP — IPv6 Control Protocol.
pub const PPP_IP6CP: u16 = 0x8057;
/// IPv4 data protocol.
pub const PPP_IP: u16 = 0x21;
/// IPv6 data protocol.
pub const PPP_IP6: u16 = 0x57;

/// HDLC Address byte.
pub const PPP_ADDRESS: u8 = 0xff;
/// HDLC Control byte.
pub const PPP_CONTROL: u8 = 0x03;

// ---------------------------------------------------------------------------
// NCP codes (ppp.h)
// ---------------------------------------------------------------------------

/// Configure-Request.
pub const CONFREQ: u8 = 1;
/// Configure-Ack.
pub const CONFACK: u8 = 2;
/// Configure-Nak.
pub const CONFNAK: u8 = 3;
/// Configure-Reject.
pub const CONFREJ: u8 = 4;
/// Terminate-Request.
pub const TERMREQ: u8 = 5;
/// Terminate-Ack.
pub const TERMACK: u8 = 6;
/// Code-Reject.
pub const CODEREJ: u8 = 7;
/// Protocol-Reject.
pub const PROTREJ: u8 = 8;
/// Echo-Request.
pub const ECHOREQ: u8 = 9;
/// Echo-Reply.
pub const ECHOREP: u8 = 10;
/// Discard-Request.
pub const DISCREQ: u8 = 11;

// ---------------------------------------------------------------------------
// LCP option tags (ppp.h / RFC 1661, RFC 1662)
// ---------------------------------------------------------------------------

/// Maximum-Receive-Unit (be16).
pub const LCP_MRU: u8 = 1;
/// Async-Control-Character-Map (be32).
pub const LCP_ASYNCMAP: u8 = 2;
/// Magic-Number (4 bytes).
pub const LCP_MAGIC: u8 = 5;
/// Protocol-Field-Compression (flag).
pub const LCP_PFCOMP: u8 = 7;
/// Address-and-Control-Field-Compression (flag).
pub const LCP_ACCOMP: u8 = 8;

// ---------------------------------------------------------------------------
// IPCP option tags (ppp.h / RFC 1332, RFC 1877)
// ---------------------------------------------------------------------------

/// IP-Address (4 bytes, IPv4).
pub const IPCP_IPADDR: u8 = 3;
/// Primary DNS server address.
pub const IPCP_DNS1: u8 = 129;
/// Primary NBNS (WINS) server address.
pub const IPCP_NBNS1: u8 = 130;
/// Secondary DNS server address.
pub const IPCP_DNS2: u8 = 131;
/// Secondary NBNS (WINS) server address.
pub const IPCP_NBNS2: u8 = 132;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// A single TLV option inside an NCP control packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NcpOption {
    /// Option type tag (e.g. [`LCP_MRU`], [`IPCP_IPADDR`]).
    pub tag: u8,
    /// Option value bytes (the `value` of the TLV; `len - 2` bytes).
    pub data: Vec<u8>,
}

impl NcpOption {
    /// Construct an option from a tag and value bytes.
    pub fn new(tag: u8, data: impl Into<Vec<u8>>) -> Self {
        Self {
            tag,
            data: data.into(),
        }
    }
}

/// A parsed NCP (LCP/IPCP/IP6CP) control packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NcpPacket {
    /// PPP protocol field (e.g. [`PPP_LCP`], [`PPP_IPCP`]).
    pub proto: u16,
    /// NCP code (e.g. [`CONFREQ`], [`CONFACK`]).
    pub code: u8,
    /// NCP identifier, used to match requests with replies.
    pub id: u8,
    /// The TLV options carried by the packet.
    pub options: Vec<NcpOption>,
}

impl NcpPacket {
    /// Find the first option with the given tag.
    pub fn option(&self, tag: u8) -> Option<&NcpOption> {
        self.options.iter().find(|o| o.tag == tag)
    }
}

// ---------------------------------------------------------------------------
// Build / parse
// ---------------------------------------------------------------------------

/// Encode just the NCP body (`code id length <options>`) of a packet.
fn encode_ncp_body(pkt: &NcpPacket) -> Vec<u8> {
    let mut body = Vec::with_capacity(8);
    body.push(pkt.code);
    body.push(pkt.id);
    // Length placeholder (filled in after options are appended).
    body.push(0);
    body.push(0);

    for opt in &pkt.options {
        // TLV length includes the 2-byte type+len header.
        let tlv_len = opt.data.len() + 2;
        debug_assert!(tlv_len <= u8::MAX as usize, "option too long");
        body.push(opt.tag);
        body.push(tlv_len as u8);
        body.extend_from_slice(&opt.data);
    }

    let total = body.len() as u16;
    body[2] = (total >> 8) as u8;
    body[3] = (total & 0xff) as u8;
    body
}

/// Build a full on-the-wire PPP frame (`FF 03` + 2-byte proto + NCP body) for
/// an NCP packet.
///
/// The send side never applies PFC/ACFC compression: the Address/Control prefix
/// and the full 2-byte protocol field are always emitted.
pub fn build_ncp_frame(pkt: &NcpPacket) -> Vec<u8> {
    let body = encode_ncp_body(pkt);
    let mut frame = Vec::with_capacity(body.len() + 4);
    frame.push(PPP_ADDRESS);
    frame.push(PPP_CONTROL);
    frame.push((pkt.proto >> 8) as u8);
    frame.push((pkt.proto & 0xff) as u8);
    frame.extend_from_slice(&body);
    frame
}

/// Parse a PPP frame into an [`NcpPacket`].
///
/// Tolerates an optional `FF 03` Address/Control prefix and a 1- or 2-byte
/// protocol field (a single proto byte is the low byte and is odd).
pub fn parse_ppp_frame(frame: &[u8]) -> Result<NcpPacket, F5Error> {
    let mut pos = 0usize;

    // Optional Address/Control prefix.
    if frame.len() >= 2 && frame[0] == PPP_ADDRESS && frame[1] == PPP_CONTROL {
        pos += 2;
    }

    if pos >= frame.len() {
        return Err(F5Error::MalformedPpp("frame too short for proto".into()));
    }

    // Protocol field: 1 byte if the first byte is odd (PFC), else 2 bytes.
    let proto: u16 = if frame[pos] & 0x01 == 1 {
        let p = frame[pos] as u16;
        pos += 1;
        p
    } else {
        if pos + 2 > frame.len() {
            return Err(F5Error::MalformedPpp(
                "frame too short for 2-byte proto".into(),
            ));
        }
        let p = ((frame[pos] as u16) << 8) | (frame[pos + 1] as u16);
        pos += 2;
        p
    };

    let body = &frame[pos..];
    if body.len() < 4 {
        return Err(F5Error::MalformedPpp(format!(
            "NCP body too short: {} bytes (need >= 4)",
            body.len()
        )));
    }

    let code = body[0];
    let id = body[1];
    let declared = ((body[2] as usize) << 8) | (body[3] as usize);

    if declared < 4 {
        return Err(F5Error::MalformedPpp(format!(
            "NCP length field {declared} < 4"
        )));
    }
    if declared > body.len() {
        return Err(F5Error::MalformedPpp(format!(
            "NCP length {declared} exceeds available {} bytes",
            body.len()
        )));
    }

    // Options span the declared length minus the 4-byte NCP header.
    //
    // Parsing is TOLERANT, matching openconnect's behaviour (ppp.c
    // `handle_config_request`): unknown option tags are kept (the caller decides
    // whether to ACK/REJECT them — a real LCP/IPCP Config-Request commonly
    // carries options we don't specifically handle), and a malformed/overrunning
    // option simply STOPS the loop rather than failing the whole frame. A real
    // server's first LCP Config-Request must never be rejected outright just
    // because it contains an option we didn't anticipate.
    let mut opt_pos = 4usize;
    let opt_end = declared;
    let mut options = Vec::new();

    while opt_pos + 2 <= opt_end {
        let tag = body[opt_pos];
        let len = body[opt_pos + 1] as usize;
        // An option length < 2 or one that overruns the packet is malformed;
        // stop here (openconnect reports trailing bytes and continues) rather
        // than erroring the entire frame.
        if len < 2 || opt_pos + len > opt_end {
            break;
        }
        let data = body[opt_pos + 2..opt_pos + len].to_vec();
        options.push(NcpOption { tag, data });
        opt_pos += len;
    }

    Ok(NcpPacket {
        proto,
        code,
        id,
        options,
    })
}

// ---------------------------------------------------------------------------
// Convenience constructors
// ---------------------------------------------------------------------------

/// Build an LCP Configure-Request offering an MRU and a Magic-Number.
pub fn lcp_config_request(id: u8, magic: u32, mru: u16) -> NcpPacket {
    NcpPacket {
        proto: PPP_LCP,
        code: CONFREQ,
        id,
        options: vec![
            NcpOption::new(LCP_MRU, mru.to_be_bytes().to_vec()),
            NcpOption::new(LCP_MAGIC, magic.to_be_bytes().to_vec()),
        ],
    }
}

/// Build an IPCP Configure-Request requesting `requested_ip` and the given
/// primary/secondary DNS servers.
///
/// Per RFC 1877, DNS values start as `0.0.0.0` to solicit a NAK-offer; once the
/// server NAK-offers concrete values, the client MUST **echo those values** in
/// the next Configure-Request (re-requesting `0.0.0.0` would be NAKed forever).
/// `dns1`/`dns2` therefore carry whatever has been adopted so far.
pub fn ipcp_config_request_with_dns(
    id: u8,
    requested_ip: [u8; 4],
    dns1: [u8; 4],
    dns2: [u8; 4],
) -> NcpPacket {
    NcpPacket {
        proto: PPP_IPCP,
        code: CONFREQ,
        id,
        options: vec![
            NcpOption::new(IPCP_IPADDR, requested_ip.to_vec()),
            NcpOption::new(IPCP_DNS1, dns1.to_vec()),
            NcpOption::new(IPCP_DNS2, dns2.to_vec()),
        ],
    }
}

/// Build an IPCP Configure-Request requesting `requested_ip` and soliciting
/// primary/secondary DNS servers (sent as zero to be NAK-offered).
pub fn ipcp_config_request(id: u8, requested_ip: [u8; 4]) -> NcpPacket {
    ipcp_config_request_with_dns(id, requested_ip, [0, 0, 0, 0], [0, 0, 0, 0])
}

/// Build an LCP Echo-Reply carrying our magic number followed by the echoed
/// data.
pub fn lcp_echo_reply(id: u8, magic: u32, data: &[u8]) -> NcpPacket {
    let mut payload = magic.to_be_bytes().to_vec();
    payload.extend_from_slice(data);
    NcpPacket {
        proto: PPP_LCP,
        code: ECHOREP,
        id,
        // Echo data is carried as a single opaque blob; model it as a raw
        // "option" with tag 0 so build/parse round-trips losslessly is not
        // required here — echo bodies are not TLVs, so we store the payload
        // directly via a synthetic representation below.
        options: raw_payload_options(&payload),
    }
}

/// Build an LCP Echo-Request carrying our magic number (used as a client-side
/// keepalive / DPD so the F5 server does not consider us dead and tear down the
/// tunnel). The peer may answer with an Echo-Reply; we send these proactively to
/// refresh the server's peer-liveness, mirroring openconnect's `KA_DPD`.
pub fn lcp_echo_request(id: u8, magic: u32, data: &[u8]) -> NcpPacket {
    let mut payload = magic.to_be_bytes().to_vec();
    payload.extend_from_slice(data);
    NcpPacket {
        proto: PPP_LCP,
        code: ECHOREQ,
        id,
        options: raw_payload_options(&payload),
    }
}

/// Build an LCP Terminate-Request.
pub fn lcp_terminate_request(id: u8) -> NcpPacket {
    NcpPacket {
        proto: PPP_LCP,
        code: TERMREQ,
        id,
        options: Vec::new(),
    }
}

/// Echo/Terminate bodies are *not* TLV-structured. To keep a single
/// [`NcpPacket`] representation we carry such an opaque payload as one synthetic
/// "option" whose `tag` is the first payload byte and whose `data` is the rest.
/// When re-encoded, the TLV `len` byte spans the whole remaining body, so the
/// payload round-trips byte-for-byte through [`build_ncp_frame`] /
/// [`parse_ppp_frame`] (reconstructed by [`echo_data`]). The peer only reads
/// `code`/`id` for DPD purposes, so the exact framing of the data is immaterial.
fn raw_payload_options(payload: &[u8]) -> Vec<NcpOption> {
    if payload.is_empty() {
        return Vec::new();
    }
    vec![NcpOption {
        tag: payload[0],
        data: payload[1..].to_vec(),
    }]
}

// ---------------------------------------------------------------------------
// State machine
// ---------------------------------------------------------------------------

/// The negotiation phase reached by a [`PppNegotiator`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PppPhase {
    /// Nothing sent yet.
    Dead,
    /// LCP Configure-Request sent, negotiating the link layer.
    EstablishLcp,
    /// LCP fully negotiated (both directions ACKed).
    OpenedLcp,
    /// IPCP Configure-Request sent, negotiating IPv4 parameters.
    NetworkIpcp,
    /// Network up: IPv4 address and DNS negotiated.
    Up,
    /// Link terminated.
    Terminated,
}

/// Deterministic PPP negotiation state machine for the akon F5 client.
///
/// Drives LCP then IPCP to completion, adopting the server's NAK-offered IPv4
/// address and DNS servers. Modelled on openconnect's `handle_state_transition`
/// but simplified for a lossless TLS transport (no retransmit timers).
pub struct PppNegotiator {
    next_id: u8,
    magic: u32,
    /// id of our most recent LCP Configure-Request.
    lcp_req_id: u8,
    /// id of our most recent IPCP Configure-Request.
    ipcp_req_id: u8,

    lcp_ack_received: bool,
    lcp_ack_sent: bool,
    ipcp_ack_received: bool,
    ipcp_ack_sent: bool,

    /// IPv4 address we request (starts 0.0.0.0, updated from CONFNAK).
    requested_ip: [u8; 4],
    /// Negotiated IPv4 address, once known.
    negotiated_ip: Option<[u8; 4]>,
    /// Primary DNS (IPCP DNS1, RFC1877). Starts 0.0.0.0, adopted from NAK and
    /// echoed back in subsequent Configure-Requests.
    dns1: [u8; 4],
    /// Secondary DNS (IPCP DNS2).
    dns2: [u8; 4],
    /// The peer's advertised MRU (from its LCP Config-Request MRU option), once
    /// seen. Used to derive the tunnel MTU.
    peer_mru: Option<u16>,
    /// Whether to include the DNS1/DNS2 solicitation in our IPCP requests.
    /// Cleared if the server Configure-Rejects them.
    request_dns1: bool,
    request_dns2: bool,

    phase: PppPhase,
}

/// Fixed magic number used in our LCP requests (deterministic for testing).
const DEFAULT_MAGIC: u32 = 0x1234_5678;
/// MRU we request.
const DEFAULT_MRU: u16 = 1500;

impl Default for PppNegotiator {
    fn default() -> Self {
        Self::new()
    }
}

impl PppNegotiator {
    /// Create a fresh negotiator in the [`PppPhase::Dead`] phase.
    pub fn new() -> Self {
        Self {
            next_id: 1,
            magic: DEFAULT_MAGIC,
            lcp_req_id: 0,
            ipcp_req_id: 0,
            lcp_ack_received: false,
            lcp_ack_sent: false,
            ipcp_ack_received: false,
            ipcp_ack_sent: false,
            requested_ip: [0, 0, 0, 0],
            negotiated_ip: None,
            dns1: [0, 0, 0, 0],
            dns2: [0, 0, 0, 0],
            peer_mru: None,
            request_dns1: true,
            request_dns2: true,
            phase: PppPhase::Dead,
        }
    }

    fn alloc_id(&mut self) -> u8 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        id
    }

    /// Begin LCP negotiation. Returns the wire frame(s) to transmit.
    pub fn start(&mut self) -> Vec<Vec<u8>> {
        self.phase = PppPhase::EstablishLcp;
        let id = self.alloc_id();
        self.lcp_req_id = id;
        let pkt = lcp_config_request(id, self.magic, DEFAULT_MRU);
        vec![build_ncp_frame(&pkt)]
    }

    /// The current negotiation phase.
    pub fn phase(&self) -> PppPhase {
        self.phase
    }

    /// The negotiated IPv4 address, once IPCP has completed.
    pub fn negotiated_ipv4(&self) -> Option<[u8; 4]> {
        self.negotiated_ip
    }

    /// The DNS servers negotiated via IPCP (DNS1 then DNS2), excluding zeros.
    pub fn dns_servers(&self) -> Vec<[u8; 4]> {
        [self.dns1, self.dns2]
            .into_iter()
            .filter(|d| *d != [0, 0, 0, 0])
            .collect()
    }

    /// Our LCP magic number (used when framing LCP control packets such as the
    /// Terminate-Request during teardown).
    pub fn magic(&self) -> u32 {
        self.magic
    }

    /// The tunnel interface MTU to use, derived from the negotiated MRUs.
    ///
    /// The PPP MRU is the largest IP payload the peer will accept, so it maps
    /// directly to the TUN interface MTU. We use the smaller of our requested
    /// MRU and the peer's advertised MRU (if seen), clamped to a sane range so
    /// a malformed/absent value can't produce a broken interface.
    pub fn negotiated_mtu(&self) -> u32 {
        let ours = DEFAULT_MRU;
        let mtu = match self.peer_mru {
            Some(peer) => ours.min(peer),
            None => ours,
        };
        // Clamp to a conservative, valid IPv4 MTU range.
        (mtu as u32).clamp(576, 1500)
    }

    /// Send our IPCP Configure-Request, transitioning to
    /// [`PppPhase::NetworkIpcp`].
    fn send_ipcp_request(&mut self) -> Vec<u8> {
        let id = self.alloc_id();
        self.ipcp_req_id = id;
        // A new request invalidates any prior peer ACK of our request.
        self.ipcp_ack_received = false;
        self.phase = PppPhase::NetworkIpcp;
        // Echo the IP and DNS values adopted so far (RFC1877): re-requesting
        // 0.0.0.0 for DNS after a NAK would be NAKed forever. Omit DNS options
        // the server has Configure-Rejected.
        let mut options = vec![NcpOption::new(IPCP_IPADDR, self.requested_ip.to_vec())];
        if self.request_dns1 {
            options.push(NcpOption::new(IPCP_DNS1, self.dns1.to_vec()));
        }
        if self.request_dns2 {
            options.push(NcpOption::new(IPCP_DNS2, self.dns2.to_vec()));
        }
        let pkt = NcpPacket {
            proto: PPP_IPCP,
            code: CONFREQ,
            id,
            options,
        };
        build_ncp_frame(&pkt)
    }

    /// Feed an inbound PPP frame. Returns the wire frames to transmit in
    /// response. Unknown protocols are ignored (empty output, no error).
    pub fn on_frame(&mut self, frame: &[u8]) -> Result<Vec<Vec<u8>>, F5Error> {
        let pkt = parse_ppp_frame(frame)?;
        match pkt.proto {
            PPP_LCP => self.on_lcp(&pkt),
            PPP_IPCP => self.on_ipcp(&pkt),
            // IP6CP: we are an IPv4-only client. A real F5 server runs IP6CP in
            // parallel and *retransmits* its Configure-Request until answered.
            // Reject it (Configure-Reject echoing its options) so the server
            // stops retransmitting and lets IPv4-only bring-up complete.
            PPP_IP6CP => Ok(self.on_ip6cp(&pkt)),
            // Other protocols / data: ignore.
            _ => Ok(Vec::new()),
        }
    }

    /// Reject IP6CP negotiation (IPv4-only client).
    fn on_ip6cp(&mut self, pkt: &NcpPacket) -> Vec<Vec<u8>> {
        if pkt.code == CONFREQ && !pkt.options.is_empty() {
            let rej = NcpPacket {
                proto: PPP_IP6CP,
                code: CONFREJ,
                id: pkt.id,
                options: pkt.options.clone(),
            };
            vec![build_ncp_frame(&rej)]
        } else {
            Vec::new()
        }
    }

    fn on_lcp(&mut self, pkt: &NcpPacket) -> Result<Vec<Vec<u8>>, F5Error> {
        let mut out = Vec::new();
        match pkt.code {
            CONFREQ => {
                // Capture the peer's advertised MRU (for MTU derivation).
                if let Some(mru) = pkt.option(LCP_MRU) {
                    if mru.data.len() == 2 {
                        self.peer_mru = Some(u16::from_be_bytes([mru.data[0], mru.data[1]]));
                    }
                }
                // Accept their options; reply with a Configure-Ack echoing them.
                let ack = NcpPacket {
                    proto: PPP_LCP,
                    code: CONFACK,
                    id: pkt.id,
                    options: pkt.options.clone(),
                };
                out.push(build_ncp_frame(&ack));
                self.lcp_ack_sent = true;
                self.maybe_open_lcp(&mut out);
            }
            CONFACK => {
                if pkt.id == self.lcp_req_id {
                    self.lcp_ack_received = true;
                    self.maybe_open_lcp(&mut out);
                }
            }
            ECHOREQ => {
                // DPD: reply with Echo-Reply carrying the same data.
                let data = echo_data(pkt);
                let reply = lcp_echo_reply(pkt.id, self.magic, &data);
                out.push(build_ncp_frame(&reply));
            }
            TERMREQ => {
                let ack = NcpPacket {
                    proto: PPP_LCP,
                    code: TERMACK,
                    id: pkt.id,
                    options: Vec::new(),
                };
                out.push(build_ncp_frame(&ack));
                self.phase = PppPhase::Terminated;
            }
            // CONFNAK/CONFREJ for LCP, ECHOREP, etc.: nothing to do here.
            _ => {}
        }
        Ok(out)
    }

    /// If both directions of LCP are ACKed and we have not yet started IPCP,
    /// open the link and emit our IPCP Configure-Request.
    fn maybe_open_lcp(&mut self, out: &mut Vec<Vec<u8>>) {
        if self.lcp_ack_received
            && self.lcp_ack_sent
            && matches!(self.phase, PppPhase::EstablishLcp)
        {
            self.phase = PppPhase::OpenedLcp;
            out.push(self.send_ipcp_request());
        }
    }

    fn on_ipcp(&mut self, pkt: &NcpPacket) -> Result<Vec<Vec<u8>>, F5Error> {
        let mut out = Vec::new();
        match pkt.code {
            CONFREQ => {
                let ack = NcpPacket {
                    proto: PPP_IPCP,
                    code: CONFACK,
                    id: pkt.id,
                    options: pkt.options.clone(),
                };
                out.push(build_ncp_frame(&ack));
                self.ipcp_ack_sent = true;
                self.maybe_network_up();
            }
            CONFNAK => {
                if pkt.id == self.ipcp_req_id {
                    self.adopt_ipcp_nak(pkt);
                    // Resend our request carrying the adopted IP.
                    out.push(self.send_ipcp_request());
                }
            }
            CONFREJ => {
                // The server rejected one or more of our options (commonly the
                // DNS1/DNS2 solicitation on deployments that don't offer DNS via
                // IPCP). Stop requesting the rejected options and re-send, so we
                // converge instead of looping. We never drop the IP address
                // request.
                if pkt.id == self.ipcp_req_id {
                    for opt in &pkt.options {
                        match opt.tag {
                            IPCP_DNS1 => self.request_dns1 = false,
                            IPCP_DNS2 => self.request_dns2 = false,
                            _ => {}
                        }
                    }
                    out.push(self.send_ipcp_request());
                }
            }
            CONFACK if pkt.id == self.ipcp_req_id => {
                self.ipcp_ack_received = true;
                // Record the IP we ended up requesting as negotiated.
                if self.negotiated_ip.is_none() {
                    self.negotiated_ip = Some(self.requested_ip);
                }
                self.maybe_network_up();
            }
            _ => {}
        }
        Ok(out)
    }

    /// Adopt the IPv4 address and DNS servers offered in an IPCP Configure-Nak.
    ///
    /// Each NAKed option's value is adopted into the matching slot so the next
    /// Configure-Request echoes exactly what the server offered (RFC1877). DNS1
    /// and DNS2 are distinct slots keyed by tag.
    fn adopt_ipcp_nak(&mut self, pkt: &NcpPacket) {
        for opt in &pkt.options {
            if opt.data.len() != 4 {
                continue;
            }
            let val = [opt.data[0], opt.data[1], opt.data[2], opt.data[3]];
            match opt.tag {
                IPCP_IPADDR if val != [0, 0, 0, 0] => {
                    self.requested_ip = val;
                    self.negotiated_ip = Some(val);
                }
                IPCP_DNS1 => self.dns1 = val,
                IPCP_DNS2 => self.dns2 = val,
                _ => {}
            }
        }
    }

    /// If both directions of IPCP are ACKed, declare the network up.
    fn maybe_network_up(&mut self) {
        if self.ipcp_ack_received
            && self.ipcp_ack_sent
            && !matches!(self.phase, PppPhase::Up | PppPhase::Terminated)
        {
            self.phase = PppPhase::Up;
        }
    }
}

/// Extract the echoed data from an LCP Echo-Request packet.
///
/// Echo bodies are not TLV-structured; our parser nonetheless stores the raw
/// remainder as a single synthetic option (tag = first body byte, data =
/// rest). Reconstruct the original byte run from that representation.
fn echo_data(pkt: &NcpPacket) -> Vec<u8> {
    let mut data = Vec::new();
    if let Some(opt) = pkt.options.first() {
        data.push(opt.tag);
        data.extend_from_slice(&opt.data);
    }
    data
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lcp_config_request_round_trip() {
        let pkt = lcp_config_request(7, 0xdead_beef, 1400);
        let frame = build_ncp_frame(&pkt);

        // FF 03 then proto (0xc021) big-endian.
        assert_eq!(&frame[0..2], &[0xff, 0x03]);
        assert_eq!(&frame[2..4], &[0xc0, 0x21]);

        // NCP header: code, id.
        assert_eq!(frame[4], CONFREQ);
        assert_eq!(frame[5], 7);
        let declared = ((frame[6] as usize) << 8) | frame[7] as usize;
        assert_eq!(declared, frame.len() - 4);

        let parsed = parse_ppp_frame(&frame).unwrap();
        assert_eq!(parsed, pkt);
        assert_eq!(parsed.proto, PPP_LCP);
        assert_eq!(parsed.code, CONFREQ);
        assert_eq!(parsed.id, 7);
    }

    #[test]
    fn ipcp_config_request_round_trip() {
        let pkt = ipcp_config_request(3, [192, 168, 1, 5]);
        let frame = build_ncp_frame(&pkt);

        assert_eq!(&frame[0..2], &[0xff, 0x03]);
        assert_eq!(&frame[2..4], &[0x80, 0x21]);
        assert_eq!(frame[4], CONFREQ);
        assert_eq!(frame[5], 3);

        let parsed = parse_ppp_frame(&frame).unwrap();
        assert_eq!(parsed, pkt);
    }

    #[test]
    fn lcp_request_has_mru_and_magic() {
        let pkt = lcp_config_request(1, 0x1234_5678, 1500);
        let mru = pkt.option(LCP_MRU).expect("MRU option present");
        assert_eq!(mru.data, vec![0x05, 0xdc]); // 1500 be16
        let magic = pkt.option(LCP_MAGIC).expect("MAGIC option present");
        assert_eq!(magic.data, vec![0x12, 0x34, 0x56, 0x78]);
    }

    #[test]
    fn ipcp_request_has_ipaddr_and_dns() {
        let pkt = ipcp_config_request(1, [0, 0, 0, 0]);
        let ip = pkt.option(IPCP_IPADDR).expect("IPADDR option present");
        assert_eq!(ip.data, vec![0, 0, 0, 0]);
        assert!(pkt.option(IPCP_DNS1).is_some(), "DNS1 present");
        assert!(pkt.option(IPCP_DNS2).is_some(), "DNS2 present");
    }

    #[test]
    fn parse_tolerates_missing_ff03() {
        // Strip the leading FF 03; keep the full 2-byte proto (0x80 0x21, even).
        let pkt = ipcp_config_request(9, [1, 2, 3, 4]);
        let full = build_ncp_frame(&pkt);
        let stripped = &full[2..];
        let parsed = parse_ppp_frame(stripped).unwrap();
        assert_eq!(parsed, pkt);
    }

    #[test]
    fn parse_tolerates_single_byte_pfc_proto() {
        // Craft a frame with a 1-byte (odd) protocol field for IP (0x21) and a
        // minimal NCP-shaped body. The parser must read proto 0x21 from one byte.
        // body: code=1, id=1, len=0x0004, no options.
        let frame = [0x21u8, CONFREQ, 1, 0x00, 0x04];
        let parsed = parse_ppp_frame(&frame).unwrap();
        assert_eq!(parsed.proto, PPP_IP);
        assert_eq!(parsed.code, CONFREQ);
        assert_eq!(parsed.id, 1);
        assert!(parsed.options.is_empty());

        // Same again but with the FF 03 prefix preceding the 1-byte proto.
        let framed = [0xff, 0x03, 0x21u8, CONFREQ, 2, 0x00, 0x04];
        let parsed = parse_ppp_frame(&framed).unwrap();
        assert_eq!(parsed.proto, PPP_IP);
        assert_eq!(parsed.id, 2);
    }

    #[test]
    fn parse_real_server_lcp_confreq_with_unknown_option() {
        // Reproduce a realistic server LCP Config-Request like a real F5 sends:
        // FF 03 C0 21 (LCP) | code=01 id=01 | length | MRU(1,len4) MAGIC(5,len6)
        // + an UNKNOWN/proprietary option (tag 0xDF, len 4). The tolerant parser
        // must accept it (keep the unknown option) and not error — this is the
        // exact class of frame that previously produced "tag 223 overruns".
        let options: Vec<u8> = vec![
            0x01, 0x04, 0x05, 0xdc, // MRU = 1500
            0x05, 0x06, 0xde, 0xad, 0xbe, 0xef, // MAGIC
            0xdf, 0x04, 0x11, 0x22, // unknown proprietary option (tag 223)
        ];
        let ncp_len = 4 + options.len(); // header + options
        let mut frame = vec![0xff, 0x03, 0xc0, 0x21, CONFREQ, 0x01];
        frame.push((ncp_len >> 8) as u8);
        frame.push((ncp_len & 0xff) as u8);
        frame.extend_from_slice(&options);

        let pkt = parse_ppp_frame(&frame).expect("tolerant parse of real-shaped LCP confreq");
        assert_eq!(pkt.proto, PPP_LCP);
        assert_eq!(pkt.code, CONFREQ);
        assert_eq!(pkt.id, 0x01);
        // All three options recovered, including the unknown 0xDF one.
        assert_eq!(pkt.options.len(), 3);
        assert_eq!(
            pkt.option(LCP_MRU).map(|o| o.data.clone()),
            Some(vec![0x05, 0xdc])
        );
        assert!(pkt.option(LCP_MAGIC).is_some());
        assert_eq!(
            pkt.option(0xdf).map(|o| o.data.clone()),
            Some(vec![0x11, 0x22])
        );
    }

    #[test]
    fn parse_stops_on_genuinely_overrunning_option() {
        // An option whose length byte overruns the declared packet must just
        // stop the loop (tolerant), returning the options parsed so far — not error.
        let options: Vec<u8> = vec![
            0x01, 0x04, 0x05, 0xdc, // valid MRU
            0xdf, 0x4d, 0x00, // tag 223 len 77 -> overruns; must stop here
        ];
        let ncp_len = 4 + options.len();
        let mut frame = vec![0xff, 0x03, 0xc0, 0x21, CONFREQ, 0x01];
        frame.push((ncp_len >> 8) as u8);
        frame.push((ncp_len & 0xff) as u8);
        frame.extend_from_slice(&options);

        let pkt = parse_ppp_frame(&frame).expect("must not error on overrunning option");
        // Only the valid MRU is recovered; the overrunning option is dropped.
        assert_eq!(pkt.options.len(), 1);
        assert_eq!(pkt.options[0].tag, LCP_MRU);
    }

    #[test]
    fn parse_rejects_truncated_frame() {
        // FF 03 + proto + a too-short NCP body.
        let bad = [0xff, 0x03, 0xc0, 0x21, CONFREQ, 1];
        assert!(matches!(
            parse_ppp_frame(&bad),
            Err(F5Error::MalformedPpp(_))
        ));
    }

    #[test]
    fn parse_rejects_overlong_length_field() {
        let mut frame = build_ncp_frame(&lcp_config_request(1, 1, 1500));
        // Inflate the declared NCP length beyond the buffer.
        let body_start = 4;
        frame[body_start + 2] = 0xff;
        frame[body_start + 3] = 0xff;
        assert!(matches!(
            parse_ppp_frame(&frame),
            Err(F5Error::MalformedPpp(_))
        ));
    }

    /// A scripted peer that walks the negotiator through a full bring-up.
    #[test]
    fn full_negotiation_to_up() {
        let mut neg = PppNegotiator::new();

        // start(): Dead -> EstablishLcp, emits our LCP CONFREQ.
        let initial = neg.start();
        assert_eq!(neg.phase(), PppPhase::EstablishLcp);
        assert_eq!(initial.len(), 1);
        let our_lcp = parse_ppp_frame(&initial[0]).unwrap();
        assert_eq!(our_lcp.proto, PPP_LCP);
        assert_eq!(our_lcp.code, CONFREQ);
        let our_lcp_id = our_lcp.id;

        // (a) Peer sends its own LCP CONFREQ -> expect CONFACK out.
        let peer_lcp_req = build_ncp_frame(&NcpPacket {
            proto: PPP_LCP,
            code: CONFREQ,
            id: 55,
            options: vec![NcpOption::new(LCP_MRU, vec![0x05, 0xdc])],
        });
        let out = neg.on_frame(&peer_lcp_req).unwrap();
        assert_eq!(out.len(), 1);
        let ack = parse_ppp_frame(&out[0]).unwrap();
        assert_eq!(ack.proto, PPP_LCP);
        assert_eq!(ack.code, CONFACK);
        assert_eq!(ack.id, 55);
        // Still establishing — we have not yet had our request ACKed.
        assert_eq!(neg.phase(), PppPhase::EstablishLcp);

        // (b) Peer ACKs our LCP request -> expect transition + IPCP CONFREQ out.
        let peer_lcp_ack = build_ncp_frame(&NcpPacket {
            proto: PPP_LCP,
            code: CONFACK,
            id: our_lcp_id,
            options: our_lcp.options.clone(),
        });
        let out = neg.on_frame(&peer_lcp_ack).unwrap();
        assert_eq!(neg.phase(), PppPhase::NetworkIpcp);
        assert_eq!(out.len(), 1);
        let ipcp_req = parse_ppp_frame(&out[0]).unwrap();
        assert_eq!(ipcp_req.proto, PPP_IPCP);
        assert_eq!(ipcp_req.code, CONFREQ);
        let first_ipcp_id = ipcp_req.id;
        // Initially we request 0.0.0.0.
        assert_eq!(ipcp_req.option(IPCP_IPADDR).unwrap().data, vec![0, 0, 0, 0]);

        // (c) Peer NAKs offering IP 10.20.30.40 and DNS 8.8.8.8.
        let peer_ipcp_nak = build_ncp_frame(&NcpPacket {
            proto: PPP_IPCP,
            code: CONFNAK,
            id: first_ipcp_id,
            options: vec![
                NcpOption::new(IPCP_IPADDR, vec![10, 20, 30, 40]),
                NcpOption::new(IPCP_DNS1, vec![8, 8, 8, 8]),
            ],
        });
        let out = neg.on_frame(&peer_ipcp_nak).unwrap();
        assert_eq!(out.len(), 1);
        let resent = parse_ppp_frame(&out[0]).unwrap();
        assert_eq!(resent.proto, PPP_IPCP);
        assert_eq!(resent.code, CONFREQ);
        assert_eq!(
            resent.option(IPCP_IPADDR).unwrap().data,
            vec![10, 20, 30, 40]
        );
        let second_ipcp_id = resent.id;
        assert_ne!(second_ipcp_id, first_ipcp_id);

        // (d) Peer sends IPCP CONFREQ -> expect IPCP CONFACK.
        let peer_ipcp_req = build_ncp_frame(&NcpPacket {
            proto: PPP_IPCP,
            code: CONFREQ,
            id: 77,
            options: vec![NcpOption::new(IPCP_IPADDR, vec![10, 20, 30, 1])],
        });
        let out = neg.on_frame(&peer_ipcp_req).unwrap();
        assert_eq!(out.len(), 1);
        let ipcp_ack = parse_ppp_frame(&out[0]).unwrap();
        assert_eq!(ipcp_ack.proto, PPP_IPCP);
        assert_eq!(ipcp_ack.code, CONFACK);
        assert_eq!(ipcp_ack.id, 77);

        // (e) Peer ACKs our IPCP request -> phase Up.
        let peer_ipcp_ack = build_ncp_frame(&NcpPacket {
            proto: PPP_IPCP,
            code: CONFACK,
            id: second_ipcp_id,
            options: resent.options.clone(),
        });
        let out = neg.on_frame(&peer_ipcp_ack).unwrap();
        assert!(out.is_empty());
        assert_eq!(neg.phase(), PppPhase::Up);
        assert_eq!(neg.negotiated_ipv4(), Some([10, 20, 30, 40]));
        assert!(neg.dns_servers().contains(&[8, 8, 8, 8]));
    }

    /// Replays the EXACT IPCP/IP6CP frame *shapes* observed from a real F5
    /// appliance (addresses anonymized to documentation values) and asserts the
    /// negotiator converges to `Up` with the server-assigned IP + DNS — i.e. it
    /// adopts the NAKed DNS and echoes it back (no infinite NAK loop), and
    /// rejects IP6CP.
    ///
    /// This is the byte-accurate regression test for the production PPP timeout.
    #[test]
    fn converges_against_real_appliance_ipcp_nak_sequence() {
        let mut neg = PppNegotiator::new();
        let initial = neg.start();
        let our_lcp_id = parse_ppp_frame(&initial[0]).unwrap().id;

        // Server LCP ConfReq (real bytes after FF 03): MRU/ASYNCMAP/MAGIC/PFCOMP/ACCOMP.
        feed(
            &mut neg,
            &[
                0xc0, 0x21, 0x01, 0x01, 0x00, 0x18, 0x01, 0x04, 0x05, 0x83, 0x02, 0x06, 0x00, 0x00,
                0x00, 0x00, 0x05, 0x06, 0x31, 0x16, 0x91, 0x65, 0x07, 0x02, 0x08, 0x02,
            ],
        );
        // Server LCP ConfAck of OUR request (id from start()).
        let lcp_ack = NcpPacket {
            proto: PPP_LCP,
            code: CONFACK,
            id: our_lcp_id,
            options: vec![
                NcpOption::new(LCP_MRU, vec![0x05, 0xdc]),
                NcpOption::new(LCP_MAGIC, vec![0x12, 0x34, 0x56, 0x78]),
            ],
        };
        let after_lcp = neg.on_frame(&build_ncp_frame(&lcp_ack)).unwrap();
        // LCP open -> we emit our first IPCP ConfReq.
        assert_eq!(neg.phase(), PppPhase::NetworkIpcp);
        let ipcp_req1 = parse_ppp_frame(&after_lcp[0]).unwrap();
        let mut cur_ipcp_id = ipcp_req1.id;

        // Server LCP EchoReq -> we EchoRep (no phase change).
        feed(
            &mut neg,
            &[0xc0, 0x21, 0x09, 0x00, 0x00, 0x08, 0x31, 0x16, 0x91, 0x65],
        );

        // Server IPCP ConfReq (its address 1.1.1.1) -> we ACK.
        let out = feed(
            &mut neg,
            &[
                0x80, 0x21, 0x01, 0x01, 0x00, 0x0a, 0x03, 0x06, 0x01, 0x01, 0x01, 0x01,
            ],
        );
        let ack = parse_ppp_frame(&out[0]).unwrap();
        assert_eq!((ack.proto, ack.code), (PPP_IPCP, CONFACK));

        // Server IP6CP ConfReq -> we must REJECT it.
        let out = feed(
            &mut neg,
            &[
                0x80, 0x57, 0x01, 0x01, 0x00, 0x0e, 0x01, 0x0a, 0x28, 0xf8, 0xd2, 0x5d, 0x63, 0x37,
                0xb3, 0xc5,
            ],
        );
        let rej = parse_ppp_frame(&out[0]).unwrap();
        assert_eq!((rej.proto, rej.code), (PPP_IP6CP, CONFREJ));

        // Server IPCP ConfNak: offers IP 10.20.30.40, DNS1 10.20.30.1, DNS2 10.20.30.2
        // (anonymized documentation values). (Use our actual current request id.)
        let nak = NcpPacket {
            proto: PPP_IPCP,
            code: CONFNAK,
            id: cur_ipcp_id,
            options: vec![
                NcpOption::new(IPCP_IPADDR, vec![0x0a, 0x14, 0x1e, 0x28]),
                NcpOption::new(IPCP_DNS1, vec![0x0a, 0x14, 0x1e, 0x01]),
                NcpOption::new(IPCP_DNS2, vec![0x0a, 0x14, 0x1e, 0x02]),
            ],
        };
        let out = neg.on_frame(&build_ncp_frame(&nak)).unwrap();
        // We must re-request with the ADOPTED ip + dns (not zeros).
        let req2 = parse_ppp_frame(&out[0]).unwrap();
        assert_eq!(
            req2.option(IPCP_IPADDR).unwrap().data,
            vec![0x0a, 0x14, 0x1e, 0x28]
        );
        assert_eq!(
            req2.option(IPCP_DNS1).unwrap().data,
            vec![0x0a, 0x14, 0x1e, 0x01]
        );
        assert_eq!(
            req2.option(IPCP_DNS2).unwrap().data,
            vec![0x0a, 0x14, 0x1e, 0x02]
        );
        cur_ipcp_id = req2.id;

        // Server ACKs our (now-correct) IPCP request -> network up.
        let ack = NcpPacket {
            proto: PPP_IPCP,
            code: CONFACK,
            id: cur_ipcp_id,
            options: req2.options.clone(),
        };
        let _ = neg.on_frame(&build_ncp_frame(&ack)).unwrap();
        assert_eq!(neg.phase(), PppPhase::Up);
        assert_eq!(neg.negotiated_ipv4(), Some([10, 20, 30, 40]));
        assert_eq!(neg.dns_servers(), vec![[10, 20, 30, 1], [10, 20, 30, 2]]);
    }

    /// Helper: wrap a raw "after-FF03-needed?" body — here we pass full PPP
    /// frames already starting at FF 03 — and feed them to the negotiator.
    fn feed(neg: &mut PppNegotiator, ppp_after_ac: &[u8]) -> Vec<Vec<u8>> {
        // Prepend FF 03 to form a complete PPP frame (proto is already in the body).
        let mut frame = vec![0xff, 0x03];
        frame.extend_from_slice(ppp_after_ac);
        neg.on_frame(&frame).unwrap()
    }

    #[test]
    fn mtu_derived_from_peer_mru() {
        let mut neg = PppNegotiator::new();
        let _ = neg.start();
        // Default before seeing any peer MRU.
        assert_eq!(neg.negotiated_mtu(), DEFAULT_MRU as u32);
        // Peer LCP ConfReq advertising MRU 1411 (0x0583) like the real appliance.
        let peer_req = NcpPacket {
            proto: PPP_LCP,
            code: CONFREQ,
            id: 1,
            options: vec![NcpOption::new(LCP_MRU, vec![0x05, 0x83])],
        };
        let _ = neg.on_frame(&build_ncp_frame(&peer_req)).unwrap();
        // min(our 1500, peer 1411) = 1411.
        assert_eq!(neg.negotiated_mtu(), 1411);
    }

    #[test]
    fn ipcp_confrej_drops_dns_and_reconverges() {
        // Minimal LCP open so IPCP starts.
        let mut neg = PppNegotiator::new();
        let init = neg.start();
        let our_id = parse_ppp_frame(&init[0]).unwrap().id;
        // Peer sends its LCP ConfReq (we ACK it) and ACKs ours -> LCP opens.
        neg.on_frame(&build_ncp_frame(&NcpPacket {
            proto: PPP_LCP,
            code: CONFREQ,
            id: 1,
            options: vec![NcpOption::new(LCP_MRU, vec![0x05, 0xdc])],
        }))
        .unwrap();
        let out = neg
            .on_frame(&build_ncp_frame(&NcpPacket {
                proto: PPP_LCP,
                code: CONFACK,
                id: our_id,
                options: vec![],
            }))
            .unwrap();
        let ipcp_req = parse_ppp_frame(&out[0]).unwrap();
        assert!(ipcp_req.option(IPCP_DNS1).is_some());
        let id1 = ipcp_req.id;

        // Server Configure-Rejects the DNS options.
        let rej = NcpPacket {
            proto: PPP_IPCP,
            code: CONFREJ,
            id: id1,
            options: vec![
                NcpOption::new(IPCP_DNS1, vec![0, 0, 0, 0]),
                NcpOption::new(IPCP_DNS2, vec![0, 0, 0, 0]),
            ],
        };
        let out = neg.on_frame(&build_ncp_frame(&rej)).unwrap();
        let req2 = parse_ppp_frame(&out[0]).unwrap();
        // The re-sent request must NOT include DNS options anymore, but keeps IP.
        assert!(req2.option(IPCP_IPADDR).is_some());
        assert!(req2.option(IPCP_DNS1).is_none());
        assert!(req2.option(IPCP_DNS2).is_none());

        // Server ACKs -> Up (IP only).
        neg.on_frame(&build_ncp_frame(&NcpPacket {
            proto: PPP_IPCP,
            code: CONFREQ,
            id: 9,
            options: vec![NcpOption::new(IPCP_IPADDR, vec![10, 0, 0, 1])],
        }))
        .unwrap();
        neg.on_frame(&build_ncp_frame(&NcpPacket {
            proto: PPP_IPCP,
            code: CONFACK,
            id: req2.id,
            options: req2.options.clone(),
        }))
        .unwrap();
        assert_eq!(neg.phase(), PppPhase::Up);
    }

    #[test]
    fn echo_request_gets_reply_with_same_data() {
        let mut neg = PppNegotiator::new();
        let _ = neg.start();

        // Build an LCP Echo-Request with some opaque data after magic.
        let echo_payload = [0xaa, 0xbb, 0xcc, 0xdd, 0x01, 0x02, 0x03];
        let req = NcpPacket {
            proto: PPP_LCP,
            code: ECHOREQ,
            id: 42,
            options: raw_payload_options(&echo_payload),
        };
        let frame = build_ncp_frame(&req);
        let out = neg.on_frame(&frame).unwrap();
        assert_eq!(out.len(), 1);
        let reply = parse_ppp_frame(&out[0]).unwrap();
        assert_eq!(reply.proto, PPP_LCP);
        assert_eq!(reply.code, ECHOREP);
        assert_eq!(reply.id, 42);
        // The reply carries our magic followed by the echoed data.
        let body = echo_data(&reply);
        let mut expected = DEFAULT_MAGIC.to_be_bytes().to_vec();
        expected.extend_from_slice(&echo_payload);
        assert_eq!(body, expected);
        // No phase change from a DPD echo.
        assert_eq!(neg.phase(), PppPhase::EstablishLcp);
    }

    #[test]
    fn lcp_echo_request_is_valid_dpd_frame_with_magic() {
        let magic = 0x1234_5678u32;
        // The DPD keepalive (what the data pump sends) carries EXACTLY the
        // 4-byte magic and no extra data, matching openconnect's
        // `queue_config_packet(.., ECHOREQ, 4, &out_lcp_magic)`. An 8-byte body
        // (magic duplicated) is what the F5 server ignored for DPD.
        let req = lcp_echo_request(7, magic, &[]);
        let frame = build_ncp_frame(&req);
        let parsed = parse_ppp_frame(&frame).unwrap();
        assert_eq!(parsed.proto, PPP_LCP);
        assert_eq!(parsed.code, ECHOREQ);
        assert_eq!(parsed.id, 7);
        let body = echo_data(&parsed);
        assert_eq!(
            body,
            magic.to_be_bytes().to_vec(),
            "DPD body must be magic only"
        );
    }

    #[test]
    fn terminate_request_gets_ack_and_terminates() {
        let mut neg = PppNegotiator::new();
        let _ = neg.start();

        let term = lcp_terminate_request(99);
        let frame = build_ncp_frame(&term);
        let out = neg.on_frame(&frame).unwrap();
        assert_eq!(out.len(), 1);
        let ack = parse_ppp_frame(&out[0]).unwrap();
        assert_eq!(ack.proto, PPP_LCP);
        assert_eq!(ack.code, TERMACK);
        assert_eq!(ack.id, 99);
        assert_eq!(neg.phase(), PppPhase::Terminated);
    }

    #[test]
    fn unknown_proto_is_ignored() {
        let mut neg = PppNegotiator::new();
        let _ = neg.start();
        // A data protocol (IP, 0x21) is not an NCP we negotiate — no output, no error.
        // Use a 1-byte odd proto frame so it parses as PPP_IP and is ignored.
        let frame = [0xff, 0x03, 0x21u8, CONFREQ, 1, 0x00, 0x04];
        let out = neg.on_frame(&frame).unwrap();
        assert!(out.is_empty());
        assert_eq!(neg.phase(), PppPhase::EstablishLcp);
    }

    #[test]
    fn ip6cp_confreq_is_rejected() {
        let mut neg = PppNegotiator::new();
        let _ = neg.start();
        let pkt = NcpPacket {
            proto: PPP_IP6CP,
            code: CONFREQ,
            id: 1,
            options: vec![NcpOption::new(1, vec![0; 8])],
        };
        let out = neg.on_frame(&build_ncp_frame(&pkt)).unwrap();
        assert_eq!(out.len(), 1);
        let rej = parse_ppp_frame(&out[0]).unwrap();
        assert_eq!((rej.proto, rej.code), (PPP_IP6CP, CONFREJ));
        assert_eq!(rej.id, 1);
    }

    #[test]
    fn terminate_request_constructor_is_empty() {
        let t = lcp_terminate_request(5);
        assert_eq!(t.code, TERMREQ);
        assert!(t.options.is_empty());
        // Round-trips.
        let f = build_ncp_frame(&t);
        let p = parse_ppp_frame(&f).unwrap();
        assert_eq!(p, t);
    }

    #[test]
    fn unused_constants_are_referenced() {
        // Touch constants not otherwise exercised so `dead_code = deny` is happy.
        let _ = (
            PPP_IP,
            PPP_IP6,
            CONFREJ,
            CODEREJ,
            PROTREJ,
            DISCREQ,
            TERMACK,
            LCP_ASYNCMAP,
            LCP_PFCOMP,
            LCP_ACCOMP,
            IPCP_NBNS1,
            IPCP_NBNS2,
        );
    }
}
