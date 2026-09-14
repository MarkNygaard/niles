//! Axum handlers.

use crate::dto::DeviceDto;
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use niles_core::{Device, DeviceId, DeviceState, RoomName};

type Failure = (StatusCode, String);

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
    /// `[r, g, b]`, each `0..=255`.
    pub rgb: Option<[u8; 3]>,
}

pub async fn devices_in_room(
    State(state): State<AppState>,
    Path(room): Path<String>,
) -> Result<Json<Vec<DeviceDto>>, Failure> {
    let room_name = parse_room(&room)?;
    let devices: Vec<DeviceDto> = state
        .registry
        .list_room(&room_name)
        .iter()
        .map(DeviceDto::from)
        .collect();
    Ok(Json(devices))
}

/// `POST /rooms/{room}/{device}` — set one light or outlet.
///
/// `{device}` is the bare name, or `source:name` when the same name
/// exists in more than one source. Which is how a caller that already
/// knows the device — the UI, reading the registry — should always
/// address it, since then there is nothing to resolve.
pub async fn set_device(
    State(state): State<AppState>,
    Path((room, device)): Path<(String, String)>,
    Json(body): Json<SetDeviceBody>,
) -> Result<StatusCode, Failure> {
    let room = parse_room(&room)?;
    let device = resolve_switchable(&state, &room, &device)?;
    let desired = desired_state(&body)?;
    within_reach(&device, &body)?;
    let (topic, payload) = command_for(&state, &device.id, &desired)?;
    hold_against_the_curve(&state, &device.id, &desired);
    publish(&state, topic, payload).await?;
    record_unechoed(&state, &device.id, &desired);
    Ok(StatusCode::ACCEPTED)
}

/// Remember what a device has no way of telling us it did.
///
/// After the publish, not before: what did not go out should not be
/// claimed. See [`niles_mqtt::unechoed`] for which fields and why.
fn record_unechoed(state: &AppState, id: &DeviceId, sent: &DeviceState) {
    if let Some(echo) = niles_mqtt::unechoed(id, sent) {
        state.registry.merge_state(id, echo);
    }
}

#[derive(serde::Serialize)]
pub struct RoomApplied {
    /// How many lights the command went to.
    pub lights: usize,
}

/// `POST /rooms/{room}` — set everything switchable in a room at once.
///
/// The room card in the UI is one tap that means "all of these", and a
/// tap that fans out to six requests from a phone arrives as six lights
/// changing one after another. One request, one fan-out here, where the
/// broker is a millisecond away.
///
/// Unlike the single-device route this **narrows** rather than refuses:
/// the caller named a room, not a device, so a colour meant for the
/// strips is simply not sent to the bulbs that can't show one, and a
/// lamp on a smart plug still hears the "off". A device left with
/// nothing it can act on is skipped, as is one Niles has no route to —
/// a strip whose `[wled]` entry is missing shouldn't stop the ceiling
/// light from turning off.
pub async fn set_room(
    State(state): State<AppState>,
    Path(room): Path<String>,
    Json(body): Json<SetDeviceBody>,
) -> Result<(StatusCode, Json<RoomApplied>), Failure> {
    let room = parse_room(&room)?;
    let desired = desired_state(&body)?;

    let lights: Vec<Device> = state
        .registry
        .list_room(&room)
        .into_iter()
        .filter(Device::is_switchable)
        .collect();
    if lights.is_empty() {
        return Err((StatusCode::NOT_FOUND, format!("no lights in room {room}")));
    }

    fan_out(&state, &lights, &desired, &room.to_string()).await
}

/// Take a light out of the curve's hands, if this command is the kind
/// the curve would undo.
///
/// Before the publish, not after: the curve ticks once a minute and
/// would otherwise be free to overwrite in between — the same ordering
/// the voice path documents.
///
/// Only for commands that set a *level*. `on` alone is deliberately not
/// manual: turning a light on is how you ask for the curve back, and
/// the off→on transition clears the flag anyway. Sending brightness to
/// a light that is off is a level, so it counts — it just also happens
/// to turn it on, and the transition then clears what was set here,
/// which is correct. Asking for a dark room to come up at 20% is asking
/// for it once, not forever.
fn hold_against_the_curve(state: &AppState, id: &DeviceId, desired: &DeviceState) {
    let Some(manual) = state.manual_mode.as_ref() else {
        return;
    };
    let sets_a_level = desired.brightness.is_some()
        || desired.color_temp_kelvin.is_some()
        || desired.rgb.is_some();
    if sets_a_level {
        manual.flag(id);
    }
}

/// Send `desired` to each light, narrowed to what that light can act on.
///
/// `where_` names the scope for the error, which is the only thing the
/// room and the house do differently.
async fn fan_out(
    state: &AppState,
    lights: &[Device],
    desired: &DeviceState,
    where_: &str,
) -> Result<(StatusCode, Json<RoomApplied>), Failure> {
    let mut sent = 0;
    for light in lights {
        let narrowed = narrow_to(light, desired);
        let Some((topic, payload)) = state.router.format(&light.id, &narrowed) else {
            tracing::debug!("{} can't act on this, skipping", light.id);
            continue;
        };
        hold_against_the_curve(state, &light.id, &narrowed);
        publish(state, topic, payload).await?;
        record_unechoed(state, &light.id, &narrowed);
        sent += 1;
    }

    if sent == 0 {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("nothing in {where_} can act on that"),
        ));
    }
    Ok((StatusCode::ACCEPTED, Json(RoomApplied { lights: sent })))
}

