use crate::error::{Error, Result};
use crate::product::{Product, SearchAnswer};
use reqwest::cookie::{CookieStore, Jar};
use reqwest::{StatusCode, Url, header};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::json;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

const WWW: &str = "https://www.nemlig.com";
/// Search lives on its own host, and wants a bearer token rather than
/// the session cookie.
const GATEWAY: &str = "https://webapi.prod.knl.nemlig.it";

/// Plain on purpose: a browser's User-Agent is sent to nemlig's queue
/// page (Queue-it) instead of the API.
const USER_AGENT: &str = concat!("niles/", env!("CARGO_PKG_VERSION"));

/// The bearer token lives five minutes; one is fetched again a minute
/// before that rather than discovering it expired mid-search.
const TOKEN_LIFETIME: Duration = Duration::from_secs(4 * 60);

/// Long before nemlig would end it: a session that has quietly lapsed
/// answers searches as nobody in particular, priced for nowhere.
const SESSION_LIFETIME: Duration = Duration::from_secs(6 * 60 * 60);

/// The household's login. Read from the config per call, so a changed
/// password takes hold on the next search rather than the next restart.
#[derive(Clone)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("username", &self.username)
            .field("password", &"…")
            .finish()
    }
}

impl Credentials {
    /// Which login a session belongs to, without keeping the password.
    fn fingerprint(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.username.trim().to_lowercase().hash(&mut hasher);
        self.password.hash(&mut hasher);
        hasher.finish()
    }
}

struct Session {
    signed_in_as: u64,
    signed_in_at: Instant,
    /// What search prices against: the account's delivery zone and slot.
    zone_id: i64,
    timeslot_utc: String,
    token: Option<(String, Instant)>,
}

/// One logged-in session with nemlig.com, kept for as long as it lasts.
pub struct NemligClient {
    http: reqwest::Client,
    jar: Arc<Jar>,
    session: Mutex<Option<Session>>,
}

impl NemligClient {
    pub fn new() -> Result<Self> {
        let jar = Arc::new(Jar::default());
        let http = reqwest::Client::builder()
            .cookie_provider(jar.clone())
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(20))
            .build()?;
        Ok(Self {
            http,
            jar,
            session: Mutex::new(None),
        })
    }

    /// Products matching `query`, priced for this account.
    ///
    /// A refused session is retried once with a fresh login: nemlig ends
    /// sessions on its own schedule, and that should cost a second, not
    /// an error on the phone.
    pub async fn search(
        &self,
        credentials: &Credentials,
        query: &str,
        take: u32,
    ) -> Result<Vec<Product>> {
        match self.try_search(credentials, query, take).await {
            Err(Error::Api { status: 401, .. }) => {
                *self.session.lock().await = None;
                self.try_search(credentials, query, take).await
            }
            other => other,
        }
    }

    async fn try_search(
        &self,
        credentials: &Credentials,
        query: &str,
        take: u32,
    ) -> Result<Vec<Product>> {
        let mut guard = self.session.lock().await;
        let session = self.signed_in(&mut guard, credentials).await?;
        let token = self.token(session).await?;
        let response = self
            .http
            .get(format!("{GATEWAY}/searchgateway/api/search"))
            .query(&[
                ("query", query),
                ("take", &take.to_string()),
                ("skip", "0"),
                ("timeslotUtc", &session.timeslot_utc),
                ("deliveryZoneId", &session.zone_id.to_string()),
            ])
            .bearer_auth(token)
            .header(header::ACCEPT, "application/json")
            .send()
            .await?;
        Ok(read::<SearchAnswer>(response).await?.into_products())
    }

    /// The session for these credentials, logging in if there is none,
    /// it is somebody else's, or it is old.
    async fn signed_in<'a>(
        &self,
        guard: &'a mut Option<Session>,
        credentials: &Credentials,
    ) -> Result<&'a mut Session> {
        let fresh = guard.as_ref().is_some_and(|s| {
            s.signed_in_as == credentials.fingerprint()
                && s.signed_in_at.elapsed() < SESSION_LIFETIME
        });
        if !fresh {
            *guard = Some(self.log_in(credentials).await?);
        }
        Ok(guard.as_mut().expect("just signed in"))
    }

    /// The website's own three steps: anti-forgery cookies, an anonymous
    /// token, then the login itself, which leaves `.ASPXAUTH` in the jar.
    async fn log_in(&self, credentials: &Credentials) -> Result<Session> {
        let antiforgery = self
            .http
            .get(format!("{WWW}/webapi/AntiForgery"))
            .send()
            .await?;
        if !antiforgery.status().is_success() {
            return Err(api_error(antiforgery).await);
        }
        let anonymous = self.fetch_token().await?;

        let mut request = self
            .http
            .post(format!("{WWW}/webapi/login"))
            .bearer_auth(anonymous)
            .header(header::REFERER, format!("{WWW}/login"))
            .header(header::ACCEPT, "application/json")
            .json(&json!({
                "Username": credentials.username.trim(),
                "Password": credentials.password,
                "CheckForExistingProducts": true,
                "DoMerge": true,
                "AppInstalled": false,
                "SaveExistingBasket": false,
            }));
        if let Some(xsrf) = self.cookie("XSRF-TOKEN") {
            request = request.header("X-XSRF-TOKEN", xsrf);
        }
        let response = request.send().await?;
        // A wrong email or password is a 400, with the reason in the body.
        if response.status() == StatusCode::BAD_REQUEST {
            let body: serde_json::Value = response.json().await.unwrap_or_default();
            let reason = body
                .get("ErrorMessage")
                .or_else(|| body.get("Message"))
                .and_then(|m| m.as_str())
                .unwrap_or("wrong email or password")
                .to_string();
            return Err(Error::Login { reason });
        }
        let answer: LoginAnswer = read(response).await?;
        tracing::info!("[nemlig] signed in");
        Ok(Session {
            signed_in_as: credentials.fingerprint(),
            signed_in_at: Instant::now(),
            zone_id: answer.delivery_zone_id,
            timeslot_utc: answer.timeslot_utc,
            token: None,
        })
    }

    async fn token(&self, session: &mut Session) -> Result<String> {
        if let Some((token, at)) = &session.token
            && at.elapsed() < TOKEN_LIFETIME
        {
            return Ok(token.clone());
        }
        let token = self.fetch_token().await?;
        session.token = Some((token.clone(), Instant::now()));
        Ok(token)
    }

    /// A bearer token for whoever the cookie jar says is signed in.
    async fn fetch_token(&self) -> Result<String> {
        #[derive(Deserialize)]
        struct Token {
            access_token: String,
        }
        let response = self
            .http
            .get(format!("{WWW}/webapi/Token"))
            .header(header::REFERER, format!("{WWW}/"))
            .header(header::ACCEPT, "application/json")
            .send()
            .await?;
        Ok(read::<Token>(response).await?.access_token)
    }

    fn cookie(&self, name: &str) -> Option<String> {
        let url: Url = WWW.parse().expect("a valid URL");
        let header = self.jar.cookies(&url)?;
        cookie_value(header.to_str().ok()?, name)
    }
}

