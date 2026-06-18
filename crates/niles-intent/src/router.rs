//! The Tier 0 router itself.

use crate::devices::DeviceIndex;
use crate::intent::Intent;
use niles_core::{DeviceId, RoomName};
use regex::Regex;
use std::sync::OnceLock;
use std::time::Duration;

const SPEAKER_VOLUME_STEP: i16 = 10; // tunable; SonosClient clamps to 0..=100

/// Matches the Hue-dimmer hold step (see Event::DeviceAction).
pub(crate) const LIGHT_STEP_PERCENT: i16 = 10;
const WARMER_DELTA: i16 = -200;
const COOLER_DELTA: i16 = 200;

const KELVIN_WARM_WHITE: u16 = 2200;
const KELVIN_COOL_WHITE: u16 = 4000;
const KELVIN_DAYLIGHT: u16 = 5500;

fn named_white_to_kelvin(named: &str) -> Option<u16> {
    match named {
        "warm white" | "warm" => Some(KELVIN_WARM_WHITE),
        "cool white" | "cool" => Some(KELVIN_COOL_WHITE),
        "daylight" => Some(KELVIN_DAYLIGHT),
        _ => None,
    }
}

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
            .or_else(|| match_light_set_all_in_room(&t))
            .or_else(|| match_light_set_all(&t))
            .or_else(|| match_light_step(&t))
            .or_else(|| match_light_kelvin_step(&t))
            .or_else(|| match_light_kelvin_set(&t))
            .or_else(|| match_light(&t))
            .or_else(|| match_back_to_normal(&t))
            .or_else(|| match_scene_save(&t))
            .or_else(|| match_scene_apply(&t))
            .or_else(|| match_scene_list(&t))
            .or_else(|| match_scene_delete(&t))
            .or_else(|| match_media_play(&t))
            .or_else(|| match_media_pause(&t))
            .or_else(|| match_media_next(&t))
            .or_else(|| match_media_previous(&t))
            .or_else(|| match_media_volume_set(&t))
            .or_else(|| match_media_volume_step(&t))
            .or_else(|| match_timer(&t))
            .or_else(|| match_timer_cancel(&t))
            .or_else(|| match_timer_list(&t))
            .or_else(|| match_timer_remaining(&t))
            .or_else(|| match_stop_cancel(&t))
    }

    /// Try the existing Tier-0 patterns first, then fall through to
    /// context-aware matchers that need a [`DeviceIndex`] and optional
    /// origin room.
    pub fn parse_with_context(&self, transcript: &str, ctx: RouterContext<'_>) -> Option<Intent> {
        // Existing patterns take precedence so behaviour doesn't change.
        if let Some(intent) = self.parse(transcript) {
            return Some(intent);
        }

        let t = normalize(transcript);
        match_light_step_implicit_room(&t, &ctx)
            .or_else(|| match_light_kelvin_step_implicit_room(&t, &ctx))
            .or_else(|| match_light_kelvin_set_implicit_room(&t, &ctx))
            .or_else(|| match_light_dim_implicit_room(&t, &ctx))
            .or_else(|| match_light_set_implicit_room(&t, &ctx))
            .or_else(|| match_device_dim(&t, &ctx))
            .or_else(|| match_device_set(&t, &ctx))
            .or_else(|| match_media_next_implicit_room(&t, &ctx))
            .or_else(|| match_media_previous_implicit_room(&t, &ctx))
    }
}

/// Context passed to context-aware matchers.
#[derive(Debug, Clone, Copy)]
pub struct RouterContext<'a> {
    pub device_index: &'a DeviceIndex,
    pub origin_room: Option<&'a RoomName>,
}

/// Lowercase, trim, collapse internal whitespace, strip trailing
/// sentence punctuation (`.`, `!`, `?`, `,`, `;`, `:`).
///
/// Note: we explicitly enumerate the trailing chars to strip rather
/// than "anything non-alphanumeric" — otherwise `%` gets eaten and
/// `dim the kitchen lights to 30%` becomes `... to 30`, which the
/// `light_dim` regex can't anchor on.
pub(crate) fn normalize(s: &str) -> String {
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

// ---- Light set all (whole-home) -------------------------------------------

fn light_set_all_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?x)
              ^
              (?:
                turn\s+(?P<state1>on|off)\s+all\s+(?:the\s+)?lights?
              |
                all\s+(?:the\s+)?lights?\s+(?P<state2>on|off)
              |
                everything\s+off
              |
                lights\s+off\s+everywhere
              )
              $",
        )
        .expect("light_set_all regex compiles")
    })
}

fn match_light_set_all(t: &str) -> Option<Intent> {
    let caps = light_set_all_regex().captures(t)?;
    let on = caps
        .name("state1")
        .or_else(|| caps.name("state2"))
        .map(|m| m.as_str() == "on")
        .unwrap_or(false);
    Some(Intent::LightSetAll { on })
}

// ---- Light set all in room ------------------------------------------------

fn light_set_all_in_room_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?x)
              ^
              (?:
                turn\s+(?P<state1>on|off)\s+all\s+(?:the\s+)?lights?\s+in\s+(?:the\s+)?(?P<room1>.+?)
              |
                all\s+(?:the\s+)?lights?\s+in\s+(?:the\s+)?(?P<room2>.+?)\s+(?P<state2>on|off)
              )
              $",
        )
        .expect("light_set_all_in_room regex compiles")
    })
}

fn match_light_set_all_in_room(t: &str) -> Option<Intent> {
    let caps = light_set_all_in_room_regex().captures(t)?;
    let (state, room) = if let Some(s) = caps.name("state1") {
        (s.as_str(), caps.name("room1")?.as_str())
    } else {
        (caps.name("state2")?.as_str(), caps.name("room2")?.as_str())
    };
    if room == "the" || room == "lights" {
        return None;
    }
    Some(Intent::LightSet {
        room: room.to_string(),
        on: state == "on",
    })
}

// ---- Light step (explicit room) -------------------------------------------

fn light_step_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?x)
              ^
              (?:
                make\s+(?:the\s+)?(?P<room2>.+?)(?:\s+lights?)?\s+(?P<dir2>brighter|dimmer)
              |
                (?:the\s+)?(?P<room1>.+?)\s+lights?\s+(?P<dir1>brighter|dimmer)
              )
              $",
        )
        .expect("light_step regex compiles")
    })
}

/// Rejects pronouns and other degenerate captures that leak through
/// the optional `lights?` anchor in the `make ... <room> <dir>` branch.
fn is_degenerate_room(room: &str) -> bool {
    room == "the"
        || room == "lights"
        || room == "it"
        || room == "them"
        || room == "that"
        || room == "this"
        || room == "everything"
}

fn match_light_step(t: &str) -> Option<Intent> {
    let caps = light_step_regex().captures(t)?;
    let dir = caps.name("dir1").or_else(|| caps.name("dir2"))?.as_str();
    let room = caps.name("room1").or_else(|| caps.name("room2"))?.as_str();
    // The `make ... brighter` branch has no `lights?` anchor, so
    // pronouns like "it" or "them" get captured as bogus rooms.
    // Reject so the caller can escalate to Tier 1.
    if is_degenerate_room(room) {
        return None;
    }
    let delta_percent = if dir == "brighter" {
        LIGHT_STEP_PERCENT
    } else {
        -LIGHT_STEP_PERCENT
    };
    Some(Intent::LightStep {
        room: room.to_string(),
        delta_percent,
    })
}

// ---- Light kelvin step (explicit room) ------------------------------------

fn light_kelvin_step_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?x)
              ^
              (?:
                make\s+(?:the\s+)?(?P<room2>.+?)(?:\s+lights?)?\s+(?P<dir2>warmer|cooler)
              |
                (?:the\s+)?(?P<room1>.+?)\s+lights?\s+(?P<dir1>warmer|cooler)
              )
              $",
        )
        .expect("light_kelvin_step regex compiles")
    })
}

fn match_light_kelvin_step(t: &str) -> Option<Intent> {
    let caps = light_kelvin_step_regex().captures(t)?;
    let dir = caps.name("dir1").or_else(|| caps.name("dir2"))?.as_str();
    let room = caps.name("room1").or_else(|| caps.name("room2"))?.as_str();
    if is_degenerate_room(room) {
        return None;
    }
    let delta_kelvin = if dir == "warmer" {
        WARMER_DELTA
    } else {
        COOLER_DELTA
    };
    Some(Intent::LightKelvinStep {
        room: room.to_string(),
        delta_kelvin,
    })
}

