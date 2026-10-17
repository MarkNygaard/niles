//! A rule said in English: "When the office door opens, turn off the
//! office lights." What Niles reads back before saving one, what it says
//! when asked which automations there are, and what the app lists.

use crate::rule::say_duration;
use niles_config::{
    ActionDto, AutomationRuleDto, BlindsPositionDto, ConditionDto, DurationDto, TriggerDto,
    TriggerKind,
};
use std::time::Duration;

/// The whole rule as one sentence.
pub fn sentence(rule: &AutomationRuleDto) -> String {
    let triggers: Vec<String> = rule.triggers.iter().map(trigger).collect();
    // Read back while it is still being worked out: "When , if…" was
    // what a rule with no trigger sounded like.
    let mut out = if triggers.is_empty() {
        "With nothing yet to start it".to_string()
    } else {
        format!("When {}", triggers.join(", or when "))
    };
    let names = trigger_names(&rule.triggers);
    let conditions: Vec<String> = rule
        .conditions
        .iter()
        .map(|c| condition(c, &names))
        .collect();
    if !conditions.is_empty() {
        out.push_str(", if ");
        out.push_str(&conditions.join(" and "));
    }
    let actions: Vec<String> = rule.actions.iter().map(|a| action(a, &names)).collect();
    if !actions.is_empty() {
        out.push_str(", ");
        out.push_str(&actions.join(", then "));
    }
    out.push('.');
    out
}

/// "office/door" → "the office door"; "living_room/living_room_lamp" →
/// "the living room lamp", not "the living room living room lamp".
pub fn thing(device: &str) -> String {
    let bare = device.split_once(':').map_or(device, |(_, rest)| rest);
    let Some((room, name)) = bare.split_once('/') else {
        return format!("the {}", words(bare));
    };
    let (room, name) = (words(room), words(name));
    if name.starts_with(&room) {
        format!("the {name}")
    } else {
        format!("the {room} {name}")
    }
}

fn words(slug: &str) -> String {
    slug.replace('_', " ")
}

/// The one sensor a motion trigger or condition names, when it names
/// one: in `device`, or as a list of one.
pub(crate) fn one_sensor<'a>(
    device: &'a Option<String>,
    devices: &'a [String],
    rooms: &[String],
) -> Option<&'a str> {
    match (device, devices, rooms) {
        (Some(device), _, _) => Some(device),
        (None, [one], []) => Some(one),
        _ => None,
    }
}

/// Where a group of motion sensors looks: "the hallway", "the kitchen".
/// A sensor named for what it is — `hallway/motion`, `kitchen/pir` —
/// stands for its room, which is how people say it; any other is said
/// by name. A room comes up once however many sensors it has.
fn places(devices: &[String], rooms: &[String]) -> Vec<String> {
    const SENSOR: &[&str] = &["motion", "presence", "occupancy", "pir", "radar"];
    let mut out: Vec<String> = Vec::new();
    let mut add = |place: String| {
        if !out.contains(&place) {
            out.push(place);
        }
    };
    for device in devices {
        let bare = device
            .split_once(':')
            .map_or(device.as_str(), |(_, rest)| rest);
        match bare.split_once('/') {
            Some((room, name)) if name == "sensor" || SENSOR.iter().any(|s| name.contains(s)) => {
                add(format!("the {}", words(room)))
            }
            _ => add(thing(device)),
        }
    }
    for room in rooms {
        add(format!("the {}", words(room)));
    }
    out
}

/// "a", "a or b", "a, b or c".
fn or_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} or {last}", rest.join(", ")),
    }
}

/// The whole rule, a line for each part: the triggers numbered, the
/// conditions, then the actions in order, with what an `if` does either
/// way indented under it.
///
/// Made from the rule alone, so what is read back for a yes is what
/// will run — not the model's paraphrase of what it meant to write.
pub fn describe(rule: &AutomationRuleDto) -> String {
    let names = trigger_names(&rule.triggers);
    let mut lines = vec!["When:".to_string()];
    if rule.triggers.is_empty() {
        lines.push("  nothing yet".into());
    }
    for (i, t) in rule.triggers.iter().enumerate() {
        lines.push(format!("  {}. {}", i + 1, upper_first(&trigger(t))));
    }
    if !rule.conditions.is_empty() {
        lines.push("Only if:".into());
        for c in &rule.conditions {
            lines.push(format!("  - {}", upper_first(&condition(c, &names))));
        }
    }
    lines.push("Do:".into());
    steps(&rule.actions, 1, &names, &mut lines);
    lines.join("\n")
}