/// What a login answers with: where the account is, for pricing.
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct LoginAnswer {
    delivery_zone_id: i64,
    timeslot_utc: String,
}

async fn read<T: DeserializeOwned>(response: reqwest::Response) -> Result<T> {
    if !response.status().is_success() {
        return Err(api_error(response).await);
    }
    let status = response.status().as_u16();
    response.json().await.map_err(|e| Error::Api {
        status,
        reason: format!("an answer Niles could not read: {e}"),
    })
}

async fn api_error(response: reqwest::Response) -> Error {
    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    Error::Api {
        status,
        reason: body.chars().take(200).collect(),
    }
}

/// One cookie's value out of a `Cookie:` header, decoded.
fn cookie_value(header: &str, name: &str) -> Option<String> {
    header.split(';').find_map(|pair| {
        let (key, value) = pair.trim().split_once('=')?;
        (key == name).then(|| {
            urlencoding::decode(value)
                .map(|v| v.into_owned())
                .unwrap_or_else(|_| value.to_string())
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_one_cookie_among_several() {
        let header = "ASP.NET_SessionId=abc; XSRF-TOKEN=CfDJ8%2Bxyz; .ASPXAUTH=secret";
        assert_eq!(
            cookie_value(header, "XSRF-TOKEN").as_deref(),
            Some("CfDJ8+xyz")
        );
        assert_eq!(cookie_value(header, "missing"), None);
    }

    #[test]
    fn a_session_belongs_to_one_login() {
        let mark = Credentials {
            username: "mark@example.com".into(),
            password: "one".into(),
        };
        let changed = Credentials {
            password: "two".into(),
            ..mark.clone()
        };
        let shouted = Credentials {
            username: " MARK@example.com ".into(),
            ..mark.clone()
        };
        assert_ne!(mark.fingerprint(), changed.fingerprint());
        assert_eq!(mark.fingerprint(), shouted.fingerprint());
    }

    #[test]
    fn the_password_stays_out_of_logs() {
        let credentials = Credentials {
            username: "mark@example.com".into(),
            password: "hunter2".into(),
        };
        assert!(!format!("{credentials:?}").contains("hunter2"));
    }

    /// The real site, with a real account. Ignored: it needs
    /// `NEMLIG_USER` and `NEMLIG_PASSWORD`, and talks to nemlig.com.
    #[tokio::test]
    #[ignore]
    async fn searches_the_real_site() {
        let credentials = Credentials {
            username: std::env::var("NEMLIG_USER").expect("NEMLIG_USER"),
            password: std::env::var("NEMLIG_PASSWORD").expect("NEMLIG_PASSWORD"),
        };
        let client = NemligClient::new().unwrap();
        let milk = client.search(&credentials, "letmælk", 5).await.unwrap();
        assert!(!milk.is_empty());
        assert!(milk.iter().all(|p| p.price > 0.0), "{milk:?}");
        // The session is reused, not logged into again.
        let rolls = client.search(&credentials, "rundstykker", 5).await.unwrap();
        assert!(!rolls.is_empty());
    }
}