// ---- Light kelvin set (explicit room) ------------------------------------

fn light_kelvin_set_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?x)
              ^
              (?:
                make\s+(?:the\s+)?(?P<room2>.+?)(?:\s+lights?)?\s+(?P<named2>warm\s+white|cool\s+white|daylight|warm|cool)
              |
                (?:the\s+)?(?P<room1>.+?)\s+lights?\s+(?P<named1>warm\s+white|cool\s+white|daylight|warm|cool)
              )
              $",
        )
        .expect("light_kelvin_set regex compiles")
    })
}

fn match_light_kelvin_set(t: &str) -> Option<Intent> {
    let caps = light_kelvin_set_regex().captures(t)?;
    let named = caps
        .name("named1")
        .or_else(|| caps.name("named2"))?
        .as_str();
    let room = caps.name("room1").or_else(|| caps.name("room2"))?.as_str();
    if is_degenerate_room(room) {
        return None;
    }
    let kelvin = named_white_to_kelvin(named)?;
    Some(Intent::LightKelvinSet {
        room: room.to_string(),
        kelvin,
    })
}

// ---- Light set implicit room ----------------------------------------------

fn light_set_implicit_room_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?x)
              ^
              (?:
                lights?\s+(?P<state1>on|off)
              |
                turn\s+(?P<state2>on|off)\s+(?:the\s+)?lights?
              )
              $",
        )
        .expect("light_set_implicit_room regex compiles")
    })
}

fn match_light_set_implicit_room(t: &str, ctx: &RouterContext<'_>) -> Option<Intent> {
    let caps = light_set_implicit_room_regex().captures(t)?;
    let origin = ctx.origin_room?;
    let state = caps
        .name("state1")
        .or_else(|| caps.name("state2"))?
        .as_str();
    Some(Intent::LightSet {
        room: origin.as_str().to_owned(),
        on: state == "on",
    })
}

// ---- Light dim implicit room ----------------------------------------------

fn light_dim_implicit_room_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?x)
              ^
              (?:dim|set)\s+(?:the\s+)?lights?\s+to\s+(?P<n>\d{1,3})\s*(?:%|percent)
              $",
        )
        .expect("light_dim_implicit_room regex compiles")
    })
}

fn match_light_dim_implicit_room(t: &str, ctx: &RouterContext<'_>) -> Option<Intent> {
    let caps = light_dim_implicit_room_regex().captures(t)?;
    let origin = ctx.origin_room?;
    let n: u8 = caps.name("n")?.as_str().parse().ok()?;
    if n > 100 {
        return None;
    }
    Some(Intent::LightDim {
        room: origin.as_str().to_owned(),
        percent: n,
    })
}

// ---- Light step implicit room ---------------------------------------------

fn light_step_implicit_room_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?x)
              ^
              (?P<dir>brighter|dimmer)
              $",
        )
        .expect("light_step_implicit_room regex compiles")
    })
}

fn match_light_step_implicit_room(t: &str, ctx: &RouterContext<'_>) -> Option<Intent> {
    let caps = light_step_implicit_room_regex().captures(t)?;
    let origin = ctx.origin_room?;
    let dir = caps.name("dir")?.as_str();
    let delta_percent = if dir == "brighter" {
        LIGHT_STEP_PERCENT
    } else {
        -LIGHT_STEP_PERCENT
    };
    Some(Intent::LightStep {
        room: origin.as_str().to_owned(),
        delta_percent,
    })
}

// ---- Light kelvin step implicit room --------------------------------------

fn light_kelvin_step_implicit_room_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?x)
              ^
              (?P<dir>warmer|cooler)
              $",
        )
        .expect("light_kelvin_step_implicit_room regex compiles")
    })
}

fn match_light_kelvin_step_implicit_room(t: &str, ctx: &RouterContext<'_>) -> Option<Intent> {
    let caps = light_kelvin_step_implicit_room_regex().captures(t)?;
    let origin = ctx.origin_room?;
    let dir = caps.name("dir")?.as_str();
    let delta_kelvin = if dir == "warmer" {
        WARMER_DELTA
    } else {
        COOLER_DELTA
    };
    Some(Intent::LightKelvinStep {
        room: origin.as_str().to_owned(),
        delta_kelvin,
    })
}

// ---- Light kelvin set implicit room --------------------------------------

fn light_kelvin_set_implicit_room_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?x)
              ^
              (?P<named>warm\s+white|cool\s+white|daylight)
              $",
        )
        .expect("light_kelvin_set_implicit_room regex compiles")
    })
}

fn match_light_kelvin_set_implicit_room(t: &str, ctx: &RouterContext<'_>) -> Option<Intent> {
    let caps = light_kelvin_set_implicit_room_regex().captures(t)?;
    let origin = ctx.origin_room?;
    let named = caps.name("named")?.as_str();
    let kelvin = named_white_to_kelvin(named)?;
    Some(Intent::LightKelvinSet {
        room: origin.as_str().to_owned(),
        kelvin,
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
    let from_suffix_form = caps.name("name2").is_some();
    let name = caps
        .name("name1")
        .or_else(|| caps.name("name2"))
        .or_else(|| caps.name("name3"))?
        .as_str()
        .trim();
    if name.is_empty() || name == "the" || name == "lights" {
        return None;
    }
    // Reject only the ambiguous "<name> scene" form when it starts with
    // "delete"/"remove"; explicit "apply <name>" and "scene <name>" should
    // still allow scene names like "delete party".
    if from_suffix_form && (name.starts_with("delete ") || name.starts_with("remove ")) {
        return None;
    }
    Some(Intent::SceneApply {
        name: name.to_string(),
    })
}

// ---- Scene list -----------------------------------------------------------

fn scene_list_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?x)
              ^
              (?:
                (?:list|show)\s+(?:me\s+)?(?:my\s+)?scenes
              |
                what\s+scenes\s+do\s+i\s+have
              )
              $",
        )
        .expect("scene_list regex compiles")
    })
}

fn match_scene_list(t: &str) -> Option<Intent> {
    scene_list_regex().is_match(t).then_some(Intent::SceneList)
}

// ---- Scene delete ---------------------------------------------------------

fn scene_delete_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?x)
              ^
              (?:
                (?:delete|remove)\s+(?:the\s+)?(?P<name1>.+?)\s+scene
              |
                (?:delete|remove)\s+scene\s+(?P<name2>.+)
              )
              $",
        )
        .expect("scene_delete regex compiles")
    })
}

fn match_scene_delete(t: &str) -> Option<Intent> {
    let caps = scene_delete_regex().captures(t)?;
    let name = caps
        .name("name1")
        .or_else(|| caps.name("name2"))?
        .as_str()
        .trim();
    if name.is_empty() || name == "the" || name == "lights" {
        return None;
    }
    Some(Intent::SceneDelete {
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

// ---- Media pause -----------------------------------------------------------

fn media_pause_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?x)
              ^
              (?:
                pause\s+music\s+in\s+(?:the\s+)?(?P<room2>.+?)
              |
                pause\s+(?:the\s+)?(?P<room>.+?)(?:\s+music)?
              )
              $",
        )
        .expect("media_pause regex compiles")
    })
}

fn match_media_pause(t: &str) -> Option<Intent> {
    let caps = media_pause_regex().captures(t)?;
    let room = caps.name("room").or_else(|| caps.name("room2"))?.as_str();
    if room == "the" || room == "music" {
        return None;
    }
    Some(Intent::MediaPause {
        room: room.to_string(),
    })
}

// ---- Media play ------------------------------------------------------------

fn media_play_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?x)
              ^
              (?:
                (?:play|resume)\s+music\s+in\s+(?:the\s+)?(?P<room2>.+?)
              |
                (?:play|resume)\s+(?:the\s+)?(?P<room>.+?)
              )
              $",
        )
        .expect("media_play regex compiles")
    })
}

fn match_media_play(t: &str) -> Option<Intent> {
    let caps = media_play_regex().captures(t)?;
    let room = caps.name("room").or_else(|| caps.name("room2"))?.as_str();
    if room == "the" || room == "music" || room == "set" || room == "volume" {
        return None;
    }
    Some(Intent::MediaPlay {
        room: room.to_string(),
    })
}

// ---- Media next ------------------------------------------------------------

