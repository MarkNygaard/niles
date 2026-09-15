//! Tado heating zones — what each room is, and what it is doing.
//!
//! In the presence crate, and not because heating is presence. The
//! token is why: tado issues a new refresh token every time the old one
//! is used, so two clients refreshing independently would invalidate
//! each other within minutes. There is one tado session, and whatever
//! reads zones has to be the thing that already holds it.
//!
//! (When this grows writes — a setpoint, a schedule — the tado client
//! should become its own crate that presence and climate both use. It
//! is not worth the file move for reads alone.)

use crate::error::{Error, Result};
use crate::tado::TadoSource;
use chrono::{DateTime, Utc};
use niles_core::canonicalize_name;
use serde::Deserialize;

/// One heating zone, with whatever it is currently reporting.
#[derive(Debug, Clone, PartialEq)]
pub struct Zone {
    pub id: u64,
    /// As tado has it — "Living Room", free text somebody typed.
    pub name: String,
    /// The Niles room this zone is.
    ///
    /// `None` when nobody has said and the name does not give it away,
    /// which is a thing to report rather than guess at: a zone Niles
    /// cannot place is better shown unplaced than quietly attached to
    /// the wrong room.
    pub room: Option<String>,
    /// How it got that room, so the page can tell a saved answer from
    /// a lucky one.
    pub placed_by: Placed,
    /// What the thermostat reads, or `None` when it is not answering.
    ///
    /// tado keeps sending the last value it heard from an offline
    /// valve, with no hint that it is old. Passing that on would put a
    /// stale number beside live ones on the same card — the same
    /// mistake an unreachable Zigbee light made when it went on
    /// reporting itself as on at 100%.
    pub temperature: Option<f32>,
    pub humidity: Option<f32>,
    /// What it is trying to reach. `None` when the zone is off, which
    /// is different from zero.
    ///
    /// Kept even when the valve is unreachable, unlike the readings
    /// above: a setpoint is a setting tado holds, not something the
    /// valve reports, so it is still true — it is just not being
    /// reached.
    pub target: Option<f32>,
    /// Whether it is heating at all.
    pub on: bool,
    /// Whether somebody has overridden the schedule.
    pub overridden: bool,
    /// When the override ends, when it ends by itself.
    ///
    /// `None` covers both "no override" and "until you resume it",
    /// which the page tells apart by looking at `overridden` — an
    /// override with no end is the one worth pointing at, because it
    /// is the one somebody has to remember.
    pub until: Option<DateTime<Utc>>,
    /// False when tado says the zone is offline — a dead battery, or a
    /// valve out of range.
    pub reachable: bool,
}

/// How a zone came to have a room.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Placed {
    /// Somebody said so, in Settings.
    Paired,
    /// The zone name is a room name. Convenient when it happens, and
    /// worth distinguishing: it changes if either name changes, and
    /// nobody chose it.
    Name,
    /// Neither. The zone is real and Niles does not know where it is.
    Nowhere,
}

impl TadoSource {
    /// Every heating zone, with its current state.
    ///
    /// Hot water zones are left out: they are not a room and there is
    /// nothing in the house they correspond to.
    ///
    /// One request for the list and one per zone. Tado does publish a
    /// bulk `zoneStates`, but the per-zone shape is the documented one
    /// and a house has a handful of zones — a poll every few minutes
    /// costs nothing worth optimising for.
    pub async fn zones(
        &self,
        rooms: &[String],
        paired: &std::collections::HashMap<String, String>,
    ) -> Result<Vec<Zone>> {
        let listed = self.list_zones().await?;
        let mut zones = Vec::with_capacity(listed.len());
        for zone in listed.into_iter().filter(|z| z.kind == "HEATING") {
            let state = self.zone_state(zone.id).await?;
            let (room, placed_by) = place(zone.id, &zone.name, rooms, paired);
            // Absent means tado said nothing about the link, which for a
            // zone that answered at all is likelier to be a shape we
            // have not seen than a dead valve.
            let reachable = state.link.as_ref().is_none_or(|l| l.state == "ONLINE");
            zones.push(Zone {
                room,
                placed_by,
                id: zone.id,
                name: zone.name,
                temperature: state
                    .sensor_data_points
                    .inside_temperature
                    .filter(|_| reachable)
                    .map(|t| t.celsius),
                humidity: state
                    .sensor_data_points
                    .humidity
                    .filter(|_| reachable)
                    .map(|h| h.percentage),
                target: state
                    .setting
                    .temperature
                    .filter(|_| state.setting.power == "ON")
                    .map(|t| t.celsius),
                on: state.setting.power == "ON",
                overridden: state.overlay.is_some(),
                until: state
                    .overlay
                    .and_then(|o| o.termination)
                    .and_then(|t| t.expiry),
                reachable,
            });
        }
        Ok(zones)
    }

