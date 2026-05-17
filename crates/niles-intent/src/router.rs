//! The Tier 0 router itself.

use crate::intent::Intent;
use regex::Regex;
use std::sync::OnceLock;
use std::time::Duration;

/// Tier 0 intent router. Cheap to construct; regexes are compiled
/// lazily on first use and reused across all subsequent `parse` calls.
pub struct IntentRouter {
    _private: (),
}

impl IntentRouter {
    pub fn new() -> Self {
        Self { _private: () }
    }

    /// Try every Tier 0 pattern against the normalized transcript.
    /// Returns `Some(Intent)` on the first match, `None` if nothing
    /// applies (caller escalates to Tier 1).
    pub fn parse(&self, transcript: &str) -> Option<Intent> {
        let t = normalize(transcript);

        if let Some(intent) = match_light(&t) {
            return Some(intent);
        }
        if let Some(intent) = match_timer(&t) {
            return Some(intent);
        }
        if let Some(intent) = match_stop_cancel(&t) {
            return Some(intent);
        }
        None
    }
}

impl Default for IntentRouter {
    fn default() -> Self {
        Self::new()
    }
}

/// Lowercase, trim, collapse internal whitespace, strip trailing punctuation.
fn normalize(s: &str) -> String {
    let lower = s.trim().to_lowercase();
    let no_trailing_punct = lower.trim_end_matches(|c: char| !c.is_alphanumeric());
    // Collapse internal whitespace runs to single spaces.
    no_trailing_punct
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

// ---- Light on/off ----------------------------------------------------------

fn light_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // Two phrasings:
        //   "turn (on|off) [the] <room> light[s]"
        //   "<room> light[s] (on|off)"
        Regex::new(
            r"(?x)
              ^
              (?:
                turn\s+(?P<state1>on|off)\s+(?:the\s+)?(?P<room1>.+?)\s+lights?
              |
                (?P<room2>.+?)\s+lights?\s+(?P<state2>on|off)
              )
              $",
        )
        .expect("light regex compiles")
    })
}

fn match_light(t: &str) -> Option<Intent> {
    let caps = light_regex().captures(t)?;
    let (state, room) = if let Some(s) = caps.name("state1") {
        (s.as_str(), caps.name("room1")?.as_str())
    } else {
        (caps.name("state2")?.as_str(), caps.name("room2")?.as_str())
    };
    Some(Intent::LightSet {
        room: room.to_string(),
        on: state == "on",
    })
}

// ---- Timer -----------------------------------------------------------------

fn timer_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // Two phrasings:
        //   "(set a )?timer for <n> <unit>[s] (called <name>)?"
        //   "<n> <unit>[s] timer (called <name>)?"
        // Units: seconds / minutes / hours (+ common short forms).
        Regex::new(
            r"(?x)
              ^
              (?:
                (?:set\s+a\s+)?timer\s+for\s+(?P<n1>\d+)\s+(?P<unit1>seconds?|secs?|minutes?|mins?|hours?|hrs?)
                (?:\s+called\s+(?P<name1>.+))?
              |
                (?P<n2>\d+)\s+(?P<unit2>second|sec|minute|min|hour|hr)\s+timer
                (?:\s+called\s+(?P<name2>.+))?
              )
              $",
        )
        .expect("timer regex compiles")
    })
}

fn match_timer(t: &str) -> Option<Intent> {
    let caps = timer_regex().captures(t)?;
    let (n_str, unit_str, name) = if let Some(n) = caps.name("n1") {
        (
            n.as_str(),
            caps.name("unit1")?.as_str(),
            caps.name("name1").map(|m| m.as_str().to_string()),
        )
    } else {
        (
            caps.name("n2")?.as_str(),
            caps.name("unit2")?.as_str(),
            caps.name("name2").map(|m| m.as_str().to_string()),
        )
    };
    let n: u64 = n_str.parse().ok()?;
    let seconds = match unit_str {
        u if u.starts_with("sec") => n,
        u if u.starts_with("min") => n * 60,
        u if u.starts_with("hr") || u.starts_with("hour") => n * 3600,
        _ => return None,
    };
    Some(Intent::TimerSet {
        duration: Duration::from_secs(seconds),
        name,
    })
}

// ---- Stop / cancel ---------------------------------------------------------

fn match_stop_cancel(t: &str) -> Option<Intent> {
    match t {
        "stop" => Some(Intent::Stop),
        "cancel" => Some(Intent::Cancel),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Option<Intent> {
        IntentRouter::new().parse(s)
    }

    // ---- Lights ----

    #[test]
    fn turn_off_kitchen_light() {
        assert_eq!(
            parse("turn off the kitchen light"),
            Some(Intent::LightSet {
                room: "kitchen".into(),
                on: false
            })
        );
    }

    #[test]
    fn turn_on_living_room_lights_multiword() {
        assert_eq!(
            parse("turn on the living room lights"),
            Some(Intent::LightSet {
                room: "living room".into(),
                on: true
            })
        );
    }

    #[test]
    fn alternate_phrasing() {
        assert_eq!(
            parse("kitchen lights off"),
            Some(Intent::LightSet {
                room: "kitchen".into(),
                on: false
            })
        );
    }

    #[test]
    fn normalizes_case_and_trailing_punctuation() {
        assert_eq!(
            parse("Turn off the kitchen light."),
            Some(Intent::LightSet {
                room: "kitchen".into(),
                on: false
            })
        );
    }

    // ---- Timers ----

    #[test]
    fn timer_minutes_canonical() {
        assert_eq!(
            parse("set a timer for 5 minutes"),
            Some(Intent::TimerSet {
                duration: Duration::from_secs(300),
                name: None
            })
        );
    }

    #[test]
    fn timer_short_form() {
        assert_eq!(
            parse("8 minute timer"),
            Some(Intent::TimerSet {
                duration: Duration::from_secs(8 * 60),
                name: None
            })
        );
    }

    #[test]
    fn timer_hours() {
        assert_eq!(
            parse("timer for 2 hours"),
            Some(Intent::TimerSet {
                duration: Duration::from_secs(2 * 3600),
                name: None
            })
        );
    }

    #[test]
    fn timer_seconds() {
        assert_eq!(
            parse("set a timer for 30 seconds"),
            Some(Intent::TimerSet {
                duration: Duration::from_secs(30),
                name: None
            })
        );
    }

    #[test]
    fn timer_with_name() {
        assert_eq!(
            parse("set a timer for 10 minutes called pasta"),
            Some(Intent::TimerSet {
                duration: Duration::from_secs(600),
                name: Some("pasta".into())
            })
        );
    }

    // ---- Stop / cancel ----

    #[test]
    fn stop() {
        assert_eq!(parse("stop"), Some(Intent::Stop));
        assert_eq!(parse("Stop!"), Some(Intent::Stop));
    }

    #[test]
    fn cancel() {
        assert_eq!(parse("cancel"), Some(Intent::Cancel));
    }

    // ---- Misses ----

    #[test]
    fn unmatched_returns_none() {
        assert_eq!(parse("what's the weather like today"), None);
        assert_eq!(parse(""), None);
        assert_eq!(parse("turn off"), None); // missing room
        assert_eq!(parse("timer"), None); // missing duration
    }

    #[test]
    fn normalize_collapses_internal_whitespace() {
        assert_eq!(
            parse("turn   off   the   kitchen   light"),
            Some(Intent::LightSet {
                room: "kitchen".into(),
                on: false
            })
        );
    }
}
