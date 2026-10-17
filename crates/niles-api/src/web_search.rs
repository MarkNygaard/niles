//! The web search add-on's test: a real web query and a real image query
//! through the SearXNG the config names, saying how many results each
//! gave.
//!
//! Asked of the config as it is saved now, not of a client made at
//! startup, so it tests the address somebody has just typed in. Both go,
//! because they fail apart: many instances have no image engine on, and
//! answer a search for pictures with nothing at all.
//!
//! What goes back never carries the address, or the instance's own
//! words. An address may have a password in it, and the HTTP client's
//! errors name the URL they failed on, so a failure is described by its
//! kind instead.

use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use niles_websearch::{Error, SearXngClient, SearXngConfig, SearchRequest};
use serde::Serialize;
use std::time::Duration;

/// Asked for something every web engine has results for.
const WEB_QUERY: &str = "weather forecast";

/// Asked for something every image engine has pictures of.
const IMAGE_QUERY: &str = "lighthouse";

/// However long the config lets a search take, the page waits no longer
/// than this for its answer.
const AT_MOST: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, Serialize)]
pub struct WebSearchTestDto {
    /// Whether there is an address to test.
    pub configured: bool,
    /// `None` when there was nothing to ask.
    pub web: Option<Outcome>,
    pub images: Option<Outcome>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Outcome {
    pub ok: bool,
    /// How many came back, at most 20.
    pub results: usize,
    /// Why it did not answer, in words for the page.
    pub error: Option<String>,
}

/// `POST /integrations/web_search/test`
pub async fn test(State(state): State<AppState>) -> Json<WebSearchTestDto> {
    let Some(cfg) = state.config.as_ref().map(|c| c.current()) else {
        return Json(nothing());
    };
    let Some(address) = cfg.web_search.address() else {
        return Json(nothing());
    };
    let within = Duration::from_secs(cfg.web_search.timeout_seconds).min(AT_MOST);
    let client = match SearXngClient::new(SearXngConfig {
        base_url: address.to_string(),
        request_timeout: within,
        user_agent: niles_websearch::USER_AGENT.into(),
    }) {
        Ok(client) => client,
        Err(e) => {
            let failed = failed(&e, within);
            return Json(WebSearchTestDto {
                configured: true,
                web: Some(failed.clone()),
                images: Some(failed),
            });
        }
    };
    let request = SearchRequest {
        query: WEB_QUERY.into(),
        num_results: 20,
    };
    let (web, images) = tokio::join!(
        client.search(&request),
        client.search_images(IMAGE_QUERY, 20),
    );
    Json(WebSearchTestDto {
        configured: true,
        web: Some(match web {
            Ok(found) => found_(found.results.len()),
            Err(e) => failed(&e, within),
        }),
        images: Some(match images {
            Ok(found) => found_(found.len()),
            Err(e) => failed(&e, within),
        }),
    })
}

fn nothing() -> WebSearchTestDto {
    WebSearchTestDto {
        configured: false,
        web: None,
        images: None,
    }
}

fn found_(results: usize) -> Outcome {
    Outcome {
        ok: true,
        results,
        error: None,
    }
}

fn failed(e: &Error, within: Duration) -> Outcome {
    Outcome {
        ok: false,
        results: 0,
        error: Some(plainly(e, within)),
    }
}

/// What went wrong, without the address or anything the instance said.
fn plainly(e: &Error, within: Duration) -> String {
    match e {
        Error::Http { error } if error.is_timeout() => {
            format!("SearXNG did not answer within {}s.", within.as_secs())
        }
        Error::Http { error } if error.is_connect() => {
            "Nothing answered at that address. Check it, and that Niles can reach it.".into()
        }
        Error::Http { .. } => "The request to SearXNG failed before it answered.".into(),
        // What SearXNG answers when the JSON format is not switched on,
        // which it is not out of the box.
        Error::BadStatus { status: 403, .. } => "SearXNG refused (HTTP 403). Switch on the \
            JSON format in its settings: search → formats, add json."
            .into(),
        Error::BadStatus { status, .. } => format!("SearXNG answered with HTTP {status}."),
        Error::Parse { .. } => "What came back is not SearXNG's JSON. Is this the address of \
            its search, usually ending in /search?"
            .into(),
        Error::InvalidInput { .. } => "That does not read as a web address.".into(),
        _ => "SearXNG could not be asked.".into(),
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
    use std::time::Duration;
    use tower::ServiceExt;
    use wiremock::matchers::{method, path, query_param};
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

    fn at(base_url: &str) -> String {
        format!("[web_search]\nbase_url = \"{base_url}\"\ntimeout_seconds = 1\n")
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

    fn request(verb: &str, uri: &str, body: Option<Value>) -> Request<Body> {
        let builder = Request::builder().method(verb).uri(uri);
        match body {
            Some(body) => builder
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
            None => builder.body(Body::empty()).unwrap(),
        }
    }

    async fn test(app: &axum::Router) -> Value {
        let (status, body) =
            send(app, request("POST", "/integrations/web_search/test", None)).await;
        assert_eq!(status, StatusCode::OK);
        body
    }

    fn results(n: usize, image: bool) -> Value {
        let results: Vec<Value> = (0..n)
            .map(|i| {
                let mut r = json!({
                    "title": format!("Result {i}"),
                    "url": format!("https://example.org/{i}"),
                    "content": "c",
                });
                if image {
                    r["img_src"] = json!(format!("https://example.org/{i}.jpg"));
                }
                r
            })
            .collect();
        json!({ "results": results })
    }

    async fn searxng(web: usize, images: usize) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("categories", "images"))
            .respond_with(ResponseTemplate::new(200).set_body_json(results(images, true)))
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(results(web, false)))
            .with_priority(2)
            .mount(&server)
            .await;
        server
    }

