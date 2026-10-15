//! Strava's API, as far as Niles reads it: the OAuth2 sign-in, the
//! list of a person's activities, one activity in detail, and the
//! request budget every answer reports.
//!
//! See <https://developers.strava.com/docs/authentication/>,
//! <https://developers.strava.com/docs/rate-limits/> and
//! <https://developers.strava.com/docs/reference/>.

use crate::error::{Error, Result};
use crate::model::{Activity, Athlete, Kind, StravaApp, StravaTokens};
use chrono::{DateTime, Duration, NaiveTime, Timelike, Utc};
use reqwest::{RequestBuilder, StatusCode, header::HeaderMap};
use serde_json::Value;

/// Private activities too: a training plan cares about the run the
/// person did not want on their feed as much as any other.
pub const SCOPE: &str = "activity:read_all";
/// The most `/athlete/activities` gives per page.
pub const PER_PAGE: usize = 200;
pub const SOURCE: &str = "strava";

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    /// `https://www.strava.com`; a mock server's address in tests.
    base: String,
}

/// What a person's app may still ask for before Strava starts saying
/// 429: the least left of the overall and the read limits, for the
/// 15-minute window and for the day.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Budget {
    pub short_left: Option<i64>,
    pub daily_left: Option<i64>,
}

impl Budget {
    /// From the `X-RateLimit-*` and `X-ReadRateLimit-*` headers, each a
    /// "15-minute,daily" pair.
    pub fn from_headers(headers: &HeaderMap) -> Budget {
        let pair = |name: &str| -> Option<(i64, i64)> {
            let text = headers.get(name)?.to_str().ok()?;
            let (short, daily) = text.split_once(',')?;
            Some((short.trim().parse().ok()?, daily.trim().parse().ok()?))
        };
        let mut budget = Budget::default();
        for (limit, usage) in [
            ("x-ratelimit-limit", "x-ratelimit-usage"),
            ("x-readratelimit-limit", "x-readratelimit-usage"),
        ] {
            if let (Some(limit), Some(usage)) = (pair(limit), pair(usage)) {
                let least = |kept: Option<i64>, left: i64| Some(kept.map_or(left, |k| k.min(left)));
                budget.short_left = least(budget.short_left, limit.0 - usage.0);
                budget.daily_left = least(budget.daily_left, limit.1 - usage.1);
            }
        }
        budget
    }

    /// When asking again is worth it, if `reserve` or fewer requests are
    /// left: the next 15-minute window, or tomorrow when the day's are
    /// gone.
    pub fn exhausted_until(&self, reserve: i64, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        if self.daily_left.is_some_and(|left| left <= reserve) {
            Some(next_utc_midnight(now))
        } else if self.short_left.is_some_and(|left| left <= reserve) {
            Some(next_quarter_hour(now))
        } else {
            None
        }
    }
}

/// Strava's 15-minute windows start on the hour and at 15, 30 and 45
/// past.
pub fn next_quarter_hour(now: DateTime<Utc>) -> DateTime<Utc> {
    let into = (now.minute() % 15) as i64 * 60 + now.second() as i64;
    now - Duration::nanoseconds(now.nanosecond() as i64) + Duration::seconds(15 * 60 - into)
}

/// The daily limit resets at midnight UTC.
pub fn next_utc_midnight(now: DateTime<Utc>) -> DateTime<Utc> {
    (now.date_naive() + Duration::days(1))
        .and_time(NaiveTime::MIN)
        .and_utc()
}

impl Client {
    pub fn new(http: reqwest::Client) -> Self {
        Self::with_base(http, "https://www.strava.com")
    }

    pub fn with_base(http: reqwest::Client, base: &str) -> Self {
        Self {
            http,
            base: base.trim_end_matches('/').to_string(),
        }
    }

    /// Where the browser goes to let the person's app read their
    /// activities. `redirect_uri` must be on the app's Authorization
    /// Callback Domain.
    pub fn authorize_url(&self, client_id: &str, redirect_uri: &str, state: &str) -> String {
        format!(
            "{}/oauth/authorize?client_id={}&redirect_uri={}&response_type=code\
             &approval_prompt=auto&scope={}&state={}",
            self.base,
            urlencoding::encode(client_id),
            urlencoding::encode(redirect_uri),
            urlencoding::encode(SCOPE),
            urlencoding::encode(state),
        )
    }

