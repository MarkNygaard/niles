//! The Media page: every group by what it has loaded, and the moves the
//! page makes — a speaker into a group or out of it, a group played or
//! paused, something new started on chosen speakers, the soundbar back
//! to the TV.
//!
//! By what is *loaded*, not what plays: two speakers paused together
//! with Spotify in their queue are a Spotify group, because pressing
//! play on either starts both. Only a speaker with nothing loaded at all
//! is "not playing".

use super::{Content, Music, Player, title_in, track_line};
use niles_api::music::{Choice, Group, MediaView, Speaker};
use niles_speakers::TransportState;
use niles_speakers::spotify::{Item, Kind as SpotifyKind};
use niles_speakers::tunein::{self, Account, Station};

impl Music {
    /// Every placed Sonos, by the group it is in and what that group
    /// has loaded.
    pub async fn view(&self) -> MediaView {
        let all = self.speakers.everywhere().await;
        let cfg = self.config.current();
        let cfg = &cfg;
        let mut groups: Vec<(&Player, Vec<&Player>)> = Vec::new();
        for player in &all {
            let leader = all
                .iter()
                .find(|p| p.sonos.id == player.sonos.coordinator)
                .unwrap_or(player);
            match groups
                .iter_mut()
                .find(|(l, _)| l.sonos.id == leader.sonos.id)
            {
                Some((_, members)) => members.push(player),
                None => groups.push((leader, vec![player])),
            }
        }
        let read = groups.into_iter().map(|(leader, members)| async move {
            let speakers = futures_util::future::join_all(
                members.into_iter().map(|player| speaker(player, cfg)),
            )
            .await;
            let (kind, what) = loaded(leader).await;
            let playing = matches!(
                leader.client.get_transport_state().await,
                Ok(TransportState::Playing)
            );
            (leader.sonos.id.clone(), kind, what, playing, speakers)
        });
        let read = futures_util::future::join_all(read);
        let read = tokio::time::timeout(std::time::Duration::from_secs(3), read)
            .await
            .unwrap_or_default();
        let mut view = MediaView {
            groups: Vec::new(),
            idle: Vec::new(),
        };
        for (leader, kind, what, playing, mut speakers) in read {
            speakers.sort_by(|a, b| a.name.cmp(&b.name));
            match kind {
                "idle" => view.idle.extend(speakers),
                kind => view.groups.push(Group {
                    leader,
                    kind,
                    what,
                    playing,
                    speakers,
                }),
            }
        }
        view.idle.sort_by(|a, b| a.name.cmp(&b.name));
        view
    }

    async fn player(&self, id: &str) -> Result<Player, String> {
        self.speakers
            .everywhere()
            .await
            .into_iter()
            .find(|p| p.sonos.id == id)
            .ok_or_else(|| format!("no speaker {id} is placed in a room"))
    }

    /// `speaker` plays along with `leader`'s group. Not with the TV: its
    /// sound stays in its own room.
    pub async fn join_group(&self, speaker: &str, leader: &str) -> Result<(), String> {
        let joining = self.player(speaker).await?;
        let leading = self.player(leader).await?;
        if leading.client.media().await.is_ok_and(|m| m.is_tv()) {
            return Err("That's the TV, and the TV only plays in its own room.".into());
        }
        joining
            .client
            .join(&leading.sonos.id)
            .await
            .map_err(|e| e.to_string())?;
        self.speakers.forget().await;
        Ok(())
    }

    /// `speaker` leaves its group and stops; the rest play on.
    pub async fn leave_group(&self, speaker: &str) -> Result<(), String> {
        let player = self.player(speaker).await?;
        // Alone already is fine: there is nothing to leave.
        let _ = player.client.go_solo().await;
        let _ = player.client.pause().await;
        self.speakers.forget().await;
        Ok(())
    }

    pub async fn play_group(&self, leader: &str, play: bool) -> Result<(), String> {
        let player = self.player(leader).await?;
        let done = if play {
            player.client.play().await
        } else {
            player.client.pause().await
        };
        done.map_err(|e| e.to_string())
    }

    pub async fn speaker_volume(&self, speaker: &str, percent: u8) -> Result<(), String> {
        self.player(speaker)
            .await?
            .client
            .set_volume(percent.min(100))
            .await
            .map_err(|e| e.to_string())
    }

