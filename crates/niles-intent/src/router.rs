//! The Tier 0 router itself.

use crate::intent::Intent;
use regex::Regex;
use std::sync::OnceLock;
use std::time::Duration;

/// Tier 0 intent router. Cheap to construct; regexes are compiled
/// lazily on first use and reused across all subsequent `parse` calls.
#[derive(Debug, Default, Clone, Copy)]
pub struct IntentRouter;

impl IntentRouter {
    pub fn new() -> Self {
        Self
    }

    /// Try every Tier 0 pattern against the normalized transcript.
    /// Returns `Some(Intent)` on the first match, `None` if nothing
    /// applies (caller escalates to Tier 1).
    pub fn parse(&self, transcript: &str) -> Option<Intent> {
        let t = normalize(transcript);

        // Order matters when patterns could conflict. `light_dim`
        // requires the trailing "... to N%" so it can't be confused
        // with `light` (which requires a trailing `on`/`off`), but
        // we still match the more specific pattern first as a habit.
        match_light_dim(&t)
            .or_else(|| match_light(&t))
            .or_else(|| match_back_to_normal(&t))
            .or_else(|| match_scene_save(&t))
            .or_else(|| match_scene_apply(&t))
            .or_else(|| match_timer(&t))
            .or_else(|| match_stop_cancel(&t))
    }
}

/// Lowercase, trim, collapse internal whitespace, strip trailing
/// sentence punctuation (`.`, `!`, `?`, `,`, `;`, `:`).
///
/// Note: we explicitly enumerate the trailing chars to strip rather
/// than "anything non-alphanumeric" — otherwise `%` gets eaten and
/// `dim the kitchen lights to 30%` becomes `... to 30`, which the
/// `light_dim` regex can't anchor on.
fn normalize(s: &str) -> String {
    s.trim()
        .to_lowercase()
        .trim_end_matches(['.', '!', '?', ',', ';', ':'])
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
    // "turn on the lights" would otherwise capture room="the" because the
    // optional `the` group can decline to match. Reject so the caller can
    // escalate to Tier 1 instead of producing a bogus room.
    if room == "the" {
        return None;
    }
    Some(Intent::LightSet {
        room: room.to_string(),
        on: state == "on",
    })
}

// ---- Light dim (brightness percent) ----------------------------------------

fn light_dim_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // "dim [the] <room> light[s] to N%"
        // "set [the] <room> light[s] to N percent"
        //
        // `set ... on/off` won't match here because the suffix
        // requires `to N (%|percent)`; the regular light regex
        // handles the on/off case.
        Regex::new(
            r"(?x)
              ^
              (?:dim|set)\s+(?:the\s+)?(?P<room>.+?)\s+lights?\s+to\s+
              (?P<n>\d{1,3})
              \s*(?:%|percent)
              $",
        )
        .expect("light_dim regex compiles")
    })
}

fn match_light_dim(t: &str) -> Option<Intent> {
    let caps = light_dim_regex().captures(t)?;
    let room = caps.name("room")?.as_str();
    // Same trap as the on/off pattern: the optional `the` group can
    // decline to match, leaving "the" as the captured room. Reject
    // so we escalate to Tier 1 instead of producing a bogus room.
    if room == "the" {
        return None;
    }
    let n: u8 = caps.name("n")?.as_str().parse().ok()?;
    if n > 100 {
        return None;
    }
    Some(Intent::LightDim {
        room: room.to_string(),
        percent: n,
    })
}

// ---- Scene save ------------------------------------------------------------

fn scene_save_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // Three phrasings in priority order:
        //   "save this as <name>"                         -> room: None
        //   "save (the )?<room> as <name>"                -> room: Some(...)
        //   "save <name>"                                 -> room: None (last resort)
        //
        // The optional `scene\s+` slot before `as` lets users phrase
        // naturally: "save this scene as cozy" and "save the kitchen
        // scene as cozy" route to the right (name, room) pair instead
        // of leaving `scene` glued onto whatever precedes it.
        Regex::new(
            r"(?x)
              ^
              (?:
                save\s+this\s+(?:scene\s+)?as\s+(?P<name1>.+)
              |
                save\s+(?:the\s+)?(?P<room>.+?)\s+(?:scene\s+)?as\s+(?P<name2>.+)
              |
                save\s+(?P<name3>.+)
              )
              $",
        )
        .expect("scene_save regex compiles")
    })
}

