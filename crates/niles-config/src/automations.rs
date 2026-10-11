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
#[serde(deny_unknown_fields)]
pub struct AutomationRuleDto {
    pub id: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub trigger: TriggerDto,
    #[serde(default)]
    pub conditions: Vec<ConditionDto>,
    #[serde(default)]
    pub actions: Vec<ActionDto>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TriggerDto {
    DeviceState {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        device: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        room: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        on: Option<bool>,
    },
    /// A door or window sensor opening (`open = true`), closing
    /// (`false`), or either (omitted).
    Contact {
        device: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        open: Option<bool>,
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
    },
    DeviceIs {
        device: String,
        on: bool,
    },
    ContactIs {
        device: String,
        open: bool,
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
    Notify {
        body: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        room: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        priority: Option<String>,
    },
}

fn default_true() -> bool {
    true
}
