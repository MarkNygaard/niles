//! Tado HTTP adapter — polls mobileDevices via OAuth2 password grant.

use crate::error::{Error, Result};
use crate::source::PresenceSource;
use crate::state::PresenceSignal;
use crate::transport::{HttpTadoTransport, TadoTransport};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// Runtime configuration for the Tado source.
#[derive(Debug, Clone)]
pub struct TadoConfig {
    pub username: String,
    pub password: String,
    pub home_id: u64,
    pub base_url: String,
    pub auth_url: String,
    pub client_id: String,
    pub client_secret: String,
    pub request_timeout: Duration,
}

impl Default for TadoConfig {
    fn default() -> Self {
        Self {
            username: String::new(),
            password: String::new(),
            home_id: 0,
            base_url: "https://my.tado.com".into(),
            auth_url: "https://auth.tado.com/oauth/token".into(),
            client_id: "tado-web-app".into(),
            client_secret: "wZaRN7rpjn3FoNyF5IFuxg9uMzYJ8Oo2m8psRRAA".into(),
            request_timeout: Duration::from_secs(30),
        }
    }
}

struct CachedToken {
    access_token: String,
    expires_at: DateTime<Utc>,
}

/// Tado presence source.
pub struct TadoSource {
    transport: Arc<dyn TadoTransport>,
    cfg: TadoConfig,
    token: Mutex<Option<CachedToken>>,
}

impl TadoSource {
    pub fn new(cfg: TadoConfig) -> Result<Self> {
        let transport = Arc::new(HttpTadoTransport::new(cfg.request_timeout)?);
        Ok(Self {
            transport,
            cfg,
            token: Mutex::new(None),
        })
    }

    #[allow(dead_code)]
    pub fn with_transport(cfg: TadoConfig, transport: Arc<dyn TadoTransport>) -> Self {
        Self {
            transport,
            cfg,
            token: Mutex::new(None),
        }
    }

