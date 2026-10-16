//! Automation rule DTOs — raw strings, parsed into typed models by
//! `niles-automations::Rule::from_dto`.
//!
//! The same shape is what the app sends and what the database keeps,
//! so it serializes as well as reads, leaving out what is not set.

use crate::error::Result;
use serde::{Deserialize, Serialize};

/// Top-level automations container.
///
/// `#[serde(transparent)]` lets `[[automation]]` TOML arrays
/// deserialize directly into the inner `Vec` while still giving us
/// a place to hang a `validate()` method.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(transparent)]
pub struct AutomationsConfig {
    pub rules: Vec<AutomationRuleDto>,
}

impl AutomationsConfig {
    /// TOML-level structural sanity. Per-rule semantic validation
    /// (device-id parsing, etc.) lives in `niles-automations`.
    pub fn validate(&self) -> Result<()> {
        // Intentionally permissive: bad rules are warned and skipped
        // at engine-build time, not startup-fatal.
        Ok(())
    }
}

/// One `[[automation]]` table from the config file, or one rule made
/// by voice or in the app.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(from = "RawRule")]
pub struct AutomationRuleDto {
    pub id: String,
    pub description: String,
    pub enabled: bool,
    pub triggers: Vec<TriggerDto>,
    pub conditions: Vec<ConditionDto>,
    pub actions: Vec<ActionDto>,
}

/// A rule as it may be written: with a list of triggers, or — as every
/// rule was before there could be several — with one.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRule {
    id: String,
    #[serde(default)]
    description: String,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    trigger: Option<TriggerDto>,
    #[serde(default)]
    triggers: Vec<TriggerDto>,
    #[serde(default)]
    conditions: Vec<ConditionDto>,
    #[serde(default)]
    actions: Vec<ActionDto>,
}

impl From<RawRule> for AutomationRuleDto {
    fn from(raw: RawRule) -> Self {
        let mut triggers = raw.triggers;
        if triggers.is_empty()
            && let Some(one) = raw.trigger
        {
            triggers.push(one);
        }
        Self {
            id: raw.id,
            description: raw.description,
            enabled: raw.enabled,
            triggers,
            conditions: raw.conditions,
            actions: raw.actions,
        }
    }
}

/// One of a rule's triggers. `id` is what `triggered_by` names it by;
/// Niles gives one (`t1`, `t2`…) to a trigger saved without.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct TriggerDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(flatten)]
    pub kind: TriggerKind,
}

/// How long a state has to have held. Each part is optional; the
/// whole is their sum.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DurationDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hours: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minutes: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds: Option<u32>,
}

