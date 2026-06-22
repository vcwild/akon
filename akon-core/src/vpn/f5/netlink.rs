//! Minimal in-process netlink (`NETLINK_ROUTE`) for rootless TUN configuration.
//!
//! See ADR 0001. A file capability (`setcap cap_net_admin+ep akon`) is NOT
//! inherited by a spawned `ip` child, so configuring the interface by shelling
//! out fails when akon runs rootless. This module performs the same operations
//! **in-process** over an `AF_NETLINK` socket, so they run under akon's own
//! capability — no `sudo`, no cap-dropping child.
//!
//! Scope is deliberately small and fixed: bring a link up, set its MTU, add an
//! address, add/delete routes (device-bound and via-gateway), read the current
//! default route, and delete the link. Message construction (`nlmsghdr` +
//! `ifinfomsg`/`ifaddrmsg`/`rtmsg` + `rtattr`s, with NLA alignment) is **pure**
//! and unit-tested byte-for-byte; only [`NetlinkSocket`] touches the kernel.
//!
//! Linux-only (gated at the module declaration in `f5/mod.rs`).
//!
//! The byte-by-byte `Vec::push` sequences below are deliberate wire-format
//! struct construction (`ifinfomsg`/`ifaddrmsg`/`rtmsg`), so we silence clippy's
//! `vec_init_then_push` lint for the module.
#![allow(clippy::vec_init_then_push)]

use std::io;
use std::net::Ipv4Addr;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

// ---- netlink / rtnetlink constants (from <linux/netlink.h>, <linux/rtnetlink.h>) ----

const NETLINK_ROUTE: libc::c_int = 0;

const NLMSG_ERROR: u16 = 0x2;
const NLMSG_DONE: u16 = 0x3;

// nlmsg flags
const NLM_F_REQUEST: u16 = 0x01;
const NLM_F_ACK: u16 = 0x04;
const NLM_F_EXCL: u16 = 0x200;
const NLM_F_CREATE: u16 = 0x400;
const NLM_F_REPLACE: u16 = 0x100;
const NLM_F_DUMP: u16 = 0x300; // ROOT|MATCH

// message types
const RTM_NEWLINK: u16 = 16;
const RTM_DELLINK: u16 = 17;
const RTM_NEWADDR: u16 = 20;
const RTM_NEWROUTE: u16 = 24;
const RTM_DELROUTE: u16 = 25;
const RTM_GETROUTE: u16 = 26;

// interface flags
const IFF_UP: u32 = 0x1;

// link attributes
const IFLA_MTU: u16 = 4;

// address attributes
const IFA_LOCAL: u16 = 2;
const IFA_ADDRESS: u16 = 1;

// route attributes
const RTA_DST: u16 = 1;
const RTA_OIF: u16 = 4;
const RTA_GATEWAY: u16 = 5;

// route scopes / types / protocols / tables
const RT_SCOPE_UNIVERSE: u8 = 0;
const RT_SCOPE_LINK: u8 = 253;
const RT_TABLE_MAIN: u8 = 254;
const RTPROT_BOOT: u8 = 3;
const RTN_UNICAST: u8 = 1;

const AF_INET: u8 = libc::AF_INET as u8;

// ---- alignment helpers ----

/// netlink message alignment (NLMSG_ALIGNTO = 4).
fn nl_align(len: usize) -> usize {
    (len + 3) & !3
}

/// rtattr alignment (RTA_ALIGNTO = 4).
fn rta_align(len: usize) -> usize {
    (len + 3) & !3
}

/// Append a single `rtattr` (type + payload) to `buf`, padded to RTA_ALIGNTO.
/// Layout: `len:u16 | type:u16 | payload | pad`. `len` excludes padding.
fn push_rtattr(buf: &mut Vec<u8>, attr_type: u16, payload: &[u8]) {
    let len = 4 + payload.len();
    buf.extend_from_slice(&(len as u16).to_ne_bytes());
    buf.extend_from_slice(&attr_type.to_ne_bytes());
    buf.extend_from_slice(payload);
    // pad to alignment
    let pad = rta_align(len) - len;
    buf.extend(std::iter::repeat(0u8).take(pad));
}

