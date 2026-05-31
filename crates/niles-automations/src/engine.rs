//! Automation engine: subscribes to `EventBus`, matches rules, and
//! dispatches actions via injected `DeviceSink` + `Notifier` traits.

use crate::rule::{Action, Priority, Rule};
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use niles_core::{DeviceId, DeviceRegistry, DeviceState, Event, EventBus};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
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
    rules: Vec<Rule>,
    registry: Arc<DeviceRegistry>,
    tz: Tz,
    device_sink: Arc<dyn DeviceSink>,
    notifier: Option<Arc<dyn Notifier>>,
    dedup: Mutex<HashMap<(String, DeviceId), DateTime<Utc>>>,
    dedup_window: chrono::Duration,
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
            rules,
            registry,
            tz,
            device_sink,
            notifier,
            dedup: Mutex::new(HashMap::new()),
            dedup_window: chrono::Duration::milliseconds(2_000),
        }
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
        for rule in &self.rules {
            if !rule.enabled {
                continue;
            }
            if !rule.trigger.matches(event) {
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
                        let key = (rule.id.clone(), device.clone());
                        let mut dedup = self.dedup.lock().await;
                        if let Some(&last) = dedup.get(&key)
                            && now >= last
                            && now - last < self.dedup_window
                        {
                            tracing::debug!(
                                "automation '{}': dedup skipped set for {}",
                                rule.id,
                                device
                            );
                            continue;
                        }
                        dedup.insert(key, now);
                        // Opportunistic prune if map is getting large.
                        if dedup.len() > 64 {
                            let cutoff = now - self.dedup_window;
                            dedup.retain(|_, v| *v > cutoff);
                        }
                        drop(dedup);

                        let desired = DeviceState {
                            on: *on,
                            brightness: *brightness,
                            color_temp_kelvin: *kelvin,
                            ..Default::default()
                        };
                        self.device_sink.set(device, &desired).await;
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

    /// Spawn a background task that subscribes to `bus` and drives
    /// `handle_event` for every incoming event.
    pub fn spawn(self: Arc<Self>, bus: EventBus) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut rx = bus.subscribe();
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

    // ---- dispatch tests --------------------------------------------

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

        // Wait out the dedup window, then fire again.
        tokio::time::sleep(Duration::from_millis(60)).await;
        let later = fixed_now() + chrono::Duration::milliseconds(60);
        engine.handle_event(&ev, later).await;

        assert_eq!(sink.calls().await.len(), 2);
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
