//! The household's automations, from the app.
//!
//! The same store the voice tools write to, so a rule made by asking
//! Niles turns up here to be checked, paused or changed by hand.

use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use niles_automations::store::DEFAULT_SOURCE;
use niles_automations::{AutomationStore, Error, Rule, sentence};
use niles_config::AutomationRuleDto;
use serde::Serialize;
use std::sync::Arc;

/// JSON, not plain text: the app reads `error` out of a failure body to
/// say what went wrong.
type Failure = (StatusCode, Json<serde_json::Value>);

#[derive(Debug, Serialize)]
pub struct AutomationView {
    pub rule: AutomationRuleDto,
    /// `stored` (made by voice or in the app) or `config` (an
    /// `[[automation]]` table in the config file, which the app cannot
    /// change).
    pub source: &'static str,
    /// "When the office door opens, turn off the office lights."
    pub sentence: String,
    /// Why the rule is not running, when it is not.
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Automations {
    /// In the order they run.
    pub rules: Vec<AutomationView>,
}

fn view(rule: AutomationRuleDto, source: &'static str) -> AutomationView {
    AutomationView {
        error: Rule::from_dto(&rule, DEFAULT_SOURCE)
            .err()
            .map(|e| e.to_string()),
        sentence: sentence(&rule),
        rule,
        source,
    }
}

/// `GET /automations`
pub async fn list(State(state): State<AppState>) -> Result<Json<Automations>, Failure> {
    let store = store(&state)?;
    let mut rules: Vec<AutomationView> = store
        .list()
        .into_iter()
        .map(|r| view(r, "stored"))
        .collect();
    rules.extend(config_rules(&state).into_iter().map(|r| view(r, "config")));
    Ok(Json(Automations { rules }))
}

/// `POST /automations` — a rule without an id; it is named after its
/// description.
pub async fn create(
    State(state): State<AppState>,
    Json(mut body): Json<serde_json::Value>,
) -> Result<(StatusCode, Json<AutomationView>), Failure> {
    if let Some(object) = body.as_object_mut() {
        object
            .entry("id")
            .or_insert_with(|| serde_json::Value::String(String::new()));
    }
    let rule = read(body)?;
    let made = store(&state)?.create(rule).map_err(failure)?;
    Ok((StatusCode::CREATED, Json(view(made, "stored"))))
}

/// `PUT /automations/{id}` — the whole rule; pausing is `enabled: false`.
pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<AutomationView>, Failure> {
    let store = store(&state)?;
    from_config(&state, store, &id)?;
    let rule = read(body)?;
    let saved = store.update(&id, rule).map_err(failure)?;
    Ok(Json(view(saved, "stored")))
}

/// `DELETE /automations/{id}`
pub async fn remove(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, Failure> {
    let store = store(&state)?;
    from_config(&state, store, &id)?;
    store.remove(&id).map_err(failure)?;
    Ok(StatusCode::NO_CONTENT)
}

fn read(body: serde_json::Value) -> Result<AutomationRuleDto, Failure> {
    serde_json::from_value(body).map_err(|e| {
        failed(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("that is not an automation: {e}"),
        )
    })
}

fn config_rules(state: &AppState) -> Vec<AutomationRuleDto> {
    state
        .config
        .as_ref()
        .map(|c| c.current().automations.rules.clone())
        .unwrap_or_default()
}

/// A rule from the config file is changed there, not here.
fn from_config(state: &AppState, store: &AutomationStore, id: &str) -> Result<(), Failure> {
    if store.get(id).is_none() && config_rules(state).iter().any(|r| r.id == id) {
        return Err(failed(
            StatusCode::CONFLICT,
            format!("'{id}' comes from the config file; change it there"),
        ));
    }
    Ok(())
}

fn store(state: &AppState) -> Result<&Arc<AutomationStore>, Failure> {
    state.automations.as_ref().ok_or_else(|| {
        failed(
            StatusCode::NOT_IMPLEMENTED,
            "this Niles instance runs no automations".into(),
        )
    })
}

fn failure(e: Error) -> Failure {
    let status = match e {
        Error::NotFound { .. } => StatusCode::NOT_FOUND,
        _ => StatusCode::UNPROCESSABLE_ENTITY,
    };
    failed(status, e.to_string())
}

fn failed(status: StatusCode, message: String) -> Failure {
    (status, Json(serde_json::json!({ "error": message })))
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
    use niles_automations::AutomationStore;
    use niles_core::{DeviceRegistry, EventBus};
    use serde_json::{Value, json};
    use std::sync::Arc;
    use tower::ServiceExt;

    struct NoopPublisher;

    #[async_trait]
    impl DevicePublisher for NoopPublisher {
        async fn publish(&self, _topic: String, _payload: Vec<u8>) -> Result<(), String> {
            Ok(())
        }
    }

    fn app(store: Arc<AutomationStore>) -> axum::Router {
        router(
            AppState::new(
                Arc::new(DeviceRegistry::new()),
                Arc::new(NoopPublisher) as Arc<dyn DevicePublisher>,
                Arc::new(niles_mqtt::CommandRouter::z2m_only("zigbee2mqtt")),
                EventBus::default(),
            )
            .with_automations(Some(store)),
        )
    }

    async fn send(
        app: &axum::Router,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    fn garden_door() -> Value {
        json!({
            "description": "Office lights off when the garden door opens",
            "enabled": true,
            "trigger": { "type": "contact", "device": "office/door", "open": true },
            "conditions": [],
            "actions": [{ "do": "set_room", "room": "office", "on": false }],
        })
    }

    #[tokio::test]
    async fn makes_a_rule_and_says_it() {
        let store = Arc::new(AutomationStore::new());
        let app = app(store.clone());
        let (status, made) = send(&app, "POST", "/automations", Some(garden_door())).await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(
            made["rule"]["id"],
            "office-lights-off-when-the-garden-door-opens"
        );
        assert_eq!(
            made["sentence"],
            "When the office door opens, turn off the office lights."
        );
        assert_eq!(made["error"], Value::Null);

        let (_, listed) = send(&app, "GET", "/automations", None).await;
        assert_eq!(listed["rules"][0]["source"], "stored");
        assert_eq!(store.list().len(), 1);
    }

    #[tokio::test]
    async fn pauses_and_removes_a_rule() {
        let app = app(Arc::new(AutomationStore::new()));
        let (_, made) = send(&app, "POST", "/automations", Some(garden_door())).await;
        let id = made["rule"]["id"].as_str().unwrap().to_string();
        let mut paused = made["rule"].clone();
        paused["enabled"] = json!(false);
        let (status, saved) = send(&app, "PUT", &format!("/automations/{id}"), Some(paused)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(saved["rule"]["enabled"], false);

        let (status, _) = send(&app, "DELETE", &format!("/automations/{id}"), None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, _) = send(&app, "DELETE", &format!("/automations/{id}"), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn says_why_a_rule_is_refused() {
        let app = app(Arc::new(AutomationStore::new()));
        let mut nothing = garden_door();
        nothing["actions"] = json!([]);
        let (status, body) = send(&app, "POST", "/automations", Some(nothing)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(body["error"].as_str().unwrap().contains("no actions"));

        let (status, body) = send(
            &app,
            "POST",
            "/automations",
            Some(json!({ "trigger": { "type": "earthquake" } })),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(body["error"].as_str().unwrap().starts_with("that is not"));
    }
}
