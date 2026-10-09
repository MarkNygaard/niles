//! The TV, for the app: pairing it, its state, and its power button.

use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use niles_config::ChangeSource;
use serde::{Deserialize, Serialize};

type Failure = (StatusCode, String);

#[derive(Debug, Serialize)]
pub struct TvDto {
    /// An address to reach it at.
    pub configured: bool,
    /// Paired: Niles has the key the TV issued.
    pub paired: bool,
    /// The address it will be woken with, once known.
    pub mac: Option<String>,
    pub room: Option<String>,
    /// Absent when it is not paired, or could not be asked.
    pub status: Option<niles_webos::Status>,
    pub error: Option<String>,
}

/// `GET /tv`
pub async fn get(State(state): State<AppState>) -> Json<TvDto> {
    let Some(cfg) = state.config.as_ref().map(|c| c.current()) else {
        return Json(TvDto {
            configured: false,
            paired: false,
            mac: None,
            room: None,
            status: None,
            error: None,
        });
    };
    let tv = &cfg.tv;
    let key = tv.resolve_client_key().ok();
    let (status, error) = match (&key, tv.is_configured()) {
        (Some(key), true) => match niles_webos::Tv::new(tv.host.trim(), key).status().await {
            Ok(status) => (Some(status), None),
            Err(e) => (None, Some(e.to_string())),
        },
        _ => (None, None),
    };
    let present = |s: &str| (!s.trim().is_empty()).then(|| s.trim().to_string());
    Json(TvDto {
        configured: tv.is_configured(),
        paired: key.is_some(),
        mac: present(&tv.mac),
        room: present(&tv.room),
        status,
        error,
    })
}

#[derive(Debug, Serialize)]
pub struct Paired {
    pub mac: Option<String>,
}

/// `POST /tv/pair` — the TV shows a prompt, somebody accepts it with the
/// remote, and the key it issues is kept as a credential. Its MAC
/// address is read while connected and written to `[tv]`, so waking it
/// needs nothing typed. Answers when the prompt is accepted, or after a
/// minute of nobody accepting it.
pub async fn pair(State(state): State<AppState>) -> Result<Json<Paired>, Failure> {
    let store = state.config.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "this Niles instance has no config store".to_string(),
    ))?;
    let secrets = crate::secrets::writable(&state)?;
    let host = store.current().tv.host.trim().to_string();
    if host.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "give the TV's address first".to_string(),
        ));
    }
    let key = niles_webos::pair(&host)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    secrets.set("tv.client_key", &key).await.map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            format!("could not keep the key: {e}"),
        )
    })?;
    crate::secrets::reload(&state).await;

    let mac = match niles_webos::Tv::new(&host, &key).mac_address().await {
        Ok(mac) => mac,
        Err(e) => {
            tracing::warn!("[tv] paired, but its MAC address could not be read: {e}");
            None
        }
    };
    if let Some(mac) = &mac {
        let mut tv = toml::Table::new();
        tv.insert("mac".into(), toml::Value::String(mac.clone()));
        let mut patch = toml::Table::new();
        patch.insert("tv".into(), toml::Value::Table(tv));
        store.apply(&patch, ChangeSource::Api).await.map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                format!("could not keep the MAC: {e}"),
            )
        })?;
    }
    Ok(Json(Paired { mac }))
}

#[derive(Debug, Deserialize)]
pub struct Power {
    pub on: bool,
}

/// `POST /tv/power` — `{"on": true}` wakes it, `{"on": false}` turns it off.
pub async fn power(
    State(state): State<AppState>,
    Json(body): Json<Power>,
) -> Result<StatusCode, Failure> {
    let cfg = state
        .config
        .as_ref()
        .map(|c| c.current())
        .ok_or((StatusCode::NOT_IMPLEMENTED, "no config store".to_string()))?;
    let tv = &cfg.tv;
    if !tv.is_configured() {
        return Err((StatusCode::NOT_FOUND, "there is no TV set up".to_string()));
    }
    let failed = |e: niles_webos::Error| (StatusCode::BAD_GATEWAY, e.to_string());
    if body.on {
        if tv.mac.trim().is_empty() {
            return Err((
                StatusCode::CONFLICT,
                "the TV's MAC address is not known; pair it again".to_string(),
            ));
        }
        niles_webos::wake(tv.host.trim(), tv.mac.trim())
            .await
            .map_err(failed)?;
    } else {
        let key = tv
            .resolve_client_key()
            .map_err(|_| (StatusCode::CONFLICT, "the TV is not paired".to_string()))?;
        niles_webos::Tv::new(tv.host.trim(), key)
            .turn_off()
            .await
            .map_err(failed)?;
    }
    Ok(StatusCode::NO_CONTENT)
}
