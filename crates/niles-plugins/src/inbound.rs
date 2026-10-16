//! Requests from outside, for a plugin (interface 0.3): its webhook, and
//! the browser coming back from a sign-in page.
//!
//! The rules are ARCHITECTURE.md's "Accounts and webhooks". Niles owns
//! what is the same for every plugin: the webhook's token and the limits
//! around it here, and the OAuth `state` in `niles-api`. The plugin owns
//! what differs between services: the sign-in address, the code
//! exchange, and what a request means.

use crate::policy::HttpPolicy;
use crate::runtime::House;
use crate::store::Stored;
use rand::TryRng;
use std::time::Instant;

/// Where the webhook's token is kept, with Niles's own storage keys.
pub const HOOK_TOKEN_KEY: &str = ".niles/hook-token";

/// The largest request body.
pub const MAX_REQUEST: usize = 64 * 1024;
/// The most headers, and their bytes together.
pub const MAX_HEADERS: usize = 32;
pub const MAX_HEADER_BYTES: usize = 8 * 1024;
/// The largest answer body.
pub const MAX_ANSWER: usize = 64 * 1024;
/// Requests a minute, and how many may come at once.
pub const PER_MINUTE: f64 = 60.0;
pub const BURST: f64 = 20.0;
/// Requests waiting for the plugin at once.
pub const WAITING: usize = 8;

/// A request to a plugin's webhook, as the API received it.
#[derive(Debug, Clone, Default)]
pub struct HookRequest {
    pub method: String,
    pub query: Vec<(String, String)>,
    /// Lowercase names, `cookie` already taken out.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// What the webhook answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookAnswer {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

impl HookAnswer {
    /// An answer with nothing in it.
    pub fn empty(status: u16) -> Self {
        Self {
            status,
            content_type: None,
            body: Vec::new(),
        }
    }
}

/// A new webhook token: 32 bytes from the operating system, as hex, like
/// the API key, so it survives being pasted anywhere.
pub fn new_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::SysRng
        .try_fill_bytes(&mut bytes)
        .expect("the operating system's random number generator failed");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Whether `given` is `token`, taking as long whatever it is.
pub fn same_token(given: &str, token: &str) -> bool {
    let (a, b) = (given.as_bytes(), token.as_bytes());
    let mut differ = a.len() ^ b.len();
    for i in 0..a.len().max(b.len()) {
        differ |= usize::from(a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0));
    }
    differ == 0
}

/// `public` with the plugin's webhook path on the end.
pub fn hook_url(public: &str, plugin: &str, token: &str) -> String {
    format!(
        "{}/plugins/{plugin}/hook/{token}",
        public.trim_end_matches('/')
    )
}

/// `public` with the plugin's callback path on the end.
pub fn callback_url(public: &str, plugin: &str) -> String {
    format!("{}/plugins/{plugin}/callback", public.trim_end_matches('/'))
}

/// Whether the browser may be sent to `url`, the sign-in page a plugin
/// named: `https`, on a host its `http` permission declares, with no
/// credentials in it.
pub fn check_sign_in_page(url: &str, policy: &HttpPolicy) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|e| format!("{url:?} is not an address: {e}"))?;
    if parsed.scheme() != "https" {
        return Err(format!("{url:?} is not https"));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("the sign-in address has a username or password in it".into());
    }
    match parsed.host_str() {
        Some(host) if policy.declares_host(host) => Ok(()),
        Some(host) => Err(format!(
            "{host} is not one of the hosts the manifest's http permission declares"
        )),
        None => Err(format!("{url:?} has no host")),
    }
}

/// Whether an answer can go back to the caller as the plugin gave it.
pub fn check_answer(answer: &HookAnswer) -> Result<(), String> {
    if !(200..=599).contains(&answer.status) {
        return Err(format!("the status {} is not 200 to 599", answer.status));
    }
    if answer.body.len() > MAX_ANSWER {
        return Err(format!(
            "the answer is {} KiB; at most {} KiB",
            answer.body.len().div_ceil(1024),
            MAX_ANSWER / 1024
        ));
    }
    if let Some(kind) = &answer.content_type
        && (kind.is_empty() || kind.len() > 128 || kind.chars().any(|c| c.is_control()))
    {
        return Err(format!("the content type {kind:?} is not one"));
    }
    Ok(())
}

