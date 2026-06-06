//! WLED device source — consumes MQTT messages and populates a
//! `niles_core::DeviceRegistry` while publishing change events on the bus.
//!
//! Unlike `Z2mSource`, WLED has no `bridge/devices` equivalent. Devices are
//! declared in config and upserted at startup, then state updates arrive on
//! per-device topics.

use crate::Result;
use crate::client::{Message, MqttClient};
use crate::wled::{parse_c, parse_g, parse_status};
use niles_core::{Device, DeviceClass, DeviceId, DeviceRegistry, DeviceState, EventBus};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tracing::{debug, warn};

/// Consumes WLED MQTT messages and keeps a shared `DeviceRegistry` in sync.
pub struct WledSource {
    client: MqttClient,
    registry: Arc<DeviceRegistry>,
    bus: EventBus,
    devices: Vec<(DeviceId, String)>, // (id, base_topic)
    ambient_lights: Arc<HashSet<DeviceId>>,
}

impl WledSource {
    pub fn new(
        client: MqttClient,
        registry: Arc<DeviceRegistry>,
        bus: EventBus,
        devices: Vec<(DeviceId, String)>,
        ambient_lights: Arc<HashSet<DeviceId>>,
    ) -> Self {
        Self {
            client,
            registry,
            bus,
            devices,
            ambient_lights,
        }
    }

    /// Subscribe to WLED topics and run the message loop until the underlying
    /// MQTT client disconnects.
    pub async fn run(mut self) -> Result<()> {
        // Upsert all configured devices first so state messages never race.
        let mut topic_index = HashMap::new();
        for (id, topic) in &self.devices {
            let mut device = Device::new(id.clone(), DeviceState::default(), DeviceClass::Light);
            device.is_ambient = self.ambient_lights.contains(id);
            self.registry.upsert(device.clone());
            self.bus.publish(niles_core::Event::DeviceAdded { device });

            topic_index.insert(format!("{topic}/g"), id.clone());
            topic_index.insert(format!("{topic}/c"), id.clone());
            topic_index.insert(format!("{topic}/status"), id.clone());

            self.client.subscribe(&format!("{topic}/g")).await?;
            self.client.subscribe(&format!("{topic}/c")).await?;
            self.client.subscribe(&format!("{topic}/status")).await?;
        }

        while let Some(msg) = self.client.next_message().await {
            dispatch_wled(&msg, &topic_index, &self.registry, &self.bus);
        }
        Ok(())
    }
}

/// Route an incoming WLED message to the right handler.
pub(crate) fn dispatch_wled(
    msg: &Message,
    topic_index: &HashMap<String, DeviceId>,
    registry: &DeviceRegistry,
    bus: &EventBus,
) {
    let Some(id) = topic_index.get(&msg.topic) else {
        debug!("wled: unknown topic {}", msg.topic);
        return;
    };

    if msg.topic.ends_with("/g") || msg.topic.ends_with("/c") {
        let partial = if msg.topic.ends_with("/g") {
            parse_g(&msg.payload)
        } else {
            parse_c(&msg.payload)
        };
        let Some(partial) = partial else {
            return;
        };
        if !registry.merge_state(id, partial) {
            debug!("wled: merge_state miss for {id} (device not in registry)");
            return;
        }
        bus.publish(niles_core::Event::DeviceStateChanged {
            id: id.clone(),
            state: registry.get(id).map(|d| d.state).unwrap_or_default(),
        });
    } else if msg.topic.ends_with("/status") {
        match parse_status(&msg.payload) {
            Some(true) => debug!("wled: {id} online"),
            Some(false) => warn!("wled: {id} offline"),
            None => debug!("wled: unparseable status for {id}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use niles_core::{DeviceName, RoomName};

    fn dev_id(room: &str, name: &str) -> DeviceId {
        DeviceId::new(
            "wled",
            RoomName::parse(room).unwrap(),
            DeviceName::parse(name).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn dispatch_g_merges_brightness() {
        let registry = Arc::new(DeviceRegistry::new());
        let bus = EventBus::default();
        let id = dev_id("office", "desk_strip");
        registry.upsert(Device::new(
            id.clone(),
            DeviceState::default(),
            DeviceClass::Light,
        ));

        let mut topic_index = HashMap::new();
        topic_index.insert("wled/office/g".into(), id.clone());

        dispatch_wled(
            &Message {
                topic: "wled/office/g".into(),
                payload: b"128".to_vec(),
            },
            &topic_index,
            &registry,
            &bus,
        );

        let d = registry.get(&id).unwrap();
        assert_eq!(d.state.brightness, Some(50));
        assert_eq!(d.state.on, Some(true));
    }

    #[test]
    fn dispatch_c_merges_rgb() {
        let registry = Arc::new(DeviceRegistry::new());
        let bus = EventBus::default();
        let id = dev_id("office", "desk_strip");
        registry.upsert(Device::new(
            id.clone(),
            DeviceState::default(),
            DeviceClass::Light,
        ));

        let mut topic_index = HashMap::new();
        topic_index.insert("wled/office/c".into(), id.clone());

        dispatch_wled(
            &Message {
                topic: "wled/office/c".into(),
                payload: b"#00FF00".to_vec(),
            },
            &topic_index,
            &registry,
            &bus,
        );

        let d = registry.get(&id).unwrap();
        assert_eq!(d.state.rgb, Some([0, 255, 0]));
    }

    #[test]
    fn dispatch_g_then_c_preserves_both() {
        let registry = Arc::new(DeviceRegistry::new());
        let bus = EventBus::default();
        let id = dev_id("office", "desk_strip");
        registry.upsert(Device::new(
            id.clone(),
            DeviceState::default(),
            DeviceClass::Light,
        ));

        let mut topic_index = HashMap::new();
        topic_index.insert("wled/office/g".into(), id.clone());
        topic_index.insert("wled/office/c".into(), id.clone());

        dispatch_wled(
            &Message {
                topic: "wled/office/g".into(),
                payload: b"128".to_vec(),
            },
            &topic_index,
            &registry,
            &bus,
        );
        dispatch_wled(
            &Message {
                topic: "wled/office/c".into(),
                payload: b"#00FF00".to_vec(),
            },
            &topic_index,
            &registry,
            &bus,
        );

        let d = registry.get(&id).unwrap();
        assert_eq!(d.state.brightness, Some(50));
        assert_eq!(d.state.rgb, Some([0, 255, 0]));
    }

    #[test]
    fn dispatch_status_mutates_nothing() {
        let registry = Arc::new(DeviceRegistry::new());
        let bus = EventBus::default();
        let id = dev_id("office", "desk_strip");
        registry.upsert(Device::new(
            id.clone(),
            DeviceState::default(),
            DeviceClass::Light,
        ));

        let mut topic_index = HashMap::new();
        topic_index.insert("wled/office/status".into(), id.clone());

        dispatch_wled(
            &Message {
                topic: "wled/office/status".into(),
                payload: b"offline".to_vec(),
            },
            &topic_index,
            &registry,
            &bus,
        );

        let d = registry.get(&id).unwrap();
        assert_eq!(d.state, DeviceState::default());
    }

    #[test]
    fn unknown_topic_ignored() {
        let registry = Arc::new(DeviceRegistry::new());
        let bus = EventBus::default();
        let topic_index = HashMap::new();

        dispatch_wled(
            &Message {
                topic: "wled/office/unknown".into(),
                payload: b"x".to_vec(),
            },
            &topic_index,
            &registry,
            &bus,
        );
        // No panic, no mutation.
    }
}
