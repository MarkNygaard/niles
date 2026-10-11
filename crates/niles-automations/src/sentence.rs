//! A rule said in English: "When the office door opens, turn off the
//! office lights." What Niles reads back before saving one, what it says
//! when asked which automations there are, and what the app lists.

use niles_config::{ActionDto, AutomationRuleDto, ConditionDto, TriggerDto};

/// The whole rule as one sentence.
pub fn sentence(rule: &AutomationRuleDto) -> String {
    let mut out = trigger(&rule.trigger);
    let conditions: Vec<String> = rule.conditions.iter().map(condition).collect();
    if !conditions.is_empty() {
        out.push_str(", if ");
        out.push_str(&conditions.join(" and "));
    }
    let actions: Vec<String> = rule.actions.iter().map(action).collect();
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

fn trigger(trigger: &TriggerDto) -> String {
    match trigger {
        TriggerDto::Contact { device, open } => {
            let what = match open {
                Some(true) => "opens",
                Some(false) => "closes",
                None => "opens or closes",
            };
            format!("When {} {what}", thing(device))
        }
        TriggerDto::DeviceState { device, room, on } => {
            let turns = match on {
                Some(true) => "turns on",
                Some(false) => "turns off",
                None => "changes",
            };
            match (device, room) {
                (Some(device), _) => format!("When {} {turns}", thing(device)),
                (None, Some(room)) => format!("When something in the {} {turns}", words(room)),
                (None, None) => format!("When any device {turns}"),
            }
        }
        TriggerDto::DeviceAction { device, action } => match action {
            Some(action) => format!("When {} is pressed ({action})", thing(device)),
            None => format!("When {} is pressed", thing(device)),
        },
        TriggerDto::TimerFired { name } => match name {
            Some(name) => format!("When the {name} timer goes off"),
            None => "When a timer goes off".to_string(),
        },
        TriggerDto::Presence { state } => match state.as_deref() {
            Some("home") => "When someone comes home".to_string(),
            Some("away") => "When everyone has left".to_string(),
            Some(other) => format!("When the house becomes {other}"),
            None => "When someone comes or goes".to_string(),
        },
    }
}

fn condition(condition: &ConditionDto) -> String {
    match condition {
        ConditionDto::TimeOfDay { after, before } => match (after, before) {
            (Some(after), Some(before)) => format!("it is between {after} and {before}"),
            (Some(after), None) => format!("it is after {after}"),
            (None, Some(before)) => format!("it is before {before}"),
            (None, None) => "it is any time".to_string(),
        },
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
    }
}

fn action(action: &ActionDto) -> String {
    match action {
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

    fn rule(
        trigger: TriggerDto,
        conditions: Vec<ConditionDto>,
        actions: Vec<ActionDto>,
    ) -> AutomationRuleDto {
        AutomationRuleDto {
            id: "r".into(),
            description: String::new(),
            enabled: true,
            trigger,
            conditions,
            actions,
        }
    }

    fn lights_off(room: &str) -> ActionDto {
        ActionDto::SetRoom {
            room: room.into(),
            on: Some(false),
            brightness: None,
            kelvin: None,
        }
    }

    #[test]
    fn says_the_garden_door_rule() {
        let r = rule(
            TriggerDto::Contact {
                device: "office/door".into(),
                open: Some(true),
            },
            vec![],
            vec![lights_off("office")],
        );
        assert_eq!(
            sentence(&r),
            "When the office door opens, turn off the office lights."
        );
    }

    #[test]
    fn says_conditions_and_several_actions() {
        let r = rule(
            TriggerDto::Presence {
                state: Some("home".into()),
            },
            vec![
                ConditionDto::TimeOfDay {
                    after: Some("18:00".into()),
                    before: Some("06:00".into()),
                },
                ConditionDto::ContactIs {
                    device: "z2m:office/door".into(),
                    open: false,
                },
            ],
            vec![
                ActionDto::SetDevice {
                    device: "hallway/ceiling_light".into(),
                    on: Some(true),
                    brightness: Some(40),
                    kelvin: Some(2700),
                },
                ActionDto::Notify {
                    body: "Welcome home".into(),
                    room: Some("living_room".into()),
                    priority: None,
                },
            ],
        );
        assert_eq!(
            sentence(&r),
            "When someone comes home, if it is between 18:00 and 06:00 and the office door is \
             closed, turn on the hallway ceiling light at 40% and 2700 K, then say \"Welcome \
             home\" in the living room."
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
