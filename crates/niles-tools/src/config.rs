//! Tools that let Niles change its own configuration when asked.
//!
//! "Make the evenings brighter" should adjust the curve, not just move a
//! light. These are the LLM's route into the [`ConfigStore`] — the same
//! store the HTTP API and the config UI write through.
//!
//! # Why voice can't write everything
//!
//! A section is only worth changing by voice if the running process will
//! pick it up. Almost nothing in Niles does: an MQTT connection, a
//! spawned poll loop, an embedded model are each built once at startup
//! and can't be re-pointed by writing a new value. Writing to one of
//! those over voice would mean answering "done" to a request that
//! changes nothing until someone restarts the pod — so [`UpdateConfig`]
//! refuses them and says which sections it can change.
//!
//! The HTTP API *does* accept those writes, because a UI can show a
//! "restart required" badge next to the field. A spoken answer has no
//! such affordance.
//!
//! # Relative changes
//!
//! "Brighter" is relative, so the model needs to read before it writes.
//! [`GetConfig`] exists for that, and every write answers with the
//! before-and-after values so the spoken confirmation can name real
//! numbers rather than repeating the request.

use crate::error::{Error, Result};
use crate::registry::ToolRegistry;
use crate::tool::{Tool, ToolDescriptor};
use async_trait::async_trait;
use niles_config::{Applied, ChangeSource, ConfigStore, Reload, section_reload};
use serde_json::{Value, json};
use std::sync::Arc;

/// Read the current configuration, whole or by section.
pub struct GetConfig {
    store: Arc<ConfigStore>,
}

impl GetConfig {
    pub fn new(store: Arc<ConfigStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for GetConfig {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "get_config".into(),
            description: "Read Niles's current configuration — the lighting curve, \
                 morning routine, and every other setting. Call this before changing \
                 a value the user described in relative terms ('brighter', 'later', \
                 'a bit dimmer') so the new value is based on what is actually set. \
                 Returns the effective values, which section each belongs to, and \
                 which values have been changed away from the config file."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "section": {
                        "type": "string",
                        "description": "Limit to one top-level section, e.g. 'lighting'. Omit for everything."
                    }
                },
                "required": [],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let effective = self.store.effective_table();
        let overrides = self.store.overrides();

        let body = match args.get("section").and_then(|v| v.as_str()) {
            Some(section) => {
                let Some(values) = effective.get(section) else {
                    return Err(Error::InvalidArgs {
                        tool: "get_config".into(),
                        reason: format!(
                            "no config section '{section}'; known sections: {}",
                            section_list(&effective)
                        ),
                    });
                };
                json!({
                    "section": section,
                    "values": to_json(values)?,
                    "overridden": overrides.get(section).map(to_json).transpose()?,
                    "changeable_by_voice": section_reload(section) == Reload::Hot,
                })
            }
            None => json!({
                "config": to_json(&effective)?,
                "overridden": to_json(&overrides)?,
                "changeable_by_voice": hot_sections(&effective),
            }),
        };
        Ok(body)
    }
}

/// Change a configuration value.
pub struct UpdateConfig {
    store: Arc<ConfigStore>,
}

impl UpdateConfig {
    pub fn new(store: Arc<ConfigStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for UpdateConfig {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "update_config".into(),
            description: "Change one Niles configuration value, e.g. how bright the \
                 lights sit during the day or when the morning ramp starts. Use \
                 get_config first when the user's request is relative ('brighter', \
                 'half an hour earlier'). The change takes effect immediately and \
                 persists. Returns the old and new values — tell the user what \
                 actually changed rather than repeating their request back."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Dotted path to the value, e.g. 'lighting.daytime_brightness' or 'lighting.sunset_start'."
                    },
                    "value": {
                        "description": "The new value. Use the same type the setting already has: a number for brightness, a 'HH:MM' string for a time."
                    }
                },
                "required": ["path", "value"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let path = str_arg(&args, "update_config", "path")?;
        let value = args.get("value").ok_or_else(|| Error::InvalidArgs {
            tool: "update_config".into(),
            reason: "missing 'value'".into(),
        })?;

        let section = path.split('.').next().unwrap_or(&path);
        if section_reload(section) != Reload::Hot {
            // Storing it would be honest only if we could also say "and
            // nothing will happen until the pod restarts", which is not a
            // useful thing to hear out loud.
            return Err(Error::InvalidArgs {
                tool: "update_config".into(),
                reason: format!(
                    "'{section}' is only read when Niles starts, so changing it by voice \
                     would not take effect; it can be edited in the config file or the \
                     config UI. Sections that can be changed now: {}",
                    hot_sections(&self.store.effective_table()).join(", ")
                ),
            });
        }

        let patch = patch_from_path(&path, value)?;
        let applied =
            self.store
                .apply(&patch, ChangeSource::Voice)
                .map_err(|e| Error::InvalidArgs {
                    tool: "update_config".into(),
                    reason: e.to_string(),
                })?;
        Ok(applied_json(&applied))
    }
}

