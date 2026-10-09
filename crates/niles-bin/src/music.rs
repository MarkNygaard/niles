//! Music: what "play" means, and where it plays.
//!
//! Sonos does the streaming; this decides what to hand it. A name is
//! looked for among the household's Sonos Favorites first — what
//! somebody starred is what they meant — then among TuneIn's stations.
//! A station whose name is exactly what was said plays at once; several
//! that merely contain it go back as a choice, because "P4" is a dozen
//! stations in three countries and only the model knows which house it
//! is standing in.
//!
//! A room plays as one: every Sonos placed in it is grouped behind one
//! of them before anything starts, the soundbar when there is one. That
//! is also what takes the TV off: music asked for in a room replaces
//! whatever its soundbar was playing.

use crate::speakers::{Player, SpeakerRegistry};
use niles_core::RoomName;
use niles_speakers::tunein::{self, Account, Station};
use niles_speakers::{Favorite, TransportState, is_radio};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// A station a room played, kept so "play the radio" can play it again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Remembered {
    pub title: String,
    pub uri: String,
    pub metadata: String,
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// Started; what it was.
    Playing(String),
    /// Spread to more rooms.
    Spread,
    Paused,
    /// Several stations could be meant.
    Choose(Vec<Station>),
    NotFound(String),
    /// "Play the radio" in a room that has never had a station.
    WhichStation,
    NoSpeaker,
    /// Nothing is playing to spread.
    NothingPlaying,
    /// What would be spread is the TV, which stays where it is.
    TvStaysPut,
    Failed(String),
}

pub struct Music {
    speakers: Arc<SpeakerRegistry>,
    http: reqwest::Client,
    last: Mutex<HashMap<String, Remembered>>,
    store: Option<Arc<niles_db::PostgresRoomMusic>>,
}