fn media_next_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?x)
              ^
              (?:
                (?:next|skip)(?:\s+(?:song|track))?\s+in\s+(?:the\s+)?(?P<room2>.+?)
              |
                (?:the\s+)?(?P<room>.+?)\s+(?:next|skip)(?:\s+(?:song|track))?
              )
              $",
        )
        .expect("media_next regex compiles")
    })
}

fn match_media_next(t: &str) -> Option<Intent> {
    let caps = media_next_regex().captures(t)?;
    let room = caps.name("room").or_else(|| caps.name("room2"))?.as_str();
    if room == "the" || room == "music" || room == "song" || room == "track" {
        return None;
    }
    Some(Intent::MediaNext {
        room: room.to_string(),
    })
}

// ---- Media previous --------------------------------------------------------

fn media_previous_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?x)
              ^
              (?:
                (?:previous|back|go\s+back)(?:\s+(?:song|track))?\s+in\s+(?:the\s+)?(?P<room2>.+?)
              |
                (?:the\s+)?(?P<room>.+?)\s+previous(?:\s+(?:song|track))?
              )
              $",
        )
        .expect("media_previous regex compiles")
    })
}

fn match_media_previous(t: &str) -> Option<Intent> {
    let caps = media_previous_regex().captures(t)?;
    let room = caps.name("room").or_else(|| caps.name("room2"))?.as_str();
    if room == "the" || room == "music" || room == "song" || room == "track" {
        return None;
    }
    Some(Intent::MediaPrevious {
        room: room.to_string(),
    })
}

// ---- Media next implicit room ----------------------------------------------

fn media_next_implicit_room_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?x)
              ^
              (?:next|skip)(?:\s+(?:song|track))?
              $",
        )
        .expect("media_next_implicit_room regex compiles")
    })
}

fn match_media_next_implicit_room(t: &str, ctx: &RouterContext<'_>) -> Option<Intent> {
    media_next_implicit_room_regex().captures(t)?;
    let origin = ctx.origin_room?;
    Some(Intent::MediaNext {
        room: origin.as_str().to_owned(),
    })
}

// ---- Media previous implicit room ------------------------------------------

fn media_previous_implicit_room_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?x)
              ^
              (?:previous|go\s+back|back)(?:\s+(?:song|track))?
              $",
        )
        .expect("media_previous_implicit_room regex compiles")
    })
}

fn match_media_previous_implicit_room(t: &str, ctx: &RouterContext<'_>) -> Option<Intent> {
    media_previous_implicit_room_regex().captures(t)?;
    let origin = ctx.origin_room?;
    Some(Intent::MediaPrevious {
        room: origin.as_str().to_owned(),
    })
}

// ---- Media volume set ------------------------------------------------------

fn media_volume_set_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?x)
              ^
              (?:
                set\s+(?:the\s+)?(?P<room1>.+?)\s+volume\s+to\s+(?P<n1>\d+)\s*(?:%|percent)
              |
                (?:the\s+)?(?P<room2>.+?)\s+volume\s+to\s+(?P<n2>\d+)\s*(?:%|percent)
              |
                set\s+(?:the\s+)?volume\s+in\s+(?:the\s+)?(?P<room3>.+?)\s+to\s+(?P<n3>\d+)\s*(?:%|percent)
              )
              $",
        )
        .expect("media_volume_set regex compiles")
    })
}

fn match_media_volume_set(t: &str) -> Option<Intent> {
    let caps = media_volume_set_regex().captures(t)?;
    let room = caps
        .name("room1")
        .or_else(|| caps.name("room2"))
        .or_else(|| caps.name("room3"))?
        .as_str();
    if room == "the" || room == "music" || room == "set" {
        return None;
    }
    let n_str = caps
        .name("n1")
        .or_else(|| caps.name("n2"))
        .or_else(|| caps.name("n3"))?
        .as_str();
    let n: u8 = n_str.parse().ok()?;
    if n > 100 {
        return None;
    }
    Some(Intent::MediaVolumeSet {
        room: room.to_string(),
        percent: n,
    })
}

// ---- Media volume step -----------------------------------------------------

fn media_volume_step_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?x)
              ^
              (?:
                volume\s+(?P<dir>up|down)(?:\s+in)?(?:\s+the)?\s+(?P<room1>.+?)
              |
                (?:the\s+)?(?P<room2>.+?)\s+volume\s+(?P<dir2>up|down)
              )
              $",
        )
        .expect("media_volume_step regex compiles")
    })
}

fn match_media_volume_step(t: &str) -> Option<Intent> {
    let caps = media_volume_step_regex().captures(t)?;
    let room = caps.name("room1").or_else(|| caps.name("room2"))?.as_str();
    if room == "the" || room == "music" {
        return None;
    }
    let dir = caps.name("dir").or_else(|| caps.name("dir2"))?.as_str();
    let delta = if dir == "up" {
        SPEAKER_VOLUME_STEP
    } else {
        -SPEAKER_VOLUME_STEP
    };
    Some(Intent::MediaVolumeStep {
        room: room.to_string(),
        delta,
    })
}

// ---- Timer -----------------------------------------------------------------

fn timer_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // Two phrasings:
        //   "(set|start|create|make)? (a|an|the|my)? timer for <n> <unit>[s] (called <name>)?"
        //   "<n> <unit>[s] timer (called <name>)?"
        // The leading verb + article are both optional so natural phrasings
        // ("set the timer for…", "start a timer for…", bare "timer for…") all
        // hit Tier 0 instead of falling through to the LLM.
        // Units: seconds / minutes / hours (+ common short forms).
        Regex::new(
            r"(?x)
              ^
              (?:
                (?:(?:set|start|create|make)\s+)?(?:(?:a|an|the|my)\s+)?timer\s+for\s+(?P<n1>\d+)\s+(?P<unit1>seconds?|secs?|minutes?|mins?|hours?|hrs?)
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

// ---- Timer cancel (named) ------------------------------------------------

fn timer_cancel_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?x)
              ^
              (?:cancel|stop)\s+(?:the\s+|my\s+)?(?P<name>.+?)\s+timer
              $",
        )
        .expect("timer_cancel regex compiles")
    })
}

fn match_timer_cancel(t: &str) -> Option<Intent> {
    let caps = timer_cancel_regex().captures(t)?;
    let name = caps.name("name")?.as_str().trim();
    if name.is_empty() || name == "the" || name == "my" || name == "lights" {
        return None;
    }
    Some(Intent::TimerCancel {
        name: name.to_string(),
    })
}

// ---- Timer list ----------------------------------------------------------

fn timer_list_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?x)
              ^
              (?:
                (?:list|show)\s+(?:me\s+)?(?:my\s+)?timers
              |
                what\s+timers\s+do\s+i\s+have
              )
              $",
        )
        .expect("timer_list regex compiles")
    })
}

fn match_timer_list(t: &str) -> Option<Intent> {
    timer_list_regex().is_match(t).then_some(Intent::TimerList)
}

// ---- Timer remaining (query time left) -----------------------------------

fn timer_remaining_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // "how long [time] [is] left/remaining [on the/my timer]"
        // "how much time [is] left/remaining …"  /  "how much longer …"
        // "[the] time [is] left/remaining …"  /  "time left …"
        Regex::new(
            r"(?x)
              ^
              (?:
                how\s+long\s+(?:time\s+)?(?:is\s+)?(?:left|remaining)
              | how\s+much\s+time\s+(?:is\s+)?(?:left|remaining)
              | how\s+much\s+longer
              | (?:the\s+)?time\s+(?:is\s+)?(?:left|remaining)
              )
              (?:\s+(?:on|for)\s+(?:the\s+|my\s+)?timer)?
              $",
        )
        .expect("timer_remaining regex compiles")
    })
}

fn match_timer_remaining(t: &str) -> Option<Intent> {
    timer_remaining_regex()
        .is_match(t)
        .then_some(Intent::TimerRemaining)
}

// ---- Stop / cancel ---------------------------------------------------------

fn match_stop_cancel(t: &str) -> Option<Intent> {
    match t {
        "stop" | "stop timer" | "stop the timer" | "stop my timer" => Some(Intent::Stop),
        "cancel" | "cancel timer" | "cancel the timer" | "cancel my timer" => Some(Intent::Cancel),
        _ => None,
    }
}

// ---- Device set (on/off by name) ------------------------------------------

fn device_set_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?x)
              ^
              turn\s+(?P<state>on|off)\s+(?:the\s+)?(?P<device>.+?)
              (?:\s+in\s+(?:the\s+)?(?P<room>.+))?
              $",
        )
        .expect("device_set regex compiles")
    })
}

