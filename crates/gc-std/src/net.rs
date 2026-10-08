//! Minimal host-edge networking on the BBA: enough for a homebrew's
//! "agent" plane (console ping reply + UDP out). No TCP.
//!
//! Layout:
//! ```text
//! eth header   = dst[6] src[6] type[2]
//! arp          = ht/pr/pt proto, op
//! ipv4         = hdr ihl=5, proto 1=icmp, 17=udp
//! udp          = sport dport len checksum(=0)
//! icmp echo    = type 8/0 + ident + seq + dummy payload
//! ```
//!
//! Callbacks: incoming ARP answers populate the cache; ICMP echo-replies get
//! counted in [`icmp_pongs`]; UDP accepts any payload on our port and queues
//! it for the app.

use alloc::vec::Vec;

use crate::bba;

pub const ETHERTYPE_ARP: u16 = 0x0806;
pub const ETHERTYPE_IP: u16 = 0x0800;

pub const ARP_REQUEST: u16 = 1;
pub const ARP_REPLY: u16 = 2;

pub const ICMP_ECHO_REQUEST: u8 = 8;
pub const ICMP_ECHO_REPLY: u8 = 0;

pub const IP_PROTO_ICMP: u8 = 1;
pub const IP_PROTO_UDP: u8 = 17;

/// One "our MAC is the target device" distribution. On Dolphin (slirp) the
/// gateway is 10.0.2.2; the homebrew takes 10.0.2.15 like real consoles.
pub const GATEWAY: [u8; 4] = [10, 0, 2, 2];
pub const LOCAL_IP: [u8; 4] = [10, 0, 2, 15];

// ---------------------------------------------------------------------------
// frame writers (all BigEndian as the wire wants)
// ---------------------------------------------------------------------------

fn be16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_be_bytes());
}

/// Minimal IPv4 header checksum (RFC 791).
fn ip_checksum(hdr: &[u8]) -> u16 {
    let mut sum = 0u32;
    let n = hdr.len() & !1;
    for i in (0..n).step_by(2) {
        sum += u16::from_be_bytes([hdr[i], hdr[i + 1]]) as u32;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !sum as u16
}

fn icmp_checksum(hdr: &[u8]) -> u16 {
    ip_checksum(hdr)
}

// ---------------------------------------------------------------------------
// arp
// ---------------------------------------------------------------------------

static mut ARP_CACHE: Option<([u8; 4], [u8; 6])> = None;

/// Send an ARP `who-has` for `ip`.
pub fn arp_resolve(ip: [u8; 4]) {
    let mut f = Vec::new();
    f.extend_from_slice(&[0xff; 6]); // dst = broadcast
    f.extend_from_slice(&bba::mac()); // src
    be16(&mut f, ETHERTYPE_ARP);
    be16(&mut f, 1); // htype Eth
    be16(&mut f, ETHERTYPE_IP); // ptype
    f.push(6); // hlen
    f.push(4); // plen
    be16(&mut f, ARP_REQUEST);
    f.extend_from_slice(&bba::mac()); // sender MAC
    f.extend_from_slice(&LOCAL_IP); // sender IP
    f.extend_from_slice(&[0; 6]); // target MAC
    f.extend_from_slice(&ip); // target IP
    let _ = bba::send_frame(&f);
}

/// Look up `ip` in the small ARP cache.
pub fn arp_lookup(ip: [u8; 4]) -> Option<[u8; 6]> {
    unsafe { ARP_CACHE }.filter(|(i, _)| *i == ip).map(|(_, m)| m)
}

// ---------------------------------------------------------------------------
// UDP
// ---------------------------------------------------------------------------

/// Current snapshot of inbound UDP payloads (bytes only, with src).
pub struct UdpPacket {
    pub from_ip: [u8; 4],
    pub from_port: u16,
    pub payload: Vec<u8>,
}

static mut UDP_QUEUE: Option<Vec<UdpPacket>> = None;

fn udp_q() -> &'static mut Vec<UdpPacket> {
    unsafe {
        let p = &raw mut UDP_QUEUE;
        if (*p).is_none() {
            *p = Some(Vec::new());
        }
        (*p).as_mut().unwrap()
    }
}

