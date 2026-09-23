//! Weather and heating, the two things a house is asked about most
//! after its lights.
//!
//! Weather was the most common question on any smart speaker and went
//! to the LLM every time, which spent a round trip to read out a
//! forecast Niles could fetch itself. Heating had no voice at all.
//!
//! Anchored at both ends like every other Tier 0 pattern: a sentence
//! these do not recognise goes to the LLM, which is slower but never
//! wrong in the way a loose regex is.

use crate::intent::{ForecastDay, Intent};
use regex::{Captures, Regex};
use std::sync::OnceLock;

/// Any of the ways Whisper writes a possessive or a contraction.
const IS: &str = r"(?:['’]s|\s+is)";

/// What a room can be besides a room: the whole house.
const EVERYWHERE: [&str; 7] = [
    "everywhere",
    "house",
    "whole house",
    "home",
    "all rooms",
    "all the rooms",
    "every room",
];

pub(crate) fn match_weather(t: &str) -> Option<Intent> {
    match_forecast(t)
        .or_else(|| match_rain(t))
        .or_else(|| match_outside(t))
}

pub(crate) fn match_heating(t: &str) -> Option<Intent> {
    match_heating_set(t)
        .or_else(|| match_heating_step(t))
        .or_else(|| match_heating_off(t))
        .or_else(|| match_heating_resume(t))
        .or_else(|| match_heating_query(t))
}

fn day_of(caps: &Captures, names: &[&str]) -> ForecastDay {
    let said = names.iter().find_map(|n| caps.name(n)).map(|m| m.as_str());
    match said {
        Some(d) if d.starts_with("tomorrow") => ForecastDay::Tomorrow,
        _ => ForecastDay::Today,
    }
}

/// "what's the weather" / "how's the weather tomorrow" / "tomorrow's
/// forecast".
fn match_forecast(t: &str) -> Option<Intent> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(&format!(
            r"(?x)^
              (?:
                (?:what{IS}|how{IS}|what\s+will)\s+the\s+(?:weather|forecast)
                  (?:\s+be)?(?:\s+looking)?(?:\s+like)?(?:\s+going\s+to\s+be(?:\s+like)?)?(?:\s+outside)?
                  (?:\s+(?:for\s+)?(?P<day>today|tomorrow))?
              |
                (?:the\s+)?(?:weather|forecast)(?:\s+(?:for\s+)?(?P<day2>today|tomorrow))?
              |
                (?:what{IS}\s+)?(?P<day3>today|tomorrow)['’]s\s+(?:weather|forecast)
              )$"
        ))
        .expect("forecast regex compiles")
    });
    let caps = re.captures(t)?;
    Some(Intent::WeatherQuery {
        day: day_of(&caps, &["day", "day2", "day3"]),
        rain: false,
    })
}

/// "will it rain tomorrow" / "is it raining" / "do I need an umbrella".
fn match_rain(t: &str) -> Option<Intent> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(&format!(
            r"(?x)^
              (?:
                (?:will\s+it|is\s+it\s+(?:going\s+to|gonna|supposed\s+to)|does\s+it\s+look\s+like\s+it{IS}\s+going\s+to)
                  \s+rain(?:\s+(?P<day>today|tomorrow))?
              |
                is\s+it\s+raining(?:\s+outside)?
              |
                (?:do\s+i\s+need|should\s+i\s+(?:bring|take))\s+an\s+umbrella(?:\s+(?P<day2>today|tomorrow))?
              )$"
        ))
        .expect("rain regex compiles")
    });
    let caps = re.captures(t)?;
    Some(Intent::WeatherQuery {
        day: day_of(&caps, &["day", "day2"]),
        rain: true,
    })
}

/// "what's the temperature" / "how cold is it outside".
///
/// A bare "what's the temperature" is the weather, as it is on every
/// other speaker. "In here" and a room name are the heating's.
fn match_outside(t: &str) -> Option<Intent> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(&format!(
            r"(?x)^
              (?:
                what{IS}\s+the\s+temperature(?:\s+outside)?
              | how\s+(?:cold|warm|hot)\s+is\s+it\s+outside
              | what{IS}\s+it\s+like\s+outside
              )$"
        ))
        .expect("outside regex compiles")
    });
    re.is_match(t).then_some(Intent::WeatherQuery {
        day: ForecastDay::Today,
        rain: false,
    })
}

/// The room as said, or `None` for the room the satellite is in.
///
/// "Everywhere" and its synonyms come back as `"everywhere"`, which
/// dispatch reads as every zone.
fn room_of(caps: &Captures, names: &[&str]) -> Option<String> {
    let said = names.iter().find_map(|n| caps.name(n))?.as_str().trim();
    let said = said.strip_prefix("the ").unwrap_or(said);
    match said {
        "" | "here" | "in here" | "this room" | "the" | "it" => None,
        s if EVERYWHERE.contains(&s) => Some("everywhere".into()),
        s => Some(s.to_string()),
    }
}