fn steps(actions: &[ActionDto], depth: usize, names: &[(String, String)], lines: &mut Vec<String>) {
    let indent = "  ".repeat(depth);
    for a in actions {
        let ActionDto::If {
            conditions,
            then,
            otherwise,
        } = a
        else {
            lines.push(format!("{indent}{}", upper_first(&action(a, names))));
            continue;
        };
        if conditions.is_empty() {
            lines.push(format!("{indent}Always:"));
        } else {
            let when: Vec<String> = conditions.iter().map(|c| condition(c, names)).collect();
            lines.push(format!("{indent}If {}:", when.join(" and ")));
        }
        steps(then, depth + 1, names, lines);
        if !otherwise.is_empty() {
            lines.push(format!("{indent}Otherwise:"));
            steps(otherwise, depth + 1, names, lines);
        }
    }
}

fn upper_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Whether an action is a press, hold or turn of a button, in
/// Zigbee2MQTT's vocabulary. A rule sentence has no device class to go
/// on, and actions come from any device now: "the printer is pressed
/// (finished)" is what treating every action as a press sounded like.
fn is_press(action: &str) -> bool {
    const PRESSES: &[&str] = &[
        "single",
        "double",
        "triple",
        "quadruple",
        "many",
        "hold",
        "release",
        "long",
        "toggle",
        "on",
        "off",
        "shake",
        "tap",
        "slide",
        "flip90",
        "flip180",
    ];
    const PARTS: &[&str] = &["press", "hold", "release", "click", "rotate", "brightness"];
    let action = action.to_ascii_lowercase();
    PRESSES.contains(&action.as_str()) || PARTS.iter().any(|p| action.contains(p))
}

/// Whether a device's name says it is something to press, for a rule
/// that takes any of its actions.
fn looks_like_a_button(device: &str) -> bool {
    const NAMES: &[&str] = &["switch", "button", "remote", "dimmer", "knob", "cube"];
    let name = device.rsplit('/').next().unwrap_or(device);
    NAMES.iter().any(|n| name.contains(n))
}

/// Each trigger's id and how it is said: by its place, as the app
/// shows it — the id is the app's and the model's, not a person's.
pub(crate) fn trigger_names(triggers: &[TriggerDto]) -> Vec<(String, String)> {
    triggers
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let id = t.id.clone().unwrap_or_else(|| format!("t{}", i + 1));
            (id, format!("trigger {}", i + 1))
        })
        .collect()
}

fn held(hold: &Option<DurationDto>) -> Option<String> {
    hold.as_ref()
        .map(|d| say_duration(Duration::from_secs(d.total_seconds())))
}

fn number(n: f64) -> String {
    if n.fract() == 0.0 {
        format!("{n:.0}")
    } else {
        format!("{n}")
    }
}

/// What a measure is called aloud. A reading is said by its name —
/// "the price of", "the soil moisture of" — with the few names that
/// are letters rather than words spelled the way they are written.
///
/// Without its unit: the sentence is made from the rule alone, and the
/// unit belongs to the device, which may not have reported yet. The
/// number is the one the person gave, in the unit they had in mind.
fn measure_words(measure: &str) -> String {
    match measure.trim().to_ascii_lowercase().as_str() {
        "illuminance" | "lux" | "light_level" => "light level".into(),
        "co2" => "CO2".into(),
        "voc" => "VOC".into(),
        "pm25" => "PM2.5".into(),
        "pm10" => "PM10".into(),
        other => words(other),
    }
}

fn bounds(above: Option<f64>, below: Option<f64>) -> String {
    match (above, below) {
        (Some(a), Some(b)) => format!("between {} and {}", number(a), number(b)),
        (Some(a), None) => format!("above {}", number(a)),
        (None, Some(b)) => format!("below {}", number(b)),
        (None, None) => "anything".into(),
    }
}

