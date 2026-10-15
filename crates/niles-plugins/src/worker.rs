//! A plugin's thread: its instance, its calls, its deadlines, and what
//! happens when it fails.
//!
//! Each call into the instance gets a deadline in engine epochs (see
//! [`Limits`]); past it wasmtime traps the call wherever it is, without
//! Niles paying for instruction counting. A trap or a timeout throws the
//! instance away — its memory may be half-updated — and a fresh one is
//! started after a backoff: 5 s, 30 s, 2 min, 10 min, then 30 min. A
//! sixth failure within the hour stops the plugin until somebody
//! restarts it. `init` returning an error is not a failure of that kind:
//! it is the plugin saying what is wrong, and it is asked again in ten
//! minutes, or as soon as its settings change.
//!
//! A component targets the 0.1 world or the 0.2 one, and [`Bindings`]
//! calls either: the exports they share take the same Rust types (see
//! [`crate::bindings`]), and a 0.1 plugin has no MQTT to deliver.

use crate::bindings::niles::integration::types::{DeviceState, Event, ToolDef};
use crate::bindings::niles::integration::ui::{ActionResult, ActionValue, RenderContext, UiTree};
use crate::bindings::{mqtt, v0_1, v0_2};
use crate::host::HostState;
use crate::log::Level;
use crate::manifest::{INTERFACES, SettingValue};
use crate::mqtt::Delivery;
use crate::policy::{HttpPolicy, MqttPolicy};
use crate::runtime::{House, Job, Limits, Next, Plugin, Status, SurfaceError};
use crate::ui::Context;
use std::collections::{BTreeSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};
use wasmtime::component::{Component, Linker};
use wasmtime::{Engine, Store};

/// Dropped MQTT messages are noted in the plugin's log at most this often.
const NOTE_DROPS: Duration = Duration::from_secs(10);

/// The `niles:integration` version a component was built against, by
/// what it imports and exports: `0.1` or `0.2`.
pub(crate) fn interface_of(component: &Component, engine: &Engine) -> Result<&'static str, String> {
    let ty = component.component_type();
    let mut found = BTreeSet::new();
    for (name, _) in ty.imports(engine) {
        let Some(version) = name
            .strip_prefix("niles:integration/")
            .and_then(|rest| rest.split_once('@'))
            .map(|(_, v)| v)
        else {
            continue;
        };
        let minor = INTERFACES
            .iter()
            .find(|i| {
                version
                    .strip_prefix(**i)
                    .is_some_and(|rest| rest.starts_with('.'))
            })
            .ok_or_else(|| {
                format!(
                    "imports {name}; this Niles runs niles:integration {}",
                    INTERFACES.join(" and ")
                )
            })?;
        found.insert(*minor);
    }
    if ty
        .exports(engine)
        .any(|(name, _)| name == "on-mqtt-message")
    {
        found.insert("0.2");
    }
    match found.len() {
        0 => Ok("0.1"),
        1 => Ok(found.into_iter().next().unwrap_or("0.1")),
        _ => Err(format!(
            "mixes niles:integration {}; build it against one",
            found.into_iter().collect::<Vec<_>>().join(" and ")
        )),
    }
}

/// Compile `bytes` and check it against the world of `interface`, the
/// manifest's, or say why not.
pub(crate) fn prepare(
    engine: &Engine,
    linker: &Linker<HostState>,
    bytes: &[u8],
    interface: &str,
) -> Result<Pre, String> {
    let component = Component::new(engine, bytes)
        .map_err(|e| format!("is not a WebAssembly component: {}", e.root_cause()))?;
    let built = interface_of(&component, engine)?;
    if built != interface {
        return Err(format!(
            "is built for niles:integration {built}, and the manifest says interface = \"{interface}\""
        ));
    }
    let pre = linker
        .instantiate_pre(&component)
        .map_err(|e| format!("imports something Niles does not provide: {e:#}"))?;
    let missing = |e: wasmtime::Error| {
        format!("does not export the niles:integration {interface} plugin world: {e:#}")
    };
    Ok(match built {
        "0.1" => Pre::V1(v0_1::PluginPre::new(pre).map_err(missing)?),
        _ => Pre::V2(v0_2::PluginPre::new(pre).map_err(missing)?),
    })
}

/// A component ready to instantiate, as one world or the other.
#[derive(Clone)]
pub(crate) enum Pre {
    V1(v0_1::PluginPre<HostState>),
    V2(v0_2::PluginPre<HostState>),
}