    /// The code from the callback, for the person's tokens and who they
    /// are on Strava.
    pub async fn exchange(&self, app: &StravaApp, code: &str) -> Result<(StravaTokens, Athlete)> {
        let body = self
            .token(
                app,
                &[("code", code), ("grant_type", "authorization_code")],
            )
            .await
            .map_err(|e| match e {
                Error::Unauthorized
                | Error::Strava {
                    status: Some(400),
                    ..
                } => Error::Strava {
                    status: Some(400),
                    message: "Strava refused the sign-in with this app's Client ID and                               Secret; check them against strava.com/settings/api"
                        .into(),
                },
                e => e,
            })?;
        let tokens = tokens(&body)?;
        let athlete = body.get("athlete").unwrap_or(&Value::Null);
        let name = [athlete.get("firstname"), athlete.get("lastname")]
            .into_iter()
            .filter_map(|n| n.and_then(Value::as_str))
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        let athlete = Athlete {
            id: athlete
                .get("id")
                .and_then(Value::as_i64)
                .unwrap_or_default(),
            name,
        };
        Ok((tokens, athlete))
    }

    /// Fresh tokens. The refresh token that comes back may be a new
    /// one, and then the old one is already dead: keep what this
    /// returns, all of it.
    pub async fn refresh(&self, app: &StravaApp, refresh_token: &str) -> Result<StravaTokens> {
        let body = self
            .token(
                app,
                &[
                    ("refresh_token", refresh_token),
                    ("grant_type", "refresh_token"),
                ],
            )
            .await
            // A refresh token Strava no longer knows — revoked on
            // strava.com, or already rotated away — is a 400 here,
            // not a 401.
            .map_err(|e| match e {
                Error::Strava {
                    status: Some(400), ..
                } => Error::Unauthorized,
                e => e,
            })?;
        tokens(&body)
    }

    async fn token(&self, app: &StravaApp, extra: &[(&str, &str)]) -> Result<Value> {
        let mut form = vec![
            ("client_id", app.client_id.as_str()),
            ("client_secret", app.client_secret.as_str()),
        ];
        form.extend_from_slice(extra);
        let (body, _) = send(
            self.http
                .post(format!("{}/oauth/token", self.base))
                .form(&form),
        )
        .await?;
        Ok(body)
    }

    /// Takes back everything the person granted the app: Strava's
    /// `/oauth/revoke`, the deauthorization it recommends since June
    /// 2026 (`/oauth/deauthorize` goes in June 2027). Revoking the
    /// refresh token revokes its access tokens too.
    pub async fn revoke(&self, app: &StravaApp, refresh_token: &str) -> Result<()> {
        send(
            self.http
                .post(format!("{}/oauth/revoke", self.base))
                .basic_auth(&app.client_id, Some(&app.client_secret))
                .form(&[
                    ("token", refresh_token),
                    ("token_type_hint", "refresh_token"),
                ]),
        )
        .await
        .map(|_| ())
    }

    /// One page of the person's activities that started after `after`,
    /// as summaries.
    pub async fn list(
        &self,
        access_token: &str,
        after: DateTime<Utc>,
        page: usize,
    ) -> Result<(Vec<Value>, Budget)> {
        let (body, budget) = send(
            self.http
                .get(format!("{}/api/v3/athlete/activities", self.base))
                .bearer_auth(access_token)
                .query(&[
                    ("after", after.timestamp().to_string()),
                    ("page", page.to_string()),
                    ("per_page", PER_PAGE.to_string()),
                ]),
        )
        .await?;
        match body {
            Value::Array(items) => Ok((items, budget)),
            other => Err(Error::Strava {
                status: None,
                message: format!("expected a list, got {other}"),
            }),
        }
    }

    /// One activity in detail: calories and the kilometre splits are
    /// only in this, not in the list.
    pub async fn detail(&self, access_token: &str, id: &str) -> Result<(Value, Budget)> {
        send(
            self.http
                .get(format!("{}/api/v3/activities/{id}", self.base))
                .bearer_auth(access_token)
                .query(&[("include_all_efforts", "false")]),
        )
        .await
    }
}

