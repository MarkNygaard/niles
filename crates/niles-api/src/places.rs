//! Finding where the house is, and what clock it keeps.
//!
//! Latitude and longitude are the two settings nobody knows off the
//! top of their head, and typing them wrong is silent — the weather is
//! simply somebody else's. So Niles looks them up instead.
//!
//! OpenStreetMap's Nominatim does the finding, because it knows streets
//! as well as towns: a house is somewhere, and something that delivers
//! to it will one day need to know where. It does not know timezones,
//! which are the other setting a first start gets wrong — the one that
//! puts the whole lighting curve an hour out — so each place found is
//! asked of Open-Meteo for its zone, by its coordinates.

use axum::Json;
use axum::extract::Query;
use axum::http::{HeaderMap, StatusCode, header};

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
    /// IANA zone, e.g. `Europe/Copenhagen`. Absent when the zone could
    /// not be looked up, which leaves the one already set alone.
    pub timezone: Option<String>,
    /// "Vestergade 12, 8000 Aarhus" — only for a street address, not
    /// for a town.
    pub address: Option<String>,
    /// ISO-3166-1 alpha-2, which is what decides metric or imperial.
    pub country_code: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct Search {
    pub q: String,
}

/// `GET /places?q=` — places and street addresses matching what was
/// typed, each with its timezone.
///
/// An empty list is a normal answer, not an error: somebody is still
/// typing, or the address is not in the map and they will have to drop
/// the pin themselves.
pub async fn search(
    Query(query): Query<Search>,
    headers: HeaderMap,
) -> Result<Json<Vec<Place>>, Failure> {
    let q = query.q.trim();
    if q.is_empty() {
        return Ok(Json(Vec::new()));
    }
    let client = reqwest::Client::new();
    // Nominatim's terms: say who is asking. And in the asker's language,
    // so a Danish house reads "Aarhus" and not "Århus" or the reverse.
    let language = headers
        .get(header::ACCEPT_LANGUAGE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("en");
    let response = client
        .get(format!(
            "https://nominatim.openstreetmap.org/search?format=jsonv2&addressdetails=1&limit=6&q={}",
            urlencoding(q)
        ))
        .header(header::USER_AGENT, USER_AGENT)
        .header(header::ACCEPT_LANGUAGE, language)
        .send()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("could not reach it: {e}")))?;
    if !response.status().is_success() {
        return Err((
            StatusCode::BAD_GATEWAY,
            format!("the map answered {}", response.status()),
        ));
    }
    let found: Vec<Found> = response
        .json()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("unreadable answer: {e}")))?;

    let mut places: Vec<Place> = Vec::new();
    for place in found.into_iter().filter_map(Found::into_place) {
        // The same street from two map objects — the road and a building
        // on it — is one choice, not two.
        if !places.iter().any(|p| p.label == place.label) {
            places.push(place);
        }
    }
    let zones = futures_util::future::join_all(
        places
            .iter()
            .map(|p| timezone_at(&client, p.latitude, p.longitude)),
    )
    .await;
    for (place, zone) in places.iter_mut().zip(zones) {
        place.timezone = zone;
    }
    Ok(Json(places))
}

const USER_AGENT: &str = concat!(
    "Niles/",
    env!("CARGO_PKG_VERSION"),
    " (home automation; https://github.com/MarkNygaard/niles)"
);

/// The IANA zone at a point, or `None` when it cannot be had — which
/// costs the convenience, not the place.
async fn timezone_at(client: &reqwest::Client, latitude: f64, longitude: f64) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct Zone {
        timezone: String,
    }
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={latitude}&longitude={longitude}&timezone=auto&forecast_days=1"
    );
    let zone: Zone = client.get(url).send().await.ok()?.json().await.ok()?;
    // Only a zone Niles can be set to: one that does not parse is
    // refused at startup.
    zone.timezone
        .parse::<chrono_tz::Tz>()
        .ok()
        .map(|tz| tz.name().to_string())
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