    /// Hold a zone at a temperature until somebody says otherwise.
    ///
    /// `MANUAL` termination, which is tado's "until you resume
    /// schedule" — and the only one worth offering without a way to
    /// pick a duration. A timer that ends at a time nobody chose would
    /// be a surprise in the other direction.
    pub async fn set_temperature(&self, zone: u64, celsius: f32) -> Result<()> {
        let body = serde_json::json!({
            "setting": {
                "type": "HEATING",
                "power": "ON",
                "temperature": { "celsius": celsius },
            },
            "termination": { "type": "MANUAL" },
        });
        self.write_home_path(&format!("zones/{zone}/overlay"), Some(body.to_string()))
            .await
            .map(|_| ())
    }

    /// Hold a zone at a temperature for a while, then let go.
    ///
    /// `TIMER` termination, which is the other half of what tado's own
    /// app offers and the only one that suits a boost: a room warmed
    /// for half an hour should go back to its schedule by itself, or
    /// the house spends the evening at 25° because nobody remembered.
    pub async fn boost(&self, zone: u64, celsius: f32, seconds: u32) -> Result<()> {
        let body = serde_json::json!({
            "setting": {
                "type": "HEATING",
                "power": "ON",
                "temperature": { "celsius": celsius },
            },
            "termination": { "type": "TIMER", "durationInSeconds": seconds },
        });
        self.write_home_path(&format!("zones/{zone}/overlay"), Some(body.to_string()))
            .await
            .map(|_| ())
    }

    /// Boost every heating zone at once.
    ///
    /// The loop lives here rather than in the caller so that "warm the
    /// house" is one request from the page: doing it a zone at a time
    /// over HTTP means a page that is half-boosted while it waits, and
    /// a failure in the middle that only the browser knows about.
    ///
    /// Stops at the first refusal. Tado saying no is nearly always
    /// about the connection — an expired token, a rate limit — rather
    /// than about one zone, so carrying on would mostly mean making
    /// the same failed request several more times.
    pub async fn boost_all(&self, celsius: f32, seconds: u32) -> Result<usize> {
        let listed = self.list_zones().await?;
        let mut boosted = 0;
        for zone in listed.into_iter().filter(|z| z.kind == "HEATING") {
            self.boost(zone.id, celsius, seconds).await?;
            boosted += 1;
        }
        Ok(boosted)
    }

    /// Turn a zone off — which in tado means frost protection, not
    /// nothing: it still heats below about 5°C so the pipes survive.
    pub async fn turn_off(&self, zone: u64) -> Result<()> {
        let body = serde_json::json!({
            "setting": { "type": "HEATING", "power": "OFF" },
            "termination": { "type": "MANUAL" },
        });
        self.write_home_path(&format!("zones/{zone}/overlay"), Some(body.to_string()))
            .await
            .map(|_| ())
    }

    /// Hand several zones back to their schedules.
    ///
    /// Told which ones rather than working it out: the page already
    /// knows, from the same `/climate` it drew the button with, and
    /// asking tado again would be a second opinion about a thing the
    /// caller is looking at. It also keeps this off the zones nobody
    /// asked about — a room somebody set by hand is not part of a
    /// boost and should not be swept up by ending one.
    pub async fn resume_all(&self, zones: &[u64]) -> Result<usize> {
        for zone in zones {
            self.resume_schedule(*zone).await?;
        }
        Ok(zones.len())
    }

    /// Drop the override and let the schedule have the zone back.
    ///
    /// Needs nothing from the schedule itself — removing the overlay is
    /// the whole operation, and tado falls back to whatever the
    /// timetable already said.
    pub async fn resume_schedule(&self, zone: u64) -> Result<()> {
        self.write_home_path(&format!("zones/{zone}/overlay"), None)
            .await
            .map(|_| ())
    }

    async fn list_zones(&self) -> Result<Vec<ListedZone>> {
        let body = self.get_home_path("zones").await?;
        serde_json::from_str(&body).map_err(|e| Error::Parse {
            reason: format!("zones: {e}"),
        })
    }

    async fn zone_state(&self, id: u64) -> Result<ZoneState> {
        let body = self.get_home_path(&format!("zones/{id}/state")).await?;
        serde_json::from_str(&body).map_err(|e| Error::Parse {
            reason: format!("zone {id} state: {e}"),
        })
    }
}