async fn send(request: RequestBuilder) -> Result<(Value, Budget)> {
    let response = request.send().await.map_err(|e| Error::Strava {
        status: None,
        message: e.without_url().to_string(),
    })?;
    let status = response.status();
    let headers = response.headers().clone();
    let budget = Budget::from_headers(&headers);
    let text = response.text().await.unwrap_or_default();
    match status {
        s if s.is_success() => {
            if text.trim().is_empty() {
                return Ok((Value::Null, budget));
            }
            let body = serde_json::from_str(&text).map_err(|e| Error::Strava {
                status: Some(s.as_u16()),
                message: format!("unreadable answer: {e}"),
            })?;
            Ok((body, budget))
        }
        StatusCode::TOO_MANY_REQUESTS => {
            let now = Utc::now();
            // The window that ran out decides; with no headers to say,
            // the next 15-minute window is the shortest honest wait.
            let until = budget
                .exhausted_until(0, now)
                .unwrap_or_else(|| next_quarter_hour(now));
            Err(Error::RateLimited { until })
        }
        StatusCode::UNAUTHORIZED => Err(Error::Unauthorized),
        s => {
            let message = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| v.get("message").and_then(Value::as_str).map(String::from))
                .unwrap_or_else(|| text.chars().take(200).collect());
            Err(Error::Strava {
                status: Some(s.as_u16()),
                message: format!("{} {message}", s.as_u16()),
            })
        }
    }
}

fn tokens(body: &Value) -> Result<StravaTokens> {
    let text = |key: &str| {
        body.get(key)
            .and_then(Value::as_str)
            .map(String::from)
            .ok_or_else(|| Error::Strava {
                status: None,
                message: format!("no {key} came back"),
            })
    };
    let expires_at = body
        .get("expires_at")
        .and_then(Value::as_i64)
        .and_then(|s| DateTime::from_timestamp(s, 0))
        .ok_or_else(|| Error::Strava {
            status: None,
            message: "no expires_at came back".into(),
        })?;
    Ok(StravaTokens {
        access_token: text("access_token")?,
        refresh_token: text("refresh_token")?,
        expires_at,
    })
}

/// Whether what the person allowed on Strava's consent screen lets
/// Niles read their activities. They can untick a scope there.
pub fn scope_allows_activities(granted: &str) -> bool {
    granted
        .split([',', ' '])
        .any(|s| s == "activity:read" || s == "activity:read_all")
}

