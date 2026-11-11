//! Destination policy: only public unicast IPs, only "game" ports.
//!
//! Mirrors `PROTOCOL.md`; the lines marked *hardening* deny address space that
//! is not public unicast but that the document's list does not enumerate
//! (RFC 3879 site-local, RFC 8215 local-use NAT64) and unwrap 6to4, which
//! embeds an IPv4 address in bits 16..48.
//!
//! The document both lists `64:ff9b::/96` as denied and says "NAT64 forms are
//! checked against the IPv4 list". Both readings reject a NAT64 embedding of a
//! private address; they differ only for a NAT64 embedding of a *public* one.
//! This implementation takes the literal deny list (the whole well-known
//! prefix is refused) because it is the stricter of the two, while the
//! IPv4-embedding rule is applied to `::ffff:a.b.c.d` and `2002::/16`.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Denied IPv4 ranges as `(network, prefix length)`.
const V4_DENY: &[(u32, u32)] = &[
    (0x0000_0000, 8),  // 0.0.0.0/8      "this network"
    (0x0a00_0000, 8),  // 10/8           private
    (0x6440_0000, 10), // 100.64/10     CGNAT
    (0x7f00_0000, 8),  // 127/8          loopback
    (0xa9fe_0000, 16), // 169.254/16    link-local
    (0xac10_0000, 12), // 172.16/12     private
    (0xc000_0000, 24), // 192.0.0/24    IETF protocol assignments
    (0xc000_0200, 24), // 192.0.2/24    TEST-NET-1
    (0xc058_6300, 24), // 192.88.99/24  6to4 relay anycast (deprecated)
    (0xc0a8_0000, 16), // 192.168/16    private
    (0xc612_0000, 15), // 198.18/15     benchmarking
    (0xc633_6400, 24), // 198.51.100/24 TEST-NET-2
    (0xcb00_7100, 24), // 203.0.113/24  TEST-NET-3
    (0xe000_0000, 4),  // 224/4         multicast
    (0xf000_0000, 4),  // 240/4         reserved + broadcast
];

/// Denied IPv6 ranges as `(network, prefix length)`.
const V6_DENY: &[(u128, u32)] = &[
    (0, 128),                         // ::
    (1, 128),                         // ::1
    (0xfc00u128 << 112, 7),           // fc00::/7  unique-local
    (0xfe80u128 << 112, 10),          // fe80::/10 link-local
    (0xff00u128 << 112, 8),           // ff00::/8  multicast
    (0x2001_0db8u128 << 96, 32),      // 2001:db8::/32 documentation
    (0x0064_ff9bu128 << 96, 96),      // 64:ff9b::/96   well-known NAT64
    (0x0100u128 << 112, 64),          // 100::/64  discard-only
    (0xfec0u128 << 112, 10),          // fec0::/10 site-local (hardening)
    (0x0064_ff9b_0001u128 << 80, 48), // 64:ff9b:1::/48 local NAT64 (hardening)
];

/// Ports that are always refused, whatever the address.
const DENY_PORTS: &[u16] = &[3306, 3389, 4567, 5432, 5900, 6379, 6380, 9200, 11211, 27017];

/// Allowed destination port: 1024–65535 minus the service deny list.
pub fn port_allowed(port: u16) -> bool {
    (1024..=65535).contains(&port) && !DENY_PORTS.contains(&port)
}

fn in_cidr4(ip: u32, net: u32, prefix: u32) -> bool {
    let mask = match prefix {
        0 => 0,
        32 => u32::MAX,
        n => u32::MAX << (32 - n),
    };
    (ip & mask) == (net & mask)
}

fn in_cidr6(ip: u128, net: u128, prefix: u32) -> bool {
    let mask = match prefix {
        0 => 0,
        128 => u128::MAX,
        n => u128::MAX << (128 - n),
    };
    (ip & mask) == (net & mask)
}

fn v4_allowed(ip: Ipv4Addr) -> bool {
    let n = u32::from(ip);
    !V4_DENY.iter().any(|&(net, p)| in_cidr4(n, net, p))
}

fn v6_allowed(ip: Ipv6Addr) -> bool {
    // IPv4-mapped (`::ffff:a.b.c.d`) is judged by the IPv4 rules, otherwise a
    // mapped private address would sail past the IPv6 list.
    if let Some(v4) = ip.to_ipv4_mapped() {
        return v4_allowed(v4);
    }
    // 6to4 carries an IPv4 address in bits 16..48 (hardening).
    let n = u128::from(ip);
    if in_cidr6(n, 0x2002u128 << 112, 16) {
        let embedded = Ipv4Addr::from(((n >> 80) & 0xffff_ffff) as u32);
        if !v4_allowed(embedded) {
            return false;
        }
    }
    !V6_DENY.iter().any(|&(net, p)| in_cidr6(n, net, p))
}

/// May the relay connect to `ip`? `allow_loopback` is the test-only seam that
/// exempts 127/8 and ::1, never anything else.
pub fn address_allowed(ip: IpAddr, allow_loopback: bool) -> bool {
    if allow_loopback && ip.is_loopback() {
        return true;
    }
    match ip {
        IpAddr::V4(v4) => v4_allowed(v4),
        IpAddr::V6(v6) => v6_allowed(v6),
    }
}