fn match_scene_save(t: &str) -> Option<Intent> {
    let caps = scene_save_regex().captures(t)?;
    if let Some(name) = caps.name("name1") {
        let name = name.as_str().trim();
        if name.is_empty() || name == "the" || name == "lights" {
            return None;
        }
        return Some(Intent::SceneSave {
            name: name.to_string(),
            room: None,
        });
    }
    if let Some(name) = caps.name("name2") {
        let name = name.as_str().trim();
        let room = caps.name("room")?.as_str();
        if name.is_empty() || name == "the" || name == "lights" {
            return None;
        }
        if room == "the" || room == "lights" {
            return None;
        }
        return Some(Intent::SceneSave {
            name: name.to_string(),
            room: Some(room.to_string()),
        });
    }
    let name = caps.name("name3")?.as_str().trim();
    if name.is_empty() || name == "the" || name == "lights" {
        return None;
    }
    Some(Intent::SceneSave {
        name: name.to_string(),
        room: None,
    })
}

// ---- Scene apply -----------------------------------------------------------

fn scene_apply_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // Three phrasings:
        //   "apply <name>"
        //   "<name> scene"
        //   "scene <name>"
        Regex::new(
            r"(?x)
              ^
              (?:
                apply\s+(?P<name1>.+)
              |
                (?P<name2>.+)\s+scene
              |
                scene\s+(?P<name3>.+)
              )
              $",
        )
        .expect("scene_apply regex compiles")
    })
}

fn match_scene_apply(t: &str) -> Option<Intent> {
    let caps = scene_apply_regex().captures(t)?;
    let name = caps
        .name("name1")
        .or_else(|| caps.name("name2"))
        .or_else(|| caps.name("name3"))?
        .as_str()
        .trim();
    if name.is_empty() || name == "the" || name == "lights" {
        return None;
    }
    Some(Intent::SceneApply {
        name: name.to_string(),
    })
}

// ---- Back to normal (clear manual mode) ------------------------------------

fn back_to_normal_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // Four phrasings (anchored — no false-match on "back to the normal way"):
        //   "back to normal"                              -> room: None
        //   "normal lights"                               -> room: None
        //   "back to normal in [the] <room>"              -> room: Some(...)
        //   "<room> back to normal"                       -> room: Some(...)
        Regex::new(
            r"(?x)
              ^
              (?:
                back\s+to\s+normal\s+in\s+(?:the\s+)?(?P<room1>.+?)
              |
                (?P<room2>.+?)\s+back\s+to\s+normal
              |
                back\s+to\s+normal
              |
                normal\s+lights
              )
              $",
        )
        .expect("back_to_normal regex compiles")
    })
}