/// One thing Nominatim found.
#[derive(serde::Deserialize)]
struct Found {
    /// Strings, in Nominatim's JSON.
    lat: String,
    lon: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    address: FoundAddress,
}

#[derive(serde::Deserialize, Default)]
struct FoundAddress {
    road: Option<String>,
    house_number: Option<String>,
    postcode: Option<String>,
    city: Option<String>,
    town: Option<String>,
    village: Option<String>,
    hamlet: Option<String>,
    municipality: Option<String>,
    state: Option<String>,
    country: Option<String>,
    country_code: Option<String>,
}

impl Found {
    fn into_place(self) -> Option<Place> {
        let latitude = self.lat.parse().ok()?;
        let longitude = self.lon.parse().ok()?;
        let a = self.address;
        let town = a
            .city
            .or(a.town)
            .or(a.village)
            .or(a.hamlet)
            .or(a.municipality);
        // Street before number, postcode before town: the way most of
        // Europe writes it, and the way this house's country does.
        let street = a.road.map(|road| join(&[Some(road), a.house_number], " "));
        let locality = join(&[a.postcode, town.clone()], " ");
        let (label, address) = match street {
            Some(street) => {
                let address = join(&[Some(street), nonempty(locality)], ", ");
                (
                    join(&[Some(address.clone()), a.country], ", "),
                    Some(address),
                )
            }
            None => (join(&[self.name.or(town), a.state, a.country], ", "), None),
        };
        if label.is_empty() {
            return None;
        }
        Some(Place {
            label,
            latitude,
            longitude,
            timezone: None,
            address,
            country_code: a.country_code.map(|c| c.to_uppercase()),
        })
    }
}

fn nonempty(s: String) -> Option<String> {
    (!s.trim().is_empty()).then_some(s)
}

fn join(parts: &[Option<String>], separator: &str) -> String {
    parts
        .iter()
        .flatten()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(separator)
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

    fn found(json: &str) -> Option<Place> {
        serde_json::from_str::<Found>(json)
            .expect("parses")
            .into_place()
    }

    #[test]
    fn a_street_address_reads_the_way_it_is_written_here() {
        let place = found(
            r#"{"lat":"56.1567","lon":"10.2039","name":"",
                "address":{"road":"Vestergade","house_number":"12","postcode":"8000",
                           "city":"Aarhus","state":"Central Denmark Region",
                           "country":"Denmark","country_code":"dk"}}"#,
        )
        .unwrap();
        assert_eq!(place.address.as_deref(), Some("Vestergade 12, 8000 Aarhus"));
        assert_eq!(place.label, "Vestergade 12, 8000 Aarhus, Denmark");
        assert_eq!(place.country_code.as_deref(), Some("DK"));
        assert!((place.latitude - 56.1567).abs() < 1e-9);
    }

    #[test]
    fn a_town_is_named_specifically_enough_to_choose_between_two() {
        // Two Springfields is the whole reason the region is in there.
        let place = found(
            r#"{"lat":"56.15","lon":"10.21","name":"Aarhus",
                "address":{"city":"Aarhus","state":"Central Denmark Region",
                           "country":"Denmark","country_code":"dk"}}"#,
        )
        .unwrap();
        assert_eq!(place.label, "Aarhus, Central Denmark Region, Denmark");
        assert_eq!(place.address, None);
    }

    #[test]
    fn missing_parts_do_not_leave_stray_commas() {
        let place =
            found(r#"{"lat":"1","lon":"2","address":{"road":"Bygaden","village":"Lille By"}}"#)
                .unwrap();
        assert_eq!(place.label, "Bygaden, Lille By");
    }

    #[test]
    fn a_result_without_coordinates_is_dropped() {
        assert!(found(r#"{"lat":"","lon":"2","name":"Nowhere"}"#).is_none());
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
