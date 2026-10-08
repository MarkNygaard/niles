//! The shopping list, from the app.
//!
//! The same store voice adds to, so "add milk" in the kitchen shows up
//! on the phone in the shop, and checking it off there is what teaches
//! Niles that milk means Letmælk.

use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use niles_groceries::{Added, Edit, Error, GroceryStore, Item};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// JSON, not plain text: the app reads `error` out of a failure body to
/// say what went wrong.
type Failure = (StatusCode, Json<serde_json::Value>);

/// How many "the usual" suggestions the page is offered.
const USUAL: usize = 12;

#[derive(Serialize)]
pub struct GroceryList {
    pub items: Vec<Item>,
    /// Products bought before and not on the list now, most bought
    /// first: one press to add again.
    pub usual: Vec<String>,
}

#[derive(Deserialize)]
pub struct NewItem {
    pub name: String,
    #[serde(default)]
    pub quantity: Option<String>,
}

#[derive(Serialize)]
pub struct Cleared {
    pub cleared: usize,
}

/// `GET /groceries` — the list, and what is usually on it.
pub async fn list(State(state): State<AppState>) -> Result<Json<GroceryList>, Failure> {
    let store = store(&state)?;
    Ok(Json(GroceryList {
        items: store.list(),
        usual: store.usual(USUAL),
    }))
}

/// `POST /groceries` — add something.
///
/// Typed words go through the catalog like spoken ones, so typing
/// "milk" adds the milk this house buys.
pub async fn add(
    State(state): State<AppState>,
    Json(body): Json<NewItem>,
) -> Result<Json<Added>, Failure> {
    let store = store(&state)?;
    store
        .add(&body.name, None, body.quantity.as_deref())
        .map(Json)
        .map_err(failure)
}

/// `PATCH /groceries/{id}` — rename, requantify, check off or uncheck.
pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<u64>,
    Json(edit): Json<Edit>,
) -> Result<Json<Item>, Failure> {
    let store = store(&state)?;
    store.update(id, edit).map(Json).map_err(failure)
}

/// `DELETE /groceries/{id}` — take it off.
pub async fn remove(
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> Result<StatusCode, Failure> {
    let store = store(&state)?;
    store.remove(id).map_err(failure)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /groceries/clear` — clear away what is in the basket.
pub async fn clear(State(state): State<AppState>) -> Result<Json<Cleared>, Failure> {
    let store = store(&state)?;
    Ok(Json(Cleared {
        cleared: store.clear_checked(),
    }))
}

fn store(state: &AppState) -> Result<&Arc<GroceryStore>, Failure> {
    state.groceries.as_ref().ok_or_else(|| {
        failed(
            StatusCode::NOT_IMPLEMENTED,
            "this Niles instance has no shopping list".into(),
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
    use niles_core::{DeviceRegistry, EventBus};
    use niles_groceries::GroceryStore;
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

    fn app(groceries: Option<Arc<GroceryStore>>) -> axum::Router {
        router(
            AppState::new(
                Arc::new(DeviceRegistry::new()),
                Arc::new(NoopPublisher) as Arc<dyn DevicePublisher>,
                Arc::new(niles_mqtt::CommandRouter::z2m_only("zigbee2mqtt")),
                EventBus::default(),
            )
            .with_groceries(groceries),
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

    #[tokio::test]
    async fn checking_off_in_the_app_teaches_the_list() {
        let app = app(Some(Arc::new(GroceryStore::new())));

        let (status, added) = send(&app, "POST", "/groceries", Some(json!({"name": "milk"}))).await;
        assert_eq!(status, StatusCode::OK);
        let id = added["item"]["id"].as_u64().unwrap();

        let (status, item) = send(
            &app,
            "PATCH",
            &format!("/groceries/{id}"),
            Some(json!({"name": "Letmælk", "checked": true})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(item["checked_at"].is_string());

        let (_, cleared) = send(&app, "POST", "/groceries/clear", None).await;
        assert_eq!(cleared["cleared"], 1);

        // Typed the same way again, it is the milk this house buys.
        let (_, again) = send(&app, "POST", "/groceries", Some(json!({"name": "milk"}))).await;
        assert_eq!(again["item"]["name"], "Letmælk");

        let (_, list) = send(&app, "GET", "/groceries", None).await;
        assert_eq!(list["items"].as_array().unwrap().len(), 1);
        assert_eq!(list["usual"], json!([]));
    }

    #[tokio::test]
    async fn an_unknown_item_is_not_found() {
        let app = app(Some(Arc::new(GroceryStore::new())));
        let (status, _) = send(&app, "DELETE", "/groceries/9", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_refusal_says_why_in_json() {
        let app = app(Some(Arc::new(GroceryStore::new())));
        let long = "x".repeat(200);
        let (status, body) = send(&app, "POST", "/groceries", Some(json!({ "name": long }))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(body["error"].as_str().unwrap().contains("80"), "{body}");
    }

    #[tokio::test]
    async fn without_a_store_it_says_so() {
        let app = app(None);
        let (status, _) = send(&app, "GET", "/groceries", None).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    }
}