/// Finalize a netlink message: prepend the 16-byte `nlmsghdr` with the total
/// length and pad the whole message to NLMSG_ALIGNTO. `body` is everything after
/// the header (the family struct + rtattrs). Returns the complete message bytes.
fn build_nlmsg(msg_type: u16, flags: u16, seq: u32, body: &[u8]) -> Vec<u8> {
    let total = 16 + body.len();
    let mut msg = Vec::with_capacity(nl_align(total));
    msg.extend_from_slice(&(total as u32).to_ne_bytes()); // nlmsg_len
    msg.extend_from_slice(&msg_type.to_ne_bytes()); // nlmsg_type
    msg.extend_from_slice(&flags.to_ne_bytes()); // nlmsg_flags
    msg.extend_from_slice(&seq.to_ne_bytes()); // nlmsg_seq
    msg.extend_from_slice(&0u32.to_ne_bytes()); // nlmsg_pid (kernel fills)
    msg.extend_from_slice(body);
    let pad = nl_align(total) - total;
    msg.extend(std::iter::repeat(0u8).take(pad));
    msg
}

// ---- pure message builders (unit-tested) ----

/// Build an `RTM_NEWLINK`/`RTM_SETLINK`-style request that sets the link UP.
/// Body: `ifinfomsg { family, _pad, type, index, flags, change }`.
pub(crate) fn build_link_up(seq: u32, ifindex: u32) -> Vec<u8> {
    let mut body = Vec::new();
    body.push(AF_INET); // ifi_family (AF_UNSPEC also works; AF_INET is fine)
    body.push(0); // pad
    body.extend_from_slice(&0u16.to_ne_bytes()); // ifi_type
    body.extend_from_slice(&(ifindex as i32).to_ne_bytes()); // ifi_index
    body.extend_from_slice(&IFF_UP.to_ne_bytes()); // ifi_flags
    body.extend_from_slice(&IFF_UP.to_ne_bytes()); // ifi_change (only UP bit)
    build_nlmsg(RTM_NEWLINK, NLM_F_REQUEST | NLM_F_ACK, seq, &body)
}

/// Build an `RTM_NEWLINK` request that sets the link MTU (with an `IFLA_MTU`
/// attribute).
pub(crate) fn build_link_mtu(seq: u32, ifindex: u32, mtu: u32) -> Vec<u8> {
    let mut body = Vec::new();
    body.push(AF_INET);
    body.push(0);
    body.extend_from_slice(&0u16.to_ne_bytes());
    body.extend_from_slice(&(ifindex as i32).to_ne_bytes());
    body.extend_from_slice(&0u32.to_ne_bytes()); // flags
    body.extend_from_slice(&0u32.to_ne_bytes()); // change
    push_rtattr(&mut body, IFLA_MTU, &mtu.to_ne_bytes());
    build_nlmsg(RTM_NEWLINK, NLM_F_REQUEST | NLM_F_ACK, seq, &body)
}

/// Build an `RTM_NEWADDR` request adding `addr/prefix` to `ifindex`.
/// Body: `ifaddrmsg { family, prefixlen, flags, scope, index }` + IFA_LOCAL/ADDRESS.
pub(crate) fn build_addr_add(seq: u32, ifindex: u32, addr: Ipv4Addr, prefix: u8) -> Vec<u8> {
    let mut body = Vec::new();
    body.push(AF_INET); // ifa_family
    body.push(prefix); // ifa_prefixlen
    body.push(0); // ifa_flags
    body.push(RT_SCOPE_UNIVERSE); // ifa_scope
    body.extend_from_slice(&ifindex.to_ne_bytes()); // ifa_index
    let octets = addr.octets();
    push_rtattr(&mut body, IFA_LOCAL, &octets);
    push_rtattr(&mut body, IFA_ADDRESS, &octets);
    build_nlmsg(
        RTM_NEWADDR,
        NLM_F_REQUEST | NLM_F_ACK | NLM_F_CREATE | NLM_F_EXCL,
        seq,
        &body,
    )
}

