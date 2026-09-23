//! Getting tado connected, from the app rather than from a ConfigMap.
//!
//! The device flow needs a person with a browser. A service has
//! neither, so these two routes hand the job to whoever is looking at
//! Settings: one says what the state is, the other starts an
//! authorisation and gives back the code to approve.
//!
//! The pending activation lives on the source, not here, so the code
//! shown on the page is the same one the background task is polling
//! for. Two codes in flight means approving one and watching the other
//! stay pending.

use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;

type Failure = (StatusCode, String);

#[derive(serde::Serialize)]
pub struct TadoStatus {
    /// Whether there is anywhere to keep a token — which is to say,
    /// whether connecting is possible at all. False means no database.
    pub connectable: bool,
    /// Whether somebody has already approved a code.
    pub authorised: bool,
    /// Whether the feature is switched on. Separate from being
    /// connected, and second: authorising something nothing uses is
    /// harmless, turning it on before anything is connected is not.
    pub presence_enabled: bool,
    /// The code currently waiting to be approved, if any.
    pub pending: Option<PendingDto>,
}

#[derive(serde::Serialize)]
pub struct PendingDto {
    pub verification_uri: String,
    pub user_code: String,
    pub expires_at: String,
}

impl From<niles_presence::DeviceActivation> for PendingDto {
    fn from(p: niles_presence::DeviceActivation) -> Self {
        Self {
            verification_uri: p.verification_uri,
            user_code: p.user_code,
            expires_at: p.expires_at.to_rfc3339(),
        }
    }
}

/// `GET /presence/tado` — what the Settings card needs to draw itself.
///
/// Deliberately does not start an authorisation: opening a page must
/// not spend tado's daily quota.
pub async fn tado_status(State(state): State<AppState>) -> Json<TadoStatus> {
    let enabled = state
        .config
        .as_ref()
        .map(|c| c.current().presence.enabled)
        .unwrap_or(false);
    let Some(tado) = state.tado.as_ref() else {
        return Json(TadoStatus {
            connectable: false,
            authorised: false,
            presence_enabled: enabled,
            pending: None,
        });
    };
    Json(TadoStatus {
        connectable: true,
        authorised: tado.is_authorised().await.unwrap_or(false),
        presence_enabled: enabled,
        pending: tado.pending_activation().await.map(PendingDto::from),
    })
}

/// `POST /presence/tado/connect` — start an authorisation, or hand back
/// the one already running.
pub async fn tado_connect(State(state): State<AppState>) -> Result<Json<PendingDto>, Failure> {
    let Some(tado) = state.tado.as_ref() else {
        return Err((
            StatusCode::NOT_IMPLEMENTED,
            "no database, so there is nowhere to keep tado's token".into(),
        ));
    };
    if tado.is_authorised().await.unwrap_or(false) {
        return Err((StatusCode::CONFLICT, "already connected to tado".into()));
    }
    let (pending, is_new) = tado
        .ensure_activation()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("tado refused: {e}")))?;

    // Whoever started it waits on it. Nothing else polls tado in the
    // background any more: an authorisation is something a person asks
    // for, and a service quietly asking for codes nobody approves
    // spends a daily quota on nothing.
    if is_new {
        wait_for_approval(tado.clone(), pending.clone());
    }
    Ok(Json(PendingDto::from(pending)))
}

/// Poll tado until the code is approved, gives up, or runs out.
///
/// The page watches our own status rather than tado's, so this is the
/// only thing that touches tado while a code is outstanding — at the
/// interval tado asked for, and not faster.
fn wait_for_approval(
    tado: std::sync::Arc<niles_presence::TadoSource>,
    pending: niles_presence::DeviceActivation,
) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(pending.interval).await;
            match tado.finish_activation(&pending).await {
                Ok(true) => {
                    tado.clear_activation().await;
                    tracing::info!("[tado] authorised — presence can be switched on");
                    return;
                }
                Ok(false) if chrono::Utc::now() < pending.expires_at => continue,
                Ok(false) => {
                    tado.clear_activation().await;
                    tracing::warn!("[tado] the code expired unapproved");
                    return;
                }
                Err(e) => {
                    tado.clear_activation().await;
                    tracing::warn!("[tado] authorisation failed: {e}");
                    return;
                }
            }
        }
    });
}

#[derive(serde::Serialize)]
pub struct SetupReport {
    pub set_up: bool,
    pub gaps: Vec<niles_config::Gap>,
}

/// `GET /setup` — what is still unset.
///
/// Niles starts with no config at all now, so starting no longer proves
/// it is configured. This is what carries the difference, and it is why
/// the defaults are allowed to be as permissive as they are.
pub async fn setup_report(State(state): State<AppState>) -> Json<SetupReport> {
    let Some(store) = state.config.as_ref() else {
        return Json(SetupReport {
            set_up: true,
            gaps: Vec::new(),
        });
    };
    let cfg = store.current();
    Json(SetupReport {
        set_up: cfg.is_set_up(),
        gaps: cfg.setup_gaps(),
    })
}

// ---------------------------------------------------------------------
// Pairing a phone to a person
// ---------------------------------------------------------------------

/// What Niles can see of the device making this request.
#[derive(Debug, serde::Serialize)]
pub struct DeviceView {
    /// Whether there is a console to ask at all.
    pub available: bool,
    /// Whether this request came from the house rather than through the
    /// tunnel. Pairing needs the phone's own address, and a request from
    /// outside carries somebody else's.
    pub on_home_network: bool,
    /// The address the console has for whoever is asking.
    pub mac: Option<String>,
    /// What the console calls it — "Mark's iPhone".
    pub name: Option<String>,
    /// Whether that is already the phone on this person's entry.
    pub paired: bool,
    /// Whether anybody is signed in to pair it to.
    pub signed_in: bool,
}

