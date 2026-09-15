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
use niles_core::canonicalize_name;
use serde::Deserialize;

/// One heating zone, with whatever it is currently reporting.
#[derive(Debug, Clone, PartialEq)]
pub struct Zone {
    pub id: u64,
    /// As tado has it — "Living Room", free text somebody typed.
    pub name: String,
    /// The Niles room this looks like, by name alone.
    ///
    /// `None` when nothing matches, which is a thing to say rather than
    /// to guess at: a zone Niles cannot place is better shown unplaced
    /// than quietly attached to the wrong room.
    pub room: Option<String>,
    /// What the thermostat reads.
    pub temperature: Option<f32>,
    pub humidity: Option<f32>,
    /// What it is trying to reach. `None` when the zone is off, which
    /// is different from zero.
    pub target: Option<f32>,
    /// Whether it is heating at all.
    pub on: bool,
    /// Whether somebody has overridden the schedule.
    pub overridden: bool,
    /// False when tado says the zone is offline — a dead battery, or a
    /// valve out of range.
    pub reachable: bool,
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
    pub async fn zones(&self, rooms: &[String]) -> Result<Vec<Zone>> {
        let listed = self.list_zones().await?;
        let mut zones = Vec::with_capacity(listed.len());
        for zone in listed.into_iter().filter(|z| z.kind == "HEATING") {
            let state = self.zone_state(zone.id).await?;
            zones.push(Zone {
                room: match_room(&zone.name, rooms),
                id: zone.id,
                name: zone.name,
                temperature: state
                    .sensor_data_points
                    .inside_temperature
                    .map(|t| t.celsius),
                humidity: state.sensor_data_points.humidity.map(|h| h.percentage),
                target: state
                    .setting
                    .temperature
                    .filter(|_| state.setting.power == "ON")
                    .map(|t| t.celsius),
                on: state.setting.power == "ON",
                overridden: state.overlay.is_some(),
                // Absent means tado said nothing about the link, which
                // for a zone that answered at all is far likelier to be
                // a shape we have not seen than a dead valve.
                reachable: state.link.is_none_or(|l| l.state == "ONLINE"),
            });
        }
        Ok(zones)
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

/// The Niles room a zone name looks like.
///
/// By canonical name and nothing else. "Living Room" is `living_room`,
/// which is what the room is already called — and where it is not, the
/// answer is nothing rather than a guess. Somebody can say which room
/// they meant; nobody can undo a radiator quietly attached to the wrong
/// one.
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
    overlay: Option<serde_json::Value>,
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