fn match_back_to_normal(t: &str) -> Option<Intent> {
    let caps = back_to_normal_regex().captures(t)?;
    let room = caps
        .name("room1")
        .or_else(|| caps.name("room2"))
        .map(|m| m.as_str());
    // Reject bogus rooms: "the" (when the optional group declines to
    // match) and "lights" (a fixture type, not a room). Escalate to
    // Tier 1 rather than produce a nonsensical room name.
    if room == Some("the") || room == Some("lights") {
        return None;
    }
    Some(Intent::ClearManualMode {
        room: room.map(|s| s.to_string()),
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
                (?P<n2>\d+)\s+(?P<unit2>seconds?|secs?|minutes?|mins?|hours?|hrs?)\s+timer
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
        u if u.starts_with("sec") => Some(n),
        u if u.starts_with("min") => n.checked_mul(60),
        u if u.starts_with("hr") || u.starts_with("hour") => n.checked_mul(3600),
        _ => None,
    }?;
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

    // ---- Light dim (brightness percent) ----

    #[test]
    fn dim_kitchen_lights_to_30_percent_symbol() {
        assert_eq!(
            parse("dim the kitchen lights to 30%"),
            Some(Intent::LightDim {
                room: "kitchen".into(),
                percent: 30
            })
        );
    }

    #[test]
    fn dim_kitchen_lights_to_30_percent_word() {
        assert_eq!(
            parse("dim the kitchen lights to 30 percent"),
            Some(Intent::LightDim {
                room: "kitchen".into(),
                percent: 30
            })
        );
    }

    #[test]
    fn set_living_room_light_to_50_percent_multiword_room() {
        assert_eq!(
            parse("set the living room light to 50%"),
            Some(Intent::LightDim {
                room: "living room".into(),
                percent: 50
            })
        );
    }

    #[test]
    fn dim_works_without_definite_article() {
        assert_eq!(
            parse("dim kitchen lights to 30%"),
            Some(Intent::LightDim {
                room: "kitchen".into(),
                percent: 30
            })
        );
    }

    #[test]
    fn dim_boundary_values_accepted() {
        assert_eq!(
            parse("set the kitchen light to 0%"),
            Some(Intent::LightDim {
                room: "kitchen".into(),
                percent: 0
            })
        );
        assert_eq!(
            parse("set the kitchen light to 100%"),
            Some(Intent::LightDim {
                room: "kitchen".into(),
                percent: 100
            })
        );
    }

    #[test]
    fn dim_rejects_over_100() {
        // "150%" is regex-shaped but out of range — fall through to Tier 1.
        assert_eq!(parse("set the kitchen light to 150%"), None);
        assert_eq!(parse("dim the kitchen light to 200 percent"), None);
    }

    #[test]
    fn dim_normalizes_case_and_trailing_punctuation() {
        assert_eq!(
            parse("Dim the Kitchen lights to 30%."),
            Some(Intent::LightDim {
                room: "kitchen".into(),
                percent: 30
            })
        );
    }

    #[test]
    fn dim_ambiguous_the_lights_rejected() {
        // Without a room, the optional `the` group leaves room="the".
        assert_eq!(parse("dim the lights to 50%"), None);
        assert_eq!(parse("set the lights to 50 percent"), None);
    }

    #[test]
    fn dim_does_not_claim_set_on_off() {
        // `set ... on/off` lacks the trailing "to N%" so the dim
        // regex must not claim it. (The on/off light regex's
        // alt-phrasing is lenient enough to match this with room =
        // "set the kitchen" — a known quirk of that pattern, not in
        // scope for this PR.)
        let result = parse("set the kitchen light on");
        assert!(
            !matches!(result, Some(Intent::LightDim { .. })),
            "light_dim must not claim {result:?}"
        );
    }

    #[test]
    fn dim_set_on_off_still_routes_to_lightset() {
        // Sanity: the original on/off regex still wins for its own
        // phrasing.
        assert_eq!(
            parse("turn off the kitchen light"),
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

    #[test]
    fn timer_short_form_units() {
        assert_eq!(
            parse("set a timer for 15 min"),
            Some(Intent::TimerSet {
                duration: Duration::from_secs(15 * 60),
                name: None
            })
        );
        assert_eq!(
            parse("set a timer for 1 hr"),
            Some(Intent::TimerSet {
                duration: Duration::from_secs(3600),
                name: None
            })
        );
        assert_eq!(
            parse("set a timer for 45 secs"),
            Some(Intent::TimerSet {
                duration: Duration::from_secs(45),
                name: None
            })
        );
    }

    #[test]
    fn timer_short_form_with_name_and_plural() {
        assert_eq!(
            parse("8 minutes timer called pasta"),
            Some(Intent::TimerSet {
                duration: Duration::from_secs(8 * 60),
                name: Some("pasta".into())
            })
        );
    }

    #[test]
    fn timer_overflow_returns_none() {
        // u64::MAX minutes would overflow when multiplied by 60.
        let input = format!("set a timer for {} minutes", u64::MAX);
        assert_eq!(parse(&input), None);
    }

    // ---- Back to normal ----

    #[test]
    fn back_to_normal_whole_home() {
        assert_eq!(
            parse("back to normal"),
            Some(Intent::ClearManualMode { room: None })
        );
    }

    #[test]
    fn back_to_normal_normal_lights_phrasing() {
        assert_eq!(
            parse("normal lights"),
            Some(Intent::ClearManualMode { room: None })
        );
    }

    #[test]
    fn back_to_normal_in_room() {
        assert_eq!(
            parse("back to normal in kitchen"),
            Some(Intent::ClearManualMode {
                room: Some("kitchen".into())
            })
        );
    }

    #[test]
    fn back_to_normal_in_the_room() {
        assert_eq!(
            parse("back to normal in the kitchen"),
            Some(Intent::ClearManualMode {
                room: Some("kitchen".into())
            })
        );
    }

    #[test]
    fn back_to_normal_multiword_room() {
        assert_eq!(
            parse("back to normal in the living room"),
            Some(Intent::ClearManualMode {
                room: Some("living room".into())
            })
        );
    }

    #[test]
    fn back_to_normal_room_prefix_phrasing() {
        assert_eq!(
            parse("living room back to normal"),
            Some(Intent::ClearManualMode {
                room: Some("living room".into())
            })
        );
    }

    #[test]
    fn back_to_normal_normalizes_case_and_punctuation() {
        assert_eq!(
            parse("Back to normal."),
            Some(Intent::ClearManualMode { room: None })
        );
        assert_eq!(
            parse("BACK TO NORMAL"),
            Some(Intent::ClearManualMode { room: None })
        );
    }

    #[test]
    fn back_to_normal_rejects_substring_in_other_phrase() {
        // Anchored regex must NOT match "back to the normal way".
        assert_eq!(parse("back to the normal way"), None);
    }

    #[test]
    fn back_to_normal_in_the_lights_rejected() {
        // "lights" isn't a room — must escalate to Tier 1.
        assert_eq!(parse("back to normal in the lights"), None);
    }

    // ---- Scene save ----

    #[test]
    fn scene_save_this_as() {
        assert_eq!(
            parse("save this as kitchen evening"),
            Some(Intent::SceneSave {
                name: "kitchen evening".into(),
                room: None,
            })
        );
    }

    #[test]
    fn scene_save_room_as_name() {
        assert_eq!(
            parse("save the kitchen as evening"),
            Some(Intent::SceneSave {
                name: "evening".into(),
                room: Some("kitchen".into()),
            })
        );
    }

    #[test]
    fn scene_save_room_as_name_multiword_room() {
        assert_eq!(
            parse("save the living room as cozy"),
            Some(Intent::SceneSave {
                name: "cozy".into(),
                room: Some("living room".into()),
            })
        );
    }

    #[test]
    fn scene_save_this_scene_as() {
        // "this scene" must not leak into the captured name as a room.
        assert_eq!(
            parse("save this scene as kitchen evening"),
            Some(Intent::SceneSave {
                name: "kitchen evening".into(),
                room: None,
            })
        );
    }

    #[test]
    fn scene_save_room_scene_as_name() {
        // "scene" between the room and "as" must not be captured as
        // part of the room (regression guard: previously produced
        // room="kitchen scene").
        assert_eq!(
            parse("save the kitchen scene as evening"),
            Some(Intent::SceneSave {
                name: "evening".into(),
                room: Some("kitchen".into()),
            })
        );
    }

    #[test]
    fn scene_save_multiword_room_scene_as_name() {
        assert_eq!(
            parse("save the living room scene as cozy"),
            Some(Intent::SceneSave {
                name: "cozy".into(),
                room: Some("living room".into()),
            })
        );
    }

    #[test]
    fn scene_save_short_form() {
        assert_eq!(
            parse("save kitchen evening"),
            Some(Intent::SceneSave {
                name: "kitchen evening".into(),
                room: None,
            })
        );
    }

    // ---- Scene apply ----

    #[test]
    fn scene_apply_explicit() {
        assert_eq!(
            parse("apply kitchen evening"),
            Some(Intent::SceneApply {
                name: "kitchen evening".into(),
            })
        );
    }

    #[test]
    fn scene_apply_suffix() {
        assert_eq!(
            parse("kitchen evening scene"),
            Some(Intent::SceneApply {
                name: "kitchen evening".into(),
            })
        );
    }

    #[test]
    fn scene_apply_prefix() {
        assert_eq!(
            parse("scene kitchen evening"),
            Some(Intent::SceneApply {
                name: "kitchen evening".into(),
            })
        );
    }

    #[test]
    fn scene_apply_bare_name_rejected() {
        assert_eq!(parse("kitchen evening"), None);
    }

    #[test]
    fn scene_normalizes_case_and_punctuation() {
        assert_eq!(
            parse("Apply Kitchen Evening."),
            Some(Intent::SceneApply {
                name: "kitchen evening".into(),
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
    fn ambiguous_the_lights_rejected() {
        // No room mentioned — must not silently produce room="the".
        // Caller is expected to escalate to Tier 1.
        assert_eq!(parse("turn on the lights"), None);
        assert_eq!(parse("turn off the lights"), None);
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