/// The address a request came from, as Envoy reports it.
///
/// `X-Forwarded-For` holds the original client; everything reaches this
/// process through the gateway, so there is always a hop in front. The
/// first entry is the caller.
fn client_ip(headers: &axum::http::HeaderMap) -> Option<String> {
    headers
        .get("x-forwarded-for")?
        .to_str()
        .ok()?
        .split(',')
        .next()
        .map(|ip| ip.trim().to_string())
        .filter(|ip| !ip.is_empty())
}

/// Whether an address is one of ours.
///
/// A phone on the home Wi-Fi presents a private address; the same phone
/// on mobile data arrives through the tunnel carrying a public one, and
/// pairing *that* would attach the tunnel to a person.
fn is_home_network(ip: &str) -> bool {
    match ip.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(v4)) => v4.is_private() || v4.is_loopback(),
        Ok(std::net::IpAddr::V6(v6)) => v6.is_loopback() || (v6.segments()[0] & 0xfe00) == 0xfc00,
        Err(_) => false,
    }
}

/// `GET /presence/device` — what Niles sees of the phone asking.
///
/// The dashboard draws its pairing offer from this and from nothing
/// else, so it can render once and correctly rather than guessing and
/// then correcting itself.
pub async fn device_status(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Json<DeviceView> {
    // Configured, not merely constructed: the source always exists now,
    // so it can pick up a host and key typed in while Niles is running.
    // Whether it can answer is the question.
    let Some(unifi) = state.unifi.as_ref().filter(|u| u.is_configured()) else {
        return Json(DeviceView {
            available: false,
            on_home_network: false,
            mac: None,
            name: None,
            paired: false,
            signed_in: false,
        });
    };

    let config = state.config.as_ref().map(|c| c.current());
    let who = config
        .as_ref()
        .and_then(|c| crate::auth::signed_in_as(&headers, c));

    let ip = client_ip(&headers).filter(|ip| is_home_network(ip));
    let seen = match &ip {
        Some(ip) => unifi.mac_at(ip).await.unwrap_or_else(|e| {
            tracing::warn!("could not ask the console who holds {ip}: {e}");
            None
        }),
        None => None,
    };

    let mac = seen.as_ref().and_then(|c| c.mac());
    let theirs = who.as_ref().zip(config.as_ref()).and_then(|(email, cfg)| {
        cfg.auth
            .person_for(email)
            .and_then(|p| p.device_mac.clone())
            .map(|m| m.trim().to_lowercase())
    });

    Json(DeviceView {
        available: true,
        on_home_network: ip.is_some(),
        paired: mac.is_some() && mac == theirs,
        name: seen.as_ref().and_then(|c| c.name.clone()),
        mac,
        signed_in: who.is_some(),
    })
}

/// `POST /presence/device` — this phone is mine.
///
/// Writes the address the console reports onto the signed-in person, so
/// presence knows who is home rather than only that somebody is. One
/// per person, replacing whatever was there: a new phone is paired the
/// same way the first one was.
pub async fn pair_device(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<Json<DeviceView>, Failure> {
    let store = state.config.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "this Niles instance has nowhere to save it".to_string(),
    ))?;
    let unifi = state.unifi.as_ref().filter(|u| u.is_configured()).ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "no UniFi console is configured".to_string(),
    ))?;

    let config = store.current();
    let who = crate::auth::signed_in_as(&headers, &config).ok_or((
        StatusCode::UNAUTHORIZED,
        "nobody is signed in to pair a phone to".to_string(),
    ))?;

    let ip = client_ip(&headers)
        .filter(|ip| is_home_network(ip))
        .ok_or((
            StatusCode::BAD_REQUEST,
            "this request did not come from the home network — connect to the Wi-Fi and try again"
                .to_string(),
        ))?;

    let client = unifi
        .mac_at(&ip)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                format!("the console would not say: {e}"),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            format!("the console does not know a device at {ip}"),
        ))?;
    let mac = client.mac().ok_or((
        StatusCode::BAD_GATEWAY,
        "the console reported that device without an address".to_string(),
    ))?;

    // The whole list, because an array is replaced rather than merged —
    // and because "one phone per person" is a property of this list
    // being rewritten as a whole.
    let allowed: Vec<toml::Value> = config
        .auth
        .allowed
        .iter()
        .map(|person| {
            let mut row = toml::map::Map::new();
            row.insert("email".into(), toml::Value::String(person.email.clone()));
            if let Some(speaker) = &person.speaker {
                row.insert("speaker".into(), toml::Value::String(speaker.clone()));
            }
            let device = if person.email == who {
                Some(mac.clone())
            } else {
                person.device_mac.clone()
            };
            if let Some(device) = device {
                row.insert("device_mac".into(), toml::Value::String(device));
            }
            toml::Value::Table(row)
        })
        .collect();

    let mut auth = toml::map::Map::new();
    auth.insert("allowed".into(), toml::Value::Array(allowed));
    let mut patch = toml::map::Map::new();
    patch.insert("auth".into(), toml::Value::Table(auth));

    store
        .apply(&patch, niles_config::ChangeSource::Api)
        .await
        .map_err(|e| (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;

    tracing::info!("{who} paired the phone at {ip} ({mac})");
    Ok(Json(DeviceView {
        available: true,
        on_home_network: true,
        mac: Some(mac),
        name: client.name.clone(),
        paired: true,
        signed_in: true,
    }))
}
