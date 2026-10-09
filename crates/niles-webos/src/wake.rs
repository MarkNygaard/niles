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

use crate::error::{Error, Result};
use std::net::Ipv4Addr;
use tokio::net::UdpSocket;

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

/// Send the packet for `mac` towards the TV at `host`.
pub async fn wake(host: &str, mac: &str) -> Result<()> {
    let packet = magic_packet(mac)?;
    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    socket.set_broadcast(true)?;
    let mut targets = vec![host.to_string()];
    if let Ok(ip) = host.parse::<Ipv4Addr>() {
        let [a, b, c, _] = ip.octets();
        targets.push(Ipv4Addr::new(a, b, c, 255).to_string());
    }
    let mut sent = false;
    for target in targets {
        match socket.send_to(&packet, (target.as_str(), 9)).await {
            Ok(_) => sent = true,
            Err(e) => tracing::debug!("[tv] wake to {target}: {e}"),
        }
    }
    if sent {
        Ok(())
    } else {
        Err(Error::Wake(std::io::Error::other("no route to the TV")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
