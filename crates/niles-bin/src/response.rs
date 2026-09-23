//! Spoken-response phrasing. Pure: (intent outcome) -> what niles
//! says. Kept separate from dispatch so the phrasings are
//! unit-testable without a DispatchCtx, and so the TTS layer has
//! one string to synthesize.

use niles_core::DeviceId;
use std::time::Duration;

/// Convert a canonical room name (`living_room`) to spoken form
/// (`living room`).
pub(crate) fn spoken_room(room: &str) -> String {
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

/// "Kitchen lights to 3000K."
pub fn light_kelvin_step(room: &str, kelvin: u16) -> String {
    format!(
        "{} lights to {}K.",
        capitalize_first(&spoken_room(room)),
        kelvin
    )
}

fn kelvin_to_named_label(kelvin: u16) -> Option<&'static str> {
    match kelvin {
        2200 => Some("warm white"),
        4000 => Some("cool white"),
        5500 => Some("daylight"),
        _ => None,
    }
}

/// "Living room lights warm white." / "Kitchen lights to 3000K." (fallback)
pub fn light_kelvin_set(room: &str, kelvin: u16) -> String {
    match kelvin_to_named_label(kelvin) {
        Some(label) => {
            format!("{} lights {}.", capitalize_first(&spoken_room(room)), label)
        }
        None => light_kelvin_step(room, kelvin),
    }
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

/// Outcome of a generic "stop"/"cancel" that found nothing ringing:
/// cancelled a counting-down timer, or found nothing at all. A ringing
/// one is answered by [`timer_stopped`].
pub enum StopOutcome {
    CancelledPending,
    Nothing,
}

/// "Okay, cancelled the timer." / "Nothing's running."
pub fn stop_outcome(outcome: StopOutcome) -> String {
    match outcome {
        StopOutcome::CancelledPending => "Okay, cancelled the timer.".into(),
        StopOutcome::Nothing => "Nothing's running.".into(),
    }
}

/// What an alarm that was ringing turned out to be, now that it isn't.
///
/// "Stopped." answered the question nobody asked. With two timers going
/// the one that matters is which of them just went quiet.
///
/// "Your 10 minute timer has been stopped." /
/// "Your 10 minute pasta timer has been stopped."
pub fn timer_stopped(name: Option<&str>, duration: Duration) -> String {
    let (n, unit) = format_duration_phrase(duration);
    match name.map(|n| n.replace('_', " ")) {
        Some(name) => format!("Your {n} {unit} {name} timer has been stopped."),
        None => format!("Your {n} {unit} timer has been stopped."),
    }
}

/// Spoken time-remaining readout for the soonest pending timer.
/// `None` = no timers; `Some(0)` = already firing.
/// "7 minutes left." / "1 minute and 30 seconds left." /
/// "The timer's going off now." / "No timers running."
pub fn timer_remaining(seconds_left: Option<u64>) -> String {
    let Some(total) = seconds_left else {
        return "No timers running.".into();
    };
    if total == 0 {
        return "The timer's going off now.".into();
    }
    let mut parts: Vec<String> = Vec::new();
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        parts.push(plural_unit(h, "hour"));
    }
    if m > 0 {
        parts.push(plural_unit(m, "minute"));
    }
    if s > 0 {
        parts.push(plural_unit(s, "second"));
    }
    format!("{} left.", join_spoken(&parts))
}

fn plural_unit(n: u64, unit: &str) -> String {
    if n == 1 {
        format!("1 {unit}")
    } else {
        format!("{n} {unit}s")
    }
}

