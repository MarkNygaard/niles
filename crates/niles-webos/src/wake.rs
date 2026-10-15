//! Waking the TV: a Wake-on-LAN "magic packet".
//!
//! Six 0xFF bytes and the MAC address sixteen times, over UDP. A TV in
//! standby listens for it when "Turn on via Wi-Fi" is switched on.
//!
//! Sent twice from a process in a cluster, because a broadcast does not
//! leave the network it was sent on: once to the TV's own address,
//! which reaches it when the router remembers its MAC (a static ARP
//! entry), and once to its network's broadcast address, which reaches
//! it when the router forwards directed broadcasts. Whichever the router
//! was set up for gets there.
//!
//! And once for every MAC the TV has: a packet for an interface it is
//! not using is ignored, and it costs nothing next to waking on the
//! wrong one.

use crate::error::{Error, Result};
use std::net::Ipv4Addr;
use tokio::net::UdpSocket;

/// Where a TV listens for its magic packet.
const PORT: u16 = 9;

/// The magic packet for `mac` ("a8:23:fe:01:02:03", dashes or colons).
pub fn magic_packet(mac: &str) -> Result<Vec<u8>> {
    let bytes: Vec<u8> = mac
        .split([':', '-'])
        .map(|b| u8::from_str_radix(b, 16))
        .collect::<std::result::Result<_, _>>()
        .map_err(|_| Error::BadMac(mac.to_string()))?;
    if bytes.len() != 6 {
        return Err(Error::BadMac(mac.to_string()));
    }
    let mut packet = vec![0xFF; 6];
    for _ in 0..16 {
        packet.extend_from_slice(&bytes);
    }
    Ok(packet)
}

/// Send the packet for each of `macs` towards the TV at `host`.
/// Succeeds when any one was sent: what a router did with it after is
/// for the TV coming on to tell.
pub async fn wake(host: &str, macs: &[String]) -> Result<()> {
    wake_at(host, PORT, macs).await
}

async fn wake_at(host: &str, port: u16, macs: &[String]) -> Result<()> {
    if macs.is_empty() {
        return Err(Error::Wake(std::io::Error::other(
            "no MAC address to wake it by",
        )));
    }
    let packets = macs
        .iter()
        .map(|mac| magic_packet(mac))
        .collect::<Result<Vec<_>>>()?;
    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    socket.set_broadcast(true)?;
    let targets = targets(host);
    // In full: comparing it with the TV's own network settings is what
    // finds a wrong one, and a MAC on the home network is no secret.
    tracing::info!("[tv] wake: {}", plan(macs, &targets, port));
    let mut sent = false;
    for target in &targets {
        for (mac, packet) in macs.iter().zip(&packets) {
            match socket.send_to(packet, (target.as_str(), port)).await {
                Ok(_) => sent = true,
                Err(e) => tracing::warn!("[tv] wake for {mac} to {target}:{port}: {e}"),
            }
        }
    }
    if sent {
        Ok(())
    } else {
        Err(Error::Wake(std::io::Error::other("no route to the TV")))
    }
}

/// The TV's own address, and its network's broadcast address when it
/// is an IPv4 address.
fn targets(host: &str) -> Vec<String> {
    let mut targets = vec![host.to_string()];
    if let Ok(ip) = host.parse::<Ipv4Addr>() {
        let [a, b, c, _] = ip.octets();
        targets.push(Ipv4Addr::new(a, b, c, 255).to_string());
    }
    targets
}

/// "magic packets for a8:…, 84:… to 192.168.4.170 (unicast) and
/// 192.168.4.255 (broadcast), UDP 9"
fn plan(macs: &[String], targets: &[String], port: u16) -> String {
    let to: Vec<String> = targets
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let kind = if i == 0 { "unicast" } else { "broadcast" };
            format!("{t} ({kind})")
        })
        .collect();
    format!(
        "magic packets for {} to {}, UDP {port}",
        macs.join(", "),
        to.join(" and ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::time::Duration;

    #[test]
    fn six_ff_then_the_mac_sixteen_times() {
        let packet = magic_packet("a8:23:fe:01:02:03").unwrap();
        assert_eq!(packet.len(), 6 + 16 * 6);
        assert_eq!(&packet[..6], &[0xFF; 6]);
        assert_eq!(&packet[6..12], &[0xA8, 0x23, 0xFE, 0x01, 0x02, 0x03]);
        assert_eq!(&packet[96..102], &[0xA8, 0x23, 0xFE, 0x01, 0x02, 0x03]);
    }

    #[test]
    fn dashes_will_do_and_nonsense_will_not() {
        assert!(magic_packet("A8-23-FE-01-02-03").is_ok());
        assert!(magic_packet("not a mac").is_err());
        assert!(magic_packet("a8:23:fe:01:02").is_err());
    }

    #[test]
    fn the_log_line_names_every_mac_and_where_it_went() {
        let macs = ["84:90:0a:63:a9:94".to_string(), "64:e4:a5:6c:ab:b4".into()];
        assert_eq!(
            plan(&macs, &targets("192.168.4.170"), 9),
            "magic packets for 84:90:0a:63:a9:94, 64:e4:a5:6c:ab:b4 to \
             192.168.4.170 (unicast) and 192.168.4.255 (broadcast), UDP 9"
        );
        assert_eq!(
            plan(&macs[..1], &targets("tv.home"), 9),
            "magic packets for 84:90:0a:63:a9:94 to tv.home (unicast), UDP 9"
        );
    }

    #[tokio::test]
    async fn a_packet_goes_out_for_every_mac() {
        let listener = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let macs = ["84:90:0a:63:a9:94".to_string(), "64:e4:a5:6c:ab:b4".into()];
        wake_at("127.0.0.1", port, &macs).await.unwrap();
        let mut heard = HashSet::new();
        let mut buf = [0u8; 256];
        while heard.len() < 2 {
            let (n, _) = tokio::time::timeout(Duration::from_secs(2), listener.recv_from(&mut buf))
                .await
                .expect("a packet for each MAC")
                .unwrap();
            assert_eq!(n, 102);
            heard.insert(buf[6..12].to_vec());
        }
        assert!(heard.contains(&vec![0x84, 0x90, 0x0a, 0x63, 0xa9, 0x94]));
        assert!(heard.contains(&vec![0x64, 0xe4, 0xa5, 0x6c, 0xab, 0xb4]));
    }

    #[tokio::test]
    async fn no_mac_is_nothing_to_send() {
        assert!(wake("127.0.0.1", &[]).await.is_err());
        assert!(wake("127.0.0.1", &["nope".into()]).await.is_err());
    }
}