/// Which room a zone is in, and on whose authority.
///
/// A saved pairing wins outright, including over a name that happens to
/// match — somebody went and said so, and a zone renamed in the tado
/// app must not quietly move rooms underneath them.
///
/// Failing that, the name: "Living Room" is `living_room`, which is
/// what the room is already called. That is convenience rather than
/// truth, which is why it is reported as its own kind of answer.
///
/// Failing both, nothing. Nobody can undo a radiator quietly heating
/// the wrong room.
pub fn place(
    id: u64,
    zone_name: &str,
    rooms: &[String],
    paired: &std::collections::HashMap<String, String>,
) -> (Option<String>, Placed) {
    if let Some(room) = paired.get(&id.to_string()) {
        return (Some(room.clone()), Placed::Paired);
    }
    match match_room(zone_name, rooms) {
        Some(room) => (Some(room), Placed::Name),
        None => (None, Placed::Nowhere),
    }
}

/// The Niles room a zone name looks like, if any.
pub fn match_room(zone_name: &str, rooms: &[String]) -> Option<String> {
    let canonical = canonicalize_name(zone_name);
    rooms.iter().find(|room| **room == canonical).cloned()
}

#[derive(Debug, Deserialize)]
struct ListedZone {
    id: u64,
    name: String,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Debug, Deserialize)]
struct ZoneState {
    setting: Setting,
    #[serde(default)]
    overlay: Option<Overlay>,
    #[serde(default)]
    link: Option<Link>,
    #[serde(rename = "sensorDataPoints", default)]
    sensor_data_points: SensorDataPoints,
}

#[derive(Debug, Deserialize)]
struct Setting {
    /// `"ON"` or `"OFF"`. A zone that is off reports no temperature at
    /// all rather than a zero, which is why the target is an option.
    power: String,
    #[serde(default)]
    temperature: Option<Celsius>,
}

#[derive(Debug, Deserialize)]
struct Overlay {
    #[serde(default)]
    termination: Option<Termination>,
}

#[derive(Debug, Deserialize)]
struct Termination {
    /// Absent for a MANUAL override, which is what "until you resume
    /// schedule" is: it has no end until somebody gives it one.
    #[serde(default)]
    expiry: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
struct Link {
    state: String,
}

#[derive(Debug, Deserialize, Default)]
struct SensorDataPoints {
    #[serde(rename = "insideTemperature", default)]
    inside_temperature: Option<Celsius>,
    #[serde(default)]
    humidity: Option<Percentage>,
}

#[derive(Debug, Deserialize)]
struct Celsius {
    celsius: f32,
}

#[derive(Debug, Deserialize)]
struct Percentage {
    percentage: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rooms() -> Vec<String> {
        vec!["living_room".into(), "bedroom".into()]
    }

    fn nothing_paired() -> std::collections::HashMap<String, String> {
        std::collections::HashMap::new()
    }

    #[test]
    fn a_manual_override_has_no_end() {
        // Which is the one worth pointing at: it lasts until somebody
        // remembers it, and nothing else will end it.
        let state: ZoneState = serde_json::from_str(
            r#"{"setting":{"type":"HEATING","power":"OFF"},
                "overlay":{"type":"MANUAL","termination":{"type":"MANUAL"}}}"#,
        )
        .expect("parses");
        let overlay = state.overlay.expect("there is one");
        assert!(overlay.termination.expect("has one").expiry.is_none());
    }