/// What the heating is called when it is spoken to.
const HEATING: &str = r"(?:heating|heat|thermostat|temperature)";

/// "in the bedroom" / "everywhere", after the thing being changed.
///
/// Needs the "in": a bare word after "heat" is more often the rest of a
/// device's name ("the heat lamp") than a room.
fn where_after(name: &str) -> String {
    format!(r"(?:\s+in\s+(?:the\s+)?(?P<{name}>[a-z][a-z\x20]*?)|\s+(?P<{name}_all>everywhere))?")
}

/// "21", "21.5", "21,5", "21 and a half", with or without degrees.
const DEGREES: &str = r"(?P<n>\d{1,2}(?:[.,]\d)?)(?P<half>\s+and\s+a\s+half)?(?:\s*°\s*c?|\s*degrees?(?:\s+(?:c|celsius))?)?";

fn tenths(caps: &Captures) -> Option<u16> {
    let n: f32 = caps.name("n")?.as_str().replace(',', ".").parse().ok()?;
    let half = if caps.name("half").is_some() {
        0.5
    } else {
        0.0
    };
    Some(((n + half) * 10.0).round() as u16)
}

/// "set the heating to 21" / "set the living room to 21 degrees" /
/// "turn the bedroom thermostat up to 20".
fn match_heating_set(t: &str) -> Option<Intent> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        let after = where_after("after");
        Regex::new(&format!(
            r"(?x)^
              (?:
                (?:set|put|turn)\s+(?:the\s+)?(?:(?P<before>[a-z][a-z\x20]*?)\s+)?{HEATING}
                  (?:\s+in\s+(?:the\s+)?(?P<within>[a-z][a-z\x20]*?))?
                  (?:\s+(?:up|down))?\s+(?:to|at)\s+{DEGREES}{after}
              |
                (?:set|heat|warm)\s+(?:up\s+)?(?:the\s+)?(?P<room>[a-z][a-z\x20]*?)\s+to\s+
                  (?P<n2>\d{{1,2}}(?:[.,]\d)?)(?P<half2>\s+and\s+a\s+half)?(?:\s*°\s*c?|\s*degrees?(?:\s+(?:c|celsius))?)
              )$"
        ))
        .expect("heating set regex compiles")
    });
    let caps = re.captures(t)?;
    let tenths = if caps.name("n2").is_some() {
        let n: f32 = caps.name("n2")?.as_str().replace(',', ".").parse().ok()?;
        let half = if caps.name("half2").is_some() {
            0.5
        } else {
            0.0
        };
        ((n + half) * 10.0).round() as u16
    } else {
        tenths(&caps)?
    };
    Some(Intent::HeatingSet {
        room: room_of(&caps, &["before", "within", "after", "after_all", "room"]),
        tenths,
    })
}

/// "turn the heating up" / "turn down the bedroom heating" — a degree.
fn match_heating_step(t: &str) -> Option<Intent> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        let after = where_after("after");
        let after2 = where_after("after2");
        Regex::new(&format!(
            r"(?x)^
              (?:
                turn\s+(?P<dir>up|down)\s+(?:the\s+)?(?:(?P<before>[a-z][a-z\x20]*?)\s+)?{HEATING}{after}
              |
                turn\s+(?:the\s+)?(?:(?P<before2>[a-z][a-z\x20]*?)\s+)?{HEATING}
                  (?:\s+in\s+(?:the\s+)?(?P<within>[a-z][a-z\x20]*?))?\s+(?P<dir2>up|down){after2}
              )$"
        ))
        .expect("heating step regex compiles")
    });
    let caps = re.captures(t)?;
    let dir = caps.name("dir").or_else(|| caps.name("dir2"))?.as_str();
    Some(Intent::HeatingStep {
        room: room_of(
            &caps,
            &[
                "before",
                "before2",
                "within",
                "after",
                "after_all",
                "after2",
                "after2_all",
            ],
        ),
        up: dir == "up",
    })
}

/// "turn off the heating" / "turn the bedroom heating off".
fn match_heating_off(t: &str) -> Option<Intent> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        let after = where_after("after");
        let after2 = where_after("after2");
        Regex::new(&format!(
            r"(?x)^
              (?:
                (?:turn|switch)\s+off\s+(?:the\s+)?(?:(?P<before>[a-z][a-z\x20]*?)\s+)?(?:heating|heat|thermostat){after}
              |
                (?:turn|switch)\s+(?:the\s+)?(?:(?P<before2>[a-z][a-z\x20]*?)\s+)?(?:heating|heat|thermostat)
                  (?:\s+in\s+(?:the\s+)?(?P<within>[a-z][a-z\x20]*?))?\s+off{after2}
              )$"
        ))
        .expect("heating off regex compiles")
    });
    let caps = re.captures(t)?;
    Some(Intent::HeatingOff {
        room: room_of(
            &caps,
            &[
                "before",
                "before2",
                "within",
                "after",
                "after_all",
                "after2",
                "after2_all",
            ],
        ),
    })
}