impl Music {
    /// With what each room last played, read from `store` when there is
    /// one. A store that cannot be read starts empty and says so: losing
    /// "the last station" costs one "which station?".
    pub async fn new(
        speakers: Arc<SpeakerRegistry>,
        store: Option<Arc<niles_db::PostgresRoomMusic>>,
    ) -> Self {
        let mut last = HashMap::new();
        if let Some(store) = &store {
            match store.load().await {
                Ok(Some(document)) => match serde_json::from_str(&document) {
                    Ok(stored) => last = stored,
                    Err(e) => tracing::warn!("[music] the stored stations would not parse: {e}"),
                },
                Ok(None) => {}
                Err(e) => tracing::warn!("[music] could not read the stored stations: {e}"),
            }
        }
        Self {
            speakers,
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(8))
                .build()
                .expect("reqwest TLS init"),
            last: Mutex::new(last),
            store,
        }
    }

    /// "Play the radio" — the station named, or this room's last one.
    pub async fn radio(&self, room: &RoomName, station: Option<&str>) -> Outcome {
        let players = self.speakers.in_room(room).await;
        if players.is_empty() {
            return Outcome::NoSpeaker;
        }
        if let Some(station) = station {
            return self.find(room, &players, station).await;
        }
        if let Some(last) = self.remembered(room) {
            return self
                .start(
                    room,
                    &players,
                    &last.uri,
                    &last.metadata,
                    false,
                    &last.title,
                )
                .await;
        }
        // Started from the Sonos app, so Niles never saw it — but the
        // speaker still has it loaded.
        for player in &players {
            if let Ok(media) = player.client.media().await
                && media.is_radio()
            {
                let title = title_in(&media.metadata).unwrap_or_else(|| "the radio".into());
                return self
                    .start(room, &players, &media.uri, &media.metadata, false, &title)
                    .await;
            }
        }
        Outcome::WhichStation
    }

    /// "Play X" — a favorite, or a station.
    pub async fn play(&self, room: &RoomName, query: &str) -> Outcome {
        let players = self.speakers.in_room(room).await;
        if players.is_empty() {
            return Outcome::NoSpeaker;
        }
        self.find(room, &players, query).await
    }

    /// A station picked from a [`Outcome::Choose`] by its TuneIn id.
    pub async fn station(&self, room: &RoomName, id: &str, name: &str) -> Outcome {
        let players = self.speakers.in_room(room).await;
        if players.is_empty() {
            return Outcome::NoSpeaker;
        }
        let favorites = self.favorites(&players).await;
        let station = Station {
            id: id.to_string(),
            name: name.to_string(),
            about: String::new(),
        };
        self.play_station(room, &players, &favorites, &station)
            .await
    }

    /// "Play it in the kitchen too" / "play it everywhere": what plays
    /// where it was said — or, said from a room playing nothing, the one
    /// thing playing anywhere — joined by `to`, or by every room.
    pub async fn spread(&self, from: Option<&RoomName>, to: Option<&RoomName>) -> Outcome {
        let all = self.speakers.everywhere().await;
        if all.is_empty() {
            return Outcome::NoSpeaker;
        }
        let mut candidates: Vec<&str> = Vec::new();
        if let Some(from) = from {
            for player in self.speakers.in_room(from).await.iter() {
                if let Some(leader) = all.iter().find(|p| p.sonos.id == player.sonos.coordinator) {
                    push_once(&mut candidates, &leader.sonos.id);
                }
            }
        }
        for player in &all {
            if player.sonos.coordinator == player.sonos.id {
                push_once(&mut candidates, &player.sonos.id);
            }
        }
        let mut source = None;
        for id in candidates {
            let Some(leader) = all.iter().find(|p| p.sonos.id == id) else {
                continue;
            };
            if matches!(
                leader.client.get_transport_state().await,
                Ok(TransportState::Playing)
            ) {
                source = Some(leader);
                break;
            }
        }
        let Some(leader) = source else {
            return Outcome::NothingPlaying;
        };
        if leader.client.media().await.is_ok_and(|m| m.is_tv()) {
            return Outcome::TvStaysPut;
        }
        let targets = match to {
            Some(room) => self.speakers.in_room(room).await,
            None => all.clone(),
        };
        if targets.is_empty() {
            return Outcome::NoSpeaker;
        }
        let leader_id = leader.sonos.id.clone();
        let mut failed = None;
        for target in targets
            .iter()
            .filter(|t| t.sonos.id != leader_id && t.sonos.coordinator != leader_id)
        {
            if let Err(e) = target.client.join(&leader_id).await {
                failed = Some(format!("{}: {e}", target.sonos.name));
            }
        }
        self.speakers.forget().await;
        match failed {
            Some(e) => Outcome::Failed(e),
            None => Outcome::Spread,
        }
    }

    /// "Pause the kitchen". A room following another room's music
    /// leaves the group, and the rest play on; a room leading a group
    /// pauses it, all of it, as the button on a Sonos does.
    pub async fn pause(&self, room: &RoomName) -> Outcome {
        let players = self.speakers.in_room(room).await;
        if players.is_empty() {
            return Outcome::NoSpeaker;
        }
        let here: Vec<&str> = players.iter().map(|p| p.sonos.id.as_str()).collect();
        let mut paused: Vec<&str> = Vec::new();
        let mut result = Ok(());
        for player in &players {
            let leader = player.sonos.coordinator.as_str();
            let step = if here.contains(&leader) {
                if paused.contains(&leader) {
                    continue;
                }
                paused.push(leader);
                match players.iter().find(|p| p.sonos.id == leader) {
                    Some(leader) => leader.client.pause().await,
                    None => continue,
                }
            } else {
                player.client.go_solo().await
            };
            if let Err(e) = step {
                result = Err(e);
            }
        }
        self.speakers.forget().await;
        match result {
            Ok(()) => Outcome::Paused,
            Err(e) => Outcome::Failed(e.to_string()),
        }
    }

    async fn find(&self, room: &RoomName, players: &[Player], query: &str) -> Outcome {
        let favorites = self.favorites(players).await;
        if let Some(favorite) = best_favorite(&favorites, query) {
            return self
                .start(
                    room,
                    players,
                    &favorite.uri,
                    &favorite.metadata,
                    !favorite.station,
                    &favorite.title,
                )
                .await;
        }
        let stations = match tunein::search(&self.http, query).await {
            Ok(stations) => stations,
            Err(e) => return Outcome::Failed(format!("TuneIn did not answer: {e}")),
        };
        if let Some(station) = stations.iter().find(|s| words(&s.name) == words(query)) {
            return self.play_station(room, players, &favorites, station).await;
        }
        if stations.is_empty() {
            return Outcome::NotFound(query.to_string());
        }
        Outcome::Choose(stations.into_iter().take(6).collect())
    }

    async fn favorites(&self, players: &[Player]) -> Vec<Favorite> {
        match leader_of(players).client.favorites().await {
            Ok(favorites) => favorites,
            Err(e) => {
                tracing::warn!("[music] could not read the Sonos favorites: {e}");
                Vec::new()
            }
        }
    }

    async fn play_station(
        &self,
        room: &RoomName,
        players: &[Player],
        favorites: &[Favorite],
        station: &Station,
    ) -> Outcome {
        // The household's own TuneIn, learned from a station it plays.
        let remembered: Vec<String> = self.lock().values().map(|r| r.uri.clone()).collect();
        let account = favorites
            .iter()
            .map(|f| f.uri.as_str())
            .chain(remembered.iter().map(String::as_str))
            .find_map(Account::of)
            .unwrap_or(Account::OPEN);
        let (uri, metadata) = account.play(station);
        self.start(room, players, &uri, &metadata, false, &station.name)
            .await
    }

    /// Group the room behind its leader, load, play.
    async fn start(
        &self,
        room: &RoomName,
        players: &[Player],
        uri: &str,
        metadata: &str,
        queue: bool,
        title: &str,
    ) -> Outcome {
        let leader = leader_of(players);
        let result: niles_speakers::Result<()> = async {
            if leader.sonos.coordinator != leader.sonos.id {
                leader.client.go_solo().await?;
            }
            for player in players {
                if player.sonos.id != leader.sonos.id && player.sonos.coordinator != leader.sonos.id
                {
                    player.client.join(&leader.sonos.id).await?;
                }
            }
            if queue {
                leader
                    .client
                    .load_queue(&leader.sonos.id, uri, metadata)
                    .await?;
            } else {
                leader.client.load(uri, metadata).await?;
            }
            leader.client.play().await
        }
        .await;
        self.speakers.forget().await;
        if let Err(e) = result {
            return Outcome::Failed(e.to_string());
        }
        if is_radio(uri) {
            self.remember(
                room,
                Remembered {
                    title: title.to_string(),
                    uri: uri.to_string(),
                    metadata: metadata.to_string(),
                },
            );
        }
        Outcome::Playing(title.to_string())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Remembered>> {
        self.last.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn remembered(&self, room: &RoomName) -> Option<Remembered> {
        self.lock().get(room.as_str()).cloned()
    }

    fn remember(&self, room: &RoomName, station: Remembered) {
        let document = {
            let mut last = self.lock();
            if last.get(room.as_str()) == Some(&station) {
                return;
            }
            last.insert(room.as_str().to_string(), station);
            serde_json::to_string(&*last)
        };
        let (Some(store), Ok(document)) = (self.store.clone(), document) else {
            return;
        };
        // Off the reply's path: a station not saved costs a "which
        // station?" after the next restart, a slow reply costs now.
        tokio::spawn(async move {
            if let Err(e) = store.store(&document).await {
                tracing::warn!("[music] could not keep the station: {e}");
            }
        });
    }
}

/// The Sonos a room's group is built around: the soundbar, which has to
/// lead to leave the TV, or else the first.
fn leader_of(players: &[Player]) -> &Player {
    players
        .iter()
        .find(|p| p.sonos.home_theater)
        .unwrap_or(&players[0])
}

fn push_once<'a>(list: &mut Vec<&'a str>, id: &'a str) {
    if !list.contains(&id) {
        list.push(id);
    }
}