/// Put a value back to what the config file says.
pub struct ResetConfig {
    store: Arc<ConfigStore>,
}

impl ResetConfig {
    pub fn new(store: Arc<ConfigStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for ResetConfig {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "reset_config".into(),
            description: "Return one configuration value to the default from the \
                 config file, discarding any change made to it. Use for 'put the \
                 brightness back to normal' or 'forget what I said about the \
                 morning ramp'. For 'undo that', prefer undo_config_change."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Dotted path, e.g. 'lighting.daytime_brightness'."
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let path = str_arg(&args, "reset_config", "path")?;
        let applied =
            self.store
                .reset(&path, ChangeSource::Voice)
                .map_err(|e| Error::InvalidArgs {
                    tool: "reset_config".into(),
                    reason: e.to_string(),
                })?;
        if applied.is_noop() {
            return Ok(json!({
                "changed": false,
                "message": format!("'{path}' was already at its configured default"),
            }));
        }
        Ok(applied_json(&applied))
    }
}

/// Walk back the most recent configuration change.
pub struct UndoConfigChange {
    store: Arc<ConfigStore>,
}

impl UndoConfigChange {
    pub fn new(store: Arc<ConfigStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for UndoConfigChange {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "undo_config_change".into(),
            description: "Undo the most recent configuration change, whoever made it. \
                 Use for 'undo that', 'no, put it back', 'that's too dark, revert'. \
                 Calling it twice walks back two changes. Returns what was reverted."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, _args: Value) -> Result<Value> {
        match self.store.undo() {
            Ok(Some(applied)) => Ok(applied_json(&applied)),
            Ok(None) => Ok(json!({
                "changed": false,
                "message": "there are no configuration changes to undo",
            })),
            Err(e) => Err(Error::InvalidArgs {
                tool: "undo_config_change".into(),
                reason: e.to_string(),
            }),
        }
    }
}

/// Register the config tools. Only called when a store exists.
pub fn register_config_tools(reg: &mut ToolRegistry, store: Arc<ConfigStore>) {
    reg.register(Box::new(GetConfig::new(store.clone())));
    reg.register(Box::new(UpdateConfig::new(store.clone())));
    reg.register(Box::new(ResetConfig::new(store.clone())));
    reg.register(Box::new(UndoConfigChange::new(store)));
}

/// Turn `lighting.daytime_brightness` + a JSON value into the nested
/// patch the store merges.
fn patch_from_path(path: &str, value: &Value) -> Result<toml::Table> {
    let value: toml::Value =
        serde_json::from_value(value.clone()).map_err(|e| Error::InvalidArgs {
            tool: "update_config".into(),
            reason: format!("value is not representable in config: {e}"),
        })?;

    let mut segments: Vec<&str> = path.split('.').collect();
    let leaf = segments
        .pop()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error::InvalidArgs {
            tool: "update_config".into(),
            reason: format!("'{path}' is not a valid config path"),
        })?;
    if segments.iter().any(|s| s.is_empty()) {
        return Err(Error::InvalidArgs {
            tool: "update_config".into(),
            reason: format!("'{path}' is not a valid config path"),
        });
    }

