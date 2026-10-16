//! Sending: one message to every device a person has.

use crate::encrypt;
use crate::error::{Error, Result};
use crate::store::{PushStore, Subscription};
use crate::vapid::{VapidKey, b64url_decode};
use chrono::Utc;
use serde::Serialize;
use std::sync::Arc;

/// How long a push service holds a message for a device that is off.
/// A day: "your plan is ready" is still news the next morning, and not
/// much after.
pub const TTL: u32 = 24 * 60 * 60;

/// What the service worker shows (web/public/sw.js).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Message {
    pub title: String,
    pub body: String,
    /// Where tapping it opens the app.
    pub url: String,
    /// A newer notification with the same tag replaces the older one
    /// instead of stacking under it.
    pub tag: String,
}

/// What became of one message.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Delivery {
    /// Taken by the push service for a device.
    pub sent: usize,
    /// Gone for good, and forgotten.
    pub gone: usize,
    /// Refused or unreachable this time; kept, to try next time.
    pub failed: usize,
}

pub struct WebPush {
    store: Arc<dyn PushStore>,
    http: reqwest::Client,
    key: tokio::sync::OnceCell<VapidKey>,
}

impl WebPush {
    pub fn new(store: Arc<dyn PushStore>, http: reqwest::Client) -> Self {
        Self {
            store,
            http,
            key: tokio::sync::OnceCell::new(),
        }
    }

    pub fn store(&self) -> &Arc<dyn PushStore> {
        &self.store
    }

    /// The server's key: the one kept, or a new one kept now.
    async fn key(&self) -> Result<&VapidKey> {
        self.key
            .get_or_try_init(|| async {
                let kept = match self.store.vapid_key().await? {
                    Some(kept) => kept,
                    None => {
                        let fresh = VapidKey::generate().to_base64url();
                        self.store.keep_vapid_key(&fresh).await?
                    }
                };
                VapidKey::from_base64url(&kept)
            })
            .await
    }

    /// The public key a browser subscribes with, base64url.
    pub async fn public_key(&self) -> Result<String> {
        Ok(self.key().await?.public_key())
    }

    /// `message` to each of `person`'s devices. A device that fails
    /// does not stop the others, and is not an error: only not being
    /// able to look them up is.
    pub async fn send(&self, person: &str, message: &Message) -> Result<Delivery> {
        let payload = serde_json::to_vec(message).map_err(|e| Error::Invalid {
            kind: "push message",
            reason: e.to_string(),
        })?;
        if payload.len() > encrypt::MAX_PLAINTEXT {
            return Err(Error::Invalid {
                kind: "push message",
                reason: format!("it is {} bytes", payload.len()),
            });
        }
        let subscriptions = self.store.subscriptions(person).await?;
        let mut delivery = Delivery::default();
        if subscriptions.is_empty() {
            return Ok(delivery);
        }
        let key = self.key().await?;
        for subscription in &subscriptions {
            match self.deliver(key, subscription, &payload).await {
                Outcome::Sent => {
                    delivery.sent += 1;
                    if let Err(e) = self
                        .store
                        .delivered(&subscription.endpoint, Utc::now())
                        .await
                    {
                        tracing::warn!("[push] could not note a delivery: {e}");
                    }
                }
                Outcome::Gone => {
                    delivery.gone += 1;
                    tracing::info!(
                        "[push] {person}: {} is no longer subscribed; forgotten",
                        subscription.device
                    );
                    if let Err(e) = self.store.gone(&subscription.endpoint).await {
                        tracing::warn!("[push] could not forget a device: {e}");
                    }
                }
                Outcome::Failed(why) => {
                    delivery.failed += 1;
                    tracing::warn!(
                        "[push] {person}: {} at {}: {why}",
                        subscription.device,
                        host_of(&subscription.endpoint)
                    );
                }
            }
        }
        Ok(delivery)
    }

    async fn deliver(&self, key: &VapidKey, s: &Subscription, payload: &[u8]) -> Outcome {
        let (Some(p256dh), Some(auth)) = (b64url_decode(&s.p256dh), b64url_decode(&s.auth)) else {
            return Outcome::Failed("its keys are not base64url".into());
        };
        let body = match encrypt::encrypt(&p256dh, &auth, payload) {
            Ok(body) => body,
            Err(e) => return Outcome::Failed(e.to_string()),
        };
        let authorization = match key.authorization(&s.endpoint, &s.origin, Utc::now()) {
            Ok(header) => header,
            Err(e) => return Outcome::Failed(e.to_string()),
        };
        let response = self
            .http
            .post(&s.endpoint)
            .header("TTL", TTL.to_string())
            .header("Urgency", "normal")
            .header("Content-Encoding", "aes128gcm")
            .header("Content-Type", "application/octet-stream")
            .header("Authorization", authorization)
            .body(body)
            .send()
            .await;
        let response = match response {
            Ok(response) => response,
            Err(e) => return Outcome::Failed(format!("not reached: {e}")),
        };
        let status = response.status();
        if status.is_success() {
            return Outcome::Sent;
        }
        // 404 and 410 are the push service saying the subscription has
        // expired or was dropped (RFC 8030 § 7.3): it will never work
        // again.
        if status.as_u16() == 404 || status.as_u16() == 410 {
            return Outcome::Gone;
        }
        let reason = response.text().await.unwrap_or_default();
        let reason: String = reason.trim().chars().take(200).collect();
        Outcome::Failed(format!("{status} {reason}").trim().to_string())
    }
}

enum Outcome {
    Sent,
    Gone,
    Failed(String),
}

