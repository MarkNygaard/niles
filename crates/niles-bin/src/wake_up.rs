//! Wake-ups an automation starts: the morning routine, at whatever
//! moment the rule arrives at, with the rule's own settings. The
//! scheduled routine is `niles_scheduler::morning`.
//!
//! Lights that are off come on at 0% and ramp to the end level; lights
//! already on, or taken by the scheduled routine, are left alone. When
//! the ramp ends the lights *hold* there, rather than going back to the
//! curve: a wake-up at 03:30 would otherwise be dimmed back to night the
//! moment it had woken somebody. A hold lets go when the light is turned
//! off or adjusted, or when the curve's own value comes up to meet it.
//!
//! A wake-up with a warmth of its own ramps the colour too, from its
//! start warmth to its end one, and holds that. Without one it follows
//! the curve's warmth all the way, held or not; with the curve off it
//! leaves the colour be.
//!
//! Everything under way is kept in Postgres, so a restart mid-ramp or
//! mid-hold carries on where it was.

use chrono::{DateTime, Duration as Span, Utc};
use niles_automations::{WakePlan, Waker};
use niles_config::ConfigStore;
use niles_core::{DeviceId, DeviceRegistry, DeviceState};
use niles_mqtt::{CommandRouter, MqttPublisher};
use niles_scheduler::{KELVIN_DEBOUNCE_K, ManualModeTracker, MorningClaimTracker, kick_on_skip};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A ramp step smaller than this is not worth a command: the same
/// rule the scheduled routine keeps.
const DEBOUNCE: u8 = 2;

/// The curve's value at the moment, when it is switched on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CurveNow {
    pub(crate) brightness: u8,
    pub(crate) kelvin: u16,
}

/// One wake-up under way.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Ramp {
    started: DateTime<Utc>,
    ends: DateTime<Utc>,
    /// The lights it turned on and drives, by id.
    lights: Vec<String>,
    /// Plugs it switches on when the ramp ends.
    plugs: Vec<String>,
    brightness: u8,
    start_kelvin: Option<u16>,
    kelvin: Option<u16>,
}

/// Where a light is held once its ramp ended.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
struct Hold {
    brightness: u8,
    /// Its own warmth; the curve's when `None`.
    kelvin: Option<u16>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct State {
    ramps: Vec<Ramp>,
    holds: BTreeMap<String, Hold>,
}

impl State {
    fn owns(&self, id: &str) -> bool {
        self.holds.contains_key(id) || self.ramps.iter().any(|r| r.lights.iter().any(|l| l == id))
    }