/// Send a UDP datagram from our IP to `dst_ip:dst_port`.
pub fn udp_send(dst_ip: [u8; 4], sport: u16, dport: u16, payload: &[u8]) -> bool {
    let Some(dst_mac) = arp_lookup(dst_ip) else {
        // ask ARP once — caller can retry; send returns false
        arp_resolve(dst_ip);
        return false;
    };
    let src_mac = bba::mac();
    let udp_len = 8 + payload.len();
    let ip_len = 20 + udp_len;

    let mut f = Vec::new();
    f.extend_from_slice(&dst_mac);
    f.extend_from_slice(&src_mac);
    be16(&mut f, ETHERTYPE_IP);

    // IPv4 header
    f.push(0x45); // ver 4, ihl 5
    f.push(0); // tos
    be16(&mut f, ip_len as u16);
    be16(&mut f, 0); // identification
    be16(&mut f, 0); // flags+frag offset
    f.push(64); // ttl
    f.push(IP_PROTO_UDP);
    // checksum over first 10 header bytes (bytes 0..9)
    f.extend_from_slice(&[0, 0]); // checksum placeholder

    f.extend_from_slice(&LOCAL_IP);
    f.extend_from_slice(&dst_ip);

    // UDP header
    be16(&mut f, sport);
    be16(&mut f, dport);
    be16(&mut f, udp_len as u16);
    be16(&mut f, 0); // no checksum
    f.extend_from_slice(payload);

    // finish IP header checksum, at offsets 14..34 (14 is eth hdr size)
    let hdr = &f[14..34];
    let sum = ip_checksum(hdr).to_be_bytes();
    f[24] = sum[0];
    f[25] = sum[1];

    bba::send_frame(&f)
}

// ---------------------------------------------------------------------------
// icmp
// ---------------------------------------------------------------------------

static mut PONGS: u32 = 0;
pub fn icmp_pongs() -> u32 {
    unsafe { PONGS }
}

/// Send an ICMP echo request ("ping") to dst_ip.
pub fn ping(dst_ip: [u8; 4], ident: u16, seq: u16) -> bool {
    let Some(dst_mac) = arp_lookup(dst_ip) else {
        arp_resolve(dst_ip);
        return false;
    };
    let payload = [0xA5u8; 32];
    let icmp_len = 8 + payload.len();
    let ip_len = 20 + icmp_len;

    let mut f = Vec::new();
    f.extend_from_slice(&dst_mac);
    f.extend_from_slice(&bba::mac());
    be16(&mut f, ETHERTYPE_IP);

    f.push(0x45);
    f.push(0);
    be16(&mut f, ip_len as u16);
    be16(&mut f, 0);
    be16(&mut f, 0);
    f.push(64);
    f.push(IP_PROTO_ICMP);
    f.extend_from_slice(&[0, 0]); // csum placeholder
    f.extend_from_slice(&LOCAL_IP);
    f.extend_from_slice(&dst_ip);

    f.push(ICMP_ECHO_REQUEST);
    f.push(0);
    f.extend_from_slice(&[0, 0]); // icmp csum placeholder
    be16(&mut f, ident);
    be16(&mut f, seq);
    f.extend_from_slice(&payload);

    let hdr = &f[14..34];
    let ipsum = ip_checksum(hdr).to_be_bytes();
    f[24] = ipsum[0];
    f[25] = ipsum[1];

    let icmp_range = &f[34..34 + icmp_len];
    let icmpsum = icmp_checksum(icmp_range).to_be_bytes();
    f[36] = icmpsum[0];
    f[37] = icmpsum[1];

    bba::send_frame(&f)
}

// ---------------------------------------------------------------------------
// rx path — auto-ARP answer + accept ICMP reply + UDP in
// ---------------------------------------------------------------------------

/// Poll the NIC once; handles ARP (answers lookups of our IP), counts pings,
/// queues UDP. Call on a loop (e.g. on the agent thread's cycle).
pub fn poll() {
    let mut frame = [0u8; 1600];
    while let Some(n) = bba::poll_frame(&mut frame) {
        let f = &frame[..n];
        if f.len() < 14 {
            continue;
        }
        let etype = u16::from_be_bytes([f[12], f[13]]);
        if etype == ETHERTYPE_ARP {
            parse_arp(f);
        } else if etype == ETHERTYPE_IP {
            parse_ip(f);
        }
    }
}