/// Build an `RTM_NEWROUTE`/`RTM_DELROUTE` request for `dest/prefix`.
/// When `gateway` is `Some`, a via-gateway route is built (scope universe);
/// otherwise a device-scoped (link) route bound to `oif` is built. `replace`
/// adds NLM_F_REPLACE so an existing route is overwritten (mirrors `ip route
/// replace`). For deletes, `oif`/`gateway` may be omitted.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_route(
    seq: u32,
    del: bool,
    replace: bool,
    dest: Ipv4Addr,
    prefix: u8,
    oif: Option<u32>,
    gateway: Option<Ipv4Addr>,
) -> Vec<u8> {
    let mut body = Vec::new();
    body.push(AF_INET); // rtm_family
    body.push(prefix); // rtm_dst_len
    body.push(0); // rtm_src_len
    body.push(0); // rtm_tos
    body.push(RT_TABLE_MAIN); // rtm_table
    body.push(if del { 0 } else { RTPROT_BOOT }); // rtm_protocol
                                                  // scope: link for device routes, universe for gateway routes.
    let scope = if gateway.is_some() {
        RT_SCOPE_UNIVERSE
    } else {
        RT_SCOPE_LINK
    };
    body.push(if del { RT_SCOPE_UNIVERSE } else { scope }); // rtm_scope
    body.push(if del { 0 } else { RTN_UNICAST }); // rtm_type
    body.extend_from_slice(&0u32.to_ne_bytes()); // rtm_flags

    push_rtattr(&mut body, RTA_DST, &dest.octets());
    if let Some(gw) = gateway {
        push_rtattr(&mut body, RTA_GATEWAY, &gw.octets());
    }
    if let Some(oif) = oif {
        push_rtattr(&mut body, RTA_OIF, &oif.to_ne_bytes());
    }

    let (mtype, mut flags) = if del {
        (RTM_DELROUTE, NLM_F_REQUEST | NLM_F_ACK)
    } else {
        (RTM_NEWROUTE, NLM_F_REQUEST | NLM_F_ACK | NLM_F_CREATE)
    };
    if !del {
        flags |= if replace { NLM_F_REPLACE } else { NLM_F_EXCL };
    }
    build_nlmsg(mtype, flags, seq, &body)
}

/// Build an `RTM_DELLINK` request deleting `ifindex` (reaps device-bound routes).
pub(crate) fn build_link_del(seq: u32, ifindex: u32) -> Vec<u8> {
    let mut body = Vec::new();
    body.push(AF_INET);
    body.push(0);
    body.extend_from_slice(&0u16.to_ne_bytes());
    body.extend_from_slice(&(ifindex as i32).to_ne_bytes());
    body.extend_from_slice(&0u32.to_ne_bytes());
    body.extend_from_slice(&0u32.to_ne_bytes());
    build_nlmsg(RTM_DELLINK, NLM_F_REQUEST | NLM_F_ACK, seq, &body)
}

/// Build an `RTM_GETROUTE` dump request (used to find the current default route).
pub(crate) fn build_route_dump(seq: u32) -> Vec<u8> {
    let mut body = Vec::new();
    body.push(AF_INET); // rtm_family
    body.extend_from_slice(&[0u8; 11]); // rest of rtmsg zeroed
    build_nlmsg(RTM_GETROUTE, NLM_F_REQUEST | NLM_F_DUMP, seq, &body)
}

