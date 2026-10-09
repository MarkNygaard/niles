//! Which Sonos plays in which room, worked out as it is asked.
//!
//! `[speakers]` places Sonos rooms in Niles rooms by Sonos's own id; the
//! household — where each answers, and who leads which group — is asked
//! of a speaker. Both change while Niles runs: a room placed from the
//! app, a speaker given a new address by DHCP, two rooms grouped in the
//! Sonos app. So nothing here is built at startup. The config is read on
//! every call, and the household is kept for a few seconds at most.

use niles_config::ConfigStore;
use niles_core::RoomName;
use niles_speakers::{HttpTransport, SonosClient, SonosRoom, SonosTransport};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// How long the household is trusted. Long enough that one command's
/// several calls share a read; short enough that grouping two rooms in
/// the Sonos app is seen by the next command.
const FRESH_FOR: Duration = Duration::from_secs(15);

pub struct SpeakerRegistry {
    config: Arc<ConfigStore>,
    transport: Arc<dyn SonosTransport>,
    household: Mutex<Household>,
}

#[derive(Default)]
struct Household {
    read_at: Option<Instant>,
    rooms: Vec<SonosRoom>,
}

/// A Sonos room in a Niles room, ready to be played to.
pub struct Player {
    pub sonos: SonosRoom,
    pub client: Arc<SonosClient>,
}

impl SpeakerRegistry {
    pub fn new(config: Arc<ConfigStore>) -> Self {
        Self::with_transport(config, Arc::new(HttpTransport::new()))
    }

    pub fn with_transport(config: Arc<ConfigStore>, transport: Arc<dyn SonosTransport>) -> Self {
        Self {
            config,
            transport,
            household: Mutex::new(Household::default()),
        }
    }

    /// Every Sonos room placed in `room` that the household knows of.
    /// Empty when Sonos is off, nothing is placed there, or no speaker
    /// answers — which to the caller are all "no speaker in this room".
    pub async fn in_room(&self, room: &RoomName) -> Vec<Player> {
        let cfg = self.config.current();
        if !cfg.speakers.is_configured() {
            return Vec::new();
        }
        let placed: Vec<&str> = cfg.speakers.in_room(room.as_str()).collect();
        if placed.is_empty() {
            return Vec::new();
        }
        let rooms = self.household(&cfg.speakers.host, &placed).await;
        let mut players: Vec<Player> = rooms
            .into_iter()
            .filter(|r| placed.contains(&r.id.as_str()))
            .map(|sonos| Player {
                client: self.client(&sonos.ip),
                sonos,
            })
            .collect();
        // The order Sonos lists them in, which is stable, rather than
        // the config's map order, which is not.
        players.sort_by(|a, b| a.sonos.name.cmp(&b.sonos.name));
        players
    }

    /// Where play, pause and skip go for `room`: the leader of each
    /// group its speakers are in, once each. A room grouped with the
    /// kitchen is paused by pausing the group, which is what Sonos does
    /// with the button on the speaker too.
    pub async fn leaders(&self, room: &RoomName) -> Vec<Arc<SonosClient>> {
        let players = self.in_room(room).await;
        let household = self.household.lock().await;
        let mut seen = Vec::new();
        let mut leaders = Vec::new();
        for player in &players {
            let leader = &player.sonos.coordinator;
            if seen.contains(leader) {
                continue;
            }
            seen.push(leader.clone());
            let ip = household
                .rooms
                .iter()
                .find(|r| &r.id == leader)
                .map_or(player.sonos.ip.as_str(), |r| r.ip.as_str());
            leaders.push(self.client(ip));
        }
        leaders
    }

    fn client(&self, ip: &str) -> Arc<SonosClient> {
        Arc::new(SonosClient::with_transport(ip, self.transport.clone()))
    }

