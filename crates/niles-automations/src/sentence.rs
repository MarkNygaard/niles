//! A rule said in English: "When the office door opens, turn off the
//! office lights." What Niles reads back before saving one, what it says
//! when asked which automations there are, and what the app lists.

use crate::rule::say_duration;
use niles_config::{
    ActionDto, AutomationRuleDto, ConditionDto, DurationDto, TriggerDto, TriggerKind,
};
use std::time::Duration;

/// The whole rule as one sentence.
pub fn sentence(rule: &AutomationRuleDto) -> String {
    let triggers: Vec<String> = rule.triggers.iter().map(trigger).collect();
    let mut out = format!("When {}", triggers.join(", or when "));
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

/// Each trigger's id and how it is said: by its place, as the app
/// shows it — the id is the app's and the model's, not a person's.
fn trigger_names(triggers: &[TriggerDto]) -> Vec<(String, String)> {
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

fn trigger(t: &TriggerDto) -> String {
    match &t.kind {
        TriggerKind::Contact { device, open, hold } => match (open, held(hold)) {
            (Some(true), Some(h)) => format!("{} has been open for {h}", thing(device)),
            (Some(false), Some(h)) => format!("{} has been closed for {h}", thing(device)),
            (None, Some(h)) => format!("{} has stayed as it is for {h}", thing(device)),
            (Some(true), None) => format!("{} opens", thing(device)),
            (Some(false), None) => format!("{} closes", thing(device)),
            (None, None) => format!("{} opens or closes", thing(device)),
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
            let what = format!("the {measure} of {}", thing(device));
            match held(hold) {
                Some(h) => format!("{what} has been {} for {h}", bounds(*above, *below)),
                None => format!("{what} goes {}", bounds(*above, *below)),
            }
        }
        TriggerKind::DeviceAction { device, action } => match action {
            Some(action) => format!("{} is pressed ({action})", thing(device)),
            None => format!("{} is pressed", thing(device)),
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

fn condition(condition: &ConditionDto, names: &[(String, String)]) -> String {
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
            "the {measure} of {} is {}",
            thing(device),
            bounds(*above, *below)
        ),
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

fn action(step: &ActionDto, names: &[(String, String)]) -> String {
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
        ActionDto::Notify { body, room, .. } => match room {
            Some(room) => format!("say \"{body}\" in the {}", words(room)),
            None => format!("say \"{body}\""),
        },
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
    fn does_not_say_the_room_twice() {
        assert_eq!(
            thing("living_room/living_room_lamp"),
            "the living room lamp"
        );
        assert_eq!(thing("z2m:office/go"), "the office go");
    }
}