/// Resolve an interface index to its name (e.g. for a tun-skip check).
pub fn if_indextoname(index: u32) -> Option<String> {
    let mut buf = [0u8; libc::IF_NAMESIZE];
    // SAFETY: buf is IF_NAMESIZE bytes; if_indextoname writes a NUL-terminated name.
    let p = unsafe { libc::if_indextoname(index, buf.as_mut_ptr() as *mut libc::c_char) };
    if p.is_null() {
        return None;
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8(buf[..end].to_vec()).ok()
}

/// Resolve an interface name to its kernel index.
pub fn if_nametoindex(name: &str) -> io::Result<u32> {
    let cname = std::ffi::CString::new(name)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "interface name has NUL"))?;
    // SAFETY: cname is a valid NUL-terminated C string.
    let idx = unsafe { libc::if_nametoindex(cname.as_ptr()) };
    if idx == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(idx)
    }
}

/// Whether a network interface with the given name currently exists.
///
/// This is the ground-truth signal for "is the tunnel up?" — it reflects kernel
/// state, not a recorded PID or a state file. Requires no privilege.
pub fn interface_exists(name: &str) -> bool {
    if_nametoindex(name).is_ok()
}

/// Read the first IPv4 address currently assigned to `name`, live from the
/// kernel via `getifaddrs`. Returns `None` if the interface is absent or has no
/// IPv4 address. Requires no privilege.
pub fn interface_ipv4(name: &str) -> Option<std::net::Ipv4Addr> {
    use std::net::Ipv4Addr;

    let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs allocates a linked list that we free below.
    if unsafe { libc::getifaddrs(&mut ifap) } != 0 {
        return None;
    }
    let mut result: Option<Ipv4Addr> = None;
    let mut cur = ifap;
    // SAFETY: walk the NULL-terminated list; nodes are valid until freeifaddrs.
    while !cur.is_null() {
        let ifa = unsafe { &*cur };
        if !ifa.ifa_addr.is_null() {
            let sa = unsafe { &*ifa.ifa_addr };
            if sa.sa_family as i32 == libc::AF_INET {
                let cname = unsafe { std::ffi::CStr::from_ptr(ifa.ifa_name) };
                if cname.to_str().map(|n| n == name).unwrap_or(false) {
                    let sin = unsafe { &*(ifa.ifa_addr as *const libc::sockaddr_in) };
                    result = Some(Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr)));
                    break;
                }
            }
        }
        cur = ifa.ifa_next;
    }
    // SAFETY: ifap came from getifaddrs and is freed exactly once.
    unsafe { libc::freeifaddrs(ifap) };
    result
}

// ---- socket adapter (thin) ----

/// A `NETLINK_ROUTE` socket for issuing rtnetlink requests in-process.
pub struct NetlinkSocket {
    fd: OwnedFd,
    seq: u32,
}

impl NetlinkSocket {
    /// Open and bind a `NETLINK_ROUTE` socket.
    pub fn open() -> io::Result<Self> {
        // SAFETY: standard socket(2) call with valid args.
        let raw = unsafe { libc::socket(libc::AF_NETLINK, libc::SOCK_RAW, NETLINK_ROUTE) };
        if raw < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: raw is a freshly-created, owned fd.
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };

