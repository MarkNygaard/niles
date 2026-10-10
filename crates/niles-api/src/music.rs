//! The music in each room, for the room cards: what plays, and the
//! pause, play and volume beside it.

use crate::state::AppState;
use async_trait::async_trait;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};

type Failure = (StatusCode, String);

/// One Niles room with a Sonos in it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RoomMusic {
    pub room: String,
    pub playing: bool,
    /// "Chariot by Gavin DeGraw", "DR P3" — absent when it cannot say.
    pub what: Option<String>,
    /// `music`, `radio` or `tv`: the TV's sound has its own mark.
    pub kind: Option<&'static str>,
    /// Percent, from the room's first speaker.
    pub volume: Option<u8>,
}

/// One Sonos (a room of Sonos's own: a speaker, a pair, a soundbar
/// with its rear speakers) on the Media page.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Speaker {
    /// Sonos's id, `RINCON_…`: what the page drags and sends back.
    pub id: String,
    /// What the Sonos app calls it: "Living Room Back".
    pub name: String,
    /// The Niles room it is placed in.
    pub room: Option<String>,
    pub volume: Option<u8>,
    /// The one that plays the TV.
    pub soundbar: bool,
}

/// Speakers playing one thing together, in step.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Group {
    /// The leading speaker's id: play and pause go to it, and a speaker
    /// joining the group joins it.
    pub leader: String,
    /// What is loaded: `tv`, `radio`, `spotify`, or `music` from
    /// anywhere else. Playing or paused alike.
    pub kind: &'static str,
    /// "DR P4 Østjylland", "Chariot by Gavin DeGraw".
    pub what: Option<String>,
    pub playing: bool,
    pub speakers: Vec<Speaker>,
}

/// Everything the Media page shows: the groups by what they have
/// loaded, and the speakers with nothing loaded at all.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MediaView {
    pub groups: Vec<Group>,
    pub idle: Vec<Speaker>,
}

/// Something to start playing, offered in the Media page's picker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Choice {
    /// `favorite` (by title), `station` (TuneIn id), `spotify` (URI),
    /// or `queue` (a speaker's id, whose loaded queue carries on).
    pub kind: String,
    pub id: String,
    pub label: String,
}

/// What answers for the rooms' music. Implemented by the binary, which
/// owns the speakers and how a room's group is paused.
#[async_trait]
pub trait Music: Send + Sync {
    async fn rooms(&self) -> Vec<RoomMusic>;
    async fn pause(&self, room: &str) -> Result<(), String>;
    async fn resume(&self, room: &str) -> Result<(), String>;
    async fn volume(&self, room: &str, percent: u8) -> Result<(), String>;

    async fn media(&self) -> MediaView;
    /// Play along with `leader`'s group.
    async fn join(&self, speaker: &str, leader: &str) -> Result<(), String>;
    /// Leave its group, and stop.
    async fn leave(&self, speaker: &str) -> Result<(), String>;
    async fn group(&self, leader: &str, play: bool) -> Result<(), String>;
    async fn speaker_volume(&self, speaker: &str, percent: u8) -> Result<(), String>;
    /// What can be started: `radio` or `spotify`, searched when `query`.
    async fn choices(&self, kind: &str, query: Option<&str>) -> Result<Vec<Choice>, String>;
    /// Group `speakers` and play `choice` on them.
    async fn start(&self, speakers: &[String], choice: &Choice) -> Result<(), String>;
    /// The soundbar back to the TV's sound, out of any group.
    async fn tv(&self) -> Result<(), String>;
}

fn music(state: &AppState) -> Result<&dyn Music, Failure> {
    state.music.as_deref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "there are no speakers set up".to_string(),
    ))
}