    #[test]
    fn a_timed_override_says_when_it_ends() {
        let state: ZoneState = serde_json::from_str(
            r#"{"setting":{"type":"HEATING","power":"ON"},
                "overlay":{"termination":{"type":"TIMER",
                "expiry":"2026-09-15T18:30:00Z"}}}"#,
        )
        .expect("parses");
        assert!(
            state
                .overlay
                .and_then(|o| o.termination)
                .and_then(|t| t.expiry)
                .is_some()
        );
    }

    #[test]
    fn an_unreachable_valve_reports_no_reading_rather_than_an_old_one() {
        // tado goes on sending the last temperature it heard, with no
        // hint that it is hours old. Passing it on would put a stale
        // number beside live ones on the same card.
        let state: ZoneState = serde_json::from_str(
            r#"{
                "setting": {"type":"HEATING","power":"ON","temperature":{"celsius":23.0}},
                "link": {"state":"OFFLINE"},
                "sensorDataPoints": {
                    "insideTemperature": {"celsius": 19.24},
                    "humidity": {"percentage": 42.3}
                }
            }"#,
        )
        .expect("parses");
        let reachable = state.link.as_ref().is_none_or(|l| l.state == "ONLINE");
        assert!(!reachable);
        assert!(
            state
                .sensor_data_points
                .inside_temperature
                .filter(|_| reachable)
                .is_none()
        );
        assert_eq!(
            state.setting.temperature.map(|t| t.celsius),
            Some(23.0),
            "the setpoint is tado's own and stays true; only the readings go stale"
        );
    }

    #[test]
    fn a_saved_pairing_beats_a_matching_name() {
        // Somebody went and said so. A zone renamed in the tado app
        // must not quietly move rooms underneath them.
        let paired = std::collections::HashMap::from([("1".to_string(), "bedroom".to_string())]);
        assert_eq!(
            place(1, "Living Room", &rooms(), &paired),
            (Some("bedroom".into()), Placed::Paired)
        );
    }

    #[test]
    fn a_name_that_matches_is_reported_as_a_guess() {
        // Convenient when it happens, and not the same as being told:
        // it changes if either name changes, and nobody chose it.
        assert_eq!(
            place(1, "Living Room", &rooms(), &nothing_paired()),
            (Some("living_room".into()), Placed::Name)
        );
    }

    #[test]
    fn a_zone_named_in_another_language_waits_to_be_placed() {
        // The case the pairing exists for. Zone names are whatever
        // somebody typed into the tado app years ago, and matching
        // nothing must leave it unplaced rather than attached to the
        // nearest room.
        assert_eq!(
            place(3, "Stue", &rooms(), &nothing_paired()),
            (None, Placed::Nowhere)
        );
    }

    #[test]
    fn a_zone_lands_in_the_room_it_is_named_after() {
        assert_eq!(
            match_room("Living Room", &rooms()),
            Some("living_room".into())
        );
        assert_eq!(match_room("bedroom", &rooms()), Some("bedroom".into()));
    }

    #[test]
    fn a_zone_with_no_matching_room_is_left_unplaced() {
        // Rather than attached to the nearest thing. Somebody can say
        // which room they meant; nobody can undo a radiator quietly
        // heating the wrong one.
        assert_eq!(match_room("Hallway", &rooms()), None);
        assert_eq!(match_room("Kids Room", &rooms()), None);
    }

    #[test]
    fn a_heating_zone_parses_the_way_tado_sends_it() {
        let state: ZoneState = serde_json::from_str(
            r#"{
                "tadoMode": "HOME",
                "setting": {
                    "type": "HEATING",
                    "power": "ON",
                    "temperature": { "celsius": 19.5, "fahrenheit": 67.1 }
                },
                "overlay": null,
                "link": { "state": "ONLINE" },
                "sensorDataPoints": {
                    "insideTemperature": { "celsius": 19.37, "fahrenheit": 66.87 },
                    "humidity": { "type": "PERCENTAGE", "percentage": 58.7 }
                }
            }"#,
        )
        .expect("parses");
        assert_eq!(state.setting.power, "ON");
        assert_eq!(state.setting.temperature.map(|t| t.celsius), Some(19.5));
        assert_eq!(
            state
                .sensor_data_points
                .inside_temperature
                .map(|t| t.celsius),
            Some(19.37)
        );
        assert!(state.overlay.is_none());
    }

    #[test]
    fn a_zone_that_is_off_reports_no_target_rather_than_zero() {
        // tado omits the temperature entirely when the power is off,
        // and a zone "set to 0°" would read as somebody having asked
        // for that.
        let state: ZoneState = serde_json::from_str(
            r#"{"setting":{"type":"HEATING","power":"OFF"},"sensorDataPoints":{}}"#,
        )
        .expect("parses");
        assert_eq!(state.setting.power, "OFF");
        assert!(state.setting.temperature.is_none());
    }

    #[test]
    fn a_state_missing_everything_optional_still_parses() {
        // Half of this response is absent on a zone that has just been
        // added, and failing the whole poll over it would take presence
        // down with it.
        let state: ZoneState =
            serde_json::from_str(r#"{"setting":{"type":"HEATING","power":"ON"}}"#).expect("parses");
        assert!(state.link.is_none());
        assert!(state.sensor_data_points.humidity.is_none());
    }

    #[test]
    fn only_heating_zones_are_rooms() {
        let listed: Vec<ListedZone> = serde_json::from_str(
            r#"[{"id":1,"name":"Living Room","type":"HEATING"},
                {"id":0,"name":"Hot Water","type":"HOT_WATER"}]"#,
        )
        .expect("parses");
        let heating: Vec<_> = listed.iter().filter(|z| z.kind == "HEATING").collect();
        assert_eq!(heating.len(), 1);
        assert_eq!(heating[0].name, "Living Room");
    }
}
