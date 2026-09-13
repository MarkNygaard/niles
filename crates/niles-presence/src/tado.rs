//! Tado HTTP adapter — mobileDevices over the OAuth2 device code flow.
//!
//! The password grant this used to speak was removed from
//! `auth.tado.com` on 15 March 2025, so there is no longer any way to
//! trade a username and a password for a token. What replaced it needs
//! a person and a browser **once**: Niles asks tado for a code,
//! somebody approves it, and a refresh token carries the session from
//! then on.

use crate::error::{Error, Result};
use crate::source::PresenceSource;
use crate::state::PresenceSignal;
use crate::tokens::TokenStore;
use crate::transport::{HttpTadoTransport, TadoTransport};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// The client id tado publishes for the device flow.
///
/// Not a secret, and not a placeholder: the device flow has no client
/// secret, which is the reason it replaced the password grant.
pub const DEVICE_CLIENT_ID: &str = "1bb50063-6b0c-4d11-bd99-387f4a91cc46";

/// Runtime configuration for the Tado source.
#[derive(Debug, Clone)]
pub struct TadoConfig {
    /// Discovered from `/api/v2/me` when absent, so nobody has to go
    /// looking for it.
    pub home_id: Option<u64>,
    pub base_url: String,
    pub auth_url: String,
    pub client_id: String,
    pub request_timeout: Duration,
}

impl Default for TadoConfig {
    fn default() -> Self {
        Self {
            home_id: None,
            base_url: "https://my.tado.com".into(),
            auth_url: "https://login.tado.com/oauth2".into(),
            client_id: DEVICE_CLIENT_ID.into(),
            request_timeout: Duration::from_secs(30),
        }
    }
}

/// A pending device authorisation: what to show a person, and what to
/// poll with while they deal with it.
#[derive(Debug, Clone)]
pub struct DeviceActivation {
    /// The URL to open. It already carries the code as a query
    /// parameter, so following it is the whole job.
    pub verification_uri: String,
    /// The code shown on that page, so somebody can check it matches.
    pub user_code: String,
    pub device_code: String,
    pub expires_at: DateTime<Utc>,
    /// How often tado is willing to be asked whether it has been
    /// approved yet.
    pub interval: Duration,
}

struct CachedToken {
    access_token: String,
    expires_at: DateTime<Utc>,
}

/// Tado presence source.
pub struct TadoSource {
    transport: Arc<dyn TadoTransport>,
    tokens: Arc<dyn TokenStore>,
    cfg: TadoConfig,
    token: Mutex<Option<CachedToken>>,
    home_id: Mutex<Option<u64>>,
    /// The activation currently being waited on, shared so the code a
    /// person is shown is the same one something is polling for. Two
    /// device codes in flight means approving one and watching the
    /// other stay pending.
    pending: Mutex<Option<DeviceActivation>>,
}

impl TadoSource {
    pub fn new(cfg: TadoConfig, tokens: Arc<dyn TokenStore>) -> Result<Self> {
        let transport = Arc::new(HttpTadoTransport::new(cfg.request_timeout)?);
        Ok(Self::with_transport(cfg, tokens, transport))
    }

    pub fn with_transport(
        cfg: TadoConfig,
        tokens: Arc<dyn TokenStore>,
        transport: Arc<dyn TadoTransport>,
    ) -> Self {
        let home_id = Mutex::new(cfg.home_id);
        Self {
            transport,
            tokens,
            cfg,
            token: Mutex::new(None),
            home_id,
            pending: Mutex::new(None),
        }
    }

    /// Whether anybody has authorised this Niles yet.
    pub async fn is_authorised(&self) -> Result<bool> {
        Ok(self.tokens.load().await?.is_some())
    }

    /// The activation being waited on, if there is one. Does not start
    /// one — a page being looked at should not spend tado's quota.
    pub async fn pending_activation(&self) -> Option<DeviceActivation> {
        let pending = self.pending.lock().await;
        pending
            .as_ref()
            .filter(|p| Utc::now() < p.expires_at)
            .cloned()
    }

    /// The activation to show somebody, starting one if the last has
    /// expired or there has never been one.
    pub async fn ensure_activation(&self) -> Result<DeviceActivation> {
        if let Some(live) = self.pending_activation().await {
            return Ok(live);
        }
        let fresh = self.begin_activation().await?;
        *self.pending.lock().await = Some(fresh.clone());
        Ok(fresh)
    }