/// A detailed activity, as Niles keeps it. Strava's units are already
/// Niles's: metres, seconds, beats per minute, kilocalories.
pub fn activity(detail: &Value) -> Option<Activity> {
    let id = match detail.get("id")? {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        _ => return None,
    };
    let started_at = DateTime::parse_from_rfc3339(detail.get("start_date")?.as_str()?)
        .ok()?
        .to_utc();
    let number = |key: &str| detail.get(key).and_then(Value::as_f64);
    // Zero is how Strava says "none" for a distance or calories: a
    // treadmill run without a foot pod, a strength session.
    let positive = |key: &str| number(key).filter(|v| *v > 0.0);
    let sport = detail
        .get("sport_type")
        .or_else(|| detail.get("type"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    Some(Activity {
        source: SOURCE.into(),
        external_id: id,
        started_at,
        kind: Kind::from_sport_type(sport),
        name: detail
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(sport)
            .to_string(),
        distance_m: positive("distance"),
        duration_s: number("elapsed_time"),
        moving_s: number("moving_time"),
        avg_hr: positive("average_heartrate"),
        max_hr: positive("max_heartrate"),
        elevation_gain_m: number("total_elevation_gain"),
        energy_kcal: positive("calories"),
        raw: detail.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderValue;
    use serde_json::json;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().to_utc()
    }

    #[test]
    fn a_run_with_heart_rate_is_kept_in_canonical_units() {
        let detail = json!({
            "id": 12345678987654321_i64,
            "name": "Morning Run",
            "sport_type": "Run",
            "type": "Run",
            "start_date": "2026-10-14T05:00:00Z",
            "distance": 10012.5,
            "moving_time": 3300,
            "elapsed_time": 3420,
            "total_elevation_gain": 42.0,
            "has_heartrate": true,
            "average_heartrate": 151.3,
            "max_heartrate": 178.0,
            "calories": 712.0,
            "splits_metric": [{ "distance": 1000.0, "moving_time": 330, "split": 1 }]
        });
        let a = activity(&detail).unwrap();
        assert_eq!(a.source, "strava");
        assert_eq!(a.external_id, "12345678987654321");
        assert_eq!(a.started_at, at("2026-10-14T05:00:00Z"));
        assert_eq!(a.kind, Kind::Run);
        assert_eq!(a.name, "Morning Run");
        assert_eq!(a.distance_m, Some(10012.5));
        assert_eq!(a.moving_s, Some(3300.0));
        assert_eq!(a.duration_s, Some(3420.0));
        assert_eq!(a.elevation_gain_m, Some(42.0));
        assert_eq!((a.avg_hr, a.max_hr), (Some(151.3), Some(178.0)));
        assert_eq!(a.energy_kcal, Some(712.0));
        assert_eq!(a.raw["splits_metric"][0]["split"], 1, "the splits are kept");
    }

    #[test]
    fn a_ride_without_heart_rate_has_none() {
        let detail = json!({
            "id": 9,
            "name": "Commute",
            "sport_type": "GravelRide",
            "start_date": "2026-10-13T16:30:00Z",
            "distance": 15000.0,
            "moving_time": 2700,
            "elapsed_time": 2900,
            "total_elevation_gain": 0.0,
            "has_heartrate": false,
            "calories": 0.0
        });
        let a = activity(&detail).unwrap();
        assert_eq!(a.kind, Kind::Ride);
        assert_eq!((a.avg_hr, a.max_hr), (None, None));
        assert_eq!(a.energy_kcal, None, "zero calories is none known");
        assert_eq!(a.elevation_gain_m, Some(0.0), "flat is a real answer");
    }

    #[test]
    fn a_strength_session_has_no_distance_and_an_old_type_still_reads() {
        let detail = json!({
            "id": 10,
            "type": "WeightTraining",
            "start_date": "2026-10-12T17:00:00Z",
            "distance": 0.0,
            "moving_time": 2400,
            "elapsed_time": 2400
        });
        let a = activity(&detail).unwrap();
        assert_eq!(a.kind, Kind::Strength);
        assert_eq!(a.distance_m, None);
        assert_eq!(a.name, "WeightTraining");
    }

    #[test]
    fn without_an_id_or_a_start_there_is_no_activity() {
        assert!(activity(&json!({ "start_date": "2026-10-12T17:00:00Z" })).is_none());
        assert!(activity(&json!({ "id": 1 })).is_none());
    }

    #[test]
    fn the_budget_is_the_least_left_of_either_limit() {
        let mut headers = HeaderMap::new();
        let mut put = |k: &'static str, v: &str| {
            headers.insert(k, HeaderValue::from_str(v).unwrap());
        };
        put("x-ratelimit-limit", "200,2000");
        put("x-ratelimit-usage", "20,300");
        put("x-readratelimit-limit", "100,1000");
        put("x-readratelimit-usage", "97,400");
        let budget = Budget::from_headers(&headers);
        assert_eq!(budget.short_left, Some(3));
        assert_eq!(budget.daily_left, Some(600));

        let now = at("2026-10-15T10:07:30Z");
        assert_eq!(
            budget.exhausted_until(3, now),
            Some(at("2026-10-15T10:15:00Z"))
        );
        assert_eq!(budget.exhausted_until(2, now), None);
        assert_eq!(Budget::from_headers(&HeaderMap::new()), Budget::default());
    }

    #[test]
    fn a_spent_day_waits_for_midnight_utc() {
        let budget = Budget {
            short_left: Some(50),
            daily_left: Some(0),
        };
        assert_eq!(
            budget.exhausted_until(0, at("2026-10-15T22:59:00Z")),
            Some(at("2026-10-16T00:00:00Z"))
        );
    }

    #[test]
    fn windows_turn_on_the_quarter_hour() {
        assert_eq!(
            next_quarter_hour(at("2026-10-15T10:44:59Z")),
            at("2026-10-15T10:45:00Z")
        );
        assert_eq!(
            next_quarter_hour(at("2026-10-15T10:45:00Z")),
            at("2026-10-15T11:00:00Z")
        );
    }

    #[test]
    fn the_scope_must_reach_activities() {
        assert!(scope_allows_activities("read,activity:read_all"));
        assert!(scope_allows_activities("read activity:read"));
        assert!(!scope_allows_activities("read"));
        assert!(!scope_allows_activities(""));
    }

    #[test]
    fn the_sign_in_asks_for_private_activities_and_comes_back_here() {
        let client = Client::new(reqwest::Client::new());
        let url = client.authorize_url("12345", "https://niles.example/me/strava/callback", "s1");
        assert!(url.starts_with("https://www.strava.com/oauth/authorize?client_id=12345&"));
        assert!(url.contains("redirect_uri=https%3A%2F%2Fniles.example%2Fme%2Fstrava%2Fcallback"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("scope=activity%3Aread_all"));
        assert!(url.contains("state=s1"));
    }
}
