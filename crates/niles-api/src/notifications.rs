//! The signed-in person's notifications: the list on their
//! Notifications page, the count on the bell, and marking them read.
//!
//! Only a session's own. Whose they are is the session's, never an id
//! or a name in the request: an id of somebody else's marks nothing, the
//! same as one that was never there. Not the API token's either, which
//! is nobody's.

use crate::chat::{Failure, failed};
use crate::state::AppState;
use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use niles_core::Event;
use niles_webpush::Notification;
use serde::{Deserialize, Serialize};

/// One page of the list.
pub const PAGE: i64 = 50;

#[derive(Debug, Serialize)]
pub struct List {
    /// Newest first.
    pub notifications: Vec<Notification>,
    /// How many of all of theirs are unread, on this page or not.
    pub unread: i64,
    /// What to ask for the next page with, while there is one.
    pub next: Option<i64>,
    /// False without a database: nothing is kept, and the page says so
    /// rather than showing an empty list as if it were one.
    pub kept: bool,
}

#[derive(Debug, Deserialize)]
pub struct Page {
    #[serde(default)]
    pub before: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Read {
    /// The newest one the page showed. Absent marks every one.
    #[serde(default)]
    pub up_to: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct Unread {
    pub unread: i64,
}

/// The person key of whoever is signed in. A request without a session
/// has nobody's notifications to see.
fn signed_in(state: &AppState, headers: &HeaderMap) -> Result<String, Failure> {
    let config = state.config.as_ref().map(|c| c.current());
    let email = config
        .as_ref()
        .and_then(|config| crate::auth::signed_in_as(headers, config));
    let (Some(config), Some(email)) = (config, email) else {
        return Err(failed(
            StatusCode::UNAUTHORIZED,
            "sign in to see your notifications".into(),
        ));
    };
    let speaker = config
        .auth
        .person_for(&email)
        .and_then(|p| p.speaker.clone());
    Ok(niles_training::person_key(speaker.as_deref(), &email))
}

fn storage(e: niles_webpush::Error) -> Failure {
    failed(StatusCode::BAD_GATEWAY, e.to_string())
}

/// `GET /me/notifications?before=`
pub async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(page): Query<Page>,
) -> Result<Json<List>, Failure> {
    let person = signed_in(&state, &headers)?;
    let Some(inbox) = &state.notifications else {
        return Ok(Json(List {
            notifications: Vec::new(),
            unread: 0,
            next: None,
            kept: false,
        }));
    };
    let mut notifications = inbox
        .page(&person, page.before, PAGE + 1)
        .await
        .map_err(storage)?;
    let more = notifications.len() as i64 > PAGE;
    notifications.truncate(PAGE as usize);
    let next = more.then(|| notifications.last().map(|n| n.id)).flatten();
    let unread = inbox.unread(&person).await.map_err(storage)?;
    Ok(Json(List {
        notifications,
        unread,
        next,
        kept: true,
    }))
}

/// `POST /me/notifications/read`: theirs read, up to the one given.
/// Their other devices hear it, so every bell of theirs clears.
pub async fn read(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Read>,
) -> Result<Json<Unread>, Failure> {
    let person = signed_in(&state, &headers)?;
    let Some(inbox) = &state.notifications else {
        return Ok(Json(Unread { unread: 0 }));
    };
    let marked = inbox.read(&person, body.up_to).await.map_err(storage)?;
    if marked > 0 {
        state.event_bus.publish(Event::NotificationsRead {
            person: person.clone(),
        });
    }
    let unread = inbox.unread(&person).await.map_err(storage)?;
    Ok(Json(Unread { unread }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::session::{self, Session};
    use crate::publish::DevicePublisher;
    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::{Request, header};
    use http_body_util::BodyExt;
    use niles_core::EventBus;
    use niles_webpush::inbox::Kind;
    use niles_webpush::{Inbox, MemoryInbox, Notice};
    use serde_json::{Value, json};
    use std::sync::Arc;
    use tower::ServiceExt;

    const SECRET_VAR: &str = "NILES_TEST_NOTIFICATIONS_SESSION_SECRET";
    const SECRET: &str = "the-notifications-page-secret";
    const MARK: &str = "person:mark";

    struct NoopPublisher;

    #[async_trait]
    impl DevicePublisher for NoopPublisher {
        async fn publish(&self, _topic: String, _payload: Vec<u8>) -> Result<(), String> {
            Ok(())
        }
    }

    struct Fixture {
        app: axum::Router,
        inbox: Arc<MemoryInbox>,
        bus: EventBus,
    }

    fn fixture(kept: bool) -> Fixture {
        // SAFETY: test-only variables nothing else reads.
        unsafe {
            std::env::set_var(SECRET_VAR, SECRET);
            std::env::set_var("NILES_TEST_NOTIFICATIONS_CLIENT_ID", "id");
            std::env::set_var("NILES_TEST_NOTIFICATIONS_CLIENT_SECRET", "secret");
        }
        let toml = format!(
            r#"{}
[auth]
github_client_id_env = "NILES_TEST_NOTIFICATIONS_CLIENT_ID"
github_client_secret_env = "NILES_TEST_NOTIFICATIONS_CLIENT_SECRET"
session_secret_env = "{SECRET_VAR}"
allowed = [
  {{ email = "mark@example.com", speaker = "mark" }},
  {{ email = "majse@example.com", speaker = "majse" }},
]
"#,
            crate::config_tests::base_toml()
        );
        let config = Arc::new(niles_config::ConfigStore::from_str_in_memory(&toml).unwrap());
        let inbox = Arc::new(MemoryInbox::default());
        let bus = EventBus::default();
        let state = AppState::new(
            Arc::new(niles_core::DeviceRegistry::new()),
            Arc::new(NoopPublisher) as Arc<dyn DevicePublisher>,
            Arc::new(niles_mqtt::CommandRouter::z2m_only("zigbee2mqtt")),
            bus.clone(),
        )
        .with_config_store(Some(config))
        .with_notifications(kept.then(|| inbox.clone() as Arc<dyn Inbox>));
        Fixture {
            app: crate::server::router(state),
            inbox,
            bus,
        }
    }

    fn request(email: Option<&str>, method: &str, uri: &str, body: Option<Value>) -> Request<Body> {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(email) = email {
            builder = builder.header(
                header::COOKIE,
                format!(
                    "{}={}",
                    session::COOKIE,
                    session::sign(SECRET, &Session::new(email, None))
                ),
            );
        }
        builder
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

    fn notice(tag: &str) -> Notice {
        Notice {
            kind: Kind::Workout,
            title: "Hellerup plan".into(),
            body: "Your 6.2 km run today was counted as the easy run.".into(),
            url: "/#/chat?artifact=1".into(),
            tag: tag.into(),
        }
    }

    #[tokio::test]
    async fn the_list_is_the_callers_own_newest_first_with_the_unread_count() {
        let f = fixture(true);
        f.inbox.add(MARK, &notice("a")).await.unwrap();
        let newest = f.inbox.add(MARK, &notice("b")).await.unwrap();
        f.inbox.add("person:majse", &notice("c")).await.unwrap();

        let (status, body) = send(
            &f.app,
            request(Some("mark@example.com"), "GET", "/me/notifications", None),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["kept"], true);
        assert_eq!(body["unread"], 2);
        assert_eq!(body["next"], Value::Null);
        let list = body["notifications"].as_array().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0]["id"], newest.id);
        assert_eq!(list[0]["kind"], "workout");
        assert_eq!(list[0]["url"], "/#/chat?artifact=1");
        assert!(list[0].get("tag").is_none());

        let (_, hers) = send(
            &f.app,
            request(Some("majse@example.com"), "GET", "/me/notifications", None),
        )
        .await;
        assert_eq!(hers["notifications"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn the_list_comes_a_page_at_a_time() {
        let f = fixture(true);
        for n in 0..PAGE + 3 {
            f.inbox.add(MARK, &notice(&format!("w{n}"))).await.unwrap();
        }
        let (_, first) = send(
            &f.app,
            request(Some("mark@example.com"), "GET", "/me/notifications", None),
        )
        .await;
        assert_eq!(
            first["notifications"].as_array().unwrap().len() as i64,
            PAGE
        );
        let next = first["next"].as_i64().expect("a next page");
        let (_, rest) = send(
            &f.app,
            request(
                Some("mark@example.com"),
                "GET",
                &format!("/me/notifications?before={next}"),
                None,
            ),
        )
        .await;
        assert_eq!(rest["notifications"].as_array().unwrap().len(), 3);
        assert_eq!(rest["next"], Value::Null);
    }

    #[tokio::test]
    async fn reading_marks_the_callers_own_and_tells_only_them() {
        let f = fixture(true);
        let first = f.inbox.add(MARK, &notice("a")).await.unwrap();
        f.inbox.add(MARK, &notice("b")).await.unwrap();
        let hers = f.inbox.add("person:majse", &notice("c")).await.unwrap();
        let mut events = f.bus.subscribe();

        let (status, body) = send(
            &f.app,
            request(
                Some("mark@example.com"),
                "POST",
                "/me/notifications/read",
                Some(json!({ "up_to": first.id })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["unread"], 1);
        match events.try_recv() {
            Ok(Event::NotificationsRead { person }) => assert_eq!(person, MARK),
            other => panic!("expected NotificationsRead, got {other:?}"),
        }

        // Her id from his session marks nothing of hers.
        let (_, body) = send(
            &f.app,
            request(
                Some("mark@example.com"),
                "POST",
                "/me/notifications/read",
                Some(json!({ "up_to": hers.id })),
            ),
        )
        .await;
        assert_eq!(body["unread"], 0);
        assert_eq!(f.inbox.unread("person:majse").await.unwrap(), 1);

        // Nothing left to read says nothing.
        let mut events = f.bus.subscribe();
        send(
            &f.app,
            request(
                Some("mark@example.com"),
                "POST",
                "/me/notifications/read",
                Some(json!({})),
            ),
        )
        .await;
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn without_a_session_there_is_nothing_to_see() {
        let f = fixture(true);
        f.inbox.add(MARK, &notice("a")).await.unwrap();
        for (method, uri) in [
            ("GET", "/me/notifications"),
            ("POST", "/me/notifications/read"),
        ] {
            let (status, _) = send(&f.app, request(None, method, uri, Some(json!({})))).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {uri}");
        }
        assert_eq!(f.inbox.unread(MARK).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn without_a_database_nothing_is_kept_and_it_says_so() {
        let f = fixture(false);
        let (status, body) = send(
            &f.app,
            request(Some("mark@example.com"), "GET", "/me/notifications", None),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["kept"], false);
        assert_eq!(body["notifications"], json!([]));
        let (status, body) = send(
            &f.app,
            request(
                Some("mark@example.com"),
                "POST",
                "/me/notifications/read",
                Some(json!({})),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["unread"], 0);
    }
}
