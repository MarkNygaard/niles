//! Each person's saved logins for Niles's browser, from their profile.
//!
//! These are theirs alone, reached through the session: the house's API
//! token is nobody in particular and reaches none of it, and another
//! person's login answers 404 on every route and is left unchanged
//! (rule 18). No response type here can hold a password, because
//! [`Secret`] has neither `Serialize` nor `Display` (rule 7).

use crate::state::AppState;
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use niles_logins::{Login, LoginEdit, LoginStore, NewLogin, Secret};
use serde::{Deserialize, Serialize};

type Failure = (StatusCode, Json<serde_json::Value>);

fn failed(status: StatusCode, message: impl Into<String>) -> Failure {
    (status, Json(serde_json::json!({ "error": message.into() })))
}

fn store(state: &AppState) -> Result<&std::sync::Arc<dyn LoginStore>, Failure> {
    state.logins.as_ref().ok_or_else(|| {
        failed(
            StatusCode::NOT_IMPLEMENTED,
            "Saved logins need Niles's database and its secrets key, and this Niles doesn't have both.",
        )
    })
}

/// The person signed in to this browser, keyed as their chat is.
///
/// A session only: the house's API key is nobody in particular.
fn signed_in_person(state: &AppState, headers: &HeaderMap) -> Result<String, Failure> {
    let config = state
        .config
        .as_ref()
        .ok_or_else(|| {
            failed(
                StatusCode::NOT_IMPLEMENTED,
                "this Niles instance has no people configured",
            )
        })?
        .current();
    let email = crate::auth::signed_in_as(headers, &config)
        .ok_or_else(|| failed(StatusCode::UNAUTHORIZED, "sign in to keep your own logins"))?;
    let speaker = config
        .auth
        .person_for(&email)
        .and_then(|p| p.speaker.clone());
    Ok(niles_training::person_key(speaker.as_deref(), &email))
}

fn refused(e: niles_logins::Error) -> Failure {
    match e {
        niles_logins::Error::Invalid { reason, .. } => {
            failed(StatusCode::UNPROCESSABLE_ENTITY, reason)
        }
        why @ niles_logins::Error::Storage(_) => {
            tracing::warn!("[logins] {why}");
            failed(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("saved logins couldn't be read or written: {why}"),
            )
        }
    }
}

const NO_SUCH: &str = "no such login";

/// What the form posts. The username and password arrive as
/// [`Secret`]s, so neither can leak into a log line from here.
#[derive(Debug, Deserialize)]
pub struct LoginBody {
    pub label: String,
    pub origins: Vec<String>,
    pub username: Secret,
    /// Absent (or empty, in an edit) keeps a saved password.
    #[serde(default)]
    pub password: Option<Secret>,
}

fn read(body: Result<Json<LoginBody>, JsonRejection>) -> Result<LoginBody, Failure> {
    // The fixed text never echoes the body: what didn't read may hold
    // what the person typed.
    body.map(|Json(b)| b).map_err(|_| {
        failed(
            StatusCode::BAD_REQUEST,
            "send the login as JSON with its label, origins, username and password",
        )
    })
}

#[derive(Serialize)]
pub struct LoginList {
    pub logins: Vec<Login>,
}

/// `GET /me/logins` — the signed-in person's own, by label then id.
pub async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<LoginList>, Failure> {
    let person = signed_in_person(&state, &headers)?;
    let logins = store(&state)?.list(&person).await.map_err(refused)?;
    Ok(Json(LoginList { logins }))
}

/// `POST /me/logins` — what the Logins form, or a card's form, saves.
pub async fn add(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<LoginBody>, JsonRejection>,
) -> Result<(StatusCode, Json<Login>), Failure> {
    let person = signed_in_person(&state, &headers)?;
    let store = store(&state)?;
    let b = read(body)?;
    let new = NewLogin::new(
        &b.label,
        &b.origins,
        b.username,
        b.password.unwrap_or_else(|| Secret::new("")),
    )
    .map_err(refused)?;
    let login = store.add(&person, &new).await.map_err(refused)?;
    // The person key and the id only: never a label, a username or an
    // origin.
    tracing::info!("[logins] {person} added login {}", login.id);
    Ok((StatusCode::CREATED, Json(login)))
}