    /// Stop doing anything with `id`. True when something changed.
    fn let_go(&mut self, id: &str) -> bool {
        let mut changed = self.holds.remove(id).is_some();
        for ramp in &mut self.ramps {
            let before = ramp.lights.len() + ramp.plugs.len();
            ramp.lights.retain(|l| l != id);
            ramp.plugs.retain(|p| p != id);
            changed |= ramp.lights.len() + ramp.plugs.len() != before;
        }
        changed
    }
}

/// Where a ramp is at `now`: 0% at its start, its end level at its end.
fn level_at(ramp: &Ramp, now: DateTime<Utc>) -> u8 {
    (f64::from(ramp.brightness) * gone(ramp, now)).round() as u8
}

/// The ramp's own warmth at `now`, from its start warmth to its end.
fn kelvin_at(ramp: &Ramp, now: DateTime<Utc>) -> Option<u16> {
    let to = f64::from(ramp.kelvin?);
    let from = ramp.start_kelvin.map_or(to, f64::from);
    Some((from + (to - from) * gone(ramp, now)).round() as u16)
}

/// The warmth a light switches on at: the wake-up's start warmth, or
/// the curve's.
fn start_warmth(plan: &WakePlan, curve: Option<CurveNow>) -> Option<u16> {
    plan.start_kelvin
        .or(plan.kelvin)
        .or(curve.map(|c| c.kelvin))
}

/// The warmth a ramping light is on at `now`: its own, or the curve's.
fn ramp_warmth(ramp: &Ramp, now: DateTime<Utc>, curve: Option<CurveNow>) -> Option<u16> {
    kelvin_at(ramp, now).or(curve.map(|c| c.kelvin))
}

/// The warmth to keep a held light on, when anything needs sending: the
/// curve's, unless the wake-up gave it its own, which it already has.
fn held_warmth(hold: &Hold, curve: Option<CurveNow>) -> Option<u16> {
    curve.map(|c| c.kelvin).filter(|_| hold.kelvin.is_none())
}

/// How far through the ramp `now` is, 0 to 1.
fn gone(ramp: &Ramp, now: DateTime<Utc>) -> f64 {
    let total = (ramp.ends - ramp.started).num_seconds().max(1) as f64;
    (now - ramp.started).num_seconds().clamp(0, total as i64) as f64 / total
}

/// Whether a light's reported warmth is far enough from `want` to send.
/// A light that reports none is sent it with the brightness only.
fn colour_behind(reported: Option<u16>, want: Option<u16>) -> bool {
    matches!((reported, want), (Some(now), Some(want)) if now.abs_diff(want) > KELVIN_DEBOUNCE_K)
}

pub(crate) struct WakeUps {
    state: Mutex<State>,
    /// The curve as the last tick had it, for a wake-up starting
    /// between ticks. `None` with the curve off.
    curve: Mutex<Option<CurveNow>>,
    store: Option<Arc<niles_db::PostgresWakeUps>>,
    registry: Arc<DeviceRegistry>,
    publisher: MqttPublisher,
    router: Arc<CommandRouter>,
    settings: Arc<ConfigStore>,
    tracker: Arc<ManualModeTracker>,
    claims: Arc<MorningClaimTracker>,
    dry_run: bool,
}

impl WakeUps {
    /// Pick up what was under way before a restart.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn load(
        store: Option<Arc<niles_db::PostgresWakeUps>>,
        registry: Arc<DeviceRegistry>,
        publisher: MqttPublisher,
        router: Arc<CommandRouter>,
        settings: Arc<ConfigStore>,
        tracker: Arc<ManualModeTracker>,
        claims: Arc<MorningClaimTracker>,
        dry_run: bool,
    ) -> Arc<Self> {
        let state = match &store {
            Some(store) => match store.load().await {
                Ok(Some(document)) => serde_json::from_str(&document).unwrap_or_else(|e| {
                    tracing::warn!("[wake-up] what was kept would not read ({e}); starting empty");
                    State::default()
                }),
                Ok(None) => State::default(),
                Err(e) => {
                    tracing::warn!("[wake-up] {e}; starting empty");
                    State::default()
                }
            },
            None => State::default(),
        };
        if !state.ramps.is_empty() || !state.holds.is_empty() {
            tracing::info!(
                "[wake-up] carrying on: {} under way, {} light(s) held",
                state.ramps.len(),
                state.holds.len()
            );
        }
        Arc::new(Self {
            state: Mutex::new(state),
            curve: Mutex::new(None),
            store,
            registry,
            publisher,
            router,
            settings,
            tracker,
            claims,
            dry_run,
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Wake-ups ramping now. A restart pauses them; they carry on after,
    /// since what is under way is kept.
    pub(crate) fn ramping(&self) -> usize {
        self.lock().ramps.len()
    }

    /// Whether a wake-up is ramping or holding `id`: the curve and the
    /// scheduled routine leave it alone.
    pub(crate) fn owns(&self, id: &DeviceId) -> bool {
        self.lock().owns(&id.to_string())
    }

    /// The light was turned off, or is gone: off means off, for this
    /// wake-up as for the scheduled one.
    pub(crate) async fn let_go(&self, id: &DeviceId) {
        let changed = self.lock().let_go(&id.to_string());
        if changed {
            tracing::info!("[wake-up] let go of {id}");
            self.save().await;
        }
    }

    async fn save(&self) {
        let Some(store) = &self.store else {
            return;
        };
        let document = match serde_json::to_string(&*self.lock()) {
            Ok(document) => document,
            Err(e) => {
                tracing::warn!("[wake-up] not kept: {e}");
                return;
            }
        };
        if let Err(e) = store.store(&document).await {
            tracing::warn!("[wake-up] not kept: {e}");
        }
    }

    /// Send `target` to `id`, as the routine does: through a device's
    /// own commander, or over MQTT.
    async fn send(&self, id: &DeviceId, target: &DeviceState, fade: Duration) -> bool {
        let what = "[wake-up]";
        if let Some(commander) = self.router.commander_for(id, target) {
            return crate::wiring::automations::command_directly(
                commander,
                id,
                target,
                self.dry_run,
                what,
            )
            .await;
        }
        let Some((topic, payload)) = self.router.format_fading(id, target, fade) else {
            return false;
        };
        if self.dry_run {
            tracing::info!("{what} [dry-run] {topic}  {payload}");
            return true;
        }
        match self.publisher.publish(&topic, payload.clone()).await {
            Ok(()) => {
                tracing::info!("{what} {topic}  {payload}");
                true
            }
            Err(e) => {
                tracing::warn!("{what} {topic} failed: {e}");
                false
            }
        }
    }

    /// The lights and plugs a plan names: every curve-managed light and
    /// lamp plug when it names none.
    fn targets(&self, plan: &WakePlan) -> (Vec<DeviceId>, Vec<DeviceId>) {
        let cfg = self.settings.current();
        let ambient = cfg.ambient_lights.ids();
        let lamp_plugs = cfg.lamp_plugs.ids();
        let mut wanted: Vec<DeviceId> = if plan.rooms.is_empty() && plan.devices.is_empty() {
            self.registry
                .list_all()
                .into_iter()
                .filter(|d| d.is_lamp(lamp_plugs) && !ambient.contains(&d.id))
                .map(|d| d.id)
                .collect()
        } else {
            let mut ids: Vec<DeviceId> = plan
                .rooms
                .iter()
                .flat_map(|room| self.registry.list_room(room))
                .filter(|d| d.is_lamp(lamp_plugs) && d.room_light && !ambient.contains(&d.id))
                .map(|d| d.id)
                .collect();
            ids.extend(plan.devices.iter().cloned());
            ids
        };
        let except: HashSet<&DeviceId> = plan.except.iter().collect();
        let mut seen = HashSet::new();
        wanted.retain(|id| !except.contains(id) && seen.insert(id.clone()));
        let mut lights = Vec::new();
        let mut plugs = Vec::new();
        for id in wanted {
            match self.registry.get(&id) {
                Some(d) if d.is_curve_driven(ambient) => lights.push(id),
                Some(d) if d.is_switchable() && !d.is_light() => plugs.push(id),
                _ => {}
            }
        }
        (lights, plugs)
    }

    async fn start(&self, plan: &WakePlan, now: DateTime<Utc>) -> Result<String, String> {
        let (lights, plugs) = self.targets(plan);
        if lights.is_empty() && plugs.is_empty() {
            return Err("there are no lights there to wake anybody with".into());
        }
        let curve = *self.curve.lock().unwrap_or_else(|e| e.into_inner());
        let warmth = start_warmth(plan, curve);
        let mut took = Vec::new();
        for id in &lights {
            let device = self.registry.get(id);
            let claimed = self.claims.is_claimed(id) || self.owns(id);
            // Every one of `lights` is curve-driven: `targets` chose them so.
            if let Some(reason) = kick_on_skip(
                device.is_some(),
                device.is_some(),
                device.as_ref().and_then(|d| d.state.on),
                claimed,
            ) {
                tracing::info!("[wake-up] skipped {id}: {}", reason.reason());
                continue;
            }
            let target = DeviceState {
                on: Some(true),
                brightness: Some(0),
                color_temp_kelvin: warmth,
                ..Default::default()
            };
            if self.send(id, &target, Duration::ZERO).await {
                took.push(id.to_string());
            }
        }
        let plugs: Vec<String> = plugs
            .iter()
            .filter(|id| {
                self.registry
                    .get(id)
                    .is_some_and(|d| d.state.on != Some(true))
            })
            .map(|id| id.to_string())
            .collect();
        if took.is_empty() && plugs.is_empty() {
            return Ok("woke nothing: every light was already on".into());
        }
        let ramp = Ramp {
            started: now,
            ends: now + Span::minutes(i64::from(plan.minutes)),
            lights: took.clone(),
            plugs: plugs.clone(),
            brightness: plan.brightness,
            start_kelvin: plan.start_kelvin,
            kelvin: plan.kelvin,
        };
        self.lock().ramps.push(ramp);
        self.save().await;
        tracing::info!(
            "[wake-up] {} light(s) ramping to {}% over {} minutes",
            took.len(),
            plan.brightness,
            plan.minutes
        );
        let lights = match took.len() {
            1 => "1 light".to_string(),
            n => format!("{n} lights"),
        };
        Ok(format!(
            "started a {}-minute wake-up to {}%, {lights}",
            plan.minutes, plan.brightness
        ))
    }

    /// Drive the ramps on toward `now`, end the ones that are over, keep
    /// held lights on the curve's warmth when they have none of their
    /// own, and let go of the ones the curve has caught up with.
    /// `curve` is the curve's value now, `None` when it is off.
    #[expect(
        clippy::too_many_lines,
        reason = "advances every ramp and hold in one pass so the state is saved once"
    )]
    pub(crate) async fn tick(&self, now: DateTime<Utc>, curve: Option<CurveNow>, fade: Duration) {
        *self.curve.lock().unwrap_or_else(|e| e.into_inner()) = curve;
        let (ramps, holds) = {
            let state = self.lock();
            (state.ramps.clone(), state.holds.clone())
        };
        if ramps.is_empty() && holds.is_empty() {
            return;
        }
        let mut changed = false;
        for ramp in &ramps {
            let over = now >= ramp.ends;
            let level = if over {
                ramp.brightness
            } else {
                level_at(ramp, now)
            };
            let warmth = ramp_warmth(ramp, now, curve);
            for raw in &ramp.lights {
                let Ok(id) = DeviceId::parse(raw) else {
                    continue;
                };
                // Adjusted by hand: theirs now, as for the scheduled
                // routine.
                if self.tracker.is_flagged(&id) {
                    changed |= self.lock().let_go(raw);
                    continue;
                }
                let Some(device) = self.registry.get(&id) else {
                    continue;
                };
                let behind = device
                    .state
                    .brightness
                    .is_none_or(|now| now.abs_diff(level) > DEBOUNCE)
                    || colour_behind(device.state.color_temp_kelvin, warmth);
                if behind || over {
                    let target = DeviceState {
                        brightness: Some(level),
                        color_temp_kelvin: warmth,
                        ..Default::default()
                    };
                    self.send(&id, &target, if over { Duration::ZERO } else { fade })
                        .await;
                }
            }
            if !over {
                continue;
            }
            for raw in &ramp.plugs {
                let Ok(id) = DeviceId::parse(raw) else {
                    continue;
                };
                if self.tracker.is_flagged(&id) {
                    continue;
                }
                let target = DeviceState {
                    on: Some(true),
                    ..Default::default()
                };
                self.send(&id, &target, Duration::ZERO).await;
            }
            let mut state = self.lock();
            state.ramps.retain(|r| r != ramp);
            for raw in &ramp.lights {
                state.holds.insert(
                    raw.clone(),
                    Hold {
                        brightness: ramp.brightness,
                        kelvin: ramp.kelvin,
                    },
                );
            }
            changed = true;
            tracing::info!(
                "[wake-up] ramp over; holding {} light(s)",
                ramp.lights.len()
            );
        }
        for (raw, hold) in &holds {
            let Ok(id) = DeviceId::parse(raw) else {
                continue;
            };
            let caught_up = curve.is_some_and(|c| c.brightness >= hold.brightness);
            let adjusted = self.tracker.is_flagged(&id);
            if !(caught_up || adjusted) {
                // Held at its brightness, but the colour moves with the
                // curve unless the wake-up gave it one.
                let follow = held_warmth(hold, curve);
                if let Some(device) = self.registry.get(&id)
                    && colour_behind(device.state.color_temp_kelvin, follow)
                {
                    let target = DeviceState {
                        color_temp_kelvin: follow,
                        ..Default::default()
                    };
                    self.send(&id, &target, fade).await;
                }
                continue;
            }
            self.lock().holds.remove(raw);
            changed = true;
            tracing::info!(
                "[wake-up] {raw} back to the curve ({})",
                if caught_up {
                    "the curve caught up"
                } else {
                    "adjusted by hand"
                }
            );
        }
        if changed {
            self.save().await;
        }
    }
}

#[async_trait::async_trait]
impl Waker for WakeUps {
    async fn wake_up(&self, plan: &WakePlan) -> Result<String, String> {
        self.start(plan, Utc::now()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp() -> Ramp {
        Ramp {
            started: "2026-10-16T03:00:00Z".parse().unwrap(),
            ends: "2026-10-16T03:30:00Z".parse().unwrap(),
            lights: vec!["z2m:bedroom/ceiling".into()],
            plugs: vec!["z2m:bedroom/lamp".into()],
            brightness: 80,
            start_kelvin: None,
            kelvin: None,
        }
    }

    fn curve(brightness: u8, kelvin: u16) -> Option<CurveNow> {
        Some(CurveNow { brightness, kelvin })
    }

    #[test]
    fn its_own_warmth_ramps_from_the_start_to_the_end() {
        let r = Ramp {
            start_kelvin: Some(2200),
            kelvin: Some(4000),
            ..ramp()
        };
        let half: DateTime<Utc> = "2026-10-16T03:15:00Z".parse().unwrap();
        assert_eq!(ramp_warmth(&r, r.started, curve(15, 2700)), Some(2200));
        assert_eq!(ramp_warmth(&r, half, curve(15, 2700)), Some(3100));
        assert_eq!(ramp_warmth(&r, r.ends, None), Some(4000));
        let level = Ramp {
            kelvin: Some(3000),
            ..ramp()
        };
        assert_eq!(ramp_warmth(&level, half, curve(15, 2700)), Some(3000));
    }

    #[test]
    fn without_its_own_warmth_it_follows_the_curve_or_leaves_the_colour_be() {
        let r = ramp();
        let half: DateTime<Utc> = "2026-10-16T03:15:00Z".parse().unwrap();
        assert_eq!(ramp_warmth(&r, half, curve(15, 2700)), Some(2700));
        assert_eq!(ramp_warmth(&r, half, None), None, "the curve is off");
        assert_eq!(start_warmth(&plan(), curve(15, 2700)), Some(2700));
        assert_eq!(start_warmth(&plan(), None), None);
        let own = WakePlan {
            start_kelvin: Some(2200),
            kelvin: Some(4000),
            ..plan()
        };
        assert_eq!(start_warmth(&own, curve(15, 2700)), Some(2200));
        let level = WakePlan {
            kelvin: Some(3000),
            ..plan()
        };
        assert_eq!(start_warmth(&level, curve(15, 2700)), Some(3000));
    }

    #[test]
    fn a_held_light_keeps_its_own_warmth_or_follows_the_curves() {
        let own = Hold {
            brightness: 80,
            kelvin: Some(4000),
        };
        let none = Hold {
            brightness: 80,
            kelvin: None,
        };
        assert_eq!(
            held_warmth(&own, curve(15, 2700)),
            None,
            "it has it already"
        );
        assert_eq!(held_warmth(&none, curve(15, 2700)), Some(2700));
        assert_eq!(held_warmth(&none, None), None);
    }

    #[test]
    fn a_colour_is_sent_only_when_it_has_drifted() {
        assert!(colour_behind(Some(2200), Some(2700)));
        assert!(!colour_behind(Some(2680), Some(2700)));
        assert!(
            !colour_behind(None, Some(2700)),
            "a light that reports none"
        );
        assert!(!colour_behind(Some(2200), None), "nothing wanted");
    }

    #[test]
    fn ramps_from_nothing_to_the_end_level() {
        let r = ramp();
        assert_eq!(level_at(&r, r.started), 0);
        assert_eq!(level_at(&r, "2026-10-16T03:15:00Z".parse().unwrap()), 40);
        assert_eq!(level_at(&r, r.ends), 80);
        assert_eq!(level_at(&r, "2026-10-16T05:00:00Z".parse().unwrap()), 80);
    }

    #[test]
    fn owns_what_it_ramps_and_holds_and_lets_go_on_off() {
        let mut state = State {
            ramps: vec![ramp()],
            holds: BTreeMap::from([(
                "z2m:hall/ceiling".to_string(),
                Hold {
                    brightness: 100,
                    kelvin: None,
                },
            )]),
        };
        assert!(state.owns("z2m:bedroom/ceiling"));
        assert!(state.owns("z2m:hall/ceiling"));
        assert!(!state.owns("z2m:kitchen/ceiling"));
        assert!(state.let_go("z2m:bedroom/ceiling"));
        assert!(state.let_go("z2m:hall/ceiling"));
        assert!(!state.owns("z2m:bedroom/ceiling") && !state.owns("z2m:hall/ceiling"));
        assert!(!state.let_go("z2m:kitchen/ceiling"), "nothing to let go of");
    }

    fn id(s: &str) -> DeviceId {
        DeviceId::parse(&format!("z2m:{s}")).unwrap()
    }

    /// The bedroom: a ceiling light and a bedside lamp, off; a plug the
    /// coffee machine is on, off; and the hall light, on.
    async fn bedroom() -> (Arc<WakeUps>, Arc<DeviceRegistry>) {
        use niles_core::{Device, DeviceClass};
        let registry = Arc::new(DeviceRegistry::new());
        let off = || DeviceState {
            on: Some(false),
            ..Default::default()
        };
        registry.upsert(Device::new(
            id("bedroom/ceiling"),
            off(),
            DeviceClass::Light,
        ));
        registry.upsert(Device::new(id("bedroom/lamp"), off(), DeviceClass::Light));
        registry.upsert(Device::new(
            id("kitchen/coffee"),
            off(),
            DeviceClass::Outlet,
        ));
        registry.upsert(Device::new(
            id("hall/ceiling"),
            DeviceState {
                on: Some(true),
                brightness: Some(15),
                ..Default::default()
            },
            DeviceClass::Light,
        ));
        // A broker that is never reached: everything here is a dry run.
        let mqtt = niles_mqtt::MqttClient::connect(niles_mqtt::MqttOptions::new(
            "127.0.0.1",
            1,
            "wake-up-test",
        ));
        let wake_ups = WakeUps::load(
            None,
            registry.clone(),
            mqtt.publisher(),
            Arc::new(CommandRouter::z2m_only("zigbee2mqtt")),
            Arc::new(ConfigStore::from_str_in_memory("").unwrap()),
            Arc::new(ManualModeTracker::new()),
            Arc::new(MorningClaimTracker::new()),
            true,
        )
        .await;
        (wake_ups, registry)
    }

    fn plan() -> WakePlan {
        WakePlan {
            minutes: 30,
            rooms: vec![niles_core::RoomName::parse("bedroom").unwrap()],
            devices: vec![id("kitchen/coffee")],
            except: vec![id("bedroom/lamp")],
            brightness: 80,
            start_kelvin: None,
            kelvin: None,
        }
    }

    #[tokio::test]
    async fn wakes_the_lights_that_are_off_ramps_them_and_holds_them() {
        let (wake_ups, _) = bedroom().await;
        assert_eq!(wake_ups.ramping(), 0, "nothing under way yet");
        let started: DateTime<Utc> = "2026-10-16T03:00:00Z".parse().unwrap();
        let said = wake_ups.start(&plan(), started).await.unwrap();
        assert_eq!(said, "started a 30-minute wake-up to 80%, 1 light");
        assert_eq!(wake_ups.ramping(), 1, "one ramp under way");
        assert!(wake_ups.owns(&id("bedroom/ceiling")));
        assert!(!wake_ups.owns(&id("bedroom/lamp")), "excepted");
        assert!(!wake_ups.owns(&id("hall/ceiling")), "not named");
        let ramp = wake_ups.lock().ramps[0].clone();
        assert_eq!(ramp.plugs, ["z2m:kitchen/coffee"]);

        // Half way: still ramping.
        let fade = Duration::from_secs(45);
        wake_ups
            .tick(
                "2026-10-16T03:15:00Z".parse().unwrap(),
                curve(15, 2200),
                fade,
            )
            .await;
        assert_eq!(wake_ups.lock().ramps.len(), 1);

        // Over, at 03:31, with the curve at its night floor: held.
        wake_ups
            .tick(
                "2026-10-16T03:31:00Z".parse().unwrap(),
                curve(15, 2200),
                fade,
            )
            .await;
        assert!(wake_ups.lock().ramps.is_empty());
        assert_eq!(
            wake_ups.lock().holds.get("z2m:bedroom/ceiling"),
            Some(&Hold {
                brightness: 80,
                kelvin: None
            })
        );
        assert!(
            wake_ups.owns(&id("bedroom/ceiling")),
            "the curve leaves it be"
        );

        // With the curve off nothing comes up to meet it: still held.
        wake_ups
            .tick("2026-10-16T05:00:00Z".parse().unwrap(), None, fade)
            .await;
        assert!(wake_ups.owns(&id("bedroom/ceiling")));

        // The curve's morning comes up to meet it: back to the curve.
        wake_ups
            .tick(
                "2026-10-16T06:20:00Z".parse().unwrap(),
                curve(85, 4000),
                fade,
            )
            .await;
        assert!(!wake_ups.owns(&id("bedroom/ceiling")));
    }

    #[tokio::test]
    async fn off_means_off_for_a_wake_up_too() {
        let (wake_ups, _) = bedroom().await;
        wake_ups
            .start(&plan(), "2026-10-16T03:00:00Z".parse().unwrap())
            .await
            .unwrap();
        wake_ups.let_go(&id("bedroom/ceiling")).await;
        assert!(!wake_ups.owns(&id("bedroom/ceiling")));
    }

    #[tokio::test]
    async fn a_light_already_on_is_left_alone() {
        let (wake_ups, _) = bedroom().await;
        let hall = WakePlan {
            rooms: vec![niles_core::RoomName::parse("hall").unwrap()],
            devices: vec![],
            except: vec![],
            ..plan()
        };
        let said = wake_ups
            .start(&hall, "2026-10-16T03:00:00Z".parse().unwrap())
            .await
            .unwrap();
        assert_eq!(said, "woke nothing: every light was already on");
        assert!(!wake_ups.owns(&id("hall/ceiling")));
    }

    #[test]
    fn what_is_kept_reads_back_after_a_restart() {
        let state = State {
            ramps: vec![ramp()],
            holds: BTreeMap::new(),
        };
        let document = serde_json::to_string(&state).unwrap();
        assert_eq!(serde_json::from_str::<State>(&document).unwrap(), state);
    }
}
