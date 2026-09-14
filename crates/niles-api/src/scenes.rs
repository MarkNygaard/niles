//! Scenes, from the app.
//!
//! They were voice-only: you could save one by speaking and recall it
//! by speaking, and there was nowhere to see that you had. Which is a
//! poor deal for the thing you most want to press without thinking —
//! and a scene you have forgotten the name of is a scene you cannot
//! use at all.

use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;

type Failure = (StatusCode, String);

/// `GET /scenes` — what is saved, by name.
///
/// An empty list is the normal answer for somebody who has never saved
/// one, and the page shows nothing at all rather than an empty shelf.
pub async fn list_scenes(State(state): State<AppState>) -> Json<Vec<String>> {
    Json(
        state
            .scenes
            .as_ref()
            .map(|scenes| scenes.names())
            .unwrap_or_default(),
    )
}

/// `POST /scenes/{name}` — put the house back the way that scene had it.
pub async fn apply_scene(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<StatusCode, Failure> {
    let scenes = state.scenes.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "this Niles instance has no scene store".to_string(),
    ))?;
    let entries = scenes.get(&name).ok_or((
        StatusCode::NOT_FOUND,
        format!("there is no scene called {name}"),
    ))?;
    if entries.is_empty() {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("{name} was saved with nothing in it"),
        ));
    }

    for entry in entries {
        let Some((topic, payload)) = state.router.format(&entry.device_id, &entry.state) else {
            continue;
        };
        // Before the publish, matching the voice path: the curve ticks
        // once a minute and would otherwise be free to undo a scene in
        // between.
        if let Some(manual) = state.manual_mode.as_ref() {
            manual.flag(&entry.device_id);
        }
        state
            .publisher
            .publish(topic, payload.into_bytes())
            .await
            .map_err(|e| (StatusCode::BAD_GATEWAY, format!("publish failed: {e}")))?;
    }
    Ok(StatusCode::ACCEPTED)
}

/// `DELETE /scenes/{name}` — forget one.
pub async fn delete_scene(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<StatusCode, Failure> {
    let scenes = state.scenes.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "this Niles instance has no scene store".to_string(),
    ))?;
    if !scenes.delete(&name) {
        return Err((
            StatusCode::NOT_FOUND,
            format!("there is no scene called {name}"),
        ));
    }
    Ok(StatusCode::NO_CONTENT)
}
