//! Automation engine: follows the event bus and the clock, starts
//! rules, and runs their actions through injected `DeviceSink` and
//! `Notifier` traits.
//!
//! A trigger that follows a state fires on the change into it — a
//! sensor repeating itself in every battery report has not changed —
//! and, with `for`, only once the state has held that long. Each rule
//! runs once at a time: a trigger firing again while a run waits out a
//! delay replaces that run.

use crate::rule::{Action, BlindTarget, Context, Priority, Rule, WakePlan};
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

/// Starts the morning routine for a rule's `wake_up`. Answers what it
/// did, for the run's history, or why it could not.
#[async_trait::async_trait]
pub trait Waker: Send + Sync {
    async fn wake_up(&self, plan: &WakePlan) -> Result<String, String>;
}

/// Where a run waiting out a `delay` is kept, so the wait outlives a
/// restart. One per rule: a rule runs once at a time.
#[async_trait::async_trait]
pub trait WaitStore: Send + Sync {
    async fn keep(&self, wait: &Waiting);
    async fn forget(&self, rule: &str);
    async fn all(&self) -> Vec<Waiting>;
}

/// A run, waiting out a delay: enough to carry on from there.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Waiting {
    pub rule: String,
    /// The rule as it was: a wait carries on only if it still is.
    pub fingerprint: String,
    /// The trigger that started the run, for `triggered_by` after it.
    pub trigger: String,
    /// Where in the rule's actions the delay is.
    pub at: Vec<Step>,
    /// When the delay ends.
    pub until: DateTime<Utc>,
}

/// One step into a rule's actions: which action, and, for an `if`,
/// which way it went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Step {
    pub index: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<bool>,
}