/// Full destination policy check: address and port.
pub fn allowed_with(ip: IpAddr, port: u16, allow_loopback: bool) -> bool {
    port_allowed(port) && address_allowed(ip, allow_loopback)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;

    fn allowed(s: &str, port: u16) -> bool {
        let ip: IpAddr = s.parse().unwrap();
        allowed_with(ip, port, false)
    }

    #[test]
    fn ipv4_deny_edges() {
        let denied = [
            "0.0.0.0",
            "0.255.255.255",
            "10.0.0.0",
            "10.255.255.255",
            "100.64.0.0",
            "100.127.255.255",
            "127.0.0.0",
            "127.255.255.255",
            "169.254.0.0",
            "169.254.255.255",
            "172.16.0.0",
            "172.31.255.255",
            "192.0.0.0",
            "192.0.0.255",
            "192.0.2.0",
            "192.0.2.255",
            "192.88.99.0",
            "192.88.99.255",
            "192.168.0.0",
            "192.168.255.255",
            "198.18.0.0",
            "198.19.255.255",
            "198.51.100.0",
            "198.51.100.255",
            "203.0.113.0",
            "203.0.113.255",
            "224.0.0.0",
            "239.255.255.255",
            "240.0.0.0",
            "255.255.255.255",
        ];
        for s in denied {
            assert!(!allowed(s, 25565), "{s} must be denied");
        }
    }

    #[test]
    fn ipv4_allow_edges() {
        let ok = [
            "1.0.0.0",
            "8.8.8.8",
            "11.0.0.0",
            "100.63.255.255",
            "100.128.0.0",
            "126.255.255.255",
            "128.0.0.0",
            "150.0.0.1",
            "169.253.255.255",
            "169.255.0.0",
            "172.15.255.255",
            "172.32.0.0",
            "192.0.1.255",
            "192.0.3.0",
            "192.88.98.255",
            "192.88.100.0",
            "192.167.255.255",
            "192.169.0.0",
            "198.17.255.255",
            "198.20.0.0",
            "198.51.99.255",
            "198.51.101.0",
            "203.0.112.255",
            "203.0.114.0",
            "223.255.255.255",
        ];
        for s in ok {
            assert!(allowed(s, 25565), "{s} must be allowed");
        }
    }

    #[test]
    fn ipv6_deny_and_allow() {
        let denied = [
            "::",
            "::1",
            "fc00::1",
            "fdff:ffff::1",
            "fe80::1",
            "febf:ffff::1",
            "ff02::1",
            "2001:db8::1",
            "64:ff9b::7f00:1",
            "100::1",
            "fec0::1",        // hardening
            "64:ff9b:1::1:1", // hardening
        ];
        for s in denied {
            assert!(!allowed(s, 25565), "{s} must be denied");
        }
        let ok = [
            "2001:4860:4860::8888",
            "2606:4700:4700::1111",
            "2a01:4f8::1",
            "2001:db9::1",
        ];
        for s in ok {
            assert!(allowed(s, 25565), "{s} must be allowed");
        }
    }

    #[test]
    fn mapped_ipv6_uses_ipv4_rules() {
        let denied = [
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "::ffff:192.168.1.1",
            "::ffff:169.254.169.254",
            "::ffff:0.0.0.0",
        ];
        for s in denied {
            assert!(!allowed(s, 25565), "{s} must be denied");
        }
        assert!(allowed("::ffff:8.8.8.8", 25565));
        assert!(allowed("::ffff:1.1.1.1", 25565));
    }

    #[test]
    fn six_to_four_uses_embedded_ipv4() {
        // 2002:7f00:1:: embeds 127.0.0.1.
        assert!(!allowed("2002:7f00:0001::1", 25565));
        assert!(!allowed("2002:a9fe:a9fe::1", 25565)); // 169.254.169.254
        assert!(allowed("2002:0808:0808::1", 25565)); // 8.8.8.8
    }

    #[test]
    fn port_rules() {
        let ip = "8.8.8.8";
        assert!(!allowed(ip, 1023));
        assert!(allowed(ip, 1024));
        assert!(allowed(ip, 25565));
        assert!(allowed(ip, 65535));
        for p in [3306, 3389, 4567, 5432, 5900, 6379, 6380, 9200, 11211, 27017] {
            assert!(!allowed(ip, p), "port {p} must be denied");
        }
        assert!(allowed(ip, 3305));
        assert!(allowed(ip, 3307));
        assert!(allowed(ip, 27016));
        assert!(allowed(ip, 27018));
        // Denied port on an allowed address is still denied.
        assert!(!allowed("1.1.1.1", 3306));
    }

    #[test]
    fn loopback_seam_only_relaxes_loopback() {
        let lo: IpAddr = "127.0.0.1".parse().unwrap();
        let lo6: IpAddr = "::1".parse().unwrap();
        let priv_ip: IpAddr = "10.0.0.1".parse().unwrap();
        assert!(allowed_with(lo, 25565, true));
        assert!(allowed_with(lo6, 25565, true));
        assert!(!allowed_with(priv_ip, 25565, true));
        // Port rules are never relaxed.
        assert!(!allowed_with(lo, 3306, true));
        assert!(!allowed_with(lo, 443, true));
    }
}
