//! Axum handlers.

use crate::dto::DeviceDto;
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use niles_core::RoomName;

pub async fn healthz() -> &'static str {
    "ok"
}

pub async fn list_devices(State(state): State<AppState>) -> Json<Vec<DeviceDto>> {
    let devices: Vec<DeviceDto> = state
        .registry
        .list_all()
        .iter()
        .map(DeviceDto::from)
        .collect();
    Json(devices)
}

pub async fn devices_in_room(
    State(state): State<AppState>,
    Path(room): Path<String>,
) -> Result<Json<Vec<DeviceDto>>, (StatusCode, String)> {
    let room_name = RoomName::parse(&room).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("invalid room name {room:?}: {e}"),
        )
    })?;
    let devices: Vec<DeviceDto> = state
        .registry
        .list_room(&room_name)
        .iter()
        .map(DeviceDto::from)
        .collect();
    Ok(Json(devices))
}
