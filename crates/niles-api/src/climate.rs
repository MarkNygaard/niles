//! What each room is heated to.
//!
//! Read-only for now. Setting a temperature means writing an overlay to
//! tado, which has its own decisions — how long an override lasts, and
//! what ends it — and none of them are worth guessing at before there
//! is a page to make them on.

use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;

type Failure = (StatusCode, String);

/// What to do to a zone.
///
/// One body rather than three routes, because they are one decision on
/// the page: the slider sets a temperature, the bottom of its range is
/// off, and the button beside it hands the zone back to the schedule.
#[derive(Debug, serde::Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum SetZone {
    /// Hold it here until somebody says otherwise.
    Heat { celsius: f32 },
    /// Off, which in tado still means frost protection.
    Off,
    /// Drop the override; the schedule has it back.
    Resume,
}

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
    /// When the override ends, if it ends by itself. Absent for one
    /// that lasts until somebody resumes the schedule — which is the
    /// one worth saying out loud, because it is the one that will be
    /// forgotten.
    pub until: Option<String>,
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
                until: z.until.map(|t| t.to_rfc3339()),
                placed_by: z.placed_by,
            })
            .collect(),
    ))
}

/// What a boost does, which is what tado's own boost does.
///
/// Warm, not hot, and short: the button exists for a cold half hour in
/// a room somebody has just walked into, and anything longer is what
/// the schedule is for. Timed rather than held, so a house nobody
/// remembers to turn back down turns itself back down.
const BOOST_CELSIUS: f32 = 25.0;
const BOOST_SECONDS: u32 = 30 * 60;

/// What a boost did, so the page can say so.
#[derive(Debug, serde::Serialize)]
pub struct BoostedDto {
    pub rooms: usize,
    pub celsius: f32,
    pub minutes: u32,
}

/// `POST /climate/boost` — warm every room for half an hour.
pub async fn boost(State(state): State<AppState>) -> Result<Json<BoostedDto>, Failure> {
    let tado = state.tado.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "this Niles instance has no tado connection".to_string(),
    ))?;

    let rooms = tado
        .boost_all(BOOST_CELSIUS, BOOST_SECONDS)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("tado refused it: {e}")))?;

    Ok(Json(BoostedDto {
        rooms,
        celsius: BOOST_CELSIUS,
        minutes: BOOST_SECONDS / 60,
    }))
}

/// Which zones to hand back to their schedules.
#[derive(Debug, serde::Deserialize)]
pub struct ResumeZones {
    pub zones: Vec<u64>,
}

/// `POST /climate/resume` — end a boost early.
///
/// Named zones rather than all of them, because ending a boost must
/// not also undo a room somebody set by hand an hour ago.
pub async fn resume_zones(
    State(state): State<AppState>,
    Json(body): Json<ResumeZones>,
) -> Result<StatusCode, Failure> {
    let tado = state.tado.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "this Niles instance has no tado connection".to_string(),
    ))?;

    tado.resume_all(&body.zones)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("tado refused it: {e}")))?;

    Ok(StatusCode::ACCEPTED)
}

/// `POST /climate/{zone}` — heat it, turn it off, or hand it back.
pub async fn set_zone(
    State(state): State<AppState>,
    Path(zone): Path<u64>,
    Json(body): Json<SetZone>,
) -> Result<StatusCode, Failure> {
    let tado = state.tado.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "this Niles instance has no tado connection".to_string(),
    ))?;

    match body {
        SetZone::Heat { celsius } => {
            // tado's own range. Refused here rather than forwarded,
            // because its error for this is a 422 with a body nobody
            // wants to read.
            if !(5.0..=25.0).contains(&celsius) {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("{celsius}° is outside what tado will take (5–25)"),
                ));
            }
            tado.set_temperature(zone, celsius).await
        }
        SetZone::Off => tado.turn_off(zone).await,
        SetZone::Resume => tado.resume_schedule(zone).await,
    }
    .map_err(|e| (StatusCode::BAD_GATEWAY, format!("tado refused it: {e}")))?;

    Ok(StatusCode::ACCEPTED)
}
