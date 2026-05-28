//! Spoken-response phrasing. Pure: (intent outcome) -> what niles
//! says. Kept separate from dispatch so the phrasings are
//! unit-testable without a DispatchCtx, and so the TTS layer has
//! one string to synthesize.

use niles_core::DeviceId;
use std::time::Duration;

/// Convert a canonical room name (`living_room`) to spoken form
/// (`living room`).
fn spoken_room(room: &str) -> String {
    room.replace('_', " ")
}

/// Uppercase the first ASCII character; non-ASCII first chars are left as-is.
fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_ascii_uppercase().to_string() + chars.as_str(),
    }
}

/// Oxford-comma join for spoken lists.
///
/// - `[]` → `""`
/// - `[a]` → `"a"`
/// - `[a, b]` → `"a and b"`
/// - `[a, b, c, ...]` → `"a, b, and c"` (Oxford comma always)
fn join_spoken(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [single] => single.clone(),
        [a, b] => format!("{a} and {b}"),
        _ => {
            let (last, head) = items.split_last().unwrap();
            format!("{}, and {}", head.join(", "), last)
        }
    }
}

/// Largest exact whole-unit phrasing for a duration.
///
/// Returns `(value, unit_singular)` so the caller can pluralize.
fn format_duration_phrase(duration: Duration) -> (u64, &'static str) {
    match duration.as_secs() {
        n if n >= 3600 && n % 3600 == 0 => (n / 3600, "hour"),
        n if n >= 60 && n % 60 == 0 => (n / 60, "minute"),
        n => (n, "second"),
    }
}

/// "Kitchen lights on." / "Living room lights off."
pub fn light_set(room: &str, on: bool) -> String {
    format!(
        "{} lights {}.",
        capitalize_first(&spoken_room(room)),
        if on { "on" } else { "off" }
    )
}

/// "Kitchen lights to 30%."
pub fn light_dim(room: &str, percent: u8) -> String {
    format!(
        "{} lights to {}%.",
        capitalize_first(&spoken_room(room)),
        percent
    )
}

/// "All lights on." / "All lights off."
pub fn all_lights(on: bool) -> String {
    if on {
        "All lights on.".into()
    } else {
        "All lights off.".into()
    }
}

/// "No lights found."
pub fn no_lights() -> String {
    "No lights found.".into()
}

/// "Saved the scene kitchen evening."
pub fn scene_saved(name: &str) -> String {
    format!("Saved the scene {}.", spoken_room(name))
}

/// "Kitchen evening."
pub fn scene_applied(name: &str) -> String {
    format!("{}.", capitalize_first(&spoken_room(name)))
}

/// "I don't have a scene called kitchen evening."
pub fn scene_not_found(name: &str) -> String {
    format!("I don't have a scene called {}.", spoken_room(name))
}

/// "The scene kitchen evening is empty."
pub fn scene_empty(name: &str) -> String {
    format!("The scene {} is empty.", spoken_room(name))
}

/// "You don't have any saved scenes." / "You have 2 scenes: a and b."
pub fn scene_list(names: &[String]) -> String {
    if names.is_empty() {
        return "You don't have any saved scenes.".into();
    }
    let spoken: Vec<String> = names.iter().map(|n| spoken_room(n)).collect();
    format!(
        "You have {} {}: {}.",
        names.len(),
        if names.len() == 1 { "scene" } else { "scenes" },
        join_spoken(&spoken)
    )
}

/// "Deleted the scene kitchen evening."
pub fn scene_deleted(name: &str) -> String {
    format!("Deleted the scene {}.", spoken_room(name))
}

/// "Back to normal." / "Back to normal in the kitchen."
pub fn cleared_manual(room: Option<&str>) -> String {
    match room {
        None => "Back to normal.".into(),
        Some(r) => format!("Back to normal in the {}.", spoken_room(r)),
    }
}

/// "5 minute timer started." / "Pasta timer started."
pub fn timer_started(duration: Duration, name: Option<&str>) -> String {
    if let Some(n) = name {
        return format!("{} timer started.", capitalize_first(n));
    }
    let (v, unit) = format_duration_phrase(duration);
    let unit = if v == 1 {
        unit.to_string()
    } else {
        format!("{unit}s")
    };
    format!("{v} {unit} timer started.")
}

