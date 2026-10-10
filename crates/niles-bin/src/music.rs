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
//! After the stations comes Spotify, when an app's keys are set up: an
//! artist, song, album or playlist whose name is what was said, or
//! "Gravity by John Mayer". "From Spotify" skips the favorites and the
//! stations and takes Spotify's best match.
//!
//! A room plays as one: every Sonos placed in it is grouped behind one
//! of them before anything starts, the soundbar when there is one. That
//! is also what takes the TV off: music asked for in a room replaces
//! whatever its soundbar was playing.

mod media;

use crate::speakers::{Player, SpeakerRegistry};
use niles_config::ConfigStore;
use niles_core::RoomName;
use niles_speakers::spotify::{self, Credentials, Item, Kind as SpotifyKind, SpotifyClient};
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
    Resumed,
    Skipped,
    /// The room's volume now, as a percent.
    Volume(u8),
    /// Several things could be meant.
    Choose {
        stations: Vec<Station>,
        spotify: Vec<Item>,
    },
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

/// What to put on the speaker.
enum Content {
    /// A station, loaded as it is.
    Stream { uri: String, metadata: String },
    /// A playlist, an album, or a run of tracks, through the queue.
    Queue(Vec<(String, String)>),
}

pub struct Music {
    speakers: Arc<SpeakerRegistry>,
    config: Arc<ConfigStore>,
    spotify: SpotifyClient,
    /// Sonos's number for the household's Spotify, once read.
    spotify_service: Mutex<Option<u32>>,
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
        config: Arc<ConfigStore>,
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
            config,
            spotify: SpotifyClient::new(),
            spotify_service: Mutex::new(None),
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
            return self.find(room, &players, station, false).await;
        }
        if let Some(last) = self.remembered(room) {
            let content = Content::Stream {
                uri: last.uri,
                metadata: last.metadata,
            };
            return self.start(room, &players, content, &last.title).await;
        }
        // Started from the Sonos app, so Niles never saw it — but the
        // speaker still has it loaded.
        for player in &players {
            if let Ok(media) = player.client.media().await
                && media.is_radio()
            {
                let title = title_in(&media.metadata).unwrap_or_else(|| "the radio".into());
                let content = Content::Stream {
                    uri: media.uri,
                    metadata: media.metadata,
                };
                return self.start(room, &players, content, &title).await;
            }
        }
        Outcome::WhichStation
    }

    /// "Play X" — a favorite, a station, or Spotify. `from_spotify`
    /// when that was said: Spotify only, and its best match.
    pub async fn play(&self, room: &RoomName, query: &str, from_spotify: bool) -> Outcome {
        let players = self.speakers.in_room(room).await;
        if players.is_empty() {
            return Outcome::NoSpeaker;
        }
        self.find(room, &players, query, from_spotify).await
    }

    /// Something picked from a [`Outcome::Choose`] by its Spotify URI.
    /// `name` is needed for an artist, which plays as tracks found by it.
    pub async fn spotify_uri(&self, room: &RoomName, uri: &str, name: &str) -> Outcome {
        let players = self.speakers.in_room(room).await;
        if players.is_empty() {
            return Outcome::NoSpeaker;
        }
        let kind = match uri.split(':').nth(1) {
            Some("artist") => SpotifyKind::Artist,
            Some("track") => SpotifyKind::Track,
            Some("album") => SpotifyKind::Album,
            Some("playlist") => SpotifyKind::Playlist,
            _ => return Outcome::Failed(format!("{uri:?} is not something Spotify plays")),
        };
        let item = Item {
            kind,
            uri: uri.to_string(),
            name: name.to_string(),
            by: None,
        };
        let Some((credentials, market)) = self.spotify_keys() else {
            return Outcome::Failed("Spotify is not set up in Niles".into());
        };
        match self
            .spotify_run(&credentials, market.as_deref(), &item)
            .await
        {
            Ok(items) => self.play_spotify(&players, items, name).await,
            Err(e) => Outcome::Failed(e),
        }
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
    ///
    /// Done when anything paused. A room is often two groups at once —
    /// the soundbar gone to the TV when it came on, and the music carried
    /// on by the speaker left behind — and the TV's sound may refuse a
    /// pause that the music takes.
    pub async fn pause(&self, room: &RoomName) -> Outcome {
        let players = self.speakers.in_room(room).await;
        if players.is_empty() {
            return Outcome::NoSpeaker;
        }
        let here: Vec<&str> = players.iter().map(|p| p.sonos.id.as_str()).collect();
        let mut paused: Vec<&str> = Vec::new();
        let mut done = 0;
        let mut refused = None;
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
            match step {
                Ok(()) => done += 1,
                Err(e) => {
                    tracing::debug!("[music] {} would not pause: {e}", player.sonos.name);
                    refused = Some(e);
                }
            }
        }
        self.speakers.forget().await;
        match refused {
            Some(e) if done == 0 => Outcome::Failed(e.to_string()),
            _ => Outcome::Paused,
        }
    }

    /// What is playing, room by room, for the model's picture of the
    /// house: "living room: Chariot by Gavin DeGraw". Rooms playing
    /// nothing are left out. Asked of each group once, all at the same
    /// time, and given up on rather than let a slow speaker hold up an
    /// answer.
    pub async fn now_playing(&self) -> Vec<String> {
        let ask = async {
            let all = self.speakers.everywhere().await;
            let cfg = self.config.current();
            let room_of = |id: &str| cfg.speakers.sonos.get(id).map(|s| s.room.replace('_', " "));
            let mut groups: Vec<(&Player, Vec<String>)> = Vec::new();
            for player in &all {
                let Some(leader) = all.iter().find(|p| p.sonos.id == player.sonos.coordinator)
                else {
                    continue;
                };
                let room = room_of(&player.sonos.id).unwrap_or_else(|| player.sonos.name.clone());
                match groups
                    .iter_mut()
                    .find(|(l, _)| l.sonos.id == leader.sonos.id)
                {
                    Some((_, rooms)) if !rooms.contains(&room) => rooms.push(room),
                    Some(_) => {}
                    None => groups.push((leader, vec![room])),
                }
            }
            let lines = groups.into_iter().map(|(leader, rooms)| async move {
                let (kind, what) = playing(leader).await?;
                let what = match (kind, what) {
                    ("tv", _) => "the TV's sound".to_string(),
                    ("radio", Some(station)) => format!("the radio, {station}"),
                    ("radio", None) => "the radio".to_string(),
                    (_, Some(what)) => what,
                    (_, None) => "music".to_string(),
                };
                Some(format!("{}: playing {what}", rooms.join(" and ")))
            });
            futures_util::future::join_all(lines)
                .await
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
        };
        tokio::time::timeout(std::time::Duration::from_millis(600), ask)
            .await
            .unwrap_or_default()
    }

    /// Each room with a Sonos: whether it plays, what, and how loud —
    /// for the room cards. Asked of every room at once.
    pub async fn rooms(&self) -> Vec<niles_api::music::RoomMusic> {
        let cfg = self.config.current();
        let mut rooms: Vec<String> = cfg
            .speakers
            .sonos
            .values()
            .map(|s| s.room.clone())
            .collect();
        rooms.sort();
        rooms.dedup();
        let asks = rooms.into_iter().map(|room| async move {
            let name = RoomName::parse(&room).ok()?;
            let players = self.speakers.in_room(&name).await;
            let first = players.first()?;
            let all = self.speakers.everywhere().await;
            let leader = all
                .iter()
                .find(|p| p.sonos.id == first.sonos.coordinator)
                .unwrap_or(first);
            let now = playing(leader).await;
            Some(niles_api::music::RoomMusic {
                room,
                playing: now.is_some(),
                what: now.as_ref().and_then(|(_, what)| what.clone()),
                kind: now.map(|(kind, _)| kind),
                volume: first.client.get_volume().await.ok(),
            })
        });
        let ask = futures_util::future::join_all(asks);
        tokio::time::timeout(std::time::Duration::from_millis(1500), ask)
            .await
            .unwrap_or_default()
            .into_iter()
            .flatten()
            .collect()
    }

    /// The household's Sonos Favorites by title, for the words speech
    /// recognition should expect. Nothing when no Sonos answers.
    pub async fn favorite_titles(&self) -> Vec<String> {
        let all = self.speakers.everywhere().await;
        if all.is_empty() {
            return Vec::new();
        }
        self.favorites(&all)
            .await
            .into_iter()
            .map(|f| f.title)
            .collect()
    }

    /// "Stop the music", no room named. The music playing where it was
    /// said, and every room grouped with it — playing everywhere, it
    /// stops everywhere. Nothing playing there, the music playing
    /// anywhere else: Majse in the living room, the radio on in the
    /// kitchen. The TV's sound is not the music, and is left alone.
    ///
    /// The rooms it stopped in, or none when no music played.
    pub async fn stop_music(&self, here: Option<&RoomName>) -> Result<Vec<String>, String> {
        let all = self.speakers.everywhere().await;
        let cfg = self.config.current();
        let room_of = |id: &str| cfg.speakers.sonos.get(id).map(|s| s.room.clone());
        // Each group, by its leader, with the rooms in it.
        let mut groups: Vec<(&Player, Vec<String>)> = Vec::new();
        for player in &all {
            let Some(leader) = all.iter().find(|p| p.sonos.id == player.sonos.coordinator) else {
                continue;
            };
            let room = room_of(&player.sonos.id).unwrap_or_default();
            match groups
                .iter_mut()
                .find(|(l, _)| l.sonos.id == leader.sonos.id)
            {
                Some((_, rooms)) if !rooms.contains(&room) => rooms.push(room),
                Some(_) => {}
                None => groups.push((leader, vec![room])),
            }
        }
        let mut music = Vec::new();
        for (leader, rooms) in groups {
            if matches!(playing(leader).await, Some((kind, _)) if kind != "tv") {
                music.push((leader, rooms));
            }
        }
        let here = here.map(|r| r.as_str().to_string());
        let mine: Vec<_> = music
            .iter()
            .filter(|(_, rooms)| here.as_ref().is_some_and(|h| rooms.contains(h)))
            .collect();
        let targets = if mine.is_empty() {
            music.iter().collect::<Vec<_>>()
        } else {
            mine
        };
        let mut stopped = Vec::new();
        for (leader, rooms) in targets {
            leader.client.pause().await.map_err(|e| e.to_string())?;
            for room in rooms {
                if !stopped.contains(room) {
                    stopped.push(room.clone());
                }
            }
        }
        self.speakers.forget().await;
        Ok(stopped)
    }

    /// "Resume the music": play on, through each group's leader.
    pub async fn resume(&self, room: &RoomName) -> Outcome {
        self.each_leader(room, Outcome::Resumed, |c| async move { c.play().await })
            .await
    }

    /// "Next song" / "previous song".
    pub async fn skip(&self, room: &RoomName, back: bool) -> Outcome {
        self.each_leader(room, Outcome::Skipped, move |c| async move {
            if back {
                c.previous().await
            } else {
                c.next().await
            }
        })
        .await
    }

    /// Set every speaker in the room to `level`, or move each by `step`.
    pub async fn volume(&self, room: &RoomName, level: Option<u8>, step: Option<i16>) -> Outcome {
        let players = self.speakers.in_room(room).await;
        if players.is_empty() {
            return Outcome::NoSpeaker;
        }
        let mut now = None;
        for player in &players {
            let target = match (level, step) {
                (Some(level), _) => level.min(100),
                (None, Some(step)) => match player.client.get_volume().await {
                    Ok(current) => (i16::from(current) + step).clamp(0, 100) as u8,
                    Err(e) => return Outcome::Failed(e.to_string()),
                },
                (None, None) => return Outcome::Failed("no volume given".into()),
            };
            if let Err(e) = player.client.set_volume(target).await {
                return Outcome::Failed(e.to_string());
            }
            now.get_or_insert(target);
        }
        Outcome::Volume(now.unwrap_or_default())
    }

    async fn each_leader<F, Fut>(&self, room: &RoomName, done: Outcome, op: F) -> Outcome
    where
        F: Fn(Arc<niles_speakers::SonosClient>) -> Fut,
        Fut: std::future::Future<Output = niles_speakers::Result<()>>,
    {
        let leaders = self.speakers.leaders(room).await;
        if leaders.is_empty() {
            return Outcome::NoSpeaker;
        }
        for leader in leaders {
            if let Err(e) = op(leader).await {
                return Outcome::Failed(e.to_string());
            }
        }
        done
    }

    async fn find(
        &self,
        room: &RoomName,
        players: &[Player],
        query: &str,
        from_spotify: bool,
    ) -> Outcome {
        let mut stations = Vec::new();
        if !from_spotify {
            let favorites = self.favorites(players).await;
            if let Some(favorite) = best_favorite(&favorites, query) {
                let content = if favorite.station {
                    Content::Stream {
                        uri: favorite.uri.clone(),
                        metadata: favorite.metadata.clone(),
                    }
                } else {
                    Content::Queue(vec![(favorite.uri.clone(), favorite.metadata.clone())])
                };
                return self.start(room, players, content, &favorite.title).await;
            }
            // Not the end of the search when TuneIn is down: Spotify may
            // still have it.
            stations = tunein::search(&self.http, query).await.unwrap_or_else(|e| {
                tracing::warn!("[music] TuneIn did not answer: {e}");
                Vec::new()
            });
            if let Some(station) = stations.iter().find(|s| words(&s.name) == words(query)) {
                return self.play_station(room, players, &favorites, station).await;
            }
        }
        let mut spotify = Vec::new();
        if let Some((credentials, market)) = self.spotify_keys() {
            match self
                .spotify_pick(&credentials, market.as_deref(), query, from_spotify)
                .await
            {
                Ok(Pick::Play { title, items }) => {
                    return self.play_spotify(players, items, &title).await;
                }
                Ok(Pick::Candidates(found)) => spotify = found,
                Err(e) => tracing::warn!("[music] Spotify search for {query:?} failed: {e}"),
            }
        }
        if stations.is_empty() && spotify.is_empty() {
            return Outcome::NotFound(query.to_string());
        }
        stations.truncate(6);
        Outcome::Choose { stations, spotify }
    }

    /// The Spotify app's keys and the market to search, when Spotify is
    /// set up and switched on.
    fn spotify_keys(&self) -> Option<(Credentials, Option<String>)> {
        let cfg = self.config.current();
        let spotify = cfg.integrations.spotify.as_ref().filter(|s| s.enabled)?;
        let (client_id, client_secret) = spotify
            .resolve_credentials()
            .map_err(|e| tracing::warn!("[music] Spotify keys: {e}"))
            .ok()?;
        Some((
            Credentials {
                client_id,
                client_secret,
            },
            cfg.home.resolved_country(),
        ))
    }

    /// What Spotify has for `query`: one thing to play when it is clear,
    /// or the candidates for the model to choose among.
    async fn spotify_pick(
        &self,
        credentials: &Credentials,
        market: Option<&str>,
        query: &str,
        sure: bool,
    ) -> Result<Pick, String> {
        let query = query.trim();
        let query = query.strip_prefix("some ").unwrap_or(query);
        if let Some((title, artist)) = query.rsplit_once(" by ")
            && let Some(track) = self
                .spotify
                .track(credentials, title, artist, market)
                .await
                .map_err(|e| e.to_string())?
        {
            let title = format!(
                "{} by {}",
                track.name,
                track.by.as_deref().unwrap_or(artist)
            );
            let items = self.spotify_run(credentials, market, &track).await?;
            return Ok(Pick::Play { title, items });
        }
        let (kind, query) = kind_named(query);
        let found = self
            .spotify
            .search(credentials, query, market)
            .await
            .map_err(|e| e.to_string())?;
        let exact = exact_matches(&found, query, kind);
        // One thing called that — or one artist, who comes before a song
        // named after them — a kind named ("the album Continuum"), or
        // "from Spotify": play it. Several called that and nothing to tell
        // them apart (four bands are called Continuum): the model chooses,
        // the exact ones first.
        let artists = exact
            .iter()
            .filter(|i| i.kind == SpotifyKind::Artist)
            .count();
        let clear = exact.len() == 1 || artists == 1 || kind.is_some() || sure;
        let mut picks: Vec<&Item> = if clear { exact.clone() } else { Vec::new() };
        if sure && picks.is_empty() {
            picks.extend(best_pick(&found, query));
        }
        for item in picks {
            let title = match (&item.kind, &item.by) {
                (SpotifyKind::Artist | SpotifyKind::Playlist, _) | (_, None) => item.name.clone(),
                (_, Some(by)) => format!("{} by {by}", item.name),
            };
            match self.spotify_run(credentials, market, item).await {
                Ok(items) => return Ok(Pick::Play { title, items }),
                // An artist called that with no songs of their own: try
                // the next thing called that.
                Err(e) => tracing::debug!("[music] {e}"),
            }
        }
        let mut offered: Vec<Item> = exact.into_iter().take(6).cloned().collect();
        for item in candidates(found) {
            if !offered.iter().any(|o| o.uri == item.uri) {
                offered.push(item);
            }
        }
        Ok(Pick::Candidates(offered))
    }

    /// What plays for one Spotify item: an artist as tracks found by
    /// them, a song followed by more of its artist, an album or a
    /// playlist as itself. An artist's songs come shuffled, so "play
    /// John Mayer" does not start with the same song every time.
    async fn spotify_run(
        &self,
        credentials: &Credentials,
        market: Option<&str>,
        item: &Item,
    ) -> Result<Vec<Item>, String> {
        let artist = match item.kind {
            SpotifyKind::Artist => Some(item.name.as_str()),
            SpotifyKind::Track => item.by.as_deref(),
            SpotifyKind::Album | SpotifyKind::Playlist => return Ok(vec![item.clone()]),
        };
        let mut run = Vec::new();
        if item.kind == SpotifyKind::Track {
            run.push(item.clone());
        }
        if let Some(artist) = artist {
            let more = self
                .spotify
                .tracks_by(credentials, artist, market)
                .await
                .map_err(|e| e.to_string())?;
            // Not the song just asked for again, in another version.
            run.extend(
                more.into_iter()
                    .filter(|t| t.uri != item.uri && !t.name.eq_ignore_ascii_case(&item.name)),
            );
        }
        run.truncate(10);
        shuffle_after_the_first(&mut run, item.kind == SpotifyKind::Track);
        if run.is_empty() {
            return Err(format!("Spotify has no tracks for {:?}", item.name));
        }
        Ok(run)
    }

    async fn play_spotify(&self, players: &[Player], items: Vec<Item>, title: &str) -> Outcome {
        // Which Spotify the household's account is on cannot be asked
        // beforehand, so each is tried until Sonos takes the queue — the
        // one that worked last time first.
        let known = *self.service_lock();
        let services = known.into_iter().chain(
            spotify::SPOTIFY_SERVICES
                .into_iter()
                .filter(|s| Some(*s) != known),
        );
        let mut refused = None;
        for service in services {
            let queue = items
                .iter()
                .filter_map(|item| spotify::enqueue(item, service))
                .collect();
            match self.load_and_play(players, &Content::Queue(queue)).await {
                Ok(()) => {
                    *self.service_lock() = Some(service);
                    return Outcome::Playing(title.to_string());
                }
                Err(e) if is_refused(&e) => {
                    refused = Some(e);
                }
                Err(e) => return Outcome::Failed(e.to_string()),
            }
        }
        Outcome::Failed(format!(
            "Sonos refused Spotify ({}): is Spotify linked in the Sonos app?",
            refused.map(|e| e.to_string()).unwrap_or_default()
        ))
    }

    fn service_lock(&self) -> std::sync::MutexGuard<'_, Option<u32>> {
        self.spotify_service
            .lock()
            .unwrap_or_else(|e| e.into_inner())
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
        self.start(
            room,
            players,
            Content::Stream { uri, metadata },
            &station.name,
        )
        .await
    }

    /// Group the room behind its leader, load, play.
    async fn start(
        &self,
        room: &RoomName,
        players: &[Player],
        content: Content,
        title: &str,
    ) -> Outcome {
        if let Err(e) = self.load_and_play(players, &content).await {
            return Outcome::Failed(e.to_string());
        }
        if let Content::Stream { uri, metadata } = content
            && is_radio(&uri)
        {
            self.remember(
                room,
                Remembered {
                    title: title.to_string(),
                    uri,
                    metadata,
                },
            );
        }
        Outcome::Playing(title.to_string())
    }

    async fn load_and_play(
        &self,
        players: &[Player],
        content: &Content,
    ) -> niles_speakers::Result<()> {
        let leader = leader_of(players);
        let result = async {
            if leader.sonos.coordinator != leader.sonos.id {
                leader.client.go_solo().await?;
            }
            for player in players {
                if player.sonos.id != leader.sonos.id && player.sonos.coordinator != leader.sonos.id
                {
                    player.client.join(&leader.sonos.id).await?;
                }
            }
            match content {
                Content::Queue(items) => leader.client.load_queue(&leader.sonos.id, items).await?,
                Content::Stream { uri, metadata } => leader.client.load(uri, metadata).await?,
            }
            leader.client.play().await
        }
        .await;
        self.speakers.forget().await;
        result
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

/// Sonos's answer when a queue names a Spotify the household has no
/// account on.
fn is_refused(e: &niles_speakers::Error) -> bool {
    matches!(e, niles_speakers::Error::SoapFault { code, .. } if code == "800")
}

/// Shuffle a run of an artist's songs; `keep_first` when the first is
/// the song that was asked for, which plays first all the same.
fn shuffle_after_the_first(run: &mut [Item], keep_first: bool) {
    use rand::seq::SliceRandom;
    let start = usize::from(keep_first).min(run.len());
    run[start..].shuffle(&mut rand::rng());
}

/// What a group's leader is playing, if anything: its kind — `music`,
/// `radio` or `tv` — and what it is, when the speaker can say.
async fn playing(leader: &Player) -> Option<(&'static str, Option<String>)> {
    if !matches!(
        leader.client.get_transport_state().await,
        Ok(TransportState::Playing)
    ) {
        return None;
    }
    let media = leader.client.media().await.ok()?;
    if media.is_tv() {
        return Some(("tv", None));
    }
    if media.is_radio() {
        return Some(("radio", title_in(&media.metadata)));
    }
    let track = leader.client.track().await.ok().unwrap_or_default();
    Some(("music", track_line(&track)))
}

/// "Chariot by Gavin DeGraw", or the title alone.
fn track_line(track: &niles_speakers::Track) -> Option<String> {
    let title = track.title.clone()?;
    Some(match &track.artist {
        Some(artist) => format!("{title} by {artist}"),
        None => title,
    })
}

/// What Spotify had for a request.
enum Pick {
    Play { title: String, items: Vec<Item> },
    Candidates(Vec<Item>),
}

/// Everything whose name is exactly what was said, artists first: "John
/// Mayer" is the man before it is a song somebody named after him.
fn exact_matches<'a>(
    found: &'a spotify::Found,
    query: &str,
    kind: Option<SpotifyKind>,
) -> Vec<&'a Item> {
    let wanted = words(query);
    [
        &found.artists,
        &found.albums,
        &found.tracks,
        &found.playlists,
    ]
    .into_iter()
    .flat_map(|items| items.iter())
    .filter(|item| kind.is_none_or(|k| item.kind == k))
    .filter(|item| words(&item.name) == wanted)
    .collect()
}