/// `PUT /me/logins/{id}` — a change from the form. An empty or absent
/// password keeps the saved one.
pub async fn update(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    body: Result<Json<LoginBody>, JsonRejection>,
) -> Result<Json<Login>, Failure> {
    let person = signed_in_person(&state, &headers)?;
    let store = store(&state)?;
    let b = read(body)?;
    let edit = LoginEdit::new(&b.label, &b.origins, b.username, b.password).map_err(refused)?;
    let login = store
        .update(&person, id, &edit)
        .await
        .map_err(refused)?
        .ok_or_else(|| failed(StatusCode::NOT_FOUND, NO_SUCH))?;
    Ok(Json(login))
}

/// `DELETE /me/logins/{id}` — the login, its sealed values and its
/// browser profile.
pub async fn remove(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<StatusCode, Failure> {
    let person = signed_in_person(&state, &headers)?;
    let store = store(&state)?;
    if !store.remove(&person, id).await.map_err(refused)? {
        return Err(failed(StatusCode::NOT_FOUND, NO_SUCH));
    }
    tracing::info!("[logins] {person} removed login {id}");
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /me/logins/{id}/sign-out` — forget the browser profile, keep
/// the login.
pub async fn sign_out(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<StatusCode, Failure> {
    let person = signed_in_person(&state, &headers)?;
    let store = store(&state)?;
    if !store.sign_out(&person, id).await.map_err(refused)? {
        return Err(failed(StatusCode::NOT_FOUND, NO_SUCH));
    }
    tracing::info!("[logins] {person} signed out of login {id}");
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::{self, Chat, NotAdded, NotEntered, Unanswerable, Undecidable, Upload, Where};
    use crate::publish::DevicePublisher;
    use axum::body::Body;
    use axum::http::{Request, header};
    use http_body_util::BodyExt;
    use niles_logins::MemoryLogins;
    use serde_json::{Value, json};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tower::ServiceExt;

    const SECRET_VAR: &str = "NILES_TEST_LOGINS_SESSION_SECRET";
    const SECRET: &str = "the-logins-page-secret";
    const TOKEN_VAR: &str = "NILES_TEST_LOGINS_API_TOKEN";
    const MAJSE: &str = "person:majse";
    const HER_USER: &str = "majse.private@example.com";
    const HER_PASSWORD: &str = "correct horse battery staple 42";

    struct NoopPublisher;

    #[async_trait::async_trait]
    impl DevicePublisher for NoopPublisher {
        async fn publish(&self, _topic: String, _payload: Vec<u8>) -> Result<(), String> {
            Ok(())
        }
    }

    /// A chat that counts what it is asked. The logins routes must never
    /// call it: a login's values go to `/me/logins`, and only the new
    /// login's id ever reaches the chat's continuation route.
    struct Watched {
        calls: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl Chat for Watched {
        async fn reply(
            &self,
            _who: &str,
            _speaker: Option<&str>,
            _text: &str,
            _attachments: Vec<Upload>,
            _into: Where,
        ) -> Result<chat::Reply, String> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Err("unused".into())
        }

        async fn conversations(&self, _who: &str, _speaker: Option<&str>) -> Vec<chat::Summary> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Vec::new()
        }

        async fn conversation(
            &self,
            _who: &str,
            _speaker: Option<&str>,
            _id: i64,
        ) -> Option<chat::Conversation> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            None
        }

        async fn attachment(
            &self,
            _who: &str,
            _speaker: Option<&str>,
            _conversation: i64,
            _id: i64,
        ) -> Option<chat::Served> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            None
        }

        async fn answer(
            &self,
            _who: &str,
            _speaker: Option<&str>,
            _id: i64,
            _turn: usize,
            _save: bool,
        ) -> Result<String, Unanswerable> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Err(Unanswerable::NoSuchConversation)
        }

        async fn decide(
            &self,
            _who: &str,
            _speaker: Option<&str>,
            _id: i64,
            _approval: &str,
            _decision: chat::Decision,
            _device: Option<&str>,
        ) -> Result<String, Undecidable> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Err(Undecidable::NoSuchConversation)
        }

        async fn delete(&self, _who: &str, _speaker: Option<&str>, _id: i64) -> bool {
            self.calls.fetch_add(1, Ordering::Relaxed);
            false
        }

        async fn login_added(
            &self,
            _who: &str,
            _speaker: Option<&str>,
            _id: i64,
            _request: &str,
            _login: i64,
        ) -> Result<String, NotAdded> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Err(NotAdded::NoSuchConversation)
        }

        async fn code_entered(
            &self,
            _who: &str,
            _speaker: Option<&str>,
            _id: i64,
            _request: &str,
            _code: niles_logins::Secret,
        ) -> Result<String, NotEntered> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Err(NotEntered::NoSuchConversation)
        }

        async fn transcribe(&self, audio: Vec<u8>, filename: &str) -> Result<String, String> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Ok(format!("{} bytes of {filename}", audio.len()))
        }
    }

    impl Watched {
        fn calls(&self) -> usize {
            self.calls.load(Ordering::Relaxed)
        }
    }

    struct Fixture {
        app: axum::Router,
        store: Arc<MemoryLogins>,
        watched: Arc<Watched>,
    }

    async fn fixture(with_store: bool) -> Fixture {
        // SAFETY: these are test-only variable names nothing else reads.
        unsafe {
            std::env::set_var(SECRET_VAR, SECRET);
            std::env::set_var("NILES_TEST_LOGINS_CLIENT_ID", "id");
            std::env::set_var("NILES_TEST_LOGINS_CLIENT_SECRET", "secret");
            std::env::set_var(TOKEN_VAR, "the-house-token");
        }
        let toml = format!(
            r#"{}
[auth]
github_client_id_env = "NILES_TEST_LOGINS_CLIENT_ID"
github_client_secret_env = "NILES_TEST_LOGINS_CLIENT_SECRET"
session_secret_env = "{SECRET_VAR}"
api_token_env = "{TOKEN_VAR}"
allowed = [
  {{ email = "mark@example.com", speaker = "mark" }},
  {{ email = "majse@example.com", speaker = "majse" }},
]
"#,
            crate::config_tests::base_toml()
        );
        let config = Arc::new(niles_config::ConfigStore::from_str_in_memory(&toml).unwrap());
        let store = Arc::new(MemoryLogins::new());
        let watched = Arc::new(Watched {
            calls: AtomicUsize::new(0),
        });
        let mut state = AppState::new(
            Arc::new(niles_core::DeviceRegistry::new()),
            Arc::new(NoopPublisher) as Arc<dyn DevicePublisher>,
            Arc::new(niles_mqtt::CommandRouter::z2m_only("zigbee2mqtt")),
            niles_core::EventBus::default(),
        )
        .with_config_store(Some(config))
        .with_chat(Some(watched.clone()));
        if with_store {
            state = state.with_logins(Some(store.clone()));
        }
        Fixture {
            app: crate::server::router(state),
            store,
            watched,
        }
    }

    fn session_cookie(email: &str) -> String {
        format!(
            "{}={}",
            crate::auth::session::COOKIE,
            crate::auth::session::sign(SECRET, &crate::auth::session::Session::new(email, None))
        )
    }

    fn by(email: &str, method: &str, uri: &str, body: Option<Value>) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header(header::COOKIE, session_cookie(email))
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::HOST, "niles.example")
            .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
            .unwrap()
    }

    async fn send(app: &axum::Router, req: Request<Body>) -> (StatusCode, Value) {
        let res = app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    /// The raw body, for tests that prove a secret never reaches a
    /// response: a `Value` comparison can't see what isn't a field.
    async fn send_text(app: &axum::Router, req: Request<Body>) -> (StatusCode, String) {
        let res = app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&bytes).to_string())
    }

    fn draft(password: Option<&str>) -> Value {
        let mut body = json!({
            "label": "Shop",
            "origins": ["shop.example"],
            "username": HER_USER,
        });
        if let Some(password) = password {
            body["password"] = json!(password);
        }
        body
    }

    async fn add_login(f: &Fixture, email: &str) -> i64 {
        let (status, body) = send(
            &f.app,
            by(email, "POST", "/me/logins", Some(draft(Some(HER_PASSWORD)))),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        body["id"].as_i64().unwrap()
    }

    #[tokio::test]
    async fn no_response_holds_the_password() {
        let f = fixture(true).await;
        let id = add_login(&f, "majse@example.com").await;
        let (post, post_body) = send_text(
            &f.app,
            by(
                "majse@example.com",
                "POST",
                "/me/logins",
                Some(draft(Some(HER_PASSWORD))),
            ),
        )
        .await;
        assert_eq!(post, StatusCode::CREATED);
        let (get, get_body) =
            send_text(&f.app, by("majse@example.com", "GET", "/me/logins", None)).await;
        assert_eq!(get, StatusCode::OK);
        let (put_status, put_body) = send_text(
            &f.app,
            by(
                "majse@example.com",
                "PUT",
                &format!("/me/logins/{id}"),
                Some(draft(None)),
            ),
        )
        .await;
        assert_eq!(put_status, StatusCode::OK);

        for body in [&post_body, &get_body, &put_body] {
            assert!(!body.contains(HER_PASSWORD), "{body}");
        }
        assert!(get_body.contains(HER_USER), "{get_body}");
        let listed = &serde_json::from_str::<Value>(&get_body).unwrap()["logins"][0];
        let mut keys = listed
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "id",
                "label",
                "last_used",
                "origins",
                "signed_in",
                "username"
            ]
        );
    }

    #[tokio::test]
    async fn an_empty_password_keeps_the_current_one() {
        let f = fixture(true).await;
        let id = add_login(&f, "majse@example.com").await;

        for body in [draft(Some("")), draft(None)] {
            let (status, answer) = send(
                &f.app,
                by(
                    "majse@example.com",
                    "PUT",
                    &format!("/me/logins/{id}"),
                    Some(body),
                ),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{answer}");
            let kept = f
                .store
                .credentials(MAJSE, id)
                .await
                .unwrap()
                .expect("her login is there");
            assert_eq!(kept.password.expose(), HER_PASSWORD);
        }

        let new_password = "a brand new passphrase";
        let (status, _) = send(
            &f.app,
            by(
                "majse@example.com",
                "PUT",
                &format!("/me/logins/{id}"),
                Some(draft(Some(new_password))),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let kept = f
            .store
            .credentials(MAJSE, id)
            .await
            .unwrap()
            .expect("her login is there");
        assert_eq!(kept.password.expose(), new_password);
    }

    #[tokio::test]
    async fn removing_deletes_the_browser_profile() {
        let f = fixture(true).await;
        let id = add_login(&f, "majse@example.com").await;
        assert!(
            f.store
                .save_profile(MAJSE, id, &Secret::new("cookies"))
                .await
                .unwrap()
        );
        let (_, body) = send(&f.app, by("majse@example.com", "GET", "/me/logins", None)).await;
        assert_eq!(body["logins"][0]["signed_in"], true);

        let (status, _) = send(
            &f.app,
            by(
                "majse@example.com",
                "DELETE",
                &format!("/me/logins/{id}"),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert_eq!(f.store.profile(MAJSE, id).await.unwrap(), None);

        let (_, body) = send(&f.app, by("majse@example.com", "GET", "/me/logins", None)).await;
        assert_eq!(body["logins"].as_array().unwrap().len(), 0);

        let (status, _) = send(
            &f.app,
            by(
                "majse@example.com",
                "DELETE",
                &format!("/me/logins/{id}"),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn signing_out_forgets_the_profile_and_keeps_the_login() {
        let f = fixture(true).await;
        let id = add_login(&f, "majse@example.com").await;
        assert!(
            f.store
                .save_profile(MAJSE, id, &Secret::new("cookies"))
                .await
                .unwrap()
        );

        let (status, _) = send(
            &f.app,
            by(
                "majse@example.com",
                "POST",
                &format!("/me/logins/{id}/sign-out"),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert_eq!(f.store.profile(MAJSE, id).await.unwrap(), None);

        let (_, body) = send(&f.app, by("majse@example.com", "GET", "/me/logins", None)).await;
        assert_eq!(body["logins"][0]["id"], id);
        assert_eq!(body["logins"][0]["signed_in"], false);
    }

    #[tokio::test]
    async fn another_persons_login_is_refused_on_every_route() {
        let f = fixture(true).await;
        let id = add_login(&f, "majse@example.com").await;
        assert!(
            f.store
                .save_profile(MAJSE, id, &Secret::new("cookies"))
                .await
                .unwrap()
        );

        let (_, body) = send(&f.app, by("mark@example.com", "GET", "/me/logins", None)).await;
        assert_eq!(body["logins"].as_array().unwrap().len(), 0);

        let same_as_never_used = send(
            &f.app,
            by(
                "mark@example.com",
                "PUT",
                "/me/logins/999",
                Some(draft(None)),
            ),
        )
        .await;
        assert_eq!(same_as_never_used.0, StatusCode::NOT_FOUND);
        for (method, uri) in [
            ("PUT", format!("/me/logins/{id}")),
            ("DELETE", format!("/me/logins/{id}")),
            ("POST", format!("/me/logins/{id}/sign-out")),
        ] {
            let (status, body) = send(
                &f.app,
                by("mark@example.com", method, &uri, Some(draft(None))),
            )
            .await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{method} {uri}: {body}");
            assert_eq!(body, same_as_never_used.1, "{method} {uri}");
        }

        // Hers is untouched: the label, the credentials and the profile.
        let (_, body) = send(&f.app, by("majse@example.com", "GET", "/me/logins", None)).await;
        assert_eq!(body["logins"][0]["label"], "Shop");
        let kept = f
            .store
            .credentials(MAJSE, id)
            .await
            .unwrap()
            .expect("her login is there");
        assert_eq!(kept.username.expose(), HER_USER);
        assert_eq!(kept.password.expose(), HER_PASSWORD);
        assert_eq!(
            f.store.profile(MAJSE, id).await.unwrap(),
            Some(Secret::new("cookies"))
        );
    }

    #[tokio::test]
    async fn the_api_token_reaches_no_logins() {
        let f = fixture(true).await;
        for method in ["GET", "POST"] {
            let req = Request::builder()
                .method(method)
                .uri("/me/logins")
                .header(header::AUTHORIZATION, "Bearer the-house-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(draft(Some(HER_PASSWORD)).to_string()))
                .unwrap();
            let (status, _) = send(&f.app, req).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{method}");
        }
    }

    #[tokio::test]
    async fn without_a_store_the_page_is_told_why() {
        let f = fixture(false).await;
        let (status, body) = send(&f.app, by("majse@example.com", "GET", "/me/logins", None)).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(
            body["error"],
            "Saved logins need Niles's database and its secrets key, and this Niles doesn't have both."
        );
    }

    #[tokio::test]
    async fn a_site_that_isnt_public_https_is_refused_with_why() {
        let f = fixture(true).await;
        for (origins, label) in [
            (json!(["http://192.168.1.10"]), "Shop"),
            (json!(["https://shop.example:8443"]), "Shop"),
            (json!([]), "Shop"),
            (json!(["shop.example"]), "   "),
        ] {
            let body = json!({
                "label": label,
                "origins": origins,
                "username": HER_USER,
                "password": HER_PASSWORD,
            });
            let (status, answer) = send(
                &f.app,
                by("majse@example.com", "POST", "/me/logins", Some(body)),
            )
            .await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
            assert!(answer["error"].is_string(), "{answer}");
        }
    }

    #[tokio::test]
    async fn a_body_that_doesnt_read_is_refused_without_echoing_it() {
        let f = fixture(true).await;
        let (status, body) = send_text(
            &f.app,
            by(
                "majse@example.com",
                "POST",
                "/me/logins",
                Some(json!({
                    "label": "Shop",
                    "origins": ["https://shop.example"],
                    "username": "u",
                    "password": 987654321,
                })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(!body.contains("987654321"), "{body}");
    }

    #[tokio::test]
    async fn adding_a_login_never_reaches_the_chat() {
        let f = fixture(true).await;
        let id = add_login(&f, "majse@example.com").await;
        send(
            &f.app,
            by(
                "majse@example.com",
                "PUT",
                &format!("/me/logins/{id}"),
                Some(draft(None)),
            ),
        )
        .await;
        send(
            &f.app,
            by(
                "majse@example.com",
                "DELETE",
                &format!("/me/logins/{id}"),
                None,
            ),
        )
        .await;
        assert_eq!(f.watched.calls(), 0);
    }
}
