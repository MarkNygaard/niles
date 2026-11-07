//! `apply_scene`: a saved scene, applied by the model.
//!
//! The fixed phrases ("movie scene", "turn on movie") apply a scene
//! fastest, but only when the sentence is one of them. Everything else
//! reaches the model, which understood "turn on the scene movie" and,
//! with no tool for it, coached the phrasing or said it was done when
//! nothing was. The description names the scenes saved now, because a
//! scene is asked for by its name and not by the word "scene".

use crate::builtin::Publisher;
use crate::error::{Error, Result};
use crate::tool::{Tool, ToolDescriptor};
use async_trait::async_trait;
use niles_core::RoomName;
use niles_mqtt::{CommandRouter, MqttPublisher};
use niles_scheduler::{ManualModeTracker, SceneStore};
use serde_json::{Value, json};
use std::sync::Arc;

pub struct ApplyScene<P: Publisher = MqttPublisher> {
    scenes: Arc<SceneStore>,
    publisher: P,
    router: Arc<CommandRouter>,
    manual: Option<Arc<ManualModeTracker>>,
    dry_run: bool,
}

impl<P: Publisher> ApplyScene<P> {
    pub fn new(
        scenes: Arc<SceneStore>,
        publisher: P,
        router: Arc<CommandRouter>,
        manual: Option<Arc<ManualModeTracker>>,
        dry_run: bool,
    ) -> Self {
        Self {
            scenes,
            publisher,
            router,
            manual,
            dry_run,
        }
    }
}

#[async_trait]
impl<P: Publisher> Tool for ApplyScene<P> {
    fn offered(&self) -> bool {
        !self.scenes.names().is_empty()
    }

    fn descriptor(&self) -> ToolDescriptor {
        let saved = self.scenes.names().join(", ");
        ToolDescriptor {
            name: "apply_scene".into(),
            description: format!(
                "Apply a saved lighting scene: put its lights back the way they were saved. \
                 A scene is usually asked for by its name alone (\"turn on movie\", \"movie time\", \
                 \"the cosy one\"). Saved scenes: {saved}."
            ),
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "One of the saved scenes."
                    },
                    "room": {
                        "type": "string",
                        "description": "Only this room's lights from the scene, when one was said. Canonical room name, lower_snake, e.g. 'living_room'."
                    }
                },
                "required": ["name"]
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let name = args
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .ok_or_else(|| Error::InvalidArgs {
                tool: "apply_scene".into(),
                reason: "name must be one of the saved scenes".into(),
            })?;
        let Some(mut entries) = self.scenes.get(name) else {
            return Ok(json!({
                "applied": false,
                "note": format!("there is no scene called {name}"),
                "saved": self.scenes.names(),
            }));
        };
        if let Some(raw) = args.get("room").and_then(Value::as_str) {
            let room =
                RoomName::parse(raw).map_err(|_| Error::RoomNotFound { name: raw.into() })?;
            entries.retain(|e| e.device_id.room() == &room);
            if entries.is_empty() {
                return Ok(json!({
                    "applied": false,
                    "note": format!("{name} has no lights in {raw}"),
                }));
            }
        }
        if entries.is_empty() {
            return Ok(json!({
                "applied": false,
                "note": format!("{name} was saved with nothing in it"),
            }));
        }

        let mut set = Vec::new();
        let mut skipped = Vec::new();
        for entry in entries {
            let id = entry.device_id.to_string();
            // Before the command, as the spoken scene does: the curve
            // ticks once a minute and would otherwise undo it in between.
            if let Some(manual) = &self.manual {
                manual.flag(&entry.device_id);
            }
            if let Some(commander) = self.router.commander_for(&entry.device_id, &entry.state) {
                if self.dry_run {
                    tracing::info!("[dry-run] would command {id} {:?}", entry.state);
                } else if let Err(e) = commander.command(&entry.device_id, &entry.state).await {
                    tracing::warn!("{id} did not take the scene: {e}");
                    skipped.push(id);
                    continue;
                }
                set.push(id);
                continue;
            }
            let Some((topic, payload)) = self.router.format(&entry.device_id, &entry.state) else {
                skipped.push(id);
                continue;
            };
            if self.dry_run {
                tracing::info!("[dry-run] would publish {topic} {payload}");
            } else {
                self.publisher.publish(&topic, payload.into_bytes()).await?;
            }
            set.push(id);
        }
        tracing::info!("applied scene {name:?} to {} devices", set.len());
        let mut out = json!({ "applied": true, "scene": name, "set": set });
        if !skipped.is_empty() {
            out["skipped"] = json!(skipped);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use niles_core::DeviceId;

    #[derive(Clone, Default)]
    struct Published(Arc<tokio::sync::Mutex<Vec<String>>>);

    #[async_trait]
    impl Publisher for Published {
        async fn publish(&self, topic: &str, _payload: Vec<u8>) -> niles_mqtt::Result<()> {
            self.0.lock().await.push(topic.to_string());
            Ok(())
        }
    }

    fn movie() -> Arc<SceneStore> {
        let document = json!({ "scenes": { "movie": [
            { "device_id": "z2m:living_room/ceiling", "state": { "on": true, "brightness": 20 } },
            { "device_id": "z2m:living_room/floor_lamp", "state": { "on": false } },
            { "device_id": "z2m:kitchen/spots", "state": { "on": false } },
        ] } });
        Arc::new(SceneStore::from_json(&document.to_string()).expect("a valid scenes document"))
    }

    fn tool(scenes: Arc<SceneStore>) -> (Published, Arc<ManualModeTracker>, ApplyScene<Published>) {
        let published = Published::default();
        let manual = Arc::new(ManualModeTracker::new());
        let tool = ApplyScene::new(
            scenes,
            published.clone(),
            Arc::new(CommandRouter::z2m_only("z2m")),
            Some(manual.clone()),
            false,
        );
        (published, manual, tool)
    }

    #[tokio::test]
    async fn a_saved_scene_is_applied_and_held_against_the_curve() {
        let (published, manual, tool) = tool(movie());
        let result = tool.execute(json!({ "name": "movie" })).await.unwrap();
        assert_eq!(result["applied"], true);
        assert_eq!(published.0.lock().await.len(), 3);
        let ceiling = DeviceId::parse("z2m:living_room/ceiling").unwrap();
        assert!(manual.is_flagged(&ceiling), "the curve must not undo it");
    }

    #[tokio::test]
    async fn a_room_takes_only_that_rooms_lights() {
        let (published, _, tool) = tool(movie());
        let result = tool
            .execute(json!({ "name": "Movie", "room": "kitchen" }))
            .await
            .unwrap();
        assert_eq!(result["applied"], true);
        assert_eq!(*published.0.lock().await, vec!["z2m/kitchen/spots/set"]);
    }

    #[tokio::test]
    async fn an_unknown_name_is_answered_with_the_saved_ones() {
        let (published, _, tool) = tool(movie());
        let result = tool.execute(json!({ "name": "cinema" })).await.unwrap();
        assert_eq!(result["applied"], false);
        assert_eq!(result["saved"], json!(["movie"]));
        assert!(published.0.lock().await.is_empty());
    }

    #[test]
    fn the_model_is_told_which_scenes_exist_and_offered_none_without_them() {
        let (_, _, with) = tool(movie());
        assert!(with.offered());
        assert!(
            with.descriptor()
                .description
                .contains("Saved scenes: movie.")
        );
        let (_, _, without) = tool(Arc::new(SceneStore::new()));
        assert!(!without.offered());
    }
}
