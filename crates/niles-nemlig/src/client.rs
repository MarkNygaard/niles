use crate::basket::{Basket, DeliveryDay, RawBasket, RawDays};
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
        retried(
            || self.try_search(credentials, query, take),
            || self.forget(),
        )
        .await
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

    /// The basket as it stands.
    pub async fn basket(&self, credentials: &Credentials) -> Result<Basket> {
        retried(|| self.try_basket(credentials), || self.forget()).await
    }

    async fn try_basket(&self, credentials: &Credentials) -> Result<Basket> {
        let mut guard = self.session.lock().await;
        let session = self.signed_in(&mut guard, credentials).await?;
        self.read_basket(session).await
    }

    /// Put these products in the basket, at least this many of each.
    ///
    /// "At least": what is in the basket already is kept, so sending the
    /// list twice does not order twice, and something added by hand on
    /// the website is not taken away.
    pub async fn fill_basket(
        &self,
        credentials: &Credentials,
        wanted: &[(String, u32)],
    ) -> Result<Basket> {
        retried(|| self.try_fill(credentials, wanted), || self.forget()).await
    }

    async fn try_fill(
        &self,
        credentials: &Credentials,
        wanted: &[(String, u32)],
    ) -> Result<Basket> {
        let mut guard = self.session.lock().await;
        let session = self.signed_in(&mut guard, credentials).await?;
        let mut basket = self.read_basket(session).await?;
        for (product_id, quantity) in wanted {
            if basket.quantity_of(product_id) >= *quantity {
                continue;
            }
            let response = self
                .post(format!("{WWW}/webapi/basket/AddToBasket"))
                .json(&json!({
                    "ProductId": product_id,
                    "quantity": quantity,
                    "AffectPartialQuantity": false,
                    "disableQuantityValidation": false,
                }))
                .send()
                .await?;
            let raw: RawBasket = read(response).await?;
            note_context(session, &raw);
            basket = raw.into();
        }
        Ok(basket)
    }

    /// The days nemlig.com delivers in the coming `days`, and when.
    pub async fn delivery_days(
        &self,
        credentials: &Credentials,
        days: u32,
    ) -> Result<Vec<DeliveryDay>> {
        retried(|| self.try_days(credentials, days), || self.forget()).await
    }

    async fn try_days(&self, credentials: &Credentials, days: u32) -> Result<Vec<DeliveryDay>> {
        let mut guard = self.session.lock().await;
        self.signed_in(&mut guard, credentials).await?;
        let response = self
            .http
            .get(format!("{WWW}/webapi/v2/Delivery/GetDeliveryDays"))
            .query(&[
                ("startDate", "undefined"),
                ("days", &days.to_string()),
                ("showForSubscriptions", "false"),
            ])
            .header(header::ACCEPT, "application/json")
            .send()
            .await?;
        Ok(read::<RawDays>(response).await?.into_days())
    }

    /// Reserve a delivery time for the basket.
    ///
    /// nemlig first only *tries*: when the new time changes the basket's
    /// prices, or something cannot be delivered then, it reports that and
    /// reserves nothing until asked again. Niles asks again — the person
    /// chose this time, and sees the new total straight after.
    pub async fn reserve(&self, credentials: &Credentials, slot_id: i64) -> Result<Basket> {
        retried(|| self.try_reserve(credentials, slot_id), || self.forget()).await
    }

    async fn try_reserve(&self, credentials: &Credentials, slot_id: i64) -> Result<Basket> {
        let mut guard = self.session.lock().await;
        let session = self.signed_in(&mut guard, credentials).await?;
        let slot = [("timeslotId", slot_id.to_string())];
        // A failure here is reported as itself. Swallowing it once turned
        // a 411 into "that time may have just filled up", which sent
        // everybody looking in the wrong place.
        let tried: serde_json::Value = read(
            self.post(format!("{WWW}/webapi/Delivery/TryUpdateDeliveryTime"))
                .query(&slot)
                .header(header::CONTENT_LENGTH, "0")
                .send()
                .await?,
        )
        .await?;
        if tried.get("IsReserved").and_then(|v| v.as_bool()) == Some(false) {
            let confirmed = self
                .post(format!("{WWW}/webapi/Delivery/UpdateDeliveryTime"))
                .query(&slot)
                .header(header::CONTENT_LENGTH, "0")
                .send()
                .await?;
            if !confirmed.status().is_success() {
                return Err(api_error(confirmed).await);
            }
        }
        let mut basket = self.read_basket(session).await?;
        if basket.slot_id != Some(slot_id) {
            let reason = tried
                .get("Message")
                .or_else(|| tried.get("ErrorMessage"))
                .and_then(|m| m.as_str())
                .unwrap_or("nemlig.com did not keep that time — it may have just filled up")
                .to_string();
            return Err(Error::Api {
                status: 409,
                reason,
            });
        }
        basket.held_minutes = tried
            .get("MinutesReserved")
            .and_then(|m| m.as_u64())
            .and_then(|m| u32::try_from(m).ok());
        Ok(basket)
    }

    async fn read_basket(&self, session: &mut Session) -> Result<Basket> {
        let response = self
            .http
            .get(format!("{WWW}/webapi/basket/GetBasket"))
            .header(header::ACCEPT, "application/json")
            .send()
            .await?;
        let raw: RawBasket = read(response).await?;
        note_context(session, &raw);
        Ok(raw.into())
    }

    /// A POST to the website's own API, carrying the anti-forgery token it
    /// checks for.
    ///
    /// One with nothing to send still needs `Content-Length: 0` said out
    /// loud: with no body — even an empty one — none is sent, and nemlig's
    /// server answers that with 411 Length Required.
    fn post(&self, url: String) -> reqwest::RequestBuilder {
        let mut request = self
            .http
            .post(url)
            .header(header::ACCEPT, "application/json")
            .header(header::REFERER, format!("{WWW}/"));
        if let Some(xsrf) = self.cookie("XSRF-TOKEN") {
            request = request.header("X-XSRF-TOKEN", xsrf);
        }
        request
    }

    async fn forget(&self) {
        *self.session.lock().await = None;
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

/// Run `attempt`, and once more after `reset` if the session was refused.
async fn retried<T, A, AF, R, RF>(attempt: A, reset: R) -> Result<T>
where
    A: Fn() -> AF,
    AF: std::future::Future<Output = Result<T>>,
    R: FnOnce() -> RF,
    RF: std::future::Future<Output = ()>,
{
    match attempt().await {
        Err(Error::Api { status: 401, .. }) => {
            reset().await;
            attempt().await
        }
        other => other,
    }
}

/// Search prices against the basket's zone and reserved time, so a new
/// reservation moves what search is asked for.
fn note_context(session: &mut Session, raw: &RawBasket) {
    if let Some(timeslot) = raw.timeslot_utc.as_ref().filter(|t| !t.is_empty()) {
        session.timeslot_utc = timeslot.clone();
    }
    if let Some(zone) = raw.delivery_zone_id {
        session.zone_id = zone;
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

    /// The basket and delivery days on the real site, and one product in
    /// and out of the basket again. Ignored, like the search above; it
    /// leaves the basket as it found it. Reserving a time is not tried:
    /// nothing releases a reservation once made.
    #[tokio::test]
    #[ignore]
    async fn fills_the_real_basket_and_empties_it_again() {
        let credentials = Credentials {
            username: std::env::var("NEMLIG_USER").expect("NEMLIG_USER"),
            password: std::env::var("NEMLIG_PASSWORD").expect("NEMLIG_PASSWORD"),
        };
        let client = NemligClient::new().unwrap();
        let days = client.delivery_days(&credentials, 3).await.unwrap();
        assert!(days.iter().any(|d| !d.slots.is_empty()), "{days:?}");

        let milk = client.search(&credentials, "letmælk", 1).await.unwrap()[0].clone();
        let before = client
            .basket(&credentials)
            .await
            .unwrap()
            .quantity_of(&milk.id);
        let wanted = vec![(milk.id.clone(), before + 1)];
        let filled = client.fill_basket(&credentials, &wanted).await.unwrap();
        assert_eq!(filled.quantity_of(&milk.id), before + 1);
        // Sending the same list again does not order twice.
        let again = client.fill_basket(&credentials, &wanted).await.unwrap();
        assert_eq!(again.quantity_of(&milk.id), before + 1);

        let response = client
            .post(format!("{WWW}/webapi/basket/AddToBasket"))
            .json(&json!({
                "ProductId": milk.id,
                "quantity": before,
                "AffectPartialQuantity": before == 0,
                "disableQuantityValidation": false,
            }))
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
        let after = client.basket(&credentials).await.unwrap();
        assert_eq!(after.quantity_of(&milk.id), before);
    }

    /// Reserving, on the real site: the time the basket already holds,
    /// again, which changes nothing but goes the whole way through.
    /// Ignored, and needs a time reserved already.
    #[tokio::test]
    #[ignore]
    async fn reserves_the_real_basket_time_again() {
        let credentials = Credentials {
            username: std::env::var("NEMLIG_USER").expect("NEMLIG_USER"),
            password: std::env::var("NEMLIG_PASSWORD").expect("NEMLIG_PASSWORD"),
        };
        let client = NemligClient::new().unwrap();
        let slot = client
            .basket(&credentials)
            .await
            .unwrap()
            .slot_id
            .expect("reserve a time in the app first");
        let basket = client.reserve(&credentials, slot).await.unwrap();
        assert_eq!(basket.slot_id, Some(slot));
        assert!(basket.delivery.is_some());
        assert!(basket.held_minutes.is_some_and(|m| m > 0), "{basket:?}");
    }
}
