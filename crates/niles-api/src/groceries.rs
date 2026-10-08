//! The shopping list, from the app.
//!
//! The same store voice adds to, so "add milk" in the kitchen shows up
//! on the phone in the shop, and checking it off there is what teaches
//! Niles that milk means Letmælk.

use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use niles_groceries::{Added, Edit, Error, GroceryStore, Item, NemligProduct};
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
    /// Whether nemlig.com is switched on and has a login, so the page
    /// offers to pick products there.
    pub nemlig: bool,
}

#[derive(Deserialize)]
pub struct NemligChoice {
    /// The product, or null to stop using nemlig.com for this item.
    pub product: Option<NemligProduct>,
}

#[derive(Deserialize)]
pub struct NemligQuery {
    pub q: String,
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
        nemlig: state.nemlig.is_some() && nemlig_login(&state).is_ok(),
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

/// `GET /groceries/nemlig/search?q=` — what nemlig.com sells by that
/// name, priced for this account.
pub async fn nemlig_search(
    State(state): State<AppState>,
    Query(query): Query<NemligQuery>,
) -> Result<Json<Vec<NemligProduct>>, Failure> {
    let client = state.nemlig.clone().ok_or_else(|| {
        failed(
            StatusCode::NOT_IMPLEMENTED,
            "this Niles instance cannot reach nemlig.com".into(),
        )
    })?;
    let credentials = nemlig_login(&state)?;
    let q = query.q.trim();
    if q.is_empty() {
        return Ok(Json(Vec::new()));
    }
    client
        .search(&credentials, q, NEMLIG_RESULTS)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::warn!("[nemlig] search for {q:?} failed: {e}");
            failed(StatusCode::BAD_GATEWAY, e.to_string())
        })
}

/// `PUT /groceries/{id}/nemlig` — choose the nemlig.com product for an
/// item, or none. Remembered for the next item of that name.
pub async fn choose_nemlig(
    State(state): State<AppState>,
    Path(id): Path<u64>,
    Json(choice): Json<NemligChoice>,
) -> Result<Json<Item>, Failure> {
    let store = store(&state)?;
    store
        .choose_nemlig(id, choice.product)
        .map(Json)
        .map_err(failure)
}

/// What sending the list did.
#[derive(Serialize)]
pub struct Sent {
    pub basket: niles_nemlig::Basket,
    /// How many items went in.
    pub sent: usize,
    /// Items still to buy with no nemlig.com product chosen, which did not.
    pub without: Vec<String>,
    /// Items whose product would not go in — sold out since it was chosen.
    pub unavailable: Vec<String>,
    /// Where to review the basket and pay.
    pub checkout: &'static str,
}

#[derive(Deserialize)]
pub struct Reserve {
    pub slot_id: i64,
}

/// `POST /groceries/nemlig/basket` — put everything still to buy that has
/// a nemlig.com product into the basket there.
///
/// At least as many as the list says, never fewer than are there already:
/// sending twice does not order twice.
pub async fn nemlig_send(State(state): State<AppState>) -> Result<Json<Sent>, Failure> {
    let store = store(&state)?;
    let (client, credentials) = nemlig(&state)?;
    let to_buy: Vec<Item> = store
        .list()
        .into_iter()
        .filter(|i| i.checked_at.is_none())
        .collect();
    let wanted: Vec<(String, u32)> = to_buy
        .iter()
        .filter_map(|i| {
            let product = i.nemlig.as_ref()?;
            Some((product.id.clone(), how_many(i.quantity.as_deref())))
        })
        .collect();
    let without = to_buy
        .iter()
        .filter(|i| i.nemlig.is_none())
        .map(|i| i.name.clone())
        .collect();
    let basket = client
        .fill_basket(&credentials, &wanted)
        .await
        .map_err(nemlig_failure)?;
    // Asked for and not there: nemlig keeps a sold-out line at nothing.
    let unavailable = to_buy
        .iter()
        .filter_map(|i| {
            let product = i.nemlig.as_ref()?;
            (basket.quantity_of(&product.id) == 0).then(|| i.name.clone())
        })
        .collect();
    Ok(Json(Sent {
        basket,
        sent: wanted.len(),
        without,
        unavailable,
        checkout: niles_nemlig::CHECKOUT,
    }))
}

/// `GET /groceries/nemlig/check` — the products chosen for what is still
/// to buy, as they are now: price, stock and offer. What was saved when
/// each was chosen goes stale; offers change every week.
pub async fn nemlig_check(
    State(state): State<AppState>,
) -> Result<Json<Vec<NemligProduct>>, Failure> {
    let store = store(&state)?;
    let (client, credentials) = nemlig(&state)?;
    let mut chosen: Vec<(String, String)> = Vec::new();
    for item in store.list().into_iter().filter(|i| i.checked_at.is_none()) {
        if let Some(product) = item.nemlig
            && !chosen.iter().any(|(id, _)| *id == product.id)
        {
            chosen.push((product.id, product.name));
        }
    }
    client
        .check(&credentials, &chosen)
        .await
        .map(Json)
        .map_err(nemlig_failure)
}

