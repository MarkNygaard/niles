//! Webhook handlers for external integrations.

use crate::state::AppState;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use chrono::Utc;
use niles_integration_linear::WebhookPayload;
use niles_notifications::Priority;

const LINEAR_WEBHOOK_TIMESTAMP_TOLERANCE_MS: u64 = 60_000;

pub async fn handle_linear(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    let Some(wh) = state.linear_webhook.as_ref() else {
        return StatusCode::NOT_FOUND;
    };
    let Some(sig) = headers
        .get("linear-signature")
        .and_then(|v| v.to_str().ok())
    else {
        return StatusCode::UNAUTHORIZED;
    };
    if !niles_integration_linear::verify_signature(&wh.secret, &body, sig) {
        return StatusCode::UNAUTHORIZED;
    }
    if headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .map(|ua| ua != "Linear-Webhook")
        .unwrap_or(false)
    {
        return StatusCode::UNAUTHORIZED;
    }
    let payload = match niles_integration_linear::parse_webhook(&body) {
        Ok(p) => p,
        Err(_) => return StatusCode::BAD_REQUEST,
    };
    if !webhook_timestamp_is_current(&payload) {
        return StatusCode::UNAUTHORIZED;
    }
    if let Some(n) = niles_integration_linear::notification_for(&payload, &wh.team) {
        wh.center
            .deliver(n.text, wh.notify_room.clone(), Priority::Important);
    }
    StatusCode::OK
}

