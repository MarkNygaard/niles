//! The Podcasts add-on's two endpoints: the connection test, asked of
//! the config as it is saved now, and Kokoro's voice list for the
//! pickers. Neither ever echoes an address or a key: what broke is
//! described by its kind, as the web search test does.

use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use chrono::Utc;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct PodcastsTestDto {
    /// Whether the add-on is there and complete enough to test.
    pub configured: bool,
    /// What is still blank, when it is not.
    pub missing: Vec<&'static str>,
    /// Each part's result, once there is something to test.
    pub kokoro: Option<niles_podcasts::Part>,
    pub storage: Option<niles_podcasts::Part>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VoicesDto {
    pub voices: Vec<String>,
    pub error: Option<String>,
}

/// `POST /integrations/podcasts/test`
pub async fn test(State(state): State<AppState>) -> Json<PodcastsTestDto> {
    let Some(cfg) = state.config.as_ref().map(|c| c.current()) else {
        return Json(PodcastsTestDto {
            configured: false,
            missing: vec!["the add-on"],
            kokoro: None,
            storage: None,
        });
    };
    let Some(podcasts) = cfg.integrations.podcasts.clone() else {
        return Json(PodcastsTestDto {
            configured: false,
            missing: vec!["the add-on"],
            kokoro: None,
            storage: None,
        });
    };
    let missing = niles_podcasts::missing(&podcasts);
    if !missing.is_empty() {
        return Json(PodcastsTestDto {
            configured: false,
            missing,
            kokoro: None,
            storage: None,
        });
    }
    let kokoro = match niles_podcasts::Kokoro::from_config(&podcasts) {
        Ok(kokoro) => kokoro,
        Err(e) => {
            let failed = niles_podcasts::Part {
                ok: false,
                error: Some(e.to_string()),
            };
            return Json(PodcastsTestDto {
                configured: true,
                missing: Vec::new(),
                kokoro: Some(failed.clone()),
                storage: Some(failed),
            });
        }
    };
    let bucket = match niles_podcasts::Bucket::from_config(&podcasts) {
        Ok(bucket) => bucket,
        Err(e) => {
            let failed = niles_podcasts::Part {
                ok: false,
                error: Some(e.to_string()),
            };
            return Json(PodcastsTestDto {
                configured: true,
                missing: Vec::new(),
                kokoro: Some(failed.clone()),
                storage: Some(failed),
            });
        }
    };
    let check = niles_podcasts::check(&kokoro, &podcasts.voices(), &bucket, Utc::now()).await;
    Json(PodcastsTestDto {
        configured: true,
        missing: Vec::new(),
        kokoro: Some(check.kokoro),
        storage: Some(check.storage),
    })
}

/// `GET /integrations/podcasts/voices`
pub async fn voices(State(state): State<AppState>) -> Json<VoicesDto> {
    let Some(podcasts) = state
        .config
        .as_ref()
        .map(|c| c.current())
        .and_then(|cfg| cfg.integrations.podcasts.clone())
    else {
        return Json(VoicesDto {
            voices: Vec::new(),
            error: Some("Add Kokoro's address first.".into()),
        });
    };
    let Some(address) = podcasts.kokoro_address() else {
        return Json(VoicesDto {
            voices: Vec::new(),
            error: Some("Add Kokoro's address first.".into()),
        });
    };
    match niles_podcasts::Kokoro::new(&address) {
        Ok(kokoro) => match kokoro.voices().await {
            Ok(voices) => Json(VoicesDto {
                voices,
                error: None,
            }),
            Err(e) => Json(VoicesDto {
                voices: Vec::new(),
                error: Some(e.to_string()),
            }),
        },
        Err(e) => Json(VoicesDto {
            voices: Vec::new(),
            error: Some(e.to_string()),
        }),
    }
}

#[cfg(test)]
mod tests {
    use crate::publish::DevicePublisher;
    use crate::server::router;
    use crate::state::AppState;
    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use niles_config::ConfigStore;
    use niles_core::{DeviceRegistry, EventBus};
    use serde_json::{Value, json};
    use std::sync::Arc;
    use tower::ServiceExt;
    use wiremock::matchers::{method, path, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    struct NoopPublisher;

    #[async_trait]
    impl DevicePublisher for NoopPublisher {
        async fn publish(&self, _topic: String, _payload: Vec<u8>) -> Result<(), String> {
            Ok(())
        }
    }

    fn app(extra: &str) -> axum::Router {
        let toml = format!("{}\n{extra}", crate::config_tests::base_toml());
        let store = Arc::new(ConfigStore::from_str_in_memory(&toml).expect("fixture is valid"));
        router(
            AppState::new(
                Arc::new(DeviceRegistry::new()),
                Arc::new(NoopPublisher) as Arc<dyn DevicePublisher>,
                Arc::new(niles_mqtt::CommandRouter::z2m_only("zigbee2mqtt")),
                EventBus::default(),
            )
            .with_config_store(Some(store)),
        )
    }

    /// One wiremock for both halves: Kokoro's `/v1/audio/*` and the
    /// bucket's `/b/…`, as the two addresses point at it.
    async fn house(kokoro: StatusLine, storage_put: StatusLine) -> (MockServer, String) {
        let server = MockServer::start().await;
        let body = kokoro_answer();
        match kokoro {
            StatusLine::Ok => {
                Mock::given(method("POST"))
                    .and(path("/v1/audio/speech"))
                    .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
                    .mount(&server)
                    .await;
            }
            StatusLine::Fail(status) => {
                Mock::given(method("POST"))
                    .and(path("/v1/audio/speech"))
                    .respond_with(ResponseTemplate::new(status.as_u16()))
                    .mount(&server)
                    .await;
            }
        }
        match storage_put {
            StatusLine::Ok => {
                Mock::given(method("PUT"))
                    .and(path_regex(r"^/episodes/podcasts/niles-test-\d+\.txt$"))
                    .respond_with(ResponseTemplate::new(200))
                    .expect(1)
                    .mount(&server)
                    .await;
                // The read-back and the delete only happen when the
                // upload went through.
                Mock::given(method("GET"))
                    .and(path_regex(r"^/episodes/podcasts/niles-test-\d+\.txt$"))
                    .respond_with(ResponseTemplate::new(200).set_body_bytes(b"niles"))
                    .expect(1)
                    .mount(&server)
                    .await;
                Mock::given(method("DELETE"))
                    .and(path_regex(r"^/episodes/podcasts/niles-test-\d+\.txt$"))
                    .respond_with(ResponseTemplate::new(204))
                    .expect(1)
                    .mount(&server)
                    .await;
            }
            StatusLine::Fail(status) => {
                Mock::given(method("PUT"))
                    .and(path_regex(r"^/episodes/podcasts/niles-test-\d+\.txt$"))
                    .respond_with(
                        ResponseTemplate::new(status.as_u16())
                            .set_body_string("<Error><Code>AccessDenied</Code></Error>"),
                    )
                    .expect(1)
                    .mount(&server)
                    .await;
            }
        }
        let config = format!(
            "[integrations.podcasts]\n\
             kokoro_url = \"{0}\"\n\
             endpoint = \"{0}\"\n\
             bucket = \"episodes\"\n\
             region = \"auto\"\n\
             access_key_id_env = \"NILES_TEST_PODCASTS_API_AKID\"\n\
             secret_access_key_env = \"NILES_TEST_PODCASTS_API_SECRET\"\n\
             keep_days = 14\n",
            server.uri()
        );
        (server, config)
    }

    enum StatusLine {
        Ok,
        Fail(StatusCode),
    }

    /// One whole silent frame, so the Kokoro part's `mp3::frames`
    /// accepts the sample.
    fn kokoro_answer() -> Vec<u8> {
        [0xFF, 0xF3, 0x84, 0xC0]
            .into_iter()
            .chain(std::iter::repeat_n(0u8, 188))
            .collect()
    }

    fn test_credentials() {
        // SAFETY: test-only variable names, unique to these tests; no
        // other test reads them.
        unsafe {
            std::env::set_var("NILES_TEST_PODCASTS_API_AKID", "AKIDEXAMPLE");
            std::env::set_var(
                "NILES_TEST_PODCASTS_API_SECRET",
                "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            );
        }
    }

    async fn send(app: &axum::Router, req: Request<Body>) -> (StatusCode, Value) {
        let response = app.clone().oneshot(req).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn tested(app: &axum::Router) -> Value {
        let (status, body) = send(
            app,
            Request::builder()
                .method("POST")
                .uri("/integrations/podcasts/test")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        body
    }

    #[tokio::test]
    async fn nothing_set_up_says_not_configured() {
        let body = tested(&app("")).await;
        assert_eq!(
            body,
            json!({
                "configured": false,
                "missing": ["the add-on"],
                "kokoro": null,
                "storage": null,
            })
        );
    }

    #[tokio::test]
    async fn a_missing_bucket_and_keys_are_listed() {
        test_credentials();
        let body = tested(&app(
            "[integrations.podcasts]\nkokoro_url = \"http://kokoro:8880\"\n",
        ))
        .await;
        assert_eq!(body["configured"], false);
        assert_eq!(
            body["missing"],
            json!([
                "R2 endpoint",
                "bucket name",
                "access key ID",
                "secret access key"
            ])
        );
    }

    #[tokio::test]
    async fn both_parts_pass_against_a_quiet_house() {
        test_credentials();
        let (_server, config) = house(StatusLine::Ok, StatusLine::Ok).await;
        let body = tested(&app(&config)).await;
        assert_eq!(body["configured"], true);
        assert_eq!(body["kokoro"], json!({ "ok": true, "error": null }));
        assert_eq!(body["storage"], json!({ "ok": true, "error": null }));
    }

    #[tokio::test]
    async fn kokoro_failing_alone_is_reported_on_its_own_part() {
        test_credentials();
        let (_server, config) = house(
            StatusLine::Fail(StatusCode::INTERNAL_SERVER_ERROR),
            StatusLine::Ok,
        )
        .await;
        let body = tested(&app(&config)).await;
        assert_eq!(body["kokoro"]["ok"], false);
        assert!(
            body["kokoro"]["error"]
                .as_str()
                .expect("an error")
                .contains("The first host's voice"),
            "{body}"
        );
        assert_eq!(body["storage"], json!({ "ok": true, "error": null }));
    }

    #[tokio::test]
    async fn storage_failing_alone_is_reported_on_its_own_part() {
        test_credentials();
        let (_server, config) =
            house(StatusLine::Ok, StatusLine::Fail(StatusCode::FORBIDDEN)).await;
        let body = tested(&app(&config)).await;
        assert_eq!(body["kokoro"]["ok"], true);
        assert_eq!(body["storage"]["ok"], false);
        assert!(
            body["storage"]["error"]
                .as_str()
                .expect("an error")
                .starts_with("Uploading a test file:"),
            "{body}"
        );
    }

    #[tokio::test]
    async fn no_answer_carries_the_address_or_the_keys() {
        test_credentials();
        let (server, config) = house(StatusLine::Ok, StatusLine::Fail(StatusCode::FORBIDDEN)).await;
        let body = tested(&app(&config)).await;
        let serialized = serde_json::to_string(&body).unwrap();
        assert!(!serialized.contains(&server.uri()), "{serialized}");
        assert!(!serialized.contains("AKIDEXAMPLE"), "{serialized}");
        assert!(!serialized.contains("wJalrXUtn"), "{serialized}");
    }

    #[tokio::test]
    async fn voices_come_back_sorted() {
        test_credentials();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/audio/voices"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "voices": ["am_michael", "af_heart", "bm_george"],
            })))
            .mount(&server)
            .await;
        let (status, body) = send(
            &app(&format!(
                "[integrations.podcasts]\nkokoro_url = \"{}\"\n",
                server.uri()
            )),
            Request::builder()
                .method("GET")
                .uri("/integrations/podcasts/voices")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body["voices"],
            json!(["af_heart", "am_michael", "bm_george"])
        );
        assert_eq!(body["error"], Value::Null);
    }

    #[tokio::test]
    async fn without_an_address_the_voice_list_says_so() {
        let (status, body) = send(
            &app(""),
            Request::builder()
                .method("GET")
                .uri("/integrations/podcasts/voices")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["voices"], json!([]));
        assert_eq!(body["error"], json!("Add Kokoro's address first."));
    }
}
