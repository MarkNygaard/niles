//! Automation engine: follows the event bus and the clock, starts
//! rules, and runs their actions through injected `DeviceSink` and
//! `Notifier` traits.
//!
//! A trigger that follows a state fires on the change into it — a
//! sensor repeating itself in every battery report has not changed —
//! and, with `for`, only once the state has held that long. Each rule
//! runs once at a time: a trigger firing again while a run waits out a
//! delay replaces that run.

use crate::rule::{Action, BlindTarget, Context, Priority, Rule};
use crate::runs::{Outcome, RunLog};
use crate::sun::Location;
use chrono::{DateTime, NaiveDateTime, Timelike, Utc};
use chrono_tz::Tz;
use niles_core::{DeviceId, DeviceRegistry, DeviceState, Event, EventBus, PresenceState};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use tokio::sync::RwLock;
use tokio::task::{AbortHandle, JoinHandle};

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

/// A rule's trigger, and the thing it concerns: a device, or the house.
type Followed = (String, String, String);

/// Rule engine that evaluates triggers and dispatches actions.
pub struct AutomationEngine {
    /// Replaced whole when a rule is saved, paused or removed.
    rules: RwLock<Vec<Arc<Rule>>>,
    registry: Arc<DeviceRegistry>,
    tz: Tz,
    location: Option<Location>,
    device_sink: Arc<dyn DeviceSink>,
    notifier: Option<Arc<dyn Notifier>>,
    dedup: Mutex<HashMap<DedupKey, DateTime<Utc>>>,
    dedup_window: chrono::Duration,
    /// Whether each followed state held when last heard of.
    observed: Mutex<HashMap<Followed, bool>>,
    /// States waiting out their `for`.
    holds: Mutex<HashMap<Followed, AbortHandle>>,
    /// The house, as the last presence change said.
    presence: Mutex<Option<PresenceState>>,
    /// Each rule's run in progress, by its number.
    running: Mutex<HashMap<String, (u64, AbortHandle)>>,
    /// The minute each clock trigger last fired, so a tick twice in one
    /// minute does not fire it twice.
    clock: Mutex<HashMap<(String, String), NaiveDateTime>>,
    log: Arc<RunLog>,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct DedupKey {
    rule_id: String,
    device: DeviceId,
    on: Option<bool>,
    brightness: Option<u8>,
    kelvin: Option<u16>,
}

type Handles = Vec<JoinHandle<()>>;

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
            rules: RwLock::new(rules.into_iter().map(Arc::new).collect()),
            registry,
            tz,
            location: None,
            device_sink,
            notifier,
            dedup: Mutex::new(HashMap::new()),
            dedup_window: chrono::Duration::milliseconds(2_000),
            observed: Mutex::new(HashMap::new()),
            holds: Mutex::new(HashMap::new()),
            presence: Mutex::new(None),
            running: Mutex::new(HashMap::new()),
            clock: Mutex::new(HashMap::new()),
            log: Arc::new(RunLog::new()),
        }
    }

    /// Where the house is, for the sun. Without it, sun triggers never
    /// fire and sun conditions never hold.
    pub fn with_location(mut self, location: Option<Location>) -> Self {
        self.location = location;
        self
    }

    /// Keep runs in `log`, which the app and the voice tools read too.
    pub fn with_log(mut self, log: Arc<RunLog>) -> Self {
        self.log = log;
        self
    }

    /// Build with a custom dedup window (useful in tests).
    #[cfg(test)]
    fn with_dedup_window(mut self, window: std::time::Duration) -> Self {
        self.dedup_window =
            chrono::Duration::from_std(window).unwrap_or(chrono::Duration::milliseconds(2_000));
        self
    }

    pub fn log(&self) -> Arc<RunLog> {
        self.log.clone()
    }

    /// Run `rules` from now on, in place of the ones before. A run or a
    /// `for` of a rule that is gone or paused stops; the states the new
    /// rules follow are read from the registry, so a change already
    /// made does not count as one.
    pub async fn replace(&self, rules: Vec<Rule>) {
        let rules: Vec<Arc<Rule>> = rules.into_iter().map(Arc::new).collect();
        let live = |id: &str| rules.iter().any(|r| r.id == id && r.enabled);
        lock(&self.running).retain(|rule, (_, handle)| {
            let keep = live(rule);
            if !keep {
                handle.abort();
            }
            keep
        });
        for (_, handle) in lock(&self.holds).drain() {
            handle.abort();
        }
        lock(&self.observed).retain(|(rule, ..), _| live(rule));
        *self.rules.write().await = rules;
        self.seed().await;
    }

    /// Take the states the rules follow as they stand: otherwise a door
    /// that is open when Niles starts "opens" with its first battery
    /// report.
    pub async fn seed(&self) {
        let rules = self.rules.read().await.clone();
        let mut observed = lock(&self.observed);
        for rule in &rules {
            for t in &rule.triggers {
                for (subject, holds) in t.trigger.seed(&self.registry) {
                    observed
                        .entry((rule.id.clone(), t.id.clone(), subject))
                        .or_insert(holds);
                }
            }
        }
    }

    /// Evaluate every enabled rule against `event`. Returns the runs and
    /// `for`s it started, which the bus loop lets go of and a test can
    /// wait for.
    pub async fn handle_event(self: &Arc<Self>, event: &Event, now: DateTime<Utc>) -> Handles {
        if let Event::PresenceChanged { state } = event {
            *lock(&self.presence) = Some(*state);
        }
        let rules = self.rules.read().await.clone();
        let mut handles = Vec::new();
        for rule in rules.iter().filter(|r| r.enabled) {
            for t in &rule.triggers {
                if t.trigger.fires(event) {
                    handles.extend(self.start(rule, &t.id, now));
                    continue;
                }
                let Some((subject, holds)) = t.trigger.observe(event) else {
                    continue;
                };
                let key = (rule.id.clone(), t.id.clone(), subject);
                let before = lock(&self.observed).insert(key.clone(), holds);
                if !holds {
                    if let Some(waiting) = lock(&self.holds).remove(&key) {
                        waiting.abort();
                        tracing::debug!("automation '{}': {} stopped holding", rule.id, t.id);
                    }
                    continue;
                }
                if before == Some(true) {
                    // Still so: a report repeating itself, not a change.
                    continue;
                }
                match t.hold {
                    None => handles.extend(self.start(rule, &t.id, now)),
                    Some(wait) => handles.push(self.hold(rule.clone(), &t.id, key, wait, now)),
                }
            }
        }
        handles
    }

    /// Fire the clock's triggers due this minute.
    pub async fn tick(self: &Arc<Self>, now: DateTime<Utc>) -> Handles {
        let local = now.with_timezone(&self.tz).naive_local();
        let Some(minute) = local.with_second(0).and_then(|m| m.with_nanosecond(0)) else {
            return Vec::new();
        };
        let rules = self.rules.read().await.clone();
        let mut handles = Vec::new();
        for rule in rules.iter().filter(|r| r.enabled) {
            for t in &rule.triggers {
                if t.trigger.due_on(now, self.tz, self.location) != Some(minute) {
                    continue;
                }
                let fired = lock(&self.clock).insert((rule.id.clone(), t.id.clone()), minute);
                if fired == Some(minute) {
                    continue;
                }
                handles.extend(self.start(rule, &t.id, now));
            }
        }
        handles
    }

    /// Wait out a trigger's `for`; start the rule if the state still
    /// holds then.
    fn hold(
        self: &Arc<Self>,
        rule: Arc<Rule>,
        trigger: &str,
        key: Followed,
        wait: std::time::Duration,
        now: DateTime<Utc>,
    ) -> JoinHandle<()> {
        let engine = self.clone();
        let trigger = trigger.to_string();
        let held = key.clone();
        let mut holds = lock(&self.holds);
        let task = tokio::spawn(async move {
            tokio::time::sleep(wait).await;
            lock(&engine.holds).remove(&held);
            let then = now + chrono::Duration::from_std(wait).unwrap_or_default();
            if let Some(run) = engine.start(&rule, &trigger, then) {
                let _ = run.await;
            }
        });
        if let Some(older) = holds.insert(key, task.abort_handle()) {
            older.abort();
        }
        task
    }

    /// Start a run of `rule`, started by `trigger`: unless a condition
    /// stops it, which is a run too, kept with the reason.
    fn start(
        self: &Arc<Self>,
        rule: &Arc<Rule>,
        trigger: &str,
        now: DateTime<Utc>,
    ) -> Option<JoinHandle<()>> {
        let presence = *lock(&self.presence);
        let unmet = {
            let ctx = Context {
                registry: &self.registry,
                now,
                tz: self.tz,
                location: self.location,
                presence,
                trigger: Some(trigger),
            };
            rule.conditions
                .iter()
                .find(|c| !c.evaluate(&ctx))
                .map(|c| c.unmet())
        };
        if let Some(reason) = unmet {
            tracing::debug!("automation '{}' ({trigger}) stopped: {reason}", rule.id);
            self.log
                .begin(&rule.id, Some(trigger), now, Outcome::Stopped, Some(reason));
            return None;
        }
        tracing::info!("automation '{}' fired ({trigger})", rule.id);
        let number = self
            .log
            .begin(&rule.id, Some(trigger), now, Outcome::Running, None);
        let engine = self.clone();
        let run_rule = rule.clone();
        let trigger = trigger.to_string();
        // Held while the run is spawned and noted, so a run that ends at
        // once cannot clear itself before it is there to be cleared.
        let mut running = lock(&self.running);
        let task = tokio::spawn(async move {
            let ended = engine
                .perform(&run_rule, &run_rule.actions, &trigger, number, now)
                .await;
            match ended {
                Ok(_) => engine.log.finish(&run_rule.id, number, Outcome::Done, None),
                Err(reason) => {
                    tracing::warn!("automation '{}' failed: {reason}", run_rule.id);
                    engine
                        .log
                        .finish(&run_rule.id, number, Outcome::Failed, Some(reason));
                }
            }
            let mut running = lock(&engine.running);
            if running.get(&run_rule.id).is_some_and(|(n, _)| *n == number) {
                running.remove(&run_rule.id);
            }
        });
        if let Some((older, handle)) =
            running.insert(rule.id.clone(), (number, task.abort_handle()))
        {
            handle.abort();
            self.log.replaced(&rule.id, older);
        }
        Some(task)
    }

    /// Run `actions` in order, waiting where a delay says. Returns the
    /// time it got to, for the conditions of an `if` after a delay.
    fn perform<'a>(
        self: &'a Arc<Self>,
        rule: &'a Rule,
        actions: &'a [Action],
        trigger: &'a str,
        number: u64,
        mut now: DateTime<Utc>,
    ) -> Pin<Box<dyn Future<Output = Result<DateTime<Utc>, String>> + Send + 'a>> {
        Box::pin(async move {
            for action in actions {
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
                            return Err(format!("there are no lights in {room}"));
                        }
                        for light in &lights {
                            self.set(&rule.id, light, &desired, now).await;
                        }
                    }
                    Action::SetBlinds { target, position } => {
                        let desired = DeviceState {
                            position: Some(*position),
                            ..Default::default()
                        };
                        let blinds: Vec<DeviceId> = match target {
                            BlindTarget::Device(device) => vec![device.clone()],
                            BlindTarget::Room(room) => self
                                .registry
                                .list_room(room)
                                .into_iter()
                                .filter(|d| d.is_cover())
                                .map(|d| d.id)
                                .collect(),
                        };
                        if blinds.is_empty() {
                            return Err("there are no blinds there".into());
                        }
                        for blind in &blinds {
                            self.set(&rule.id, blind, &desired, now).await;
                        }
                    }
                    Action::Notify {
                        body,
                        room,
                        priority,
                    } => match &self.notifier {
                        Some(n) => n.notify(body, room.as_deref(), *priority).await,
                        None => return Err("Niles has nobody to say it through".into()),
                    },
                    Action::Delay(wait) => {
                        tokio::time::sleep(*wait).await;
                        now += chrono::Duration::from_std(*wait).unwrap_or_default();
                    }
                    Action::If {
                        conditions,
                        then,
                        otherwise,
                    } => {
                        let presence = *lock(&self.presence);
                        let holds = {
                            let ctx = Context {
                                registry: &self.registry,
                                now,
                                tz: self.tz,
                                location: self.location,
                                presence,
                                trigger: Some(trigger),
                            };
                            conditions.iter().all(|c| c.evaluate(&ctx))
                        };
                        let branch = if holds { then } else { otherwise };
                        now = self.perform(rule, branch, trigger, number, now).await?;
                        continue;
                    }
                }
                self.log.step(&rule.id, number, action.done());
            }
            Ok(now)
        })
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
        {
            let mut dedup = lock(&self.dedup);
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
        }
        self.device_sink.set(device, desired).await;
    }

    /// Spawn the tasks that drive the engine: one following the bus,
    /// one following the clock, on the minute.
    pub fn spawn(self: Arc<Self>, bus: EventBus) -> JoinHandle<()> {
        let clock = self.clone();
        tokio::spawn(async move {
            loop {
                let now = Utc::now();
                let into =
                    u64::from(now.second()) * 1000 + u64::from(now.timestamp_subsec_millis());
                tokio::time::sleep(std::time::Duration::from_millis(60_000 - into.min(59_999)))
                    .await;
                clock.tick(Utc::now()).await;
            }
        });
        tokio::spawn(async move {
            let mut rx = bus.subscribe();
            self.seed().await;
            loop {
                match rx.recv().await {
                    Ok(ev) => {
                        self.handle_event(&ev, Utc::now()).await;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("automation subscriber lagged by {n} events");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        })
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runs::Run;
    use chrono::TimeZone;
    use niles_config::AutomationRuleDto;
    use niles_core::{Device, DeviceClass};
    use serde_json::json;
    use std::time::Duration;

    struct RecordingSink {
        calls: Mutex<Vec<(DeviceId, DeviceState)>>,
    }

    #[async_trait::async_trait]
    impl DeviceSink for RecordingSink {
        async fn set(&self, device: &DeviceId, desired: &DeviceState) {
            lock(&self.calls).push((device.clone(), desired.clone()));
        }
    }

    struct RecordingNotifier {
        calls: Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl Notifier for RecordingNotifier {
        async fn notify(&self, body: &str, _room: Option<&str>, _priority: Priority) {
            lock(&self.calls).push(body.to_string());
        }
    }

    struct Test {
        engine: Arc<AutomationEngine>,
        sink: Arc<RecordingSink>,
        said: Arc<RecordingNotifier>,
        registry: Arc<DeviceRegistry>,
    }

    impl Test {
        fn set(&self) -> Vec<String> {
            lock(&self.sink.calls)
                .iter()
                .map(|(id, s)| format!("{id} {}", if s.on == Some(false) { "off" } else { "on" }))
                .collect()
        }
        fn said(&self) -> Vec<String> {
            lock(&self.said.calls).clone()
        }
        fn runs(&self, rule: &str) -> Vec<Run> {
            self.engine.log().runs(rule)
        }
        async fn send(&self, event: Event, now: DateTime<Utc>) {
            for h in self.engine.handle_event(&event, now).await {
                let _ = h.await;
            }
        }
        /// Sent, and not waited for: what a delayed run needs to be
        /// replaced while it waits.
        async fn send_and_go_on(&self, event: Event, now: DateTime<Utc>) -> Handles {
            self.engine.handle_event(&event, now).await
        }
    }

    fn id(s: &str) -> DeviceId {
        DeviceId::parse(&format!("z2m:{s}")).unwrap()
    }

    fn rule(value: serde_json::Value) -> Rule {
        let dto: AutomationRuleDto = serde_json::from_value(value).unwrap();
        Rule::from_dto(&dto, "z2m").unwrap()
    }

    /// The office: a door to the garden, two lights, a plug and a
    /// thermometer, the door shut.
    fn office(rules: Vec<serde_json::Value>) -> Test {
        let registry = Arc::new(DeviceRegistry::new());
        let device = |name: &str, state: DeviceState, class: DeviceClass| {
            registry.upsert(Device::new(id(name), state, class));
        };
        device(
            "office/door",
            DeviceState {
                open: Some(false),
                ..Default::default()
            },
            DeviceClass::Contact,
        );
        for lamp in ["office/go", "office/lightstrip"] {
            device(
                lamp,
                DeviceState {
                    on: Some(true),
                    ..Default::default()
                },
                DeviceClass::Light,
            );
        }
        device(
            "office/charger",
            DeviceState {
                on: Some(true),
                ..Default::default()
            },
            DeviceClass::Outlet,
        );
        device(
            "office/sensor",
            DeviceState {
                temperature_celsius: Some(21.0),
                ..Default::default()
            },
            DeviceClass::Sensor,
        );
        let sink = Arc::new(RecordingSink {
            calls: Mutex::new(Vec::new()),
        });
        let said = Arc::new(RecordingNotifier {
            calls: Mutex::new(Vec::new()),
        });
        let engine = Arc::new(
            AutomationEngine::new(
                rules.into_iter().map(rule).collect(),
                registry.clone(),
                Tz::UTC,
                sink.clone(),
                Some(said.clone()),
            )
            .with_dedup_window(Duration::from_millis(0)),
        );
        Test {
            engine,
            sink,
            said,
            registry,
        }
    }

    fn door(open: bool) -> Event {
        Event::DeviceStateChanged {
            id: id("office/door"),
            state: DeviceState {
                open: Some(open),
                ..Default::default()
            },
        }
    }

    fn noon() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 12, 12, 0, 0).unwrap()
    }

    fn garden_door() -> serde_json::Value {
        json!({
            "id": "garden-door",
            "triggers": [{ "type": "contact", "device": "office/door", "open": true }],
            "actions": [{ "do": "set_room", "room": "office", "on": false }],
        })
    }

    #[tokio::test]
    async fn the_garden_door_turns_off_the_office_lights_only() {
        let t = office(vec![garden_door()]);
        t.engine.seed().await;
        t.send(door(true), noon()).await;
        let mut set = t.set();
        set.sort();
        assert_eq!(set, ["z2m:office/go off", "z2m:office/lightstrip off"]);
        let runs = t.runs("garden-door");
        assert_eq!(runs[0].outcome, Outcome::Done);
        assert_eq!(runs[0].trigger.as_deref(), Some("t1"));
        assert_eq!(runs[0].steps, ["set the office lights off"]);
    }

    #[tokio::test]
    async fn a_door_still_open_does_not_open_again() {
        let t = office(vec![garden_door()]);
        t.engine.seed().await;
        t.send(door(true), noon()).await;
        t.send(door(true), noon() + chrono::Duration::minutes(5))
            .await;
        assert_eq!(t.set().len(), 2, "one opening, two lights");
        t.send(door(false), noon()).await;
        t.send(door(true), noon()).await;
        assert_eq!(t.set().len(), 4);
    }

    #[tokio::test]
    async fn a_door_open_when_niles_starts_has_not_just_opened() {
        let t = office(vec![garden_door()]);
        t.registry.upsert(Device::new(
            id("office/door"),
            DeviceState {
                open: Some(true),
                ..Default::default()
            },
            DeviceClass::Contact,
        ));
        t.engine.seed().await;
        t.send(door(true), noon()).await;
        assert!(t.set().is_empty());
    }

    #[tokio::test]
    async fn a_stopped_run_says_why() {
        let mut night_only = garden_door();
        night_only["conditions"] =
            json!([{ "kind": "time_of_day", "after": "22:00", "before": "06:00" }]);
        let t = office(vec![night_only]);
        t.engine.seed().await;
        t.send(door(true), noon()).await;
        assert!(t.set().is_empty());
        let runs = t.runs("garden-door");
        assert_eq!(runs[0].outcome, Outcome::Stopped);
        assert_eq!(
            runs[0].reason.as_deref(),
            Some("it was not between 22:00 and 06:00")
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_door_left_open_is_said_only_once_it_has_been_open_long_enough() {
        let t = office(vec![json!({
            "id": "left-open",
            "triggers": [{ "type": "contact", "device": "office/door", "open": true, "for": { "minutes": 10 } }],
            "actions": [{ "do": "notify", "body": "The garden door is open" }],
        })]);
        t.engine.seed().await;
        // Opened and shut again within the ten minutes: nothing.
        let waiting = t.send_and_go_on(door(true), noon()).await;
        tokio::time::sleep(Duration::from_secs(5 * 60)).await;
        t.send(door(false), noon()).await;
        for h in waiting {
            let _ = h.await;
        }
        assert!(t.said().is_empty());
        // Left open: said once the ten minutes are up.
        t.send(door(true), noon()).await;
        assert_eq!(t.said(), ["The garden door is open"]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_new_run_replaces_one_waiting_out_a_delay() {
        let t = office(vec![json!({
            "id": "after-closing",
            "triggers": [
                { "id": "closed", "type": "contact", "device": "office/door", "open": false },
                { "id": "opened", "type": "contact", "device": "office/door", "open": true }
            ],
            "actions": [{
                "do": "if",
                "conditions": [{ "kind": "triggered_by", "triggers": ["closed"] }],
                "then": [{ "do": "delay", "minutes": 5 }, { "do": "set_room", "room": "office", "on": false }],
            }],
        })]);
        t.engine.seed().await;
        t.send(door(true), noon()).await;
        // Shut, and opened again within the five minutes.
        let waiting = t.send_and_go_on(door(false), noon()).await;
        tokio::time::sleep(Duration::from_secs(60)).await;
        t.send(door(true), noon()).await;
        for h in waiting {
            let _ = h.await;
        }
        assert!(t.set().is_empty(), "the lights were left alone");
        let outcomes: Vec<_> = t.runs("after-closing").iter().map(|r| r.outcome).collect();
        assert_eq!(outcomes, [Outcome::Done, Outcome::Replaced, Outcome::Done]);
        // Shut and left shut: off after five minutes.
        t.send(door(false), noon()).await;
        assert_eq!(t.set().len(), 2);
        assert_eq!(
            t.runs("after-closing")[0].steps,
            ["waited 5 minutes", "set the office lights off"]
        );
    }

    #[tokio::test]
    async fn one_rule_does_different_things_for_different_triggers() {
        let t = office(vec![json!({
            "id": "door-both-ways",
            "triggers": [
                { "id": "opened", "type": "contact", "device": "office/door", "open": true },
                { "id": "closed", "type": "contact", "device": "office/door", "open": false }
            ],
            "actions": [{
                "do": "if",
                "conditions": [{ "kind": "triggered_by", "triggers": ["opened"] }],
                "then": [{ "do": "notify", "body": "opened" }],
                "else": [{ "do": "notify", "body": "closed" }],
            }],
        })]);
        t.engine.seed().await;
        t.send(door(true), noon()).await;
        t.send(door(false), noon()).await;
        assert_eq!(t.said(), ["opened", "closed"]);
    }

    #[tokio::test]
    async fn a_reading_crossing_a_line_fires_once() {
        let t = office(vec![json!({
            "id": "too-warm",
            "triggers": [{ "type": "numeric", "device": "office/sensor", "measure": "temperature", "above": 24 }],
            "actions": [{ "do": "notify", "body": "warm" }],
        })]);
        t.engine.seed().await;
        let at = |c: f32| Event::DeviceStateChanged {
            id: id("office/sensor"),
            state: DeviceState {
                temperature_celsius: Some(c),
                ..Default::default()
            },
        };
        t.send(at(23.0), noon()).await;
        t.send(at(24.5), noon()).await;
        t.send(at(25.0), noon()).await;
        assert_eq!(t.said(), ["warm"], "crossing once, staying above");
        t.send(at(22.0), noon()).await;
        t.send(at(24.2), noon()).await;
        assert_eq!(t.said().len(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn a_price_reading_fires_once_it_has_stayed_low_long_enough() {
        let t = office(vec![json!({
            "id": "cheap-power",
            "triggers": [{
                "type": "numeric", "device": "office/meter", "measure": "price",
                "below": 1, "for": { "minutes": 15 }
            }],
            "actions": [{ "do": "notify", "body": "cheap" }],
        })]);
        let reading = |name: &str, value: f64, unit: &str| DeviceState {
            readings: [(
                name.to_string(),
                niles_core::Reading::new(value, Some(unit.into())),
            )]
            .into(),
            ..Default::default()
        };
        t.registry.upsert(Device::new(
            id("office/meter"),
            reading("price", 1.4, "kr/kWh"),
            DeviceClass::Sensor,
        ));
        t.engine.seed().await;
        let price = |kr: f64| Event::DeviceStateChanged {
            id: id("office/meter"),
            state: reading("price", kr, "kr/kWh"),
        };

        // Dipped, and back up within the quarter of an hour: nothing.
        let waiting = t.send_and_go_on(price(0.9), noon()).await;
        tokio::time::sleep(Duration::from_secs(5 * 60)).await;
        t.send(price(1.2), noon()).await;
        for h in waiting {
            let _ = h.await;
        }
        assert!(t.said().is_empty());

        // Down and staying down. Another reading arriving meanwhile says
        // nothing about the price, and must not restart the wait.
        let waiting = t.send_and_go_on(price(0.8), noon()).await;
        t.send(
            Event::DeviceStateChanged {
                id: id("office/meter"),
                state: reading("power", 300.0, "W"),
            },
            noon(),
        )
        .await;
        for h in waiting {
            let _ = h.await;
        }
        assert_eq!(t.said(), ["cheap"]);
        // Still low: not again.
        t.send(price(0.7), noon()).await;
        assert_eq!(t.said(), ["cheap"]);
    }

    #[tokio::test]
    async fn the_clock_fires_once_on_its_minute() {
        let t = office(vec![json!({
            "id": "bedtime",
            "triggers": [{ "type": "time", "at": "22:00" }],
            "actions": [{ "do": "notify", "body": "bedtime" }],
        })]);
        let at = |h: u32, m: u32, s: u32| Utc.with_ymd_and_hms(2026, 10, 12, h, m, s).unwrap();
        for when in [at(21, 59, 0), at(22, 0, 0), at(22, 0, 30)] {
            for h in t.engine.tick(when).await {
                let _ = h.await;
            }
        }
        assert_eq!(t.said(), ["bedtime"]);
    }

    #[tokio::test]
    async fn sunset_needs_a_house_to_set_over() {
        let sunset = json!({
            "id": "porch",
            "triggers": [{ "type": "sun", "event": "sunset" }],
            "actions": [{ "do": "notify", "body": "sunset" }],
        });
        let t = office(vec![sunset.clone()]);
        let due = rule(sunset).triggers[0]
            .trigger
            .due_on(noon(), Tz::UTC, Location::of(56.1572, 10.2107))
            .unwrap();
        let at = Utc.from_utc_datetime(&due);
        // Without a location the engine never fires it.
        assert!(t.engine.tick(at).await.is_empty());
    }

    #[tokio::test]
    async fn presence_is_read_from_the_bus() {
        let t = office(vec![json!({
            "id": "welcome",
            "triggers": [{ "type": "contact", "device": "office/door", "open": true }],
            "conditions": [{ "kind": "presence_is", "state": "home" }],
            "actions": [{ "do": "notify", "body": "welcome" }],
        })]);
        t.engine.seed().await;
        t.send(door(true), noon()).await;
        assert!(t.said().is_empty(), "nobody known to be home");
        t.send(
            Event::PresenceChanged {
                state: PresenceState::Home,
            },
            noon(),
        )
        .await;
        t.send(door(false), noon()).await;
        t.send(door(true), noon()).await;
        assert_eq!(t.said(), ["welcome"]);
    }

    #[tokio::test]
    async fn new_rules_run_in_place_of_the_old() {
        let t = office(vec![]);
        t.engine.seed().await;
        t.engine.replace(vec![rule(garden_door())]).await;
        t.send(door(true), noon()).await;
        assert_eq!(t.set().len(), 2);
    }

    #[tokio::test]
    async fn a_button_press_is_a_moment() {
        let t = office(vec![json!({
            "id": "press",
            "triggers": [{ "type": "device_action", "device": "office/switch", "action": "on_press" }],
            "actions": [{ "do": "notify", "body": "pressed" }],
        })]);
        let press = Event::DeviceAction {
            id: id("office/switch"),
            action: "on_press".into(),
        };
        t.send(press.clone(), noon()).await;
        t.send(press, noon()).await;
        assert_eq!(t.said().len(), 2, "every press counts");
    }
}