    async fn ensure_token(&self) -> Result<String> {
        let mut cache = self.token.lock().await;

        if let Some(token) = cache.as_ref()
            && token.expires_at > Utc::now() + chrono::Duration::seconds(30)
        {
            return Ok(token.access_token.clone());
        }

        let form = [
            ("grant_type", "password"),
            ("client_id", &self.cfg.client_id),
            ("client_secret", &self.cfg.client_secret),
            ("username", &self.cfg.username),
            ("password", &self.cfg.password),
            ("scope", "home.user"),
        ];

        let (status, body) = self.transport.post_form(&self.cfg.auth_url, &form).await?;

        if status == 401 {
            return Err(Error::Auth {
                reason: "invalid tado credentials".into(),
            });
        }
        if !(200..300).contains(&status) {
            return Err(Error::BadStatus { status, body });
        }

        let token_resp: TokenResponse = serde_json::from_str(&body).map_err(|e| Error::Parse {
            reason: format!("auth response: {e}"),
        })?;

        if token_resp.expires_in == 0 {
            return Err(Error::Parse {
                reason: "expires_in is zero or missing".into(),
            });
        }

        let expires_at = Utc::now() + chrono::Duration::seconds(token_resp.expires_in as i64);
        let access_token = token_resp.access_token.clone();
        *cache = Some(CachedToken {
            access_token: access_token.clone(),
            expires_at,
        });
        Ok(access_token)
    }
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
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
        let url = format!(
            "{}/api/v2/homes/{}/mobileDevices",
            self.cfg.base_url, self.cfg.home_id
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
    use crate::transport::TadoTransport;
    use async_trait::async_trait;
    use std::sync::Mutex;

    type ResponseQueue = Arc<Mutex<Vec<Result<(u16, String)>>>>;

    #[derive(Clone)]
    struct MockTransport {
        post_calls: Arc<Mutex<Vec<String>>>,
        get_calls: Arc<Mutex<Vec<String>>>,
        responses: ResponseQueue,
    }

    impl MockTransport {
        fn new(responses: Vec<Result<(u16, String)>>) -> Self {
            Self {
                post_calls: Arc::new(Mutex::new(Vec::new())),
                get_calls: Arc::new(Mutex::new(Vec::new())),
                responses: Arc::new(Mutex::new(responses)),
            }
        }

        fn post_count(&self) -> usize {
            self.post_calls.lock().unwrap().len()
        }

        fn get_count(&self) -> usize {
            self.get_calls.lock().unwrap().len()
        }
    }

    #[async_trait]
    impl TadoTransport for MockTransport {
        async fn post_form(&self, url: &str, _form: &[(&str, &str)]) -> Result<(u16, String)> {
            self.post_calls.lock().unwrap().push(url.to_string());
            self.responses.lock().unwrap().remove(0)
        }

        async fn get_bearer(&self, url: &str, _token: &str) -> Result<(u16, String)> {
            self.get_calls.lock().unwrap().push(url.to_string());
            self.responses.lock().unwrap().remove(0)
        }
    }

    fn source_with(responses: Vec<Result<(u16, String)>>) -> (MockTransport, TadoSource) {
        let mock = MockTransport::new(responses);
        let cfg = TadoConfig {
            username: "u".into(),
            password: "p".into(),
            home_id: 123,
            ..Default::default()
        };
        let source = TadoSource::with_transport(cfg, Arc::new(mock.clone()));
        (mock, source)
    }

    fn auth_ok() -> Result<(u16, String)> {
        Ok((200, r#"{"access_token":"tok","expires_in":3600}"#.into()))
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

    #[tokio::test]
    async fn token_fetched_on_first_poll_and_cached() {
        let (mock, source) = source_with(vec![auth_ok(), devices_home(), devices_home()]);
        let _ = source.poll().await.unwrap();
        assert_eq!(mock.post_count(), 1);
        assert_eq!(mock.get_count(), 1);

        let _ = source.poll().await.unwrap();
        assert_eq!(mock.post_count(), 1); // cached
        assert_eq!(mock.get_count(), 2);
    }

    #[tokio::test]
    async fn mobile_devices_any_at_home_true_when_one_device_home() {
        let (_mock, source) = source_with(vec![auth_ok(), devices_home()]);
        let sig = source.poll().await.unwrap();
        assert!(sig.anyone_home);
    }

    #[tokio::test]
    async fn mobile_devices_all_away_returns_false() {
        let (_mock, source) = source_with(vec![auth_ok(), devices_away()]);
        let sig = source.poll().await.unwrap();
        assert!(!sig.anyone_home);
    }

    #[tokio::test]
    async fn geo_tracking_disabled_devices_ignored() {
        let body = r#"[
            {"settings":{"geoTrackingEnabled":false},"location":{"atHome":true}}
        ]"#;
        let (_mock, source) = source_with(vec![auth_ok(), Ok((200, body.into()))]);
        let sig = source.poll().await.unwrap();
        assert!(!sig.anyone_home);
    }

    #[tokio::test]
    async fn missing_location_treated_as_not_home() {
        let body = r#"[
            {"settings":{"geoTrackingEnabled":true},"location":null}
        ]"#;
        let (_mock, source) = source_with(vec![auth_ok(), Ok((200, body.into()))]);
        let sig = source.poll().await.unwrap();
        assert!(!sig.anyone_home);
    }

    #[tokio::test]
    async fn auth_endpoint_401_returns_error_auth() {
        let (_mock, source) = source_with(vec![Ok((401, "unauthorized".into()))]);
        let err = source.poll().await.unwrap_err();
        assert!(matches!(err, Error::Auth { .. }));
    }

    #[tokio::test]
    async fn mobile_devices_endpoint_500_returns_bad_status() {
        let (_mock, source) = source_with(vec![auth_ok(), Ok((500, "err".into()))]);
        let err = source.poll().await.unwrap_err();
        assert!(matches!(err, Error::BadStatus { status: 500, .. }));
    }

    #[tokio::test]
    async fn mobile_devices_endpoint_401_invalidates_cache_and_returns_auth_error() {
        let (_mock, source) = source_with(vec![
            auth_ok(),
            Ok((401, "unauthorized".into())),
            auth_ok(),
            devices_home(),
        ]);
        let err = source.poll().await.unwrap_err();
        assert!(matches!(err, Error::Auth { .. }));

        // next poll should re-auth
        let sig = source.poll().await.unwrap();
        assert!(sig.anyone_home);
    }

    #[tokio::test]
    async fn invalid_devices_json_returns_parse_error() {
        let (_mock, source) = source_with(vec![auth_ok(), Ok((200, "not-json".into()))]);
        let err = source.poll().await.unwrap_err();
        assert!(matches!(err, Error::Parse { .. }));
    }

    #[tokio::test]
    async fn empty_device_list_returns_false() {
        let (_mock, source) = source_with(vec![auth_ok(), Ok((200, "[]".into()))]);
        let sig = source.poll().await.unwrap();
        assert!(!sig.anyone_home);
    }
}