    /// What can be started. Radio: the stations among the favorites and
    /// the ones rooms played last, or TuneIn's for a search. Spotify:
    /// carrying on what a speaker has queued, and the playlists among
    /// the favorites, or Spotify's for a search.
    pub async fn choices(&self, kind: &str, query: Option<&str>) -> Result<Vec<Choice>, String> {
        let all = self.speakers.everywhere().await;
        if all.is_empty() {
            return Err("there are no speakers set up".into());
        }
        let favorites = self.favorites(&all).await;
        let mut choices = Vec::new();
        match (kind, query) {
            ("radio", Some(query)) => {
                let stations = tunein::search(&self.http, query)
                    .await
                    .map_err(|e| format!("TuneIn did not answer: {e}"))?;
                for station in stations.into_iter().take(10) {
                    let label = if station.about.is_empty() {
                        station.name.clone()
                    } else {
                        format!("{} — {}", station.name, station.about)
                    };
                    choices.push(choice("station", &station.id, &label));
                }
            }
            ("radio", None) => {
                for favorite in favorites.iter().filter(|f| f.station) {
                    choices.push(choice("favorite", &favorite.title, &favorite.title));
                }
                let last: Vec<(String, String)> = self
                    .lock()
                    .iter()
                    .map(|(room, station)| (room.clone(), station.title.clone()))
                    .collect();
                for (room, title) in last {
                    if !choices.iter().any(|c| c.label == title) {
                        choices.push(choice("remembered", &room, &title));
                    }
                }
            }
            ("spotify", Some(query)) => {
                let Some((credentials, market)) = self.spotify_keys() else {
                    return Err("Spotify is not set up in Niles".into());
                };
                let found = self
                    .spotify
                    .search(&credentials, query, market.as_deref())
                    .await
                    .map_err(|e| e.to_string())?;
                for item in super::candidates(found) {
                    let label = match (&item.kind, &item.by) {
                        (SpotifyKind::Artist, _) => format!("{} (artist)", item.name),
                        (SpotifyKind::Playlist, _) => format!("{} (playlist)", item.name),
                        (SpotifyKind::Album, Some(by)) => format!("{} — {by} (album)", item.name),
                        (_, Some(by)) => format!("{} — {by}", item.name),
                        (_, None) => item.name.clone(),
                    };
                    choices.push(choice(
                        "spotify",
                        &format!("{}|{}", item.uri, item.name),
                        &label,
                    ));
                }
            }
            ("spotify", None) => {
                for player in &all {
                    if player.sonos.coordinator != player.sonos.id {
                        continue;
                    }
                    let Ok(track) = player.client.track().await else {
                        continue;
                    };
                    if track.is_spotify() {
                        let what = track_line(&track).unwrap_or_else(|| "Spotify".into());
                        choices.push(choice(
                            "queue",
                            &player.sonos.id,
                            &format!("Carry on: {what} ({})", player.sonos.name),
                        ));
                    }
                }
                for favorite in favorites.iter().filter(|f| !f.station) {
                    choices.push(choice("favorite", &favorite.title, &favorite.title));
                }
            }
            (other, _) => return Err(format!("{other:?} is not radio or spotify")),
        }
        Ok(choices)
    }

