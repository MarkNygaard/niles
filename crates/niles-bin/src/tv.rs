//! The TV: what voice and the model can ask of it.
//!
//! Read from `[tv]` on every call, so pairing it in the app, or giving
//! it a new address, takes hold at once. Apps and inputs are matched by
//! the name said against what the TV itself lists — "DR TV" is the app
//! the TV calls DRTV — rather than a table of ids that goes stale.

use niles_config::ConfigStore;
use niles_core::RoomName;
use niles_tools::{Result as ToolResult, Tool, ToolDescriptor, ToolRegistry};
use niles_webos::{App, Input, Tv, Volume, VolumeChange};
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

    /// The room the TV stands in, when it was given one.
    pub fn room(&self) -> Option<RoomName> {
        RoomName::parse(self.config.current().tv.room.trim()).ok()
    }

    /// Change the TV's own sound. The error is what to say: no TV, not
    /// paired, or off.
    pub async fn change_volume(&self, change: VolumeChange) -> Result<Volume, String> {
        match self.tv()?.change_volume(change).await {
            Ok(Some(volume)) => Ok(volume),
            Ok(None) => Err("The TV is off.".into()),
            Err(e) => Err(e.to_string()),
        }
    }

    /// The same, said.
    pub async fn volume(&self, change: VolumeChange) -> Result<String, String> {
        self.change_volume(change)
            .await
            .map(|after| said(change, after))
    }

    /// A room's volume on the TV, when the TV is the room's sound: it
    /// stands there, set up and paired, and the room has no Sonos.
    /// `None` leaves the room to its speakers, or to having none — a
    /// room with Sonos keeps its volume on them, TV or not.
    pub async fn room_volume(
        &self,
        room: &RoomName,
        has_speakers: bool,
        change: VolumeChange,
    ) -> Option<Result<Volume, String>> {
        if has_speakers || self.room().as_ref() != Some(room) || self.tv().is_err() {
            return None;
        }
        Some(self.change_volume(change).await)
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

/// "TV volume to 20." / "TV muted."
pub fn said(change: VolumeChange, after: Volume) -> String {
    match change {
        VolumeChange::Mute(true) => "TV muted.".into(),
        VolumeChange::Mute(false) => format!("TV unmuted, volume {}.", after.volume),
        _ if after.muted => format!("TV volume to {}. It's muted.", after.volume),
        _ => format!("TV volume to {}.", after.volume),
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
            "tv_volume" => {
                let Some(change) = asked_change(&args) else {
                    return Ok(
                        json!({ "error": "say set with a level, up, down, mute or unmute" }),
                    );
                };
                return Ok(match self.tv.change_volume(change).await {
                    Ok(after) => json!(after),
                    Err(e) => json!({ "error": e }),
                });
            }
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

/// What `tv_volume` was asked to do.
fn asked_change(args: &Value) -> Option<VolumeChange> {
    Some(match args.get("action").and_then(Value::as_str)? {
        "set" => VolumeChange::To(args.get("level").and_then(Value::as_u64)?.min(100) as u8),
        "up" => VolumeChange::By(SPEAKER_STEP),
        "down" => VolumeChange::By(-SPEAKER_STEP),
        "mute" => VolumeChange::Mute(true),
        "unmute" => VolumeChange::Mute(false),
        _ => return None,
    })
}

/// What "louder" moves a Sonos by, so the TV answers it the same.
const SPEAKER_STEP: i16 = 10;

pub fn register(reg: &mut ToolRegistry, tv: Arc<TvControl>) {
    let tools: [(&str, &str, Value, &[&str]); 7] = [
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
            "Pause or play what an app on the TV is playing. For its volume, \
             tv_volume.",
            json!({ "action": { "type": "string", "enum": ["play", "pause"] } }),
            &["action"],
        ),
        (
            "tv_volume",
            "The LG TV's own speakers: set the volume to a level (0-100), up or down a \
             step, mute or unmute. Answers the volume after. In a room with a Sonos the \
             TV's sound is the Sonos: use music_volume for that room instead.",
            json!({
                "action": { "type": "string", "enum": ["set", "up", "down", "mute", "unmute"] },
                "level": { "type": "integer", "minimum": 0, "maximum": 100 }
            }),
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
                app_id: None,
            },
            Input {
                id: "HDMI_2".into(),
                label: "HDMI 2".into(),
                app_id: None,
            },
        ];
        assert_eq!(best_input(&inputs, "hdmi 2").unwrap().id, "HDMI_2");
        assert_eq!(best_input(&inputs, "hdmi 1").unwrap().id, "HDMI_1");
        assert_eq!(best_input(&inputs, "playstation").unwrap().id, "HDMI_1");
    }

    /// A TV paired and standing in the office. Nothing listens on
    /// loopback for it, which is how a TV that is off looks.
    fn tv_in_the_office() -> TvControl {
        unsafe { std::env::set_var("NILES_TEST_TV_VOLUME_KEY", "key") };
        TvControl::new(Arc::new(
            ConfigStore::from_str_in_memory(
                "[tv]\nhost = \"127.0.0.1\"\nroom = \"office\"\n\
                 client_key_env = \"NILES_TEST_TV_VOLUME_KEY\"\n",
            )
            .unwrap(),
        ))
    }

    fn room(name: &str) -> RoomName {
        RoomName::parse(name).unwrap()
    }

    #[tokio::test]
    async fn a_room_with_sonos_keeps_its_volume_on_them() {
        let tv = tv_in_the_office();
        assert!(
            tv.room_volume(&room("office"), true, VolumeChange::To(20))
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn a_room_with_only_the_tv_turns_the_tv_and_says_when_it_is_off() {
        let tv = tv_in_the_office();
        assert_eq!(
            tv.room_volume(&room("office"), false, VolumeChange::By(10))
                .await,
            Some(Err("The TV is off.".to_string()))
        );
    }

    #[tokio::test]
    async fn a_room_with_neither_is_left_alone() {
        let tv = tv_in_the_office();
        assert!(
            tv.room_volume(&room("kitchen"), false, VolumeChange::To(20))
                .await
                .is_none()
        );
        let none = TvControl::new(Arc::new(ConfigStore::from_str_in_memory("").unwrap()));
        assert!(
            none.room_volume(&room("office"), false, VolumeChange::To(20))
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn a_tv_not_yet_paired_is_not_the_room_sound() {
        let tv = TvControl::new(Arc::new(
            ConfigStore::from_str_in_memory(
                "[tv]\nhost = \"127.0.0.1\"\nroom = \"office\"\n\
                 client_key_env = \"NILES_TEST_TV_VOLUME_UNSET\"\n",
            )
            .unwrap(),
        ));
        assert!(
            tv.room_volume(&room("office"), false, VolumeChange::To(20))
                .await
                .is_none()
        );
    }

    #[test]
    fn the_tv_volume_is_said_plainly() {
        let at = |volume, muted| Volume {
            volume,
            muted,
            sound_output: None,
        };
        assert_eq!(
            said(VolumeChange::To(20), at(20, false)),
            "TV volume to 20."
        );
        assert_eq!(
            said(VolumeChange::By(-10), at(10, false)),
            "TV volume to 10."
        );
        assert_eq!(
            said(VolumeChange::By(10), at(30, true)),
            "TV volume to 30. It's muted."
        );
        assert_eq!(said(VolumeChange::Mute(true), at(30, true)), "TV muted.");
        assert_eq!(
            said(VolumeChange::Mute(false), at(30, false)),
            "TV unmuted, volume 30."
        );
    }

    #[test]
    fn the_volume_tool_reads_what_it_was_asked() {
        let asked = |args: Value| asked_change(&args);
        assert_eq!(
            asked(json!({ "action": "set", "level": 140 })),
            Some(VolumeChange::To(100))
        );
        assert_eq!(asked(json!({ "action": "set" })), None);
        assert_eq!(
            asked(json!({ "action": "down" })),
            Some(VolumeChange::By(-10))
        );
        assert_eq!(
            asked(json!({ "action": "unmute" })),
            Some(VolumeChange::Mute(false))
        );
        assert_eq!(asked(json!({ "action": "louder" })), None);
    }

    #[tokio::test]
    async fn the_volume_tool_says_when_the_tv_is_off() {
        let tool = TvTool {
            tv: Arc::new(tv_in_the_office()),
            name: "tv_volume",
            description: "",
            properties: json!({}),
            required: &[],
        };
        let answer = tool.execute(json!({ "action": "mute" })).await.unwrap();
        assert_eq!(answer, json!({ "error": "The TV is off." }));
    }

    #[tokio::test]
    async fn no_tv_set_up_is_said_plainly() {
        let config = Arc::new(ConfigStore::from_str_in_memory("").unwrap());
        let tv = TvControl::new(config);
        assert_eq!(tv.power(false).await.unwrap_err(), "There's no TV set up.");
        assert!(!tv.shows_announcements());
    }
}