/// Lowercase words, punctuation dropped: "DR P4 Østjylland 95.9" is
/// `dr p4 østjylland 95 9`.
fn words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty() && *w != "the")
        .map(str::to_string)
        .collect()
}

/// The favorite every word of `query` is in, the shortest when several
/// are: "p4" is "DR P4 Østjylland", and "dr p3" is not "DR P3 Classics".
fn best_favorite<'a>(favorites: &'a [Favorite], query: &str) -> Option<&'a Favorite> {
    let wanted = words(query);
    if wanted.is_empty() {
        return None;
    }
    favorites
        .iter()
        .filter(|f| {
            let title = words(&f.title);
            wanted.iter().all(|w| title.contains(w))
        })
        .min_by_key(|f| words(&f.title).len())
}

/// The `dc:title` in a DIDL-Lite document.
fn title_in(metadata: &str) -> Option<String> {
    let start = metadata.find("<dc:title>")? + "<dc:title>".len();
    let end = metadata[start..].find("</dc:title>")? + start;
    Some(metadata[start..end].to_string())
}

// ---- Tools ------------------------------------------------------------------

use niles_tools::{Error as ToolError, Result as ToolResult, Tool, ToolDescriptor, ToolRegistry};
use serde_json::{Value, json};

fn room_arg(tool: &str, args: &Value, key: &str) -> ToolResult<Option<RoomName>> {
    let Some(raw) = args.get(key).and_then(Value::as_str) else {
        return Ok(None);
    };
    let canonical = raw.trim().to_lowercase().replace([' ', '-'], "_");
    RoomName::parse(&canonical)
        .map(Some)
        .map_err(|e| ToolError::InvalidArgs {
            tool: tool.into(),
            reason: format!("{raw:?} is not a room: {e}"),
        })
}

