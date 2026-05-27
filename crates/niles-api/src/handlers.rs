//! Axum handlers.

use crate::dto::DeviceDto;
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use niles_core::{DeviceId, DeviceState, RoomName};

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

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetDeviceBody {
    pub on: Option<bool>,
    pub brightness: Option<u8>,
    pub color_temp_kelvin: Option<u16>,
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

pub async fn set_device(
    State(state): State<AppState>,
    Path((room, device)): Path<(String, String)>,
    Json(body): Json<SetDeviceBody>,
) -> Result<StatusCode, (StatusCode, String)> {
    let id = DeviceId::parse(&format!("z2m:{room}/{device}")).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("invalid device id z2m:{room}/{device}: {e}"),
        )
    })?;

    let dev = state
        .registry
        .get(&id)
        .ok_or((StatusCode::NOT_FOUND, format!("device {id} not found")))?;

    if !dev.is_light() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("device {id} is a {:?}, not a light", dev.class),
        ));
    }

    if let Some(b) = body.brightness
        && b > 100
    {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("brightness {b} is out of range (0–100)"),
        ));
    }

    if let Some(k) = body.color_temp_kelvin
        && !(1000..=10000).contains(&k)
    {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("color_temp_kelvin {k} is out of range (1000–10000)"),
        ));
    }

    let desired = DeviceState {
        on: body.on,
        brightness: body.brightness,
        color_temp_kelvin: body.color_temp_kelvin,
        ..Default::default()
    };

    if !niles_mqtt::is_actionable(&desired) {
        return Err((StatusCode::BAD_REQUEST, "no settable field provided".into()));
    }

    let (topic, payload) = niles_mqtt::format_set_command(&state.z2m_prefix, &id, &desired);
    state
        .publisher
        .publish(topic, payload.into_bytes())
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("publish failed: {e}")))?;

    Ok(StatusCode::ACCEPTED)
}
