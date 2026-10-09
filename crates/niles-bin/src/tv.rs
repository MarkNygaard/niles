//! The TV: what voice and the model can ask of it.
//!
//! Read from `[tv]` on every call, so pairing it in the app, or giving
//! it a new address, takes hold at once. Apps and inputs are matched by
//! the name said against what the TV itself lists — "DR TV" is the app
//! the TV calls DRTV — rather than a table of ids that goes stale.

use niles_config::ConfigStore;
use niles_tools::{Result as ToolResult, Tool, ToolDescriptor, ToolRegistry};
use niles_webos::{App, Input, Tv};
use serde_json::{Value, json};
use std::sync::Arc;

pub struct TvControl {
    config: Arc<ConfigStore>,
}

impl TvControl {
    pub fn new(config: Arc<ConfigStore>) -> Self {
        Self { config }
    }

    /// The TV, when there is one set up and paired.
    fn tv(&self) -> Result<Tv, String> {
        let cfg = self.config.current();
        if !cfg.tv.is_configured() {
            return Err("There's no TV set up.".into());
        }
        let key = cfg
            .tv
            .resolve_client_key()
            .map_err(|_| "The TV isn't paired with Niles yet.".to_string())?;
        Ok(Tv::new(cfg.tv.host.trim(), key))
    }

    /// Whether Niles's announcements should also go on the screen.
    pub fn shows_announcements(&self) -> bool {
        let cfg = self.config.current();
        cfg.tv.is_configured() && cfg.tv.show_announcements
    }

    pub async fn power(&self, on: bool) -> Result<String, String> {
        if on {
            let cfg = self.config.current();
            if !cfg.tv.is_configured() {
                return Err("There's no TV set up.".into());
            }
            let mac = cfg.tv.mac.trim();
            if mac.is_empty() {
                return Err("I don't know the TV's MAC address, so I can't wake it.".into());
            }
            niles_webos::wake(cfg.tv.host.trim(), mac)
                .await
                .map_err(|e| e.to_string())?;
            return Ok("Turning on the TV.".into());
        }
        self.tv()?.turn_off().await.map_err(|e| e.to_string())?;
        Ok("TV off.".into())
    }

    pub async fn open(&self, asked: &str) -> Result<String, String> {
        let tv = self.tv()?;
        let apps = tv.apps().await.map_err(|e| e.to_string())?;
        let Some(app) = best_app(&apps, asked) else {
            return Err(format!("The TV has no app called {asked}."));
        };
        tv.launch(&app.id).await.map_err(|e| e.to_string())?;
        Ok(format!("{} on the TV.", app.title))
    }

    pub async fn input(&self, asked: &str) -> Result<String, String> {
        let tv = self.tv()?;
        let inputs = tv.inputs().await.map_err(|e| e.to_string())?;
        let Some(input) = best_input(&inputs, asked) else {
            return Err(format!("The TV has no input called {asked}."));
        };
        tv.switch_input(&input.id)
            .await
            .map_err(|e| e.to_string())?;
        Ok(format!("TV to {}.", input.label))
    }

    pub async fn playback(&self, play: bool) -> Result<String, String> {
        let tv = self.tv()?;
        if play {
            tv.play().await.map_err(|e| e.to_string())?;
            Ok("Playing on the TV.".into())
        } else {
            tv.pause().await.map_err(|e| e.to_string())?;
            Ok("Paused on the TV.".into())
        }
    }

    /// A message on the screen. Quietly nothing when the TV is off: an
    /// announcement has already been spoken, and the screen is extra.
    pub async fn show(&self, message: &str) -> Result<(), String> {
        self.tv()?.show(message).await.map_err(|e| e.to_string())
    }

    /// On or off, and what is on screen.
    pub async fn status(&self) -> Result<Value, String> {
        let status = self.tv()?.status().await.map_err(|e| e.to_string())?;
        Ok(json!(status))
    }
}

/// Lowercase letters and digits only: "DR TV" and "DRTV" are one name.
fn squashed(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// The app called what was said, or failing that the one whose name
/// holds it ("prime" is Prime Video).
fn best_app<'a>(apps: &'a [App], asked: &str) -> Option<&'a App> {
    let asked = squashed(asked);
    if asked.is_empty() {
        return None;
    }
    apps.iter()
        .find(|a| squashed(&a.title) == asked)
        .or_else(|| apps.iter().find(|a| squashed(&a.title).contains(&asked)))
}