fn required_room(tool: &str, args: &Value) -> ToolResult<RoomName> {
    room_arg(tool, args, "room")?.ok_or_else(|| ToolError::InvalidArgs {
        tool: tool.into(),
        reason: "which room? Use the room the person is in unless they named one".into(),
    })
}

/// What the model is told happened.
fn reported(outcome: Outcome, room: Option<&RoomName>) -> Value {
    let room = room.map(|r| r.as_str().replace('_', " "));
    match outcome {
        Outcome::Playing(what) => json!({ "playing": what, "room": room }),
        Outcome::Spread => {
            json!({ "spread": true, "to": room.unwrap_or_else(|| "every room".into()) })
        }
        Outcome::Paused => json!({ "paused": room }),
        Outcome::Choose(stations) => json!({
            "choose_from": stations.iter().map(|s| json!({
                "station_id": s.id, "name": s.name, "about": s.about,
            })).collect::<Vec<_>>(),
            "next": "Call play_radio again with the station_id of the one meant — prefer a station from the household's own country — or ask which.",
        }),
        Outcome::NotFound(what) => json!({ "error": format!("found nothing called {what:?}") }),
        Outcome::WhichStation => {
            json!({ "error": "this room has not played a station yet; ask which station" })
        }
        Outcome::NoSpeaker => {
            json!({ "error": format!("there is no speaker in the {}", room.unwrap_or_default()) })
        }
        Outcome::NothingPlaying => json!({ "error": "nothing is playing to share" }),
        Outcome::TvStaysPut => {
            json!({ "error": "what plays is the TV, which stays in its own room" })
        }
        Outcome::Failed(e) => json!({ "error": e }),
    }
}

struct PlayRadio(Arc<Music>);

#[async_trait::async_trait]
impl Tool for PlayRadio {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "play_radio".into(),
            description: "Play a radio station on the Sonos in a room, grouping every speaker in it.                 No station: the one that room played last. A station by name is looked for in                 the household's Sonos favorites, then on TuneIn; when several could be meant,                 the result lists them, and you call again with the station_id."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "room": { "type": "string", "description": "Where to play it." },
                    "station": { "type": "string", "description": "A station's name, as said: \"P4\", \"DR P1\"." },
                    "station_id": { "type": "string", "description": "A TuneIn id from an earlier result's choose_from." }
                },
                "required": ["room"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> ToolResult<Value> {
        let room = required_room("play_radio", &args)?;
        let station = args.get("station").and_then(Value::as_str);
        let outcome = match args.get("station_id").and_then(Value::as_str) {
            Some(id) => self.0.station(&room, id, station.unwrap_or(id)).await,
            None => self.0.radio(&room, station).await,
        };
        Ok(reported(outcome, Some(&room)))
    }
}

