//! What each room is heated to.
//!
//! Read-only for now. Setting a temperature means writing an overlay to
//! tado, which has its own decisions — how long an override lasts, and
//! what ends it — and none of them are worth guessing at before there
//! is a page to make them on.

use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;

type Failure = (StatusCode, String);

/// One heating zone, as the dashboard sees it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ZoneDto {
    pub id: u64,
    /// What tado calls it.
    pub name: String,
    /// The Niles room it matches, or `null` when nothing does.
    pub room: Option<String>,
    pub temperature: Option<f32>,
    pub humidity: Option<f32>,
    /// What it is heating towards. `null` when the zone is off, which
    /// is not the same as zero.
    pub target: Option<f32>,
    pub on: bool,
    /// Whether somebody has overridden the schedule.
    pub overridden: bool,
    pub reachable: bool,
    /// How it came to have that room: `paired`, `name`, or `nowhere`.
    ///
    /// The page needs the difference. A room that happens to share a
    /// name is convenience, not an answer somebody gave — and offering
    /// to confirm it is very different from offering to change it.
    pub placed_by: niles_presence::Placed,
}

/// `GET /climate` — every heating zone tado reports.
///
/// An empty list is the normal answer for a house with no tado, and the
/// page shows nothing rather than an error.
pub async fn list_zones(State(state): State<AppState>) -> Result<Json<Vec<ZoneDto>>, Failure> {
    let Some(tado) = state.tado.as_ref() else {
        return Ok(Json(Vec::new()));
    };

    // The rooms Niles knows, so a zone can be matched to one by name.
    // From the registry rather than the config: a room exists because
    // something in it does.
    let mut rooms: Vec<String> = state
        .registry
        .list_all()
        .into_iter()
        .map(|d| d.id.room().as_str().to_string())
        .collect();
    rooms.sort();
    rooms.dedup();

    // What somebody has already said about which zone is which room.
    let paired = state
        .config
        .as_ref()
        .map(|c| c.current())
        .and_then(|cfg| cfg.presence.tado.as_ref().map(|t| t.rooms.clone()))
        .unwrap_or_default();

    let zones = tado
        .zones(&rooms, &paired)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("tado would not say: {e}")))?;

    Ok(Json(
        zones
            .into_iter()
            .map(|z| ZoneDto {
                id: z.id,
                name: z.name,
                room: z.room,
                temperature: z.temperature,
                humidity: z.humidity,
                target: z.target,
                on: z.on,
                overridden: z.overridden,
                reachable: z.reachable,
                placed_by: z.placed_by,
            })
            .collect(),
    ))
}