/// "put the heating back on the schedule" / "resume the heating" /
/// "turn the heating on".
///
/// Turning the heating on means handing it back to its schedule: that
/// is what "on" is in a house with one, rather than a temperature
/// somebody would have to pick.
fn match_heating_resume(t: &str) -> Option<Intent> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(
            r"(?x)^
              (?:
                (?:put|set|turn)\s+(?:the\s+)?(?:(?P<before>[a-z][a-z\x20]*?)\s+)?(?:heating|heat|thermostat)
                  (?:\s+in\s+(?:the\s+)?(?P<within>[a-z][a-z\x20]*?))?\s+back\s+(?:on|to)\s+(?:the\s+|its\s+)?schedule
              |
                resume\s+(?:the\s+)?(?:(?P<before2>[a-z][a-z\x20]*?)\s+)?(?:heating(?:\s+schedule)?|heating|schedule)
                  (?:\s+in\s+(?:the\s+)?(?P<within2>[a-z][a-z\x20]*?))?
              |
                (?:turn|switch)\s+on\s+(?:the\s+)?(?:(?P<before3>[a-z][a-z\x20]*?)\s+)?(?:heating|heat)
                  (?:\s+in\s+(?:the\s+)?(?P<after>[a-z][a-z\x20]*?)|\s+(?P<after_all>everywhere))?
              |
                (?:turn|switch)\s+(?:the\s+)?(?:(?P<before4>[a-z][a-z\x20]*?)\s+)?(?:heating|heat)
                  (?:\s+in\s+(?:the\s+)?(?P<within4>[a-z][a-z\x20]*?))?\s+(?:back\s+)?on
              )$",
        )
        .expect("heating resume regex compiles")
    });
    let caps = re.captures(t)?;
    Some(Intent::HeatingResume {
        room: room_of(
            &caps,
            &[
                "before",
                "within",
                "before2",
                "within2",
                "before3",
                "after",
                "after_all",
                "before4",
                "within4",
            ],
        ),
    })
}

