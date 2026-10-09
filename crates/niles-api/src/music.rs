//! The music in each room, for the room cards: what plays, and the
//! pause, play and volume beside it.

use crate::state::AppState;
use async_trait::async_trait;
use axum::Json;
use axum::extract::{Path, State};
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

/// What answers for the rooms' music. Implemented by the binary, which
/// owns the speakers and how a room's group is paused.
#[async_trait]
pub trait Music: Send + Sync {
    async fn rooms(&self) -> Vec<RoomMusic>;
    async fn pause(&self, room: &str) -> Result<(), String>;
    async fn resume(&self, room: &str) -> Result<(), String>;
    async fn volume(&self, room: &str, percent: u8) -> Result<(), String>;
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
}