/// "Cancelled the pasta timer." / "I don't have a timer called pasta."
pub fn timer_cancelled(name: &str, count: usize) -> String {
    match (count, name.is_empty()) {
        (0, true) => "No timers running.".into(),
        (0, false) => format!("I don't have a timer called {name}."),
        (1, true) => "Cancelled your timer.".into(),
        (_, true) => format!("Cancelled {count} timers."),
        (1, false) => format!("Cancelled the {name} timer."),
        _ => format!("Cancelled {count} {name} timers."),
    }
}

/// "No timers running." / "You have 2 timers."
pub fn timer_list(count: usize) -> String {
    match count {
        0 => "No timers running.".into(),
        1 => "You have 1 timer.".into(),
        n => format!("You have {n} timers."),
    }
}

/// "Stopped." / "Nothing's running."
pub fn stopped(was_ringing: bool) -> String {
    match was_ringing {
        true => "Stopped.".into(),
        false => "Nothing's running.".into(),
    }
}

/// "I couldn't find a room called office."
pub fn room_not_found(room: &str) -> String {
    format!("I couldn't find a room called {}.", spoken_room(room))
}

/// "I couldn't find any lights in the kitchen."
pub fn room_no_devices(room: &str) -> String {
    format!("I couldn't find any lights in the {}.", spoken_room(room))
}

/// "Still waking up, try again in a moment."
pub fn room_warming_up() -> String {
    "Still waking up, try again in a moment.".into()
}

/// "Playing in the living room."
pub fn media_play(room: &str) -> String {
    format!("Playing in the {}.", spoken_room(room))
}

/// "Paused in the kitchen."
pub fn media_pause(room: &str) -> String {
    format!("Paused in the {}.", spoken_room(room))
}

/// "Kitchen volume to 30%."
pub fn media_volume(room: &str, percent: u8) -> String {
    format!(
        "{} volume to {}%.",
        capitalize_first(&spoken_room(room)),
        percent
    )
}

/// "Living room floor lamp on." / "Bedroom floor lamp off."
pub fn device_set(id: &DeviceId, on: bool) -> String {
    let phrase = format!(
        "{} {} {}.",
        spoken_room(id.room().as_str()),
        spoken_room(id.name().as_str()),
        if on { "on" } else { "off" }
    );
    capitalize_first(&phrase)
}

/// "Living room floor lamp to 30%."
pub fn device_dim(id: &DeviceId, percent: u8) -> String {
    let phrase = format!(
        "{} {} to {}%.",
        spoken_room(id.room().as_str()),
        spoken_room(id.name().as_str()),
        percent
    );
    capitalize_first(&phrase)
}

/// "I couldn't find that device anymore."
pub fn device_not_found(_id: &DeviceId) -> String {
    "I couldn't find that device anymore.".into()
}

/// "No speaker in the office."
pub fn no_speaker_in_room(room: &str) -> String {
    format!("No speaker in the {}.", spoken_room(room))
}

/// "I couldn't reach the speaker in the kitchen."
pub fn speaker_unreachable(room: &str) -> String {
    format!("I couldn't reach the speaker in the {}.", spoken_room(room))
}