    /// The household, read again when it is old or does not know a
    /// speaker that has been placed. Asked of the configured address
    /// first, then of any speaker that answered before: the one written
    /// down may be the one that was unplugged.
    async fn household(&self, host: &str, wanted: &[&str]) -> Vec<SonosRoom> {
        let mut household = self.household.lock().await;
        let fresh = household.read_at.is_some_and(|at| at.elapsed() < FRESH_FOR);
        let knows_all = wanted
            .iter()
            .all(|id| household.rooms.iter().any(|r| r.id == *id));
        if fresh && knows_all {
            return household.rooms.clone();
        }
        let mut candidates = vec![host.trim().to_string()];
        for room in &household.rooms {
            if !candidates.contains(&room.ip) {
                candidates.push(room.ip.clone());
            }
        }
        for ip in &candidates {
            match niles_speakers::household(self.transport.as_ref(), ip).await {
                Ok(rooms) => {
                    household.rooms = rooms;
                    household.read_at = Some(Instant::now());
                    return household.rooms.clone();
                }
                Err(e) => tracing::warn!("[sonos] {ip} did not describe the household: {e}"),
            }
        }
        // Better the last known than nothing: an address rarely moves
        // between two reads, and a speaker that is gone fails on its own.
        household.rooms.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex as StdMutex;

    /// Answers the household from one address and records every call.
    #[derive(Clone, Default)]
    struct FakeSonos {
        calls: Arc<StdMutex<Vec<(String, String)>>>,
    }

    const STATE: &str = r#"<ZoneGroupState><ZoneGroups>
<ZoneGroup Coordinator="RINCON_BAR" ID="g1">
  <ZoneGroupMember UUID="RINCON_BAR" Location="http://10.0.0.2:1400/x.xml" ZoneName="Living Room" HTSatChanMapSet="RINCON_BAR:LF,RF;RINCON_REAR:LR"/>
  <ZoneGroupMember UUID="RINCON_BACK" Location="http://10.0.0.6:1400/x.xml" ZoneName="Living Room Back"/>
</ZoneGroup>
<ZoneGroup Coordinator="RINCON_MOVE" ID="g2">
  <ZoneGroupMember UUID="RINCON_MOVE" Location="http://10.0.0.4:1400/x.xml" ZoneName="Sonos Move"/>
</ZoneGroup>
</ZoneGroups></ZoneGroupState>"#;

    #[async_trait]
    impl SonosTransport for FakeSonos {
        async fn send_action(
            &self,
            endpoint: &str,
            soap_action: &str,
            _body: &str,
        ) -> niles_speakers::Result<String> {
            self.calls
                .lock()
                .unwrap()
                .push((endpoint.to_string(), soap_action.to_string()));
            if !endpoint.contains("10.0.0.4") {
                return Err(niles_speakers::Error::ParseResponse {
                    reason: "unplugged".into(),
                });
            }
            let escaped = STATE
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('"', "&quot;");
            Ok(format!("<ZoneGroupState>{escaped}</ZoneGroupState>"))
        }
    }

    fn registry(toml: &str) -> (SpeakerRegistry, FakeSonos) {
        let transport = FakeSonos::default();
        let config = Arc::new(ConfigStore::from_str_in_memory(toml).unwrap());
        (
            SpeakerRegistry::with_transport(config, Arc::new(transport.clone())),
            transport,
        )
    }

    const PLACED: &str = r#"
[speakers]
host = "10.0.0.4"
[speakers.sonos.RINCON_BAR]
room = "living_room"
[speakers.sonos.RINCON_BACK]
room = "living_room"
[speakers.sonos.RINCON_MOVE]
room = "kitchen"
"#;

    fn room(name: &str) -> RoomName {
        RoomName::parse(name).unwrap()
    }

    #[tokio::test]
    async fn a_room_has_every_sonos_placed_in_it() {
        let (speakers, _) = registry(PLACED);
        let names: Vec<_> = speakers
            .in_room(&room("living_room"))
            .await
            .into_iter()
            .map(|p| p.sonos.name)
            .collect();
        assert_eq!(names, ["Living Room", "Living Room Back"]);
    }

    #[tokio::test]
    async fn nothing_placed_is_no_speaker() {
        let (speakers, transport) = registry(PLACED);
        assert!(speakers.in_room(&room("bedroom")).await.is_empty());
        // Not worth asking Sonos about a room nothing is placed in.
        assert!(transport.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn switched_off_is_no_speaker() {
        let (speakers, transport) = registry(&PLACED.replace(
            "host = \"10.0.0.4\"",
            "host = \"10.0.0.4\"\nenabled = false",
        ));
        assert!(speakers.in_room(&room("kitchen")).await.is_empty());
        assert!(transport.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_grouped_room_is_paused_through_its_leader_once() {
        // Living Room Back plays with the soundbar: one group, so one
        // place to send play and pause.
        let (speakers, _) = registry(PLACED);
        let leaders = speakers.leaders(&room("living_room")).await;
        assert_eq!(leaders.len(), 1);
        assert_eq!(
            format!("{:?}", leaders[0]),
            format!("{:?}", SonosClient::new("10.0.0.2"))
        );
    }

    #[tokio::test]
    async fn one_read_serves_the_calls_that_follow() {
        let (speakers, transport) = registry(PLACED);
        speakers.in_room(&room("living_room")).await;
        speakers.in_room(&room("kitchen")).await;
        assert_eq!(transport.calls.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn asks_a_speaker_that_answered_before_when_the_written_one_does_not() {
        let (speakers, transport) = registry(PLACED);
        speakers.in_room(&room("kitchen")).await;
        // The written address stops answering; the household is old.
        let cfg = PLACED.replace("10.0.0.4", "10.0.0.9");
        speakers
            .config
            .apply(
                &toml::from_str(&cfg).unwrap(),
                niles_config::ChangeSource::Api,
            )
            .await
            .unwrap();
        speakers.household.lock().await.read_at = None;
        assert_eq!(speakers.in_room(&room("kitchen")).await.len(), 1);
        let asked: Vec<_> = transport
            .calls
            .lock()
            .unwrap()
            .iter()
            .map(|(endpoint, _)| endpoint.clone())
            .collect();
        assert!(asked[1].contains("10.0.0.9"), "{asked:?}");
        assert!(
            asked.iter().skip(2).any(|e| e.contains("10.0.0.4")),
            "{asked:?}"
        );
    }
}
