//! The Sonos household: every room, the speaker that answers for it,
//! and how they are grouped right now.
//!
//! Read from any one speaker, because each of them can describe all the
//! others. Discovery (SSDP) is a multicast that does not reach a process
//! in a cluster network, so one address is what Niles is given, and this
//! is what it learns from it.

use crate::error::{Error, Result};
use crate::transport::{SonosTransport, extract_tag};
use regex::Regex;
use std::sync::LazyLock;

/// One Sonos room — a single speaker, a stereo pair, or a soundbar with
/// its rear speakers and sub. Always played as one, so the speakers
/// inside it are not listed.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SonosRoom {
    /// Sonos's own identity for it (`RINCON_…`). Kept rather than the
    /// address, which DHCP can change.
    pub id: String,
    /// What the Sonos app calls it: "Living Room".
    pub name: String,
    /// Where it answers right now.
    pub ip: String,
    /// A soundbar with speakers around it: the one that can play a TV.
    pub home_theater: bool,
    /// The room leading the group it is in. Its own id when it plays
    /// alone. Play, pause and skip go to the leader; volume to each.
    pub coordinator: String,
}

static GROUP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<ZoneGroup\s[^>]*?Coordinator="([^"]+)"[^>]*>(.*?)</ZoneGroup>"#)
        .expect("valid regex")
});
static MEMBER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<ZoneGroupMember\s([^>]*?)/?>").expect("valid regex"));
static ATTRIBUTE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(\w+)="([^"]*)""#).expect("valid regex"));
static HOST: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://([^:/]+)").expect("valid regex"));

const TOPOLOGY_SERVICE: &str = "urn:schemas-upnp-org:service:ZoneGroupTopology:1";

/// Ask the speaker at `ip` for the whole household.
pub async fn household(transport: &dyn SonosTransport, ip: &str) -> Result<Vec<SonosRoom>> {
    let body = crate::client::soap_envelope(TOPOLOGY_SERVICE, "GetZoneGroupState", "");
    let response = transport
        .send_action(
            &format!("http://{ip}:1400/ZoneGroupTopology/Control"),
            &format!("{TOPOLOGY_SERVICE}#GetZoneGroupState"),
            &body,
        )
        .await?;
    let state = extract_tag(&response, "ZoneGroupState").ok_or_else(|| Error::ParseResponse {
        reason: "missing <ZoneGroupState>".into(),
    })?;
    Ok(parse(&unescape(&state)))
}

/// The rooms in a `ZoneGroupState` document, as Sonos lists them.
fn parse(state: &str) -> Vec<SonosRoom> {
    let mut rooms = Vec::new();
    for group in GROUP.captures_iter(state) {
        let coordinator = &group[1];
        for member in MEMBER.captures_iter(&group[2]) {
            let attributes: Vec<(&str, &str)> = ATTRIBUTE
                .captures_iter(member.get(1).map_or("", |m| m.as_str()))
                .map(|c| {
                    let (_, [key, value]) = c.extract();
                    (key, value)
                })
                .collect();
            let get = |key: &str| attributes.iter().find(|(k, _)| *k == key).map(|(_, v)| *v);
            // The second half of a stereo pair, and a Boost or Bridge:
            // not rooms anybody plays to.
            if get("Invisible") == Some("1") || get("IsZoneBridge") == Some("1") {
                continue;
            }
            let (Some(id), Some(name), Some(ip)) = (
                get("UUID"),
                get("ZoneName"),
                get("Location")
                    .and_then(|l| HOST.captures(l))
                    .map(|c| c[1].to_string()),
            ) else {
                continue;
            };
            rooms.push(SonosRoom {
                id: id.to_string(),
                name: name.to_string(),
                ip,
                home_theater: get("HTSatChanMapSet").is_some_and(|m| !m.is_empty()),
                coordinator: coordinator.to_string(),
            });
        }
    }
    rooms
}

/// The document arrives as text inside a text element, so once escaped.
pub(crate) fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape a real household answers with, trimmed: a speaker on
    /// its own, a soundbar with two rear speakers, and a stereo pair.
    const STATE: &str = r#"<ZoneGroupState><ZoneGroups>
