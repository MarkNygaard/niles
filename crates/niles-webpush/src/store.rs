//! Where the server's key and each person's devices are kept: Postgres
//! in the house (niles-db), in memory for tests.
//!
//! A subscription is keyed by its endpoint, which the push service made
//! for one browser on one device. The same endpoint subscribed again —
//! by someone else signed in on that phone — moves to them: a device
//! notifies whoever turned notifications on there last.

use crate::encrypt;
use crate::error::{Error, Result};
use crate::vapid::b64url_decode;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::sync::Mutex;

/// The longest device name kept; a User-Agent is not a name.
const DEVICE_LEN: usize = 60;

/// Push services' addresses run to a few hundred characters.
const ENDPOINT_LEN: usize = 2048;

/// One browser on one device that said yes to notifications.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Subscription {
    pub endpoint: String,
    /// The browser's public key, base64url. Never shown back.
    #[serde(skip)]
    pub p256dh: String,
    /// The browser's secret, base64url. Never shown back.
    #[serde(skip)]
    pub auth: String,
    /// "iPhone", "Chrome on Android": which one it is, on a list of them.
    pub device: String,
    /// The address the app was open at when it subscribed: the contact
    /// in the token sent with each push ([`crate::VapidKey::authorization`]).
    #[serde(skip)]
    pub origin: String,
    pub created_at: DateTime<Utc>,
    /// When a push service last took a message for it.
    pub last_ok_at: Option<DateTime<Utc>>,
}

/// A subscription as the browser hands it over (`PushSubscription.toJSON()`),
/// with where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSubscription {
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
    pub device: String,
    pub origin: String,
}

impl NewSubscription {
    /// Only what can be pushed to: an https endpoint, and keys that
    /// encrypt. Checked before it is kept, so a bad one is refused when
    /// the person can see why rather than failing quietly later.
    pub fn checked(mut self) -> Result<Self> {
        let invalid = |reason: String| Error::Invalid {
            kind: "push subscription",
            reason,
        };
        if self.endpoint.len() > ENDPOINT_LEN {
            return Err(invalid("its endpoint is too long".into()));
        }
        let url = url::Url::parse(&self.endpoint)
            .map_err(|e| invalid(format!("its endpoint is not a URL: {e}")))?;
        if url.scheme() != "https" || url.host_str().is_none() {
            return Err(invalid("its endpoint is not an https address".into()));
        }
        let p256dh = b64url_decode(&self.p256dh)
            .ok_or_else(|| invalid("its p256dh key is not base64url".into()))?;
        let auth = b64url_decode(&self.auth)
            .ok_or_else(|| invalid("its auth secret is not base64url".into()))?;
        // The keys work if a message encrypts with them.
        encrypt::encrypt(&p256dh, &auth, b"").map_err(|e| invalid(e.to_string()))?;
        self.device = self.device.trim().chars().take(DEVICE_LEN).collect();
        if self.device.is_empty() {
            self.device = "A browser".into();
        }
        Ok(self)
    }
}

/// A short name for the device a User-Agent describes: "iPhone",
/// "Chrome on Android", "Safari on Mac".
///
/// On an iPhone or iPad only the app added to the Home Screen can be
/// notified, and every browser there is Safari underneath, so the
/// device alone says it.
pub fn device_label(user_agent: &str) -> String {
    let ua = user_agent;
    if ua.contains("iPhone") {
        return "iPhone".into();
    }
    if ua.contains("iPad") {
        return "iPad".into();
    }
    let platform = if ua.contains("Android") {
        "Android"
    } else if ua.contains("CrOS") {
        "ChromeOS"
    } else if ua.contains("Windows") {
        "Windows"
    } else if ua.contains("Macintosh") || ua.contains("Mac OS X") {
        "Mac"
    } else if ua.contains("Linux") {
        "Linux"
    } else {
        ""
    };
    let browser = if ua.contains("Edg/") || ua.contains("EdgA/") {
        "Edge"
    } else if ua.contains("Firefox/") {
        "Firefox"
    } else if ua.contains("SamsungBrowser/") {
        "Samsung Internet"
    } else if ua.contains("OPR/") {
        "Opera"
    } else if ua.contains("Chrome/") {
        "Chrome"
    } else if ua.contains("Safari/") {
        "Safari"
    } else {
        ""
    };
    match (browser, platform) {
        ("", "") => "A browser".into(),
        ("", platform) => platform.into(),
        (browser, "") => browser.into(),
        (browser, platform) => format!("{browser} on {platform}"),
    }
}