/// "what's the temperature in the bedroom" / "how warm is it in here" /
/// "what's the living room temperature".
fn match_heating_query(t: &str) -> Option<Intent> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(&format!(
            r"(?x)^
              (?:
                (?:what{IS}\s+the\s+temperature|what\s+temperature\s+is\s+it|how\s+(?:warm|cold|hot)\s+is\s+it)
                  \s+in\s+(?P<within>[a-z][a-z\x20]*?)
              |
                what{IS}\s+the\s+(?P<before>[a-z][a-z\x20]*?)\s+temperature
              )$"
        ))
        .expect("heating query regex compiles")
    });
    let caps = re.captures(t)?;
    Some(Intent::HeatingQuery {
        room: room_of(&caps, &["within", "before"]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::IntentRouter;

    fn parse(s: &str) -> Option<Intent> {
        IntentRouter::new().parse(s)
    }

    fn weather(day: ForecastDay, rain: bool) -> Option<Intent> {
        Some(Intent::WeatherQuery { day, rain })
    }

    #[test]
    fn the_everyday_weather_questions() {
        use ForecastDay::*;
        assert_eq!(parse("What's the weather?"), weather(Today, false));
        assert_eq!(
            parse("what is the weather like today"),
            weather(Today, false)
        );
        assert_eq!(
            parse("How's the weather tomorrow?"),
            weather(Tomorrow, false)
        );
        assert_eq!(
            parse("what's the weather going to be like tomorrow"),
            weather(Tomorrow, false)
        );
        assert_eq!(
            parse("What’s the forecast for tomorrow?"),
            weather(Tomorrow, false)
        );
        assert_eq!(parse("tomorrow's weather"), weather(Tomorrow, false));
        assert_eq!(parse("weather"), weather(Today, false));
        assert_eq!(
            parse("what's the weather like outside"),
            weather(Today, false)
        );
    }

    #[test]
    fn rain_is_its_own_question() {
        use ForecastDay::*;
        assert_eq!(parse("Will it rain tomorrow?"), weather(Tomorrow, true));
        assert_eq!(parse("is it going to rain"), weather(Today, true));
        assert_eq!(parse("Is it raining?"), weather(Today, true));
        assert_eq!(parse("do I need an umbrella today"), weather(Today, true));
    }

    #[test]
    fn a_bare_temperature_is_the_weather() {
        assert_eq!(
            parse("What's the temperature?"),
            weather(ForecastDay::Today, false)
        );
        assert_eq!(
            parse("how cold is it outside"),
            weather(ForecastDay::Today, false)
        );
    }

    #[test]
    fn weather_somewhere_else_is_for_the_llm() {
        // Geocoding a place name is the LLM's tool; Tier 0 answers home.
        assert_eq!(parse("what's the weather in paris"), None);
        assert_eq!(parse("what was the weather like yesterday"), None);
    }

    fn set(room: Option<&str>, tenths: u16) -> Option<Intent> {
        Some(Intent::HeatingSet {
            room: room.map(str::to_string),
            tenths,
        })
    }

    #[test]
    fn setting_the_heating() {
        assert_eq!(parse("Set the heating to 21."), set(None, 210));
        assert_eq!(
            parse("set the living room to 21 degrees"),
            set(Some("living room"), 210)
        );
        assert_eq!(
            parse("set the heating in the bedroom to 19"),
            set(Some("bedroom"), 190)
        );
        assert_eq!(
            parse("set the bedroom heating to 19.5"),
            set(Some("bedroom"), 195)
        );
        assert_eq!(parse("set the thermostat to 20 and a half"), set(None, 205));
        assert_eq!(parse("turn the heating up to 22 degrees"), set(None, 220));
        assert_eq!(parse("set the temperature to 21°"), set(None, 210));
        assert_eq!(
            parse("set the heating to 18 everywhere"),
            set(Some("everywhere"), 180)
        );
        assert_eq!(
            parse("heat the office to 20 degrees"),
            set(Some("office"), 200)
        );
    }

    #[test]
    fn a_room_set_to_a_bare_number_is_not_heating() {
        // Without "degrees" or the heating named, "set the kitchen to 30"
        // is as likely a light or a speaker as a radiator.
        assert_eq!(parse("set the kitchen to 30"), None);
    }

    #[test]
    fn stepping_the_heating() {
        let up = |room: Option<&str>| {
            Some(Intent::HeatingStep {
                room: room.map(str::to_string),
                up: true,
            })
        };
        assert_eq!(parse("turn the heating up"), up(None));
        assert_eq!(
            parse("turn up the heating in the bedroom"),
            up(Some("bedroom"))
        );
        assert_eq!(parse("turn the bedroom heating up"), up(Some("bedroom")));
        assert_eq!(
            parse("turn down the heating"),
            Some(Intent::HeatingStep {
                room: None,
                up: false
            })
        );
    }

    #[test]
    fn warmer_is_still_the_lights() {
        // Colour temperature got there first, and "make the bedroom
        // warmer" has meant the bulbs since.
        assert!(matches!(
            parse("make the bedroom warmer"),
            Some(Intent::LightKelvinStep { .. })
        ));
    }

    #[test]
    fn heating_off_and_back_on() {
        let off = |room: Option<&str>| {
            Some(Intent::HeatingOff {
                room: room.map(str::to_string),
            })
        };
        let resume = |room: Option<&str>| {
            Some(Intent::HeatingResume {
                room: room.map(str::to_string),
            })
        };
        assert_eq!(parse("turn off the heating"), off(None));
        assert_eq!(
            parse("turn the heating off in the bedroom"),
            off(Some("bedroom"))
        );
        assert_eq!(parse("turn off the bedroom heating"), off(Some("bedroom")));
        assert_eq!(
            parse("turn off the heating everywhere"),
            off(Some("everywhere"))
        );
        assert_eq!(parse("turn the heating off"), off(None));
        assert_eq!(parse("put the heating back on the schedule"), resume(None));
        assert_eq!(parse("resume the heating"), resume(None));
        assert_eq!(
            parse("turn on the heating in the office"),
            resume(Some("office"))
        );
        assert_eq!(parse("turn the heating back on"), resume(None));
    }

    #[test]
    fn a_device_named_heat_something_is_not_the_heating() {
        assert_eq!(parse("turn on the heat lamp"), None);
    }

    #[test]
    fn asking_what_a_room_is_at() {
        let q = |room: Option<&str>| {
            Some(Intent::HeatingQuery {
                room: room.map(str::to_string),
            })
        };
        assert_eq!(
            parse("What's the temperature in the bedroom?"),
            q(Some("bedroom"))
        );
        assert_eq!(parse("how warm is it in here"), q(None));
        assert_eq!(
            parse("what's the living room temperature"),
            q(Some("living room"))
        );
        assert_eq!(parse("how cold is it in the office"), q(Some("office")));
    }

    #[test]
    fn heating_does_not_take_the_lights() {
        assert!(matches!(
            parse("turn off the kitchen lights"),
            Some(Intent::LightSet { .. })
        ));
        assert!(matches!(
            parse("set the kitchen lights to 30%"),
            Some(Intent::LightDim { .. })
        ));
    }
}