/// "I couldn't find a room called office."
pub fn room_not_found(room: &str) -> String {
    format!("I couldn't find a room called {}.", spoken_room(room))
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

/// "Next track in the living room."
pub fn media_next(room: &str) -> String {
    format!("Next track in the {}.", spoken_room(room))
}

/// "Previous track in the kitchen."
pub fn media_previous(room: &str) -> String {
    format!("Previous track in the {}.", spoken_room(room))
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

// ---- speaker enrollment ----------------------------------------------

/// Said after learning a voice. The clip count is the point: one
/// sample is a thin voice print, and the fix is to say it again — so
/// the reply asks for that rather than leaving recognition quietly
/// unreliable.
pub fn enrolled(name: &str, clips: usize) -> String {
    let name = capitalize_first(name);
    match clips {
        0 | 1 => {
            format!("Nice to meet you, {name}. Say it once or twice more and I'll know you better.")
        }
        2 => format!("Got it, {name}. One more would help."),
        _ => format!("I'll know you now, {name}."),
    }
}

/// Someone else is already enrolled under this name and this voice
/// isn't theirs.
pub fn enrollment_name_taken(name: &str) -> String {
    format!(
        "I already know a {}, and you don't sound like them.",
        capitalize_first(name)
    )
}

/// The answer to "who am I", in each of the four states recognition
/// can be in.
///
/// Four rather than two because they want four different things done
/// about them, and a house that says the same thing to all of them
/// tells you nothing: recognition off is a setting, nobody enrolled is
/// an introduction, and a voice that does not match is the one case
/// worth investigating.
pub fn who_you_are(speaker: Option<&str>, recognition_on: bool, anyone_enrolled: bool) -> String {
    match (speaker, recognition_on, anyone_enrolled) {
        (Some(name), _, _) => format!("You're {}.", capitalize_first(name)),
        (None, false, _) => "I'm not set up to recognise voices yet.".into(),
        (None, true, false) => {
            "I don't know yet. Say \"my name is\" and your name, a few times.".into()
        }
        // Says why, and what to do about it. "Who am I" is about a
        // second of audio and a second is under what a voice print
        // needs, so the commonest reason for landing here is not being
        // a stranger — it is having asked too briefly. A bare "I don't
        // recognise you" sends an enrolled person off to re-enrol a
        // voice that was already fine.
        (None, true, true) => {
            "I didn't catch enough of your voice. Ask again with a bit more?".into()
        }
    }
}

pub fn enrollment_unavailable() -> String {
    "I'm not set up to recognise voices yet.".to_string()
}

/// Recognition is on but this utterance produced no usable voice print
/// — usually too short, which "I am Mark" can be.
pub fn enrollment_no_audio() -> String {
    "I didn't get a clear enough sample. Try saying it again, a little slower.".to_string()
}

pub fn enrollment_failed() -> String {
    "I couldn't save your voice just now.".to_string()
}

/// "Office lights back on." — says what it acted on, because "it" was
/// resolved from memory and the user deserves to hear whether we
/// resolved it the way they meant.
pub fn light_set_last(spoken_room: &str, on: bool) -> String {
    format!(
        "{} lights {}.",
        capitalize_first(spoken_room),
        if on { "back on" } else { "off" }
    )
}

/// Said when what was heard was too thin to act on.
///
/// Deliberately an invitation rather than an apology: nine times in
/// ten nobody was talking to Niles at all, and a room that says "sorry,
/// I didn't understand" to the television is worse than one that says
/// nothing much.
pub fn didnt_catch_that() -> String {
    "Sorry?".to_string()
}

// ---- the clock ------------------------------------------------------

/// "It's 15:20." / "It's Saturday the 12th of September."
///
/// Spoken, so the date drops the year: nobody asks what day it is and
/// wants to be told which year they are in.
pub fn datetime_now(timezone: &str, date: bool) -> String {
    let Ok(tz) = timezone.parse::<chrono_tz::Tz>() else {
        // Tier 0 cannot answer without a clock it trusts, and guessing
        // the time is worse than admitting it.
        return "I'm not sure what time it is — my timezone isn't set.".to_string();
    };
    let now = chrono::Utc::now().with_timezone(&tz);
    if date {
        format!(
            "It's {}, the {} of {}.",
            now.format("%A"),
            ordinal(now.format("%-d").to_string().parse().unwrap_or(1)),
            now.format("%B"),
        )
    } else {
        format!("It's {}.", now.format("%-H:%M"))
    }
}

/// 1 -> "1st", 12 -> "12th", 23 -> "23rd". Said aloud, so it has to be
/// the spoken form rather than the numeral.
fn ordinal(day: u32) -> String {
    let suffix = match (day % 10, day % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{day}{suffix}")
}

// ---- Weather ----------------------------------------------------------------
//
// A sentence, not a report. Asked "what's the weather", people want to
// know what to wear: how warm, and whether it rains. Wind, humidity and
// sunrise are all in the forecast and none of them are the answer.

/// A whole number of degrees, as Piper should say it.
///
/// "-3" is read out as "dash three" often enough to spell it.
fn degrees(value: f64) -> String {
    let n = value.round() as i64;
    if n < 0 {
        format!("minus {}", -n)
    } else {
        n.to_string()
    }
}

/// One day of a forecast, reduced to what gets said.
pub struct DaySummary<'a> {
    pub description: &'a str,
    pub low: f64,
    pub high: f64,
    /// How much falls, in the forecast's own unit.
    pub precipitation: f64,
    /// Whether that much is worth mentioning, which depends on the unit.
    pub wet: bool,
    /// Whether what falls is snow.
    pub snow: bool,
}

impl DaySummary<'_> {
    fn falls(&self) -> &'static str {
        if self.snow { "snow" } else { "rain" }
    }
}

