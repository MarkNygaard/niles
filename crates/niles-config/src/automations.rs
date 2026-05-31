//! Automation rule DTOs — raw strings, parsed into typed models by
//! `niles-automations::Rule::from_dto`.

use crate::error::Result;
use serde::Deserialize;

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

/// One `[[automation]]` table from the config file.
#[derive(Debug, Clone, Deserialize)]
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

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TriggerDto {
    DeviceState {
        #[serde(default)]
        device: Option<String>,
        #[serde(default)]
        room: Option<String>,
        #[serde(default)]
        on: Option<bool>,
    },
    DeviceAction {
        device: String,
        #[serde(default)]
        action: Option<String>,
    },
    TimerFired {
        #[serde(default)]
        name: Option<String>,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConditionDto {
    TimeOfDay {
        #[serde(default)]
        after: Option<String>,
        #[serde(default)]
        before: Option<String>,
    },
    DeviceIs {
        device: String,
        on: bool,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "do", rename_all = "snake_case")]
pub enum ActionDto {
    SetDevice {
        device: String,
        #[serde(default)]
        on: Option<bool>,
        #[serde(default)]
        brightness: Option<u8>,
        #[serde(default)]
        kelvin: Option<u16>,
    },
    Notify {
        body: String,
        #[serde(default)]
        room: Option<String>,
        #[serde(default)]
        priority: Option<String>,
    },
}

fn default_true() -> bool {
    true
}