fn days(days: &Option<Vec<String>>) -> String {
    let Some(days) = days.as_ref().filter(|d| !d.is_empty()) else {
        return String::new();
    };
    let named: Vec<&str> = days
        .iter()
        .map(
            |d| match d.trim().to_ascii_lowercase().get(..3).unwrap_or("") {
                "mon" => "Monday",
                "tue" => "Tuesday",
                "wed" => "Wednesday",
                "thu" => "Thursday",
                "fri" => "Friday",
                "sat" => "Saturday",
                "sun" => "Sunday",
                _ => "?",
            },
        )
        .collect();
    let weekdays = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday"];
    if named.len() == 5 && weekdays.iter().all(|d| named.contains(d)) {
        return " on weekdays".into();
    }
    if named.len() == 2 && named.contains(&"Saturday") && named.contains(&"Sunday") {
        return " at weekends".into();
    }
    format!(" on {}", named.join(" and "))
}

fn sun_moment(event: &str, offset: i32) -> String {
    let event = event.trim().to_ascii_lowercase();
    match offset {
        0 => event,
        o if o < 0 => format!(
            "{} before {event}",
            say_duration(Duration::from_secs(u64::from(o.unsigned_abs()) * 60))
        ),
        o => format!(
            "{} after {event}",
            say_duration(Duration::from_secs(u64::from(o.unsigned_abs()) * 60))
        ),
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "one arm per trigger kind, each the sentence it reads as"
)]
pub(crate) fn trigger(t: &TriggerDto) -> String {
    match &t.kind {
        TriggerKind::Contact { device, open, hold } => match (open, held(hold)) {
            (Some(true), Some(h)) => format!("{} has been open for {h}", thing(device)),
            (Some(false), Some(h)) => format!("{} has been closed for {h}", thing(device)),
            (None, Some(h)) => format!("{} has stayed as it is for {h}", thing(device)),
            (Some(true), None) => format!("{} opens", thing(device)),
            (Some(false), None) => format!("{} closes", thing(device)),
            (None, None) => format!("{} opens or closes", thing(device)),
        },
        TriggerKind::Motion {
            device,
            devices,
            rooms,
            detected,
            hold,
        } => match one_sensor(device, devices, rooms) {
            Some(device) => match (detected, held(hold)) {
                (Some(true), Some(h)) => format!("{} has seen motion for {h}", thing(device)),
                (Some(false), Some(h)) => format!("{} has seen no motion for {h}", thing(device)),
                (None, Some(h)) => format!("{} has stayed as it is for {h}", thing(device)),
                (Some(true), None) => format!("{} sees motion", thing(device)),
                (Some(false), None) => format!("{} stops seeing motion", thing(device)),
                (None, None) => format!("{} sees motion or stops", thing(device)),
            },
            None => {
                let at = or_list(&places(devices, rooms));
                match (detected, held(hold)) {
                    (Some(true), Some(h)) => format!("there has been motion in {at} for {h}"),
                    (Some(false), Some(h)) => {
                        format!("there has been no motion in {at} for {h}")
                    }
                    (Some(true), None) => format!("there is motion in {at}"),
                    (Some(false), None) => format!("there is no motion left in {at}"),
                    (None, _) => format!("motion in {at} starts or stops"),
                }
            }
        },
        TriggerKind::DeviceState {
            device,
            room,
            on,
            hold,
        } => {
            let what = match (device, room) {
                (Some(device), _) => thing(device),
                (None, Some(room)) => format!("something in the {}", words(room)),
                (None, None) => "any device".to_string(),
            };
            match (on, held(hold)) {
                (Some(on), Some(h)) => {
                    format!("{what} has been {} for {h}", if *on { "on" } else { "off" })
                }
                (None, Some(h)) => format!("{what} has stayed as it is for {h}"),
                (Some(true), None) => format!("{what} turns on"),
                (Some(false), None) => format!("{what} turns off"),
                (None, None) => format!("{what} changes"),
            }
        }
        TriggerKind::Numeric {
            device,
            measure,
            above,
            below,
            hold,
        } => {
            let what = format!("the {} of {}", measure_words(measure), thing(device));
            match held(hold) {
                Some(h) => format!("{what} has been {} for {h}", bounds(*above, *below)),
                None => format!("{what} goes {}", bounds(*above, *below)),
            }
        }
        TriggerKind::DeviceAction { device, action } => match action {
            Some(action) if is_press(action) => {
                format!("{} is pressed ({})", thing(device), words(action))
            }
            Some(action) => format!("{} reports {}", thing(device), words(action)),
            None if looks_like_a_button(device) => format!("{} is pressed", thing(device)),
            None => format!("{} reports anything", thing(device)),
        },
        TriggerKind::TimerFired { name } => match name {
            Some(name) => format!("the {name} timer goes off"),
            None => "a timer goes off".to_string(),
        },
        TriggerKind::Presence { state, hold } => match (state.as_deref(), held(hold)) {
            (Some("home"), Some(h)) => format!("someone has been home for {h}"),
            (Some("away"), Some(h)) => format!("everyone has been away for {h}"),
            (Some("home"), None) => "someone comes home".to_string(),
            (Some("away"), None) => "everyone has left".to_string(),
            (Some(other), _) => format!("the house becomes {other}"),
            (None, _) => "someone comes or goes".to_string(),
        },
        TriggerKind::Time { at, days: d } => format!("it is {at}{}", days(d)),
        TriggerKind::Sun {
            event,
            offset_minutes,
        } => match offset_minutes.unwrap_or(0) {
            0 if event.eq_ignore_ascii_case("sunset") => "the sun sets".to_string(),
            0 => "the sun rises".to_string(),
            o => format!("it is {}", sun_moment(event, o)),
        },
    }
}