struct PlayMusic(Arc<Music>);

#[async_trait::async_trait]
impl Tool for PlayMusic {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "play_music".into(),
            description: "Play something by name on the Sonos in a room — a playlist, album or                 station the household saved as a Sonos favorite, or a radio station. Every                 speaker in the room plays it together, and it replaces what was playing,                 the TV included."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "room": { "type": "string" },
                    "query": { "type": "string", "description": "What was asked for, as said." }
                },
                "required": ["room", "query"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> ToolResult<Value> {
        let room = required_room("play_music", &args)?;
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if query.is_empty() {
            return Ok(json!({ "error": "what should I play?" }));
        }
        Ok(reported(self.0.play(&room, query).await, Some(&room)))
    }
}

struct PlayElsewhere(Arc<Music>);

#[async_trait::async_trait]
impl Tool for PlayElsewhere {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "play_elsewhere".into(),
            description: "Spread what is playing to another room, or to every room:                 \"play it in the kitchen too\", \"play it everywhere\". The music comes from                 from_room, or from whatever is playing when that room plays nothing.                 TV sound is never spread."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "room": { "type": "string", "description": "The room to add. Leave out for every room." },
                    "from_room": { "type": "string", "description": "Where the music plays now, usually where the person is." }
                },
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> ToolResult<Value> {
        let to = room_arg("play_elsewhere", &args, "room")?;
        let from = room_arg("play_elsewhere", &args, "from_room")?;
        Ok(reported(
            self.0.spread(from.as_ref(), to.as_ref()).await,
            to.as_ref(),
        ))
    }
}