    /// Forget the pending activation — approved, or past saving.
    pub async fn clear_activation(&self) {
        *self.pending.lock().await = None;
    }

    /// Ask tado to start a device authorisation.
    ///
    /// Nothing is stored yet — [`finish_activation`](Self::finish_activation)
    /// does that once a person has approved it.
    pub async fn begin_activation(&self) -> Result<DeviceActivation> {
        let url = format!(
            "{}/device_authorize",
            self.cfg.auth_url.trim_end_matches('/')
        );
        let form = [
            ("client_id", self.cfg.client_id.as_str()),
            ("scope", "offline_access"),
        ];
        let (status, body) = self.transport.post_form(&url, &form).await?;
        if !(200..300).contains(&status) {
            return Err(Error::BadStatus { status, body });
        }
        let resp: DeviceAuthResponse = serde_json::from_str(&body).map_err(|e| Error::Parse {
            reason: format!("device authorize: {e}"),
        })?;
        Ok(DeviceActivation {
            verification_uri: resp.verification_uri_complete,
            user_code: resp.user_code,
            device_code: resp.device_code,
            expires_at: Utc::now() + chrono::Duration::seconds(resp.expires_in as i64),
            interval: Duration::from_secs(resp.interval.max(1)),
        })
    }

    /// Ask once whether a pending activation has been approved.
    ///
    /// `Ok(false)` means "not yet, ask again after `interval`", which is
    /// a normal answer rather than a failure — tado reports it as a 400
    /// carrying `authorization_pending`. Everything else is an error,
    /// including the code expiring unapproved.
    pub async fn finish_activation(&self, pending: &DeviceActivation) -> Result<bool> {
        let url = format!("{}/token", self.cfg.auth_url.trim_end_matches('/'));
        let form = [
            ("client_id", self.cfg.client_id.as_str()),
            ("device_code", pending.device_code.as_str()),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ];
        let (status, body) = self.transport.post_form(&url, &form).await?;

        if status == 400 || status == 428 {
            let err: OAuthError = serde_json::from_str(&body).unwrap_or_default();
            return match err.error.as_deref() {
                Some("authorization_pending") | Some("slow_down") => Ok(false),
                Some("expired_token") => Err(Error::Auth {
                    reason: "the code expired before it was approved".into(),
                }),
                Some("access_denied") => Err(Error::Auth {
                    reason: "the request was declined".into(),
                }),
                _ => Err(Error::BadStatus { status, body }),
            };
        }
        if !(200..300).contains(&status) {
            return Err(Error::BadStatus { status, body });
        }
        self.accept_tokens(&body).await?;
        Ok(true)
    }

    /// Take a token response: access token in memory, refresh token on
    /// disk.
    async fn accept_tokens(&self, body: &str) -> Result<()> {
        let resp: TokenResponse = serde_json::from_str(body).map_err(|e| Error::Parse {
            reason: format!("token response: {e}"),
        })?;
        if resp.expires_in == 0 {
            return Err(Error::Parse {
                reason: "expires_in is zero or missing".into(),
            });
        }
        let Some(refresh) = resp.refresh_token.as_deref() else {
            return Err(Error::Parse {
                reason: "no refresh_token — was offline_access requested?".into(),
            });
        };
        // Saved before the access token is cached, so a crash in
        // between leaves a refresh token that still works rather than
        // one already spent.
        self.tokens.save(refresh).await?;
        *self.token.lock().await = Some(CachedToken {
            access_token: resp.access_token,
            expires_at: Utc::now() + chrono::Duration::seconds(resp.expires_in as i64),
        });
        Ok(())
    }

    async fn ensure_token(&self) -> Result<String> {
        {
            let cache = self.token.lock().await;
            if let Some(token) = cache.as_ref()
                && token.expires_at > Utc::now() + chrono::Duration::seconds(30)
            {
                return Ok(token.access_token.clone());
            }
        }

        let Some(refresh) = self.tokens.load().await? else {
            return Err(Error::Auth {
                reason: "not authorised with tado yet — approve the device code".into(),
            });
        };

        let url = format!("{}/token", self.cfg.auth_url.trim_end_matches('/'));
        let form = [
            ("client_id", self.cfg.client_id.as_str()),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh.as_str()),
        ];
        let (status, body) = self.transport.post_form(&url, &form).await?;