impl Pre {
    pub fn instantiate(&self, store: &mut Store<HostState>) -> wasmtime::Result<Bindings> {
        Ok(match self {
            Pre::V1(pre) => Bindings::V1(pre.instantiate(store)?),
            Pre::V2(pre) => Bindings::V2(pre.instantiate(store)?),
        })
    }
}

/// An instance's exports, as one world or the other.
pub(crate) enum Bindings {
    V1(v0_1::Plugin),
    V2(v0_2::Plugin),
}

/// The same call on either world's bindings.
macro_rules! either {
    ($self:ident, $b:ident => $call:expr) => {
        match $self {
            Bindings::V1($b) => $call,
            Bindings::V2($b) => $call,
        }
    };
}

type Called<T> = wasmtime::Result<T>;

impl Bindings {
    pub fn call_init(&self, s: &mut Store<HostState>) -> Called<Result<(), String>> {
        either!(self, b => b.call_init(s))
    }

    pub fn call_tools(&self, s: &mut Store<HostState>) -> Called<Vec<ToolDef>> {
        either!(self, b => b.call_tools(s))
    }

    pub fn call_call_tool(
        &self,
        s: &mut Store<HostState>,
        name: &str,
        args: &str,
    ) -> Called<Result<String, String>> {
        either!(self, b => b.call_call_tool(s, name, args))
    }

    pub fn call_set_device(
        &self,
        s: &mut Store<HostState>,
        id: &str,
        desired: &DeviceState,
    ) -> Called<Result<(), String>> {
        either!(self, b => b.call_set_device(s, &id.to_owned(), desired))
    }

    pub fn call_on_timer(&self, s: &mut Store<HostState>, id: &str) -> Called<()> {
        either!(self, b => b.call_on_timer(s, id))
    }

    pub fn call_on_event(&self, s: &mut Store<HostState>, event: &Event) -> Called<()> {
        either!(self, b => b.call_on_event(s, event))
    }

    pub fn call_render(
        &self,
        s: &mut Store<HostState>,
        surface: &str,
        context: &RenderContext,
    ) -> Called<Result<UiTree, String>> {
        either!(self, b => b.call_render(s, surface, context))
    }

    pub fn call_on_action(
        &self,
        s: &mut Store<HostState>,
        surface: &str,
        control: &str,
        value: &ActionValue,
        context: &RenderContext,
    ) -> Called<Result<ActionResult, String>> {
        either!(self, b => b.call_on_action(s, surface, control, value, context))
    }

    pub fn call_stop(&self, s: &mut Store<HostState>) -> Called<()> {
        either!(self, b => b.call_stop(s))
    }

    /// A 0.1 plugin cannot connect, so it never has anything delivered.
    pub fn call_on_mqtt_message(
        &self,
        s: &mut Store<HostState>,
        conn: u32,
        topic: &str,
        payload: &[u8],
    ) -> Called<()> {
        match self {
            Bindings::V1(_) => Ok(()),
            Bindings::V2(b) => b.call_on_mqtt_message(s, conn, topic, payload),
        }
    }

    pub fn call_on_mqtt_status(
        &self,
        s: &mut Store<HostState>,
        conn: u32,
        status: &mqtt::ConnectionStatus,
    ) -> Called<()> {
        match self {
            Bindings::V1(_) => Ok(()),
            Bindings::V2(b) => b.call_on_mqtt_status(s, conn, status),
        }
    }
}

struct Live {
    store: Store<HostState>,
    bindings: Bindings,
}

struct Worker {
    plugin: Arc<Plugin>,
    house: Arc<House>,
    limits: Arc<Limits>,
    engine: Engine,
    pre: Option<Pre>,
    live: Option<Live>,
    /// When each restart after a failure happened, for the hourly limit.
    restarts: VecDeque<Instant>,
    /// When to start an instance, if one should be started.
    start_at: Option<Instant>,
    /// When dropped MQTT messages were last noted in the log.
    noted_drops: Option<Instant>,
}

