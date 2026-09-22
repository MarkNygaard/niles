//! The voices Niles has been taught.
//!
//! Until this existed there was no way to see what recognition knew.
//! A voice print lives in Postgres and announced itself once, when
//! somebody said their name — after that, whether it had one clip or
//! six, and whether it had been heard since, was invisible. That is
//! not a cosmetic gap: an enrolment that had gone wrong looked exactly
//! like one that had gone right.

use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::Deserialize;

type Failure = (StatusCode, String);

/// One enrolled voice, as the page sees it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct VoiceDto {
    /// The slug commands and `auth.allowed[].speaker` refer to.
    pub speaker: String,
    pub display_name: String,
    /// How many clips the print is built from. One is thin; the voice
    /// asks for more until it has three.
    pub clip_count: usize,
    pub created_at: String,
    /// When Niles last recognised them, if ever. `None` on a voice
    /// enrolled and never matched since — which is the shape of an
    /// enrolment that is not working.
    pub last_seen_at: Option<String>,
}

fn roster(state: &AppState) -> Result<&dyn niles_recognition::VoiceRoster, Failure> {
    state.voices.as_deref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "speaker recognition is not configured".into(),
    ))
}

/// `GET /voices` — everybody enrolled.
pub async fn list(State(state): State<AppState>) -> Result<Json<Vec<VoiceDto>>, Failure> {
    let voices = roster(&state)?
        .voices()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("could not read them: {e}")))?;
    Ok(Json(
        voices
            .into_iter()
            .map(|v| VoiceDto {
                speaker: v.speaker,
                display_name: v.display_name,
                clip_count: v.clip_count,
                created_at: v.created_at.to_rfc3339(),
                last_seen_at: v.last_seen_at.map(|t| t.to_rfc3339()),
            })
            .collect(),
    ))
}

/// `DELETE /voices/{speaker}` — forget one entirely.
///
/// The way to start a voice over. A print built from a bad first clip
/// cannot be repaired by adding better ones — the bad one stays a
/// reference, and with `MaxSimilarity` a bad reference is one more
/// thing a stranger can match.
pub async fn forget(
    State(state): State<AppState>,
    Path(speaker): Path<String>,
) -> Result<StatusCode, Failure> {
    roster(&state)?.forget(&speaker).await.map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            format!("could not forget them: {e}"),
        )
    })?;
    tracing::info!("forgot the voice {speaker:?}");
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub struct Rename {
    pub display_name: String,
}

/// `PUT /voices/{speaker}` — correct the name.
///
/// The name came from a transcript, and a transcript is a guess.
/// Whisper spelled one Danish name four ways in four attempts; the
/// slug is stuck with whichever it produced first, but what anybody
/// reads does not have to be.
pub async fn rename(
    State(state): State<AppState>,
    Path(speaker): Path<String>,
    axum::Json(body): axum::Json<Rename>,
) -> Result<StatusCode, Failure> {
    let name = body.display_name.trim();
    if name.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "a name cannot be blank".into()));
    }
    roster(&state)?.rename(&speaker, name).await.map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            format!("could not rename them: {e}"),
        )
    })?;
    tracing::info!("the voice {speaker:?} is now called {name:?}");
    Ok(StatusCode::NO_CONTENT)
}