/// "12 degrees and overcast now, up to 14 today. Expect rain."
///
/// The high is left out once it has passed: in the evening "up to 14"
/// is a number that already happened.
pub fn weather_now(temperature: f64, description: &str, today: &DaySummary) -> String {
    let mut out = format!("{} degrees and {} now", degrees(temperature), description);
    if today.high.round() > temperature.round() {
        out.push_str(&format!(", up to {} today", degrees(today.high)));
    }
    out.push('.');
    if today.wet {
        out.push_str(&format!(" Expect {}.", today.falls()));
    }
    out
}

/// "Tomorrow: slight rain, 9 to 14 degrees."
pub fn weather_on(day: &str, forecast: &DaySummary) -> String {
    format!(
        "{}: {}, {} to {} degrees.",
        capitalize_first(day),
        forecast.description,
        degrees(forecast.low),
        degrees(forecast.high)
    )
}

/// "Yes, about 4 millimetres of rain tomorrow." / "No rain expected today."
pub fn rain_on(day: &str, forecast: &DaySummary, metric: bool) -> String {
    if !forecast.wet {
        return format!("No {} expected {day}.", forecast.falls());
    }
    let amount = if metric {
        let mm = forecast.precipitation.round().max(1.0) as i64;
        format!(
            "{mm} {}",
            if mm == 1 { "millimetre" } else { "millimetres" }
        )
    } else {
        format!("{:.1} inches", forecast.precipitation.max(0.1))
    };
    format!("Yes, about {amount} of {} {day}.", forecast.falls())
}

pub fn weather_unavailable() -> String {
    "I couldn't get the forecast.".into()
}

// ---- Heating ----------------------------------------------------------------

/// Tenths of a degree, as said: 210 is "21", 205 is "20.5".
pub fn tenths_spoken(tenths: u16) -> String {
    if tenths.is_multiple_of(10) {
        (tenths / 10).to_string()
    } else {
        format!("{}.{}", tenths / 10, tenths % 10)
    }
}

/// A reading to the nearest tenth, as said.
fn celsius_spoken(value: f32) -> String {
    tenths_spoken((value * 10.0).round().max(0.0) as u16)
}

/// Where a heating command landed: one room, or all of them.
pub enum Heated<'a> {
    Room(&'a str),
    Everywhere,
}