fn match_device_set(t: &str, ctx: &RouterContext<'_>) -> Option<Intent> {
    let caps = device_set_regex().captures(t)?;
    let state = caps.name("state")?.as_str();
    let device_phrase = caps.name("device")?.as_str();
    if device_phrase.is_empty()
        || device_phrase == "the"
        || device_phrase == "light"
        || device_phrase == "lights"
    {
        return None;
    }
    let on = state == "on";
    let candidates = ctx.device_index.matches(device_phrase);
    let room_hint = caps.name("room").map(|m| m.as_str());
    let id = resolve_device(candidates, room_hint, ctx.origin_room)?;
    Some(Intent::DeviceSet { device_id: id, on })
}

// ---- Device dim (brightness by name) --------------------------------------

fn device_dim_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?x)
              ^
              (?:dim|set)\s+(?:the\s+)?(?P<device>.+?)
              (?:\s+in\s+(?:the\s+)?(?P<room>.+?))?
              \s+to\s+(?P<n>\d{1,3})\s*(?:%|percent)
              $",
        )
        .expect("device_dim regex compiles")
    })
}

fn match_device_dim(t: &str, ctx: &RouterContext<'_>) -> Option<Intent> {
    let caps = device_dim_regex().captures(t)?;
    let device_phrase = caps.name("device")?.as_str();
    if device_phrase.is_empty()
        || device_phrase == "the"
        || device_phrase == "light"
        || device_phrase == "lights"
    {
        return None;
    }
    let n: u8 = caps.name("n")?.as_str().parse().ok()?;
    if n > 100 {
        return None;
    }
    let candidates = ctx.device_index.matches(device_phrase);
    let room_hint = caps.name("room").map(|m| m.as_str());
    let id = resolve_device(candidates, room_hint, ctx.origin_room)?;
    Some(Intent::DeviceDim {
        device_id: id,
        percent: n,
    })
}

// ---- Device resolution ----------------------------------------------------