        if status == 400 || status == 401 {
            return Err(Error::Auth {
                reason: "the stored refresh token was refused — authorise again".into(),
            });
        }
        if !(200..300).contains(&status) {
            return Err(Error::BadStatus { status, body });
        }
        self.accept_tokens(&body).await?;
        let cache = self.token.lock().await;
        Ok(cache.as_ref().expect("just stored").access_token.clone())
    }

    /// The home to ask about, discovered on first use when the config
    /// did not name one. Cached: an account's homes do not move.
    async fn home_id(&self, token: &str) -> Result<u64> {
        if let Some(id) = *self.home_id.lock().await {
            return Ok(id);
        }
        let url = format!("{}/api/v2/me", self.cfg.base_url.trim_end_matches('/'));
        let (status, body) = self.transport.get_bearer(&url, token).await?;
        if !(200..300).contains(&status) {
            return Err(Error::BadStatus { status, body });
        }
        let me: Me = serde_json::from_str(&body).map_err(|e| Error::Parse {
            reason: format!("me: {e}"),
        })?;
        let id = me.homes.first().map(|h| h.id).ok_or_else(|| Error::Parse {
            reason: "this tado account has no homes".into(),
        })?;
        *self.home_id.lock().await = Some(id);
        Ok(id)
    }
}

#[derive(Debug, Deserialize)]
struct DeviceAuthResponse {
    device_code: String,
    user_code: String,
    verification_uri_complete: String,
    expires_in: u64,
    #[serde(default = "default_interval")]
    interval: u64,
}

fn default_interval() -> u64 {
    5
}

#[derive(Debug, Default, Deserialize)]
struct OAuthError {
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
    refresh_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Me {
    #[serde(default)]
    homes: Vec<MeHome>,
}

#[derive(Debug, Deserialize)]
struct MeHome {
    id: u64,
}

#[derive(Debug, Deserialize)]
struct MobileDevice {
    settings: DeviceSettings,
    location: Option<DeviceLocation>,
}

#[derive(Debug, Deserialize)]
struct DeviceSettings {
    #[serde(rename = "geoTrackingEnabled")]
    geo_tracking_enabled: bool,
}

#[derive(Debug, Deserialize)]
struct DeviceLocation {
    #[serde(rename = "atHome")]
    at_home: Option<bool>,
}

#[async_trait]
impl PresenceSource for TadoSource {
    async fn poll(&self) -> Result<PresenceSignal> {
        let token = self.ensure_token().await?;
        let home = self.home_id(&token).await?;
        let url = format!(
            "{}/api/v2/homes/{}/mobileDevices",
            self.cfg.base_url.trim_end_matches('/'),
            home
        );

        let (status, body) = self.transport.get_bearer(&url, &token).await?;

        if status == 401 {
            *self.token.lock().await = None;
            return Err(Error::Auth {
                reason: "token expired or invalid".into(),
            });
        }
        if !(200..300).contains(&status) {
            return Err(Error::BadStatus { status, body });
        }

        let devices: Vec<MobileDevice> = serde_json::from_str(&body).map_err(|e| Error::Parse {
            reason: format!("mobile devices: {e}"),
        })?;

        let anyone_home = devices.iter().any(|d| {
            d.settings.geo_tracking_enabled
                && d.location.as_ref().and_then(|l| l.at_home).unwrap_or(false)
        });

        Ok(PresenceSignal {
            source: "tado".into(),
            anyone_home,
            observed_at: Utc::now(),
        })
    }

