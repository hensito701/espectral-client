//! PROXY protocol v2 header for the relay's upstream hop (`--proxy-v2`).
//!
//! Some Espectral destinations sit behind a HAProxy front that expects a PROXY
//! protocol v2 header before the first Minecraft byte. For those, the relay
//! writes this header on the upstream socket right after connecting, carrying
//! the player's real source address, so AuthMe per-IP limits and AdvancedBan
//! IP bans keep seeing the player instead of the relay.
//!
//! Only the TCP-over-IPv4 and TCP-over-IPv6 forms with the PROXY command are
//! produced, in the fixed binary layout of the HAProxy spec. A destination that
//! does not expect the header would read it as Minecraft traffic and drop the
//! connection, so the list of destinations is explicit and operator-controlled.

use std::net::{IpAddr, Ipv6Addr, SocketAddr};

/// The 12-byte v2 signature.
const SIGNATURE: [u8; 12] = [
    0x0D, 0x0A, 0x0D, 0x0A, 0x00, 0x0D, 0x0A, 0x51, 0x55, 0x49, 0x54, 0x0A,
];
/// Protocol version 2 in the high nibble, PROXY command in the low nibble.
const VERSION_PROXY: u8 = 0x21;
/// AF_INET + SOCK_STREAM.
const TCP4: u8 = 0x11;
/// AF_INET6 + SOCK_STREAM.
const TCP6: u8 = 0x21;
/// Address block length for IPv4: 4 + 4 + 2 + 2.
const LEN_TCP4: u16 = 12;
/// Address block length for IPv6: 16 + 16 + 2 + 2.
const LEN_TCP6: u16 = 36;

/// Folds an IPv4-mapped IPv6 address (`::ffff:a.b.c.d`) back to plain IPv4, so
/// one peer has one spelling whether the socket was dual-stack or not.
pub fn canonical(addr: SocketAddr) -> SocketAddr {
    match addr {
        SocketAddr::V6(v6) => match v6.ip().to_ipv4_mapped() {
            Some(v4) => SocketAddr::new(IpAddr::V4(v4), v6.port()),
            None => addr,
        },
        SocketAddr::V4(_) => addr,
    }
}

/// The complete PROXY v2 header for a TCP connection from `src` (the player)
/// to `dst` (the upstream). The IPv4 form is used only when both ends are
/// IPv4; otherwise the IPv6 form is used, with an IPv4 end written as an
/// IPv4-mapped address.
pub fn encode(src: SocketAddr, dst: SocketAddr) -> Vec<u8> {
    let (src, dst) = (canonical(src), canonical(dst));
    let mut out = Vec::with_capacity(SIGNATURE.len() + 4 + LEN_TCP6 as usize);
    out.extend_from_slice(&SIGNATURE);
    match (src, dst) {
        (SocketAddr::V4(s), SocketAddr::V4(d)) => {
            out.push(VERSION_PROXY);
            out.push(TCP4);
            out.extend_from_slice(&LEN_TCP4.to_be_bytes());
            out.extend_from_slice(&s.ip().octets());
            out.extend_from_slice(&d.ip().octets());
            out.extend_from_slice(&s.port().to_be_bytes());
            out.extend_from_slice(&d.port().to_be_bytes());
        }
        _ => {
            out.push(VERSION_PROXY);
            out.push(TCP6);
            out.extend_from_slice(&LEN_TCP6.to_be_bytes());
            out.extend_from_slice(&as_v6(src.ip()).octets());
            out.extend_from_slice(&as_v6(dst.ip()).octets());
            out.extend_from_slice(&src.port().to_be_bytes());
            out.extend_from_slice(&dst.port().to_be_bytes());
        }
    }
    out
}

fn as_v6(ip: IpAddr) -> Ipv6Addr {
    match ip {
        IpAddr::V4(v4) => v4.to_ipv6_mapped(),
        IpAddr::V6(v6) => v6,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, SocketAddrV6};

    fn v4(a: [u8; 4], port: u16) -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::from(a)), port)
    }

    #[test]
    fn ipv4_header_matches_the_spec_bytes() {
        // Player 203.0.113.7:51234 to <nvh1-ip>:25564 (the MAIN front).
        let h = encode(v4([203, 0, 113, 7], 51234), v4([2, 27, 233, 26], 25564));
        assert_eq!(
            h,
            vec![
                0x0D, 0x0A, 0x0D, 0x0A, 0x00, 0x0D, 0x0A, 0x51, 0x55, 0x49, 0x54, 0x0A, 0x21,
                0x11, 0x00, 0x0C, 0xCB, 0x00, 0x71, 0x07, 0x02, 0x1B, 0xE9, 0x1A, 0xC8, 0x22,
                0x63, 0xDC,
            ]
        );
    }

    #[test]
    fn ipv6_header_uses_the_inet6_block() {
        let src = SocketAddr::V6(SocketAddrV6::new(
            "2001:db8::7".parse().unwrap(),
            51234,
            0,
            0,
        ));
        let dst = SocketAddr::V6(SocketAddrV6::new(
            "2a00:1450::1".parse().unwrap(),
            25564,
            0,
            0,
        ));
        let h = encode(src, dst);
        assert_eq!(h.len(), SIGNATURE.len() + 4 + 36);
        assert_eq!(&h[12..16], &[0x21, 0x21, 0x00, 0x24]);
        assert_eq!(&h[16..32], &"2001:db8::7".parse::<Ipv6Addr>().unwrap().octets());
        assert_eq!(&h[32..48], &"2a00:1450::1".parse::<Ipv6Addr>().unwrap().octets());
        assert_eq!(&h[48..50], &51234u16.to_be_bytes());
        assert_eq!(&h[50..52], &25564u16.to_be_bytes());
    }

    #[test]
    fn mixed_families_are_sent_as_ipv6_with_mapped_ipv4() {
        let src = v4([203, 0, 113, 7], 51234);
        let dst = SocketAddr::V6(SocketAddrV6::new(
            "2a00:1450::1".parse().unwrap(),
            25564,
            0,
            0,
        ));
        let h = encode(src, dst);
        assert_eq!(&h[12..14], &[0x21, 0x21]);
        assert_eq!(
            &h[16..32],
            &Ipv4Addr::new(203, 0, 113, 7).to_ipv6_mapped().octets()
        );
    }

    #[test]
    fn ipv4_mapped_addresses_are_canonicalised_to_ipv4() {
        let mapped: SocketAddr = "[::ffff:203.0.113.7]:51234".parse().unwrap();
        let dst = v4([2, 27, 233, 26], 25564);
        assert_eq!(encode(mapped, dst), encode(v4([203, 0, 113, 7], 51234), dst));
        assert_eq!(canonical(mapped), v4([203, 0, 113, 7], 51234));
        // A real IPv6 address is left alone.
        let real: SocketAddr = "[2001:db8::7]:1".parse().unwrap();
        assert_eq!(canonical(real), real);
    }
}