/// `GET /music`
pub async fn rooms(State(state): State<AppState>) -> Result<Json<Vec<RoomMusic>>, Failure> {
    Ok(Json(music(&state)?.rooms().await))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", tag = "action")]
pub enum Control {
    Pause,
    Play,
    Volume { percent: u8 },
}

/// `POST /music/{room}` — `{"action":"pause"}`, `{"action":"play"}`,
/// `{"action":"volume","percent":30}`.
pub async fn control(
    State(state): State<AppState>,
    Path(room): Path<String>,
    Json(body): Json<Control>,
) -> Result<StatusCode, Failure> {
    let music = music(&state)?;
    match body {
        Control::Pause => music.pause(&room).await,
        Control::Play => music.resume(&room).await,
        Control::Volume { percent } => music.volume(&room, percent.min(100)).await,
    }
    .map_err(|e| (StatusCode::BAD_GATEWAY, e))?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /media`
pub async fn media(State(state): State<AppState>) -> Result<Json<MediaView>, Failure> {
    Ok(Json(music(&state)?.media().await))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", tag = "action")]
pub enum SpeakerControl {
    Join { leader: String },
    Leave,
    Volume { percent: u8 },
}

/// `POST /media/speakers/{id}` — `{"action":"join","leader":"RINCON_…"}`,
/// `{"action":"leave"}`, `{"action":"volume","percent":20}`.
pub async fn speaker(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<SpeakerControl>,
) -> Result<StatusCode, Failure> {
    let music = music(&state)?;
    match body {
        SpeakerControl::Join { leader } => music.join(&id, &leader).await,
        SpeakerControl::Leave => music.leave(&id).await,
        SpeakerControl::Volume { percent } => music.speaker_volume(&id, percent.min(100)).await,
    }
    .map_err(|e| (StatusCode::BAD_GATEWAY, e))?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /media/groups/{leader}` — `{"action":"play"}` or `{"action":"pause"}`.
pub async fn group(
    State(state): State<AppState>,
    Path(leader): Path<String>,
    Json(body): Json<Control>,
) -> Result<StatusCode, Failure> {
    let play = match body {
        Control::Play => true,
        Control::Pause => false,
        Control::Volume { .. } => {
            return Err((
                StatusCode::BAD_REQUEST,
                "a group's volume is its speakers'".to_string(),
            ));
        }
    };
    music(&state)?
        .group(&leader, play)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e))?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub struct ChoicesQuery {
    pub kind: String,
    #[serde(default)]
    pub q: Option<String>,
}

/// `GET /media/choices?kind=radio` / `?kind=spotify&q=gavin`
pub async fn choices(
    State(state): State<AppState>,
    Query(query): Query<ChoicesQuery>,
) -> Result<Json<Vec<Choice>>, Failure> {
    let q = query.q.as_deref().map(str::trim).filter(|q| !q.is_empty());
    music(&state)?
        .choices(&query.kind, q)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::BAD_GATEWAY, e))
}

#[derive(Debug, Deserialize)]
pub struct Start {
    pub speakers: Vec<String>,
    pub choice: Choice,
}

/// `POST /media/start` — `{"speakers":["RINCON_…"],"choice":{…}}`
pub async fn start(
    State(state): State<AppState>,
    Json(body): Json<Start>,
) -> Result<StatusCode, Failure> {
    if body.speakers.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "choose a speaker".to_string()));
    }
    music(&state)?
        .start(&body.speakers, &body.choice)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e))?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /media/tv` — the soundbar to the TV's sound.
pub async fn tv(State(state): State<AppState>) -> Result<StatusCode, Failure> {
    music(&state)?
        .tv()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e))?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_each_control() {
        let pause: Control = serde_json::from_str(r#"{"action":"pause"}"#).unwrap();
        assert!(matches!(pause, Control::Pause));
        let volume: Control = serde_json::from_str(r#"{"action":"volume","percent":30}"#).unwrap();
        assert!(matches!(volume, Control::Volume { percent: 30 }));
        assert!(serde_json::from_str::<Control>(r#"{"action":"louder"}"#).is_err());
    }

    #[test]
    fn reads_each_speaker_control() {
        let join: SpeakerControl =
            serde_json::from_str(r#"{"action":"join","leader":"RINCON_BAR"}"#).unwrap();
        assert!(matches!(join, SpeakerControl::Join { leader } if leader == "RINCON_BAR"));
        let leave: SpeakerControl = serde_json::from_str(r#"{"action":"leave"}"#).unwrap();
        assert!(matches!(leave, SpeakerControl::Leave));
    }
}