/// `GET /groceries/nemlig/next` — the next order still to arrive, or
/// null: what the dashboard shows on the day it comes.
pub async fn nemlig_next(
    State(state): State<AppState>,
) -> Result<Json<Option<niles_nemlig::Order>>, Failure> {
    let (client, credentials) = nemlig(&state)?;
    client
        .next_delivery(&credentials, niles_nemlig::danish_now())
        .await
        .map(Json)
        .map_err(nemlig_failure)
}

/// `GET /groceries/nemlig/delivery` — the coming week's delivery times.
pub async fn nemlig_delivery(
    State(state): State<AppState>,
) -> Result<Json<Vec<niles_nemlig::DeliveryDay>>, Failure> {
    let (client, credentials) = nemlig(&state)?;
    client
        .delivery_days(&credentials, 7)
        .await
        .map(Json)
        .map_err(nemlig_failure)
}

/// `POST /groceries/nemlig/delivery {slot_id}` — reserve one.
pub async fn nemlig_reserve(
    State(state): State<AppState>,
    Json(body): Json<Reserve>,
) -> Result<Json<niles_nemlig::Basket>, Failure> {
    let (client, credentials) = nemlig(&state)?;
    client
        .reserve(&credentials, body.slot_id)
        .await
        .map(Json)
        .map_err(nemlig_failure)
}

/// How many of a product the list asks for: the number it starts with —
/// "2", "2 poser" — or one. Capped, because "500 g" is a weight, not
/// five hundred packets.
fn how_many(quantity: Option<&str>) -> u32 {
    let digits: String = quantity
        .unwrap_or("")
        .trim()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    match digits.parse::<u32>() {
        Ok(n) if (1..=20).contains(&n) => n,
        _ => 1,
    }
}

fn nemlig(
    state: &AppState,
) -> Result<(Arc<niles_nemlig::NemligClient>, niles_nemlig::Credentials), Failure> {
    let client = state.nemlig.clone().ok_or_else(|| {
        failed(
            StatusCode::NOT_IMPLEMENTED,
            "this Niles instance cannot reach nemlig.com".into(),
        )
    })?;
    Ok((client, nemlig_login(state)?))
}

fn nemlig_failure(e: niles_nemlig::Error) -> Failure {
    tracing::warn!("[nemlig] {e}");
    let status = match e {
        niles_nemlig::Error::Api { status: 409, .. } => StatusCode::CONFLICT,
        _ => StatusCode::BAD_GATEWAY,
    };
    failed(status, e.to_string())
}

/// How many products the picker is offered: a screenful, no more.
const NEMLIG_RESULTS: u32 = 12;

/// The nemlig.com login, when the integration is on and has one.
fn nemlig_login(state: &AppState) -> Result<niles_nemlig::Credentials, Failure> {
    let config = state.config.as_ref().map(|c| c.current());
    let nemlig = config
        .as_ref()
        .and_then(|c| c.integrations.nemlig.clone())
        .filter(|n| n.enabled)
        .ok_or_else(|| failed(StatusCode::CONFLICT, "nemlig.com is not switched on".into()))?;
    let (username, password) = nemlig
        .resolve_credentials()
        .map_err(|e| failed(StatusCode::CONFLICT, e.to_string()))?;
    Ok(niles_nemlig::Credentials { username, password })
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
    async fn a_nemlig_choice_is_kept_and_offered_again() {
        let app = app(Some(Arc::new(GroceryStore::new())));
        let (_, added) = send(
            &app,
            "POST",
            "/groceries",
            Some(json!({"name": "Rundstykker"})),
        )
        .await;
        let id = added["item"]["id"].as_u64().unwrap();
        let product = json!({
            "id": "5060220", "name": "Surdejsrundstykker",
            "description": "6 stk. / 420 g / frost / Hatting", "price": 14.95,
            "unit_price": "35,60 kr/kg", "image": null, "available": true
        });
        let (status, item) = send(
            &app,
            "PUT",
            &format!("/groceries/{id}/nemlig"),
            Some(json!({ "product": product })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(item["nemlig"]["id"], "5060220");

        send(&app, "DELETE", &format!("/groceries/{id}"), None).await;
        let (_, again) = send(
            &app,
            "POST",
            "/groceries",
            Some(json!({"name": "rundstykker"})),
        )
        .await;
        assert_eq!(again["item"]["nemlig"]["name"], "Surdejsrundstykker");
    }

    #[tokio::test]
    async fn nemlig_is_off_without_a_client_or_a_login() {
        let app = app(Some(Arc::new(GroceryStore::new())));
        let (_, list) = send(&app, "GET", "/groceries", None).await;
        assert_eq!(list["nemlig"], false);
        let (status, _) = send(&app, "GET", "/groceries/nemlig/search?q=milk", None).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    }

    #[test]
    fn the_list_says_how_many_or_it_is_one() {
        use super::how_many;
        assert_eq!(how_many(None), 1);
        assert_eq!(how_many(Some("2")), 2);
        assert_eq!(how_many(Some("3 poser")), 3);
        assert_eq!(how_many(Some("en pakke")), 1);
        assert_eq!(how_many(Some("500 g")), 1);
        assert_eq!(how_many(Some("0")), 1);
    }

    #[tokio::test]
    async fn sending_needs_nemlig_to_be_on() {
        let app = app(Some(Arc::new(GroceryStore::new())));
        let (status, body) = send(&app, "POST", "/groceries/nemlig/basket", None).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert!(body["error"].is_string());
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