impl DurationDto {
    pub fn total_seconds(&self) -> u64 {
        u64::from(self.hours.unwrap_or(0)) * 3600
            + u64::from(self.minutes.unwrap_or(0)) * 60
            + u64::from(self.seconds.unwrap_or(0))
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TriggerKind {
    DeviceState {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        device: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        room: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        on: Option<bool>,
        #[serde(default, rename = "for", skip_serializing_if = "Option::is_none")]
        hold: Option<DurationDto>,
    },
    /// A door or window sensor opening (`open = true`), closing
    /// (`false`), or either (omitted).
    Contact {
        device: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        open: Option<bool>,
        #[serde(default, rename = "for", skip_serializing_if = "Option::is_none")]
        hold: Option<DurationDto>,
    },
    /// A motion sensor seeing somebody (`detected = true`), or nothing
    /// any more (`false`) — with `for`, "no motion for 5 minutes".
    Motion {
        device: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detected: Option<bool>,
        #[serde(default, rename = "for", skip_serializing_if = "Option::is_none")]
        hold: Option<DurationDto>,
    },
    /// A reading going above or below a value.
    Numeric {
        device: String,
        /// "temperature" | "humidity" | "battery" | "brightness" |
        /// "illuminance", or the name of any reading the device reports
        /// ("price", "power", "co2").
        measure: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        above: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        below: Option<f64>,
        #[serde(default, rename = "for", skip_serializing_if = "Option::is_none")]
        hold: Option<DurationDto>,
    },
    DeviceAction {
        device: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        action: Option<String>,
    },
    TimerFired {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    Presence {
        /// "home" | "away" | "unknown". Omit to fire on any change.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        state: Option<String>,
        #[serde(default, rename = "for", skip_serializing_if = "Option::is_none")]
        hold: Option<DurationDto>,
    },
    /// A time of day, `HH:MM`, on the weekdays named (every day when
    /// none are).
    Time {
        at: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        days: Option<Vec<String>>,
    },
    /// Sunrise or sunset at the house, moved by an offset in minutes
    /// (negative: before).
    Sun {
        /// "sunrise" | "sunset".
        event: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        offset_minutes: Option<i32>,
    },
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConditionDto {
    TimeOfDay {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        before: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        days: Option<Vec<String>>,
    },
    /// After and/or before sunrise or sunset, each with its own offset:
    /// `after: sunset, before: sunrise` is "while it is dark".
    Sun {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        before: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after_offset_minutes: Option<i32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        before_offset_minutes: Option<i32>,
    },
    DeviceIs {
        device: String,
        on: bool,
    },
    ContactIs {
        device: String,
        open: bool,
    },
    MotionIs {
        device: String,
        detected: bool,
    },
    Numeric {
        device: String,
        measure: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        above: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        below: Option<f64>,
    },
    PresenceIs {
        state: String,
    },
    /// Which of the rule's triggers started this run, by their ids.
    TriggeredBy {
        triggers: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(tag = "do", rename_all = "snake_case")]
pub enum ActionDto {
    SetDevice {
        device: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        on: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        brightness: Option<u8>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kelvin: Option<u16>,
    },
    /// Every light in a room.
    SetRoom {
        room: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        on: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        brightness: Option<u8>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kelvin: Option<u16>,
    },
    /// One blind (`device`), or every blind in a room (`room`), to a
    /// position: `0` shut, `100` open.
    SetBlinds {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        device: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        room: Option<String>,
        position: u8,
    },
    Notify {
        body: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        room: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        priority: Option<String>,
    },
    /// The morning routine, now, with its own settings: lights that are
    /// off come on at 0% and ramp to `brightness` over `minutes`, then
    /// hold there. Every curve-managed light when neither `rooms` nor
    /// `devices` is given.
    WakeUp {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        minutes: Option<u32>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        rooms: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        devices: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        except: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        brightness: Option<u8>,
        /// The warmth the ramp starts at; `kelvin` when left out.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        start_kelvin: Option<u16>,
        /// The warmth it ends at. Neither is the curve's, as it goes.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kelvin: Option<u16>,
    },
    /// Wait, then go on with the next action.
    Delay {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hours: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        minutes: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seconds: Option<u32>,
    },
    /// When every condition holds, `then`; otherwise `else`.
    If {
        conditions: Vec<ConditionDto>,
        then: Vec<ActionDto>,
        #[serde(default, rename = "else", skip_serializing_if = "Vec::is_empty")]
        otherwise: Vec<ActionDto>,
    },
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rule_with_one_trigger_reads_as_a_list_of_one() {
        let old: AutomationRuleDto = serde_json::from_str(
            r#"{"id":"r","trigger":{"type":"contact","device":"office/door","open":true},
                "actions":[{"do":"set_room","room":"office","on":false}]}"#,
        )
        .unwrap();
        assert_eq!(old.triggers.len(), 1);
        assert_eq!(old.triggers[0].id, None);
        let written = serde_json::to_value(&old).unwrap();
        assert!(written.get("trigger").is_none());
        assert_eq!(written["triggers"][0]["type"], "contact");
    }

    #[test]
    fn reads_durations_branches_and_the_sun() {
        let rule: AutomationRuleDto = serde_json::from_str(
            r#"{"id":"r","triggers":[
                  {"id":"opened","type":"contact","device":"office/door","open":true},
                  {"id":"left-open","type":"contact","device":"office/door","open":true,"for":{"minutes":10}},
                  {"type":"sun","event":"sunset","offset_minutes":-30}],
                "conditions":[{"kind":"sun","after":"sunset","before":"sunrise"}],
                "actions":[{"do":"if","conditions":[{"kind":"triggered_by","triggers":["opened"]}],
                            "then":[{"do":"delay","minutes":5},{"do":"set_room","room":"office","on":false}],
                            "else":[{"do":"notify","body":"Shut the door"}]}]}"#,
        )
        .unwrap();
        let TriggerKind::Contact { hold, .. } = &rule.triggers[1].kind else {
            panic!("a contact trigger");
        };
        assert_eq!(hold.as_ref().map(DurationDto::total_seconds), Some(600));
        let ActionDto::If {
            then, otherwise, ..
        } = &rule.actions[0]
        else {
            panic!("an if");
        };
        assert_eq!(then.len(), 2);
        assert_eq!(otherwise.len(), 1);
        // And back, as written.
        let again: AutomationRuleDto =
            serde_json::from_value(serde_json::to_value(&rule).unwrap()).unwrap();
        assert_eq!(again, rule);
    }
}