pub(crate) fn run(
    plugin: Arc<Plugin>,
    component: Vec<u8>,
    engine: Engine,
    linker: Arc<Linker<HostState>>,
    house: Arc<House>,
    limits: Arc<Limits>,
) {
    let mut worker = Worker {
        plugin,
        house,
        limits,
        pre: None,
        live: None,
        restarts: VecDeque::new(),
        start_at: Some(Instant::now()),
        noted_drops: None,
        engine,
    };
    let interface = worker.plugin.manifest.interface.clone();
    match prepare(&worker.engine, &linker, &component, &interface) {
        Ok(pre) => worker.pre = Some(pre),
        Err(why) => {
            tracing::error!(plugin = %worker.plugin.id, "the component {why}");
            worker.plugin.count_failure();
            worker
                .plugin
                .set_status(Status::Failing(format!("the component {why}")));
            worker.start_at = None;
        }
    }
    drop(component);

    loop {
        let timers = worker
            .live
            .as_ref()
            .and_then(|l| l.store.data().timers.values().map(|t| t.due).min());
        let start = if worker.live.is_none() && worker.pre.is_some() {
            worker.start_at
        } else {
            None
        };
        match worker.plugin.queue.next(timers, start) {
            Next::Stop => {
                worker.stop_instance();
                break;
            }
            Next::Restart => {
                worker.stop_instance();
                worker.restarts.clear();
                worker.start_at = Some(Instant::now());
                if worker.pre.is_some() {
                    worker.plugin.set_status(Status::Starting);
                }
            }
            Next::Start => worker.start(),
            Next::Timers => worker.fire_timers(),
            Next::Event(event) => {
                worker.call("on-event", worker.limits.call, |b, s| {
                    b.call_on_event(s, &event)
                });
            }
            Next::Job(job) => worker.handle(job),
            Next::Mqtt(delivery) => worker.deliver(delivery),
        }
    }
}

impl Worker {
    fn id(&self) -> &str {
        &self.plugin.id
    }

    fn start(&mut self) {
        self.start_at = None;
        let Some(pre) = self.pre.clone() else {
            return;
        };
        let manifest = &self.plugin.manifest;
        let settings = (self.house.settings)(&self.plugin.id);
        let missing = manifest.missing_required(&settings, |key| {
            (self.house.secrets)(&self.plugin.id, key).is_some()
        });
        if !missing.is_empty() {
            let why = match missing.as_slice() {
                [one] => format!("{one} is required"),
                [rest @ .., last] => format!("{} and {last} are required", rest.join(", ")),
                [] => unreachable!(),
            };
            tracing::info!(plugin = %self.id(), "not set up: {why}");
            self.plugin.set_status(Status::NotSetUp(why));
            return;
        }
        let text = |key: &str| match manifest.setting(key)?.value(&settings)? {
            SettingValue::Text(text) => Some(text),
            _ => None,
        };
        let mut policy = HttpPolicy::new(&manifest.permissions, text);
        let mut mqtt = MqttPolicy::new(&manifest.permissions, text);
        if self.house.loopback {
            policy = policy.allowing_loopback();
            mqtt = mqtt.allowing_loopback();
        }
        let state = HostState::new(
            self.plugin.clone(),
            self.house.clone(),
            settings,
            policy,
            mqtt,
        );
        let mut store = Store::new(&self.engine, state);
        store.limiter(|state| &mut state.memory);
        self.deadline(&mut store, self.limits.init);
        let bindings = match pre.instantiate(&mut store) {
            Ok(bindings) => bindings,
            Err(e) => {
                drop(store);
                self.failed("instantiate", self.limits.init, &e);
                return;
            }
        };
        self.live = Some(Live { store, bindings });

        match self.call("init", self.limits.init, |b, s| b.call_init(s)) {
            None => {}
            Some(Err(why)) => {
                tracing::warn!(plugin = %self.id(), "init: {why}");
                self.discard();
                self.plugin.set_status(Status::Failing(why));
                self.start_at = Some(Instant::now() + self.limits.init_retry);
            }
            Some(Ok(())) => {
                let Some(tools) = self.call("tools", self.limits.call, |b, s| b.call_tools(s))
                else {
                    return;
                };
                crate::tools::offer(&self.plugin, &self.house, tools);
                self.plugin.set_status(Status::Running);
                tracing::info!(plugin = %self.id(), "running");
            }
        }
    }

    fn deadline(&self, store: &mut Store<HostState>, limit: Duration) {
        let ticks = limit
            .as_nanos()
            .div_ceil(self.limits.tick.as_nanos().max(1)) as u64;
        store.set_epoch_deadline(ticks.max(1));
        store.data_mut().deadline = Instant::now() + limit;
    }