    fn name(&self) -> &str {
        "tado"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokens::MemoryTokenStore;
    use crate::transport::TadoTransport;
    use async_trait::async_trait;
    use std::sync::Mutex as StdMutex;

    type ResponseQueue = Arc<StdMutex<Vec<Result<(u16, String)>>>>;
    /// URL plus the form fields it was posted with.
    type PostLog = Arc<StdMutex<Vec<(String, Vec<(String, String)>)>>>;

    #[derive(Clone)]
    struct MockTransport {
        post_calls: PostLog,
        get_calls: Arc<StdMutex<Vec<String>>>,
        responses: ResponseQueue,
    }

    impl MockTransport {
        fn new(responses: Vec<Result<(u16, String)>>) -> Self {
            Self {
                post_calls: Arc::new(StdMutex::new(Vec::new())),
                get_calls: Arc::new(StdMutex::new(Vec::new())),
                responses: Arc::new(StdMutex::new(responses)),
            }
        }

        fn post_count(&self) -> usize {
            self.post_calls.lock().unwrap().len()
        }

        fn get_count(&self) -> usize {
            self.get_calls.lock().unwrap().len()
        }

        /// The form fields of the nth POST, for asserting on grant types.
        fn posted(&self, n: usize) -> Vec<(String, String)> {
            self.post_calls.lock().unwrap()[n].1.clone()
        }
    }

    #[async_trait]
    impl TadoTransport for MockTransport {
        async fn post_form(&self, url: &str, form: &[(&str, &str)]) -> Result<(u16, String)> {
            let fields = form
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            self.post_calls
                .lock()
                .unwrap()
                .push((url.to_string(), fields));
            self.responses.lock().unwrap().remove(0)
        }

        async fn get_bearer(&self, url: &str, _token: &str) -> Result<(u16, String)> {
            self.get_calls.lock().unwrap().push(url.to_string());
            self.responses.lock().unwrap().remove(0)
        }
    }

    /// A source that is already authorised, with the home id known so a
    /// test counting requests is not also counting discovery.
    fn source_with(responses: Vec<Result<(u16, String)>>) -> (MockTransport, TadoSource) {
        with_store(
            responses,
            Arc::new(MemoryTokenStore::with_token("refresh-1")),
        )
    }

    fn with_store(
        responses: Vec<Result<(u16, String)>>,
        tokens: Arc<MemoryTokenStore>,
    ) -> (MockTransport, TadoSource) {
        let mock = MockTransport::new(responses);
        let cfg = TadoConfig {
            home_id: Some(123),
            ..Default::default()
        };
        let source = TadoSource::with_transport(cfg, tokens, Arc::new(mock.clone()));
        (mock, source)
    }

    fn token_ok(refresh: &str) -> Result<(u16, String)> {
        Ok((
            200,
            format!(r#"{{"access_token":"tok","expires_in":600,"refresh_token":"{refresh}"}}"#),
        ))
    }

    fn devices_home() -> Result<(u16, String)> {
        Ok((
            200,
            r#"[
                {"settings":{"geoTrackingEnabled":true},"location":{"atHome":true}},
                {"settings":{"geoTrackingEnabled":true},"location":{"atHome":false}}
            ]"#
            .into(),
        ))
    }

    fn devices_away() -> Result<(u16, String)> {
        Ok((
            200,
            r#"[
                {"settings":{"geoTrackingEnabled":true},"location":{"atHome":false}}
            ]"#
            .into(),
        ))
    }

    // ── the device flow ──────────────────────────────────────────────

    #[tokio::test]
    async fn activation_hands_back_the_url_and_the_code() {
        let (_mock, source) = source_with(vec![Ok((
            200,
            r#"{"device_code":"dc","user_code":"ABCD-1234",
                "verification_uri_complete":"https://login.tado.com/oauth2/device?user_code=ABCD-1234",
                "expires_in":300,"interval":5}"#
                .into(),
        ))]);
        let pending = source.begin_activation().await.unwrap();
        assert_eq!(pending.user_code, "ABCD-1234");
        assert!(pending.verification_uri.contains("ABCD-1234"));
        assert_eq!(pending.interval, Duration::from_secs(5));
    }

    #[tokio::test]
    async fn waiting_to_be_approved_is_not_a_failure() {
        // tado answers a not-yet-approved poll with a 400. Treating
        // that as an error would abandon the activation a second after
        // starting it, before anyone could reach their phone.
        let (_mock, source) = source_with(vec![Ok((
            400,
            r#"{"error":"authorization_pending"}"#.into(),
        ))]);
        let pending = pending_activation();
        assert!(!source.finish_activation(&pending).await.unwrap());
    }