/// "Living room set to 21 degrees." / "Every room set to 21 degrees."
pub fn heating_set(place: Heated, tenths: u16) -> String {
    match place {
        Heated::Room(room) => format!(
            "{} set to {} degrees.",
            capitalize_first(&spoken_room(room)),
            tenths_spoken(tenths)
        ),
        Heated::Everywhere => format!("Every room set to {} degrees.", tenths_spoken(tenths)),
    }
}

/// "Every room turned up a degree."
pub fn heating_stepped_everywhere(up: bool) -> String {
    format!(
        "Every room turned {} a degree.",
        if up { "up" } else { "down" }
    )
}

/// "It's 20.5 degrees in the living room, heating to 21."
///
/// The target only when it is being worked towards: "heating to 19" in
/// a room at 21 describes a radiator that is off.
pub fn heating_reading(room: &str, temperature: Option<f32>, target: Option<f32>) -> String {
    let room = spoken_room(room);
    let Some(now) = temperature else {
        return format!("The {room} thermostat isn't answering.");
    };
    match target {
        Some(target) if target > now + 0.2 => format!(
            "It's {} degrees in the {room}, heating to {}.",
            celsius_spoken(now),
            celsius_spoken(target)
        ),
        _ => format!("It's {} degrees in the {room}.", celsius_spoken(now)),
    }
}

/// "Living room 21, bedroom 19 and office 20 degrees."
pub fn heating_readings(readings: &[(String, Option<f32>)]) -> String {
    let said: Vec<String> = readings
        .iter()
        .filter_map(|(room, t)| t.map(|t| format!("{} {}", spoken_room(room), celsius_spoken(t))))
        .collect();
    if said.is_empty() {
        return "None of the thermostats are answering.".into();
    }
    format!("{} degrees.", capitalize_first(&join_spoken(&said)))
}

/// "Heating off in the living room." / "Heating off everywhere."
pub fn heating_off(place: Heated) -> String {
    match place {
        Heated::Room(room) => format!("Heating off in the {}.", spoken_room(room)),
        Heated::Everywhere => "Heating off everywhere.".into(),
    }
}

/// "The living room is back on its schedule."
pub fn heating_resumed(place: Heated) -> String {
    match place {
        Heated::Room(room) => format!("The {} is back on its schedule.", spoken_room(room)),
        Heated::Everywhere => "Every room is back on its schedule.".into(),
    }
}

/// "There's no heating I know of in the office."
pub fn heating_no_zone(room: &str) -> String {
    format!("There's no heating I know of in the {}.", spoken_room(room))
}

pub fn heating_which_room() -> String {
    "Which room? I don't know where this speaker is.".into()
}

pub fn heating_out_of_range() -> String {
    "The heating goes from 5 to 25 degrees.".into()
}

pub fn heating_unavailable() -> String {
    "Niles isn't connected to tado.".into()
}

pub fn heating_failed() -> String {
    "tado didn't answer.".into()
}

#[cfg(test)]
mod tests {
    #[test]
    fn who_you_are_names_a_recognised_speaker() {
        assert_eq!(who_you_are(Some("mark"), true, true), "You're Mark.");
    }

    #[test]
    fn who_you_are_tells_the_four_states_apart() {
        // Four rather than two because they want four different things
        // done about them. A house that answers all of them the same
        // way tells you nothing.
        let off = who_you_are(None, false, false);
        let nobody = who_you_are(None, true, false);
        let stranger = who_you_are(None, true, true);
        assert!(off.contains("not set up"), "{off}");
        assert!(nobody.contains("my name is"), "{nobody}");
        assert!(stranger.contains("didn't catch enough"), "{stranger}");
        assert_ne!(nobody, stranger);
    }

    #[test]
    fn a_known_speaker_is_named_whatever_else_is_true() {
        // Recognition cannot be off while it has just recognised
        // somebody, but the reply should not depend on the caller
        // getting those two flags consistent.
        assert_eq!(who_you_are(Some("majse"), false, false), "You're Majse.");
    }