    /// Call into the instance with a deadline. `None` when there is no
    /// instance, or the call trapped and the instance is gone.
    fn call<R>(
        &mut self,
        export: &'static str,
        limit: Duration,
        f: impl FnOnce(&Bindings, &mut Store<HostState>) -> wasmtime::Result<R>,
    ) -> Option<R> {
        let mut live = self.live.take()?;
        self.deadline(&mut live.store, limit);
        let result = f(&live.bindings, &mut live.store);
        self.live = Some(live);
        match result {
            Ok(r) => Some(r),
            Err(e) => {
                self.failed(export, limit, &e);
                None
            }
        }
    }

    /// The instance trapped or ran out of time: throw it away and start
    /// another after the backoff, or stop for good.
    fn failed(&mut self, export: &str, limit: Duration, error: &wasmtime::Error) {
        let reason = describe(export, limit, error);
        self.plugin.count_failure();
        self.discard();
        let now = Instant::now();
        let hour = Duration::from_secs(3600);
        while self
            .restarts
            .front()
            .is_some_and(|t| now.duration_since(*t) >= hour)
        {
            self.restarts.pop_front();
        }
        if self.restarts.len() >= self.limits.restarts_per_hour {
            tracing::error!(
                plugin = %self.id(),
                "{reason}; stopped after {} restarts within an hour",
                self.restarts.len()
            );
            self.plugin.set_status(Status::Stopped(reason));
            self.start_at = None;
            return;
        }
        let backoff = &self.limits.backoff;
        let delay = backoff
            .get(self.restarts.len())
            .or(backoff.last())
            .copied()
            .unwrap_or(Duration::from_secs(60));
        self.restarts.push_back(now);
        tracing::warn!(
            plugin = %self.id(),
            "{reason}; starting again in {}",
            human(delay)
        );
        self.plugin.set_status(Status::Failing(reason));
        self.start_at = Some(now + delay);
    }

    /// Drop the instance and everything it had set up.
    fn discard(&mut self) {
        self.live = None;
        self.plugin.forget_instance(&self.house);
    }

    /// `stop`, then drop the instance. A trap in `stop` changes nothing:
    /// it was going anyway.
    fn stop_instance(&mut self) {
        if let Some(mut live) = self.live.take() {
            self.deadline(&mut live.store, self.limits.stop);
            if let Err(e) = live.bindings.call_stop(&mut live.store) {
                tracing::debug!(
                    plugin = %self.id(),
                    "{}",
                    describe("stop", self.limits.stop, &e)
                );
            }
        }
        self.discard();
    }

    fn fire_timers(&mut self) {
        let now = Instant::now();
        let Some(live) = self.live.as_mut() else {
            return;
        };
        let mut due: Vec<(Instant, String)> = live
            .store
            .data()
            .timers
            .iter()
            .filter(|(_, t)| t.due <= now)
            .map(|(id, t)| (t.due, id.clone()))
            .collect();
        due.sort();
        for (_, id) in due {
            let Some(live) = self.live.as_mut() else {
                return;
            };
            // Rescheduled before the call, so one the plugin sets again
            // from inside `on-timer` wins. Missed periods are skipped,
            // not queued: a slow poll must not pile up.
            let timers = &mut live.store.data_mut().timers;
            match timers.get_mut(&id) {
                Some(timer) => match timer.every {
                    Some(period) => {
                        let mut next = timer.due + period;
                        while next <= now {
                            next += period;
                        }
                        timer.due = next;
                    }
                    None => {
                        timers.remove(&id);
                    }
                },
                None => continue,
            }
            tracing::debug!(plugin = %self.plugin.id, "timer {id} fired");
            if self
                .call("on-timer", self.limits.call, |b, s| b.call_on_timer(s, &id))
                .is_none()
            {
                return;
            }
        }
    }

    fn handle(&mut self, job: Job) {
        let running = self.live.is_some();
        match job {
            Job::CallTool { name, args, reply } => {
                let answer = match running {
                    false => Err(self.plugin.not_working()),
                    true => self
                        .call("call-tool", self.limits.call, |b, s| {
                            b.call_call_tool(s, &name, &args)
                        })
                        .unwrap_or_else(|| Err(self.plugin.not_working())),
                };
                let _ = reply.send(answer);
            }
            Job::SetDevice { id, desired, reply } => {
                let answer = match running {
                    false => Err(self.plugin.not_working()),
                    true => self
                        .call("set-device", self.limits.call, |b, s| {
                            b.call_set_device(s, &id, &desired)
                        })
                        .unwrap_or_else(|| Err(self.plugin.not_working())),
                };
                let _ = reply.send(answer);
            }
            Job::Render {
                surface,
                context,
                reply,
            } => {
                let _ = reply.send(self.render(&surface, &context));
            }
            Job::Action {
                surface,
                control,
                value,
                context,
                reply,
            } => {
                let value = value.into();
                let wit_context = (&context).into();
                let answer = match running {
                    false => Err(SurfaceError::Unavailable(self.plugin.status().line())),
                    true => match self.call("on-action", self.limits.call, |b, s| {
                        b.call_on_action(s, &surface, &control, &value, &wit_context)
                    }) {
                        Some(Ok(result)) => Ok(result.into()),
                        Some(Err(why)) => Err(SurfaceError::Plugin(why)),
                        None => Err(SurfaceError::Unavailable(self.plugin.status().line())),
                    },
                };
                let _ = reply.send(answer);
            }
        }
    }

