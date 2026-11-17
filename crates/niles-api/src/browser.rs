//! The live view of a person's browser (NIL-82): watch Niles's browser
//! as it works, take it over, give it back.
//!
//! Every route is the signed-in person's own session; nobody else's is
//! reachable. The live socket carries the page's pictures out and the
//! person's input in, and takes input only while the person has the
//! browser. Neither is logged, kept or shown to the model.

use crate::chat::{Failure, caller, failed};
use crate::state::AppState;
use async_trait::async_trait;
use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Who has the browser now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Control {
    Niles,
    Person,
}

/// The person's browsing session, as the app shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LiveView {
    /// The page's address.
    pub address: String,
    pub control: Control,
    /// The handoff the person has the browser for, when Niles asked.
    pub handoff: Option<crate::chat::Handoff>,
    /// The page's size in CSS pixels, which input is in.
    pub width: u32,
    pub height: u32,
}

/// Why something the person asked of their browser wasn't done.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// They have no browsing session open.
    NoSession,
    /// Niles has the browser: take it over first.
    NotInControl,
    /// Not an input Niles passes on.
    NotInput,
    /// The browser went away.
    Gone,
}

/// What giving the browser back leads to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Then {
    /// Niles carries on in the conversation, reading the page afresh.
    CarryOn,
    /// Niles has the browser again, and does nothing until asked.
    Stop,
}

/// Back or Reload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Go {
    Back,
    Reload,
}

/// The pictures of one live view, the newest each time.
#[async_trait]
pub trait Watching: Send {
    /// The next picture, a JPEG; `None` once the session or the view
    /// ended.
    async fn next(&mut self) -> Option<Arc<[u8]>>;
}

/// The person's browser. Implemented by the binary.
#[async_trait]
pub trait LiveBrowser: Send + Sync {
    async fn view(&self, who: &str, speaker: Option<&str>) -> Option<LiveView>;
    /// The person takes the browser over; Niles stops acting on it.
    async fn take(&self, who: &str, speaker: Option<&str>) -> Result<(), Refused>;
    /// The person gives it back, and with `Then::CarryOn` Niles carries
    /// on in `conversation`. Answers once the browser is Niles's again;
    /// the turn it starts runs on its own.
    async fn give_back(
        &self,
        who: &str,
        speaker: Option<&str>,
        conversation: Option<i64>,
        then: Then,
    ) -> Result<(), Refused>;
    async fn go(&self, who: &str, speaker: Option<&str>, go: Go) -> Result<(), Refused>;
    /// One input from the live view, as its JSON text.
    async fn input(&self, who: &str, speaker: Option<&str>, input: &str) -> Result<(), Refused>;
    /// The pictures, while the person has a session.
    async fn watch(&self, who: &str, speaker: Option<&str>) -> Option<Box<dyn Watching>>;
}

fn live(state: &AppState) -> Result<&Arc<dyn LiveBrowser>, Failure> {
    state.live_browser.as_ref().ok_or_else(|| {
        failed(
            StatusCode::NOT_IMPLEMENTED,
            "this Niles has no browser".into(),
        )
    })
}

pub(crate) fn refused(why: Refused) -> Failure {
    match why {
        Refused::NoSession => failed(
            StatusCode::NOT_FOUND,
            "nothing is open in your browser".into(),
        ),
        Refused::NotInControl => failed(
            StatusCode::CONFLICT,
            "Niles has the browser: take it over first".into(),
        ),
        Refused::NotInput => failed(
            StatusCode::UNPROCESSABLE_ENTITY,
            "not an input Niles passes on".into(),
        ),
        Refused::Gone => failed(
            StatusCode::SERVICE_UNAVAILABLE,
            "the browser went away".into(),
        ),
    }
}