    #[test]
    fn the_time_is_spoken_not_printed() {
        let said = datetime_now("Europe/Copenhagen", false);
        assert!(said.starts_with("It's "), "{said}");
        assert!(said.ends_with('.'), "{said}");
    }

    #[test]
    fn the_date_leaves_out_the_year() {
        // Nobody asks what day it is and wants to be told which year
        // they are in.
        let said = datetime_now("Europe/Copenhagen", true);
        assert!(!said.contains("202"), "{said}");
        assert!(said.contains("the "), "{said}");
    }

    #[test]
    fn ordinals_read_the_way_they_are_said() {
        assert_eq!(ordinal(1), "1st");
        assert_eq!(ordinal(2), "2nd");
        assert_eq!(ordinal(3), "3rd");
        assert_eq!(ordinal(4), "4th");
        assert_eq!(ordinal(11), "11th");
        assert_eq!(ordinal(12), "12th");
        assert_eq!(ordinal(13), "13th");
        assert_eq!(ordinal(21), "21st");
        assert_eq!(ordinal(22), "22nd");
        assert_eq!(ordinal(23), "23rd");
    }

    #[test]
    fn a_broken_timezone_admits_it_rather_than_guessing() {
        let said = datetime_now("Not/AZone", false);
        assert!(said.contains("not sure"), "{said}");
    }

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
    fn light_kelvin_step_basic() {
        assert_eq!(
            light_kelvin_step("living_room", 2800),
            "Living room lights to 2800K."
        );
    }

    #[test]
    fn light_kelvin_set_warm_white() {
        assert_eq!(
            light_kelvin_set("living_room", 2200),
            "Living room lights warm white."
        );
    }

    #[test]
    fn light_kelvin_set_cool_white() {
        assert_eq!(
            light_kelvin_set("kitchen", 4000),
            "Kitchen lights cool white."
        );
    }

    #[test]
    fn light_kelvin_set_daylight() {
        assert_eq!(
            light_kelvin_set("bedroom", 5500),
            "Bedroom lights daylight."
        );
    }

    #[test]
    fn light_kelvin_set_fallback_numeric() {
        assert_eq!(
            light_kelvin_set("kitchen", 3000),
            "Kitchen lights to 3000K."
        );
    }

    #[test]
    fn light_kelvin_step_upper() {
        assert_eq!(
            light_kelvin_step("kitchen", 6500),
            "Kitchen lights to 6500K."
        );
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
    fn a_stopped_timer_says_how_long_it_was() {
        assert_eq!(
            timer_stopped(None, Duration::from_secs(600)),
            "Your 10 minute timer has been stopped."
        );
    }

    #[test]
    fn a_stopped_timer_says_its_name() {
        assert_eq!(
            timer_stopped(Some("pasta"), Duration::from_secs(600)),
            "Your 10 minute pasta timer has been stopped."
        );
        assert_eq!(
            timer_stopped(Some("pasta_sauce"), Duration::from_secs(3600)),
            "Your 1 hour pasta sauce timer has been stopped."
        );
    }

    fn dry(description: &str, low: f64, high: f64) -> DaySummary<'_> {
        DaySummary {
            description,
            low,
            high,
            precipitation: 0.0,
            wet: false,
            snow: false,
        }
    }

    #[test]
    fn the_weather_now_is_one_sentence() {
        assert_eq!(
            weather_now(12.3, "overcast", &dry("overcast", 8.0, 14.4)),
            "12 degrees and overcast now, up to 14 today."
        );
    }

    #[test]
    fn a_wet_day_says_so() {
        let day = DaySummary {
            precipitation: 4.2,
            wet: true,
            ..dry("slight rain", 8.0, 14.0)
        };
        assert_eq!(
            weather_now(12.0, "slight rain", &day),
            "12 degrees and slight rain now, up to 14 today. Expect rain."
        );
        assert_eq!(
            rain_on("today", &day, true),
            "Yes, about 4 millimetres of rain today."
        );
    }