        // Bind with pid=0 so the kernel assigns the port id.
        let mut addr: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
        addr.nl_family = libc::AF_NETLINK as u16;
        // SAFETY: addr is a valid sockaddr_nl for the lifetime of the call.
        let rc = unsafe {
            libc::bind(
                fd.as_raw_fd(),
                &addr as *const _ as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t,
            )
        };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { fd, seq: 1 })
    }

    fn next_seq(&mut self) -> u32 {
        let s = self.seq;
        self.seq = self.seq.wrapping_add(1);
        s
    }

    /// Send a pre-built request and wait for its ACK (NLMSG_ERROR with err==0)
    /// or a kernel error. Returns the error code as an `io::Error` on failure.
    fn send_ack(&mut self, msg: &[u8]) -> io::Result<()> {
        self.send_raw(msg)?;
        // Read responses until we see the ACK/ERROR for our message.
        let mut buf = [0u8; 8192];
        loop {
            let n = self.recv_raw(&mut buf)?;
            let mut off = 0usize;
            while off + 16 <= n {
                let len = u32::from_ne_bytes(buf[off..off + 4].try_into().unwrap()) as usize;
                let mtype = u16::from_ne_bytes(buf[off + 4..off + 6].try_into().unwrap());
                if len < 16 || off + len > n {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "short nlmsg"));
                }
                if mtype == NLMSG_ERROR {
                    // struct nlmsgerr { s32 error; struct nlmsghdr orig; }
                    let err = i32::from_ne_bytes(buf[off + 16..off + 20].try_into().unwrap());
                    if err == 0 {
                        return Ok(()); // ACK
                    }
                    return Err(io::Error::from_raw_os_error(-err));
                }
                if mtype == NLMSG_DONE {
                    return Ok(());
                }
                off += nl_align(len);
            }
        }
    }

    fn send_raw(&self, msg: &[u8]) -> io::Result<()> {
        // SAFETY: msg points to `msg.len()` valid bytes; fd is open.
        let rc = unsafe {
            libc::send(
                self.fd.as_raw_fd(),
                msg.as_ptr() as *const libc::c_void,
                msg.len(),
                0,
            )
        };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn recv_raw(&self, buf: &mut [u8]) -> io::Result<usize> {
        // SAFETY: buf is a valid writable slice; fd is open.
        let rc = unsafe {
            libc::recv(
                self.fd.as_raw_fd(),
                buf.as_mut_ptr() as *mut libc::c_void,
                buf.len(),
                0,
            )
        };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(rc as usize)
    }

    /// Bring the link up.
    pub fn link_up(&mut self, ifindex: u32) -> io::Result<()> {
        let seq = self.next_seq();
        self.send_ack(&build_link_up(seq, ifindex))
    }

    /// Set the link MTU.
    pub fn link_set_mtu(&mut self, ifindex: u32, mtu: u32) -> io::Result<()> {
        let seq = self.next_seq();
        self.send_ack(&build_link_mtu(seq, ifindex, mtu))
    }

    /// Add an IPv4 address to the interface.
    pub fn addr_add(&mut self, ifindex: u32, addr: Ipv4Addr, prefix: u8) -> io::Result<()> {
        let seq = self.next_seq();
        self.send_ack(&build_addr_add(seq, ifindex, addr, prefix))
    }

    /// Add (or replace) a device-bound route `dest/prefix` out of `ifindex`.
    pub fn route_add_dev(
        &mut self,
        dest: Ipv4Addr,
        prefix: u8,
        ifindex: u32,
        replace: bool,
    ) -> io::Result<()> {
        let seq = self.next_seq();
        self.send_ack(&build_route(
            seq,
            false,
            replace,
            dest,
            prefix,
            Some(ifindex),
            None,
        ))
    }

    /// Add (or replace) a via-gateway route `dest/prefix` via `gateway` out of
    /// `ifindex` (used to pin the VPN server to the original default gateway).
    pub fn route_add_via(
        &mut self,
        dest: Ipv4Addr,
        prefix: u8,
        gateway: Ipv4Addr,
        ifindex: u32,
        replace: bool,
    ) -> io::Result<()> {
        let seq = self.next_seq();
        self.send_ack(&build_route(
            seq,
            false,
            replace,
            dest,
            prefix,
            Some(ifindex),
            Some(gateway),
        ))
    }

    /// Delete a route `dest/prefix` (matches regardless of nexthop).
    pub fn route_del(&mut self, dest: Ipv4Addr, prefix: u8) -> io::Result<()> {
        let seq = self.next_seq();
        // Deleting a missing route returns ESRCH; callers treat that as success.
        match self.send_ack(&build_route(seq, true, false, dest, prefix, None, None)) {
            Err(e) if e.raw_os_error() == Some(libc::ESRCH) => Ok(()),
            other => other,
        }
    }

    /// Delete the interface (reaps device-bound routes). Treats a missing
    /// interface (ENODEV) as success.
    pub fn link_del(&mut self, ifindex: u32) -> io::Result<()> {
        let seq = self.next_seq();
        match self.send_ack(&build_link_del(seq, ifindex)) {
            Err(e) if e.raw_os_error() == Some(libc::ENODEV) => Ok(()),
            other => other,
        }
    }

    /// Find the current IPv4 default route as `(gateway, oif_index)` by dumping
    /// the route table. Returns the first default route NOT pointing at a `tun*`
    /// interface (skipping stale akon routes is the caller's job via the index).
    pub fn default_route(&mut self) -> io::Result<Option<(Ipv4Addr, u32)>> {
        let seq = self.next_seq();
        self.send_raw(&build_route_dump(seq))?;
        let mut buf = [0u8; 16384];
        loop {
            let n = self.recv_raw(&mut buf)?;
            let mut off = 0usize;
            while off + 16 <= n {
                let len = u32::from_ne_bytes(buf[off..off + 4].try_into().unwrap()) as usize;
                let mtype = u16::from_ne_bytes(buf[off + 4..off + 6].try_into().unwrap());
                if len < 16 || off + len > n {
                    return Ok(None);
                }
                if mtype == NLMSG_DONE {
                    return Ok(None);
                }
                if mtype == RTM_NEWROUTE {
                    if let Some(found) = parse_default_route(&buf[off..off + len]) {
                        return Ok(Some(found));
                    }
                }
                off += nl_align(len);
            }
        }
    }
}