/// `GET /me/browser` — the person's session, or null.
pub async fn view(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Option<LiveView>>, Failure> {
    let live = live(&state)?;
    let (who, speaker) = caller(&state, &headers);
    Ok(Json(live.view(&who, speaker.as_deref()).await))
}

/// `POST /me/browser/take`
pub async fn take(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, Failure> {
    let live = live(&state)?;
    let (who, speaker) = caller(&state, &headers);
    live.take(&who, speaker.as_deref())
        .await
        .map(|()| StatusCode::NO_CONTENT)
        .map_err(refused)
}

#[derive(Debug, Deserialize)]
pub struct GiveBack {
    #[serde(default)]
    pub conversation: Option<i64>,
    pub then: Then,
}

/// `POST /me/browser/give-back` — `{conversation, then}`.
pub async fn give_back(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<GiveBack>, JsonRejection>,
) -> Result<StatusCode, Failure> {
    let live = live(&state)?;
    let Ok(Json(body)) = body else {
        return Err(failed(
            StatusCode::BAD_REQUEST,
            "send {\"then\": \"carry_on\" | \"stop\", \"conversation\": id}".into(),
        ));
    };
    let (who, speaker) = caller(&state, &headers);
    live.give_back(&who, speaker.as_deref(), body.conversation, body.then)
        .await
        .map(|()| StatusCode::NO_CONTENT)
        .map_err(refused)
}

/// `POST /me/browser/back`
pub async fn back(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, Failure> {
    go(&state, &headers, Go::Back).await
}

/// `POST /me/browser/reload`
pub async fn reload(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, Failure> {
    go(&state, &headers, Go::Reload).await
}

async fn go(state: &AppState, headers: &HeaderMap, go: Go) -> Result<StatusCode, Failure> {
    let live = live(state)?;
    let (who, speaker) = caller(state, headers);
    live.go(&who, speaker.as_deref(), go)
        .await
        .map(|()| StatusCode::NO_CONTENT)
        .map_err(refused)
}

/// What the socket says back about an input it didn't pass on.
#[derive(Serialize)]
struct Said {
    refused: &'static str,
}

/// `GET /me/browser/live` — a WebSocket: binary JPEG frames out, input
/// as JSON text in. A page from anywhere else is refused, since this
/// socket can work the person's browser.
pub async fn live_socket(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Response {
    if !crate::events::same_origin(&headers) {
        return (
            StatusCode::FORBIDDEN,
            "this websocket is for pages served by Niles",
        )
            .into_response();
    }
    let live = match live(&state) {
        Ok(live) => live.clone(),
        Err(failure) => return failure.into_response(),
    };
    let (who, speaker) = caller(&state, &headers);
    let Some(watching) = live.watch(&who, speaker.as_deref()).await else {
        return refused(Refused::NoSession).into_response();
    };
    ws.on_upgrade(move |socket| run_socket(socket, live, watching, who, speaker))
}

async fn run_socket(
    mut socket: WebSocket,
    live: Arc<dyn LiveBrowser>,
    mut watching: Box<dyn Watching>,
    who: String,
    speaker: Option<String>,
) {
    loop {
        tokio::select! {
            frame = watching.next() => {
                let Some(frame) = frame else { break };
                if socket.send(Message::Binary(frame.to_vec().into())).await.is_err() {
                    break;
                }
            }
            message = socket.recv() => {
                let text = match message {
                    Some(Ok(Message::Text(text))) => text,
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    Some(Ok(_)) => continue,
                };
                // The input itself is never logged: it can be what the
                // person types into a page.
                let answer = live.input(&who, speaker.as_deref(), text.as_str()).await;
                let said = match answer {
                    Ok(()) => continue,
                    Err(Refused::NotInControl) => "not_in_control",
                    Err(Refused::NotInput) => "not_input",
                    Err(Refused::NoSession | Refused::Gone) => break,
                };
                let Ok(json) = serde_json::to_string(&Said { refused: said }) else { continue };
                if socket.send(Message::Text(json.into())).await.is_err() {
                    break;
                }
            }
        }
    }
    let _ = socket.send(Message::Close(None)).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::publish::DevicePublisher;
    use axum::body::Body;
    use axum::http::{Request, header};
    use http_body_util::BodyExt;
    use serde_json::{Value, json};
    use std::sync::Mutex;
    use tower::ServiceExt;

    const SECRET_VAR: &str = "NILES_TEST_BROWSER_SESSION_SECRET";
    const SECRET: &str = "the-live-browser-secret";
    const MAJSE: &str = "majse@example.com";
    const MARK: &str = "mark@example.com";

    struct NoopPublisher;

    #[async_trait]
    impl DevicePublisher for NoopPublisher {
        async fn publish(&self, _topic: String, _payload: Vec<u8>) -> Result<(), String> {
            Ok(())
        }
    }

    /// Majse has a session, which she has taken over; Mark has none.
    #[derive(Default)]
    struct Fake {
        given_back: Mutex<Vec<(String, Option<i64>, Then)>>,
    }

    fn hers(who: &str) -> Result<(), Refused> {
        if who == MAJSE {
            Ok(())
        } else {
            Err(Refused::NoSession)
        }
    }

    #[async_trait]
    impl LiveBrowser for Fake {
        async fn view(&self, who: &str, _speaker: Option<&str>) -> Option<LiveView> {
            hers(who).ok()?;
            Some(LiveView {
                address: "https://shop.example/basket".into(),
                control: Control::Person,
                handoff: None,
                width: 1280,
                height: 900,
            })
        }
        async fn take(&self, who: &str, _speaker: Option<&str>) -> Result<(), Refused> {
            hers(who)
        }
        async fn give_back(
            &self,
            who: &str,
            _speaker: Option<&str>,
            conversation: Option<i64>,
            then: Then,
        ) -> Result<(), Refused> {
            hers(who)?;
            self.given_back
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push((who.into(), conversation, then));
            Ok(())
        }
        async fn go(&self, who: &str, _speaker: Option<&str>, _go: Go) -> Result<(), Refused> {
            hers(who)?;
            Err(Refused::NotInControl)
        }
        async fn input(
            &self,
            who: &str,
            _speaker: Option<&str>,
            _input: &str,
        ) -> Result<(), Refused> {
            hers(who)
        }
        async fn watch(&self, _who: &str, _speaker: Option<&str>) -> Option<Box<dyn Watching>> {
            None
        }
    }

    fn app() -> (axum::Router, Arc<Fake>) {
        // SAFETY: test-only variable names nothing else reads.
        unsafe {
            std::env::set_var(SECRET_VAR, SECRET);
            std::env::set_var("NILES_TEST_BROWSER_CLIENT_ID", "id");
            std::env::set_var("NILES_TEST_BROWSER_CLIENT_SECRET", "secret");
        }
        let toml = format!(
            r#"{}
[auth]
github_client_id_env = "NILES_TEST_BROWSER_CLIENT_ID"
github_client_secret_env = "NILES_TEST_BROWSER_CLIENT_SECRET"
session_secret_env = "{SECRET_VAR}"
allowed = [
  {{ email = "{MARK}", speaker = "mark" }},
  {{ email = "{MAJSE}", speaker = "majse" }},
]
"#,
            crate::config_tests::base_toml()
        );
        let config = Arc::new(niles_config::ConfigStore::from_str_in_memory(&toml).unwrap());
        let fake = Arc::new(Fake::default());
        let state = AppState::new(
            Arc::new(niles_core::DeviceRegistry::new()),
            Arc::new(NoopPublisher) as Arc<dyn DevicePublisher>,
            Arc::new(niles_mqtt::CommandRouter::z2m_only("zigbee2mqtt")),
            niles_core::EventBus::default(),
        )
        .with_config_store(Some(config))
        .with_live_browser(Some(fake.clone() as Arc<dyn LiveBrowser>));
        (crate::server::router(state), fake)
    }

    fn by(email: &str, method: &str, uri: &str, body: Option<Value>) -> Request<Body> {
        let session = crate::auth::session::Session::new(email, None);
        let cookie = format!(
            "{}={}",
            crate::auth::session::COOKIE,
            crate::auth::session::sign(SECRET, &session)
        );
        Request::builder()
            .method(method)
            .uri(uri)
            .header(header::COOKIE, cookie)
            .header(header::CONTENT_TYPE, "application/json")
            .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
            .unwrap()
    }

    async fn send(app: &axum::Router, req: Request<Body>) -> (StatusCode, String) {
        let res = app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&bytes).to_string())
    }

    #[tokio::test]
    async fn each_person_sees_only_their_own_browser() {
        let (app, _) = app();
        let (status, hers) = send(&app, by(MAJSE, "GET", "/me/browser", None)).await;
        assert_eq!(status, StatusCode::OK);
        let hers: Value = serde_json::from_str(&hers).unwrap();
        assert_eq!(hers["control"], "person");
        assert_eq!(hers["width"], 1280);
        let (_, his) = send(&app, by(MARK, "GET", "/me/browser", None)).await;
        assert_eq!(his, "null");
        for (method, uri) in [
            ("POST", "/me/browser/take"),
            ("POST", "/me/browser/back"),
            ("POST", "/me/browser/reload"),
        ] {
            let (status, said) = send(&app, by(MARK, method, uri, None)).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{uri}: {said}");
        }
    }

    #[tokio::test]
    async fn giving_back_says_what_follows_and_where() {
        let (app, fake) = app();
        let body = json!({ "conversation": 7, "then": "carry_on" });
        let (status, said) =
            send(&app, by(MAJSE, "POST", "/me/browser/give-back", Some(body))).await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{said}");
        assert_eq!(
            *fake.given_back.lock().unwrap(),
            [(MAJSE.to_string(), Some(7), Then::CarryOn)]
        );
        let (status, _) = send(
            &app,
            by(
                MAJSE,
                "POST",
                "/me/browser/give-back",
                Some(json!({ "then": "later" })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn back_while_niles_has_it_is_a_conflict() {
        let (app, _) = app();
        let (status, said) = send(&app, by(MAJSE, "POST", "/me/browser/back", None)).await;
        assert_eq!(status, StatusCode::CONFLICT, "{said}");
        assert!(said.contains("take it over"), "{said}");
    }
}
