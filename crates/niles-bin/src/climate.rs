//! Weather and heating by voice, answered without the LLM.
//!
//! Both have a fixed shape: the forecast for home, or a tado zone read
//! or changed. A round trip to a language model to decide which of five
//! things "set the bedroom to 19" means is a round trip that can only
//! add delay — and, for the heating, a chance of picking the wrong one.

use crate::DispatchCtx;
use crate::response::{self, DaySummary, Heated};
use niles_core::{RoomName, canonicalize_name};
use niles_intent::{ForecastDay, Intent};
use niles_presence::Zone;
use std::net::SocketAddr;

/// "what's the weather" / "will it rain tomorrow", for home.
pub(crate) async fn weather(ctx: &DispatchCtx, day: ForecastDay, rain: bool) -> String {
    let Some(client) = ctx.weather.as_ref() else {
        return response::weather_unavailable();
    };
    let metric = matches!(ctx.home.resolved_units(), niles_config::Units::Metric);
    let units = if metric {
        niles_weather::Units::Metric
    } else {
        niles_weather::Units::Imperial
    };
    let home = niles_weather::Location {
        name: None,
        latitude: ctx.home.latitude,
        longitude: ctx.home.longitude,
        country: None,
    };
    let report = match client.fetch_forecast(&home, 2, units, true).await {
        Ok(report) => report,
        Err(e) => {
            tracing::warn!("the forecast would not come: {e}");
            return response::weather_unavailable();
        }
    };

    let (index, word) = match day {
        ForecastDay::Today => (0, "today"),
        ForecastDay::Tomorrow => (1, "tomorrow"),
    };
    let Some(forecast) = report.daily.get(index) else {
        return response::weather_unavailable();
    };
    let summary = summarise(forecast, metric);

    if rain {
        return response::rain_on(word, &summary, metric);
    }
    match (day, report.current.as_ref()) {
        (ForecastDay::Today, Some(now)) => {
            response::weather_now(now.temperature, &now.weather_description, &summary)
        }
        _ => response::weather_on(word, &summary),
    }
}

fn summarise(day: &niles_weather::DailyForecast, metric: bool) -> DaySummary<'_> {
    DaySummary {
        description: &day.weather_description,
        low: day.temperature_min,
        high: day.temperature_max,
        precipitation: day.precipitation_sum,
        // Half a millimetre is a damp pavement, not a reason to take a
        // coat; below that the forecast is noise.
        wet: day.precipitation_sum >= if metric { 0.5 } else { 0.02 },
        // WMO snow codes: falls, grains, showers.
        snow: matches!(day.weather_code, 71..=77 | 85 | 86),
    }
}

/// Where a heating command applies.
#[derive(Debug, PartialEq)]
enum Place {
    Room(String),
    Everywhere,
}

impl Place {
    fn heated(&self) -> Heated<'_> {
        match self {
            Place::Room(room) => Heated::Room(room),
            Place::Everywhere => Heated::Everywhere,
        }
    }
}

#[derive(Debug, PartialEq)]
enum Miss {
    /// Nothing said, and a satellite Niles has no room for.
    WhichRoom,
}

/// Which zones a sentence means, decided before asking tado anything.
///
/// Unnamed is the room the satellite is in, which is what "turn the
/// heating up" means said in it.
fn wanted(said: Option<&str>, here: Option<&RoomName>) -> Result<Place, Miss> {
    match said {
        Some("everywhere") => Ok(Place::Everywhere),
        Some(room) => Ok(Place::Room(canonicalize_name(room))),
        None => Ok(Place::Room(
            here.ok_or(Miss::WhichRoom)?.as_str().to_string(),
        )),
    }
}

/// Whether a zone is one of them.
///
/// Matched on the room the zone was placed in, and on the zone's own
/// name as a fallback: "the office" should find a zone called Office
/// whether or not anybody paired it.
fn is_in(place: &Place, zone_name: &str, zone_room: Option<&str>) -> bool {
    match place {
        Place::Everywhere => true,
        Place::Room(key) => zone_room == Some(key.as_str()) || canonicalize_name(zone_name) == *key,
    }
}

/// A degree up or down from what the zone is aiming for.
///
/// From the target, not the reading: "turn it up" in a room that is
/// still warming means aim higher, not aim at a degree above however
/// cold it is right now. A zone that is off has no target, so it starts
/// from the room as it is.
fn stepped(zone: &Zone, up: bool) -> u16 {
    let from = zone
        .target
        .or(zone.temperature.map(|t| (t * 2.0).round() / 2.0))
        .unwrap_or(20.0);
    let to = if up { from + 1.0 } else { from - 1.0 };
    (to.clamp(5.0, 25.0) * 10.0).round() as u16
}

