//! Automation engine: subscribes to `EventBus`, matches rules, and
//! dispatches actions via injected `DeviceSink` + `Notifier` traits.

use crate::rule::{Action, Priority, Rule, Trigger};
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use niles_core::{DeviceId, DeviceRegistry, DeviceState, Event, EventBus};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;

/// Async trait for sending device commands.
#[async_trait::async_trait]
pub trait DeviceSink: Send + Sync {
    async fn set(&self, device: &DeviceId, desired: &DeviceState);
}

/// Async trait for delivering notifications.
#[async_trait::async_trait]
pub trait Notifier: Send + Sync {
    async fn notify(&self, body: &str, room: Option<&str>, priority: Priority);
}

/// Rule engine that evaluates triggers and dispatches actions.
pub struct AutomationEngine {
    /// Replaced whole when a rule is saved, paused or removed.
    rules: RwLock<Vec<Rule>>,
    registry: Arc<DeviceRegistry>,
    tz: Tz,
    device_sink: Arc<dyn DeviceSink>,
    notifier: Option<Arc<dyn Notifier>>,
    dedup: Mutex<HashMap<DedupKey, DateTime<Utc>>>,
    dedup_window: chrono::Duration,
    /// What each door and window sensor last said. A sensor repeats its
    /// state in every report (battery, signal), and a door that is
    /// still open has not opened again.
    contacts: Mutex<HashMap<DeviceId, bool>>,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct DedupKey {
    rule_id: String,
    device: DeviceId,
    on: Option<bool>,
    brightness: Option<u8>,
    kelvin: Option<u16>,
}

impl AutomationEngine {
    /// Create a new engine. `rules` should already be validated.
    pub fn new(
        rules: Vec<Rule>,
        registry: Arc<DeviceRegistry>,
        tz: Tz,
        device_sink: Arc<dyn DeviceSink>,
        notifier: Option<Arc<dyn Notifier>>,
    ) -> Self {
        Self {
            rules: RwLock::new(rules),
            registry,
            tz,
            device_sink,
            notifier,
            dedup: Mutex::new(HashMap::new()),
            dedup_window: chrono::Duration::milliseconds(2_000),
            contacts: Mutex::new(HashMap::new()),
        }
    }

    /// Run `rules` from now on, in place of the ones before.
    pub async fn replace(&self, rules: Vec<Rule>) {
        *self.rules.write().await = rules;
    }

    /// Take the sensors' state as it stands, so the first report after
    /// a start is compared with something: otherwise a door that is
    /// open when Niles starts "opens" with its first battery report.
    pub async fn seed_contacts(&self) {
        let mut contacts = self.contacts.lock().await;
        for device in self.registry.list_all() {
            if let Some(open) = device.state.open {
                contacts.insert(device.id.clone(), open);
            }
        }
    }

    /// Whether `event` is a door or window changing, as opposed to a
    /// sensor repeating itself. Anything that is not a contact report
    /// counts as a change.
    async fn contact_changed(&self, event: &Event) -> bool {
        let Event::DeviceStateChanged { id, state } = event else {
            return true;
        };
        let Some(open) = state.open else {
            return true;
        };
        self.contacts.lock().await.insert(id.clone(), open) != Some(open)
    }

    /// Build with a custom dedup window (useful in tests).
    #[cfg(test)]
    fn with_dedup_window(mut self, window: std::time::Duration) -> Self {
        self.dedup_window =
            chrono::Duration::from_std(window).unwrap_or(chrono::Duration::milliseconds(2_000));
        self
    }

