//! Finding where the house is, and what clock it keeps.
//!
//! Latitude and longitude are the two settings nobody knows off the
//! top of their head, and typing them wrong is silent — the weather is
//! simply somebody else's. So Niles looks them up from a place name
//! instead.
//!
//! It searches places rather than street addresses. What reads these is
//! the weather, which is a regional thing; a house number would change
//! the fourth decimal and nothing else. The real reason for this
//! particular service, though, is the third field it returns: the
//! timezone. That is the other setting a first start gets wrong, it is
//! the one that puts the whole lighting curve an hour out, and asking
//! the same question twice would be two chances to disagree.

use axum::Json;
use axum::extract::Query;
use axum::http::StatusCode;

type Failure = (StatusCode, String);

/// A place somebody might live in, with the two things Niles wants
/// from it.
#[derive(Debug, Clone, serde::Serialize, PartialEq)]
pub struct Place {
    /// What to show in the list: "Aarhus, Central Denmark Region,
    /// Denmark". Assembled here because the parts arrive separately
    /// and any of them can be missing.
    pub label: String,
    pub latitude: f64,
    pub longitude: f64,
    /// IANA zone, e.g. `Europe/Copenhagen`.
    pub timezone: String,
    /// ISO-3166-1 alpha-2, which is what decides metric or imperial.
    pub country_code: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct Search {
    pub q: String,
}

/// `GET /places?q=` — places matching a name.
///
/// An empty list is a normal answer, not an error: somebody is still
/// typing, or their village is not in the index and they will have to
/// enter the numbers themselves.
pub async fn search(Query(query): Query<Search>) -> Result<Json<Vec<Place>>, Failure> {
    let q = query.q.trim();
    if q.is_empty() {
        return Ok(Json(Vec::new()));
    }
    let url = format!(
        "https://geocoding-api.open-meteo.com/v1/search?count=8&format=json&name={}",
        urlencoding(q)
    );
    let response = reqwest::Client::new()
        .get(&url)
        .send()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("could not reach it: {e}")))?;
    if !response.status().is_success() {
        return Err((
            StatusCode::BAD_GATEWAY,
            format!("the place index answered {}", response.status()),
        ));
    }
    let body: GeocodeResponse = response
        .json()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("unreadable answer: {e}")))?;
    Ok(Json(body.results.into_iter().map(Place::from).collect()))
}

/// `GET /timezones` — every IANA zone Niles can be set to.
///
/// A list rather than a free-text box, because a zone that does not
/// parse is refused at startup and a zone that parses but is the wrong
/// one is not refused at all — it just runs the curve to the wrong
/// clock.
pub async fn timezones() -> Json<Vec<&'static str>> {
    Json(chrono_tz::TZ_VARIANTS.iter().map(|tz| tz.name()).collect())
}

#[derive(serde::Deserialize, Default)]
struct GeocodeResponse {
    /// Absent entirely when nothing matched, rather than empty.
    #[serde(default)]
    results: Vec<GeocodeResult>,
}

#[derive(serde::Deserialize)]
struct GeocodeResult {
    name: String,
    latitude: f64,
    longitude: f64,
    timezone: String,
    country: Option<String>,
    country_code: Option<String>,
    /// The region within the country — a state, a county. Often
    /// missing, and the only thing separating two towns of one name.
    admin1: Option<String>,
}

impl From<GeocodeResult> for Place {
    fn from(r: GeocodeResult) -> Self {
        let label = [Some(r.name), r.admin1, r.country]
            .into_iter()
            .flatten()
            .filter(|part| !part.trim().is_empty())
            .collect::<Vec<_>>()
            .join(", ");
        Self {
            label,
            latitude: r.latitude,
            longitude: r.longitude,
            timezone: r.timezone,
            country_code: r.country_code,
        }
    }
}

/// Percent-encode a query. Narrow on purpose: this escapes everything
/// that is not unreserved, which is more than a URL strictly needs and
/// exactly what a search box full of spaces, commas and accents needs.
fn urlencoding(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(name: &str, admin1: Option<&str>, country: Option<&str>) -> GeocodeResult {
        GeocodeResult {
            name: name.into(),
            latitude: 56.1572,
            longitude: 10.2107,
            timezone: "Europe/Copenhagen".into(),
            country: country.map(Into::into),
            country_code: Some("DK".into()),
            admin1: admin1.map(Into::into),
        }
    }

    #[test]
    fn a_place_is_named_specifically_enough_to_choose_between_two() {
        // Two Springfields is the whole reason the region is in there.
        let place = Place::from(result(
            "Aarhus",
            Some("Central Denmark Region"),
            Some("Denmark"),
        ));
        assert_eq!(place.label, "Aarhus, Central Denmark Region, Denmark");
    }

    #[test]
    fn missing_parts_do_not_leave_stray_commas() {
        let place = Place::from(result("Somewhere", None, Some("Denmark")));
        assert_eq!(place.label, "Somewhere, Denmark");
    }

    #[test]
    fn nothing_found_parses_as_nothing_found() {
        // The service omits `results` rather than sending an empty one,
        // and a missing key must not read as a failed request — the
        // difference between "no such village" and "the internet is
        // down" is the difference between two very different messages.
        let body: GeocodeResponse =
            serde_json::from_str(r#"{"generationtime_ms":0.4}"#).expect("parses");
        assert!(body.results.is_empty());
    }

    #[test]
    fn a_query_survives_spaces_and_accents() {
        assert_eq!(urlencoding("Aarhus C"), "Aarhus%20C");
        assert_eq!(urlencoding("Malmö"), "Malm%C3%B6");
    }

    #[test]
    fn the_timezone_list_holds_real_zones() {
        let zones = chrono_tz::TZ_VARIANTS;
        assert!(zones.iter().any(|tz| tz.name() == "Europe/Copenhagen"));
        assert!(zones.iter().any(|tz| tz.name() == "UTC"));
    }
}
