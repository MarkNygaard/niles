//! The TV, for the app: pairing it, its state, its power button and its
//! volume.

use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use niles_config::{ChangeSource, TvConfig};
use serde::{Deserialize, Serialize};
use std::time::Duration;

type Failure = (StatusCode, String);

#[derive(Debug, Serialize)]
pub struct TvDto {
    /// An address to reach it at.
    pub configured: bool,
    /// Paired: Niles has the key the TV issued.
    pub paired: bool,
    /// The address it will be woken with first, once known.
    pub mac: Option<String>,
    /// Where `mac` came from: "detected" at pairing, or "hand": typed,
    /// and kept through a pairing again.
    pub mac_source: Option<&'static str>,
    /// The MAC pairing chose, when it is not `mac`.
    pub detected_mac: Option<String>,
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
            mac_source: None,
            detected_mac: None,
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
    // A cleared `mac` is woken by what pairing found.
    let mac = present(&tv.mac).or_else(|| tv.detected_mac().map(str::to_string));
    let by_hand = tv.mac_set_by_hand();
    Json(TvDto {
        configured: tv.is_configured(),
        paired: key.is_some(),
        mac_source: mac
            .as_ref()
            .map(|_| if by_hand { "hand" } else { "detected" }),
        detected_mac: tv.detected_mac().filter(|_| by_hand).map(str::to_string),
        mac,
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
/// addresses are read while connected and written to `[tv]`, so waking
/// it needs nothing typed, and a MAC somebody did type stays. Answers
/// when the prompt is accepted, or after a minute of nobody accepting it.
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

    let macs = match niles_webos::Tv::new(&host, &key).macs().await {
        Ok(macs) => macs,
        Err(e) => {
            tracing::warn!("[tv] paired, but its MAC address could not be read: {e}");
            None
        }
    };
    if let Some(macs) = &macs {
        store
            .apply(&pairing_patch(&store.current().tv, macs), ChangeSource::Api)
            .await
            .map_err(|e| {
                (
                    StatusCode::BAD_GATEWAY,
                    format!("could not keep the MAC: {e}"),
                )
            })?;
    }
    let mac = Some(store.current().tv.mac.trim().to_string()).filter(|m| !m.is_empty());
    Ok(Json(Paired { mac }))
}

/// What a pairing writes to `[tv]`: every MAC the TV reported, and the
/// chosen one as `mac` unless the `mac` there was set by hand.
fn pairing_patch(tv: &TvConfig, macs: &niles_webos::Macs) -> toml::Table {
    let mut section = toml::Table::new();
    let all = macs.all.iter().cloned().map(toml::Value::String).collect();
    section.insert("detected_macs".into(), toml::Value::Array(all));
    if tv.mac_set_by_hand() {
        tracing::info!(
            "[tv] keeping the wake MAC {} that was set by hand; the TV is connected by {}",
            tv.mac.trim(),
            macs.chosen
        );
    } else {
        section.insert("mac".into(), toml::Value::String(macs.chosen.clone()));
    }
    let mut patch = toml::Table::new();
    patch.insert("tv".into(), toml::Value::Table(section));
    patch
}

#[derive(Debug, Deserialize)]
pub struct Power {
    pub on: bool,
}

/// What came of a press of the power button.
#[derive(Debug, Serialize)]
pub struct PowerDto {
    /// Whether the TV answered as on: after a wake-up, only once it has.
    pub on: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Said when the wake-up went out and the TV did not answer in time.
pub const NO_ANSWER: &str = "Sent the wake signal, but the TV hasn't answered: check the wake MAC under Settings → Add-ons → LG TV.";

/// `POST /tv/power` — `{"on": true}` wakes it, `{"on": false}` turns it
/// off. Waking answers 200 once the TV says it is on, and 202 with a
/// message when the wake-up went out and the TV did not answer in time.
pub async fn power(
    State(state): State<AppState>,
    Json(body): Json<Power>,
) -> Result<(StatusCode, Json<PowerDto>), Failure> {
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
        let macs = tv.wake_macs();
        if macs.is_empty() {
            return Err((
                StatusCode::CONFLICT,
                "the TV's MAC address is not known; pair it again".to_string(),
            ));
        }
        let paired = tv
            .resolve_client_key()
            .ok()
            .map(|key| niles_webos::Tv::new(tv.host.trim(), key));
        return turn_on(tv.host.trim(), &macs, paired, niles_webos::WAKING).await;
    }
    let key = tv
        .resolve_client_key()
        .map_err(|_| (StatusCode::CONFLICT, "the TV is not paired".to_string()))?;
    niles_webos::Tv::new(tv.host.trim(), key)
        .turn_off()
        .await
        .map_err(failed)?;
    Ok((
        StatusCode::OK,
        Json(PowerDto {
            on: false,
            message: None,
        }),
    ))
}

/// Send the wake-up, then wait up to `within` for the TV to say it is
/// on. Without a pairing there is nobody to ask.
async fn turn_on(
    host: &str,
    macs: &[String],
    tv: Option<niles_webos::Tv>,
    within: Duration,
) -> Result<(StatusCode, Json<PowerDto>), Failure> {
    niles_webos::wake(host, macs)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    let unconfirmed = |message: &str| {
        Ok((
            StatusCode::ACCEPTED,
            Json(PowerDto {
                on: false,
                message: Some(message.to_string()),
            }),
        ))
    };
    let Some(tv) = tv else {
        return unconfirmed("Sent the wake signal. Pair the TV for Niles to see it come on.");
    };
    if tv.came_on(within).await {
        return Ok((
            StatusCode::OK,
            Json(PowerDto {
                on: true,
                message: None,
            }),
        ));
    }
    tracing::warn!(
        "[tv] sent the wake signal for {}, but the TV did not answer within {}s",
        macs.join(", "),
        within.as_secs()
    );
    unconfirmed(NO_ANSWER)
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
        // SAFETY: a test-only variable name nothing else reads.
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

    const WIRED: &str = "64:e4:a5:6c:ab:b4";
    const WIFI: &str = "84:90:0a:63:a9:94";

    fn on_wifi() -> niles_webos::Macs {
        niles_webos::Macs {
            chosen: WIFI.into(),
            all: vec![WIFI.into(), WIRED.into()],
        }
    }

    fn store(tv: &str) -> ConfigStore {
        let toml = format!(
            "{}\n[tv]\nhost = \"192.168.4.170\"\n{tv}",
            crate::config_tests::base_toml()
        );
        ConfigStore::from_str_in_memory(&toml).expect("fixture is valid")
    }

    async fn paired_again(store: &ConfigStore, macs: &niles_webos::Macs) {
        store
            .apply(&pairing_patch(&store.current().tv, macs), ChangeSource::Api)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn pairing_keeps_the_chosen_mac_and_waking_uses_it() {
        let store = store("");
        paired_again(&store, &on_wifi()).await;
        let tv = &store.current().tv;
        assert_eq!(tv.mac, WIFI);
        assert_eq!(tv.detected_macs, [WIFI, WIRED]);
        assert!(!tv.mac_set_by_hand());
        assert_eq!(tv.wake_macs(), [WIFI, WIRED]);
    }

    #[tokio::test]
    async fn pairing_again_replaces_a_mac_it_detected() {
        // Detected as wired before; the TV has since moved to Wi-Fi.
        let store = store(&format!(
            "mac = \"{WIRED}\"\ndetected_macs = [\"{WIRED}\", \"{WIFI}\"]"
        ));
        paired_again(&store, &on_wifi()).await;
        assert_eq!(store.current().tv.mac, WIFI);
    }

    #[tokio::test]
    async fn pairing_again_keeps_a_mac_set_by_hand() {
        let store = store(&format!(
            "mac = \"A8:23:FE:01:02:03\"\ndetected_macs = [\"{WIRED}\"]"
        ));
        paired_again(&store, &on_wifi()).await;
        let tv = &store.current().tv;
        assert_eq!(tv.mac, "A8:23:FE:01:02:03");
        assert_eq!(tv.detected_macs, [WIFI, WIRED]);
        assert_eq!(tv.wake_macs(), ["A8:23:FE:01:02:03", WIFI, WIRED]);
    }

    async fn power_on(
        tv: Option<niles_webos::Tv>,
        within: Duration,
    ) -> (StatusCode, serde_json::Value) {
        let (status, Json(body)) = turn_on("127.0.0.1", &[WIFI.to_string()], tv, within)
            .await
            .unwrap();
        (status, serde_json::to_value(body).unwrap())
    }

    #[tokio::test]
    async fn a_tv_that_comes_on_is_said_to_be_on() {
        use niles_webos::fake::{WebOs, fake_tv};
        let fake = fake_tv(WebOs::Newer("Active"), 0, "").await;
        let (status, body) = power_on(Some(fake.tv()), Duration::from_secs(5)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!({ "on": true }));
    }

    #[tokio::test]
    async fn a_tv_that_does_not_answer_in_time_says_where_to_look() {
        use niles_webos::fake::{WebOs, fake_tv};
        let fake = fake_tv(WebOs::Newer("Active Standby"), 0, "").await;
        let (status, body) = power_on(Some(fake.tv()), Duration::from_millis(1500)).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(body, json!({ "on": false, "message": NO_ANSWER }));
    }

    #[tokio::test]
    async fn an_unpaired_tv_is_woken_without_waiting_on_it() {
        let (status, body) = power_on(None, Duration::from_secs(30)).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert!(body["message"].as_str().unwrap().contains("Pair the TV"));
    }

    async fn get_tv(tv: &str) -> serde_json::Value {
        let request = Request::builder().uri("/tv").body(Body::empty()).unwrap();
        let response = app(Some(tv)).oneshot(request).await.unwrap();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn the_tv_says_whether_its_mac_was_detected_or_set_by_hand() {
        let unpaired = "[tv]\nhost = \"127.0.0.1\"\nclient_key_env = \"NILES_TEST_API_TV_UNSET\"\n";
        let detected = get_tv(&format!(
            "{unpaired}mac = \"{WIFI}\"\ndetected_macs = [\"{WIFI}\", \"{WIRED}\"]\n"
        ))
        .await;
        assert_eq!(detected["mac"], WIFI);
        assert_eq!(detected["mac_source"], "detected");
        assert_eq!(detected["detected_mac"], serde_json::Value::Null);
        let by_hand = get_tv(&format!(
            "{unpaired}mac = \"{WIRED}\"\ndetected_macs = [\"{WIFI}\", \"{WIRED}\"]\n"
        ))
        .await;
        assert_eq!(by_hand["mac_source"], "hand");
        assert_eq!(by_hand["detected_mac"], WIFI);
        let cleared = get_tv(&format!(
            "{unpaired}mac = \"\"\ndetected_macs = [\"{WIFI}\", \"{WIRED}\"]\n"
        ))
        .await;
        assert_eq!(cleared["mac"], WIFI);
        assert_eq!(cleared["mac_source"], "detected");
        let none = get_tv(unpaired).await;
        assert_eq!(none["mac"], serde_json::Value::Null);
        assert_eq!(none["mac_source"], serde_json::Value::Null);
    }

    #[tokio::test]
    async fn waking_needs_a_mac() {
        let request = Request::builder()
            .method("POST")
            .uri("/tv/power")
            .header("content-type", "application/json")
            .body(Body::from(json!({ "on": true }).to_string()))
            .unwrap();
        let response = app(Some("[tv]\nhost = \"127.0.0.1\"\n"))
            .oneshot(request)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
    }
}