#[async_trait]
pub trait PushStore: Send + Sync {
    /// The server's private key, base64url, once there is one.
    async fn vapid_key(&self) -> Result<Option<String>>;
    /// Keep `key` unless one is kept already, and answer whichever is:
    /// two replicas making one at once must end up with the same.
    async fn keep_vapid_key(&self, key: &str) -> Result<String>;

    /// Made, or moved to `person` if the endpoint was someone else's.
    async fn subscribe(&self, person: &str, subscription: &NewSubscription) -> Result<()>;
    /// False when `person` had no such subscription.
    async fn unsubscribe(&self, person: &str, endpoint: &str) -> Result<bool>;
    /// Newest first.
    async fn subscriptions(&self, person: &str) -> Result<Vec<Subscription>>;

    /// A push service took a message for it at `at`.
    async fn delivered(&self, endpoint: &str, at: DateTime<Utc>) -> Result<()>;
    /// The push service says it is gone for good: the person turned
    /// notifications off in the browser, or removed the app.
    async fn gone(&self, endpoint: &str) -> Result<()>;
}

#[derive(Default)]
struct Kept {
    key: Option<String>,
    subscriptions: Vec<(String, Subscription)>,
}

/// In memory, for tests.
#[derive(Default)]
pub struct MemoryPush {
    kept: Mutex<Kept>,
}

impl MemoryPush {
    fn kept(&self) -> std::sync::MutexGuard<'_, Kept> {
        self.kept.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[async_trait]
impl PushStore for MemoryPush {
    async fn vapid_key(&self) -> Result<Option<String>> {
        Ok(self.kept().key.clone())
    }

    async fn keep_vapid_key(&self, key: &str) -> Result<String> {
        Ok(self.kept().key.get_or_insert_with(|| key.into()).clone())
    }

    async fn subscribe(&self, person: &str, s: &NewSubscription) -> Result<()> {
        let mut kept = self.kept();
        let (created_at, last_ok_at) = kept
            .subscriptions
            .iter()
            .find(|(_, k)| k.endpoint == s.endpoint)
            .map_or((Utc::now(), None), |(_, k)| (k.created_at, k.last_ok_at));
        kept.subscriptions.retain(|(_, k)| k.endpoint != s.endpoint);
        kept.subscriptions.push((
            person.into(),
            Subscription {
                endpoint: s.endpoint.clone(),
                p256dh: s.p256dh.clone(),
                auth: s.auth.clone(),
                device: s.device.clone(),
                origin: s.origin.clone(),
                created_at,
                last_ok_at,
            },
        ));
        Ok(())
    }

    async fn unsubscribe(&self, person: &str, endpoint: &str) -> Result<bool> {
        let mut kept = self.kept();
        let before = kept.subscriptions.len();
        kept.subscriptions
            .retain(|(p, k)| !(p == person && k.endpoint == endpoint));
        Ok(kept.subscriptions.len() < before)
    }

    async fn subscriptions(&self, person: &str) -> Result<Vec<Subscription>> {
        let mut out: Vec<Subscription> = self
            .kept()
            .subscriptions
            .iter()
            .filter(|(p, _)| p == person)
            .map(|(_, s)| s.clone())
            .collect();
        out.sort_by_key(|s| std::cmp::Reverse(s.created_at));
        Ok(out)
    }

    async fn delivered(&self, endpoint: &str, at: DateTime<Utc>) -> Result<()> {
        for (_, s) in self.kept().subscriptions.iter_mut() {
            if s.endpoint == endpoint {
                s.last_ok_at = Some(at);
            }
        }
        Ok(())
    }

