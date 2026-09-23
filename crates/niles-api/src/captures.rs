//! The kept wake audio, for getting it out again.
//!
//! Collecting recordings nobody can retrieve would be a microphone
//! writing to a database for its own sake. These routes are what make
//! the collection worth switching on: a list to see what has
//! accumulated, a download per clip, and a way to throw the lot away.

use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;

type Failure = (StatusCode, String);

fn store(state: &AppState) -> Result<&niles_db::PostgresCaptures, Failure> {
    state.captures.as_deref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "keeping wake audio needs a database".into(),
    ))
}

/// `GET /captures` — what has been kept, newest first, without audio.
pub async fn list(State(state): State<AppState>) -> Result<Json<Vec<niles_db::Capture>>, Failure> {
    let captures = store(&state)?
        .list()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("could not read them: {e}")))?;
    Ok(Json(captures))
}

/// `GET /captures/{id}.wav` — one recording, as a file.
pub async fn download(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, Failure> {
    let id: i64 = id
        .trim_end_matches(".wav")
        .parse()
        .map_err(|_| (StatusCode::BAD_REQUEST, "not a capture id".to_string()))?;
    let wav = store(&state)?
        .wav(id)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("could not read it: {e}")))?
        .ok_or((StatusCode::NOT_FOUND, "no such capture".to_string()))?;
    Ok((
        [
            (header::CONTENT_TYPE, "audio/wav".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"wake-{id}.wav\""),
            ),
        ],
        wav,
    ))
}

/// `DELETE /captures` — throw them all away.
///
/// All of them rather than one at a time. This is a pile of recordings
/// of a living room; the thing somebody wants from it is a way to be
/// rid of the pile.
pub async fn clear(State(state): State<AppState>) -> Result<StatusCode, Failure> {
    store(&state)?.clear().await.map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            format!("could not clear them: {e}"),
        )
    })?;
    tracing::info!("cleared the kept wake audio");
    Ok(StatusCode::NO_CONTENT)
}