    let mut table = toml::Table::new();
    table.insert(leaf.to_string(), value);
    for segment in segments.into_iter().rev() {
        let mut parent = toml::Table::new();
        parent.insert(segment.to_string(), toml::Value::Table(table));
        table = parent;
    }
    Ok(table)
}

/// The shape a spoken confirmation is built from: what changed, from
/// what, to what.
fn applied_json(applied: &Applied) -> Value {
    json!({
        "changed": !applied.is_noop(),
        "summary": applied.summary(),
        "changes": applied
            .changes
            .iter()
            .map(|c| json!({
                "path": c.path,
                "from": c.from.as_ref().and_then(|v| to_json(v).ok()),
                "to": to_json(&c.to).ok(),
            }))
            .collect::<Vec<_>>(),
        "revision": applied.revision,
    })
}

fn hot_sections(effective: &toml::Table) -> Vec<String> {
    effective
        .keys()
        .filter(|name| section_reload(name) == Reload::Hot)
        .cloned()
        .collect()
}

fn section_list(effective: &toml::Table) -> String {
    effective.keys().cloned().collect::<Vec<_>>().join(", ")
}

fn to_json<T: serde::Serialize>(value: &T) -> Result<Value> {
    Ok(serde_json::to_value(value)?)
}

fn str_arg(args: &Value, tool: &str, key: &str) -> Result<String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error::InvalidArgs {
            tool: tool.into(),
            reason: format!("missing or empty '{key}'"),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Arc<ConfigStore> {
        Arc::new(ConfigStore::from_str_in_memory(BASE).expect("fixture is a valid config"))
    }

    async fn update(store: &Arc<ConfigStore>, path: &str, value: Value) -> Result<Value> {
        UpdateConfig::new(store.clone())
            .execute(json!({"path": path, "value": value}))
            .await
    }

    #[tokio::test]
    async fn update_changes_a_value_and_reports_both_ends() {
        let store = store();
        let out = update(&store, "lighting.daytime_brightness", json!(85))
            .await
            .unwrap();
        assert_eq!(out["changed"], true);
        assert_eq!(out["summary"], "lighting.daytime_brightness 100 → 85");
        assert_eq!(out["changes"][0]["from"], 100);
        assert_eq!(out["changes"][0]["to"], 85);
        assert_eq!(store.current().lighting.daytime_brightness, 85);
    }

    #[tokio::test]
    async fn update_accepts_a_time_string() {
        let store = store();
        let out = update(&store, "lighting.sunset_start", json!("22:00"))
            .await
            .unwrap();
        assert_eq!(out["summary"], "lighting.sunset_start 21:30 → 22:00");
    }

    #[tokio::test]
    async fn update_refuses_a_section_the_running_process_would_ignore() {
        // Answering "done" to a change nothing will read until the pod
        // restarts is the failure this guard exists to prevent.
        let store = store();
        let err = update(&store, "mqtt.host", json!("10.0.0.1"))
            .await
            .expect_err("boot-only section must be refused");
        let message = err.to_string();
        assert!(message.contains("only read when Niles starts"), "{message}");
        assert!(message.contains("lighting"), "names what it can change");
        assert_eq!(store.current().mqtt.host, "192.168.42.16");
    }

    #[tokio::test]
    async fn update_refuses_a_value_the_config_rejects() {
        let store = store();
        let err = update(&store, "lighting.morning_start", json!("09:00"))
            .await
            .expect_err("invalid curve must be refused");
        assert!(err.to_string().contains("morning_start"));
    }

    #[tokio::test]
    async fn update_refuses_an_unknown_key() {
        let store = store();
        let err = update(&store, "lighting.brightnes", json!(85))
            .await
            .expect_err("typo must be refused");
        assert!(err.to_string().contains("brightnes"));
    }

    #[tokio::test]
    async fn update_reports_a_noop_rather_than_claiming_a_change() {
        let store = store();
        let out = update(&store, "lighting.daytime_brightness", json!(100))
            .await
            .unwrap();
        assert_eq!(out["changed"], false);
    }

    #[tokio::test]
    async fn update_rejects_a_malformed_path() {
        let store = store();
        assert!(update(&store, "", json!(1)).await.is_err());
        assert!(update(&store, "lighting..x", json!(1)).await.is_err());
    }

    #[tokio::test]
    async fn get_config_reports_a_section_and_whether_voice_can_change_it() {
        let store = store();
        let tool = GetConfig::new(store.clone());
        let out = tool.execute(json!({"section": "lighting"})).await.unwrap();
        assert_eq!(out["values"]["daytime_brightness"], 100);
        assert_eq!(out["changeable_by_voice"], true);

        let out = tool.execute(json!({"section": "mqtt"})).await.unwrap();
        assert_eq!(out["changeable_by_voice"], false);
    }

    #[tokio::test]
    async fn get_config_names_the_sections_when_asked_for_a_missing_one() {
        let store = store();
        let err = GetConfig::new(store)
            .execute(json!({"section": "lightning"}))
            .await
            .expect_err("unknown section");
        assert!(err.to_string().contains("lighting"), "suggests what exists");
    }

    #[tokio::test]
    async fn get_config_without_a_section_returns_everything() {
        let store = store();
        let out = GetConfig::new(store).execute(json!({})).await.unwrap();
        assert_eq!(out["config"]["lighting"]["daytime_brightness"], 100);
        assert!(
            out["changeable_by_voice"]
                .as_array()
                .unwrap()
                .contains(&json!("lighting"))
        );
    }

    #[tokio::test]
    async fn undo_reverts_the_last_change() {
        let store = store();
        update(&store, "lighting.daytime_brightness", json!(85))
            .await
            .unwrap();
        let out = UndoConfigChange::new(store.clone())
            .execute(json!({}))
            .await
            .unwrap();
        assert_eq!(out["changed"], true);
        assert_eq!(store.current().lighting.daytime_brightness, 100);
    }

    #[tokio::test]
    async fn undo_with_nothing_to_undo_says_so_instead_of_failing() {
        // The model should be able to tell the user plainly, not surface
        // a tool error.
        let store = store();
        let out = UndoConfigChange::new(store)
            .execute(json!({}))
            .await
            .unwrap();
        assert_eq!(out["changed"], false);
        assert!(
            out["message"]
                .as_str()
                .unwrap()
                .contains("no configuration")
        );
    }

    #[tokio::test]
    async fn reset_returns_a_value_to_the_file_default() {
        let store = store();
        update(&store, "lighting.daytime_brightness", json!(85))
            .await
            .unwrap();
        let out = ResetConfig::new(store.clone())
            .execute(json!({"path": "lighting.daytime_brightness"}))
            .await
            .unwrap();
        assert_eq!(out["changed"], true);
        assert_eq!(store.current().lighting.daytime_brightness, 100);
    }

    #[tokio::test]
    async fn reset_of_an_unchanged_value_says_so() {
        let store = store();
        let out = ResetConfig::new(store)
            .execute(json!({"path": "lighting.daytime_brightness"}))
            .await
            .unwrap();
        assert_eq!(out["changed"], false);
    }

    #[test]
    fn patch_from_path_nests_by_segment() {
        let patch = patch_from_path("lighting.daytime_brightness", &json!(85)).unwrap();
        assert_eq!(
            patch["lighting"].as_table().unwrap()["daytime_brightness"].as_integer(),
            Some(85)
        );
    }

    const BASE: &str = r#"
[home]
name = "test home"
latitude = 56.1572
longitude = 10.2107
timezone = "Europe/Copenhagen"

[mqtt]
host = "192.168.42.16"
port = 1883
username_env = "NILES_MQTT_USERNAME"
password_env = "NILES_MQTT_PASSWORD"

[api]
bind_address = "0.0.0.0:8080"

[wyoming]
bind_address = "0.0.0.0:10300"

[stt]
api_key_env = "GROQ_API_KEY"

[tts]

[llm]
api_key_env = "GROQ_API_KEY"

[lighting]
morning_start = "05:45"
morning_end = "06:30"
sunset_start = "21:30"
sunset_end = "23:00"
night_floor_brightness = 15
daytime_brightness = 100

[[lighting.color_temp_anchors]]
time = "00:00"
kelvin = 2000

[[lighting.color_temp_anchors]]
time = "12:00"
kelvin = 4500

[[lighting.color_temp_anchors]]
time = "23:59"
kelvin = 2000
"#;
}
