//! Typed automation rule model + trigger matching + condition evaluation.

use crate::error::{Error, Result};
use chrono::{DateTime, NaiveTime, Utc};
use chrono_tz::Tz;
use niles_core::{DeviceId, DeviceRegistry, Event, RoomName};

/// A validated, ready-to-run automation rule.
#[derive(Debug, Clone)]
pub struct Rule {
    pub id: String,
    pub description: String,
    pub enabled: bool,
    pub trigger: Trigger,
    pub conditions: Vec<Condition>,
    pub actions: Vec<Action>,
}

/// What event starts the rule.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Trigger {
    DeviceState {
        device: Option<DeviceId>,
        room: Option<RoomName>,
        on: Option<bool>,
    },
    DeviceAction {
        device: DeviceId,
        action: Option<String>,
    },
    TimerFired {
        name: Option<String>,
    },
}

/// Gate that must pass after the trigger fires.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Condition {
    TimeOfDay {
        after: Option<NaiveTime>,
        before: Option<NaiveTime>,
    },
    DeviceIs {
        device: DeviceId,
        on: bool,
    },
}

/// What to do when the rule fires.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Action {
    SetDevice {
        device: DeviceId,
        on: Option<bool>,
        brightness: Option<u8>,
        kelvin: Option<u16>,
    },
    Notify {
        body: String,
        room: Option<String>,
        priority: Priority,
    },
}

/// Local priority enum — kept inside `niles-automations` so the crate
/// does not depend on `niles-notifications`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Priority {
    Routine,
    Important,
    Urgent,
}

impl Rule {
    /// Convert a config DTO into a typed, validated `Rule`.
    pub fn from_dto(dto: &niles_config::AutomationRuleDto, default_source: &str) -> Result<Self> {
        let id = dto.id.clone();
        validate_id(&id)?;

        let trigger = parse_trigger(&id, &dto.trigger, default_source)?;
        let conditions = dto
            .conditions
            .iter()
            .map(|c| parse_condition(&id, c, default_source))
            .collect::<Result<Vec<_>>>()?;
        let actions = dto
            .actions
            .iter()
            .map(|a| parse_action(&id, a, default_source))
            .collect::<Result<Vec<_>>>()?;

        if actions.is_empty() {
            return Err(Error::NoActions { id });
        }

        Ok(Self {
            id,
            description: dto.description.clone(),
            enabled: dto.enabled,
            trigger,
            conditions,
            actions,
        })
    }
}

impl Trigger {
    /// Returns `true` if this trigger matches the given event.
    pub fn matches(&self, ev: &Event) -> bool {
        match self {
            Trigger::DeviceState { device, room, on } => match ev {
                Event::DeviceStateChanged { id, state } => {
                    (device.is_none() || device.as_ref() == Some(id))
                        && (room.is_none() || room.as_ref() == Some(id.room()))
                        && (on.is_none() || state.on == *on)
                }
                _ => false,
            },
            Trigger::DeviceAction { device, action } => match ev {
                Event::DeviceAction {
                    id,
                    action: ev_action,
                } => {
                    id == device
                        && (action.is_none() || Some(ev_action.as_str()) == action.as_deref())
                }
                _ => false,
            },
            Trigger::TimerFired { name } => match ev {
                Event::TimerFired { name: ev_name, .. } => {
                    name.is_none() || name.as_ref() == ev_name.as_ref()
                }
                _ => false,
            },
        }
    }
}

impl Condition {
    /// Evaluate this condition against the current world state.
    pub fn evaluate(&self, registry: &DeviceRegistry, now: DateTime<Utc>, tz: Tz) -> bool {
        match self {
            Condition::TimeOfDay { after, before } => {
                let local = now.with_timezone(&tz);
                let t = local.time();
                covers(*after, *before, t)
            }
            Condition::DeviceIs { device, on } => registry
                .get(device)
                .and_then(|d| d.state.on)
                .map(|s| s == *on)
                .unwrap_or(false),
        }
    }
}

// ------------------------------------------------------------------
// Helpers
// ------------------------------------------------------------------