    /// Evaluate all enabled rules against `event`.
    pub async fn handle_event(&self, event: &Event, now: DateTime<Utc>) {
        let contact_changed = self.contact_changed(event).await;
        let rules = self.rules.read().await;
        for rule in rules.iter() {
            if !rule.enabled {
                tracing::debug!("automation '{}' disabled, skipping", rule.id);
                continue;
            }
            if !rule.trigger.matches(event) {
                tracing::debug!("automation '{}' trigger mismatch", rule.id);
                continue;
            }
            if matches!(rule.trigger, Trigger::Contact { .. }) && !contact_changed {
                tracing::debug!("automation '{}': the sensor only repeated itself", rule.id);
                continue;
            }
            if !rule
                .conditions
                .iter()
                .all(|c| c.evaluate(&self.registry, now, self.tz))
            {
                tracing::debug!("automation '{}' conditions not met", rule.id);
                continue;
            }

            tracing::info!("automation '{}' fired", rule.id);

            for action in &rule.actions {
                match action {
                    Action::SetDevice {
                        device,
                        on,
                        brightness,
                        kelvin,
                    } => {
                        let desired = DeviceState {
                            on: *on,
                            brightness: *brightness,
                            color_temp_kelvin: *kelvin,
                            ..Default::default()
                        };
                        self.set(&rule.id, device, &desired, now).await;
                    }
                    Action::SetRoom {
                        room,
                        on,
                        brightness,
                        kelvin,
                    } => {
                        let desired = DeviceState {
                            on: *on,
                            brightness: *brightness,
                            color_temp_kelvin: *kelvin,
                            ..Default::default()
                        };
                        // The lights, as "the office lights" means: not
                        // the plug that charges something, nor the door
                        // sensor that may have started this.
                        let lights: Vec<DeviceId> = self
                            .registry
                            .list_room(room)
                            .into_iter()
                            .filter(|d| d.is_light() && d.is_switchable())
                            .map(|d| d.id)
                            .collect();
                        if lights.is_empty() {
                            tracing::warn!("automation '{}': no lights in {room} to set", rule.id);
                        }
                        for light in &lights {
                            self.set(&rule.id, light, &desired, now).await;
                        }
                    }
                    Action::Notify {
                        body,
                        room,
                        priority,
                    } => {
                        if let Some(n) = &self.notifier {
                            n.notify(body, room.as_deref(), *priority).await;
                        } else {
                            tracing::warn!(
                                "automation '{}': notify skipped (no notifier configured)",
                                rule.id
                            );
                        }
                    }
                }
            }
        }
    }

    /// Send `desired` to `device`, unless this rule sent the same a
    /// moment ago — one change can arrive as several events.
    async fn set(
        &self,
        rule_id: &str,
        device: &DeviceId,
        desired: &DeviceState,
        now: DateTime<Utc>,
    ) {
        let key = DedupKey {
            rule_id: rule_id.to_string(),
            device: device.clone(),
            on: desired.on,
            brightness: desired.brightness,
            kelvin: desired.color_temp_kelvin,
        };
        let mut dedup = self.dedup.lock().await;
        if let Some(&last) = dedup.get(&key)
            && now >= last
            && now - last < self.dedup_window
        {
            tracing::debug!("automation '{rule_id}': dedup skipped set for {device}");
            return;
        }
        dedup.insert(key, now);
        // Opportunistic prune if map is getting large.
        if dedup.len() > 64 {
            let cutoff = now - self.dedup_window;
            dedup.retain(|_, v| *v > cutoff);
        }
        drop(dedup);
        self.device_sink.set(device, desired).await;
    }

    /// Spawn a background task that subscribes to `bus` and drives
    /// `handle_event` for every incoming event.
    pub fn spawn(self: Arc<Self>, bus: EventBus) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut rx = bus.subscribe();
            self.seed_contacts().await;
            loop {
                match rx.recv().await {
                    Ok(ev) => self.handle_event(&ev, Utc::now()).await,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("automation subscriber lagged by {n} events");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rule::{Condition, Trigger};
    use chrono::{NaiveTime, TimeZone};
    use niles_core::{DeviceClass, DeviceName, RoomName};
    use std::time::Duration;

    fn device_id(room: &str, name: &str) -> DeviceId {
        DeviceId::new(
            "z2m",
            RoomName::parse(room).unwrap(),
            DeviceName::parse(name).unwrap(),
        )
        .unwrap()
    }

    fn state(on: bool) -> DeviceState {
        DeviceState {
            on: Some(on),
            ..Default::default()
        }
    }

    fn make_device(id: &DeviceId, on: bool) -> niles_core::Device {
        niles_core::Device::new(id.clone(), state(on), DeviceClass::Light)
    }

    struct RecordingSink {
        calls: Mutex<Vec<(DeviceId, DeviceState)>>,
    }

    impl RecordingSink {
        fn new() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
            }
        }
        async fn calls(&self) -> Vec<(DeviceId, DeviceState)> {
            self.calls.lock().await.clone()
        }
    }