pub(crate) fn condition(condition: &ConditionDto, names: &[(String, String)]) -> String {
    match condition {
        ConditionDto::TimeOfDay {
            after,
            before,
            days: d,
        } => {
            let window = match (after, before) {
                (Some(after), Some(before)) => format!("it is between {after} and {before}"),
                (Some(after), None) => format!("it is after {after}"),
                (None, Some(before)) => format!("it is before {before}"),
                (None, None) => "it is".to_string(),
            };
            format!("{window}{}", days(d))
        }
        ConditionDto::Sun {
            after,
            before,
            after_offset_minutes,
            before_offset_minutes,
        } => {
            let at = |e: &String, o: &Option<i32>| sun_moment(e, o.unwrap_or(0));
            match (after, before) {
                (Some(a), Some(b))
                    if a.eq_ignore_ascii_case("sunset")
                        && b.eq_ignore_ascii_case("sunrise")
                        && after_offset_minutes.unwrap_or(0) == 0
                        && before_offset_minutes.unwrap_or(0) == 0 =>
                {
                    "it is dark".to_string()
                }
                (Some(a), Some(b)) => format!(
                    "it is after {} and before {}",
                    at(a, after_offset_minutes),
                    at(b, before_offset_minutes)
                ),
                (Some(a), None) => format!("it is after {}", at(a, after_offset_minutes)),
                (None, Some(b)) => format!("it is before {}", at(b, before_offset_minutes)),
                (None, None) => "the sun is anywhere".to_string(),
            }
        }
        ConditionDto::DeviceIs { device, on } => {
            format!("{} is {}", thing(device), if *on { "on" } else { "off" })
        }
        ConditionDto::ContactIs { device, open } => {
            format!(
                "{} is {}",
                thing(device),
                if *open { "open" } else { "closed" }
            )
        }
        ConditionDto::Numeric {
            device,
            measure,
            above,
            below,
        } => format!(
            "the {} of {} is {}",
            measure_words(measure),
            thing(device),
            bounds(*above, *below)
        ),
        ConditionDto::MotionIs {
            device,
            devices,
            rooms,
            detected,
        } => match (one_sensor(device, devices, rooms), detected) {
            (Some(device), true) => format!("{} sees motion", thing(device)),
            (Some(device), false) => format!("{} sees no motion", thing(device)),
            (None, true) => format!("there is motion in {}", or_list(&places(devices, rooms))),
            (None, false) => format!("there is no motion in {}", or_list(&places(devices, rooms))),
        },
        ConditionDto::PresenceIs { state } => match state.as_str() {
            "away" => "everyone is away".to_string(),
            _ => "someone is home".to_string(),
        },
        ConditionDto::TriggeredBy { triggers } => {
            let said: Vec<String> = triggers
                .iter()
                .map(|id| {
                    names
                        .iter()
                        .find(|(known, _)| known == id)
                        .map(|(_, name)| name.clone())
                        .unwrap_or_else(|| id.clone())
                })
                .collect();
            format!("it was started by {}", said.join(" or "))
        }
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "one arm per action kind, each the sentence it reads as"
)]
pub(crate) fn action(step: &ActionDto, names: &[(String, String)]) -> String {
    match step {
        ActionDto::SetDevice {
            device,
            on,
            brightness,
            kelvin,
        } => set(&thing(device), *on, *brightness, *kelvin),
        ActionDto::SetRoom {
            room,
            on,
            brightness,
            kelvin,
        } => set(
            &format!("the {} lights", words(room)),
            *on,
            *brightness,
            *kelvin,
        ),
        ActionDto::SetBlinds {
            device,
            room,
            position,
        } => {
            let what = match (device, room) {
                (Some(device), _) => thing(device),
                (None, Some(room)) => format!("the {} blinds", words(room)),
                (None, None) => "the blinds".into(),
            };
            match position {
                BlindsPositionDto::Open => format!("open {what}"),
                BlindsPositionDto::Exact(0) => format!("close {what}"),
                BlindsPositionDto::Exact(100) => format!("fully open {what}"),
                BlindsPositionDto::Exact(p) => format!("open {what} to {p}%"),
            }
        }
        ActionDto::Notify { body, room, .. } => match room {
            Some(room) => format!("say \"{body}\" in the {}", words(room)),
            None => format!("say \"{body}\""),
        },
        ActionDto::WakeUp {
            minutes,
            rooms,
            devices,
            except,
            brightness,
            start_kelvin,
            kelvin,
        } => {
            let mut lights: Vec<String> = rooms
                .iter()
                .map(|r| format!("the {}", words(r)))
                .chain(devices.iter().map(|d| thing(d)))
                .collect();
            if lights.is_empty() {
                lights.push("the house".into());
            }
            let mut out = format!(
                "wake {} up over {} minutes, to {}%",
                and_list(&lights),
                minutes.unwrap_or(crate::rule::WAKE_UP_MINUTES),
                brightness.unwrap_or(100)
            );
            match (start_kelvin, kelvin) {
                (Some(from), Some(to)) if from != to => {
                    out.push_str(&format!(", from {from} K to {to} K"))
                }
                (_, Some(k)) => out.push_str(&format!(" at {k} K")),
                _ => {}
            }
            if !except.is_empty() {
                let left: Vec<String> = except.iter().map(|d| thing(d)).collect();
                out.push_str(&format!(", but not {}", and_list(&left)));
            }
            out
        }
        ActionDto::Delay {
            hours,
            minutes,
            seconds,
        } => format!(
            "wait {}",
            say_duration(Duration::from_secs(
                DurationDto {
                    hours: *hours,
                    minutes: *minutes,
                    seconds: *seconds,
                }
                .total_seconds()
            ))
        ),
        ActionDto::If {
            conditions,
            then,
            otherwise,
        } => {
            let when: Vec<String> = conditions.iter().map(|c| condition(c, names)).collect();
            let then: Vec<String> = then.iter().map(|a| action(a, names)).collect();
            let mut out = format!("if {}, {}", when.join(" and "), then.join(" and "));
            if !otherwise.is_empty() {
                let otherwise: Vec<String> = otherwise.iter().map(|a| action(a, names)).collect();
                out.push_str(&format!(", otherwise {}", otherwise.join(" and ")));
            }
            out
        }
    }
}