/// `POST /lights` — set everything switchable in the house at once.
///
/// The bar above the rooms is one press meaning "the whole house", and
/// a press that fans out to a request per room arrives as the house
/// going dark room by room over a mobile connection. Same narrowing as
/// a room: the caller named the house, not a device, so each light
/// gets the part of the command it can act on.
pub async fn set_all_lights(
    State(state): State<AppState>,
    Json(body): Json<SetDeviceBody>,
) -> Result<(StatusCode, Json<RoomApplied>), Failure> {
    let desired = desired_state(&body)?;
    let lights: Vec<Device> = state
        .registry
        .list_all()
        .into_iter()
        .filter(Device::is_switchable)
        .collect();
    if lights.is_empty() {
        return Err((
            StatusCode::NOT_FOUND,
            "Niles has no lights to switch".into(),
        ));
    }
    fan_out(&state, &lights, &desired, "the house").await
}

fn parse_room(raw: &str) -> Result<RoomName, Failure> {
    RoomName::parse(raw).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("invalid room name {raw:?}: {e}"),
        )
    })
}

/// The device `{device}` names in `room`, if it is one a switch reaches.
///
/// Mirrors how the `set_device` tool resolves a name for the LLM: a
/// bare name is enough until two sources both have it, and then the
/// error says so rather than picking one.
fn resolve_switchable(state: &AppState, room: &RoomName, device: &str) -> Result<Device, Failure> {
    let (source, name) = match device.split_once(':') {
        Some((source, name)) => (Some(source), name),
        None => (None, device),
    };

    let matches: Vec<Device> = state
        .registry
        .list_room(room)
        .into_iter()
        .filter(|d| d.id.name().as_str() == name)
        .filter(|d| source.is_none_or(|s| d.id.source() == s))
        .collect();

    let device = match matches.as_slice() {
        [] => {
            return Err((
                StatusCode::NOT_FOUND,
                format!("device {device:?} not found in room {room}"),
            ));
        }
        [only] => only.clone(),
        several => {
            let mut sources: Vec<&str> = several.iter().map(|d| d.id.source()).collect();
            sources.sort_unstable();
            return Err((
                StatusCode::CONFLICT,
                format!(
                    "{device:?} exists in {}; address it as one of them, e.g. {}:{device}",
                    sources.join(" and "),
                    sources[0]
                ),
            ));
        }
    };

    if !device.is_switchable() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "device {} is a {:?}; nothing here can be switched on it",
                device.id, device.class
            ),
        ));
    }
    Ok(device)
}

/// Refuse a field this particular device cannot act on.
///
/// Silently dropping it would be worse: the caller named *this* device
/// and would be told the command was accepted while half of it went
/// nowhere. A lamp on a smart plug has one thing it can be told, and a
/// bulb with no colour channel is not a colour bulb.
fn within_reach(device: &Device, body: &SetDeviceBody) -> Result<(), Failure> {
    let refuse = |what: &str, why: String| Err((StatusCode::BAD_REQUEST, format!("{what}: {why}")));

    if !device.is_light() && (body.brightness.is_some() || body.color_temp_kelvin.is_some()) {
        return refuse(
            "brightness and colour temperature",
            format!("{} is an outlet; it can only be on or off", device.id),
        );
    }
    if body.rgb.is_some() && !device.supports_rgb() {
        return refuse("rgb", format!("{} has no colour channel", device.id));
    }
    if body.color_temp_kelvin.is_some() && !device.supports_color_temperature() {
        return refuse(
            "color_temp_kelvin",
            format!("{} has no white channel", device.id),
        );
    }
    Ok(())
}

/// The part of a room-wide command this device can act on.
fn narrow_to(device: &Device, desired: &DeviceState) -> DeviceState {
    if !device.is_light() {
        // An outlet has exactly one thing it can be told.
        return DeviceState {
            on: desired.on,
            ..Default::default()
        };
    }
    DeviceState {
        rgb: desired.rgb.filter(|_| device.supports_rgb()),
        color_temp_kelvin: desired
            .color_temp_kelvin
            .filter(|_| device.supports_color_temperature()),
        ..desired.clone()
    }
}

/// Validate the body into the state to ask for, rejecting anything a
/// light would have to guess at.
fn desired_state(body: &SetDeviceBody) -> Result<DeviceState, Failure> {
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
    // A light is in colour mode or white mode, not both, so a body
    // carrying each leaves which one wins to the firmware.
    if body.rgb.is_some() && body.color_temp_kelvin.is_some() {
        return Err((
            StatusCode::BAD_REQUEST,
            "rgb and color_temp_kelvin are alternatives; send one".into(),
        ));
    }

    let desired = DeviceState {
        on: body.on,
        brightness: body.brightness,
        color_temp_kelvin: body.color_temp_kelvin,
        rgb: body.rgb,
        ..Default::default()
    };
    if !niles_mqtt::is_actionable(&desired) {
        return Err((StatusCode::BAD_REQUEST, "no settable field provided".into()));
    }
    Ok(desired)
}

fn command_for(
    state: &AppState,
    id: &DeviceId,
    desired: &DeviceState,
) -> Result<(String, String), Failure> {
    state.router.format(id, desired).ok_or((
        StatusCode::BAD_REQUEST,
        // The body was already checked for actionable fields, so
        // reaching here means the source itself is unroutable —
        // typically a WLED strip with no `[wled]` entry.
        format!("Niles has no way to command {id}"),
    ))
}

async fn publish(state: &AppState, topic: String, payload: String) -> Result<(), Failure> {
    state
        .publisher
        .publish(topic, payload.into_bytes())
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("publish failed: {e}")))
}