fn validate_id(id: &str) -> Result<()> {
    if id.is_empty() {
        return Err(Error::InvalidId {
            id: id.to_string(),
            reason: "empty".into(),
        });
    }
    if id.starts_with('-') || id.ends_with('-') {
        return Err(Error::InvalidId {
            id: id.to_string(),
            reason: "leading or trailing hyphen".into(),
        });
    }
    if id.contains("--") {
        return Err(Error::InvalidId {
            id: id.to_string(),
            reason: "double hyphen".into(),
        });
    }
    if let Some(c) = id
        .chars()
        .find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-'))
    {
        return Err(Error::InvalidId {
            id: id.to_string(),
            reason: format!("'{c}' not in [a-z0-9-]"),
        });
    }
    Ok(())
}

fn parse_trigger(
    id: &str,
    dto: &niles_config::TriggerDto,
    default_source: &str,
) -> Result<Trigger> {
    match dto {
        niles_config::TriggerDto::DeviceState { device, room, on } => Ok(Trigger::DeviceState {
            device: device
                .as_ref()
                .map(|s| parse_device_id(id, s, default_source))
                .transpose()?,
            room: room.as_ref().map(|s| parse_room_name(id, s)).transpose()?,
            on: *on,
        }),
        niles_config::TriggerDto::DeviceAction { device, action } => Ok(Trigger::DeviceAction {
            device: parse_device_id(id, device, default_source)?,
            action: action.clone(),
        }),
        niles_config::TriggerDto::TimerFired { name } => {
            Ok(Trigger::TimerFired { name: name.clone() })
        }
    }
}

fn parse_condition(
    id: &str,
    dto: &niles_config::ConditionDto,
    default_source: &str,
) -> Result<Condition> {
    match dto {
        niles_config::ConditionDto::TimeOfDay { after, before } => Ok(Condition::TimeOfDay {
            after: after.as_ref().map(|s| parse_time(id, s)).transpose()?,
            before: before.as_ref().map(|s| parse_time(id, s)).transpose()?,
        }),
        niles_config::ConditionDto::DeviceIs { device, on } => Ok(Condition::DeviceIs {
            device: parse_device_id(id, device, default_source)?,
            on: *on,
        }),
    }
}

fn parse_action(id: &str, dto: &niles_config::ActionDto, default_source: &str) -> Result<Action> {
    match dto {
        niles_config::ActionDto::SetDevice {
            device,
            on,
            brightness,
            kelvin,
        } => {
            if let Some(b) = brightness
                && *b > 100
            {
                return Err(Error::InvalidBrightness {
                    id: id.to_string(),
                    value: *b,
                });
            }
            if let Some(k) = kelvin
                && !(2000..=6500).contains(k)
            {
                return Err(Error::InvalidKelvin {
                    id: id.to_string(),
                    value: *k,
                });
            }
            Ok(Action::SetDevice {
                device: parse_device_id(id, device, default_source)?,
                on: *on,
                brightness: *brightness,
                kelvin: *kelvin,
            })
        }
        niles_config::ActionDto::Notify {
            body,
            room,
            priority,
        } => Ok(Action::Notify {
            body: body.clone(),
            room: room.clone(),
            priority: parse_priority(id, priority.as_deref())?,
        }),
    }
}

fn parse_device_id(id: &str, value: &str, default_source: &str) -> Result<DeviceId> {
    let s = if value.contains(':') {
        value.to_string()
    } else {
        format!("{default_source}:{value}")
    };
    DeviceId::parse(&s).map_err(|e| Error::InvalidDeviceId {
        id: id.to_string(),
        value: value.to_string(),
        source: e,
    })
}

fn parse_room_name(id: &str, value: &str) -> Result<RoomName> {
    RoomName::parse(value).map_err(|e| Error::InvalidRoom {
        id: id.to_string(),
        value: value.to_string(),
        source: e,
    })
}

fn parse_time(id: &str, value: &str) -> Result<NaiveTime> {
    NaiveTime::parse_from_str(value, "%H:%M").map_err(|e| Error::InvalidTime {
        id: id.to_string(),
        value: value.to_string(),
        source: e,
    })
}

fn parse_priority(id: &str, value: Option<&str>) -> Result<Priority> {
    match value.map(|s| s.to_ascii_lowercase()).as_deref() {
        None | Some("routine") => Ok(Priority::Routine),
        Some("important") => Ok(Priority::Important),
        Some("urgent") => Ok(Priority::Urgent),
        Some(other) => Err(Error::InvalidPriority {
            id: id.to_string(),
            value: other.to_string(),
        }),
    }
}