<ZoneGroup Coordinator="RINCON_KITCHEN" ID="RINCON_KITCHEN:1">
  <ZoneGroupMember UUID="RINCON_KITCHEN" Location="http://10.0.0.4:1400/xml/device_description.xml" ZoneName="Kitchen" Icon="" />
</ZoneGroup>
<ZoneGroup Coordinator="RINCON_BAR" ID="RINCON_BAR:2">
  <ZoneGroupMember UUID="RINCON_BAR" Location="http://10.0.0.2:1400/xml/device_description.xml" ZoneName="Living Room" HTSatChanMapSet="RINCON_BAR:LF,RF;RINCON_REAR1:LR">
    <Satellite UUID="RINCON_REAR1" Location="http://10.0.0.3:1400/xml/device_description.xml" ZoneName="Living Room" Invisible="1"/>
  </ZoneGroupMember>
  <ZoneGroupMember UUID="RINCON_BACK" Location="http://10.0.0.6:1400/xml/device_description.xml" ZoneName="Living Room Back"/>
</ZoneGroup>
<ZoneGroup Coordinator="RINCON_LEFT" ID="RINCON_LEFT:3">
  <ZoneGroupMember UUID="RINCON_LEFT" Location="http://10.0.0.7:1400/xml/device_description.xml" ZoneName="Office" ChannelMapSet="RINCON_LEFT:LF,LF;RINCON_RIGHT:RF,RF"/>
  <ZoneGroupMember UUID="RINCON_RIGHT" Location="http://10.0.0.8:1400/xml/device_description.xml" ZoneName="Office" Invisible="1"/>
</ZoneGroup>
</ZoneGroups></ZoneGroupState>"#;

    #[test]
    fn lists_each_room_once() {
        let names: Vec<_> = parse(STATE).into_iter().map(|r| r.name).collect();
        assert_eq!(
            names,
            ["Kitchen", "Living Room", "Living Room Back", "Office"]
        );
    }

    #[test]
    fn knows_where_each_answers() {
        let rooms = parse(STATE);
        assert_eq!(rooms[0].id, "RINCON_KITCHEN");
        assert_eq!(rooms[0].ip, "10.0.0.4");
    }

    #[test]
    fn a_soundbar_with_rear_speakers_is_a_home_theater() {
        let rooms = parse(STATE);
        assert!(rooms[1].home_theater);
        assert!(!rooms[0].home_theater);
        // A stereo pair is two speakers, but not a television's.
        assert!(!rooms[3].home_theater);
    }

    #[test]
    fn a_room_grouped_with_another_follows_its_leader() {
        let rooms = parse(STATE);
        assert_eq!(rooms[2].name, "Living Room Back");
        assert_eq!(rooms[2].coordinator, "RINCON_BAR");
        assert_eq!(rooms[0].coordinator, "RINCON_KITCHEN");
    }

    #[test]
    fn reads_the_document_as_sonos_sends_it() {
        // Escaped once, inside the SOAP response's own element.
        let escaped = STATE
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;");
        assert_eq!(parse(&unescape(&escaped)), parse(STATE));
    }

    /// Against a real household: `SONOS_HOST=192.168.10.174 cargo test
    /// -p niles-speakers -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn live_household() {
        let host = std::env::var("SONOS_HOST").expect("SONOS_HOST");
        let rooms = household(&crate::HttpTransport::new(), &host)
            .await
            .unwrap();
        for room in &rooms {
            let media = crate::SonosClient::new(room.ip.clone())
                .media()
                .await
                .unwrap();
            println!(
                "{room:?}
    loaded: {}",
                media.uri
            );
        }
        assert!(!rooms.is_empty());
        let favorites = crate::SonosClient::new(host).favorites().await.unwrap();
        for favorite in &favorites {
            println!(
                "favorite {:?} station={} {}",
                favorite.title, favorite.station, favorite.uri
            );
        }
    }
}