    #[test]
    fn the_high_is_dropped_once_it_has_passed() {
        assert_eq!(
            weather_now(14.2, "clear sky", &dry("clear sky", 8.0, 14.0)),
            "14 degrees and clear sky now."
        );
    }

    #[test]
    fn tomorrow_is_a_range() {
        assert_eq!(
            weather_on("tomorrow", &dry("slight rain", 9.4, 13.6)),
            "Tomorrow: slight rain, 9 to 14 degrees."
        );
    }

    #[test]
    fn below_zero_is_spelled() {
        assert_eq!(
            weather_on("tomorrow", &dry("fog", -3.2, 1.0)),
            "Tomorrow: fog, minus 3 to 1 degrees."
        );
    }

    #[test]
    fn no_rain_is_a_short_no() {
        assert_eq!(
            rain_on("tomorrow", &dry("clear sky", 5.0, 9.0), true),
            "No rain expected tomorrow."
        );
    }

    #[test]
    fn snow_is_called_snow() {
        let day = DaySummary {
            precipitation: 0.3,
            wet: true,
            snow: true,
            ..dry("slight snow fall", -2.0, 1.0)
        };
        assert_eq!(
            rain_on("tomorrow", &day, false),
            "Yes, about 0.3 inches of snow tomorrow."
        );
    }

    #[test]
    fn heating_replies() {
        assert_eq!(
            heating_set(Heated::Room("living_room"), 210),
            "Living room set to 21 degrees."
        );
        assert_eq!(
            heating_set(Heated::Everywhere, 185),
            "Every room set to 18.5 degrees."
        );
        assert_eq!(
            heating_off(Heated::Room("bedroom")),
            "Heating off in the bedroom."
        );
        assert_eq!(
            heating_resumed(Heated::Everywhere),
            "Every room is back on its schedule."
        );
    }

    #[test]
    fn a_reading_mentions_the_target_only_while_heating_to_it() {
        assert_eq!(
            heating_reading("living_room", Some(20.46), Some(21.0)),
            "It's 20.5 degrees in the living room, heating to 21."
        );
        assert_eq!(
            heating_reading("bedroom", Some(21.0), Some(19.0)),
            "It's 21 degrees in the bedroom."
        );
        assert_eq!(
            heating_reading("office", None, Some(20.0)),
            "The office thermostat isn't answering."
        );
    }

    #[test]
    fn every_room_at_once() {
        assert_eq!(
            heating_readings(&[
                ("living_room".into(), Some(21.0)),
                ("bedroom".into(), Some(19.0)),
                ("office".into(), None),
            ]),
            "Living room 21 and bedroom 19 degrees."
        );
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
    fn room_not_found_phrasing() {
        assert_eq!(
            room_not_found("office"),
            "I couldn't find a room called office."
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
    fn timer_remaining_phrasings() {
        assert_eq!(timer_remaining(None), "No timers running.");
        assert_eq!(timer_remaining(Some(0)), "The timer's going off now.");
        assert_eq!(timer_remaining(Some(7)), "7 seconds left.");
        assert_eq!(timer_remaining(Some(1)), "1 second left.");
        assert_eq!(timer_remaining(Some(300)), "5 minutes left.");
        assert_eq!(timer_remaining(Some(90)), "1 minute and 30 seconds left.");
        assert_eq!(
            timer_remaining(Some(3661)),
            "1 hour, 1 minute, and 1 second left."
        );
    }

    #[test]
    fn stop_outcome_phrasings() {
        assert_eq!(
            stop_outcome(StopOutcome::CancelledPending),
            "Okay, cancelled the timer."
        );
        assert_eq!(stop_outcome(StopOutcome::Nothing), "Nothing's running.");
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
    fn media_next_response() {
        assert_eq!(media_next("kitchen"), "Next track in the kitchen.");
    }

    #[test]
    fn media_previous_response() {
        assert_eq!(media_previous("kitchen"), "Previous track in the kitchen.");
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