/// Where a run picks up: the steps to its delay, and when that ends.
type Resume = (Vec<Step>, DateTime<Utc>);

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
    waker: Option<Arc<dyn Waker>>,
    waits: Option<Arc<dyn WaitStore>>,
    /// The live config, for how far "open" opens. Without it, "open"
    /// means all the way.
    settings: Option<Arc<niles_config::ConfigStore>>,
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
    /// Set by [`Self::stop`]; nothing starts once it is.
    stopping: std::sync::atomic::AtomicBool,
    log: Arc<RunLog>,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct DedupKey {
    rule_id: String,
    device: DeviceId,
    on: Option<bool>,
    brightness: Option<u8>,
    kelvin: Option<u16>,
    position: Option<u8>,
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
            waker: None,
            waits: None,
            settings: None,
            dedup: Mutex::new(HashMap::new()),
            dedup_window: chrono::Duration::milliseconds(2_000),
            observed: Mutex::new(HashMap::new()),
            holds: Mutex::new(HashMap::new()),
            presence: Mutex::new(None),
            running: Mutex::new(HashMap::new()),
            clock: Mutex::new(HashMap::new()),
            stopping: std::sync::atomic::AtomicBool::new(false),
            log: Arc::new(RunLog::new()),
        }
    }

    /// Where the house is, for the sun. Without it, sun triggers never
    /// fire and sun conditions never hold.
    pub fn with_location(mut self, location: Option<Location>) -> Self {
        self.location = location;
        self
    }

    /// What starts a wake-up. Without one, a `wake_up` fails its run.
    pub fn with_waker(mut self, waker: Option<Arc<dyn Waker>>) -> Self {
        self.waker = waker;
        self
    }

    /// Where delays are kept over a restart. Without one, a restart
    /// forgets them.
    pub fn with_waits(mut self, waits: Option<Arc<dyn WaitStore>>) -> Self {
        self.waits = waits;
        self
    }

    /// The live config, for how far "open" opens. Without it, "open"
    /// means all the way.
    pub fn with_settings(mut self, settings: Option<Arc<niles_config::ConfigStore>>) -> Self {
        self.settings = settings;
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

    /// Stop, for a restart or a shutdown.
    ///
    /// No run starts from here on, and each run in progress stops where
    /// it is. A run waiting out a `delay` keeps its wait in the store,
    /// since nothing here forgets it, and [`Self::resume`] carries on
    /// from it when Niles is back. Called before MQTT closes, so a delay
    /// that ends during a shutdown cannot run its last steps into a
    /// closed broker and then forget its wait.
    pub fn stop(&self) {
        use std::sync::atomic::Ordering;
        self.stopping.store(true, Ordering::SeqCst);
        let holds: Vec<AbortHandle> = lock(&self.holds).drain().map(|(_, h)| h).collect();
        for handle in holds {
            handle.abort();
        }
        let running: Vec<AbortHandle> = lock(&self.running).drain().map(|(_, (_, h))| h).collect();
        for handle in running {
            handle.abort();
        }
    }

    /// Run `rules` from now on, in place of the ones before. A run or a
    /// `for` of a rule that is gone or paused stops; the states the new
    /// rules follow are read from the registry, so a change already
    /// made does not count as one.
    pub async fn replace(&self, rules: Vec<Rule>) {
        let rules: Vec<Arc<Rule>> = rules.into_iter().map(Arc::new).collect();
        let live = |id: &str| rules.iter().any(|r| r.id == id && r.enabled);
        let mut stopped = Vec::new();
        lock(&self.running).retain(|rule, (_, handle)| {
            let keep = live(rule);
            if !keep {
                handle.abort();
                stopped.push(rule.clone());
            }
            keep
        });
        if let Some(waits) = &self.waits {
            for rule in &stopped {
                waits.forget(rule).await;
            }
        }
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
        if self.stopping.load(std::sync::atomic::Ordering::SeqCst) {
            return Vec::new();
        }
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
                let Some((subject, holds)) = t.trigger.observe(event, &self.registry) else {
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
                    Some(wait) => handles.extend(self.hold(rule.clone(), &t.id, key, wait, now)),
                }
            }
        }
        handles
    }

    /// Fire the clock's triggers due this minute.
    pub async fn tick(self: &Arc<Self>, now: DateTime<Utc>) -> Handles {
        if self.stopping.load(std::sync::atomic::Ordering::SeqCst) {
            return Vec::new();
        }
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
    ) -> Option<JoinHandle<()>> {
        let engine = self.clone();
        let trigger = trigger.to_string();
        let held = key.clone();
        let mut holds = lock(&self.holds);
        if self.stopping.load(std::sync::atomic::Ordering::SeqCst) {
            return None;
        }
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
        Some(task)
    }

    /// Start a run of `rule`, started by `trigger`: unless a condition
    /// stops it, which is a run too, kept with the reason.
    fn start(
        self: &Arc<Self>,
        rule: &Arc<Rule>,
        trigger: &str,
        now: DateTime<Utc>,
    ) -> Option<JoinHandle<()>> {
        if self.stopping.load(std::sync::atomic::Ordering::SeqCst) {
            return None;
        }
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
        self.launch(rule, trigger, now, None)
    }

    /// Run `rule`'s actions: from the start, or from a
    /// delay it was waiting out when Niles stopped. Replaces the rule's
    /// run in progress, if there is one.
    fn launch(
        self: &Arc<Self>,
        rule: &Arc<Rule>,
        trigger: &str,
        now: DateTime<Utc>,
        resume: Option<Resume>,
    ) -> Option<JoinHandle<()>> {
        let engine = self.clone();
        let run_rule = rule.clone();
        let trigger = trigger.to_string();
        // Held while the run is spawned and noted, so a run that ends at
        // once cannot clear itself before it is there to be cleared.
        let mut running = lock(&self.running);
        // Check under the same lock stop drains: an in-flight trigger
        // must either be registered in time to abort, or not launch.
        if self.stopping.load(std::sync::atomic::Ordering::SeqCst) {
            return None;
        }
        let number = self
            .log
            .begin(&rule.id, Some(&trigger), now, Outcome::Running, None);
        let task = tokio::spawn(async move {
            // A run this one replaced may have left its wait behind.
            if resume.is_none()
                && let Some(waits) = &engine.waits
            {
                waits.forget(&run_rule.id).await;
            }
            let (from, until) = match &resume {
                Some((steps, until)) => (Some(steps.as_slice()), Some(*until)),
                None => (None, None),
            };
            let ended = engine
                .perform(
                    &run_rule,
                    &run_rule.actions,
                    &trigger,
                    number,
                    now,
                    Vec::new(),
                    from,
                    until,
                )
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

    /// Carry on with the runs that were waiting out a delay when Niles
    /// stopped: the rest of the wait, or at once if it is over. A wait
    /// whose rule was changed, paused or removed is dropped, and its run
    /// says so.
    pub async fn resume(self: &Arc<Self>, now: DateTime<Utc>) -> Handles {
        if self.stopping.load(std::sync::atomic::Ordering::SeqCst) {
            return Vec::new();
        }
        let Some(waits) = self.waits.clone() else {
            return Vec::new();
        };
        let rules = self.rules.read().await.clone();
        let mut handles = Vec::new();
        for wait in waits.all().await {
            if self.stopping.load(std::sync::atomic::Ordering::SeqCst) {
                break;
            }
            let rule = rules
                .iter()
                .find(|r| r.id == wait.rule && r.enabled && r.fingerprint == wait.fingerprint);
            let Some(rule) = rule else {
                tracing::info!(
                    "automation '{}': a wait from before the restart was dropped",
                    wait.rule
                );
                self.log.begin(
                    &wait.rule,
                    Some(&wait.trigger),
                    now,
                    Outcome::Stopped,
                    Some("it was waiting when Niles restarted, and the rule had changed".into()),
                );
                waits.forget(&wait.rule).await;
                continue;
            };
            tracing::info!(
                "automation '{}': carrying on after the restart, until {}",
                rule.id,
                wait.until
            );
            handles.extend(self.launch(
                rule,
                &wait.trigger,
                now,
                Some((wait.at.clone(), wait.until)),
            ));
        }
        handles
    }

    /// Run `actions` in order, waiting where a delay says. Returns the
    /// time it got to, for the conditions of an `if` after a delay.
    ///
    /// `at` is where `actions` sits in the rule. `from`, picking up after
    /// a restart, is the way to the delay the run was waiting out, which
    /// ends at `until`: everything before it is skipped, and the `if`s on
    /// the way go the way they went.
    #[allow(clippy::too_many_arguments)]
    #[expect(
        clippy::too_many_lines,
        reason = "one arm per action kind, all sharing the resume-after-restart position"
    )]
    fn perform<'a>(
        self: &'a Arc<Self>,
        rule: &'a Rule,
        actions: &'a [Action],
        trigger: &'a str,
        number: u64,
        mut now: DateTime<Utc>,
        at: Vec<Step>,
        from: Option<&'a [Step]>,
        until: Option<DateTime<Utc>>,
    ) -> Pin<Box<dyn Future<Output = Result<DateTime<Utc>, String>> + Send + 'a>> {
        Box::pin(async move {
            let first = from.and_then(|f| f.first()).map_or(0, |s| s.index);
            for (index, action) in actions.iter().enumerate().skip(first) {
                let resuming = from.filter(|_| index == first);
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
                        // sensor that may have started this, nor a light
                        // its source keeps out of the room's.
                        let lights: Vec<DeviceId> = self
                            .registry
                            .list_room(room)
                            .into_iter()
                            .filter(|d| d.is_light() && d.is_switchable() && d.room_light)
                            .map(|d| d.id)
                            .collect();
                        if lights.is_empty() {
                            return Err(format!("there are no lights in {room}"));
                        }
                        for light in &lights {
                            self.set(&rule.id, light, &desired, now).await;
                        }
                    }
                    Action::SetBlinds { target, level } => {
                        let blinds: Vec<(DeviceId, Option<u8>)> = match target {
                            // Still sent to a device the registry has not
                            // seen: the topic comes from the id, and a
                            // rule must survive running before Z2M's
                            // device list has arrived.
                            BlindTarget::Device(id) => vec![(
                                id.clone(),
                                self.registry.get(id).and_then(|d| d.state.position),
                            )],
                            BlindTarget::Room(room) => self
                                .registry
                                .list_room(room)
                                .into_iter()
                                .filter(|d| d.is_cover())
                                .map(|d| (d.id, d.state.position))
                                .collect(),
                        };
                        if blinds.is_empty() {
                            return Err("there are no blinds there".into());
                        }
                        let snapshot = self.settings.as_ref().map(|s| s.current());
                        let fallback = niles_config::BlindsConfig::default();
                        let cfg = snapshot.as_deref().map(|c| &c.blinds).unwrap_or(&fallback);
                        let resolved = cfg.targets(*level, blinds.iter().map(|(id, at)| (id, *at)));
                        for ((id, _), position) in blinds.iter().zip(&resolved.positions) {
                            let desired = DeviceState {
                                position: Some(*position),
                                ..Default::default()
                            };
                            self.set(&rule.id, id, &desired, now).await;
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
                    Action::WakeUp(plan) => match &self.waker {
                        Some(waker) => {
                            let did = waker.wake_up(plan).await?;
                            self.log.step(&rule.id, number, did);
                            continue;
                        }
                        None => return Err("Niles has no lights to wake anybody with".into()),
                    },
                    Action::Delay(wait) => {
                        let ends = match (resuming, until) {
                            (Some(_), Some(until)) => until,
                            _ => now + chrono::Duration::from_std(*wait).unwrap_or_default(),
                        };
                        let mut here = at.clone();
                        here.push(Step {
                            index,
                            branch: None,
                        });
                        if let Some(waits) = &self.waits {
                            waits
                                .keep(&Waiting {
                                    rule: rule.id.clone(),
                                    fingerprint: rule.fingerprint.clone(),
                                    trigger: trigger.to_string(),
                                    at: here,
                                    until: ends,
                                })
                                .await;
                        }
                        let left = (ends - Utc::now()).to_std().unwrap_or_default();
                        // A wait picked up after a restart has only the
                        // rest to go; a fresh one, all of it.
                        let left = if resuming.is_some() { left } else { *wait };
                        tokio::time::sleep(left).await;
                        if let Some(waits) = &self.waits {
                            waits.forget(&rule.id).await;
                        }
                        now = ends;
                    }
                    Action::If {
                        conditions,
                        then,
                        otherwise,
                    } => {
                        // Picking up inside a branch: it went this way
                        // before the restart, and goes the same way now.
                        let deeper = resuming.filter(|steps| steps.len() > 1);
                        let holds = match deeper {
                            Some(steps) => steps[0].branch.unwrap_or(true),
                            None => {
                                let presence = *lock(&self.presence);
                                let ctx = Context {
                                    registry: &self.registry,
                                    now,
                                    tz: self.tz,
                                    location: self.location,
                                    presence,
                                    trigger: Some(trigger),
                                };
                                conditions.iter().all(|c| c.evaluate(&ctx))
                            }
                        };
                        let branch = if holds { then } else { otherwise };
                        let mut here = at.clone();
                        here.push(Step {
                            index,
                            branch: Some(holds),
                        });
                        now = self
                            .perform(
                                rule,
                                branch,
                                trigger,
                                number,
                                now,
                                here,
                                deeper.map(|steps| &steps[1..]),
                                until,
                            )
                            .await?;
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
            position: desired.position,
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
            self.resume(Utc::now()).await;
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
        office_with(rules, |engine| engine)
    }

    /// The office, with more given to the engine.
    fn office_with(
        rules: Vec<serde_json::Value>,
        more: impl FnOnce(AutomationEngine) -> AutomationEngine,
    ) -> Test {
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
        let engine = Arc::new(more(
            AutomationEngine::new(
                rules.into_iter().map(rule).collect(),
                registry.clone(),
                Tz::UTC,
                sink.clone(),
                Some(said.clone()),
            )
            .with_dedup_window(Duration::from_millis(0)),
        ));
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
    async fn a_light_kept_out_of_the_room_is_not_one_of_its_lights() {
        let t = office(vec![garden_door()]);
        let mut strip = t.registry.get(&id("office/lightstrip")).unwrap();
        strip.room_light = false;
        t.registry.upsert(strip);
        t.engine.seed().await;
        t.send(door(true), noon()).await;
        assert_eq!(t.set(), ["z2m:office/go off"]);
    }

    #[tokio::test]
    async fn any_device_can_start_a_rule_with_an_action() {
        let t = office(vec![json!({
            "id": "print-done",
            "triggers": [{ "type": "device_action", "device": "office/sensor", "action": "finished" }],
            "actions": [{ "do": "set_room", "room": "office", "on": false }],
        })]);
        t.engine.seed().await;
        let action = |action: &str| Event::DeviceAction {
            id: id("office/sensor"),
            action: action.into(),
        };
        t.send(action("failed"), noon()).await;
        assert!(t.set().is_empty());
        t.send(action("finished"), noon()).await;
        assert_eq!(t.set().len(), 2);
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

    /// The hallway and the kitchen share a light: each room's motion
    /// turns its lights on, and both go off once neither has seen
    /// anybody for two minutes.
    fn shared_lights() -> serde_json::Value {
        json!({
            "id": "shared",
            "triggers": [
                { "id": "hall-seen", "type": "motion", "device": "hallway/motion", "detected": true },
                { "id": "kitchen-seen", "type": "motion", "device": "kitchen/motion", "detected": true },
                { "id": "all-clear", "type": "motion", "devices": ["hallway/motion", "kitchen/motion"], "detected": false, "for": { "minutes": 2 } }
            ],
            "actions": [
                { "do": "if", "conditions": [{ "kind": "triggered_by", "triggers": ["hall-seen"] }],
                  "then": [{ "do": "set_room", "room": "hallway", "on": true }] },
                { "do": "if", "conditions": [{ "kind": "triggered_by", "triggers": ["kitchen-seen"] }],
                  "then": [{ "do": "set_room", "room": "kitchen", "on": true }, { "do": "set_room", "room": "hallway", "on": true }] },
                { "do": "if", "conditions": [{ "kind": "triggered_by", "triggers": ["all-clear"] }],
                  "then": [{ "do": "set_room", "room": "kitchen", "on": false }, { "do": "set_room", "room": "hallway", "on": false }] }
            ],
        })
    }

    /// The office, with a hallway and a kitchen beside it: a motion
    /// sensor and a ceiling light in each, the sensors as given.
    async fn hallway_and_kitchen(rule: serde_json::Value, hallway: bool, kitchen: bool) -> Test {
        let t = office(vec![rule]);
        for (room, motion) in [("hallway", hallway), ("kitchen", kitchen)] {
            t.registry.upsert(Device::new(
                id(&format!("{room}/motion")),
                DeviceState {
                    motion: Some(motion),
                    ..Default::default()
                },
                DeviceClass::Motion,
            ));
            t.registry.upsert(Device::new(
                id(&format!("{room}/ceiling")),
                DeviceState {
                    on: Some(true),
                    ..Default::default()
                },
                DeviceClass::Light,
            ));
        }
        t.engine.seed().await;
        t
    }

    impl Test {
        /// A sensor's report, as a source makes it: into the registry,
        /// then onto the bus. Not waited for, so a `for` can run on.
        async fn motion(&self, room: &str, motion: bool) {
            let sensor = id(&format!("{room}/motion"));
            let state = DeviceState {
                motion: Some(motion),
                ..Default::default()
            };
            self.registry.merge_state(&sensor, state.clone());
            let event = Event::DeviceStateChanged { id: sensor, state };
            drop(self.engine.handle_event(&event, noon()).await);
        }

        fn turned_off(&self) -> Vec<String> {
            let mut off: Vec<String> = self
                .set()
                .into_iter()
                .filter(|s| s.ends_with(" off"))
                .collect();
            off.sort();
            off
        }
    }

    /// From the start of the test, on the paused clock.
    async fn until(start: tokio::time::Instant, secs: u64) {
        tokio::time::sleep_until(start + Duration::from_secs(secs)).await;
    }

    const BOTH_OFF: [&str; 2] = ["z2m:hallway/ceiling off", "z2m:kitchen/ceiling off"];

    #[tokio::test(start_paused = true)]
    async fn moving_between_the_rooms_keeps_the_lights_on_until_both_are_clear() {
        let t = hallway_and_kitchen(shared_lights(), false, true).await;
        let start = tokio::time::Instant::now();
        until(start, 10).await;
        t.motion("kitchen", false).await;
        until(start, 100).await;
        t.motion("hallway", true).await;
        until(start, 131).await;
        assert!(t.turned_off().is_empty(), "the hallway saw somebody at 100");
        until(start, 150).await;
        t.motion("hallway", false).await;
        until(start, 269).await;
        assert!(t.turned_off().is_empty());
        until(start, 271).await;
        assert_eq!(
            t.turned_off(),
            BOTH_OFF,
            "two minutes after the last one cleared"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_two_minutes_count_from_the_last_room_to_clear() {
        let t = hallway_and_kitchen(shared_lights(), true, true).await;
        let start = tokio::time::Instant::now();
        t.motion("kitchen", false).await;
        until(start, 119).await;
        t.motion("hallway", false).await;
        until(start, 121).await;
        assert!(
            t.turned_off().is_empty(),
            "the kitchen alone has been clear for two minutes"
        );
        until(start, 238).await;
        assert!(t.turned_off().is_empty());
        until(start, 240).await;
        assert_eq!(t.turned_off(), BOTH_OFF);
    }

    #[tokio::test(start_paused = true)]
    async fn sensors_repeating_that_they_are_clear_do_not_start_the_count_again() {
        let t = hallway_and_kitchen(shared_lights(), false, true).await;
        let start = tokio::time::Instant::now();
        t.motion("kitchen", false).await;
        until(start, 60).await;
        t.motion("kitchen", false).await;
        t.motion("hallway", false).await;
        until(start, 121).await;
        assert_eq!(t.turned_off(), BOTH_OFF);
        let runs = t.runs("shared");
        assert_eq!(runs.len(), 1, "one run, of all-clear");
        assert_eq!(runs[0].trigger.as_deref(), Some("all-clear"));
    }

    #[tokio::test(start_paused = true)]
    async fn rooms_already_clear_when_niles_starts_have_not_just_cleared() {
        let t = hallway_and_kitchen(shared_lights(), false, false).await;
        let start = tokio::time::Instant::now();
        t.motion("hallway", false).await;
        until(start, 300).await;
        assert!(t.turned_off().is_empty(), "nothing changed");
        t.motion("kitchen", true).await;
        t.motion("kitchen", false).await;
        until(start, 421).await;
        assert_eq!(t.turned_off(), BOTH_OFF);
    }

    #[tokio::test(start_paused = true)]
    async fn a_sensor_never_heard_from_keeps_the_lights_on() {
        let t = hallway_and_kitchen(shared_lights(), true, false).await;
        // The kitchen sensor, re-paired, has said nothing since.
        t.registry.upsert(Device::new(
            id("kitchen/motion"),
            DeviceState::default(),
            DeviceClass::Motion,
        ));
        let start = tokio::time::Instant::now();
        t.motion("hallway", false).await;
        until(start, 300).await;
        assert!(t.turned_off().is_empty());
        t.motion("kitchen", false).await;
        until(start, 421).await;
        assert_eq!(t.turned_off(), BOTH_OFF);
    }

    #[tokio::test(start_paused = true)]
    async fn a_rule_for_one_sensor_runs_as_it_always_has() {
        let t = hallway_and_kitchen(
            json!({
                "id": "hallway",
                "triggers": [
                    { "id": "seen", "type": "motion", "device": "hallway/motion", "detected": true },
                    { "id": "quiet", "type": "motion", "device": "hallway/motion", "detected": false, "for": { "minutes": 2 } }
                ],
                "actions": [{
                    "do": "if",
                    "conditions": [{ "kind": "triggered_by", "triggers": ["seen"] }],
                    "then": [{ "do": "set_room", "room": "hallway", "on": true }],
                    "else": [{ "do": "set_room", "room": "hallway", "on": false }],
                }],
            }),
            false,
            false,
        )
        .await;
        let start = tokio::time::Instant::now();
        t.motion("hallway", true).await;
        until(start, 1).await;
        assert_eq!(t.set(), ["z2m:hallway/ceiling on"]);
        t.motion("hallway", false).await;
        until(start, 60).await;
        t.motion("hallway", true).await;
        t.motion("hallway", false).await;
        until(start, 179).await;
        assert!(
            t.turned_off().is_empty(),
            "the motion at 60 started it over"
        );
        until(start, 181).await;
        assert_eq!(t.turned_off(), ["z2m:hallway/ceiling off"]);
    }

    /// Waits kept in memory, standing in for the database.
    #[derive(Default)]
    struct MemoryWaits(Mutex<HashMap<String, Waiting>>);

    #[async_trait::async_trait]
    impl WaitStore for MemoryWaits {
        async fn keep(&self, wait: &Waiting) {
            lock(&self.0).insert(wait.rule.clone(), wait.clone());
        }
        async fn forget(&self, rule: &str) {
            lock(&self.0).remove(rule);
        }
        async fn all(&self) -> Vec<Waiting> {
            lock(&self.0).values().cloned().collect()
        }
    }

    #[derive(Default)]
    struct RecordingWaker(Mutex<Vec<WakePlan>>);

    #[async_trait::async_trait]
    impl Waker for RecordingWaker {
        async fn wake_up(&self, plan: &WakePlan) -> Result<String, String> {
            lock(&self.0).push(plan.clone());
            Ok(format!("started a {}-minute wake-up", plan.minutes))
        }
    }

    fn seven_hours_after_closing() -> serde_json::Value {
        json!({
            "id": "wake",
            "triggers": [{ "id": "closed", "type": "contact", "device": "office/door", "open": false }],
            "actions": [
                { "do": "delay", "hours": 7 },
                { "do": "set_room", "room": "office", "on": false }
            ],
        })
    }

    #[tokio::test(start_paused = true)]
    async fn a_delay_is_kept_while_it_waits_and_forgotten_after() {
        let waits = Arc::new(MemoryWaits::default());
        let kept = waits.clone();
        let t = office_with(vec![seven_hours_after_closing()], move |e| {
            e.with_waits(Some(kept as Arc<dyn WaitStore>))
        });
        t.engine.seed().await;
        t.send(door(true), noon()).await;
        let waiting = t.send_and_go_on(door(false), noon()).await;
        tokio::time::sleep(Duration::from_secs(60)).await;
        let kept = waits.all().await;
        assert_eq!(kept.len(), 1);
        assert_eq!(
            kept[0].at,
            [Step {
                index: 0,
                branch: None
            }]
        );
        assert_eq!(kept[0].trigger, "closed");
        assert_eq!(kept[0].until, noon() + chrono::Duration::hours(7));
        for h in waiting {
            let _ = h.await;
        }
        assert!(waits.all().await.is_empty(), "forgotten once over");
        assert_eq!(t.set().len(), 2);
    }

    #[tokio::test]
    async fn stopping_while_resume_waits_does_not_launch_or_forget_the_wait() {
        let waits = Arc::new(MemoryWaits::default());
        let kept = waits.clone();
        let t = office_with(vec![seven_hours_after_closing()], move |e| {
            e.with_waits(Some(kept as Arc<dyn WaitStore>))
        });
        let rules = t.engine.rules.write().await;
        waits
            .keep(&Waiting {
                rule: "wake".into(),
                fingerprint: rules[0].fingerprint.clone(),
                trigger: "closed".into(),
                at: vec![Step {
                    index: 0,
                    branch: None,
                }],
                until: noon(),
            })
            .await;
        let mut resume = std::pin::pin!(t.engine.resume(Utc::now()));
        std::future::poll_fn(|cx| {
            assert!(resume.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;

        t.engine.stop();
        drop(rules);
        for handle in resume.await {
            handle.await.unwrap();
        }
        assert!(
            t.set().is_empty(),
            "a resume already in flight must stop too"
        );
        assert_eq!(
            waits.all().await.len(),
            1,
            "the next process still needs its wait"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn stopping_keeps_a_waiting_run_for_after_the_restart() {
        // The order of a shutdown promises automations stop before the
        // broker closes: a run cut mid-wait must keep its wait, or a
        // delay that ends during the shutdown would run its last steps
        // into a closed broker and then forget itself.
        let waits = Arc::new(MemoryWaits::default());
        let kept = waits.clone();
        let t = office_with(vec![seven_hours_after_closing()], move |e| {
            e.with_waits(Some(kept as Arc<dyn WaitStore>))
        });
        t.engine.seed().await;
        t.send(door(true), noon()).await;
        let _waiting = t.send_and_go_on(door(false), noon()).await;
        tokio::time::sleep(Duration::from_secs(60)).await;
        assert_eq!(waits.all().await.len(), 1, "the run is in its delay");

        t.engine.stop();
        // The delay is up while Niles is down: nothing may run, and the
        // wait is still there for the next start to carry on from.
        tokio::time::sleep(Duration::from_secs(8 * 60 * 60)).await;
        assert!(
            t.set().is_empty(),
            "the delay is not up and the run was stopped: nothing ran"
        );
        assert_eq!(
            waits.all().await.len(),
            1,
            "stopping forgets nothing that was kept"
        );

        // A fresh engine on the same store carries the run on.
        let carried = waits.clone();
        let t = office_with(vec![seven_hours_after_closing()], move |e| {
            e.with_waits(Some(carried as Arc<dyn WaitStore>))
        });
        for h in t.engine.resume(Utc::now()).await {
            let _ = h.await;
        }
        assert_eq!(t.set().len(), 2, "the rest of the run, from the keep");
        assert!(waits.all().await.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn nothing_starts_once_stopped() {
        let t = office_with(vec![seven_hours_after_closing()], |e| e);
        t.engine.seed().await;
        t.engine.stop();
        t.send(door(true), noon()).await;
        t.send(door(false), noon()).await;
        tokio::time::sleep(Duration::from_secs(60)).await;
        assert!(
            t.runs("wake").is_empty(),
            "an event after the stop starts no run"
        );
        assert!(t.set().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_wait_carries_on_after_a_restart() {
        let waits = Arc::new(MemoryWaits::default());
        let rule = rule(seven_hours_after_closing());
        // The night: Niles stopped five hours in, and is back after the
        // seven were up.
        waits
            .keep(&Waiting {
                rule: "wake".into(),
                fingerprint: rule.fingerprint.clone(),
                trigger: "closed".into(),
                at: vec![Step {
                    index: 0,
                    branch: None,
                }],
                until: noon() - chrono::Duration::minutes(10),
            })
            .await;
        let kept = waits.clone();
        let t = office_with(vec![seven_hours_after_closing()], move |e| {
            e.with_waits(Some(kept as Arc<dyn WaitStore>))
        });
        for h in t.engine.resume(noon()).await {
            let _ = h.await;
        }
        assert_eq!(t.set().len(), 2, "the rest of the run, at once");
        assert!(waits.all().await.is_empty());
        let runs = t.runs("wake");
        assert_eq!(runs[0].outcome, Outcome::Done);
        assert_eq!(runs[0].trigger.as_deref(), Some("closed"));
    }

    #[tokio::test(start_paused = true)]
    async fn a_wait_inside_a_branch_goes_the_way_it_went() {
        let branching = json!({
            "id": "after-closing",
            "triggers": [
                { "id": "closed", "type": "contact", "device": "office/door", "open": false },
                { "id": "opened", "type": "contact", "device": "office/door", "open": true }
            ],
            "actions": [{
                "do": "if",
                "conditions": [{ "kind": "triggered_by", "triggers": ["opened"] }],
                "then": [{ "do": "delay", "hours": 1 }, { "do": "set_room", "room": "office", "on": false }],
                "else": [{ "do": "notify", "body": "Closed" }],
            }],
        });
        let waits = Arc::new(MemoryWaits::default());
        // Kept as started by "opened", which the condition asks for; the
        // trigger is kept with it, but the way it went is what counts.
        waits
            .keep(&Waiting {
                rule: "after-closing".into(),
                fingerprint: rule(branching.clone()).fingerprint,
                trigger: "opened".into(),
                at: vec![
                    Step {
                        index: 0,
                        branch: Some(true),
                    },
                    Step {
                        index: 0,
                        branch: None,
                    },
                ],
                until: noon() + chrono::Duration::minutes(20),
            })
            .await;
        let kept = waits.clone();
        let t = office_with(vec![branching], move |e| {
            e.with_waits(Some(kept as Arc<dyn WaitStore>))
        });
        let handles = t.engine.resume(Utc::now()).await;
        for h in handles {
            let _ = h.await;
        }
        assert_eq!(t.set().len(), 2);
        assert!(t.said().is_empty(), "not the other branch");
    }

    #[tokio::test]
    async fn a_wait_whose_rule_changed_is_dropped_and_said() {
        let waits = Arc::new(MemoryWaits::default());
        waits
            .keep(&Waiting {
                rule: "wake".into(),
                fingerprint: "the rule as it was".into(),
                trigger: "closed".into(),
                at: vec![Step {
                    index: 0,
                    branch: None,
                }],
                until: noon(),
            })
            .await;
        let kept = waits.clone();
        let t = office_with(vec![seven_hours_after_closing()], move |e| {
            e.with_waits(Some(kept as Arc<dyn WaitStore>))
        });
        assert!(t.engine.resume(noon()).await.is_empty());
        assert!(t.set().is_empty());
        assert!(waits.all().await.is_empty());
        let runs = t.runs("wake");
        assert_eq!(runs[0].outcome, Outcome::Stopped);
        assert!(
            runs[0]
                .reason
                .as_deref()
                .unwrap()
                .contains("the rule had changed")
        );
    }

    #[tokio::test]
    async fn a_wake_up_starts_the_routine_with_the_rules_settings() {
        let waker = Arc::new(RecordingWaker::default());
        let used = waker.clone();
        let t = office_with(
            vec![json!({
                "id": "morning",
                "triggers": [{ "type": "contact", "device": "office/door", "open": true }],
                "actions": [{ "do": "wake_up", "minutes": 30, "rooms": ["office"], "except": ["office/lightstrip"], "brightness": 80 }],
            })],
            move |e| e.with_waker(Some(used as Arc<dyn Waker>)),
        );
        t.engine.seed().await;
        t.send(door(true), noon()).await;
        let plans = lock(&waker.0).clone();
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].minutes, 30);
        assert_eq!(plans[0].brightness, 80);
        assert_eq!(plans[0].rooms[0].as_str(), "office");
        assert_eq!(plans[0].except, [id("office/lightstrip")]);
        assert_eq!(t.runs("morning")[0].steps, ["started a 30-minute wake-up"]);
    }

    #[tokio::test]
    async fn a_wake_up_without_a_waker_fails_its_run() {
        let t = office(vec![json!({
            "id": "morning",
            "triggers": [{ "type": "contact", "device": "office/door", "open": true }],
            "actions": [{ "do": "wake_up" }],
        })]);
        t.engine.seed().await;
        t.send(door(true), noon()).await;
        assert_eq!(t.runs("morning")[0].outcome, Outcome::Failed);
    }

    /// Two covers in the living room, a door that opens them, and the
    /// live config. `position` is what each blind currently reports.
    fn blinds_room(
        toml: &str,
        left_at: Option<u8>,
        right_at: Option<u8>,
    ) -> (Test, Arc<niles_config::ConfigStore>) {
        let store = Arc::new(
            niles_config::ConfigStore::from_str_in_memory(toml).expect("fixture config is valid"),
        );
        let registry = Arc::new(DeviceRegistry::new());
        let cover = |name: &str, position: Option<u8>| {
            registry.upsert(Device::new(
                id(name),
                DeviceState {
                    position,
                    ..Default::default()
                },
                DeviceClass::Cover,
            ));
        };
        cover("living_room/left_blind", left_at);
        cover("living_room/right_blind", right_at);
        let sink = Arc::new(RecordingSink {
            calls: Mutex::new(Vec::new()),
        });
        let said = Arc::new(RecordingNotifier {
            calls: Mutex::new(Vec::new()),
        });
        let kept = store.clone();
        let engine = Arc::new(
            AutomationEngine::new(
                vec![rule(json!({
                    "id": "blinds",
                    "triggers": [{ "type": "contact", "device": "living_room/door", "open": true }],
                    "actions": [{ "do": "set_blinds", "room": "living_room", "position": "open" }],
                }))],
                registry.clone(),
                Tz::UTC,
                sink.clone(),
                Some(said.clone()),
            )
            .with_settings(Some(kept)),
        );
        (
            Test {
                engine,
                sink,
                said,
                registry,
            },
            store,
        )
    }

    fn blind_positions(t: &Test) -> Vec<(DeviceId, u8)> {
        // In call order: each run appends in registry order, so two
        // runs of the same rule line up index by index.
        lock(&t.sink.calls)
            .iter()
            .map(|(id, s)| (id.clone(), s.position.unwrap()))
            .collect()
    }

    fn open_door() -> Event {
        Event::DeviceStateChanged {
            id: id("living_room/door"),
            state: DeviceState {
                open: Some(true),
                ..Default::default()
            },
        }
    }

    fn shut_door() -> Event {
        Event::DeviceStateChanged {
            id: id("living_room/door"),
            state: DeviceState {
                open: Some(false),
                ..Default::default()
            },
        }
    }

    #[tokio::test]
    async fn open_sends_each_blind_its_normal_level() {
        let (t, _store) = blinds_room("[blinds]\nnormal_open = 75\n", Some(0), Some(0));
        t.engine.seed().await;
        t.send(open_door(), noon()).await;
        let mut sent = blind_positions(&t);
        sent.sort_by_key(|(id, _)| id.to_string());
        assert_eq!(
            sent,
            vec![
                (id("living_room/left_blind"), 75),
                (id("living_room/right_blind"), 75)
            ]
        );
    }

    #[tokio::test]
    async fn a_changed_level_applies_to_the_next_run() {
        let (t, store) = blinds_room("[blinds]\nnormal_open = 75\n", Some(0), Some(0));
        t.engine.seed().await;
        t.send(open_door(), noon()).await;
        assert_eq!(blind_positions(&t)[0].1, 75);
        let patch: toml::Table = toml::from_str("[blinds]\nnormal_open = 60").unwrap();
        store
            .apply(&patch, niles_config::ChangeSource::Api)
            .await
            .unwrap();
        assert_eq!(store.current().blinds.normal_open, 60);
        t.send(shut_door(), noon()).await;
        t.send(open_door(), noon()).await;
        let sent = blind_positions(&t);
        // First run sent 75; after the patch the same rule sends 60 —
        // the level is not frozen at save time.
        assert_eq!(sent.len(), 4);
        assert_eq!(sent[2].1, 60);
        assert_eq!(sent[3].1, 60);
    }

    #[tokio::test]
    async fn already_at_the_level_opens_all_the_way() {
        let (t, _store) = blinds_room("[blinds]\nnormal_open = 75\n", Some(75), Some(74));
        t.engine.seed().await;
        t.send(open_door(), noon()).await;
        assert_eq!(blind_positions(&t)[0].1, 100);
        assert_eq!(blind_positions(&t)[1].1, 100);
    }

    #[tokio::test]
    async fn a_mixed_group_each_goes_to_its_own_level() {
        let (t, _store) = blinds_room("[blinds]\nnormal_open = 75\n", Some(75), Some(10));
        t.engine.seed().await;
        t.send(open_door(), noon()).await;
        assert_eq!(blind_positions(&t)[0].1, 75);
        assert_eq!(blind_positions(&t)[1].1, 75);
    }

    #[tokio::test]
    async fn an_exact_hundred_always_means_a_hundred() {
        let (mut t, _store) = blinds_room("[blinds]\nnormal_open = 75\n", Some(75), Some(75));
        t.engine = Arc::new(
            AutomationEngine::new(
                vec![rule(json!({
                    "id": "blinds",
                    "triggers": [{ "type": "contact", "device": "living_room/door", "open": true }],
                    "actions": [{ "do": "set_blinds", "room": "living_room", "position": 100 }],
                }))],
                t.registry.clone(),
                Tz::UTC,
                t.sink.clone(),
                Some(t.said.clone()),
            )
            .with_dedup_window(Duration::from_millis(0)),
        );
        t.engine.seed().await;
        t.send(open_door(), noon()).await;
        assert_eq!(blind_positions(&t)[0].1, 100);
    }

    #[tokio::test]
    async fn without_settings_open_means_all_the_way() {
        let (mut t, _store) = blinds_room("[blinds]\nnormal_open = 75\n", Some(0), Some(0));
        t.engine = Arc::new(
            AutomationEngine::new(
                vec![rule(json!({
                    "id": "blinds",
                    "triggers": [{ "type": "contact", "device": "living_room/door", "open": true }],
                    "actions": [{ "do": "set_blinds", "room": "living_room", "position": "open" }],
                }))],
                t.registry.clone(),
                Tz::UTC,
                t.sink.clone(),
                Some(t.said.clone()),
            )
            .with_dedup_window(Duration::from_millis(0)),
        );
        t.engine.seed().await;
        t.send(open_door(), noon()).await;
        assert_eq!(blind_positions(&t)[0].1, 100);
    }

    #[tokio::test]
    async fn a_per_blind_override_is_honoured() {
        let toml = r#"
            [blinds]
            normal_open = 75

            [[blinds.overrides]]
            device = "living_room/right_blind"
            normal_open = 60
        "#;
        let (t, _store) = blinds_room(toml, Some(0), Some(0));
        t.engine.seed().await;
        t.send(open_door(), noon()).await;
        let mut sent = blind_positions(&t);
        sent.sort_by_key(|(id, _)| id.to_string());
        assert_eq!(
            sent,
            vec![
                (id("living_room/left_blind"), 75),
                (id("living_room/right_blind"), 60)
            ]
        );
    }
}