    async fn gone(&self, endpoint: &str) -> Result<()> {
        self.kept()
            .subscriptions
            .retain(|(_, s)| s.endpoint != endpoint);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use p256::elliptic_curve::Generate;
    use p256::elliptic_curve::sec1::ToSec1Point;

    fn browser(endpoint: &str) -> NewSubscription {
        let key = p256::SecretKey::generate();
        NewSubscription {
            endpoint: endpoint.into(),
            p256dh: URL_SAFE_NO_PAD.encode(key.public_key().to_sec1_point(false).as_bytes()),
            auth: URL_SAFE_NO_PAD.encode([9u8; 16]),
            device: "iPhone".into(),
            origin: "https://niles.example".into(),
        }
    }

    #[test]
    fn a_subscription_from_a_browser_is_accepted() {
        assert!(browser("https://web.push.apple.com/QO").checked().is_ok());
    }

    #[test]
    fn an_endpoint_that_is_not_https_is_refused() {
        assert!(browser("http://push.example/x").checked().is_err());
        assert!(browser("not a url").checked().is_err());
    }

    #[test]
    fn keys_that_do_not_encrypt_are_refused() {
        let mut s = browser("https://push.example/x");
        s.auth = URL_SAFE_NO_PAD.encode([1u8; 8]);
        assert!(s.clone().checked().is_err());
        let mut s = browser("https://push.example/x");
        s.p256dh = "@@@".into();
        assert!(s.checked().is_err());
    }

    #[test]
    fn a_long_device_name_is_cut_and_an_empty_one_filled_in() {
        let mut s = browser("https://push.example/x");
        s.device = "x".repeat(500);
        assert_eq!(s.checked().unwrap().device.len(), DEVICE_LEN);
        let mut s = browser("https://push.example/x");
        s.device = "  ".into();
        assert_eq!(s.checked().unwrap().device, "A browser");
    }

    #[test]
    fn devices_are_named_as_a_person_would() {
        let cases = [
            (
                "Mozilla/5.0 (iPhone; CPU iPhone OS 17_4 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148",
                "iPhone",
            ),
            (
                "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/129.0.0.0 Mobile Safari/537.36",
                "Chrome on Android",
            ),
            (
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Safari/605.1.15",
                "Safari on Mac",
            ),
            (
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/129.0.0.0 Safari/537.36 Edg/129.0.0.0",
                "Edge on Windows",
            ),
            (
                "Mozilla/5.0 (X11; Linux x86_64; rv:131.0) Gecko/20100101 Firefox/131.0",
                "Firefox on Linux",
            ),
            ("", "A browser"),
        ];
        for (ua, label) in cases {
            assert_eq!(device_label(ua), label, "{ua}");
        }
    }

    #[tokio::test]
    async fn the_first_key_kept_is_the_key() {
        let store = MemoryPush::default();
        assert_eq!(store.vapid_key().await.unwrap(), None);
        assert_eq!(store.keep_vapid_key("one").await.unwrap(), "one");
        assert_eq!(store.keep_vapid_key("two").await.unwrap(), "one");
    }

    #[tokio::test]
    async fn an_endpoint_subscribed_again_moves_to_whoever_did() {
        let store = MemoryPush::default();
        let s = browser("https://push.example/1");
        store.subscribe("person:mark", &s).await.unwrap();
        store.subscribe("person:anna", &s).await.unwrap();
        assert!(store.subscriptions("person:mark").await.unwrap().is_empty());
        assert_eq!(store.subscriptions("person:anna").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn only_your_own_device_can_be_removed() {
        let store = MemoryPush::default();
        let s = browser("https://push.example/1");
        store.subscribe("person:mark", &s).await.unwrap();
        assert!(!store.unsubscribe("person:anna", &s.endpoint).await.unwrap());
        assert!(store.unsubscribe("person:mark", &s.endpoint).await.unwrap());
        assert!(store.subscriptions("person:mark").await.unwrap().is_empty());
    }

    #[test]
    fn the_keys_are_never_shown_back() {
        let s = browser("https://push.example/1");
        let shown = serde_json::to_string(&Subscription {
            endpoint: s.endpoint.clone(),
            p256dh: s.p256dh.clone(),
            auth: s.auth.clone(),
            device: s.device.clone(),
            origin: s.origin.clone(),
            created_at: Utc::now(),
            last_ok_at: None,
        })
        .unwrap();
        assert!(
            !shown.contains(&s.p256dh) && !shown.contains(&s.auth),
            "{shown}"
        );
    }
}