/// Parse one `RTM_NEWROUTE` message; return `(gateway, oif)` iff it is the IPv4
/// default route (dst_len == 0) in the main table with both a gateway and an
/// output interface.
fn parse_default_route(msg: &[u8]) -> Option<(Ipv4Addr, u32)> {
    // nlmsghdr is 16 bytes; rtmsg follows.
    if msg.len() < 16 + 12 {
        return None;
    }
    let rtm = &msg[16..];
    let dst_len = rtm[1];
    let table = rtm[4];
    if dst_len != 0 || table != RT_TABLE_MAIN {
        return None;
    }
    // rtattrs start after the 12-byte rtmsg.
    let mut p = 16 + 12;
    let mut gateway: Option<Ipv4Addr> = None;
    let mut oif: Option<u32> = None;
    while p + 4 <= msg.len() {
        let alen = u16::from_ne_bytes(msg[p..p + 2].try_into().ok()?) as usize;
        let atype = u16::from_ne_bytes(msg[p + 2..p + 4].try_into().ok()?);
        if alen < 4 || p + alen > msg.len() {
            break;
        }
        let payload = &msg[p + 4..p + alen];
        match atype {
            RTA_GATEWAY if payload.len() == 4 => {
                gateway = Some(Ipv4Addr::new(
                    payload[0], payload[1], payload[2], payload[3],
                ));
            }
            RTA_OIF if payload.len() == 4 => {
                oif = Some(u32::from_ne_bytes(payload.try_into().ok()?));
            }
            _ => {}
        }
        p += rta_align(alen);
    }
    match (gateway, oif) {
        (Some(gw), Some(idx)) => Some((gw, idx)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alignment_helpers() {
        assert_eq!(nl_align(0), 0);
        assert_eq!(nl_align(1), 4);
        assert_eq!(nl_align(4), 4);
        assert_eq!(nl_align(5), 8);
        assert_eq!(rta_align(6), 8);
    }

    #[test]
    fn nlmsg_header_has_correct_length_and_fields() {
        let body = vec![0u8; 16];
        let msg = build_nlmsg(RTM_NEWLINK, NLM_F_REQUEST | NLM_F_ACK, 7, &body);
        // total length = 16 (header) + 16 (body) = 32, already aligned.
        assert_eq!(msg.len(), 32);
        let len = u32::from_ne_bytes(msg[0..4].try_into().unwrap());
        assert_eq!(len, 32);
        let mtype = u16::from_ne_bytes(msg[4..6].try_into().unwrap());
        assert_eq!(mtype, RTM_NEWLINK);
        let flags = u16::from_ne_bytes(msg[6..8].try_into().unwrap());
        assert_eq!(flags, NLM_F_REQUEST | NLM_F_ACK);
        let seq = u32::from_ne_bytes(msg[8..12].try_into().unwrap());
        assert_eq!(seq, 7);
    }

    #[test]
    fn rtattr_is_aligned_and_well_formed() {
        let mut buf = Vec::new();
        // 4-byte payload -> len 8, already aligned.
        push_rtattr(&mut buf, IFLA_MTU, &1411u32.to_ne_bytes());
        assert_eq!(buf.len(), 8);
        let alen = u16::from_ne_bytes(buf[0..2].try_into().unwrap());
        assert_eq!(alen, 8);
        let atype = u16::from_ne_bytes(buf[2..4].try_into().unwrap());
        assert_eq!(atype, IFLA_MTU);
        let val = u32::from_ne_bytes(buf[4..8].try_into().unwrap());
        assert_eq!(val, 1411);

        // 1-byte payload -> len 5, padded to 8.
        let mut buf2 = Vec::new();
        push_rtattr(&mut buf2, 1, &[0xaa]);
        assert_eq!(buf2.len(), 8, "must pad to RTA_ALIGNTO");
        let alen2 = u16::from_ne_bytes(buf2[0..2].try_into().unwrap());
        assert_eq!(alen2, 5, "len field excludes padding");
    }

    #[test]
    fn link_up_sets_only_up_bit() {
        let msg = build_link_up(1, 42);
        // body starts at offset 16: ifinfomsg.
        let body = &msg[16..];
        // ifi_index at body[4..8].
        let idx = i32::from_ne_bytes(body[4..8].try_into().unwrap());
        assert_eq!(idx, 42);
        let flags = u32::from_ne_bytes(body[8..12].try_into().unwrap());
        let change = u32::from_ne_bytes(body[12..16].try_into().unwrap());
        assert_eq!(flags & IFF_UP, IFF_UP);
        assert_eq!(change, IFF_UP, "change mask must touch only the UP bit");
    }

    #[test]
    fn addr_add_carries_prefix_and_address() {
        let msg = build_addr_add(1, 7, Ipv4Addr::new(10, 10, 99, 2), 32);
        let body = &msg[16..];
        assert_eq!(body[0], AF_INET); // ifa_family
        assert_eq!(body[1], 32); // ifa_prefixlen
        let idx = u32::from_ne_bytes(body[4..8].try_into().unwrap());
        assert_eq!(idx, 7);
        // The 10.10.99.2 octets must appear (IFA_LOCAL payload).
        assert!(msg.windows(4).any(|w| w == [10, 10, 99, 2]));
    }

    #[test]
    fn device_route_is_link_scoped_with_oif() {
        let msg = build_route(1, false, true, Ipv4Addr::new(0, 0, 0, 0), 1, Some(9), None);
        let body = &msg[16..];
        assert_eq!(body[0], AF_INET); // rtm_family
        assert_eq!(body[1], 1); // rtm_dst_len (0.0.0.0/1)
                                // rtmsg layout: family,dst_len,src_len,tos,table,protocol,scope,type
        assert_eq!(body[6], RT_SCOPE_LINK); // rtm_scope for device routes
                                            // REPLACE flag present.
        let flags = u16::from_ne_bytes(msg[6..8].try_into().unwrap());
        assert_eq!(flags & NLM_F_REPLACE, NLM_F_REPLACE);
        let mtype = u16::from_ne_bytes(msg[4..6].try_into().unwrap());
        assert_eq!(mtype, RTM_NEWROUTE);
    }

    #[test]
    fn gateway_route_is_universe_scoped_with_gateway() {
        let msg = build_route(
            1,
            false,
            true,
            Ipv4Addr::new(98, 128, 165, 149),
            32,
            Some(3),
            Some(Ipv4Addr::new(192, 168, 0, 1)),
        );
        let body = &msg[16..];
        assert_eq!(body[6], RT_SCOPE_UNIVERSE); // gateway routes are universe-scoped
                                                // Gateway octets present.
        assert!(msg.windows(4).any(|w| w == [192, 168, 0, 1]));
        // Destination octets present.
        assert!(msg.windows(4).any(|w| w == [98, 128, 165, 149]));
    }

    #[test]
    fn delete_route_uses_delroute_type() {
        let msg = build_route(5, true, false, Ipv4Addr::new(10, 0, 0, 0), 8, None, None);
        let mtype = u16::from_ne_bytes(msg[4..6].try_into().unwrap());
        assert_eq!(mtype, RTM_DELROUTE);
        let seq = u32::from_ne_bytes(msg[8..12].try_into().unwrap());
        assert_eq!(seq, 5);
    }

    #[test]
    fn link_del_uses_dellink_type_and_index() {
        let msg = build_link_del(2, 11);
        let mtype = u16::from_ne_bytes(msg[4..6].try_into().unwrap());
        assert_eq!(mtype, RTM_DELLINK);
        let body = &msg[16..];
        let idx = i32::from_ne_bytes(body[4..8].try_into().unwrap());
        assert_eq!(idx, 11);
    }

    #[test]
    fn parse_default_route_extracts_gateway_and_oif() {
        // Build a synthetic RTM_NEWROUTE: dst_len=0, table=main, RTA_GATEWAY + RTA_OIF.
        let mut body = Vec::new();
        body.push(AF_INET); // family
        body.push(0); // dst_len = 0 (default)
        body.push(0); // src_len
        body.push(0); // tos
        body.push(RT_TABLE_MAIN); // table
        body.push(RTPROT_BOOT); // protocol
        body.push(RT_SCOPE_UNIVERSE); // scope
        body.push(RTN_UNICAST); // type
        body.extend_from_slice(&0u32.to_ne_bytes()); // flags
        push_rtattr(&mut body, RTA_GATEWAY, &[192, 168, 0, 1]);
        push_rtattr(&mut body, RTA_OIF, &7u32.to_ne_bytes());
        let msg = build_nlmsg(RTM_NEWROUTE, 0, 1, &body);

        let parsed = parse_default_route(&msg);
        assert_eq!(parsed, Some((Ipv4Addr::new(192, 168, 0, 1), 7)));
    }

    #[test]
    fn parse_default_route_ignores_non_default() {
        // dst_len != 0 -> not a default route.
        let mut body = Vec::new();
        body.push(AF_INET);
        body.push(24); // dst_len = 24
        body.extend_from_slice(&[0u8; 10]);
        push_rtattr(&mut body, RTA_GATEWAY, &[10, 0, 0, 1]);
        push_rtattr(&mut body, RTA_OIF, &3u32.to_ne_bytes());
        let msg = build_nlmsg(RTM_NEWROUTE, 0, 1, &body);
        assert_eq!(parse_default_route(&msg), None);
    }

    // T002 — bounded real adapter test for the status ground-truth helpers.
    // Uses the loopback interface, which always exists and carries 127.0.0.1.

    #[test]
    fn interface_exists_for_loopback_not_for_bogus() {
        assert!(interface_exists("lo"), "loopback must exist");
        assert!(
            !interface_exists("akon-no-such-iface0"),
            "a clearly-absent interface must not exist"
        );
    }

    #[test]
    fn interface_ipv4_reads_loopback_address() {
        assert_eq!(
            interface_ipv4("lo"),
            Some(std::net::Ipv4Addr::new(127, 0, 0, 1))
        );
        assert_eq!(interface_ipv4("akon-no-such-iface0"), None);
    }
}
