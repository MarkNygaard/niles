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

use crate::bindings::{Plugin as Bindings, PluginPre};
use crate::host::HostState;
use crate::manifest::SettingValue;
use crate::policy::HttpPolicy;
use crate::runtime::{House, Job, Limits, Next, Plugin, Status, SurfaceError};
use crate::ui::Context;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};
use wasmtime::component::{Component, Linker};
use wasmtime::{Engine, Store};

/// Compile `bytes` and check it against the world, or say why not.
pub(crate) fn prepare(
    engine: &Engine,
    linker: &Linker<HostState>,
    bytes: &[u8],
) -> Result<PluginPre<HostState>, String> {
    let component = Component::new(engine, bytes)
        .map_err(|e| format!("is not a WebAssembly component: {}", e.root_cause()))?;
    let pre = linker
        .instantiate_pre(&component)
        .map_err(|e| format!("imports something Niles does not provide: {e:#}"))?;
    PluginPre::new(pre)
        .map_err(|e| format!("does not export the niles:integration plugin world: {e:#}"))
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
    pre: Option<PluginPre<HostState>>,
    live: Option<Live>,
    /// When each restart after a failure happened, for the hourly limit.
    restarts: VecDeque<Instant>,
    /// When to start an instance, if one should be started.
    start_at: Option<Instant>,
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
        engine,
    };
    match prepare(&worker.engine, &linker, &component) {
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
        let policy = HttpPolicy::new(&manifest.permissions, |key| {
            match manifest.setting(key)?.value(&settings)? {
                SettingValue::Text(text) => Some(text),
                _ => None,
            }
        });
        let state = HostState::new(self.plugin.clone(), self.house.clone(), settings, policy);
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