fn webhook_timestamp_is_current(payload: &WebhookPayload) -> bool {
    let Some(sent_at) = payload.webhook_timestamp else {
        return false;
    };
    sent_at.abs_diff(Utc::now().timestamp_millis()) <= LINEAR_WEBHOOK_TIMESTAMP_TOLERANCE_MS
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::publish::DevicePublisher;
    use crate::server::router;
    use crate::state::{AppState, LinearWebhookState};
    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::Request;
    use niles_notifications::NotificationCenter;
    use std::sync::Arc;
    use tower::ServiceExt;

    struct NoopPublisher;

    #[async_trait]
    impl DevicePublisher for NoopPublisher {
        async fn publish(&self, _topic: String, _payload: Vec<u8>) -> Result<(), String> {
            Ok(())
        }
    }

    fn sign(secret: &[u8], body: &[u8]) -> String {
        type HmacSha256 = hmac::Hmac<sha2::Sha256>;
        let mut mac = <HmacSha256 as hmac::digest::KeyInit>::new_from_slice(secret).unwrap();
        hmac::Mac::update(&mut mac, body);
        hex::encode(hmac::Mac::finalize(mac).into_bytes())
    }

    fn app_state_with_webhook(secret: &[u8], team: &str) -> AppState {
        let center = Arc::new(NotificationCenter::new(10));
        AppState::new(
            Arc::new(niles_core::DeviceRegistry::default()),
            Arc::new(NoopPublisher),
            Arc::new("zigbee2mqtt".into()),
            niles_core::EventBus::new(16),
        )
        .with_linear_webhook(Some(Arc::new(LinearWebhookState {
            secret: secret.to_vec(),
            team: team.into(),
            notify_room: None,
            center,
        })))
    }

    fn webhook_timestamp() -> i64 {
        Utc::now().timestamp_millis()
    }

    #[tokio::test]
    async fn valid_signed_webhook_delivers() {
        let secret = b"shh";
        let state = app_state_with_webhook(secret, "TEAM");
        let body = serde_json::json!({
            "action": "update",
            "type": "Issue",
            "webhookTimestamp": webhook_timestamp(),
            "data": {
                "identifier": "TEAM-1",
                "title": "Fix it",
                "state": {"name": "In Review", "type": "started"},
                "team": {"key": "TEAM", "name": "My Team"}
            },
            "updatedFrom": {"stateId": "old"}
        })
        .to_string();
        let sig = sign(secret, body.as_bytes());

        let app = router(state.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/webhooks/linear")
                    .header("linear-signature", sig)
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let recent = state.linear_webhook.unwrap().center.recent(1);
        assert_eq!(recent.len(), 1);
        assert!(recent[0].text.contains("pull request ready"));
    }

    #[tokio::test]
    async fn tampered_signature_rejected() {
        let secret = b"shh";
        let state = app_state_with_webhook(secret, "TEAM");
        let body = b"{}";

        let app = router(state.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/webhooks/linear")
                    .header("linear-signature", "bad-sig")
                    .body(Body::from(body.as_slice()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let recent = state.linear_webhook.unwrap().center.recent(1);
        assert!(recent.is_empty());
    }

    #[tokio::test]
    async fn bad_user_agent_rejected() {
        let secret = b"shh";
        let state = app_state_with_webhook(secret, "TEAM");
        let body = serde_json::json!({
            "action": "update",
            "type": "Issue",
            "webhookTimestamp": webhook_timestamp(),
            "data": {
                "identifier": "TEAM-1",
                "title": "Fix it",
                "state": {"name": "In Review", "type": "started"},
                "team": {"key": "TEAM", "name": "My Team"}
            },
            "updatedFrom": {"stateId": "old"}
        })
        .to_string();
        let sig = sign(secret, body.as_bytes());

        let app = router(state.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/webhooks/linear")
                    .header("linear-signature", sig)
                    .header("user-agent", "BadBot/1.0")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let recent = state.linear_webhook.unwrap().center.recent(1);
        assert!(recent.is_empty());
    }

    #[tokio::test]
    async fn missing_signature_rejected() {
        let secret = b"shh";
        let state = app_state_with_webhook(secret, "TEAM");

        let app = router(state.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/webhooks/linear")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn irrelevant_event_acked_without_notification() {
        let secret = b"shh";
        let state = app_state_with_webhook(secret, "TEAM");
        let body = serde_json::json!({
            "action": "create",
            "type": "Issue",
            "webhookTimestamp": webhook_timestamp(),
            "data": {
                "identifier": "TEAM-1",
                "title": "New issue",
                "state": {"name": "Todo", "type": "unstarted"},
                "team": {"key": "TEAM", "name": "My Team"}
            },
            "updatedFrom": {"stateId": "old"}
        })
        .to_string();
        let sig = sign(secret, body.as_bytes());

        let app = router(state.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/webhooks/linear")
                    .header("linear-signature", sig)
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let recent = state.linear_webhook.unwrap().center.recent(1);
        assert!(recent.is_empty());
    }

    #[tokio::test]
    async fn stale_webhook_replay_rejected() {
        let secret = b"shh";
        let state = app_state_with_webhook(secret, "TEAM");
        let body = serde_json::json!({
            "action": "update",
            "type": "Issue",
            "webhookTimestamp": webhook_timestamp() - 120_000,
            "data": {
                "identifier": "TEAM-1",
                "title": "Fix it",
                "state": {"name": "In Review", "type": "started"},
                "team": {"key": "TEAM", "name": "My Team"}
            },
            "updatedFrom": {"stateId": "old"}
        })
        .to_string();
        let sig = sign(secret, body.as_bytes());

        let app = router(state.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/webhooks/linear")
                    .header("linear-signature", sig)
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let recent = state.linear_webhook.unwrap().center.recent(1);
        assert!(recent.is_empty());
    }

    #[tokio::test]
    async fn missing_webhook_timestamp_rejected() {
        let secret = b"shh";
        let state = app_state_with_webhook(secret, "TEAM");
        let body = serde_json::json!({
            "action": "update",
            "type": "Issue",
            "data": {
                "identifier": "TEAM-1",
                "title": "Fix it",
                "state": {"name": "In Review", "type": "started"},
                "team": {"key": "TEAM", "name": "My Team"}
            },
            "updatedFrom": {"stateId": "old"}
        })
        .to_string();
        let sig = sign(secret, body.as_bytes());

        let app = router(state.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/webhooks/linear")
                    .header("linear-signature", sig)
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let recent = state.linear_webhook.unwrap().center.recent(1);
        assert!(recent.is_empty());
    }

    #[tokio::test]
    async fn route_absent_when_unconfigured() {
        let state = AppState::new(
            Arc::new(niles_core::DeviceRegistry::default()),
            Arc::new(NoopPublisher),
            Arc::new("zigbee2mqtt".into()),
            niles_core::EventBus::new(16),
        );

        let app = router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/webhooks/linear")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