/// Generic fallback for intents that don't have a specific phrase yet.
pub fn fallback() -> String {
    "I'm not sure how to help with that.".into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn light_set_on() {
        assert_eq!(light_set("kitchen", true), "Kitchen lights on.");
    }

    #[test]
    fn light_set_off() {
        assert_eq!(light_set("kitchen", false), "Kitchen lights off.");
    }

    #[test]
    fn light_set_multiword_room() {
        assert_eq!(light_set("living_room", true), "Living room lights on.");
    }

    #[test]
    fn light_dim_basic() {
        assert_eq!(light_dim("kitchen", 30), "Kitchen lights to 30%.");
    }

    #[test]
    fn all_lights_on() {
        assert_eq!(all_lights(true), "All lights on.");
    }

    #[test]
    fn all_lights_off() {
        assert_eq!(all_lights(false), "All lights off.");
    }

    #[test]
    fn no_lights_phrasing() {
        assert_eq!(no_lights(), "No lights found.");
    }

    #[test]
    fn scene_saved_underscore() {
        assert_eq!(
            scene_saved("kitchen_evening"),
            "Saved the scene kitchen evening."
        );
    }

    #[test]
    fn scene_applied_capitalizes() {
        assert_eq!(scene_applied("kitchen_evening"), "Kitchen evening.");
    }

    #[test]
    fn scene_not_found_phrasing() {
        assert_eq!(
            scene_not_found("kitchen_evening"),
            "I don't have a scene called kitchen evening."
        );
    }

    #[test]
    fn scene_empty_phrasing() {
        assert_eq!(
            scene_empty("kitchen_evening"),
            "The scene kitchen evening is empty."
        );
    }

    #[test]
    fn scene_deleted_phrasing() {
        assert_eq!(
            scene_deleted("kitchen_evening"),
            "Deleted the scene kitchen evening."
        );
    }

    #[test]
    fn scene_list_empty() {
        assert_eq!(scene_list(&[]), "You don't have any saved scenes.");
    }

    #[test]
    fn scene_list_one() {
        assert_eq!(scene_list(&["cozy".into()]), "You have 1 scene: cozy.");
    }

    #[test]
    fn scene_list_two() {
        assert_eq!(
            scene_list(&["cozy".into(), "movie_night".into()]),
            "You have 2 scenes: cozy and movie night."
        );
    }

    #[test]
    fn scene_list_three() {
        assert_eq!(
            scene_list(&[
                "cozy".into(),
                "kitchen_evening".into(),
                "movie_night".into(),
            ]),
            "You have 3 scenes: cozy, kitchen evening, and movie night."
        );
    }

    #[test]
    fn cleared_manual_none() {
        assert_eq!(cleared_manual(None), "Back to normal.");
    }

    #[test]
    fn cleared_manual_some() {
        assert_eq!(
            cleared_manual(Some("kitchen")),
            "Back to normal in the kitchen."
        );
    }

    #[test]
    fn timer_started_minutes() {
        assert_eq!(
            timer_started(Duration::from_secs(300), None),
            "5 minutes timer started."
        );
    }

    #[test]
    fn timer_started_singular_minute() {
        assert_eq!(
            timer_started(Duration::from_secs(60), None),
            "1 minute timer started."
        );
    }

    #[test]
    fn timer_started_hour() {
        assert_eq!(
            timer_started(Duration::from_secs(3600), None),
            "1 hour timer started."
        );
    }

    #[test]
    fn timer_started_seconds() {
        assert_eq!(
            timer_started(Duration::from_secs(30), None),
            "30 seconds timer started."
        );
    }

    #[test]
    fn timer_started_named() {
        assert_eq!(
            timer_started(Duration::from_secs(300), Some("pasta")),
            "Pasta timer started."
        );
    }

    #[test]
    fn timer_cancelled_hit() {
        assert_eq!(timer_cancelled("pasta", 1), "Cancelled the pasta timer.");
    }

    #[test]
    fn timer_cancelled_miss() {
        assert_eq!(
            timer_cancelled("pasta", 0),
            "I don't have a timer called pasta."
        );
    }

    #[test]
    fn timer_cancelled_unnamed_miss() {
        assert_eq!(timer_cancelled("", 0), "No timers running.");
    }

    #[test]
    fn timer_cancelled_unnamed() {
        assert_eq!(timer_cancelled("", 1), "Cancelled your timer.");
    }

    #[test]
    fn timer_cancelled_plural() {
        assert_eq!(timer_cancelled("pasta", 2), "Cancelled 2 pasta timers.");
    }

    #[test]
    fn timer_cancelled_unnamed_plural() {
        assert_eq!(timer_cancelled("", 3), "Cancelled 3 timers.");
    }

    #[test]
    fn timer_list_zero() {
        assert_eq!(timer_list(0), "No timers running.");
    }

    #[test]
    fn timer_list_one() {
        assert_eq!(timer_list(1), "You have 1 timer.");
    }

    #[test]
    fn timer_list_two() {
        assert_eq!(timer_list(2), "You have 2 timers.");
    }

    #[test]
    fn stopped_was_ringing() {
        assert_eq!(stopped(true), "Stopped.");
    }

    #[test]
    fn stopped_idle() {
        assert_eq!(stopped(false), "Nothing's running.");
    }

    #[test]
    fn room_not_found_phrasing() {
        assert_eq!(
            room_not_found("office"),
            "I couldn't find a room called office."
        );
    }

    #[test]
    fn room_no_devices_phrasing() {
        assert_eq!(
            room_no_devices("kitchen"),
            "I couldn't find any lights in the kitchen."
        );
    }

    #[test]
    fn room_warming_up_phrasing() {
        assert_eq!(room_warming_up(), "Still waking up, try again in a moment.");
    }

    #[test]
    fn fallback_phrasing() {
        assert_eq!(fallback(), "I'm not sure how to help with that.");
    }

    #[test]
    fn join_spoken_empty() {
        assert_eq!(join_spoken(&[]), "");
    }

    #[test]
    fn join_spoken_one() {
        assert_eq!(join_spoken(&["a".into()]), "a");
    }

    #[test]
    fn join_spoken_two() {
        assert_eq!(join_spoken(&["a".into(), "b".into()]), "a and b");
    }

    #[test]
    fn join_spoken_three() {
        assert_eq!(
            join_spoken(&["a".into(), "b".into(), "c".into()]),
            "a, b, and c"
        );
    }

    #[test]
    fn format_duration_phrase_hours() {
        assert_eq!(
            format_duration_phrase(Duration::from_secs(7200)),
            (2, "hour")
        );
    }

    #[test]
    fn format_duration_phrase_minutes() {
        assert_eq!(
            format_duration_phrase(Duration::from_secs(300)),
            (5, "minute")
        );
    }

    #[test]
    fn format_duration_phrase_seconds() {
        assert_eq!(
            format_duration_phrase(Duration::from_secs(30)),
            (30, "second")
        );
    }

    #[test]
    fn format_duration_phrase_singular_hour() {
        assert_eq!(
            format_duration_phrase(Duration::from_secs(3600)),
            (1, "hour")
        );
    }

    #[test]
    fn format_duration_phrase_singular_minute() {
        assert_eq!(
            format_duration_phrase(Duration::from_secs(60)),
            (1, "minute")
        );
    }

    #[test]
    fn format_duration_phrase_singular_second() {
        assert_eq!(
            format_duration_phrase(Duration::from_secs(1)),
            (1, "second")
        );
    }

    #[test]
    fn media_play_phrasing() {
        assert_eq!(media_play("living_room"), "Playing in the living room.");
    }

    #[test]
    fn media_pause_phrasing() {
        assert_eq!(media_pause("kitchen"), "Paused in the kitchen.");
    }

    #[test]
    fn media_volume_phrasing() {
        assert_eq!(media_volume("kitchen", 30), "Kitchen volume to 30%.");
    }

    #[test]
    fn no_speaker_in_room_phrasing() {
        assert_eq!(no_speaker_in_room("office"), "No speaker in the office.");
    }

    #[test]
    fn speaker_unreachable_phrasing() {
        assert_eq!(
            speaker_unreachable("kitchen"),
            "I couldn't reach the speaker in the kitchen."
        );
    }

    #[test]
    fn device_set_on() {
        let id = DeviceId::new(
            "z2m",
            niles_core::RoomName::parse("living_room").unwrap(),
            niles_core::DeviceName::parse("floor_lamp").unwrap(),
        )
        .unwrap();
        assert_eq!(device_set(&id, true), "Living room floor lamp on.");
    }

    #[test]
    fn device_set_off() {
        let id = DeviceId::new(
            "z2m",
            niles_core::RoomName::parse("bedroom").unwrap(),
            niles_core::DeviceName::parse("floor_lamp").unwrap(),
        )
        .unwrap();
        assert_eq!(device_set(&id, false), "Bedroom floor lamp off.");
    }

    #[test]
    fn device_dim_basic() {
        let id = DeviceId::new(
            "z2m",
            niles_core::RoomName::parse("living_room").unwrap(),
            niles_core::DeviceName::parse("floor_lamp").unwrap(),
        )
        .unwrap();
        assert_eq!(device_dim(&id, 30), "Living room floor lamp to 30%.");
    }

    #[test]
    fn device_not_found_phrasing() {
        let id = DeviceId::new(
            "z2m",
            niles_core::RoomName::parse("kitchen").unwrap(),
            niles_core::DeviceName::parse("missing").unwrap(),
        )
        .unwrap();
        assert_eq!(
            device_not_found(&id),
            "I couldn't find that device anymore."
        );
    }
}