/// The input by its label or its id: "hdmi 2" is "HDMI 2" and "HDMI_2",
/// and "playstation" is the HDMI somebody named so.
fn best_input<'a>(inputs: &'a [Input], asked: &str) -> Option<&'a Input> {
    let asked = squashed(asked);
    if asked.is_empty() {
        return None;
    }
    inputs
        .iter()
        .find(|i| squashed(&i.label) == asked || squashed(&i.id) == asked)
        .or_else(|| inputs.iter().find(|i| squashed(&i.label).contains(&asked)))
}

// ---- Tools ------------------------------------------------------------------

struct TvTool {
    tv: Arc<TvControl>,
    name: &'static str,
    description: &'static str,
    properties: Value,
    required: &'static [&'static str],
}

#[async_trait::async_trait]
impl Tool for TvTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: self.name.into(),
            description: self.description.into(),
            parameters: json!({
                "type": "object",
                "properties": self.properties,
                "required": self.required,
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> ToolResult<Value> {
        let text = |key: &str| {
            args.get(key)
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string()
        };
        let result = match self.name {
            "tv_power" => {
                self.tv
                    .power(args.get("on").and_then(Value::as_bool).unwrap_or(false))
                    .await
            }
            "tv_open_app" => self.tv.open(&text("app")).await,
            "tv_input" => self.tv.input(&text("input")).await,
            "tv_playback" => self.tv.playback(text("action") != "pause").await,
            "tv_message" => self
                .tv
                .show(&text("message"))
                .await
                .map(|()| "Shown on the TV.".into()),
            _ => {
                return Ok(self
                    .tv
                    .status()
                    .await
                    .unwrap_or_else(|e| json!({ "error": e })));
            }
        };
        Ok(match result {
            Ok(done) => json!({ "done": done }),
            Err(e) => json!({ "error": e }),
        })
    }
}

pub fn register(reg: &mut ToolRegistry, tv: Arc<TvControl>) {
    let tools: [(&str, &str, Value, &[&str]); 6] = [
        (
            "tv_power",
            "Turn the LG TV on or off.",
            json!({ "on": { "type": "boolean" } }),
            &["on"],
        ),
        (
            "tv_open_app",
            "Open an app on the TV by name: Netflix, YouTube, DR TV, TV 2 Play, Disney+.",
            json!({ "app": { "type": "string" } }),
            &["app"],
        ),
        (
            "tv_input",
            "Switch the TV to an input: \"HDMI 2\", or a name the input was given.",
            json!({ "input": { "type": "string" } }),
            &["input"],
        ),
        (
            "tv_playback",
            "Pause or play what an app on the TV is playing. The TV's sound is the \
             living room Sonos; for its volume use the music tools.",
            json!({ "action": { "type": "string", "enum": ["play", "pause"] } }),
            &["action"],
        ),
        (
            "tv_message",
            "Show a short message on the TV screen, over whatever is on.",
            json!({ "message": { "type": "string" } }),
            &["message"],
        ),
        (
            "tv_status",
            "Whether the TV is on, and which app or input is on screen.",
            json!({}),
            &[],
        ),
    ];
    for (name, description, properties, required) in tools {
        reg.register(Box::new(TvTool {
            tv: tv.clone(),
            name,
            description,
            properties,
            required,
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str, title: &str) -> App {
        App {
            id: id.into(),
            title: title.into(),
        }
    }

    #[test]
    fn an_app_is_found_by_the_name_said() {
        let apps = [
            app("netflix", "Netflix"),
            app("dk.dr.drtv", "DRTV"),
            app("amazon", "Prime Video"),
        ];
        assert_eq!(best_app(&apps, "netflix").unwrap().id, "netflix");
        assert_eq!(best_app(&apps, "dr tv").unwrap().id, "dk.dr.drtv");
        assert_eq!(best_app(&apps, "prime").unwrap().id, "amazon");
        assert!(best_app(&apps, "hbo").is_none());
    }

    #[test]
    fn an_input_is_found_by_label_or_id() {
        let inputs = [
            Input {
                id: "HDMI_1".into(),
                label: "PlayStation".into(),
            },
            Input {
                id: "HDMI_2".into(),
                label: "HDMI 2".into(),
            },
        ];
        assert_eq!(best_input(&inputs, "hdmi 2").unwrap().id, "HDMI_2");
        assert_eq!(best_input(&inputs, "hdmi 1").unwrap().id, "HDMI_1");
        assert_eq!(best_input(&inputs, "playstation").unwrap().id, "HDMI_1");
    }

    #[tokio::test]
    async fn no_tv_set_up_is_said_plainly() {
        let config = Arc::new(ConfigStore::from_str_in_memory("").unwrap());
        let tv = TvControl::new(config);
        assert_eq!(tv.power(false).await.unwrap_err(), "There's no TV set up.");
        assert!(!tv.shows_announcements());
    }
}