    /// Hand the plugin what one of its connections received, unless the
    /// connection is not this instance's.
    fn deliver(&mut self, delivery: Delivery) {
        if self.noted_drops.is_none_or(|at| at.elapsed() >= NOTE_DROPS) {
            let dropped = self.plugin.queue.take_mqtt_dropped();
            if dropped > 0 {
                self.plugin.note(
                    Level::Warn,
                    &format!(
                        "{dropped} MQTT message(s) were dropped: they came faster than the plugin took them"
                    ),
                );
                self.noted_drops = Some(Instant::now());
            }
        }
        let Some(live) = &self.live else {
            return;
        };
        if !live.store.data().connections.contains_key(&delivery.conn()) {
            return;
        }
        match delivery {
            Delivery::Message {
                conn,
                topic,
                payload,
                ..
            } => {
                self.call("on-mqtt-message", self.limits.call, |b, s| {
                    b.call_on_mqtt_message(s, conn, &topic, &payload)
                });
            }
            Delivery::Status { conn, status } => {
                self.call("on-mqtt-status", self.limits.call, |b, s| {
                    b.call_on_mqtt_status(s, conn, &status)
                });
            }
        }
    }

    fn render(
        &mut self,
        surface: &str,
        context: &Context,
    ) -> Result<crate::ui::Surface, SurfaceError> {
        let unavailable = |plugin: &Plugin| SurfaceError::Unavailable(plugin.status().line());
        if self.live.is_none() {
            return Err(unavailable(&self.plugin));
        }
        let wit_context = context.into();
        let tree = match self.call("render", self.limits.render, |b, s| {
            b.call_render(s, surface, &wit_context)
        }) {
            None => return Err(unavailable(&self.plugin)),
            Some(Err(why)) => return Err(SurfaceError::Plugin(why)),
            Some(Ok(tree)) => tree,
        };
        let live = self
            .live
            .as_ref()
            .ok_or_else(|| unavailable(&self.plugin))?;
        crate::ui::surface(tree, live.store.data().policy()).map_err(|why| {
            tracing::warn!(plugin = %self.plugin.id, "{surface} cannot be drawn: {why}");
            SurfaceError::Plugin(format!("This page cannot be drawn: {why}."))
        })
    }
}

/// What went wrong, as the card says it: "timed out in on-timer after
/// 10 s", "trapped in call-tool: out of memory".
fn describe(export: &str, limit: Duration, error: &wasmtime::Error) -> String {
    match error.downcast_ref::<wasmtime::Trap>() {
        Some(wasmtime::Trap::Interrupt) => {
            format!("timed out in {export} after {}", human(limit))
        }
        Some(trap) => format!("trapped in {export}: {trap}"),
        None => format!("trapped in {export}: {}", error.root_cause()),
    }
}

fn human(d: Duration) -> String {
    let secs = d.as_secs();
    match secs {
        0 => format!("{} ms", d.as_millis()),
        s if s % 60 == 0 && s >= 60 => format!("{} min", s / 60),
        s => format!("{s} s"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_read_as_people_say_them() {
        assert_eq!(human(Duration::from_secs(10)), "10 s");
        assert_eq!(human(Duration::from_secs(120)), "2 min");
        assert_eq!(human(Duration::from_millis(300)), "300 ms");
    }

    #[test]
    fn a_timeout_says_where_and_how_long() {
        let e = wasmtime::Error::new(wasmtime::Trap::Interrupt);
        assert_eq!(
            describe("on-timer", Duration::from_secs(10), &e),
            "timed out in on-timer after 10 s"
        );
        let e = wasmtime::format_err!("out of memory: the plugin may use 64 MiB");
        assert_eq!(
            describe("call-tool", Duration::from_secs(10), &e),
            "trapped in call-tool: out of memory: the plugin may use 64 MiB"
        );
    }
}