/// "a", "a and b", "a, b and c".
fn and_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

fn set(what: &str, on: Option<bool>, brightness: Option<u8>, kelvin: Option<u16>) -> String {
    let mut how = Vec::new();
    if let Some(b) = brightness {
        how.push(format!("{b}%"));
    }
    if let Some(k) = kelvin {
        how.push(format!("{k} K"));
    }
    let how = if how.is_empty() {
        String::new()
    } else {
        format!(" at {}", how.join(" and "))
    };
    match on {
        Some(false) => format!("turn off {what}"),
        Some(true) => format!("turn on {what}{how}"),
        None if how.is_empty() => format!("leave {what} as it is"),
        None => format!("set {what}{how}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn said(value: serde_json::Value) -> String {
        sentence(&serde_json::from_value(value).unwrap())
    }

    #[test]
    fn says_the_garden_door_rule() {
        assert_eq!(
            said(json!({
                "id": "r",
                "triggers": [{ "type": "contact", "device": "office/door", "open": true }],
                "actions": [{ "do": "set_room", "room": "office", "on": false }],
            })),
            "When the office door opens, turn off the office lights."
        );
    }

    #[test]
    fn says_a_wake_up() {
        assert_eq!(
            said(json!({
                "id": "r",
                "triggers": [{ "type": "time", "at": "06:00" }],
                "actions": [
                    { "do": "delay", "hours": 1 },
                    { "do": "wake_up", "minutes": 30, "rooms": ["bedroom"], "except": ["bedroom/lamp"], "brightness": 80 }
                ],
            })),
            "When it is 06:00, wait 1 hour, then wake the bedroom up over 30 minutes, to 80%, but not the bedroom lamp."
        );
        assert!(
            said(json!({
                "id": "r",
                "triggers": [{ "type": "time", "at": "06:00" }],
                "actions": [{ "do": "wake_up" }],
            }))
            .contains("wake the house up over 45 minutes, to 100%")
        );
        let warming = |start: u16, end: u16| {
            said(json!({
                "id": "r",
                "triggers": [{ "type": "time", "at": "06:00" }],
                "actions": [{ "do": "wake_up", "start_kelvin": start, "kelvin": end }],
            }))
        };
        assert!(warming(2200, 4000).contains("to 100%, from 2200 K to 4000 K"));
        assert!(warming(3000, 3000).contains("to 100% at 3000 K"));
    }

    fn when_action(device: &str, action: Option<&str>) -> String {
        let mut trigger = json!({ "type": "device_action", "device": device });
        if let Some(action) = action {
            trigger["action"] = json!(action);
        }
        said(json!({
            "id": "r",
            "triggers": [trigger],
            "actions": [{ "do": "set_room", "room": "office", "on": false }],
        }))
    }

    #[test]
    fn a_button_is_pressed() {
        assert_eq!(
            when_action("office/switch", Some("single")),
            "When the office switch is pressed (single), turn off the office lights."
        );
        assert_eq!(
            when_action("hallway/remote", Some("up_hold")),
            "When the hallway remote is pressed (up hold), turn off the office lights."
        );
        assert_eq!(
            when_action("office/wall_switch", None),
            "When the office wall switch is pressed, turn off the office lights."
        );
    }

    #[test]
    fn any_other_device_reports_its_action() {
        assert_eq!(
            when_action("x_bambulab:office/printer", Some("finished")),
            "When the office printer reports finished, turn off the office lights."
        );
        assert_eq!(
            when_action("office/printer", Some("print_failed")),
            "When the office printer reports print failed, turn off the office lights."
        );
        assert_eq!(
            when_action("office/printer", None),
            "When the office printer reports anything, turn off the office lights."
        );
    }

    #[test]
    fn says_the_blinds_rule() {
        assert_eq!(
            said(json!({
                "id": "r",
                "triggers": [{ "type": "sun", "event": "sunset" }],
                "actions": [
                    { "do": "set_blinds", "room": "living_room", "position": 0 },
                    { "do": "set_blinds", "device": "office/blind", "position": 30 }
                ],
            })),
            "When the sun sets, close the living room blinds, then open the office blind to 30%."
        );
        assert_eq!(
            said(json!({
                "id": "r",
                "triggers": [{ "type": "sun", "event": "sunrise" }],
                "actions": [{ "do": "set_blinds", "room": "living_room", "position": "open" }],
            })),
            "When the sun rises, open the living room blinds."
        );
        assert_eq!(
            said(json!({
                "id": "r",
                "triggers": [{ "type": "sun", "event": "sunrise" }],
                "actions": [{ "do": "set_blinds", "room": "living_room", "position": 100 }],
            })),
            "When the sun rises, fully open the living room blinds."
        );
    }

    #[test]
    fn says_the_motion_rule() {
        assert_eq!(
            said(json!({
                "id": "r",
                "triggers": [
                    { "type": "motion", "device": "hallway/motion_sensor", "detected": true },
                    { "type": "motion", "device": "hallway/motion_sensor", "detected": false, "for": { "minutes": 2 } }
                ],
                "conditions": [{ "kind": "numeric", "device": "hallway/motion_sensor", "measure": "illuminance", "below": 30 }],
                "actions": [{ "do": "set_room", "room": "hallway", "on": true }],
            })),
            "When the hallway motion sensor sees motion, or when the hallway motion sensor has \
             seen no motion for 2 minutes, if the light level of the hallway motion sensor is below 30, \
             turn on the hallway lights."
        );
    }

    #[test]
    fn says_conditions_and_several_actions() {
        assert_eq!(
            said(json!({
                "id": "r",
                "triggers": [{ "type": "presence", "state": "home" }],
                "conditions": [
                    { "kind": "time_of_day", "after": "18:00", "before": "06:00" },
                    { "kind": "contact_is", "device": "z2m:office/door", "open": false }
                ],
                "actions": [
                    { "do": "set_device", "device": "hallway/ceiling_light", "on": true, "brightness": 40, "kelvin": 2700 },
                    { "do": "notify", "body": "Welcome home", "room": "living_room" }
                ],
            })),
            "When someone comes home, if it is between 18:00 and 06:00 and the office door is \
             closed, turn on the hallway ceiling light at 40% and 2700 K, then say \"Welcome \
             home\" in the living room."
        );
    }

    #[test]
    fn says_several_triggers_a_wait_and_a_branch() {
        assert_eq!(
            said(json!({
                "id": "r",
                "triggers": [
                    { "id": "closed", "type": "contact", "device": "office/door", "open": false },
                    { "id": "late", "type": "contact", "device": "office/door", "open": true, "for": { "minutes": 10 } }
                ],
                "conditions": [{ "kind": "sun", "after": "sunset", "before": "sunrise" }],
                "actions": [{
                    "do": "if",
                    "conditions": [{ "kind": "triggered_by", "triggers": ["closed"] }],
                    "then": [{ "do": "delay", "minutes": 5 }, { "do": "set_room", "room": "office", "on": false }],
                    "else": [{ "do": "notify", "body": "Shut the door" }]
                }],
            })),
            "When the office door closes, or when the office door has been open for 10 \
             minutes, if it is dark, if it was started by trigger 1, wait 5 minutes and turn off \
             the office lights, otherwise say \"Shut the door\"."
        );
    }

    #[test]
    fn says_the_clock_the_sun_and_readings() {
        assert_eq!(
            said(json!({
                "id": "r",
                "triggers": [
                    { "type": "time", "at": "22:00", "days": ["mon", "tue", "wed", "thu", "fri"] },
                    { "type": "sun", "event": "sunset", "offset_minutes": -30 },
                    { "type": "numeric", "device": "office/sensor", "measure": "temperature", "above": 24 }
                ],
                "actions": [{ "do": "notify", "body": "x" }],
            })),
            "When it is 22:00 on weekdays, or when it is 30 minutes before sunset, or when the \
             temperature of the office sensor goes above 24, say \"x\"."
        );
    }

    #[test]
    fn says_a_rule_that_follows_a_reading() {
        assert_eq!(
            said(json!({
                "id": "r",
                "triggers": [{
                    "type": "numeric", "device": "system/electricity", "measure": "price",
                    "below": 1, "for": { "minutes": 15 }
                }],
                "conditions": [
                    { "kind": "numeric", "device": "kitchen/dishwasher", "measure": "power", "below": 5 },
                    { "kind": "numeric", "device": "office/air", "measure": "co2", "above": 1000 },
                    { "kind": "numeric", "device": "garden/plant", "measure": "soil_moisture", "below": 20.5 }
                ],
                "actions": [{ "do": "set_device", "device": "kitchen/dishwasher", "on": true }],
            })),
            "When the price of the system electricity has been below 1 for 15 minutes, if the \
             power of the kitchen dishwasher is below 5 and the CO2 of the office air is above \
             1000 and the soil moisture of the garden plant is below 20.5, turn on the kitchen \
             dishwasher."
        );
    }

    /// The hallway and the kitchen share a light: each room's motion
    /// turns its lights on, and both go off once neither has seen
    /// anybody for two minutes.
    fn shared_lights() -> serde_json::Value {
        json!({
            "id": "r",
            "triggers": [
                { "id": "hall-seen", "type": "motion", "device": "hallway/motion", "detected": true },
                { "id": "kitchen-seen", "type": "motion", "device": "kitchen/motion", "detected": true },
                { "id": "all-clear", "type": "motion", "devices": ["hallway/motion", "kitchen/motion"], "detected": false, "for": { "minutes": 2 } }
            ],
            "actions": [
                { "do": "if", "conditions": [{ "kind": "triggered_by", "triggers": ["hall-seen"] }],
                  "then": [{ "do": "set_room", "room": "hallway", "on": true }] },
                { "do": "if", "conditions": [{ "kind": "triggered_by", "triggers": ["kitchen-seen"] }],
                  "then": [{ "do": "set_room", "room": "kitchen", "on": true }, { "do": "set_room", "room": "hallway", "on": true }] },
                { "do": "if", "conditions": [{ "kind": "triggered_by", "triggers": ["all-clear"] }],
                  "then": [{ "do": "set_room", "room": "kitchen", "on": false }, { "do": "set_room", "room": "hallway", "on": false }] }
            ],
        })
    }

    #[test]
    fn says_a_group_of_sensors_by_their_rooms() {
        let s = said(shared_lights());
        assert!(
            s.contains(
                "or when there has been no motion in the hallway or the kitchen for 2 minutes,"
            ),
            "{s}"
        );
        let when = |trigger: serde_json::Value| {
            said(
                json!({ "id": "r", "triggers": [trigger], "actions": [{ "do": "notify", "body": "x" }] }),
            )
        };
        assert_eq!(
            when(
                json!({ "type": "motion", "devices": ["hallway/motion", "office/radar", "hallway/pir"], "rooms": ["kitchen"], "detected": true })
            ),
            "When there is motion in the hallway, the office or the kitchen, say \"x\"."
        );
        assert_eq!(
            when(
                json!({ "type": "motion", "devices": ["hallway/motion", "office/desk_lamp"], "detected": false })
            ),
            "When there is no motion left in the hallway or the office desk lamp, say \"x\"."
        );
        assert_eq!(
            when(json!({ "type": "motion", "devices": ["hallway/motion"], "detected": true })),
            "When the hallway motion sees motion, say \"x\".",
            "a list of one is that sensor"
        );
        assert_eq!(
            said(json!({
                "id": "r",
                "triggers": [{ "type": "time", "at": "23:00" }],
                "conditions": [{ "kind": "motion_is", "rooms": ["hallway", "kitchen"], "detected": false }],
                "actions": [{ "do": "notify", "body": "x" }],
            })),
            "When it is 23:00, if there is no motion in the hallway or the kitchen, say \"x\"."
        );
    }

    #[test]
    fn describes_the_rule_a_line_at_a_time() {
        let described = describe(&serde_json::from_value(shared_lights()).unwrap());
        assert_eq!(
            described,
            "When:\n\
             \x20 1. The hallway motion sees motion\n\
             \x20 2. The kitchen motion sees motion\n\
             \x20 3. There has been no motion in the hallway or the kitchen for 2 minutes\n\
             Do:\n\
             \x20 If it was started by trigger 1:\n\
             \x20   Turn on the hallway lights\n\
             \x20 If it was started by trigger 2:\n\
             \x20   Turn on the kitchen lights\n\
             \x20   Turn on the hallway lights\n\
             \x20 If it was started by trigger 3:\n\
             \x20   Turn off the kitchen lights\n\
             \x20   Turn off the hallway lights"
        );
    }

    #[test]
    fn describes_conditions_waits_and_both_ways_of_an_if() {
        let described = describe(
            &serde_json::from_value(json!({
                "id": "r",
                "triggers": [{ "type": "contact", "device": "office/door", "open": false }],
                "conditions": [{ "kind": "sun", "after": "sunset", "before": "sunrise" }],
                "actions": [
                    { "do": "delay", "minutes": 5 },
                    { "do": "if",
                      "conditions": [{ "kind": "motion_is", "device": "office/motion", "detected": false }],
                      "then": [{ "do": "set_room", "room": "office", "on": false }],
                      "else": [{ "do": "notify", "body": "Somebody is still in the office" }] }
                ],
            }))
            .unwrap(),
        );
        assert_eq!(
            described,
            "When:\n\
             \x20 1. The office door closes\n\
             Only if:\n\
             \x20 - It is dark\n\
             Do:\n\
             \x20 Wait 5 minutes\n\
             \x20 If the office motion sees no motion:\n\
             \x20   Turn off the office lights\n\
             \x20 Otherwise:\n\
             \x20   Say \"Somebody is still in the office\""
        );
    }

    #[test]
    fn does_not_say_the_room_twice() {
        assert_eq!(
            thing("living_room/living_room_lamp"),
            "the living room lamp"
        );
        assert_eq!(thing("z2m:office/go"), "the office go");
    }
}