/// "the album Continuum" is an album called Continuum.
fn kind_named(query: &str) -> (Option<SpotifyKind>, &str) {
    let bare = query.strip_prefix("the ").unwrap_or(query);
    for (prefix, kind) in [
        ("album ", SpotifyKind::Album),
        ("record ", SpotifyKind::Album),
        ("song ", SpotifyKind::Track),
        ("track ", SpotifyKind::Track),
        ("playlist ", SpotifyKind::Playlist),
        ("artist ", SpotifyKind::Artist),
        ("band ", SpotifyKind::Artist),
    ] {
        if let Some(rest) = bare.strip_prefix(prefix) {
            return (Some(kind), rest.trim());
        }
    }
    (None, query)
}

/// Spotify's best guess, for "from Spotify": an artist whose name holds
/// every word said, else its most relevant song.
fn best_pick<'a>(found: &'a spotify::Found, query: &str) -> Option<&'a Item> {
    let wanted = words(query);
    found
        .artists
        .iter()
        .find(|a| {
            let name = words(&a.name);
            wanted.iter().all(|w| name.contains(w))
        })
        .or_else(|| found.tracks.first())
}

/// A few of each kind, for the model to choose among.
fn candidates(found: spotify::Found) -> Vec<Item> {
    found
        .artists
        .into_iter()
        .take(2)
        .chain(found.tracks.into_iter().take(3))
        .chain(found.albums.into_iter().take(2))
        .chain(found.playlists.into_iter().take(2))
        .collect()
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
        Outcome::Resumed => json!({ "resumed": room }),
        Outcome::Skipped => json!({ "skipped": room }),
        Outcome::Volume(percent) => json!({ "room": room, "volume_percent": percent }),
        Outcome::Choose { stations, spotify } => json!({
            "stations": stations.iter().map(|s| json!({
                "station_id": s.id, "name": s.name, "about": s.about,
            })).collect::<Vec<_>>(),
            "spotify": spotify.iter().map(|i| json!({
                "spotify_uri": i.uri, "kind": i.kind, "name": i.name, "by": i.by,
            })).collect::<Vec<_>>(),
            "next": "Pick the one meant and play it: a station with play_radio and its station_id \
                (prefer the household's own country), Spotify with play_music and its spotify_uri \
                and name. Ask which when it is not clear.",
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
            description:
                "Play a radio station on the Sonos in a room, grouping every speaker in it. \
                No station: the one that room played last. A station by name is looked for in \
                the household's Sonos favorites, then on TuneIn; when several could be meant, \
                the result lists them, and you call again with the station_id."
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
            description: "Play something by name on the Sonos in a room: a Sonos favorite, a \
                radio station, or an artist, song (\"Gravity by John Mayer\"), album or playlist \
                on Spotify. Every speaker in the room plays it together, and it replaces what \
                was playing, the TV included. When several things could be meant, the result \
                lists them; call again with the spotify_uri and name of the one meant."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "room": { "type": "string" },
                    "query": { "type": "string", "description": "What was asked for, as said." },
                    "from_spotify": { "type": "boolean", "description": "True when the person said Spotify: skip favorites and radio." },
                    "spotify_uri": { "type": "string", "description": "A spotify_uri from an earlier result." },
                    "name": { "type": "string", "description": "The name that went with that spotify_uri." }
                },
                "required": ["room"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> ToolResult<Value> {
        let room = required_room("play_music", &args)?;
        let name = args.get("name").and_then(Value::as_str).unwrap_or("");
        if let Some(uri) = args.get("spotify_uri").and_then(Value::as_str) {
            return Ok(reported(
                self.0.spotify_uri(&room, uri, name).await,
                Some(&room),
            ));
        }
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if query.is_empty() {
            return Ok(json!({ "error": "what should I play?" }));
        }
        let from_spotify = args
            .get("from_spotify")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        Ok(reported(
            self.0.play(&room, query, from_spotify).await,
            Some(&room),
        ))
    }
}

struct PlayElsewhere(Arc<Music>);

#[async_trait::async_trait]
impl Tool for PlayElsewhere {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "play_elsewhere".into(),
            description: "Spread what is playing to another room, or to every room: \
                \"play it in the kitchen too\", \"play it everywhere\". The music comes from \
                from_room, or from whatever is playing when that room plays nothing. \
                TV sound is never spread."
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

/// One of the plain controls: pause, resume, skip, volume.
struct Control {
    music: Arc<Music>,
    name: &'static str,
    description: &'static str,
    extra: Value,
}

#[async_trait::async_trait]
impl Tool for Control {
    fn descriptor(&self) -> ToolDescriptor {
        let mut properties = json!({ "room": { "type": "string" } });
        if let (Some(all), Some(extra)) = (properties.as_object_mut(), self.extra.as_object()) {
            all.extend(extra.clone());
        }
        // Pausing may leave the room out: "stop the music" is about the
        // music, wherever it plays.
        let required: &[&str] = if self.name == "pause_music" {
            &[]
        } else {
            &["room"]
        };
        ToolDescriptor {
            name: self.name.into(),
            description: self.description.into(),
            parameters: json!({
                "type": "object",
                "properties": properties,
                "required": required,
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> ToolResult<Value> {
        // Pausing with no room named is "stop the music": whatever
        // music plays, wherever it is.
        if self.name == "pause_music" && room_arg(self.name, &args, "room")?.is_none() {
            return Ok(match self.music.stop_music(None).await {
                Ok(rooms) if rooms.is_empty() => {
                    json!({ "stopped": [], "note": "no music was playing" })
                }
                Ok(rooms) => json!({ "stopped": rooms }),
                Err(e) => json!({ "error": e }),
            });
        }
        let room = required_room(self.name, &args)?;
        let outcome = match self.name {
            "pause_music" => self.music.pause(&room).await,
            "resume_music" => self.music.resume(&room).await,
            "skip_track" => {
                let back = args.get("back").and_then(Value::as_bool).unwrap_or(false);
                self.music.skip(&room, back).await
            }
            _ => {
                let level = args
                    .get("level")
                    .and_then(Value::as_u64)
                    .map(|l| l.min(100) as u8);
                let step = args
                    .get("change")
                    .and_then(Value::as_i64)
                    .map(|c| c.clamp(-100, 100) as i16);
                self.music.volume(&room, level, step).await
            }
        };
        Ok(reported(outcome, Some(&room)))
    }
}

pub fn register(reg: &mut ToolRegistry, music: Arc<Music>) {
    reg.register(Box::new(PlayRadio(music.clone())));
    reg.register(Box::new(PlayMusic(music.clone())));
    reg.register(Box::new(PlayElsewhere(music.clone())));
    let controls = [
        (
            "pause_music",
            "Stop or pause music on the Sonos. Name a room only when the person named one; \
             left out, the music playing anywhere is stopped — \"stop the music\" in a quiet \
             room means the music somewhere else. The TV's sound is not the music. A room \
             following another room's music leaves the group and the rest play on.",
            json!({}),
        ),
        (
            "resume_music",
            "Carry on playing what the Sonos in a room was playing before it was paused.",
            json!({}),
        ),
        (
            "skip_track",
            "Next song on the Sonos in a room, or the previous one with back.",
            json!({ "back": { "type": "boolean" } }),
        ),
        (
            "music_volume",
            "The Sonos volume in a room: set a level (0-100), or change it by a step \
             (+10 louder, -10 quieter). Every speaker in the room moves together.",
            json!({
                "level": { "type": "integer", "minimum": 0, "maximum": 100 },
                "change": { "type": "integer", "minimum": -100, "maximum": 100 }
            }),
        ),
    ];
    for (name, description, extra) in controls {
        reg.register(Box::new(Control {
            music: music.clone(),
            name,
            description,
            extra,
        }));
    }
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
        /// (address, action) pairs answered with a UPnP fault.
        refuse: Vec<(&'static str, &'static str)>,
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
            let name = action.rsplit('#').next().unwrap();
            if self.refuse.contains(&(ip, name)) {
                return Err(niles_speakers::Error::SoapFault {
                    code: "701".into(),
                    reason: "Transition not available".into(),
                });
            }
            Ok(match name {
                "GetZoneGroupState" => {
                    format!("<ZoneGroupState>{}</ZoneGroupState>", self.state())
                }
                "GetMediaInfo" => format!(
                    "<CurrentURI>{}</CurrentURI>",
                    escaped(self.loaded.get(ip).copied().unwrap_or(""))
                ),
                "GetVolume" => "<CurrentVolume>30</CurrentVolume>".to_string(),
                "GetPositionInfo" => format!(
                    "<TrackURI>{}</TrackURI><TrackMetaData>{}</TrackMetaData>",
                    escaped("x-sonos-spotify:spotify%3atrack%3achariot?sid=9&flags=8224&sn=3"),
                    escaped(
                        "<DIDL-Lite><item><dc:title>Chariot</dc:title><dc:creator>Gavin DeGraw</dc:creator></item></DIDL-Lite>"
                    )
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
            config.clone(),
            Arc::new(house.clone()),
        ));
        Music::new(speakers, config, None).await
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
    async fn the_music_left_behind_by_the_tv_is_paused() {
        // The soundbar went to the TV and refuses a pause; the speaker at
        // the back carries the music on its own, and takes it.
        let mut house = House::new();
        house.refuse.push(("10.0.0.2", "Pause"));
        let outcome = music(&house).await.pause(&room("living_room")).await;
        assert_eq!(outcome, Outcome::Paused);
        assert!(house.did("10.0.0.6", "Pause", ""));
    }

    #[tokio::test]
    async fn nothing_pausing_is_said() {
        let mut house = House::new();
        house.refuse.push(("10.0.0.4", "Pause"));
        let outcome = music(&house).await.pause(&room("kitchen")).await;
        assert!(matches!(outcome, Outcome::Failed(_)), "{outcome:?}");
    }

    #[tokio::test]
    async fn says_what_plays_room_by_room() {
        let mut house = House::new();
        house.leaders.insert("RINCON_BACK", "RINCON_BAR");
        house.playing.extend(["10.0.0.2", "10.0.0.4"]);
        house
            .loaded
            .insert("10.0.0.2", "x-rincon-queue:RINCON_BAR#0");
        house.loaded.insert(
            "10.0.0.4",
            "x-sonosapi-stream:s24861?sid=333&flags=8224&sn=14",
        );
        let mut lines = music(&house).await.now_playing().await;
        lines.sort();
        assert_eq!(
            lines,
            [
                "kitchen: playing the radio",
                "living room: playing Chariot by Gavin DeGraw"
            ]
        );
    }

    #[tokio::test]
    async fn each_room_says_what_plays_and_how_loud() {
        let mut house = House::new();
        house.leaders.insert("RINCON_BACK", "RINCON_BAR");
        house.playing.push("10.0.0.2");
        house
            .loaded
            .insert("10.0.0.2", "x-rincon-queue:RINCON_BAR#0");
        let rooms = music(&house).await.rooms().await;
        let living = rooms.iter().find(|r| r.room == "living_room").unwrap();
        assert!(living.playing);
        assert_eq!(living.kind, Some("music"));
        assert_eq!(living.what.as_deref(), Some("Chariot by Gavin DeGraw"));
        assert_eq!(living.volume, Some(30));
        let kitchen = rooms.iter().find(|r| r.room == "kitchen").unwrap();
        assert!(!kitchen.playing);
        assert_eq!(kitchen.what, None);
    }

    #[tokio::test]
    async fn the_tv_is_said_as_the_tv() {
        let mut house = House::new();
        house.playing.push("10.0.0.2");
        house
            .loaded
            .insert("10.0.0.2", "x-sonos-htastream:RINCON_BAR:spdif");
        assert_eq!(
            music(&house).await.now_playing().await,
            ["living room: playing the TV's sound"]
        );
    }

    #[tokio::test]
    async fn nothing_playing_says_nothing() {
        let house = House::new();
        assert!(music(&house).await.now_playing().await.is_empty());
    }

    #[tokio::test]
    async fn stop_in_a_quiet_room_stops_the_music_elsewhere() {
        // Majse in the living room, the radio on in the kitchen.
        let mut house = House::new();
        house.playing.push("10.0.0.4");
        house.loaded.insert(
            "10.0.0.4",
            "x-sonosapi-stream:s24861?sid=333&flags=8224&sn=14",
        );
        let stopped = music(&house)
            .await
            .stop_music(Some(&room("living_room")))
            .await
            .unwrap();
        assert_eq!(stopped, ["kitchen"]);
        assert!(house.did("10.0.0.4", "Pause", ""));
        assert!(!house.did("10.0.0.2", "Pause", ""));
    }

    #[tokio::test]
    async fn music_everywhere_stops_everywhere() {
        let mut house = House::new();
        house.leaders.insert("RINCON_BACK", "RINCON_BAR");
        house.leaders.insert("RINCON_MOVE", "RINCON_BAR");
        house.playing.push("10.0.0.2");
        house
            .loaded
            .insert("10.0.0.2", "x-rincon-queue:RINCON_BAR#0");
        let mut stopped = music(&house)
            .await
            .stop_music(Some(&room("kitchen")))
            .await
            .unwrap();
        stopped.sort();
        assert_eq!(stopped, ["kitchen", "living_room"]);
        // One pause, to the group's leader.
        let pauses = house
            .asked("10.0.0.2")
            .into_iter()
            .filter(|(a, _)| a == "Pause")
            .count();
        assert_eq!(pauses, 1);
        assert!(!house.did("10.0.0.4", "BecomeCoordinatorOfStandaloneGroup", ""));
    }

    #[tokio::test]
    async fn the_tv_is_not_the_music() {
        let mut house = House::new();
        house.playing.extend(["10.0.0.2", "10.0.0.4"]);
        house
            .loaded
            .insert("10.0.0.2", "x-sonos-htastream:RINCON_BAR:spdif");
        house
            .loaded
            .insert("10.0.0.4", "x-rincon-queue:RINCON_MOVE#0");
        let stopped = music(&house)
            .await
            .stop_music(Some(&room("living_room")))
            .await
            .unwrap();
        assert_eq!(stopped, ["kitchen"]);
        assert!(!house.did("10.0.0.2", "Pause", ""));
    }

    #[tokio::test]
    async fn nothing_playing_stops_nothing() {
        let house = House::new();
        assert!(
            music(&house)
                .await
                .stop_music(None)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn the_page_groups_by_what_is_loaded() {
        // This morning's house: the living room's two paused together
        // with Spotify queued, the radio on in the kitchen.
        let mut house = House::new();
        house.leaders.insert("RINCON_BACK", "RINCON_BAR");
        house
            .loaded
            .insert("10.0.0.2", "x-rincon-queue:RINCON_BAR#0");
        house.playing.push("10.0.0.4");
        house.loaded.insert(
            "10.0.0.4",
            "x-sonosapi-stream:s24861?sid=333&flags=8224&sn=14",
        );
        let view = music(&house).await.view().await;
        let spotify = view.groups.iter().find(|g| g.kind == "spotify").unwrap();
        assert!(!spotify.playing);
        assert_eq!(spotify.leader, "RINCON_BAR");
        assert_eq!(
            spotify
                .speakers
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>(),
            ["Living Room", "Living Room Back"]
        );
        assert!(spotify.speakers[0].soundbar);
        let radio = view.groups.iter().find(|g| g.kind == "radio").unwrap();
        assert!(radio.playing);
        assert!(view.idle.is_empty());
    }

    #[tokio::test]
    async fn a_speaker_with_nothing_loaded_is_not_playing() {
        let house = House::new();
        let view = music(&house).await.view().await;
        assert!(view.groups.is_empty());
        assert_eq!(view.idle.len(), 3);
    }

    #[tokio::test]
    async fn nothing_joins_the_tv() {
        let mut house = House::new();
        house
            .loaded
            .insert("10.0.0.2", "x-sonos-htastream:RINCON_BAR:spdif");
        let music = music(&house).await;
        assert!(music.join_group("RINCON_MOVE", "RINCON_BAR").await.is_err());
        assert!(!house.did("10.0.0.4", "SetAVTransportURI", ""));
    }

    #[tokio::test]
    async fn a_speaker_joins_a_group_and_leaves_it() {
        let house = House::new();
        let music = music(&house).await;
        music.join_group("RINCON_MOVE", "RINCON_BAR").await.unwrap();
        assert!(house.did("10.0.0.4", "SetAVTransportURI", "x-rincon:RINCON_BAR"));
        music.leave_group("RINCON_MOVE").await.unwrap();
        assert!(house.did("10.0.0.4", "BecomeCoordinatorOfStandaloneGroup", ""));
        assert!(house.did("10.0.0.4", "Pause", ""));
    }

    #[tokio::test]
    async fn the_tv_takes_its_soundbar_back() {
        let mut house = House::new();
        house.leaders.insert("RINCON_BAR", "RINCON_MOVE");
        let music = music(&house).await;
        music.tv_sound().await.unwrap();
        assert!(house.did("10.0.0.2", "BecomeCoordinatorOfStandaloneGroup", ""));
        assert!(house.did(
            "10.0.0.2",
            "SetAVTransportURI",
            "x-sonos-htastream:RINCON_BAR:spdif"
        ));
        assert!(house.did("10.0.0.2", "Play", ""));
    }

    #[tokio::test]
    async fn carrying_on_plays_from_the_speaker_with_the_queue() {
        let house = House::new();
        let music = music(&house).await;
        let choice = niles_api::music::Choice {
            kind: "queue".into(),
            id: "RINCON_BAR".into(),
            label: "Carry on".into(),
        };
        music
            .start_on(&["RINCON_BAR".into(), "RINCON_MOVE".into()], &choice)
            .await
            .unwrap();
        assert!(house.did("10.0.0.4", "SetAVTransportURI", "x-rincon:RINCON_BAR"));
        assert!(house.did("10.0.0.2", "Play", ""));
    }

    #[tokio::test]
    async fn resuming_goes_to_the_leader_once() {
        let mut house = House::new();
        house.leaders.insert("RINCON_BACK", "RINCON_BAR");
        assert_eq!(
            music(&house).await.resume(&room("living_room")).await,
            Outcome::Resumed
        );
        assert_eq!(
            house
                .asked("10.0.0.2")
                .iter()
                .filter(|(a, _)| a == "Play")
                .count(),
            1
        );
        assert!(!house.did("10.0.0.6", "Play", ""));
    }

    #[tokio::test]
    async fn a_volume_step_moves_every_speaker_in_the_room() {
        let house = House::new();
        let outcome = music(&house)
            .await
            .volume(&room("living_room"), None, Some(-10))
            .await;
        assert!(matches!(outcome, Outcome::Volume(_)), "{outcome:?}");
        for ip in ["10.0.0.2", "10.0.0.6"] {
            assert!(house.did(ip, "SetVolume", "DesiredVolume"), "{ip}");
        }
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

    fn spotify_item(kind: SpotifyKind, name: &str) -> Item {
        Item {
            kind,
            uri: format!("spotify:x:{name}"),
            name: name.into(),
            by: None,
        }
    }

    fn found() -> spotify::Found {
        spotify::Found {
            artists: vec![
                spotify_item(SpotifyKind::Artist, "John Mayer Trio"),
                spotify_item(SpotifyKind::Artist, "John Mayer"),
            ],
            tracks: vec![
                spotify_item(SpotifyKind::Track, "Gravity"),
                spotify_item(SpotifyKind::Track, "John Mayer"),
            ],
            albums: vec![spotify_item(SpotifyKind::Album, "Continuum")],
            playlists: vec![],
        }
    }

    #[test]
    fn an_exact_name_is_found_and_the_artist_comes_first() {
        let found = found();
        let exact = exact_matches(&found, "john mayer", None);
        assert_eq!(
            exact.iter().map(|i| i.kind).collect::<Vec<_>>(),
            [SpotifyKind::Artist, SpotifyKind::Track]
        );
        assert_eq!(
            exact_matches(&found, "continuum", None)[0].kind,
            SpotifyKind::Album
        );
        assert!(exact_matches(&found, "mayer", None).is_empty());
    }

    #[test]
    fn the_song_asked_for_still_plays_first() {
        let mut run: Vec<Item> = (0..10)
            .map(|n| spotify_item(SpotifyKind::Track, &format!("song {n}")))
            .collect();
        for _ in 0..20 {
            shuffle_after_the_first(&mut run, true);
            assert_eq!(run[0].name, "song 0");
        }
    }

    #[test]
    fn an_artist_does_not_start_with_the_same_song_every_time() {
        let original: Vec<Item> = (0..10)
            .map(|n| spotify_item(SpotifyKind::Track, &format!("song {n}")))
            .collect();
        let firsts: std::collections::HashSet<String> = (0..50)
            .map(|_| {
                let mut run = original.clone();
                shuffle_after_the_first(&mut run, false);
                run[0].name.clone()
            })
            .collect();
        // Fifty shuffles of ten songs starting the same way would be a
        // one-in-10^49 coincidence.
        assert!(firsts.len() > 1);
    }

    #[test]
    fn a_kind_named_narrows_the_search() {
        assert_eq!(
            kind_named("the album continuum"),
            (Some(SpotifyKind::Album), "continuum")
        );
        assert_eq!(
            kind_named("song gravity"),
            (Some(SpotifyKind::Track), "gravity")
        );
        assert_eq!(kind_named("john mayer"), (None, "john mayer"));
        let found = found();
        let exact = exact_matches(&found, "john mayer", Some(SpotifyKind::Track));
        assert_eq!(exact.len(), 1);
        assert_eq!(exact[0].kind, SpotifyKind::Track);
    }

    #[test]
    fn from_spotify_takes_the_best_guess() {
        let found = found();
        // Every word said is in the trio's name.
        assert_eq!(best_pick(&found, "mayer").unwrap().name, "John Mayer Trio");
        // No artist fits: the most relevant song.
        assert_eq!(best_pick(&found, "something else").unwrap().name, "Gravity");
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