    #[async_trait::async_trait]
    impl DeviceSink for RecordingSink {
        async fn set(&self, device: &DeviceId, desired: &DeviceState) {
            self.calls
                .lock()
                .await
                .push((device.clone(), desired.clone()));
        }
    }

    struct RecordingNotifier {
        calls: Mutex<Vec<(String, Option<String>, Priority)>>,
    }

    impl RecordingNotifier {
        fn new() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
            }
        }
        async fn calls(&self) -> Vec<(String, Option<String>, Priority)> {
            self.calls.lock().await.clone()
        }
    }

    #[async_trait::async_trait]
    impl Notifier for RecordingNotifier {
        async fn notify(&self, body: &str, room: Option<&str>, priority: Priority) {
            self.calls
                .lock()
                .await
                .push((body.to_string(), room.map(String::from), priority));
        }
    }

    fn engine_with_rules(
        rules: Vec<Rule>,
        registry: Arc<DeviceRegistry>,
    ) -> (
        Arc<AutomationEngine>,
        Arc<RecordingSink>,
        Arc<RecordingNotifier>,
    ) {
        let sink = Arc::new(RecordingSink::new());
        let notifier = Arc::new(RecordingNotifier::new());
        let engine = Arc::new(AutomationEngine::new(
            rules,
            registry,
            Tz::UTC,
            sink.clone(),
            Some(notifier.clone()),
        ));
        (engine, sink, notifier)
    }

    fn fixed_now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap()
    }

    // ---- doors and rooms ---------------------------------------------

    fn contact(open: bool) -> DeviceState {
        DeviceState {
            open: Some(open),
            ..Default::default()
        }
    }

    /// The office: a door to the garden, two lights, and a plug.
    fn office() -> (Arc<DeviceRegistry>, DeviceId) {
        let registry = Arc::new(DeviceRegistry::new());
        let door = device_id("office", "door");
        registry.upsert(niles_core::Device::new(
            door.clone(),
            contact(false),
            DeviceClass::Contact,
        ));
        registry.upsert(make_device(&device_id("office", "go"), true));
        registry.upsert(make_device(&device_id("office", "lightstrip"), true));
        registry.upsert(niles_core::Device::new(
            device_id("office", "charger"),
            state(true),
            DeviceClass::Outlet,
        ));
        (registry, door)
    }

    fn garden_door(door: &DeviceId) -> Rule {
        Rule {
            id: "garden-door".into(),
            description: String::new(),
            enabled: true,
            trigger: Trigger::Contact {
                device: door.clone(),
                open: Some(true),
            },
            conditions: vec![],
            actions: vec![Action::SetRoom {
                room: RoomName::parse("office").unwrap(),
                on: Some(false),
                brightness: None,
                kelvin: None,
            }],
        }
    }

    fn opened(door: &DeviceId, open: bool) -> Event {
        Event::DeviceStateChanged {
            id: door.clone(),
            state: contact(open),
        }
    }

    #[tokio::test]
    async fn the_garden_door_turns_off_the_office_lights_only() {
        let (registry, door) = office();
        let (engine, sink, _) = engine_with_rules(vec![garden_door(&door)], registry);
        engine.seed_contacts().await;
        engine.handle_event(&opened(&door, true), fixed_now()).await;
        let mut set: Vec<String> = sink
            .calls()
            .await
            .into_iter()
            .map(|(id, state)| {
                assert_eq!(state.on, Some(false));
                id.to_string()
            })
            .collect();
        set.sort();
        assert_eq!(set, ["z2m:office/go", "z2m:office/lightstrip"]);
    }

    #[tokio::test]
    async fn a_door_still_open_does_not_open_again() {
        let (registry, door) = office();
        let (engine, sink, _) = engine_with_rules(vec![garden_door(&door)], registry);
        engine.seed_contacts().await;
        engine.handle_event(&opened(&door, true), fixed_now()).await;
        // The sensor's next report, minutes later, still says open.
        let later = fixed_now() + chrono::Duration::minutes(5);
        engine.handle_event(&opened(&door, true), later).await;
        assert_eq!(sink.calls().await.len(), 2, "one opening, two lights");
        // Closed and opened again is a new opening.
        engine.handle_event(&opened(&door, false), later).await;
        engine
            .handle_event(&opened(&door, true), later + chrono::Duration::minutes(1))
            .await;
        assert_eq!(sink.calls().await.len(), 4);
    }

    #[tokio::test]
    async fn a_door_open_when_niles_starts_has_not_just_opened() {
        let (registry, door) = office();
        registry.upsert(niles_core::Device::new(
            door.clone(),
            contact(true),
            DeviceClass::Contact,
        ));
        let (engine, sink, _) = engine_with_rules(vec![garden_door(&door)], registry);
        engine.seed_contacts().await;
        engine.handle_event(&opened(&door, true), fixed_now()).await;
        assert!(sink.calls().await.is_empty());
    }

    #[tokio::test]
    async fn a_door_rule_waits_for_its_condition() {
        let (registry, door) = office();
        let mut rule = garden_door(&door);
        rule.conditions.push(Condition::TimeOfDay {
            after: NaiveTime::from_hms_opt(22, 0, 0),
            before: NaiveTime::from_hms_opt(6, 0, 0),
        });
        let (engine, sink, _) = engine_with_rules(vec![rule], registry);
        engine.seed_contacts().await;
        engine.handle_event(&opened(&door, true), fixed_now()).await;
        assert!(sink.calls().await.is_empty(), "noon is not night");
    }

    #[tokio::test]
    async fn new_rules_run_in_place_of_the_old() {
        let (registry, door) = office();
        let (engine, sink, _) = engine_with_rules(vec![], registry);
        engine.seed_contacts().await;
        engine.replace(vec![garden_door(&door)]).await;
        engine.handle_event(&opened(&door, true), fixed_now()).await;
        assert_eq!(sink.calls().await.len(), 2);
    }

    // ---- dispatch tests --------------------------------------------

    #[tokio::test]
    async fn trigger_mismatch_no_dispatch() {
        let registry = Arc::new(DeviceRegistry::new());
        let id = device_id("kitchen", "motion");
        let target = device_id("kitchen", "light");
        registry.upsert(make_device(&id, true));

        let rule = Rule {
            id: "r1".into(),
            description: String::new(),
            enabled: true,
            trigger: Trigger::DeviceState {
                device: Some(id.clone()),
                room: None,
                on: Some(true),
            },
            conditions: vec![],
            actions: vec![Action::SetDevice {
                device: target.clone(),
                on: Some(true),
                brightness: None,
                kelvin: None,
            }],
        };

        let (engine, sink, _notifier) = engine_with_rules(vec![rule], registry);
        // Wrong device in event — trigger should not match
        let ev = Event::DeviceStateChanged {
            id: device_id("hallway", "motion"),
            state: state(true),
        };
        engine.handle_event(&ev, fixed_now()).await;

        assert!(sink.calls().await.is_empty());
    }

    #[tokio::test]
    async fn mixed_actions_fire_both() {
        let registry = Arc::new(DeviceRegistry::new());
        let id = device_id("kitchen", "motion");
        let target = device_id("kitchen", "light");
        registry.upsert(make_device(&id, true));

        let rule = Rule {
            id: "r1".into(),
            description: String::new(),
            enabled: true,
            trigger: Trigger::DeviceState {
                device: Some(id.clone()),
                room: None,
                on: Some(true),
            },
            conditions: vec![],
            actions: vec![
                Action::SetDevice {
                    device: target.clone(),
                    on: Some(true),
                    brightness: Some(50),
                    kelvin: None,
                },
                Action::Notify {
                    body: "motion detected".into(),
                    room: Some("kitchen".into()),
                    priority: Priority::Important,
                },
            ],
        };

        let (engine, sink, notifier) = engine_with_rules(vec![rule], registry);
        let ev = Event::DeviceStateChanged {
            id,
            state: state(true),
        };
        engine.handle_event(&ev, fixed_now()).await;

        assert_eq!(sink.calls().await.len(), 1);
        assert_eq!(sink.calls().await[0].0, target);

        let note_calls = notifier.calls().await;
        assert_eq!(note_calls.len(), 1);
        assert_eq!(note_calls[0].0, "motion detected");
    }

    #[tokio::test]
    async fn missing_notifier_does_not_block_set_device() {
        let registry = Arc::new(DeviceRegistry::new());
        let id = device_id("kitchen", "motion");
        let target = device_id("kitchen", "light");
        registry.upsert(make_device(&id, true));

        let rule = Rule {
            id: "r1".into(),
            description: String::new(),
            enabled: true,
            trigger: Trigger::DeviceState {
                device: Some(id.clone()),
                room: None,
                on: Some(true),
            },
            conditions: vec![],
            actions: vec![
                Action::SetDevice {
                    device: target.clone(),
                    on: Some(true),
                    brightness: None,
                    kelvin: None,
                },
                Action::Notify {
                    body: "motion!".into(),
                    room: None,
                    priority: Priority::Routine,
                },
            ],
        };

        let sink = Arc::new(RecordingSink::new());
        let engine = Arc::new(AutomationEngine::new(
            vec![rule],
            registry,
            Tz::UTC,
            sink.clone(),
            None::<Arc<dyn Notifier>>,
        ));

        let ev = Event::DeviceStateChanged {
            id,
            state: state(true),
        };
        engine.handle_event(&ev, fixed_now()).await;

        assert_eq!(sink.calls().await.len(), 1);
        assert_eq!(sink.calls().await[0].0, target);
    }

    #[tokio::test]
    async fn matching_trigger_no_conditions_dispatches() {
        let registry = Arc::new(DeviceRegistry::new());
        let id = device_id("kitchen", "light");
        registry.upsert(make_device(&id, false));

        let rule = Rule {
            id: "r1".into(),
            description: String::new(),
            enabled: true,
            trigger: Trigger::DeviceState {
                device: Some(id.clone()),
                room: None,
                on: Some(true),
            },
            conditions: vec![],
            actions: vec![Action::SetDevice {
                device: id.clone(),
                on: Some(true),
                brightness: Some(50),
                kelvin: None,
            }],
        };

        let (engine, sink, _notifier) = engine_with_rules(vec![rule], registry);
        let ev = Event::DeviceStateChanged {
            id: id.clone(),
            state: state(true),
        };
        engine.handle_event(&ev, fixed_now()).await;

        let calls = sink.calls().await;
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, id);
        assert_eq!(calls[0].1.on, Some(true));
        assert_eq!(calls[0].1.brightness, Some(50));
    }

    #[tokio::test]
    async fn failing_time_of_day_blocks_dispatch() {
        let registry = Arc::new(DeviceRegistry::new());
        let id = device_id("kitchen", "light");
        registry.upsert(make_device(&id, false));

        let rule = Rule {
            id: "r1".into(),
            description: String::new(),
            enabled: true,
            trigger: Trigger::DeviceState {
                device: Some(id.clone()),
                room: None,
                on: Some(true),
            },
            conditions: vec![Condition::TimeOfDay {
                after: Some(NaiveTime::from_hms_opt(20, 0, 0).unwrap()),
                before: Some(NaiveTime::from_hms_opt(23, 0, 0).unwrap()),
            }],
            actions: vec![Action::SetDevice {
                device: id.clone(),
                on: Some(true),
                brightness: None,
                kelvin: None,
            }],
        };

        let (engine, sink, _notifier) = engine_with_rules(vec![rule], registry);
        let ev = Event::DeviceStateChanged {
            id,
            state: state(true),
        };
        engine.handle_event(&ev, fixed_now()).await;

        assert!(sink.calls().await.is_empty());
    }

    #[tokio::test]
    async fn passing_device_is_allows_dispatch() {
        let registry = Arc::new(DeviceRegistry::new());
        let motion = device_id("hallway", "motion");
        let light = device_id("hallway", "light");
        registry.upsert(make_device(&motion, true));
        registry.upsert(make_device(&light, true));

        let rule = Rule {
            id: "r1".into(),
            description: String::new(),
            enabled: true,
            trigger: Trigger::DeviceState {
                device: Some(motion.clone()),
                room: None,
                on: Some(true),
            },
            conditions: vec![Condition::DeviceIs {
                device: light.clone(),
                on: true,
            }],
            actions: vec![Action::SetDevice {
                device: light.clone(),
                on: Some(false),
                brightness: None,
                kelvin: None,
            }],
        };

        let (engine, sink, _notifier) = engine_with_rules(vec![rule], registry);
        let ev = Event::DeviceStateChanged {
            id: motion,
            state: state(true),
        };
        engine.handle_event(&ev, fixed_now()).await;

        let calls = sink.calls().await;
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, light);
    }

    #[tokio::test]
    async fn device_is_unknown_blocks_dispatch() {
        let registry = Arc::new(DeviceRegistry::new());
        let motion = device_id("hallway", "motion");
        let light = device_id("hallway", "light");
        registry.upsert(make_device(&motion, true));
        // light NOT in registry

        let rule = Rule {
            id: "r1".into(),
            description: String::new(),
            enabled: true,
            trigger: Trigger::DeviceState {
                device: Some(motion.clone()),
                room: None,
                on: Some(true),
            },
            conditions: vec![Condition::DeviceIs {
                device: light.clone(),
                on: true,
            }],
            actions: vec![Action::SetDevice {
                device: light.clone(),
                on: Some(false),
                brightness: None,
                kelvin: None,
            }],
        };

        let (engine, sink, _notifier) = engine_with_rules(vec![rule], registry);
        let ev = Event::DeviceStateChanged {
            id: motion,
            state: state(true),
        };
        engine.handle_event(&ev, fixed_now()).await;

        assert!(sink.calls().await.is_empty());
    }

    #[tokio::test]
    async fn notify_action_records() {
        let registry = Arc::new(DeviceRegistry::new());
        let id = device_id("kitchen", "motion");
        registry.upsert(make_device(&id, true));

        let rule = Rule {
            id: "r1".into(),
            description: String::new(),
            enabled: true,
            trigger: Trigger::DeviceState {
                device: Some(id.clone()),
                room: None,
                on: Some(true),
            },
            conditions: vec![],
            actions: vec![Action::Notify {
                body: "motion!".into(),
                room: Some("kitchen".into()),
                priority: Priority::Important,
            }],
        };

        let (engine, _sink, notifier) = engine_with_rules(vec![rule], registry);
        let ev = Event::DeviceStateChanged {
            id,
            state: state(true),
        };
        engine.handle_event(&ev, fixed_now()).await;

        let calls = notifier.calls().await;
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "motion!");
        assert_eq!(calls[0].1, Some("kitchen".into()));
        assert_eq!(calls[0].2, Priority::Important);
    }

    #[tokio::test]
    async fn notify_without_notifier_warns_not_panics() {
        let registry = Arc::new(DeviceRegistry::new());
        let id = device_id("kitchen", "motion");
        registry.upsert(make_device(&id, true));

        let rule = Rule {
            id: "r1".into(),
            description: String::new(),
            enabled: true,
            trigger: Trigger::DeviceState {
                device: Some(id.clone()),
                room: None,
                on: Some(true),
            },
            conditions: vec![],
            actions: vec![Action::Notify {
                body: "motion!".into(),
                room: None,
                priority: Priority::Routine,
            }],
        };

        let sink = Arc::new(RecordingSink::new());
        let engine = Arc::new(AutomationEngine::new(
            vec![rule],
            registry,
            Tz::UTC,
            sink.clone(),
            None::<Arc<dyn Notifier>>,
        ));

        let ev = Event::DeviceStateChanged {
            id,
            state: state(true),
        };
        engine.handle_event(&ev, fixed_now()).await;

        assert!(sink.calls().await.is_empty());
    }

    #[tokio::test]
    async fn dedup_window_prevents_double_dispatch() {
        let registry = Arc::new(DeviceRegistry::new());
        let id = device_id("kitchen", "motion");
        let target = device_id("kitchen", "light");
        registry.upsert(make_device(&id, true));

        let rule = Rule {
            id: "r1".into(),
            description: String::new(),
            enabled: true,
            trigger: Trigger::DeviceState {
                device: Some(id.clone()),
                room: None,
                on: Some(true),
            },
            conditions: vec![],
            actions: vec![Action::SetDevice {
                device: target.clone(),
                on: Some(true),
                brightness: None,
                kelvin: None,
            }],
        };

        let sink = Arc::new(RecordingSink::new());
        let engine = Arc::new(
            AutomationEngine::new(
                vec![rule],
                registry,
                Tz::UTC,
                sink.clone(),
                None::<Arc<dyn Notifier>>,
            )
            .with_dedup_window(Duration::from_millis(50)),
        );

        let ev = Event::DeviceStateChanged {
            id: id.clone(),
            state: state(true),
        };

        engine.handle_event(&ev, fixed_now()).await;
        engine.handle_event(&ev, fixed_now()).await;

        assert_eq!(sink.calls().await.len(), 1);

        let later = fixed_now() + chrono::Duration::milliseconds(60);
        engine.handle_event(&ev, later).await;

        assert_eq!(sink.calls().await.len(), 2);
    }

    #[tokio::test]
    async fn dedup_does_not_suppress_distinct_desired_state() {
        let registry = Arc::new(DeviceRegistry::new());
        let id = device_id("kitchen", "motion");
        let target = device_id("kitchen", "light");
        registry.upsert(make_device(&id, true));

        let rule = Rule {
            id: "r1".into(),
            description: String::new(),
            enabled: true,
            trigger: Trigger::DeviceState {
                device: Some(id.clone()),
                room: None,
                on: Some(true),
            },
            conditions: vec![],
            actions: vec![
                Action::SetDevice {
                    device: target.clone(),
                    on: Some(true),
                    brightness: None,
                    kelvin: None,
                },
                Action::SetDevice {
                    device: target.clone(),
                    on: Some(false),
                    brightness: None,
                    kelvin: None,
                },
            ],
        };

        let sink = Arc::new(RecordingSink::new());
        let engine = Arc::new(
            AutomationEngine::new(
                vec![rule],
                registry,
                Tz::UTC,
                sink.clone(),
                None::<Arc<dyn Notifier>>,
            )
            .with_dedup_window(Duration::from_secs(5)),
        );

        let ev = Event::DeviceStateChanged {
            id,
            state: state(true),
        };
        engine.handle_event(&ev, fixed_now()).await;

        let calls = sink.calls().await;
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].1.on, Some(true));
        assert_eq!(calls[1].1.on, Some(false));
    }

    #[tokio::test]
    async fn disabled_rule_never_fires() {
        let registry = Arc::new(DeviceRegistry::new());
        let id = device_id("kitchen", "motion");
        registry.upsert(make_device(&id, true));

        let rule = Rule {
            id: "r1".into(),
            description: String::new(),
            enabled: false,
            trigger: Trigger::DeviceState {
                device: Some(id.clone()),
                room: None,
                on: Some(true),
            },
            conditions: vec![],
            actions: vec![Action::SetDevice {
                device: id.clone(),
                on: Some(true),
                brightness: None,
                kelvin: None,
            }],
        };

        let (engine, sink, _notifier) = engine_with_rules(vec![rule], registry);
        let ev = Event::DeviceStateChanged {
            id,
            state: state(true),
        };
        engine.handle_event(&ev, fixed_now()).await;

        assert!(sink.calls().await.is_empty());
    }

    #[tokio::test]
    async fn multiple_conditions_all_must_pass() {
        let registry = Arc::new(DeviceRegistry::new());
        let motion = device_id("hallway", "motion");
        let light = device_id("hallway", "light");
        registry.upsert(make_device(&motion, true));
        registry.upsert(make_device(&light, true));

        let rule = Rule {
            id: "r1".into(),
            description: String::new(),
            enabled: true,
            trigger: Trigger::DeviceState {
                device: Some(motion.clone()),
                room: None,
                on: Some(true),
            },
            conditions: vec![
                Condition::TimeOfDay {
                    after: Some(NaiveTime::from_hms_opt(10, 0, 0).unwrap()),
                    before: Some(NaiveTime::from_hms_opt(14, 0, 0).unwrap()),
                },
                Condition::DeviceIs {
                    device: light.clone(),
                    on: true,
                },
            ],
            actions: vec![Action::SetDevice {
                device: light.clone(),
                on: Some(false),
                brightness: None,
                kelvin: None,
            }],
        };

        let (engine, sink, _notifier) = engine_with_rules(vec![rule], registry);
        let ev = Event::DeviceStateChanged {
            id: motion,
            state: state(true),
        };
        engine.handle_event(&ev, fixed_now()).await;

        assert_eq!(sink.calls().await.len(), 1);
    }

    #[tokio::test]
    async fn multiple_conditions_one_fails_blocks_dispatch() {
        let registry = Arc::new(DeviceRegistry::new());
        let motion = device_id("hallway", "motion");
        let light = device_id("hallway", "light");
        registry.upsert(make_device(&motion, true));
        registry.upsert(make_device(&light, false));

        let rule = Rule {
            id: "r1".into(),
            description: String::new(),
            enabled: true,
            trigger: Trigger::DeviceState {
                device: Some(motion.clone()),
                room: None,
                on: Some(true),
            },
            conditions: vec![
                Condition::TimeOfDay {
                    after: Some(NaiveTime::from_hms_opt(10, 0, 0).unwrap()),
                    before: Some(NaiveTime::from_hms_opt(14, 0, 0).unwrap()),
                },
                Condition::DeviceIs {
                    device: light.clone(),
                    on: true,
                },
            ],
            actions: vec![Action::SetDevice {
                device: light.clone(),
                on: Some(false),
                brightness: None,
                kelvin: None,
            }],
        };

        let (engine, sink, _notifier) = engine_with_rules(vec![rule], registry);
        let ev = Event::DeviceStateChanged {
            id: motion,
            state: state(true),
        };
        engine.handle_event(&ev, fixed_now()).await;

        assert!(sink.calls().await.is_empty());
    }

    #[tokio::test]
    async fn dedup_allows_dispatch_after_clock_skew() {
        let registry = Arc::new(DeviceRegistry::new());
        let id = device_id("kitchen", "motion");
        let target = device_id("kitchen", "light");
        registry.upsert(make_device(&id, true));

        let rule = Rule {
            id: "r1".into(),
            description: String::new(),
            enabled: true,
            trigger: Trigger::DeviceState {
                device: Some(id.clone()),
                room: None,
                on: Some(true),
            },
            conditions: vec![],
            actions: vec![Action::SetDevice {
                device: target.clone(),
                on: Some(true),
                brightness: None,
                kelvin: None,
            }],
        };

        let sink = Arc::new(RecordingSink::new());
        let engine = Arc::new(
            AutomationEngine::new(
                vec![rule],
                registry,
                Tz::UTC,
                sink.clone(),
                None::<Arc<dyn Notifier>>,
            )
            .with_dedup_window(Duration::from_secs(60)),
        );

        let ev = Event::DeviceStateChanged {
            id: id.clone(),
            state: state(true),
        };

        let first = fixed_now();
        engine.handle_event(&ev, first).await;
        assert_eq!(sink.calls().await.len(), 1);

        // Clock goes backwards — should still allow dispatch because now <= last
        let skewed = first - chrono::Duration::seconds(10);
        engine.handle_event(&ev, skewed).await;
        assert_eq!(sink.calls().await.len(), 2);
    }
}