/// Any heating intent, against tado.
pub(crate) async fn heating(ctx: &DispatchCtx, peer: SocketAddr, intent: &Intent) -> String {
    let Some(tado) = ctx.tado.as_ref() else {
        return response::heating_unavailable();
    };
    let said = match intent {
        Intent::HeatingQuery { room }
        | Intent::HeatingSet { room, .. }
        | Intent::HeatingStep { room, .. }
        | Intent::HeatingOff { room }
        | Intent::HeatingResume { room } => room.as_deref(),
        _ => return response::heating_failed(),
    };

    // The same inputs the dashboard places zones with, so a room the
    // page shows as heated is the room the voice finds.
    let mut rooms: Vec<String> = ctx
        .registry
        .list_all()
        .into_iter()
        .map(|d| d.id.room().as_str().to_string())
        .collect();
    rooms.sort();
    rooms.dedup();
    let paired = ctx
        .settings
        .as_ref()
        .and_then(|s| s.current().presence.tado.as_ref().map(|t| t.rooms.clone()))
        .unwrap_or_default();

    let place = match wanted(said, ctx.satellites.room_for(peer)) {
        Ok(place) => place,
        Err(Miss::WhichRoom) => return response::heating_which_room(),
    };
    let zones = match tado
        .zones_where(&rooms, &paired, |name, room| is_in(&place, name, room))
        .await
    {
        Ok(zones) => zones,
        Err(e) => {
            tracing::warn!("[{peer}] tado would not list zones: {e}");
            return response::heating_failed();
        }
    };
    if zones.is_empty() {
        return response::heating_no_zone(match &place {
            Place::Room(room) => room,
            Place::Everywhere => "house",
        });
    }

    match intent {
        Intent::HeatingQuery { .. } => match &place {
            Place::Room(room) => {
                let z = &zones[0];
                response::heating_reading(room, z.temperature, z.target)
            }
            Place::Everywhere => response::heating_readings(
                &zones
                    .iter()
                    .map(|z| {
                        (
                            z.room.clone().unwrap_or_else(|| z.name.to_lowercase()),
                            z.temperature,
                        )
                    })
                    .collect::<Vec<_>>(),
            ),
        },
        Intent::HeatingSet { tenths, .. } => {
            // tado's own range, refused here so the answer is a sentence
            // rather than tado's 422.
            if !(50..=250).contains(tenths) {
                return response::heating_out_of_range();
            }
            for z in &zones {
                if let Err(e) = tado.set_temperature(z.id, *tenths as f32 / 10.0).await {
                    tracing::warn!("[{peer}] tado refused {}: {e}", z.name);
                    return response::heating_failed();
                }
            }
            response::heating_set(place.heated(), *tenths)
        }
        Intent::HeatingStep { up, .. } => {
            let mut last = 0;
            for z in &zones {
                last = stepped(z, *up);
                if let Err(e) = tado.set_temperature(z.id, last as f32 / 10.0).await {
                    tracing::warn!("[{peer}] tado refused {}: {e}", z.name);
                    return response::heating_failed();
                }
            }
            match &place {
                Place::Room(_) => response::heating_set(place.heated(), last),
                Place::Everywhere => response::heating_stepped_everywhere(*up),
            }
        }
        Intent::HeatingOff { .. } => {
            for z in &zones {
                if let Err(e) = tado.turn_off(z.id).await {
                    tracing::warn!("[{peer}] tado refused {}: {e}", z.name);
                    return response::heating_failed();
                }
            }
            response::heating_off(place.heated())
        }
        Intent::HeatingResume { .. } => {
            for z in &zones {
                if let Err(e) = tado.resume_schedule(z.id).await {
                    tracing::warn!("[{peer}] tado refused {}: {e}", z.name);
                    return response::heating_failed();
                }
            }
            response::heating_resumed(place.heated())
        }
        _ => response::heating_failed(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use niles_presence::Placed;

    fn zone(
        id: u64,
        name: &str,
        room: Option<&str>,
        target: Option<f32>,
        temperature: Option<f32>,
    ) -> Zone {
        Zone {
            id,
            name: name.into(),
            room: room.map(str::to_string),
            placed_by: if room.is_some() {
                Placed::Paired
            } else {
                Placed::Nowhere
            },
            temperature,
            humidity: None,
            target,
            on: target.is_some(),
            overridden: false,
            until: None,
            reachable: true,
        }
    }

    fn kept(place: &Place) -> Vec<u64> {
        [(1, "Stue", Some("living_room")), (2, "Bedroom", None)]
            .into_iter()
            .filter(|(_, name, room)| is_in(place, name, *room))
            .map(|(id, _, _)| id)
            .collect()
    }

    #[test]
    fn a_named_room_finds_its_zone() {
        let place = wanted(Some("living room"), None).unwrap();
        assert_eq!(place, Place::Room("living_room".into()));
        assert_eq!(kept(&place), vec![1]);
    }

    #[test]
    fn an_unpaired_zone_is_found_by_its_own_name() {
        assert_eq!(kept(&wanted(Some("bedroom"), None).unwrap()), vec![2]);
    }

    #[test]
    fn unnamed_is_the_room_you_are_in() {
        let here = RoomName::parse("living_room").unwrap();
        assert_eq!(kept(&wanted(None, Some(&here)).unwrap()), vec![1]);
    }

    #[test]
    fn a_satellite_with_no_room_has_to_ask() {
        assert_eq!(wanted(None, None).unwrap_err(), Miss::WhichRoom);
    }

    #[test]
    fn a_room_without_heating_keeps_nothing() {
        assert!(kept(&wanted(Some("office"), None).unwrap()).is_empty());
    }

    #[test]
    fn everywhere_is_every_zone() {
        let place = wanted(Some("everywhere"), None).unwrap();
        assert_eq!(place, Place::Everywhere);
        assert_eq!(kept(&place), vec![1, 2]);
    }

    #[test]
    fn up_is_a_degree_above_the_target() {
        assert_eq!(
            stepped(&zone(1, "Stue", None, Some(21.0), Some(19.0)), true),
            220
        );
        assert_eq!(
            stepped(&zone(1, "Stue", None, Some(21.0), Some(19.0)), false),
            200
        );
    }

    #[test]
    fn a_zone_that_is_off_steps_from_the_room() {
        assert_eq!(stepped(&zone(1, "Stue", None, None, Some(17.8)), true), 190);
    }

    #[test]
    fn steps_stay_inside_what_tado_takes() {
        assert_eq!(stepped(&zone(1, "Stue", None, Some(25.0), None), true), 250);
        assert_eq!(stepped(&zone(1, "Stue", None, Some(5.0), None), false), 50);
    }
}