pub fn register(reg: &mut ToolRegistry, music: Arc<Music>) {
    reg.register(Box::new(PlayRadio(music.clone())));
    reg.register(Box::new(PlayMusic(music.clone())));
    reg.register(Box::new(PlayElsewhere(music)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use niles_config::ConfigStore;
    use niles_speakers::SonosTransport;
    use std::collections::HashMap as Map;

    /// A household of three: a soundbar and a speaker at the back of
    /// the living room, and a Move in the kitchen. Answers each action
    /// by name and records everything it is asked.
    #[derive(Clone, Default)]
    struct House {
        /// Who each Sonos follows: its own id when it plays alone.
        leaders: Map<&'static str, &'static str>,
        /// What each address has loaded.
        loaded: Map<&'static str, &'static str>,
        playing: Vec<&'static str>,
        calls: Arc<Mutex<Vec<(String, String, String)>>>,
    }

    const SONOS: [(&str, &str, &str, bool); 3] = [
        ("RINCON_BAR", "Living Room", "10.0.0.2", true),
        ("RINCON_BACK", "Living Room Back", "10.0.0.6", false),
        ("RINCON_MOVE", "Sonos Move", "10.0.0.4", false),
    ];

    impl House {
        fn new() -> Self {
            let mut house = House::default();
            for (id, ..) in SONOS {
                house.leaders.insert(id, id);
            }
            house
        }

        fn state(&self) -> String {
            let groups: String = SONOS
                .iter()
                .filter(|(id, ..)| self.leaders[id] == *id)
                .map(|(leader, ..)| {
                    let members: String = SONOS
                        .iter()
                        .filter(|(id, ..)| self.leaders[id] == *leader)
                        .map(|(id, name, ip, bar)| {
                            let ht = if *bar {
                                r#" HTSatChanMapSet="x:LF,RF;y:LR""#
                            } else {
                                ""
                            };
                            format!(
                                r#"<ZoneGroupMember UUID="{id}" Location="http://{ip}:1400/x.xml" ZoneName="{name}"{ht}/>"#
                            )
                        })
                        .collect();
                    format!(r#"<ZoneGroup Coordinator="{leader}" ID="g">{members}</ZoneGroup>"#)
                })
                .collect();
            escaped(&format!(
                "<ZoneGroupState><ZoneGroups>{groups}</ZoneGroups></ZoneGroupState>"
            ))
        }

        /// What was asked of `ip`: each action's name and body, in order.
        fn asked(&self, ip: &str) -> Vec<(String, String)> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(endpoint, ..)| endpoint.contains(ip))
                .map(|(_, action, body)| {
                    (action.rsplit('#').next().unwrap().to_string(), body.clone())
                })
                .collect()
        }

        fn did(&self, ip: &str, action: &str, containing: &str) -> bool {
            self.asked(ip)
                .iter()
                .any(|(a, b)| a == action && b.contains(containing))
        }
    }

    fn escaped(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    }

    #[async_trait::async_trait]
    impl SonosTransport for House {
        async fn send_action(
            &self,
            endpoint: &str,
            action: &str,
            body: &str,
        ) -> niles_speakers::Result<String> {
            self.calls
                .lock()
                .unwrap()
                .push((endpoint.into(), action.into(), body.into()));
            let ip = SONOS
                .iter()
                .map(|(_, _, ip, _)| *ip)
                .find(|ip| endpoint.contains(ip))
                .unwrap_or("");
            Ok(match action.rsplit('#').next().unwrap() {
                "GetZoneGroupState" => {
                    format!("<ZoneGroupState>{}</ZoneGroupState>", self.state())
                }
                "GetMediaInfo" => format!(
                    "<CurrentURI>{}</CurrentURI>",
                    escaped(self.loaded.get(ip).copied().unwrap_or(""))
                ),
                "GetTransportInfo" => format!(
                    "<CurrentTransportState>{}</CurrentTransportState>",
                    if self.playing.contains(&ip) {
                        "PLAYING"
                    } else {
                        "STOPPED"
                    }
                ),
                "Browse" => {
                    let item = r#"<item id="FV:2/1"><dc:title>DR P3</dc:title><res>x-sonosapi-stream:s24861?sid=333&amp;flags=8224&amp;sn=14</res><r:resMD>&lt;DIDL-Lite&gt;&lt;/DIDL-Lite&gt;</r:resMD></item>"#;
                    format!(
                        "<Result>{}</Result>",
                        escaped(&format!("<DIDL-Lite>{item}</DIDL-Lite>"))
                    )
                }
                _ => String::new(),
            })
        }
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

    async fn music(house: &House) -> Music {
        let config = Arc::new(ConfigStore::from_str_in_memory(PLACED).unwrap());
        let speakers = Arc::new(SpeakerRegistry::with_transport(
            config,
            Arc::new(house.clone()),
        ));
        Music::new(speakers, None).await
    }

    fn room(name: &str) -> RoomName {
        RoomName::parse(name).unwrap()
    }

    #[tokio::test]
    async fn a_station_plays_on_the_whole_room_behind_the_soundbar() {
        let house = House::new();
        let outcome = music(&house)
            .await
            .radio(&room("living_room"), Some("p3"))
            .await;
        assert_eq!(outcome, Outcome::Playing("DR P3".into()));
        assert!(house.did("10.0.0.6", "SetAVTransportURI", "x-rincon:RINCON_BAR"));
        let bar = house.asked("10.0.0.2");
        let actions: Vec<_> = bar.iter().map(|(a, _)| a.as_str()).collect();
        assert!(
            actions.ends_with(&["SetAVTransportURI", "Play"]),
            "{actions:?}"
        );
        assert!(house.did(
            "10.0.0.2",
            "SetAVTransportURI",
            "s24861?sid=333&amp;flags=8224&amp;sn=14"
        ));
    }

    #[tokio::test]
    async fn the_radio_is_the_station_the_room_had_loaded() {
        let mut house = House::new();
        house.loaded.insert(
            "10.0.0.4",
            "x-sonosapi-stream:s10136?sid=333&flags=8232&sn=14",
        );
        let outcome = music(&house).await.radio(&room("kitchen"), None).await;
        assert!(matches!(outcome, Outcome::Playing(_)), "{outcome:?}");
        assert!(house.did("10.0.0.4", "SetAVTransportURI", "s10136"));
    }

    #[tokio::test]
    async fn the_next_radio_is_the_station_niles_played() {
        let house = House::new();
        let music = music(&house).await;
        music.radio(&room("kitchen"), Some("dr p3")).await;
        assert_eq!(
            music.radio(&room("kitchen"), None).await,
            Outcome::Playing("DR P3".into())
        );
    }

    #[tokio::test]
    async fn a_room_that_never_played_a_station_is_asked_which() {
        let house = House::new();
        assert_eq!(
            music(&house).await.radio(&room("kitchen"), None).await,
            Outcome::WhichStation
        );
    }

    #[tokio::test]
    async fn everywhere_joins_every_other_room_to_what_plays() {
        let mut house = House::new();
        house.playing.push("10.0.0.2");
        let outcome = music(&house)
            .await
            .spread(Some(&room("living_room")), None)
            .await;
        assert_eq!(outcome, Outcome::Spread);
        for ip in ["10.0.0.4", "10.0.0.6"] {
            assert!(
                house.did(ip, "SetAVTransportURI", "x-rincon:RINCON_BAR"),
                "{ip}"
            );
        }
    }

    #[tokio::test]
    async fn the_tv_is_not_spread() {
        let mut house = House::new();
        house.playing.push("10.0.0.2");
        house
            .loaded
            .insert("10.0.0.2", "x-sonos-htastream:RINCON_BAR:spdif");
        let outcome = music(&house)
            .await
            .spread(Some(&room("living_room")), None)
            .await;
        assert_eq!(outcome, Outcome::TvStaysPut);
        assert!(!house.did("10.0.0.4", "SetAVTransportURI", ""));
    }

    #[tokio::test]
    async fn pausing_a_room_that_follows_another_takes_it_out_of_the_group() {
        let mut house = House::new();
        house.leaders.insert("RINCON_MOVE", "RINCON_BAR");
        let outcome = music(&house).await.pause(&room("kitchen")).await;
        assert_eq!(outcome, Outcome::Paused);
        assert!(house.did("10.0.0.4", "BecomeCoordinatorOfStandaloneGroup", ""));
        // The living room plays on.
        assert!(!house.did("10.0.0.2", "Pause", ""));
    }

    #[tokio::test]
    async fn pausing_the_room_that_leads_pauses_the_group() {
        let mut house = House::new();
        house.leaders.insert("RINCON_BACK", "RINCON_BAR");
        music(&house).await.pause(&room("living_room")).await;
        let pauses = house
            .asked("10.0.0.2")
            .into_iter()
            .filter(|(a, _)| a == "Pause")
            .count();
        assert_eq!(pauses, 1);
        assert!(!house.did("10.0.0.6", "BecomeCoordinatorOfStandaloneGroup", ""));
    }

    fn favorite(title: &str, station: bool) -> Favorite {
        Favorite {
            title: title.into(),
            uri: format!("x-sonosapi-stream:{title}"),
            metadata: String::new(),
            station,
        }
    }

    #[test]
    fn a_favorite_is_found_by_any_of_its_words() {
        let favorites = [
            favorite("DR P3", true),
            favorite("DR P4 Østjylland 95.9 (Local Music)", true),
            favorite("GO FM!", true),
        ];
        assert_eq!(
            best_favorite(&favorites, "P4").unwrap().title,
            "DR P4 Østjylland 95.9 (Local Music)"
        );
        assert_eq!(best_favorite(&favorites, "go fm").unwrap().title, "GO FM!");
        assert_eq!(best_favorite(&favorites, "the p3").unwrap().title, "DR P3");
        assert!(best_favorite(&favorites, "john mayer").is_none());
    }

    #[test]
    fn the_shortest_favorite_wins_when_several_fit() {
        let favorites = [favorite("DR P3 Classics", true), favorite("DR P3", true)];
        assert_eq!(best_favorite(&favorites, "dr p3").unwrap().title, "DR P3");
    }

    #[test]
    fn a_station_name_said_back_matches_exactly() {
        assert_eq!(words("DR P1"), words("dr p1"));
        assert_ne!(words("DR P1"), words("p1"));
    }

    #[test]
    fn reads_the_title_sonos_stored() {
        assert_eq!(
            title_in("<DIDL-Lite><item><dc:title>DR P3</dc:title></item></DIDL-Lite>").as_deref(),
            Some("DR P3")
        );
        assert_eq!(title_in(""), None);
    }
}