    #[tokio::test]
    async fn an_expired_code_is_a_failure() {
        let (_mock, source) = source_with(vec![Ok((400, r#"{"error":"expired_token"}"#.into()))]);
        let pending = pending_activation();
        let err = source.finish_activation(&pending).await.unwrap_err();
        assert!(matches!(err, Error::Auth { .. }));
    }

    #[tokio::test]
    async fn approval_stores_the_refresh_token() {
        let store = Arc::new(MemoryTokenStore::new());
        let (_mock, source) = with_store(vec![token_ok("refresh-new")], store.clone());
        assert!(!source.is_authorised().await.unwrap());

        assert!(
            source
                .finish_activation(&pending_activation())
                .await
                .unwrap()
        );
        assert_eq!(store.load().await.unwrap().as_deref(), Some("refresh-new"));
        assert!(source.is_authorised().await.unwrap());
    }

    fn pending_activation() -> DeviceActivation {
        DeviceActivation {
            verification_uri: "https://login.tado.com/oauth2/device".into(),
            user_code: "ABCD-1234".into(),
            device_code: "dc".into(),
            expires_at: Utc::now() + chrono::Duration::seconds(300),
            interval: Duration::from_secs(5),
        }
    }

    // ── refreshing ───────────────────────────────────────────────────

    #[tokio::test]
    async fn the_rotated_refresh_token_is_stored_every_time() {
        // tado invalidates a refresh token as it is used and issues a
        // replacement. Keeping the original would work until the next
        // restart and then lock Niles out, a day later, for no visible
        // reason.
        let store = Arc::new(MemoryTokenStore::with_token("refresh-1"));
        let (mock, source) = with_store(vec![token_ok("refresh-2"), devices_home()], store.clone());

        let _ = source.poll().await.unwrap();
        assert_eq!(store.load().await.unwrap().as_deref(), Some("refresh-2"));

        let sent = mock.posted(0);
        assert!(sent.contains(&("grant_type".into(), "refresh_token".into())));
        assert!(sent.contains(&("refresh_token".into(), "refresh-1".into())));
    }

    #[tokio::test]
    async fn polling_without_an_authorisation_says_so() {
        let store = Arc::new(MemoryTokenStore::new());
        let (mock, source) = with_store(vec![], store);
        let err = source.poll().await.unwrap_err();
        assert!(matches!(err, Error::Auth { .. }), "{err}");
        assert_eq!(mock.post_count(), 0, "tado should not be troubled for it");
    }

    #[tokio::test]
    async fn a_refused_refresh_token_asks_for_a_new_authorisation() {
        let (_mock, source) = source_with(vec![Ok((400, r#"{"error":"invalid_grant"}"#.into()))]);
        let err = source.poll().await.unwrap_err();
        match err {
            Error::Auth { reason } => assert!(reason.contains("authorise again"), "{reason}"),
            other => panic!("expected an auth error, got {other}"),
        }
    }

    #[tokio::test]
    async fn access_token_is_cached_between_polls() {
        let (mock, source) =
            source_with(vec![token_ok("refresh-2"), devices_home(), devices_home()]);
        let _ = source.poll().await.unwrap();
        assert_eq!(mock.post_count(), 1);
        assert_eq!(mock.get_count(), 1);

        let _ = source.poll().await.unwrap();
        assert_eq!(mock.post_count(), 1, "still cached");
        assert_eq!(mock.get_count(), 2);
    }

    #[tokio::test]
    async fn token_refreshed_when_expired() {
        let (mock, source) = source_with(vec![token_ok("refresh-2"), devices_home()]);
        {
            let mut cache = source.token.lock().await;
            *cache = Some(CachedToken {
                access_token: "expired".into(),
                expires_at: Utc::now() - chrono::Duration::seconds(60),
            });
        }
        let sig = source.poll().await.unwrap();
        assert!(sig.anyone_home);
        assert_eq!(mock.post_count(), 1);
        assert_eq!(mock.get_count(), 1);
    }

    // ── home discovery ───────────────────────────────────────────────

    #[tokio::test]
    async fn the_home_is_discovered_when_the_config_does_not_name_one() {
        let mock = MockTransport::new(vec![
            token_ok("refresh-2"),
            Ok((200, r#"{"homes":[{"id":4242}]}"#.into())),
            devices_home(),
            devices_home(),
        ]);
        let source = TadoSource::with_transport(
            TadoConfig::default(),
            Arc::new(MemoryTokenStore::with_token("refresh-1")),
            Arc::new(mock.clone()),
        );

        let _ = source.poll().await.unwrap();
        let urls = mock.get_calls.lock().unwrap().clone();
        assert!(urls[0].ends_with("/api/v2/me"));
        assert!(urls[1].contains("/homes/4242/mobileDevices"));
        drop(urls);

        // Asked once: an account's homes do not move.
        let _ = source.poll().await.unwrap();
        assert_eq!(mock.get_count(), 3);
    }

    #[tokio::test]
    async fn an_account_with_no_homes_is_a_parse_error() {
        let mock = MockTransport::new(vec![
            token_ok("refresh-2"),
            Ok((200, r#"{"homes":[]}"#.into())),
        ]);
        let source = TadoSource::with_transport(
            TadoConfig::default(),
            Arc::new(MemoryTokenStore::with_token("refresh-1")),
            Arc::new(mock),
        );
        let err = source.poll().await.unwrap_err();
        assert!(matches!(err, Error::Parse { .. }));
    }

    // ── reading the devices ──────────────────────────────────────────

    #[tokio::test]
    async fn mobile_devices_any_at_home_true_when_one_device_home() {
        let (_mock, source) = source_with(vec![token_ok("r2"), devices_home()]);
        assert!(source.poll().await.unwrap().anyone_home);
    }

    #[tokio::test]
    async fn mobile_devices_all_away_returns_false() {
        let (_mock, source) = source_with(vec![token_ok("r2"), devices_away()]);
        assert!(!source.poll().await.unwrap().anyone_home);
    }

    #[tokio::test]
    async fn geo_tracking_disabled_devices_ignored() {
        let body = r#"[
            {"settings":{"geoTrackingEnabled":false},"location":{"atHome":true}}
        ]"#;
        let (_mock, source) = source_with(vec![token_ok("r2"), Ok((200, body.into()))]);
        assert!(!source.poll().await.unwrap().anyone_home);
    }

    #[tokio::test]
    async fn missing_location_treated_as_not_home() {
        let body = r#"[
            {"settings":{"geoTrackingEnabled":true},"location":null}
        ]"#;
        let (_mock, source) = source_with(vec![token_ok("r2"), Ok((200, body.into()))]);
        assert!(!source.poll().await.unwrap().anyone_home);
    }

    #[tokio::test]
    async fn mobile_devices_endpoint_500_returns_bad_status() {
        let (_mock, source) = source_with(vec![token_ok("r2"), Ok((500, "err".into()))]);
        let err = source.poll().await.unwrap_err();
        assert!(matches!(err, Error::BadStatus { status: 500, .. }));
    }

    #[tokio::test]
    async fn mobile_devices_endpoint_401_invalidates_cache_and_re_auths() {
        let (_mock, source) = source_with(vec![
            token_ok("r2"),
            Ok((401, "unauthorized".into())),
            token_ok("r3"),
            devices_home(),
        ]);
        let err = source.poll().await.unwrap_err();
        assert!(matches!(err, Error::Auth { .. }));

        assert!(source.poll().await.unwrap().anyone_home);
    }

    #[tokio::test]
    async fn invalid_devices_json_returns_parse_error() {
        let (_mock, source) = source_with(vec![token_ok("r2"), Ok((200, "not-json".into()))]);
        let err = source.poll().await.unwrap_err();
        assert!(matches!(err, Error::Parse { .. }));
    }

    #[tokio::test]
    async fn empty_device_list_returns_false() {
        let (_mock, source) = source_with(vec![token_ok("r2"), Ok((200, "[]".into()))]);
        assert!(!source.poll().await.unwrap().anyone_home);
    }

    #[tokio::test]
    async fn trims_trailing_slash_from_base_url() {
        let mock = MockTransport::new(vec![token_ok("r2"), devices_home()]);
        let cfg = TadoConfig {
            home_id: Some(123),
            base_url: "https://my.tado.com/".into(),
            ..Default::default()
        };
        let source = TadoSource::with_transport(
            cfg,
            Arc::new(MemoryTokenStore::with_token("refresh-1")),
            Arc::new(mock.clone()),
        );
        let _ = source.poll().await.unwrap();
        let urls = mock.get_calls.lock().unwrap();
        assert_eq!(urls.len(), 1);
        assert!(!urls[0].contains("://my.tado.com//api"));
    }
}