/// Returns `true` if `t` falls inside the window `[after, before]`.
///
/// Handles both non-wrapping (10:00–14:00) and wrapping (21:00–06:00)
/// windows, including exact boundary times. If both bounds are `None`,
/// always returns `true`.
pub(crate) fn covers(after: Option<NaiveTime>, before: Option<NaiveTime>, t: NaiveTime) -> bool {
    match (after, before) {
        (None, None) => true,
        (Some(start), None) => t >= start,
        (None, Some(end)) => t <= end,
        (Some(start), Some(end)) => {
            if start <= end {
                // Non-wrapping: e.g. 10:00–14:00
                t >= start && t <= end
            } else {
                // Wrapping: e.g. 21:00–06:00
                t >= start || t <= end
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use niles_core::{DeviceClass, DeviceName, DeviceState};

    fn device_id(room: &str, name: &str) -> DeviceId {
        DeviceId::new(
            "z2m",
            RoomName::parse(room).unwrap(),
            DeviceName::parse(name).unwrap(),
        )
        .unwrap()
    }

    fn rule_dto(
        id: &str,
        trigger: niles_config::TriggerDto,
        actions: Vec<niles_config::ActionDto>,
    ) -> niles_config::AutomationRuleDto {
        niles_config::AutomationRuleDto {
            id: id.to_string(),
            description: String::new(),
            enabled: true,
            trigger,
            conditions: Vec::new(),
            actions,
        }
    }

    // ---- validation ------------------------------------------------

    #[test]
    fn rejects_empty_id() {
        let dto = rule_dto(
            "",
            niles_config::TriggerDto::TimerFired { name: None },
            vec![niles_config::ActionDto::Notify {
                body: "x".into(),
                room: None,
                priority: None,
            }],
        );
        assert!(Rule::from_dto(&dto, "z2m").is_err());
    }

    #[test]
    fn rejects_leading_hyphen_id() {
        let dto = rule_dto(
            "-bad",
            niles_config::TriggerDto::TimerFired { name: None },
            vec![niles_config::ActionDto::Notify {
                body: "x".into(),
                room: None,
                priority: None,
            }],
        );
        assert!(Rule::from_dto(&dto, "z2m").is_err());
    }

    #[test]
    fn rejects_double_hyphen_id() {
        let dto = rule_dto(
            "bad--id",
            niles_config::TriggerDto::TimerFired { name: None },
            vec![niles_config::ActionDto::Notify {
                body: "x".into(),
                room: None,
                priority: None,
            }],
        );
        assert!(Rule::from_dto(&dto, "z2m").is_err());
    }

    #[test]
    fn rejects_no_actions() {
        let dto = rule_dto(
            "no-actions",
            niles_config::TriggerDto::TimerFired { name: None },
            vec![],
        );
        assert!(Rule::from_dto(&dto, "z2m").is_err());
    }

    #[test]
    fn rejects_brightness_over_100() {
        let dto = rule_dto(
            "bright",
            niles_config::TriggerDto::TimerFired { name: None },
            vec![niles_config::ActionDto::SetDevice {
                device: "z2m:kitchen/light".into(),
                on: None,
                brightness: Some(101),
                kelvin: None,
            }],
        );
        assert!(Rule::from_dto(&dto, "z2m").is_err());
    }

    #[test]
    fn rejects_kelvin_out_of_range() {
        let dto = rule_dto(
            "kelv",
            niles_config::TriggerDto::TimerFired { name: None },
            vec![niles_config::ActionDto::SetDevice {
                device: "z2m:kitchen/light".into(),
                on: None,
                brightness: None,
                kelvin: Some(9000),
            }],
        );
        assert!(Rule::from_dto(&dto, "z2m").is_err());
    }

    #[test]
    fn rejects_malformed_time() {
        let dto = niles_config::AutomationRuleDto {
            id: "time-bad".into(),
            description: String::new(),
            enabled: true,
            trigger: niles_config::TriggerDto::TimerFired { name: None },
            conditions: vec![niles_config::ConditionDto::TimeOfDay {
                after: Some("25:99".into()),
                before: None,
            }],
            actions: vec![niles_config::ActionDto::Notify {
                body: "x".into(),
                room: None,
                priority: None,
            }],
        };
        assert!(Rule::from_dto(&dto, "z2m").is_err());
    }

    #[test]
    fn rejects_unknown_priority() {
        let dto = rule_dto(
            "prio",
            niles_config::TriggerDto::TimerFired { name: None },
            vec![niles_config::ActionDto::Notify {
                body: "x".into(),
                room: None,
                priority: Some("critical".into()),
            }],
        );
        assert!(Rule::from_dto(&dto, "z2m").is_err());
    }

    #[test]
    fn rejects_bad_device_id() {
        let dto = rule_dto(
            "dev",
            niles_config::TriggerDto::DeviceState {
                device: Some("not_valid".into()),
                room: None,
                on: None,
            },
            vec![niles_config::ActionDto::Notify {
                body: "x".into(),
                room: None,
                priority: None,
            }],
        );
        assert!(Rule::from_dto(&dto, "z2m").is_err());
    }

    // ---- round-trip ------------------------------------------------

    #[test]
    fn round_trip_all_variants() {
        let dto = niles_config::AutomationRuleDto {
            id: "all-vars".into(),
            description: "test".into(),
            enabled: true,
            trigger: niles_config::TriggerDto::DeviceState {
                device: Some("z2m:kitchen/motion".into()),
                room: Some("kitchen".into()),
                on: Some(true),
            },
            conditions: vec![
                niles_config::ConditionDto::TimeOfDay {
                    after: Some("21:00".into()),
                    before: Some("06:00".into()),
                },
                niles_config::ConditionDto::DeviceIs {
                    device: "z2m:hallway/light".into(),
                    on: true,
                },
            ],
            actions: vec![
                niles_config::ActionDto::SetDevice {
                    device: "z2m:kitchen/ceiling".into(),
                    on: Some(true),
                    brightness: Some(30),
                    kelvin: Some(2700),
                },
                niles_config::ActionDto::Notify {
                    body: "motion detected".into(),
                    room: Some("kitchen".into()),
                    priority: Some("important".into()),
                },
            ],
        };
        let rule = Rule::from_dto(&dto, "z2m").unwrap();
        assert_eq!(rule.id, "all-vars");
        assert_eq!(rule.description, "test");
        assert!(rule.enabled);
        assert_eq!(rule.conditions.len(), 2);
        assert_eq!(rule.actions.len(), 2);
    }

    // ---- trigger matching ------------------------------------------

    #[test]
    fn device_state_matches_no_filters() {
        let trigger = Trigger::DeviceState {
            device: None,
            room: None,
            on: None,
        };
        let ev = Event::DeviceStateChanged {
            id: device_id("kitchen", "light"),
            state: DeviceState {
                on: Some(true),
                ..Default::default()
            },
        };
        assert!(trigger.matches(&ev));
    }

    #[test]
    fn device_state_matches_with_filters() {
        let trigger = Trigger::DeviceState {
            device: Some(device_id("kitchen", "light")),
            room: Some(RoomName::parse("kitchen").unwrap()),
            on: Some(true),
        };
        let ev = Event::DeviceStateChanged {
            id: device_id("kitchen", "light"),
            state: DeviceState {
                on: Some(true),
                ..Default::default()
            },
        };
        assert!(trigger.matches(&ev));
    }

    #[test]
    fn device_state_no_match_wrong_device() {
        let trigger = Trigger::DeviceState {
            device: Some(device_id("kitchen", "light")),
            room: None,
            on: None,
        };
        let ev = Event::DeviceStateChanged {
            id: device_id("hallway", "light"),
            state: DeviceState::default(),
        };
        assert!(!trigger.matches(&ev));
    }

    #[test]
    fn device_state_no_match_none_on() {
        let trigger = Trigger::DeviceState {
            device: None,
            room: None,
            on: Some(true),
        };
        let ev = Event::DeviceStateChanged {
            id: device_id("kitchen", "light"),
            state: DeviceState {
                on: None,
                ..Default::default()
            },
        };
        assert!(!trigger.matches(&ev));
    }

    #[test]
    fn device_state_no_match_wrong_room() {
        let trigger = Trigger::DeviceState {
            device: None,
            room: Some(RoomName::parse("kitchen").unwrap()),
            on: None,
        };
        let ev = Event::DeviceStateChanged {
            id: device_id("hallway", "light"),
            state: DeviceState::default(),
        };
        assert!(!trigger.matches(&ev));
    }

    #[test]
    fn device_action_matches() {
        let trigger = Trigger::DeviceAction {
            device: device_id("kitchen", "switch"),
            action: None,
        };
        let ev = Event::DeviceAction {
            id: device_id("kitchen", "switch"),
            action: "on_press".into(),
        };
        assert!(trigger.matches(&ev));
    }

    #[test]
    fn device_action_matches_filtered() {
        let trigger = Trigger::DeviceAction {
            device: device_id("kitchen", "switch"),
            action: Some("on_press".into()),
        };
        let ev = Event::DeviceAction {
            id: device_id("kitchen", "switch"),
            action: "on_press".into(),
        };
        assert!(trigger.matches(&ev));
    }

    #[test]
    fn device_action_no_match_wrong_action() {
        let trigger = Trigger::DeviceAction {
            device: device_id("kitchen", "switch"),
            action: Some("on_press".into()),
        };
        let ev = Event::DeviceAction {
            id: device_id("kitchen", "switch"),
            action: "off_press".into(),
        };
        assert!(!trigger.matches(&ev));
    }

    #[test]
    fn timer_fired_matches_no_name() {
        let trigger = Trigger::TimerFired { name: None };
        let ev = Event::TimerFired {
            id: 1,
            name: Some("pasta".into()),
            origin: "127.0.0.1:1".parse().unwrap(),
        };
        assert!(trigger.matches(&ev));
    }

    #[test]
    fn timer_fired_matches_named() {
        let trigger = Trigger::TimerFired {
            name: Some("pasta".into()),
        };
        let ev = Event::TimerFired {
            id: 1,
            name: Some("pasta".into()),
            origin: "127.0.0.1:1".parse().unwrap(),
        };
        assert!(trigger.matches(&ev));
    }

    #[test]
    fn timer_fired_no_match_wrong_name() {
        let trigger = Trigger::TimerFired {
            name: Some("pasta".into()),
        };
        let ev = Event::TimerFired {
            id: 1,
            name: Some("rice".into()),
            origin: "127.0.0.1:1".parse().unwrap(),
        };
        assert!(!trigger.matches(&ev));
    }

    // ---- condition evaluation --------------------------------------

    fn utc_dt(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, minute, 0)
            .unwrap()
    }

    #[test]
    fn time_of_day_wrapping_window() {
        let cond = Condition::TimeOfDay {
            after: Some(NaiveTime::from_hms_opt(21, 0, 0).unwrap()),
            before: Some(NaiveTime::from_hms_opt(6, 0, 0).unwrap()),
        };
        assert!(cond.evaluate(&DeviceRegistry::new(), utc_dt(2026, 1, 1, 23, 0), Tz::UTC));
        assert!(!cond.evaluate(&DeviceRegistry::new(), utc_dt(2026, 1, 1, 12, 0), Tz::UTC));
        assert!(cond.evaluate(&DeviceRegistry::new(), utc_dt(2026, 1, 1, 21, 0), Tz::UTC));
        assert!(cond.evaluate(&DeviceRegistry::new(), utc_dt(2026, 1, 1, 6, 0), Tz::UTC));
    }

    #[test]
    fn device_is_present_matches() {
        let registry = DeviceRegistry::new();
        let id = device_id("kitchen", "light");
        registry.upsert(niles_core::Device::new(
            id.clone(),
            DeviceState {
                on: Some(true),
                ..Default::default()
            },
            DeviceClass::Light,
        ));
        let cond = Condition::DeviceIs {
            device: id,
            on: true,
        };
        assert!(cond.evaluate(&registry, Utc::now(), Tz::UTC));
    }

    #[test]
    fn device_is_present_no_match() {
        let registry = DeviceRegistry::new();
        let id = device_id("kitchen", "light");
        registry.upsert(niles_core::Device::new(
            id.clone(),
            DeviceState {
                on: Some(false),
                ..Default::default()
            },
            DeviceClass::Light,
        ));
        let cond = Condition::DeviceIs {
            device: id,
            on: true,
        };
        assert!(!cond.evaluate(&registry, Utc::now(), Tz::UTC));
    }

    #[test]
    fn device_is_absent_fails_safe() {
        let registry = DeviceRegistry::new();
        let id = device_id("kitchen", "light");
        let cond = Condition::DeviceIs {
            device: id,
            on: true,
        };
        assert!(!cond.evaluate(&registry, Utc::now(), Tz::UTC));
    }

    // ---- covers ----------------------------------------------------

    #[test]
    fn covers_none_none_is_true() {
        assert!(covers(
            None,
            None,
            NaiveTime::from_hms_opt(12, 0, 0).unwrap()
        ));
    }

    #[test]
    fn covers_after_only() {
        let t = NaiveTime::from_hms_opt(12, 0, 0).unwrap();
        assert!(covers(
            Some(t),
            None,
            NaiveTime::from_hms_opt(13, 0, 0).unwrap()
        ));
        assert!(!covers(
            Some(t),
            None,
            NaiveTime::from_hms_opt(11, 0, 0).unwrap()
        ));
    }

    #[test]
    fn covers_before_only() {
        let t = NaiveTime::from_hms_opt(12, 0, 0).unwrap();
        assert!(covers(
            None,
            Some(t),
            NaiveTime::from_hms_opt(11, 0, 0).unwrap()
        ));
        assert!(!covers(
            None,
            Some(t),
            NaiveTime::from_hms_opt(13, 0, 0).unwrap()
        ));
    }

    #[test]
    fn covers_non_wrapping_window() {
        let start = NaiveTime::from_hms_opt(10, 0, 0).unwrap();
        let end = NaiveTime::from_hms_opt(14, 0, 0).unwrap();
        assert!(covers(
            Some(start),
            Some(end),
            NaiveTime::from_hms_opt(12, 0, 0).unwrap()
        ));
        assert!(covers(
            Some(start),
            Some(end),
            NaiveTime::from_hms_opt(10, 0, 0).unwrap()
        ));
        assert!(covers(
            Some(start),
            Some(end),
            NaiveTime::from_hms_opt(14, 0, 0).unwrap()
        ));
        assert!(!covers(
            Some(start),
            Some(end),
            NaiveTime::from_hms_opt(9, 59, 0).unwrap()
        ));
        assert!(!covers(
            Some(start),
            Some(end),
            NaiveTime::from_hms_opt(14, 1, 0).unwrap()
        ));
    }

    #[test]
    fn rejects_trailing_hyphen_id() {
        let dto = rule_dto(
            "bad-",
            niles_config::TriggerDto::TimerFired { name: None },
            vec![niles_config::ActionDto::Notify {
                body: "x".into(),
                room: None,
                priority: None,
            }],
        );
        assert!(Rule::from_dto(&dto, "z2m").is_err());
    }

    #[test]
    fn device_action_no_match_wrong_device() {
        let trigger = Trigger::DeviceAction {
            device: device_id("kitchen", "switch"),
            action: None,
        };
        let ev = Event::DeviceAction {
            id: device_id("hallway", "switch"),
            action: "on_press".into(),
        };
        assert!(!trigger.matches(&ev));
    }

    #[test]
    fn device_state_no_match_non_state_event() {
        let trigger = Trigger::DeviceState {
            device: None,
            room: None,
            on: None,
        };
        let ev = Event::DeviceAction {
            id: device_id("kitchen", "switch"),
            action: "on_press".into(),
        };
        assert!(!trigger.matches(&ev));
    }

    #[test]
    fn timer_fired_matches_both_none() {
        let trigger = Trigger::TimerFired { name: None };
        let ev = Event::TimerFired {
            id: 1,
            name: None,
            origin: "127.0.0.1:1".parse().unwrap(),
        };
        assert!(trigger.matches(&ev));
    }

    #[test]
    fn timer_fired_no_match_named_vs_none() {
        let trigger = Trigger::TimerFired {
            name: Some("pasta".into()),
        };
        let ev = Event::TimerFired {
            id: 1,
            name: None,
            origin: "127.0.0.1:1".parse().unwrap(),
        };
        assert!(!trigger.matches(&ev));
    }

    #[test]
    fn device_is_present_false_matches() {
        let registry = DeviceRegistry::new();
        let id = device_id("kitchen", "light");
        registry.upsert(niles_core::Device::new(
            id.clone(),
            DeviceState {
                on: Some(false),
                ..Default::default()
            },
            DeviceClass::Light,
        ));
        let cond = Condition::DeviceIs {
            device: id,
            on: false,
        };
        assert!(cond.evaluate(&registry, Utc::now(), Tz::UTC));
    }
}
