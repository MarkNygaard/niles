//! The TV, for the app: pairing it, its state, its power button and its
//! volume.

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

#[derive(Debug, Deserialize)]
pub struct VolumeBody {
    /// To a level, `0..=100`.
    pub volume: Option<u8>,
    /// Up or down from where it is.
    pub delta: Option<i8>,
    pub mute: Option<bool>,
}

/// The one change a body asks for.
fn change_for(body: &VolumeBody) -> Result<niles_webos::VolumeChange, Failure> {
    use niles_webos::VolumeChange;
    let bad = |reason: &str| (StatusCode::BAD_REQUEST, reason.to_string());
    match (body.volume, body.delta, body.mute) {
        (Some(level), None, None) if level <= 100 => Ok(VolumeChange::To(level)),
        (Some(_), None, None) => Err(bad("volume is 0 to 100")),
        (None, Some(delta), None) => Ok(VolumeChange::By(i16::from(delta))),
        (None, None, Some(mute)) => Ok(VolumeChange::Mute(mute)),
        _ => Err(bad("give one of volume, delta or mute")),
    }
}

/// `POST /tv/volume` — `{"volume": 20}`, `{"delta": -2}` or
/// `{"mute": true}` on the TV's own speakers. Answers the volume after.
pub async fn volume(
    State(state): State<AppState>,
    Json(body): Json<VolumeBody>,
) -> Result<Json<niles_webos::Volume>, Failure> {
    let change = change_for(&body)?;
    let cfg = state
        .config
        .as_ref()
        .map(|c| c.current())
        .ok_or((StatusCode::NOT_IMPLEMENTED, "no config store".to_string()))?;
    let tv = &cfg.tv;
    if !tv.is_configured() {
        return Err((StatusCode::NOT_FOUND, "there is no TV set up".to_string()));
    }
    let key = tv
        .resolve_client_key()
        .map_err(|_| (StatusCode::CONFLICT, "the TV is not paired".to_string()))?;
    match niles_webos::Tv::new(tv.host.trim(), key)
        .change_volume(change)
        .await
    {
        Ok(Some(after)) => Ok(Json(after)),
        Ok(None) => Err((StatusCode::CONFLICT, "the TV is off".to_string())),
        Err(e) => Err((StatusCode::BAD_GATEWAY, e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::publish::DevicePublisher;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use niles_config::ConfigStore;
    use niles_core::{DeviceRegistry, EventBus};
    use niles_webos::VolumeChange;
    use serde_json::json;
    use std::sync::Arc;
    use tower::ServiceExt;

    struct NoopPublisher;

    #[async_trait::async_trait]
    impl DevicePublisher for NoopPublisher {
        async fn publish(&self, _topic: String, _payload: Vec<u8>) -> Result<(), String> {
            Ok(())
        }
    }

    /// The app with `tv` appended to a valid config, or with no config
    /// store at all.
    fn app(tv: Option<&str>) -> axum::Router {
        let store = tv.map(|tv| {
            let toml = format!("{}\n{tv}", crate::config_tests::base_toml());
            Arc::new(ConfigStore::from_str_in_memory(&toml).expect("fixture is valid"))
        });
        let state = AppState::new(
            Arc::new(DeviceRegistry::new()),
            Arc::new(NoopPublisher) as Arc<dyn DevicePublisher>,
            Arc::new(niles_mqtt::CommandRouter::z2m_only("zigbee2mqtt")),
            EventBus::default(),
        )
        .with_config_store(store);
        crate::server::router(state)
    }

    async fn post_volume(app: axum::Router, body: serde_json::Value) -> (StatusCode, String) {
        let request = Request::builder()
            .method("POST")
            .uri("/tv/volume")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    fn body(volume: Option<u8>, delta: Option<i8>, mute: Option<bool>) -> VolumeBody {
        VolumeBody {
            volume,
            delta,
            mute,
        }
    }

    #[test]
    fn a_body_asks_for_one_change() {
        assert_eq!(
            change_for(&body(Some(20), None, None)).unwrap(),
            VolumeChange::To(20)
        );
        assert_eq!(
            change_for(&body(None, Some(-2), None)).unwrap(),
            VolumeChange::By(-2)
        );
        assert_eq!(
            change_for(&body(None, None, Some(true))).unwrap(),
            VolumeChange::Mute(true)
        );
        assert_eq!(
            change_for(&body(Some(101), None, None)).unwrap_err().0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            change_for(&body(None, None, None)).unwrap_err().0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            change_for(&body(Some(20), None, Some(true))).unwrap_err().0,
            StatusCode::BAD_REQUEST
        );
    }

    #[tokio::test]
    async fn the_volume_needs_a_tv_set_up_and_paired() {
        let (status, _) = post_volume(app(None), json!({ "volume": 20 })).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        let (status, _) = post_volume(app(Some("")), json!({ "volume": 20 })).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let unpaired = "[tv]\nhost = \"127.0.0.1\"\nclient_key_env = \"NILES_TEST_API_TV_UNSET\"\n";
        let (status, said) = post_volume(app(Some(unpaired)), json!({ "mute": true })).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(said, "the TV is not paired");
    }

    #[tokio::test]
    async fn a_tv_that_is_off_says_so() {
        // Nothing listens on loopback for it, which is how a TV that is
        // off looks.
        unsafe { std::env::set_var("NILES_TEST_API_TV_KEY", "key") };
        let paired = "[tv]\nhost = \"127.0.0.1\"\nclient_key_env = \"NILES_TEST_API_TV_KEY\"\n";
        let (status, said) = post_volume(app(Some(paired)), json!({ "delta": 2 })).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(said, "the TV is off");
    }

    #[tokio::test]
    async fn a_level_out_of_range_is_refused_before_the_tv_is_asked() {
        let (status, _) = post_volume(app(None), json!({ "volume": 150 })).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
}