/// `plugin`'s webhook token, made now if it has none and `make` says so.
pub(crate) async fn token(
    house: &House,
    plugin: &str,
    make: bool,
) -> Result<Option<String>, String> {
    let _held = house.hook_tokens.lock().await;
    if let Some(token) = house.store.get(plugin, HOOK_TOKEN_KEY).await? {
        return Ok(Some(token));
    }
    if !make {
        return Ok(None);
    }
    keep(house, plugin, new_token()).await.map(Some)
}

/// A new token for `plugin`, replacing the old one, which stops working.
pub(crate) async fn rotate(house: &House, plugin: &str) -> Result<String, String> {
    let _held = house.hook_tokens.lock().await;
    keep(house, plugin, new_token()).await
}

async fn keep(house: &House, plugin: &str, token: String) -> Result<String, String> {
    match house.store.set(plugin, HOOK_TOKEN_KEY, &token).await? {
        Stored::Yes => Ok(token),
        Stored::OverQuota => Err("its storage is full, so its webhook has no address".into()),
    }
}

/// A token bucket: [`BURST`] at once, refilled at [`PER_MINUTE`].
#[derive(Debug)]
pub(crate) struct Bucket {
    tokens: f64,
    at: Instant,
}

impl Default for Bucket {
    fn default() -> Self {
        Self {
            tokens: BURST,
            at: Instant::now(),
        }
    }
}

impl Bucket {
    /// Take one, if there is one.
    pub fn take(&mut self) -> bool {
        let now = Instant::now();
        let refill = now.duration_since(self.at).as_secs_f64() * PER_MINUTE / 60.0;
        self.tokens = (self.tokens + refill).min(BURST);
        self.at = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// The last request the webhook had, for the plugin's panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastHook {
    pub at: chrono::DateTime<chrono::Utc>,
    pub status: u16,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Permissions;

    #[test]
    fn a_token_is_64_hex_digits_and_never_the_same_twice() {
        let a = new_token();
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, new_token());
    }

    #[test]
    fn tokens_are_compared_whole() {
        let token = new_token();
        assert!(same_token(&token, &token));
        assert!(!same_token(&token[..63], &token));
        assert!(!same_token(&format!("{token}0"), &token));
        assert!(!same_token("", &token));
    }

    #[test]
    fn the_addresses_are_built_on_the_public_one() {
        assert_eq!(
            hook_url("https://home.example.dk/", "withings", "ab"),
            "https://home.example.dk/plugins/withings/hook/ab"
        );
        assert_eq!(
            callback_url("https://home.example.dk", "withings"),
            "https://home.example.dk/plugins/withings/callback"
        );
    }

    fn policy() -> HttpPolicy {
        let permissions = Permissions {
            http: vec!["account.withings.com".into()],
            ..Default::default()
        };
        HttpPolicy::new(&permissions, |_| None)
    }

    #[test]
    fn the_browser_goes_only_to_a_declared_https_host() {
        let ok = "https://account.withings.com/oauth2_user/authorize2?state=x";
        assert!(check_sign_in_page(ok, &policy()).is_ok());
        for bad in [
            "http://account.withings.com/oauth2_user/authorize2",
            "https://evil.example.com/authorize",
            "https://user:pw@account.withings.com/",
            "javascript:alert(1)",
            "not a url",
        ] {
            assert!(check_sign_in_page(bad, &policy()).is_err(), "{bad}");
        }
    }

    #[test]
    fn an_answer_has_a_real_status_and_a_small_body() {
        assert!(check_answer(&HookAnswer::empty(200)).is_ok());
        assert!(check_answer(&HookAnswer::empty(503)).is_ok());
        assert!(check_answer(&HookAnswer::empty(199)).is_err());
        assert!(check_answer(&HookAnswer::empty(600)).is_err());
        let big = HookAnswer {
            status: 200,
            content_type: None,
            body: vec![b'x'; MAX_ANSWER + 1],
        };
        assert!(check_answer(&big).is_err());
        let header_injection = HookAnswer {
            status: 200,
            content_type: Some("text/plain\r\nset-cookie: x=1".into()),
            body: Vec::new(),
        };
        assert!(check_answer(&header_injection).is_err());
    }

    #[test]
    fn the_bucket_allows_a_burst_then_refuses() {
        let mut bucket = Bucket::default();
        for _ in 0..BURST as usize {
            assert!(bucket.take());
        }
        assert!(!bucket.take());
    }
}
