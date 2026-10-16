//! What the public internet is, for whatever Niles fetches on a model's
//! say-so: a page to read, a picture to keep.
//!
//! Niles runs inside the house's network and its cluster, beside its own
//! API, the router and the cloud's metadata service. An address a model
//! asks for may have come from a page a stranger wrote, so a name is
//! refused when it is never the public internet, and every address it
//! resolves to is refused when it is not a public one, including the
//! ways IPv6 has of carrying an IPv4 address inside it.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Names that are never the public internet, whatever they resolve to.
/// An address written as an IP is judged by [`public_ip`] instead.
pub(crate) fn public_name(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host.parse::<IpAddr>().is_ok() || host.starts_with('[') {
        return true;
    }
    const PRIVATE_SUFFIXES: [&str; 6] = [
        ".local",
        ".localdomain",
        ".internal",
        ".lan",
        ".home.arpa",
        ".cluster.local",
    ];
    host.contains('.')
        && host != "localhost"
        && !host.ends_with(".localhost")
        && !PRIVATE_SUFFIXES.iter().any(|s| host.ends_with(s))
}

/// Whether `ip` is on the public internet: not this machine, the house,
/// the cluster, a carrier's NAT, the link, a multicast group, or a range
/// set aside for documentation, benchmarks or the future.
pub(crate) fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => public_v4(v4),
        IpAddr::V6(v6) => public_v6(v6),
    }
}

fn public_v4(v4: Ipv4Addr) -> bool {
    let o = v4.octets();
    !(v4.is_private()
        || v4.is_loopback()
        || v4.is_link_local() // 169.254/16, the metadata service among it
        || v4.is_broadcast()
        || v4.is_documentation()
        || v4.is_unspecified()
        || v4.is_multicast()
        || o[0] == 0
        || (o[0] == 100 && (64..128).contains(&o[1])) // carrier-grade NAT
        || (o[0] == 192 && o[1] == 0 && o[2] == 0) // IETF protocol assignments
        || (o[0] == 192 && o[1] == 88 && o[2] == 99) // 6to4 relays
        || (o[0] == 198 && (18..20).contains(&o[1])) // benchmarking
        || o[0] >= 240)
}

fn public_v6(v6: Ipv6Addr) -> bool {
    let s = v6.segments();
    if v6.is_loopback() || v6.is_unspecified() || v6.is_multicast() {
        return false;
    }
    // An IPv4 address inside: mapped (::ffff:a.b.c.d), compatible
    // (::a.b.c.d), NAT64 (64:ff9b::a.b.c.d) and 6to4 (2002:ab:cd::).
    if let Some(v4) = v6.to_ipv4_mapped() {
        return public_v4(v4);
    }
    if s[..6] == [0; 6] {
        return public_v4(Ipv4Addr::new(
            (s[6] >> 8) as u8,
            s[6] as u8,
            (s[7] >> 8) as u8,
            s[7] as u8,
        ));
    }
    if s[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
        return public_v4(Ipv4Addr::new(
            (s[6] >> 8) as u8,
            s[6] as u8,
            (s[7] >> 8) as u8,
            s[7] as u8,
        ));
    }
    if s[0] == 0x2002 {
        return public_v4(Ipv4Addr::new(
            (s[1] >> 8) as u8,
            s[1] as u8,
            (s[2] >> 8) as u8,
            s[2] as u8,
        ));
    }
    !((s[0] & 0xfe00) == 0xfc00 // unique local, fd00::/8 among it
        || (s[0] & 0xffc0) == 0xfe80 // link local
        || (s[0] & 0xffc0) == 0xfec0 // site local, deprecated
        || (s[0] == 0x64 && s[1] == 0xff9b && s[2] == 1) // local NAT64
        || (s[0] == 0x100 && s[1..4] == [0, 0, 0]) // discard
        || (s[0] == 0x2001 && s[1] < 0x200) // IETF assignments, Teredo among them
        || (s[0] == 0x2001 && s[1] == 0xdb8)) // documentation
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_house_the_cluster_and_the_cloud_are_not_the_internet() {
        for ip in [
            "127.0.0.1",
            "127.255.255.254",
            "0.0.0.0",
            "0.1.2.3",
            "10.0.0.5",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.42.1",
            "169.254.169.254",
            "169.254.1.1",
            "100.64.0.1",
            "100.127.255.255",
            "192.0.0.170",
            "192.0.2.1",
            "198.18.0.1",
            "198.51.100.7",
            "203.0.113.9",
            "224.0.0.251",
            "239.255.255.250",
            "240.0.0.1",
            "255.255.255.255",
            "::",
            "::1",
            "fd00::1",
            "fc00::1",
            "fe80::1",
            "fec0::1",
            "ff02::1",
            "::ffff:127.0.0.1",
            "::ffff:192.168.1.1",
            "::ffff:169.254.169.254",
            "::127.0.0.1",
            "::10.0.0.1",
            "64:ff9b::7f00:1",
            "64:ff9b::a9fe:a9fe",
            "64:ff9b:1::1",
            "2002:7f00:1::",
            "2002:c0a8:101::1",
            "2001::1",
            "2001:db8::1",
            "100::1",
        ] {
            assert!(!public_ip(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn the_internet_is() {
        for ip in [
            "93.184.216.34",
            "1.1.1.1",
            "100.63.255.255",
            "100.128.0.1",
            "172.32.0.1",
            "2606:4700::1111",
            "2a00:1450:4001::200e",
            "::ffff:1.1.1.1",
            "64:ff9b::101:101",
            "2002:101:101::1",
        ] {
            assert!(public_ip(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn internal_names_are_refused_before_any_lookup() {
        for host in [
            "localhost",
            "LOCALHOST.",
            "niles.localhost",
            "searxng.productivity.svc.cluster.local",
            "metadata.google.internal",
            "printer.local",
            "nas.lan",
            "router",
            "box.home.arpa",
        ] {
            assert!(!public_name(host), "{host}");
        }
        assert!(public_name("www.bbc.co.uk"));
        assert!(public_name("upload.wikimedia.org."));
    }
}
