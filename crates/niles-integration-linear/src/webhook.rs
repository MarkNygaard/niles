//! Linear webhook payload parsing, signature verification, and notification mapping.

use crate::error::{Error, Result};
use serde::Deserialize;

/// Verify the HMAC-SHA256 signature of a webhook body.
pub fn verify_signature(secret: &[u8], body: &[u8], signature_hex: &str) -> bool {
    type HmacSha256 = hmac::Hmac<sha2::Sha256>;
    let Ok(sig) = hex::decode(signature_hex) else {
        return false;
    };
    let Ok(mut mac) = <HmacSha256 as hmac::digest::KeyInit>::new_from_slice(secret) else {
        return false;
    };
    hmac::Mac::update(&mut mac, body);
    hmac::Mac::verify_slice(mac, &sig).is_ok()
}

/// Typed subset of a Linear webhook payload.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebhookPayload {
    pub action: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub webhook_timestamp: Option<i64>,
    #[serde(default)]
    pub data: Option<IssueData>,
    #[serde(default)]
    pub updated_from: Option<serde_json::Value>,
}

/// Issue data nested inside a webhook payload.
#[derive(Debug, Clone, Deserialize)]
pub struct IssueData {
    #[serde(default)]
    pub identifier: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub state: Option<IssueState>,
    #[serde(default)]
    pub team: Option<TeamRef>,
}

/// Workflow state of an issue.
#[derive(Debug, Clone, Deserialize)]
pub struct IssueState {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
}

/// Team reference inside issue data.
#[derive(Debug, Clone, Deserialize)]
pub struct TeamRef {
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

/// Mapped notification text extracted from a webhook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebhookNotification {
    pub text: String,
}

/// Parse a raw JSON webhook body.
pub fn parse_webhook(body: &[u8]) -> Result<WebhookPayload> {
    serde_json::from_slice(body).map_err(|e| Error::Parse {
        reason: e.to_string(),
    })
}

/// Map a verified webhook payload to a notification text, if relevant.
pub fn notification_for(p: &WebhookPayload, team: &str) -> Option<WebhookNotification> {
    if p.action != "update" || p.kind != "Issue" {
        return None;
    }
    let data = p.data.as_ref()?;
    if let Some(t) = &data.team {
        if t.key.as_deref() != Some(team) && t.name.as_deref() != Some(team) {
            return None;
        }
    } else {
        return None;
    }
    let updated = p.updated_from.as_ref()?;
    updated.get("stateId")?;
    let state = data.state.as_ref()?;
    let title = data.title.as_deref().unwrap_or("issue");
    let text = match state.kind.as_str() {
        "started" => {
            if state.name.to_lowercase().contains("review") {
                format!("The {title} task has a pull request ready for review.")
            } else {
                format!(
                    "The {title} task is now {state_name}.",
                    state_name = state.name
                )
            }
        }
        "completed" => format!("The {title} task is done."),
        _ => return None,
    };
    Some(WebhookNotification { text })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_signature_accepts_valid() {
        let secret = b"my-secret";
        let body = b"hello world";
        type HmacSha256 = hmac::Hmac<sha2::Sha256>;
        let mut mac = <HmacSha256 as hmac::digest::KeyInit>::new_from_slice(secret).unwrap();
        hmac::Mac::update(&mut mac, body);
        let sig = hex::encode(hmac::Mac::finalize(mac).into_bytes());
        assert!(verify_signature(secret, body, &sig));
    }

    #[test]
    fn verify_signature_rejects_tampered_body() {
        let secret = b"my-secret";
        let body = b"hello world";
        type HmacSha256 = hmac::Hmac<sha2::Sha256>;
        let mut mac = <HmacSha256 as hmac::digest::KeyInit>::new_from_slice(secret).unwrap();
        hmac::Mac::update(&mut mac, body);
        let sig = hex::encode(hmac::Mac::finalize(mac).into_bytes());
        assert!(!verify_signature(secret, b"tampered", &sig));
    }

    #[test]
    fn verify_signature_rejects_bad_signature() {
        assert!(!verify_signature(b"secret", b"body", "not-hex"));
        assert!(!verify_signature(b"secret", b"body", ""));
    }

    fn make_update_payload(state_kind: &str, state_name: &str, team_key: &str) -> WebhookPayload {
        WebhookPayload {
            action: "update".into(),
            kind: "Issue".into(),
            webhook_timestamp: Some(1_767_644_800_000),
            data: Some(IssueData {
                identifier: Some("TEAM-123".into()),
                title: Some("Fix bug".into()),
                state: Some(IssueState {
                    name: state_name.into(),
                    kind: state_kind.into(),
                }),
                team: Some(TeamRef {
                    key: Some(team_key.into()),
                    name: Some("My Team".into()),
                }),
            }),
            updated_from: Some(serde_json::json!({"stateId": "old-state"})),
        }
    }

    #[test]
    fn notification_for_in_review() {
        let p = make_update_payload("started", "In Review", "TEAM");
        let n = notification_for(&p, "TEAM").unwrap();
        assert!(n.text.contains("pull request ready"));
    }

    #[test]
    fn notification_for_done() {
        let p = make_update_payload("completed", "Done", "TEAM");
        let n = notification_for(&p, "TEAM").unwrap();
        assert_eq!(n.text, "The Fix bug task is done.");
    }

    #[test]
    fn notification_for_ignores_create() {
        let mut p = make_update_payload("started", "In Progress", "TEAM");
        p.action = "create".into();
        assert!(notification_for(&p, "TEAM").is_none());
    }

    #[test]
    fn notification_for_ignores_non_state_update() {
        let mut p = make_update_payload("started", "In Progress", "TEAM");
        p.updated_from = Some(serde_json::json!({"title": "old title"}));
        assert!(notification_for(&p, "TEAM").is_none());
    }

    #[test]
    fn notification_for_ignores_other_team() {
        let p = make_update_payload("started", "In Progress", "OTHER");
        assert!(notification_for(&p, "TEAM").is_none());
    }

    #[test]
    fn parse_webhook_tolerates_extra_fields() {
        let json = serde_json::json!({
            "action": "update",
            "type": "Issue",
            "data": {
                "identifier": "TEAM-42",
                "title": "Do thing",
                "state": {"name": "In Review", "type": "started"},
                "team": {"key": "TEAM", "name": "My Team"}
            },
            "updatedFrom": {"stateId": "old"},
            "url": "https://linear.app/issue/TEAM-42",
            "createdAt": "2026-01-01T00:00:00.000Z",
            "webhookId": "wh-123"
        });
        let payload = parse_webhook(json.to_string().as_bytes()).unwrap();
        assert_eq!(payload.action, "update");
        assert_eq!(payload.kind, "Issue");
        assert!(payload.webhook_timestamp.is_none());
        let data = payload.data.unwrap();
        assert_eq!(data.title, Some("Do thing".into()));
        let state = data.state.unwrap();
        assert_eq!(state.kind, "started");
    }
}