fn parse_arp(f: &[u8]) {
    if f.len() < 14 + 28 {
        return;
    }
    let op = u16::from_be_bytes([f[14 + 6], f[14 + 7]]);
    let sha = &f[14 + 8..14 + 14];
    let spa = &f[14 + 14..14 + 18];
    let _tha = &f[14 + 18..14 + 24];
    let tpa = &f[14 + 24..14 + 28];
    if tpa != LOCAL_IP.as_slice() && op == ARP_REQUEST {
        return;
    }
    if op == ARP_REQUEST && tpa == LOCAL_IP.as_slice() {
        // answer: my MAC for LOCAL_IP
        let mut r = Vec::new();
        r.extend_from_slice(sha);
        r.extend_from_slice(&bba::mac());
        be16(&mut r, ETHERTYPE_ARP);
        be16(&mut r, 1);
        be16(&mut r, ETHERTYPE_IP);
        r.push(6);
        r.push(4);
        be16(&mut r, ARP_REPLY);
        r.extend_from_slice(&bba::mac());
        r.extend_from_slice(&LOCAL_IP);
        r.extend_from_slice(sha);
        r.extend_from_slice(spa);
        let _ = bba::send_frame(&r);
    }
    if op == ARP_REPLY && spa == GATEWAY.as_slice() {
        let ip: [u8; 4] = spa.try_into().unwrap();
        let mac: [u8; 6] = sha.try_into().unwrap();
        unsafe { ARP_CACHE = Some((ip, mac)) };
    }
}

fn parse_ip(f: &[u8]) {
    if f.len() < 14 + 20 {
        return;
    }
    let ip_off = 14;
    let src = &f[ip_off + 12..ip_off + 16];
    let proto = f[ip_off + 9];
    let ihl = (f[ip_off] & 0x0f) as usize;
    let _dst = &f[ip_off + 16..ip_off + 20];
    match proto {
        IP_PROTO_ICMP => {
            if f.len() < ip_off + ihl * 4 + 2 {
                return;
            }
            let icmp_off = ip_off + ihl * 4;
            match f[icmp_off] {
                ICMP_ECHO_REPLY => unsafe { PONGS += 1 },
                ICMP_ECHO_REQUEST => send_echo_reply(f, src),
                _ => {}
            }
        }
        IP_PROTO_UDP => {
            if f.len() < ip_off + ihl * 4 + 8 {
                return;
            }
            let and = ip_off + ihl * 4;
            let sport = u16::from_be_bytes([f[and], f[and + 1]]);
            let dport = u16::from_be_bytes([f[and + 2], f[and + 3]]);
            let _ = dport;
            let ulen = u16::from_be_bytes([f[and + 4], f[and + 5]]);
            let plen = ulen as usize - 8;
            if f.len() < and + 8 + plen {
                return;
            }
            let mut src_ip = [0u8; 4];
            src_ip.copy_from_slice(src);
            udp_q().push(UdpPacket {
                from_ip: src_ip,
                from_port: sport,
                payload: f[and + 8..and + 8 + plen].to_vec(),
            });
        }
        _ => {}
    }
}

fn send_echo_reply(req: &[u8], dst_ip: &[u8]) {
    // reply by echoing the whole ICMP packet with the type byte switched
    let ip_off = 14;
    let ihl = (req[ip_off] & 0x0f) as usize;
    let icmp_off = ip_off + ihl * 4;
    let ip_total = u16::from_be_bytes([req[ip_off + 2], req[ip_off + 3]]) as usize;
    let icmp_len = ip_total - ihl * 4;
    if req.len() < icmp_off + icmp_len {
        return;
    }
    let Some(dst_mac) = arp_lookup([dst_ip[0], dst_ip[1], dst_ip[2], dst_ip[3]]) else {
        return; // ask arp first
    };
    let mut f = Vec::new();
    f.extend_from_slice(&dst_mac);
    f.extend_from_slice(&bba::mac());
    be16(&mut f, ETHERTYPE_IP);
    // copy IP header, swap src/dst, fix checksum
    let hdr = &mut f[0..icmp_off][14..];
    let _ = hdr;
    f.extend_from_slice(&req[ip_off..ip_off + ihl * 4]);
    let mut iphdr = Vec::new();
    iphdr.extend_from_slice(&req[ip_off..ip_off + ihl * 4]);
    iphdr[10] = 0;
    iphdr[11] = 0;
    iphdr[12..16].copy_from_slice(&LOCAL_IP);
    iphdr[16..20].copy_from_slice(dst_ip);
    let sum = ip_checksum(&iphdr).to_be_bytes();
    iphdr[10] = sum[0];
    iphdr[11] = sum[1];
    f.truncate(14); // drop eth hdr copy attempt
    f.extend_from_slice(&iphdr);

    // icmp: echo reply
    let mut payload = req[icmp_off..icmp_off + icmp_len].to_vec();
    payload[0] = ICMP_ECHO_REPLY;
    payload[2] = 0;
    payload[3] = 0;
    let icsum = icmp_checksum(&payload).to_be_bytes();
    payload[2] = icsum[0];
    payload[3] = icsum[1];
    f.extend_from_slice(&payload);
    bba::send_frame(&f);
}