    /// Group `speakers` and play `choice` on them.
    pub async fn start_on(&self, speakers: &[String], choice: &Choice) -> Result<(), String> {
        let all = self.speakers.everywhere().await;
        let players: Vec<Player> = all
            .iter()
            .filter(|p| speakers.contains(&p.sonos.id))
            .cloned()
            .collect();
        if players.is_empty() {
            return Err("none of those speakers is placed in a room".into());
        }
        match choice.kind.as_str() {
            "favorite" => {
                let favorites = self.favorites(&players).await;
                let favorite = favorites
                    .iter()
                    .find(|f| f.title == choice.id)
                    .ok_or_else(|| format!("no favorite called {:?}", choice.id))?;
                let content = if favorite.station {
                    Content::Stream {
                        uri: favorite.uri.clone(),
                        metadata: favorite.metadata.clone(),
                    }
                } else {
                    Content::Queue(vec![(favorite.uri.clone(), favorite.metadata.clone())])
                };
                self.load_and_play(&players, &content)
                    .await
                    .map_err(|e| e.to_string())
            }
            "station" => {
                let favorites = self.favorites(&players).await;
                let remembered: Vec<String> = self.lock().values().map(|r| r.uri.clone()).collect();
                let account = favorites
                    .iter()
                    .map(|f| f.uri.as_str())
                    .chain(remembered.iter().map(String::as_str))
                    .find_map(Account::of)
                    .unwrap_or(Account::OPEN);
                let name = choice.label.split(" — ").next().unwrap_or(&choice.label);
                let (uri, metadata) = account.play(&Station {
                    id: choice.id.clone(),
                    name: name.to_string(),
                    about: String::new(),
                });
                self.load_and_play(&players, &Content::Stream { uri, metadata })
                    .await
                    .map_err(|e| e.to_string())
            }
            "remembered" => {
                let station = self
                    .lock()
                    .get(&choice.id)
                    .cloned()
                    .ok_or_else(|| "that station is no longer remembered".to_string())?;
                let content = Content::Stream {
                    uri: station.uri,
                    metadata: station.metadata,
                };
                self.load_and_play(&players, &content)
                    .await
                    .map_err(|e| e.to_string())
            }
            "spotify" => {
                let (uri, name) = choice.id.split_once('|').unwrap_or((&choice.id, ""));
                let kind = match uri.split(':').nth(1) {
                    Some("artist") => SpotifyKind::Artist,
                    Some("track") => SpotifyKind::Track,
                    Some("album") => SpotifyKind::Album,
                    Some("playlist") => SpotifyKind::Playlist,
                    _ => return Err(format!("{uri:?} is not something Spotify plays")),
                };
                let item = Item {
                    kind,
                    uri: uri.to_string(),
                    name: name.to_string(),
                    by: None,
                };
                let Some((credentials, market)) = self.spotify_keys() else {
                    return Err("Spotify is not set up in Niles".into());
                };
                let items = self
                    .spotify_run(&credentials, market.as_deref(), &item)
                    .await?;
                match self.play_spotify(&players, items, name).await {
                    super::Outcome::Playing(_) => Ok(()),
                    other => Err(format!("{other:?}")),
                }
            }
            "queue" => {
                // Carry on what one speaker has queued: it leads, and the
                // others join it.
                let holder = all
                    .iter()
                    .find(|p| p.sonos.id == choice.id)
                    .ok_or_else(|| "that speaker is gone".to_string())?;
                for player in &players {
                    if player.sonos.id != holder.sonos.id
                        && player.sonos.coordinator != holder.sonos.id
                    {
                        player
                            .client
                            .join(&holder.sonos.id)
                            .await
                            .map_err(|e| e.to_string())?;
                    }
                }
                holder.client.play().await.map_err(|e| e.to_string())?;
                self.speakers.forget().await;
                Ok(())
            }
            other => Err(format!("{other:?} is not something to play")),
        }
    }

    /// The soundbar to the TV's sound, out of whatever group it is in —
    /// the rest of that group plays on without it.
    pub async fn tv_sound(&self) -> Result<(), String> {
        let all = self.speakers.everywhere().await;
        let cfg = self.config.current();
        let tv_room = cfg.tv.room.trim();
        let soundbar = all
            .iter()
            .filter(|p| p.sonos.home_theater)
            .find(|p| {
                tv_room.is_empty()
                    || cfg
                        .speakers
                        .sonos
                        .get(&p.sonos.id)
                        .is_some_and(|s| s.room == tv_room)
            })
            .or_else(|| all.iter().find(|p| p.sonos.home_theater))
            .ok_or_else(|| "no soundbar is placed in a room".to_string())?;
        if soundbar.sonos.coordinator != soundbar.sonos.id {
            let _ = soundbar.client.go_solo().await;
        }
        soundbar
            .client
            .load(
                &format!("x-sonos-htastream:{}:spdif", soundbar.sonos.id),
                "",
            )
            .await
            .map_err(|e| e.to_string())?;
        soundbar.client.play().await.map_err(|e| e.to_string())?;
        self.speakers.forget().await;
        Ok(())
    }
}

/// A speaker as the page shows it.
async fn speaker(player: &Player, cfg: &niles_config::Config) -> Speaker {
    Speaker {
        id: player.sonos.id.clone(),
        name: player.sonos.name.clone(),
        room: cfg
            .speakers
            .sonos
            .get(&player.sonos.id)
            .map(|s| s.room.clone()),
        volume: player.client.get_volume().await.ok(),
        soundbar: player.sonos.home_theater,
    }
}

fn choice(kind: &str, id: &str, label: &str) -> Choice {
    Choice {
        kind: kind.into(),
        id: id.into(),
        label: label.into(),
    }
}

/// What a group has loaded, playing or paused: `tv`, `radio`,
/// `spotify`, `music`, or `idle` for nothing at all.
async fn loaded(leader: &Player) -> (&'static str, Option<String>) {
    let Ok(media) = leader.client.media().await else {
        return ("idle", None);
    };
    if media.is_tv() {
        return ("tv", None);
    }
    if media.is_radio() {
        return ("radio", title_in(&media.metadata));
    }
    if media.uri.is_empty() {
        return ("idle", None);
    }
    let track = leader.client.track().await.unwrap_or_default();
    if track.uri.is_empty() {
        return ("idle", None);
    }
    let kind = if track.is_spotify() {
        "spotify"
    } else {
        "music"
    };
    (kind, track_line(&track))
}