    #[tokio::test]
    async fn a_working_instance_says_how_many_of_each_it_found() {
        let server = searxng(7, 4).await;
        let body = test(&app(&at(&format!("{}/search", server.uri())))).await;
        assert_eq!(body["configured"], true);
        assert_eq!(
            body["web"],
            json!({ "ok": true, "results": 7, "error": null })
        );
        assert_eq!(
            body["images"],
            json!({ "ok": true, "results": 4, "error": null })
        );
    }

    #[tokio::test]
    async fn an_instance_with_no_image_engines_finds_pages_and_no_pictures() {
        let server = searxng(5, 0).await;
        let body = test(&app(&at(&format!("{}/search", server.uri())))).await;
        assert_eq!(body["web"]["results"], 5);
        assert_eq!(
            body["images"],
            json!({ "ok": true, "results": 0, "error": null })
        );
    }

    #[tokio::test]
    async fn nothing_to_test_without_an_address() {
        let body = test(&app("")).await;
        assert_eq!(
            body,
            json!({ "configured": false, "web": null, "images": null })
        );
        let body = test(&app("[web_search]\nbase_url = \"\"\n")).await;
        assert_eq!(body["configured"], false);
    }

    #[tokio::test]
    async fn an_unreachable_instance_is_said_without_its_address() {
        // A port nothing listens on, and a password in the address.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let address = format!("http://niles:hunter2@127.0.0.1:{port}/search");
        // Long enough for Windows, which tries a refused port again for
        // a couple of seconds before it says so.
        let body = test(&app(&format!(
            "[web_search]\nbase_url = \"{address}\"\ntimeout_seconds = 10\n"
        )))
        .await;
        assert_eq!(body["configured"], true);
        for which in ["web", "images"] {
            assert_eq!(body[which]["ok"], false, "{body}");
            let error = body[which]["error"].as_str().unwrap();
            assert!(error.contains("Nothing answered"), "{error}");
        }
        let text = body.to_string();
        assert!(!text.contains("hunter2"), "{text}");
        assert!(!text.contains(&port.to_string()), "{text}");
    }

    #[tokio::test]
    async fn a_slow_instance_times_out_within_the_configured_time() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(results(3, false))
                    .set_delay(Duration::from_secs(5)),
            )
            .mount(&server)
            .await;
        let started = std::time::Instant::now();
        let body = test(&app(&at(&format!("{}/search", server.uri())))).await;
        assert!(started.elapsed() < Duration::from_secs(4), "it waited");
        assert_eq!(body["web"]["ok"], false);
        assert_eq!(body["web"]["error"], "SearXNG did not answer within 1s.");
    }

    #[tokio::test]
    async fn json_switched_off_is_named_and_the_instance_s_words_are_not_echoed() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(403).set_body_string("secret page body"))
            .mount(&server)
            .await;
        let body = test(&app(&at(&format!("{}/search", server.uri())))).await;
        let error = body["web"]["error"].as_str().unwrap();
        assert!(error.contains("JSON format"), "{error}");
        assert!(!body.to_string().contains("secret page body"), "{body}");
    }

    #[tokio::test]
    async fn the_address_is_written_read_back_and_removed_through_the_config() {
        let server = searxng(2, 1).await;
        let app = app("");
        let added = |body: &Value| {
            body.as_array()
                .unwrap()
                .iter()
                .find(|i| i["id"] == "web_search")
                .unwrap()["added"]
                .clone()
        };
        let (_, before) = send(&app, request("GET", "/integrations", None)).await;
        assert_eq!(added(&before), false);

        let address = format!("{}/search", server.uri());
        let (status, _) = send(
            &app,
            request(
                "PATCH",
                "/config",
                Some(json!({ "web_search": { "base_url": address } })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (_, config) = send(&app, request("GET", "/config", None)).await;
        assert_eq!(config["effective"]["web_search"]["base_url"], address);
        let section = config["sections"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["name"] == "web_search")
            .unwrap()
            .clone();
        assert_eq!(section["reload"], "hot", "no restart: {section}");
        let (_, after) = send(&app, request("GET", "/integrations", None)).await;
        assert_eq!(added(&after), true);
        assert_eq!(test(&app).await["web"]["results"], 2);

        let (status, _) = send(
            &app,
            request(
                "PATCH",
                "/config",
                Some(json!({ "web_search": { "base_url": "" } })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (_, removed) = send(&app, request("GET", "/integrations", None)).await;
        assert_eq!(added(&removed), false);
        assert_eq!(test(&app).await["configured"], false);
    }
}