/// The push service, for a log line: the endpoint itself is the key to
/// a person's device, and stays out of the logs.
fn host_of(endpoint: &str) -> String {
    url::Url::parse(endpoint)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_else(|| "a push service".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{MemoryPush, NewSubscription};
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use p256::elliptic_curve::Generate;
    use p256::elliptic_curve::sec1::ToSec1Point;
    use wiremock::matchers::{header, header_exists, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn message() -> Message {
        Message {
            title: "Niles".into(),
            body: "Your Thailand plan is ready".into(),
            url: "/#/chat?artifact=7".into(),
            tag: "artifact-7".into(),
        }
    }

    async fn subscribed(store: &MemoryPush, person: &str, endpoint: String) {
        let key = p256::SecretKey::generate();
        store
            .subscribe(
                person,
                &NewSubscription {
                    endpoint,
                    p256dh: URL_SAFE_NO_PAD
                        .encode(key.public_key().to_sec1_point(false).as_bytes()),
                    auth: URL_SAFE_NO_PAD.encode([3u8; 16]),
                    device: "iPhone".into(),
                    origin: "https://niles.example".into(),
                },
            )
            .await
            .unwrap();
    }

    fn push(store: Arc<MemoryPush>) -> WebPush {
        WebPush::new(store, reqwest::Client::new())
    }

    #[tokio::test]
    async fn the_key_is_made_once_and_kept() {
        let store = Arc::new(MemoryPush::default());
        let first = push(store.clone()).public_key().await.unwrap();
        let again = push(store.clone()).public_key().await.unwrap();
        assert_eq!(first, again, "a second start reads the kept key");
        assert!(store.vapid_key().await.unwrap().is_some());
    }

    #[tokio::test]
    async fn a_push_is_encrypted_signed_and_sent_with_its_headers() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/push/1"))
            .and(header("content-encoding", "aes128gcm"))
            .and(header("ttl", TTL.to_string().as_str()))
            .and(header("urgency", "normal"))
            .and(header_exists("authorization"))
            .respond_with(ResponseTemplate::new(201))
            .expect(1)
            .mount(&server)
            .await;
        let store = Arc::new(MemoryPush::default());
        subscribed(&store, "person:mark", format!("{}/push/1", server.uri())).await;

        let delivery = push(store.clone())
            .send("person:mark", &message())
            .await
            .unwrap();
        assert_eq!(
            delivery,
            Delivery {
                sent: 1,
                ..Delivery::default()
            }
        );
        let kept = store.subscriptions("person:mark").await.unwrap();
        assert!(kept[0].last_ok_at.is_some());

        let request = &server.received_requests().await.unwrap()[0];
        let authorization = request.headers["authorization"].to_str().unwrap();
        assert!(authorization.starts_with("vapid t="), "{authorization}");
        // Salt, record size 4096, a 65-byte key id.
        assert_eq!(&request.body[16..21], &[0, 0, 16, 0, 65]);
    }

    #[tokio::test]
    async fn a_device_the_push_service_says_is_gone_is_forgotten() {
        let server = MockServer::start().await;
        Mock::given(path("/gone"))
            .respond_with(ResponseTemplate::new(410))
            .mount(&server)
            .await;
        Mock::given(path("/missing"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let store = Arc::new(MemoryPush::default());
        subscribed(&store, "person:mark", format!("{}/gone", server.uri())).await;
        subscribed(&store, "person:mark", format!("{}/missing", server.uri())).await;

        let delivery = push(store.clone())
            .send("person:mark", &message())
            .await
            .unwrap();
        assert_eq!(delivery.gone, 2);
        assert!(store.subscriptions("person:mark").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_refusal_is_counted_and_the_device_kept_and_the_others_still_get_it() {
        let server = MockServer::start().await;
        Mock::given(path("/refuses"))
            .respond_with(
                ResponseTemplate::new(403).set_body_string("{\"reason\":\"BadJwtToken\"}"),
            )
            .mount(&server)
            .await;
        Mock::given(path("/works"))
            .respond_with(ResponseTemplate::new(201))
            .mount(&server)
            .await;
        let store = Arc::new(MemoryPush::default());
        subscribed(&store, "person:mark", format!("{}/refuses", server.uri())).await;
        subscribed(&store, "person:mark", format!("{}/works", server.uri())).await;

        let delivery = push(store.clone())
            .send("person:mark", &message())
            .await
            .unwrap();
        assert_eq!(delivery.sent, 1);
        assert_eq!(delivery.failed, 1);
        assert_eq!(store.subscriptions("person:mark").await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn an_unreachable_push_service_is_a_failure_not_an_error() {
        let store = Arc::new(MemoryPush::default());
        subscribed(&store, "person:mark", "http://127.0.0.1:9/push".into()).await;
        let delivery = push(store).send("person:mark", &message()).await.unwrap();
        assert_eq!(delivery.failed, 1);
    }

    #[tokio::test]
    async fn only_the_persons_own_devices_are_sent_to() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(201))
            .expect(1)
            .mount(&server)
            .await;
        let store = Arc::new(MemoryPush::default());
        subscribed(&store, "person:mark", format!("{}/mark", server.uri())).await;
        subscribed(&store, "person:anna", format!("{}/anna", server.uri())).await;
        push(store).send("person:mark", &message()).await.unwrap();
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests[0].url.path(), "/mark");
    }

    #[tokio::test]
    async fn nobody_subscribed_is_nothing_sent_and_no_key_made() {
        let store = Arc::new(MemoryPush::default());
        let delivery = push(store.clone())
            .send("person:mark", &message())
            .await
            .unwrap();
        assert_eq!(delivery, Delivery::default());
        assert!(store.vapid_key().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_message_too_big_to_push_is_an_error() {
        let store = Arc::new(MemoryPush::default());
        let mut big = message();
        big.body = "x".repeat(encrypt::MAX_PLAINTEXT);
        assert!(push(store).send("person:mark", &big).await.is_err());
    }
}