fn resolve_device(
    candidates: &[DeviceId],
    room_hint: Option<&str>,
    origin_room: Option<&RoomName>,
) -> Option<DeviceId> {
    match candidates {
        [] => None,
        [id] => Some(id.clone()),
        _ => {
            let target_room = room_hint
                .and_then(|r| {
                    let normalized = r.trim().to_ascii_lowercase().replace([' ', '\t'], "_");
                    RoomName::parse(&normalized).ok()
                })
                .or_else(|| origin_room.cloned())?;
            candidates
                .iter()
                .find(|id| id.room() == &target_room)
                .cloned()
        }
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

    #[test]
    fn cancel_the_pasta_timer_returns_timer_cancel() {
        assert_eq!(
            parse("cancel the pasta timer"),
            Some(Intent::TimerCancel {
                name: "pasta".into(),
            })
        );
    }

    #[test]
    fn stop_the_pasta_timer_alt_form() {
        assert_eq!(
            parse("stop the pasta timer"),
            Some(Intent::TimerCancel {
                name: "pasta".into(),
            })
        );
    }

    #[test]
    fn cancel_timer_without_name_maps_to_cancel() {
        // "cancel timer" has no name segment, so it isn't a named TimerCancel;
        // it (and bare "cancel") resolves to the generic Intent::Cancel.
        assert_eq!(parse("cancel timer"), Some(Intent::Cancel));
        assert_eq!(parse("cancel"), Some(Intent::Cancel));
    }

    #[test]
    fn stop_or_cancel_the_timer_maps_to_ack_intents() {
        // All the natural unnamed phrasings resolve to the generic ack intents
        // at Tier 0 — they never reach the LLM.
        assert_eq!(parse("stop the timer"), Some(Intent::Stop));
        assert_eq!(parse("stop timer"), Some(Intent::Stop));
        assert_eq!(parse("stop my timer"), Some(Intent::Stop));
        assert_eq!(parse("cancel the timer"), Some(Intent::Cancel));
        assert_eq!(parse("cancel my timer"), Some(Intent::Cancel));
    }

    #[test]
    fn set_timer_natural_phrasings_hit_tier0() {
        // The exact command that previously fell through to the LLM.
        assert_eq!(
            parse("Set the timer for 10 seconds."),
            Some(Intent::TimerSet {
                duration: Duration::from_secs(10),
                name: None,
            })
        );
        assert_eq!(
            parse("start a timer for 5 minutes"),
            Some(Intent::TimerSet {
                duration: Duration::from_secs(300),
                name: None,
            })
        );
        assert_eq!(
            parse("set my timer for 1 minute"),
            Some(Intent::TimerSet {
                duration: Duration::from_secs(60),
                name: None,
            })
        );
    }

    #[test]
    fn list_my_timers_returns_timer_list() {
        assert_eq!(parse("list my timers"), Some(Intent::TimerList));
    }

    #[test]
    fn show_me_my_timers_returns_timer_list() {
        assert_eq!(parse("show me my timers"), Some(Intent::TimerList));
    }

    #[test]
    fn what_timers_do_i_have_returns_timer_list() {
        assert_eq!(parse("what timers do I have"), Some(Intent::TimerList));
    }

    // ---- Timer remaining (query) ----

    #[test]
    fn timer_remaining_natural_phrasings() {
        // The user's exact phrasing, plus common variants — all Tier 0.
        assert_eq!(
            parse("how long time is left on the timer"),
            Some(Intent::TimerRemaining)
        );
        assert_eq!(parse("how much time is left"), Some(Intent::TimerRemaining));
        assert_eq!(parse("how long is left"), Some(Intent::TimerRemaining));
        assert_eq!(
            parse("how much longer on the timer"),
            Some(Intent::TimerRemaining)
        );
        assert_eq!(
            parse("how much time is remaining on my timer"),
            Some(Intent::TimerRemaining)
        );
        assert_eq!(
            parse("time left on the timer"),
            Some(Intent::TimerRemaining)
        );
    }

    #[test]
    fn timer_remaining_does_not_swallow_other_commands() {
        // Must not be mistaken for set/cancel/list, and unrelated speech
        // shouldn't match.
        assert_ne!(
            parse("set a timer for 5 minutes"),
            Some(Intent::TimerRemaining)
        );
        assert_ne!(
            parse("turn off the kitchen light"),
            Some(Intent::TimerRemaining)
        );
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

    // ---- Scene list ----

    #[test]
    fn scene_list_my_scenes() {
        assert_eq!(parse("list my scenes"), Some(Intent::SceneList));
    }

    #[test]
    fn scene_list_bare() {
        assert_eq!(parse("list scenes"), Some(Intent::SceneList));
    }

    #[test]
    fn scene_list_show_me_my_scenes() {
        assert_eq!(parse("show me my scenes"), Some(Intent::SceneList));
    }

    #[test]
    fn scene_list_what_scenes_do_i_have() {
        assert_eq!(parse("what scenes do I have"), Some(Intent::SceneList));
    }

    // ---- Scene delete ----

    #[test]
    fn scene_delete_the_name_scene() {
        assert_eq!(
            parse("delete the kitchen evening scene"),
            Some(Intent::SceneDelete {
                name: "kitchen evening".into(),
            })
        );
    }

    #[test]
    fn scene_delete_remove_name_scene_no_the() {
        assert_eq!(
            parse("remove kitchen evening scene"),
            Some(Intent::SceneDelete {
                name: "kitchen evening".into(),
            })
        );
    }

    #[test]
    fn scene_delete_scene_name_form() {
        assert_eq!(
            parse("delete scene kitchen evening"),
            Some(Intent::SceneDelete {
                name: "kitchen evening".into(),
            })
        );
    }

    #[test]
    fn scene_delete_remove_scene_name_form() {
        assert_eq!(
            parse("remove scene kitchen evening"),
            Some(Intent::SceneDelete {
                name: "kitchen evening".into(),
            })
        );
    }

    #[test]
    fn scene_delete_single_word_name() {
        assert_eq!(
            parse("remove the kitchen scene"),
            Some(Intent::SceneDelete {
                name: "kitchen".into(),
            })
        );
    }

    #[test]
    fn scene_delete_normalizes_case_and_punctuation() {
        assert_eq!(
            parse("Delete the Kitchen Evening Scene!"),
            Some(Intent::SceneDelete {
                name: "kitchen evening".into(),
            })
        );
    }

    #[test]
    fn scene_delete_lights_rejected() {
        assert_eq!(parse("delete the lights scene"), None);
    }

    #[test]
    fn scene_apply_still_wins_over_delete_for_suffix_phrasing() {
        assert_eq!(
            parse("kitchen evening scene"),
            Some(Intent::SceneApply {
                name: "kitchen evening".into(),
            })
        );
    }

    #[test]
    fn scene_apply_explicit_form_allows_delete_prefixed_name() {
        assert_eq!(
            parse("apply delete party"),
            Some(Intent::SceneApply {
                name: "delete party".into(),
            })
        );
    }

    #[test]
    fn scene_apply_scene_prefix_form_allows_remove_prefixed_name() {
        assert_eq!(
            parse("scene remove clutter"),
            Some(Intent::SceneApply {
                name: "remove clutter".into(),
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

    // ---- Media pause ----

    #[test]
    fn media_pause_simple() {
        assert_eq!(
            parse("pause the living room"),
            Some(Intent::MediaPause {
                room: "living room".into(),
            })
        );
    }

    #[test]
    fn media_pause_music_in() {
        assert_eq!(
            parse("pause music in the kitchen"),
            Some(Intent::MediaPause {
                room: "kitchen".into(),
            })
        );
    }

    #[test]
    fn media_pause_trailing_music() {
        assert_eq!(
            parse("pause the living room music"),
            Some(Intent::MediaPause {
                room: "living room".into(),
            })
        );
    }

    // ---- Media play ----

    #[test]
    fn media_play_simple() {
        assert_eq!(
            parse("play the kitchen"),
            Some(Intent::MediaPlay {
                room: "kitchen".into(),
            })
        );
    }

    #[test]
    fn media_play_music_in() {
        assert_eq!(
            parse("play music in the living room"),
            Some(Intent::MediaPlay {
                room: "living room".into(),
            })
        );
    }

    #[test]
    fn media_resume() {
        assert_eq!(
            parse("resume the kitchen"),
            Some(Intent::MediaPlay {
                room: "kitchen".into(),
            })
        );
    }

    // ---- Media next ----

    #[test]
    fn media_next_song_in_room() {
        assert_eq!(
            parse("next song in the kitchen"),
            Some(Intent::MediaNext {
                room: "kitchen".into(),
            })
        );
    }

    #[test]
    fn media_skip_in_room() {
        assert_eq!(
            parse("skip in the living room"),
            Some(Intent::MediaNext {
                room: "living room".into(),
            })
        );
    }

    #[test]
    fn media_next_track_in_room() {
        assert_eq!(
            parse("next track in the bedroom"),
            Some(Intent::MediaNext {
                room: "bedroom".into(),
            })
        );
    }

    #[test]
    fn media_next_room_first_phrasing() {
        assert_eq!(
            parse("kitchen next song"),
            Some(Intent::MediaNext {
                room: "kitchen".into(),
            })
        );
    }

    #[test]
    fn media_next_room_first_bare() {
        assert_eq!(
            parse("kitchen next"),
            Some(Intent::MediaNext {
                room: "kitchen".into(),
            })
        );
    }

    // ---- Media previous ----

    #[test]
    fn media_previous_song_in_room() {
        assert_eq!(
            parse("previous song in the kitchen"),
            Some(Intent::MediaPrevious {
                room: "kitchen".into(),
            })
        );
    }

    #[test]
    fn media_go_back_in_room() {
        assert_eq!(
            parse("go back in the kitchen"),
            Some(Intent::MediaPrevious {
                room: "kitchen".into(),
            })
        );
    }

    #[test]
    fn media_back_in_room() {
        assert_eq!(
            parse("back in the living room"),
            Some(Intent::MediaPrevious {
                room: "living room".into(),
            })
        );
    }

    #[test]
    fn media_previous_track_in_room() {
        assert_eq!(
            parse("previous track in the bedroom"),
            Some(Intent::MediaPrevious {
                room: "bedroom".into(),
            })
        );
    }

    #[test]
    fn media_previous_room_first_phrasing() {
        assert_eq!(
            parse("kitchen previous song"),
            Some(Intent::MediaPrevious {
                room: "kitchen".into(),
            })
        );
    }

    #[test]
    fn media_previous_room_first_bare() {
        assert_eq!(
            parse("kitchen previous"),
            Some(Intent::MediaPrevious {
                room: "kitchen".into(),
            })
        );
    }

    // ---- Media rejections ----

    #[test]
    fn media_next_degenerate_room_rejected() {
        assert_eq!(parse("next song in the the"), None);
        assert_eq!(parse("next song in the music"), None);
        assert_eq!(parse("next song in the song"), None);
        assert_eq!(parse("next song in the track"), None);
    }

    #[test]
    fn media_previous_degenerate_room_rejected() {
        assert_eq!(parse("previous song in the the"), None);
        assert_eq!(parse("previous song in the music"), None);
        assert_eq!(parse("previous song in the song"), None);
        assert_eq!(parse("previous song in the track"), None);
    }

    #[test]
    fn media_next_alone_rejected() {
        assert_eq!(parse("next song"), None);
        assert_eq!(parse("skip"), None);
    }

    #[test]
    fn media_previous_alone_rejected() {
        assert_eq!(parse("previous song"), None);
        assert_eq!(parse("go back"), None);
        assert_eq!(parse("back"), None);
    }

    // ---- Media volume set ----

    #[test]
    fn media_volume_set_set_prefix() {
        assert_eq!(
            parse("set the kitchen volume to 30%"),
            Some(Intent::MediaVolumeSet {
                room: "kitchen".into(),
                percent: 30,
            })
        );
    }

    #[test]
    fn media_volume_set_no_prefix() {
        assert_eq!(
            parse("kitchen volume to 30 percent"),
            Some(Intent::MediaVolumeSet {
                room: "kitchen".into(),
                percent: 30,
            })
        );
    }

    #[test]
    fn media_volume_set_in_room() {
        assert_eq!(
            parse("set the volume in the kitchen to 40%"),
            Some(Intent::MediaVolumeSet {
                room: "kitchen".into(),
                percent: 40,
            })
        );
    }

    #[test]
    fn media_volume_set_rejects_over_100() {
        assert_eq!(parse("set kitchen volume to 150%"), None);
    }

    // ---- Media volume step ----

    #[test]
    fn media_volume_step_up_in() {
        assert_eq!(
            parse("volume up in the kitchen"),
            Some(Intent::MediaVolumeStep {
                room: "kitchen".into(),
                delta: 10,
            })
        );
    }

    #[test]
    fn media_volume_step_room_first() {
        assert_eq!(
            parse("kitchen volume up"),
            Some(Intent::MediaVolumeStep {
                room: "kitchen".into(),
                delta: 10,
            })
        );
    }

    #[test]
    fn media_volume_step_room_first_down() {
        assert_eq!(
            parse("kitchen volume down"),
            Some(Intent::MediaVolumeStep {
                room: "kitchen".into(),
                delta: -10,
            })
        );
    }

    #[test]
    fn media_volume_step_down() {
        assert_eq!(
            parse("volume down in the living room"),
            Some(Intent::MediaVolumeStep {
                room: "living room".into(),
                delta: -10,
            })
        );
    }

    // ---- Media rejections ----

    #[test]
    fn media_pause_the_alone_rejected() {
        assert_eq!(parse("pause the"), None);
    }

    #[test]
    fn media_pause_alone_rejected() {
        assert_eq!(parse("pause"), None);
    }

    #[test]
    fn media_normalizes_case_and_trailing_punctuation() {
        assert_eq!(
            parse("PAUSE the Living Room."),
            Some(Intent::MediaPause {
                room: "living room".into(),
            })
        );
    }

    #[test]
    fn media_next_normalizes_case_and_trailing_punctuation() {
        assert_eq!(
            parse("SKIP Track in the Living Room."),
            Some(Intent::MediaNext {
                room: "living room".into(),
            })
        );
    }

    #[test]
    fn media_volume_set_boundary_0() {
        assert_eq!(
            parse("set kitchen volume to 0%"),
            Some(Intent::MediaVolumeSet {
                room: "kitchen".into(),
                percent: 0,
            })
        );
    }

    #[test]
    fn media_volume_set_boundary_100() {
        assert_eq!(
            parse("set kitchen volume to 100%"),
            Some(Intent::MediaVolumeSet {
                room: "kitchen".into(),
                percent: 100,
            })
        );
    }

    #[test]
    fn media_volume_set_rejects_101() {
        assert_eq!(parse("set kitchen volume to 101%"), None);
    }

    #[test]
    fn media_volume_set_rejects_missing_percent() {
        assert_eq!(parse("set kitchen volume to 30"), None);
    }

    #[test]
    fn media_volume_set_rejects_missing_room() {
        assert_eq!(parse("set volume to 30%"), None);
        assert_eq!(parse("the volume to 30 percent"), None);
    }

    #[test]
    fn media_volume_step_without_in() {
        assert_eq!(
            parse("volume up kitchen"),
            Some(Intent::MediaVolumeStep {
                room: "kitchen".into(),
                delta: 10,
            })
        );
    }

    #[test]
    fn media_play_alone_rejected() {
        assert_eq!(parse("play"), None);
    }

    // ---- Light set all (whole-home) ----

    #[test]
    fn light_set_all_turn_off_all_lights() {
        assert_eq!(
            parse("turn off all the lights"),
            Some(Intent::LightSetAll { on: false })
        );
    }

    #[test]
    fn light_set_all_alt_phrasing() {
        assert_eq!(
            parse("all the lights off"),
            Some(Intent::LightSetAll { on: false })
        );
        assert_eq!(
            parse("all lights on"),
            Some(Intent::LightSetAll { on: true })
        );
    }

    #[test]
    fn light_set_all_everything_off() {
        assert_eq!(
            parse("everything off"),
            Some(Intent::LightSetAll { on: false })
        );
        assert_eq!(
            parse("Everything off."),
            Some(Intent::LightSetAll { on: false })
        );
    }

    #[test]
    fn light_set_all_lights_off_everywhere() {
        assert_eq!(
            parse("lights off everywhere"),
            Some(Intent::LightSetAll { on: false })
        );
    }

    // ---- Light set all in room ----

    #[test]
    fn light_set_all_in_room_off() {
        assert_eq!(
            parse("turn off all the lights in the living room"),
            Some(Intent::LightSet {
                room: "living room".into(),
                on: false,
            })
        );
    }

    #[test]
    fn light_set_all_in_room_subject_first() {
        assert_eq!(
            parse("all the lights in the bedroom off"),
            Some(Intent::LightSet {
                room: "bedroom".into(),
                on: false,
            })
        );
    }

    // ---- Light step (explicit room) ----

    #[test]
    fn light_step_room_lights_brighter() {
        assert_eq!(
            parse("kitchen lights brighter"),
            Some(Intent::LightStep {
                room: "kitchen".into(),
                delta_percent: 10,
            })
        );
        assert_eq!(
            parse("kitchen lights dimmer"),
            Some(Intent::LightStep {
                room: "kitchen".into(),
                delta_percent: -10,
            })
        );
    }

    // ---- Light kelvin step (explicit room) ----

    #[test]
    fn light_kelvin_step_room_lights_warmer() {
        assert_eq!(
            parse("kitchen lights warmer"),
            Some(Intent::LightKelvinStep {
                room: "kitchen".into(),
                delta_kelvin: -200,
            })
        );
        assert_eq!(
            parse("kitchen lights cooler"),
            Some(Intent::LightKelvinStep {
                room: "kitchen".into(),
                delta_kelvin: 200,
            })
        );
    }

    #[test]
    fn light_kelvin_step_make_room_warmer() {
        assert_eq!(
            parse("make the kitchen warmer"),
            Some(Intent::LightKelvinStep {
                room: "kitchen".into(),
                delta_kelvin: -200,
            })
        );
        assert_eq!(
            parse("make the kitchen lights warmer"),
            Some(Intent::LightKelvinStep {
                room: "kitchen".into(),
                delta_kelvin: -200,
            })
        );
        assert_eq!(
            parse("make kitchen lights cooler"),
            Some(Intent::LightKelvinStep {
                room: "kitchen".into(),
                delta_kelvin: 200,
            })
        );
    }

    #[test]
    fn light_kelvin_step_rejects_pronouns() {
        assert_eq!(parse("make it warmer"), None);
        assert_eq!(parse("make them cooler"), None);
        assert_eq!(parse("make everything warmer"), None);
    }

    #[test]
    fn light_kelvin_step_rejects_bare_room() {
        // "kitchen warmer" without "lights" or "make" must NOT match.
        assert_eq!(parse("kitchen warmer"), None);
        assert_eq!(parse("kitchen cooler"), None);
    }

    #[test]
    fn brighter_dimmer_still_routes_to_lightstep() {
        // The new kelvin regex must not steal brighter/dimmer utterances.
        assert_eq!(
            parse("kitchen lights brighter"),
            Some(Intent::LightStep {
                room: "kitchen".into(),
                delta_percent: 10,
            })
        );
        assert_eq!(
            parse("make the kitchen dimmer"),
            Some(Intent::LightStep {
                room: "kitchen".into(),
                delta_percent: -10,
            })
        );
    }

    // ---- Light kelvin set (explicit room) ----

    #[test]
    fn light_kelvin_set_warm_white() {
        assert_eq!(
            parse("kitchen lights warm white"),
            Some(Intent::LightKelvinSet {
                room: "kitchen".into(),
                kelvin: 2200,
            })
        );
    }

    #[test]
    fn light_kelvin_set_cool_white() {
        assert_eq!(
            parse("kitchen lights cool white"),
            Some(Intent::LightKelvinSet {
                room: "kitchen".into(),
                kelvin: 4000,
            })
        );
    }

    #[test]
    fn light_kelvin_set_daylight() {
        assert_eq!(
            parse("kitchen lights daylight"),
            Some(Intent::LightKelvinSet {
                room: "kitchen".into(),
                kelvin: 5500,
            })
        );
    }

    #[test]
    fn light_kelvin_set_make_form() {
        assert_eq!(
            parse("make the kitchen warm white"),
            Some(Intent::LightKelvinSet {
                room: "kitchen".into(),
                kelvin: 2200,
            })
        );
        assert_eq!(
            parse("make the kitchen lights cool white"),
            Some(Intent::LightKelvinSet {
                room: "kitchen".into(),
                kelvin: 4000,
            })
        );
    }

    #[test]
    fn light_kelvin_set_bare_warm_with_room() {
        assert_eq!(
            parse("make the kitchen warm"),
            Some(Intent::LightKelvinSet {
                room: "kitchen".into(),
                kelvin: 2200,
            })
        );
        assert_eq!(
            parse("make the kitchen cool"),
            Some(Intent::LightKelvinSet {
                room: "kitchen".into(),
                kelvin: 4000,
            })
        );
    }

    #[test]
    fn light_kelvin_set_make_form_daylight() {
        assert_eq!(
            parse("make the kitchen daylight"),
            Some(Intent::LightKelvinSet {
                room: "kitchen".into(),
                kelvin: 5500,
            })
        );
    }

    #[test]
    fn light_kelvin_set_the_prefix() {
        assert_eq!(
            parse("the kitchen lights warm white"),
            Some(Intent::LightKelvinSet {
                room: "kitchen".into(),
                kelvin: 2200,
            })
        );
    }

    #[test]
    fn light_kelvin_set_rejects_pronouns() {
        assert_eq!(parse("make it warm white"), None);
        assert_eq!(parse("make them daylight"), None);
    }

    #[test]
    fn light_kelvin_set_step_still_routes_first() {
        // Ordering regression: kelvin-step must run before kelvin-set
        // so "warmer" / "cooler" aren't swallowed by "warm" / "cool" prefix.
        assert_eq!(
            parse("kitchen lights warmer"),
            Some(Intent::LightKelvinStep {
                room: "kitchen".into(),
                delta_kelvin: -200,
            })
        );
        assert_eq!(
            parse("kitchen lights cooler"),
            Some(Intent::LightKelvinStep {
                room: "kitchen".into(),
                delta_kelvin: 200,
            })
        );
    }

    #[test]
    fn light_kelvin_set_normalizes_case_and_punctuation() {
        assert_eq!(
            parse("Kitchen lights Warm White."),
            Some(Intent::LightKelvinSet {
                room: "kitchen".into(),
                kelvin: 2200,
            })
        );
    }

    #[test]
    fn light_step_make_room_brighter() {
        assert_eq!(
            parse("make the kitchen brighter"),
            Some(Intent::LightStep {
                room: "kitchen".into(),
                delta_percent: 10,
            })
        );
        assert_eq!(
            parse("make the kitchen lights brighter"),
            Some(Intent::LightStep {
                room: "kitchen".into(),
                delta_percent: 10,
            })
        );
        assert_eq!(
            parse("make kitchen lights dimmer"),
            Some(Intent::LightStep {
                room: "kitchen".into(),
                delta_percent: -10,
            })
        );
    }

    #[test]
    fn light_step_rejects_pronouns() {
        // Without a `lights?` anchor, the `make ... brighter` branch
        // would capture pronouns as bogus rooms. They must fall
        // through to Tier 1.
        assert_eq!(parse("make it brighter"), None);
        assert_eq!(parse("make them dimmer"), None);
        assert_eq!(parse("make everything brighter"), None);
    }

    // ---- Regressions ----

    #[test]
    fn regression_kitchen_lights_on_unchanged() {
        assert_eq!(
            parse("turn on the kitchen lights"),
            Some(Intent::LightSet {
                room: "kitchen".into(),
                on: true,
            })
        );
    }

    #[test]
    fn regression_dim_kitchen_lights_to_30_unchanged() {
        assert_eq!(
            parse("dim the kitchen lights to 30%"),
            Some(Intent::LightDim {
                room: "kitchen".into(),
                percent: 30,
            })
        );
    }
}

#[cfg(test)]
mod context_tests {
    use super::*;
    use niles_core::{DeviceName, RoomName};

    fn make_id(room: &str, name: &str) -> DeviceId {
        DeviceId::new(
            "z2m",
            RoomName::parse(room).unwrap(),
            DeviceName::parse(name).unwrap(),
        )
        .unwrap()
    }

    /// Index with one unique floor_lamp and one unique desk_lamp.
    fn fixture_unique() -> DeviceIndex {
        let mut idx = DeviceIndex::new();
        idx.insert(make_id("living_room", "floor_lamp"));
        idx.insert(make_id("kitchen", "desk_lamp"));
        idx
    }

    /// Index with two floor_lamps across rooms (for disambiguation tests).
    fn fixture_multi() -> DeviceIndex {
        let mut idx = DeviceIndex::new();
        idx.insert(make_id("living_room", "floor_lamp"));
        idx.insert(make_id("bedroom", "floor_lamp"));
        idx.insert(make_id("kitchen", "ceiling_light"));
        idx
    }

    fn ctx_with<'a>(idx: &'a DeviceIndex, origin: Option<&'a RoomName>) -> RouterContext<'a> {
        RouterContext {
            device_index: idx,
            origin_room: origin,
        }
    }

    fn parse_with(transcript: &str, ctx: RouterContext<'_>) -> Option<Intent> {
        IntentRouter::new().parse_with_context(transcript, ctx)
    }

    #[test]
    fn turn_on_floor_lamp_unique_device() {
        let idx = fixture_unique();
        let living_floor = make_id("living_room", "floor_lamp");
        let ctx = ctx_with(&idx, None);
        assert_eq!(
            parse_with("turn on the floor lamp", ctx),
            Some(Intent::DeviceSet {
                device_id: living_floor,
                on: true,
            })
        );
    }

    #[test]
    fn turn_off_floor_lamp_unique_device() {
        let idx = fixture_unique();
        let living_floor = make_id("living_room", "floor_lamp");
        let ctx = ctx_with(&idx, None);
        assert_eq!(
            parse_with("turn off the floor lamp", ctx),
            Some(Intent::DeviceSet {
                device_id: living_floor,
                on: false,
            })
        );
    }

    #[test]
    fn turn_on_floor_lamp_with_room_hint() {
        let idx = fixture_multi();
        let bedroom_floor = make_id("bedroom", "floor_lamp");
        let ctx = ctx_with(&idx, None);
        assert_eq!(
            parse_with("turn on the floor lamp in the living room", ctx),
            Some(Intent::DeviceSet {
                device_id: make_id("living_room", "floor_lamp"),
                on: true,
            })
        );
        assert_eq!(
            parse_with("turn on the floor lamp in the bedroom", ctx),
            Some(Intent::DeviceSet {
                device_id: bedroom_floor,
                on: true,
            })
        );
    }

    #[test]
    fn turn_on_floor_lamp_with_origin_room() {
        let idx = fixture_multi();
        let bedroom = RoomName::parse("bedroom").unwrap();
        let ctx = ctx_with(&idx, Some(&bedroom));
        assert_eq!(
            parse_with("turn on the floor lamp", ctx),
            Some(Intent::DeviceSet {
                device_id: make_id("bedroom", "floor_lamp"),
                on: true,
            })
        );
    }

    #[test]
    fn ambiguous_device_without_hint_or_origin_returns_none() {
        let idx = fixture_multi();
        let ctx = ctx_with(&idx, None);
        assert_eq!(parse_with("turn on the floor lamp", ctx), None);
    }

    #[test]
    fn unknown_device_name_returns_none() {
        let idx = fixture_unique();
        let ctx = ctx_with(&idx, None);
        assert_eq!(parse_with("turn on the unicorn lamp", ctx), None);
    }

    #[test]
    fn dim_desk_lamp_unique_device() {
        let idx = fixture_unique();
        let desk = make_id("kitchen", "desk_lamp");
        let ctx = ctx_with(&idx, None);
        assert_eq!(
            parse_with("dim the desk lamp to 30%", ctx),
            Some(Intent::DeviceDim {
                device_id: desk,
                percent: 30,
            })
        );
    }

    #[test]
    fn set_floor_lamp_in_bedroom_to_50_percent() {
        let idx = fixture_multi();
        let bedroom_floor = make_id("bedroom", "floor_lamp");
        let ctx = ctx_with(&idx, None);
        assert_eq!(
            parse_with("set the floor lamp in the bedroom to 50 percent", ctx),
            Some(Intent::DeviceDim {
                device_id: bedroom_floor,
                percent: 50,
            })
        );
    }

    #[test]
    fn dim_rejects_over_100() {
        let idx = fixture_unique();
        let ctx = ctx_with(&idx, None);
        assert_eq!(parse_with("dim the desk lamp to 101%", ctx), None);
    }

    #[test]
    fn existing_light_patterns_still_work() {
        let idx = fixture_multi();
        let ctx = ctx_with(&idx, None);
        // These should be caught by parse() first, not fall through.
        assert_eq!(
            parse_with("turn on the lights", ctx),
            None // existing light regex rejects "the" as room
        );
        assert_eq!(
            parse_with("turn off the kitchen light", ctx),
            Some(Intent::LightSet {
                room: "kitchen".into(),
                on: false,
            })
        );
    }

    #[test]
    fn literal_light_guarded_out_of_device_matcher() {
        let idx = fixture_multi();
        let ctx = ctx_with(&idx, None);
        // "turn on the light" is guarded because device phrase == "light"
        assert_eq!(parse_with("turn on the light", ctx), None);
        assert_eq!(parse_with("turn on the lights", ctx), None);
    }

    #[test]
    fn dim_floor_lamp_with_origin_room() {
        let idx = fixture_multi();
        let bedroom = RoomName::parse("bedroom").unwrap();
        let ctx = ctx_with(&idx, Some(&bedroom));
        assert_eq!(
            parse_with("dim the floor lamp to 30%", ctx),
            Some(Intent::DeviceDim {
                device_id: make_id("bedroom", "floor_lamp"),
                percent: 30,
            })
        );
    }

    #[test]
    fn set_desk_lamp_unique_device() {
        let idx = fixture_unique();
        let desk = make_id("kitchen", "desk_lamp");
        let ctx = ctx_with(&idx, None);
        assert_eq!(
            parse_with("set the desk lamp to 30%", ctx),
            Some(Intent::DeviceDim {
                device_id: desk,
                percent: 30,
            })
        );
    }

    #[test]
    fn room_hint_no_match_returns_none() {
        let idx = fixture_multi();
        let ctx = ctx_with(&idx, None);
        // floor lamps exist in living_room and bedroom, not kitchen
        assert_eq!(
            parse_with("turn on the floor lamp in the kitchen", ctx),
            None
        );
    }

    #[test]
    fn dim_guarded_light_phrase() {
        let idx = fixture_multi();
        let ctx = ctx_with(&idx, None);
        assert_eq!(parse_with("dim the light to 30%", ctx), None);
        assert_eq!(parse_with("dim the lights to 30%", ctx), None);
    }

    #[test]
    fn dim_boundary_0() {
        let idx = fixture_unique();
        let desk = make_id("kitchen", "desk_lamp");
        let ctx = ctx_with(&idx, None);
        assert_eq!(
            parse_with("dim the desk lamp to 0%", ctx),
            Some(Intent::DeviceDim {
                device_id: desk,
                percent: 0,
            })
        );
    }

    // ---- Implicit-room light set ----

    #[test]
    fn implicit_room_lights_on_uses_origin() {
        let idx = fixture_multi();
        let kitchen = RoomName::parse("kitchen").unwrap();
        let ctx = ctx_with(&idx, Some(&kitchen));
        assert_eq!(
            parse_with("lights on", ctx),
            Some(Intent::LightSet {
                room: "kitchen".into(),
                on: true,
            })
        );
        assert_eq!(
            parse_with("turn on the lights", ctx),
            Some(Intent::LightSet {
                room: "kitchen".into(),
                on: true,
            })
        );
    }

    #[test]
    fn implicit_room_lights_off_no_origin_returns_none() {
        let idx = fixture_multi();
        let ctx = ctx_with(&idx, None);
        assert_eq!(parse_with("lights off", ctx), None);
        assert_eq!(parse_with("turn on the lights", ctx), None);
    }

    // ---- Implicit-room light dim ----

    #[test]
    fn implicit_room_dim_lights_to_30() {
        let idx = fixture_multi();
        let kitchen = RoomName::parse("kitchen").unwrap();
        let ctx = ctx_with(&idx, Some(&kitchen));
        assert_eq!(
            parse_with("dim the lights to 30%", ctx),
            Some(Intent::LightDim {
                room: "kitchen".into(),
                percent: 30,
            })
        );
    }

    #[test]
    fn implicit_room_dim_lights_over_100_returns_none() {
        let idx = fixture_multi();
        let kitchen = RoomName::parse("kitchen").unwrap();
        let ctx = ctx_with(&idx, Some(&kitchen));
        assert_eq!(parse_with("dim the lights to 150%", ctx), None);
    }

    // ---- Implicit-room light step ----

    #[test]
    fn light_step_brighter_uses_origin() {
        let idx = fixture_multi();
        let bedroom = RoomName::parse("bedroom").unwrap();
        let ctx = ctx_with(&idx, Some(&bedroom));
        assert_eq!(
            parse_with("brighter", ctx),
            Some(Intent::LightStep {
                room: "bedroom".into(),
                delta_percent: 10,
            })
        );
        assert_eq!(
            parse_with("dimmer", ctx),
            Some(Intent::LightStep {
                room: "bedroom".into(),
                delta_percent: -10,
            })
        );
    }

    #[test]
    fn light_step_no_origin_returns_none() {
        let idx = fixture_multi();
        let ctx = ctx_with(&idx, None);
        assert_eq!(parse_with("brighter", ctx), None);
        assert_eq!(parse_with("dimmer", ctx), None);
    }

    // ---- Implicit-room light kelvin step ----

    #[test]
    fn light_kelvin_step_warmer_uses_origin() {
        let idx = fixture_multi();
        let bedroom = RoomName::parse("bedroom").unwrap();
        let ctx = ctx_with(&idx, Some(&bedroom));
        assert_eq!(
            parse_with("warmer", ctx),
            Some(Intent::LightKelvinStep {
                room: "bedroom".into(),
                delta_kelvin: -200,
            })
        );
        assert_eq!(
            parse_with("cooler", ctx),
            Some(Intent::LightKelvinStep {
                room: "bedroom".into(),
                delta_kelvin: 200,
            })
        );
    }

    #[test]
    fn light_kelvin_step_no_origin_returns_none() {
        let idx = fixture_multi();
        let ctx = ctx_with(&idx, None);
        assert_eq!(parse_with("warmer", ctx), None);
        assert_eq!(parse_with("cooler", ctx), None);
    }

    #[test]
    fn light_kelvin_step_explicit_wins_over_origin() {
        let idx = fixture_multi();
        let living_room = RoomName::parse("living_room").unwrap();
        let ctx = ctx_with(&idx, Some(&living_room));
        // "kitchen lights warmer" should be caught by parse() first, not fall through.
        assert_eq!(
            parse_with("kitchen lights warmer", ctx),
            Some(Intent::LightKelvinStep {
                room: "kitchen".into(),
                delta_kelvin: -200,
            })
        );
    }

    // ---- Implicit-room light kelvin set ----

    #[test]
    fn light_kelvin_set_warm_white_uses_origin() {
        let idx = fixture_multi();
        let living_room = RoomName::parse("living_room").unwrap();
        let ctx = ctx_with(&idx, Some(&living_room));
        assert_eq!(
            parse_with("warm white", ctx),
            Some(Intent::LightKelvinSet {
                room: "living_room".into(),
                kelvin: 2200,
            })
        );
    }

    #[test]
    fn light_kelvin_set_cool_white_uses_origin() {
        let idx = fixture_multi();
        let living_room = RoomName::parse("living_room").unwrap();
        let ctx = ctx_with(&idx, Some(&living_room));
        assert_eq!(
            parse_with("cool white", ctx),
            Some(Intent::LightKelvinSet {
                room: "living_room".into(),
                kelvin: 4000,
            })
        );
    }

    #[test]
    fn light_kelvin_set_daylight_uses_origin() {
        let idx = fixture_multi();
        let living_room = RoomName::parse("living_room").unwrap();
        let ctx = ctx_with(&idx, Some(&living_room));
        assert_eq!(
            parse_with("daylight", ctx),
            Some(Intent::LightKelvinSet {
                room: "living_room".into(),
                kelvin: 5500,
            })
        );
    }

    #[test]
    fn light_kelvin_set_bare_warm_no_origin_returns_none() {
        let idx = fixture_multi();
        let ctx = ctx_with(&idx, None);
        assert_eq!(parse_with("warm", ctx), None);
        assert_eq!(parse_with("cool", ctx), None);
    }

    #[test]
    fn light_kelvin_set_bare_warm_with_origin_returns_none() {
        // Implicit-room form requires full two-word phrases; bare warm/cool
        // are too ambiguous even with origin room context.
        let idx = fixture_multi();
        let living_room = RoomName::parse("living_room").unwrap();
        let ctx = ctx_with(&idx, Some(&living_room));
        assert_eq!(parse_with("warm", ctx), None);
        assert_eq!(parse_with("cool", ctx), None);
    }

    #[test]
    fn light_kelvin_set_full_phrase_no_origin_returns_none() {
        let idx = fixture_multi();
        let ctx = ctx_with(&idx, None);
        assert_eq!(parse_with("warm white", ctx), None);
        assert_eq!(parse_with("cool white", ctx), None);
        assert_eq!(parse_with("daylight", ctx), None);
    }

    // ---- Implicit-room media next ----

    #[test]
    fn media_next_implicit_room_uses_origin() {
        let idx = fixture_multi();
        let kitchen = RoomName::parse("kitchen").unwrap();
        let ctx = ctx_with(&idx, Some(&kitchen));
        assert_eq!(
            parse_with("next song", ctx),
            Some(Intent::MediaNext {
                room: "kitchen".into(),
            })
        );
        assert_eq!(
            parse_with("skip", ctx),
            Some(Intent::MediaNext {
                room: "kitchen".into(),
            })
        );
        assert_eq!(
            parse_with("next track", ctx),
            Some(Intent::MediaNext {
                room: "kitchen".into(),
            })
        );
    }

    #[test]
    fn media_next_no_origin_returns_none() {
        let idx = fixture_multi();
        let ctx = ctx_with(&idx, None);
        assert_eq!(parse_with("next song", ctx), None);
        assert_eq!(parse_with("skip", ctx), None);
    }

    // ---- Implicit-room media previous ----

    #[test]
    fn media_previous_implicit_room_uses_origin() {
        let idx = fixture_multi();
        let living_room = RoomName::parse("living_room").unwrap();
        let ctx = ctx_with(&idx, Some(&living_room));
        assert_eq!(
            parse_with("previous song", ctx),
            Some(Intent::MediaPrevious {
                room: "living_room".into(),
            })
        );
        assert_eq!(
            parse_with("go back", ctx),
            Some(Intent::MediaPrevious {
                room: "living_room".into(),
            })
        );
        assert_eq!(
            parse_with("back", ctx),
            Some(Intent::MediaPrevious {
                room: "living_room".into(),
            })
        );
    }

    #[test]
    fn media_previous_no_origin_returns_none() {
        let idx = fixture_multi();
        let ctx = ctx_with(&idx, None);
        assert_eq!(parse_with("previous song", ctx), None);
        assert_eq!(parse_with("go back", ctx), None);
    }
}
