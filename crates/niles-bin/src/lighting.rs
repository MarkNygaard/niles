//! The lighting curve and the morning routine, and the `lighting` subcommand that runs them.

use crate::cli::LightingArgs;
use crate::wake_up;
use crate::wiring::automations::command_directly;
use crate::wiring::mqtt::connect_from_config;
use anyhow::Context;
use chrono::{Datelike, Timelike, Utc};
use niles_config::{Config, ConfigStore};
use niles_core::{DeviceId, DeviceRegistry, DeviceState, Event, EventBus};
use niles_mqtt::{CommandRouter, MqttPublisher, Z2mSource, is_actionable};
use niles_scheduler::{
    BRIGHTNESS_DEBOUNCE, KELVIN_DEBOUNCE_K, ManualModeTracker, MinuteOfDay, MorningClaimTracker,
    MorningRoutineConfig, WeekInstant, brightness_at, build_ambient_target, build_curve_target,
    color_temp_at, effective_minute, kick_on_skip, routine_brightness_at, routine_kelvin_at,
    should_fire_today,
};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

/// Current wall-clock time in `tz` converted to a [`MinuteOfDay`].
/// Returns `None` and logs an error if the conversion fails (should
/// be impossible with chrono's contract). The returned `DateTime`
/// is the same instant used for the conversion.
pub(crate) fn current_minute_of_day(
    tz: chrono_tz::Tz,
) -> Option<(MinuteOfDay, chrono::DateTime<chrono_tz::Tz>)> {
    let now = Utc::now().with_timezone(&tz);
    let hour = u8::try_from(now.hour()).expect("chrono::Timelike::hour is 0..=23");
    let minute = u8::try_from(now.minute()).expect("chrono::Timelike::minute is 0..=59");
    match MinuteOfDay::new(hour, minute) {
        Ok(m) => Some((m, now)),
        Err(e) => {
            tracing::error!("could not construct MinuteOfDay from {now}: {e}");
            None
        }
    }
}

/// Where the house is, for sunrise and sunset; none until it is set.
pub(crate) fn house_location(cfg: &Config) -> Option<niles_automations::sun::Location> {
    niles_automations::sun::Location::of(cfg.home.latitude, cfg.home.longitude)
}

/// The typed lighting config the curve loop runs on, plus the snapshot it
/// came from.
///
/// `[lighting]` is the one section a running Niles re-reads (see
/// `niles_config::Reload`), so the loop has to notice when it changes.
/// Deriving `CurveConfig` means parsing time strings and rebuilding the
/// anchor list, which is wasteful once a second — so the snapshot pointer
/// is compared first and the work happens only on an actual change.
pub(crate) struct Lighting {
    pub(crate) snapshot: Arc<Config>,
    pub(crate) curve: niles_scheduler::CurveConfig,
    /// `None` when there is no `[lighting.morning_routine]`, and when
    /// there is one that is switched off.
    ///
    /// Re-derived with the curve rather than built once at startup:
    /// `lighting` reports as `Reload::Hot`, and a routine read only at
    /// boot would have made that a lie — the page would say a change
    /// had applied while nothing had, which is precisely what `Reload`
    /// exists to prevent.
    pub(crate) routine: Option<MorningRoutineConfig>,
}

impl Lighting {
    /// Which lights sit out the curve, as the current config has it.
    pub(crate) fn ambient(&self) -> &HashSet<DeviceId> {
        self.snapshot.ambient_lights.ids()
    }

    /// Which plugs have a lamp on them, as the current config has it.
    pub(crate) fn lamp_plugs(&self) -> &HashSet<DeviceId> {
        self.snapshot.lamp_plugs.ids()
    }

    /// How long the ramps give a light to arrive.
    pub(crate) fn transition(&self) -> Duration {
        self.snapshot.lighting.transition()
    }

    /// The wake-up ramp, if one is configured and switched on.
    pub(crate) fn routine(&self) -> Option<&MorningRoutineConfig> {
        self.routine.as_ref()
    }
}

impl Lighting {
    pub(crate) fn derive(cfg: &Arc<Config>) -> anyhow::Result<Self> {
        Ok(Self {
            snapshot: Arc::clone(cfg),
            curve: cfg
                .lighting
                .to_curve_config()
                .context("converting [lighting] section to a CurveConfig")?,
            routine: derive_routine(cfg)
                .context("converting [lighting.morning_routine] to a MorningRoutineConfig")?,
        })
    }

    /// Re-derive if the store has swapped in a new config. A store only
    /// ever publishes validated snapshots, so a failure here would mean
    /// `to_curve_config` disagreeing with `validate` — worth logging
    /// rather than killing the loop, since the previous curve is still
    /// perfectly usable.
    pub(crate) fn refresh(&mut self, store: &ConfigStore) {
        let snapshot = store.current();
        if Arc::ptr_eq(&snapshot, &self.snapshot) {
            return;
        }
        match snapshot.lighting.to_curve_config() {
            Ok(curve) => {
                // A routine that will not convert keeps the previous
                // one, for the same reason the curve does: the old one
                // still works, and waking somebody up is not a good
                // thing to stop doing quietly.
                match derive_routine(&snapshot) {
                    Ok(routine) => self.routine = routine,
                    Err(e) => tracing::error!(
                        "[lighting.morning_routine] would not convert ({e}); keeping the previous one"
                    ),
                }
                self.snapshot = snapshot;
                self.curve = curve;
                tracing::info!("lighting curve reloaded from a config change");
            }
            Err(e) => {
                tracing::error!(
                    "new config passed validation but its [lighting] section would not convert ({e}); keeping the previous curve"
                );
                // Adopt the snapshot anyway: retrying the same broken
                // conversion every tick would log forever.
                self.snapshot = snapshot;
            }
        }
    }
}

/// The wake-up ramp a config describes, or `None` when it has none or
/// has switched it off.
pub(crate) fn derive_routine(cfg: &Config) -> anyhow::Result<Option<MorningRoutineConfig>> {
    cfg.lighting
        .morning_routine
        .as_ref()
        .filter(|dto| dto.enabled)
        .map(|dto| dto.to_morning_routine_config().map_err(anyhow::Error::from))
        .transpose()
}

/// Keep trying to load config overrides after the backend was unreachable
/// at startup.
///
/// Without this, a pod that restarted while its backend was down — a
/// Postgres failover during a node drain, say, which happens on roughly
/// every cluster upgrade — would run on base defaults for the rest of its
/// life. The lights would be subtly wrong and nothing would ever say why.
///
/// Adopting is safe whenever it succeeds: while the backend is down every
/// write fails, so there is no local change for a late load to clobber.
pub(crate) fn spawn_override_retry(store: Arc<ConfigStore>) {
    tokio::spawn(async move {
        let mut delay = Duration::from_secs(5);
        loop {
            tokio::time::sleep(delay).await;
            match store.reload().await {
                Ok(found) => {
                    tracing::info!(
                        "config overrides loaded from {} after an earlier failure{}",
                        store.backend_description(),
                        if found { "" } else { " (nothing stored yet)" }
                    );
                    return;
                }
                Err(e) => tracing::debug!("config override retry failed: {e}"),
            }
            // Back off to a minute and stay there: this is a background
            // repair, and hammering a database that is failing over helps
            // nobody.
            delay = (delay * 2).min(Duration::from_secs(60));
        }
    });
}

pub(crate) async fn lighting(args: LightingArgs) -> anyhow::Result<()> {
    let (cfg, mqtt_client) = connect_from_config(&args.config).await?;
    let publisher = mqtt_client.publisher();
    let z2m_prefix = cfg.mqtt.z2m_prefix.clone();

    // Time zone first — fail fast on bad config rather than after
    // we've spun up MQTT and Z2mSource.
    let tz: chrono_tz::Tz = cfg.home.timezone.parse().map_err(|e| {
        anyhow::anyhow!(
            "home.timezone '{}' is not a valid IANA zone: {e}",
            cfg.home.timezone
        )
    })?;
    let curve = cfg
        .lighting
        .to_curve_config()
        .context("converting [lighting] section to a CurveConfig")?;

    let registry = Arc::new(DeviceRegistry::new());
    // Subscribe before spawning the source: broadcast receivers only see
    // messages sent after they bind, and we want every off→on transition
    // from the first retained state onwards.
    let bus = EventBus::default();
    let mut bus_rx = bus.subscribe();
    let source = Z2mSource::new(mqtt_client, registry.clone(), bus, z2m_prefix.as_str());
    let source_handle = tokio::spawn(async move {
        if let Err(e) = source.run().await {
            tracing::error!("Z2mSource exited: {e}");
        }
    });

    let tick = Duration::from_secs(args.tick_seconds.max(1));
    let mode = if args.dry_run { " (dry-run)" } else { "" };
    eprintln!(
        "Lighting curve driver running in {tz}{mode}; ticking every {}s. Press Ctrl-C to exit.\n",
        tick.as_secs()
    );

    // Last `(brightness, kelvin)` we published per device. Lets us
    // skip re-sending the same set command while we wait for Z2M to
    // ack the state change — without this, a slow bulb keeps showing
    // its pre-publish brightness and we'd republish every tick.
    let mut last_published: HashMap<DeviceId, DeviceState> = HashMap::new();
    let tracker = ManualModeTracker::new();
    let claim_tracker = MorningClaimTracker::new();

    let mut ticker = tokio::time::interval(tick);
    // The first tick fires immediately; that's what we want — no
    // need to wait a full minute on startup before applying the
    // curve to lights that are already on.
    loop {
        tokio::select! {
            event = bus_rx.recv() => {
                // Same idea as `serve`: a light that just came on gets the
                // curve re-asserted immediately rather than at the next tick.
                let Ok(Event::DeviceStateChanged { id, state }) = event else {
                    continue;
                };
                if !tracker.observe(&id, &state) {
                    continue;
                }
                last_published.remove(&id);
                run_curve_tick(
                    &registry,
                    &publisher,
                    &CommandRouter::z2m_only(&z2m_prefix),
                    &curve,
                    cfg.lighting.ambient_target(),
                    cfg.ambient_lights.ids(),
                    tz,
                    args.dry_run,
                    Duration::ZERO,
                    &mut last_published,
                    &tracker,
                    &claim_tracker,
                    None,
                )
                .await;
            }
            _ = ticker.tick() => {
                if source_handle.is_finished() {
                    anyhow::bail!(
                        "Z2mSource task has exited; the device registry is no longer \
                         being updated, so the curve would publish blindly. Bailing."
                    );
                }
                run_curve_tick(
                    &registry,
                    &publisher,
                    &CommandRouter::z2m_only(&z2m_prefix),
                    &curve,
                    cfg.lighting.ambient_target(),
                    cfg.ambient_lights.ids(),
                    tz,
                    args.dry_run,
                    cfg.lighting.transition(),
                    &mut last_published,
                    &tracker,
                    &claim_tracker,
                    None,
                )
                .await;
            }
            _ = tokio::signal::ctrl_c() => {
                eprintln!("\nReceived Ctrl-C. Exiting.");
                break;
            }
        }
    }

    source_handle.abort();
    Ok(())
}

/// One tick of the curve driver. Reads the current wall-clock time
/// in `tz`, computes the curve, and publishes (or logs, if `dry_run`)
/// a set command for every currently-on light whose state has
/// drifted past the debounce threshold and whose last-published
/// target differs from the current one.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_curve_tick(
    registry: &DeviceRegistry,
    publisher: &MqttPublisher,
    router: &CommandRouter,
    curve: &niles_scheduler::CurveConfig,
    ambient_target: Option<niles_config::AmbientTarget>,
    ambient: &HashSet<DeviceId>,
    tz: chrono_tz::Tz,
    dry_run: bool,
    // Zero on the paths that run because something just happened — a
    // light coming on, or somebody moving a slider in Settings — where
    // a slow creep towards the right level reads as a fault rather
    // than as a sunset.
    fade: Duration,
    last_published: &mut HashMap<DeviceId, DeviceState>,
    tracker: &ManualModeTracker,
    claim_tracker: &MorningClaimTracker,
    wake_ups: Option<&wake_up::WakeUps>,
) {
    // Checked here rather than at each call site, because there are
    // five of them and a sixth would forget. Off means the curve
    // publishes nothing — not that it publishes something neutral.
    if !curve.enabled {
        return;
    }
    let Some((minute_of_day, now)) = current_minute_of_day(tz) else {
        return;
    };
    // Weekly pause window: the curve freezes at the pause's start minute
    // rather than going dark, so e.g. weekend evenings stay bright *and*
    // a light switched on mid-pause still lands on the held value instead
    // of whatever level it was last left at. The morning routine is gated
    // separately by fire_days, so this only affects the curve.
    let now_wi = WeekInstant::new(now.weekday(), minute_of_day);
    let eval_minute = effective_minute(curve, now_wi);
    if eval_minute != minute_of_day {
        tracing::debug!(
            "curve paused at {minute_of_day} ({:?}) — holding the {eval_minute} values",
            now.weekday()
        );
    }
    let target_brightness = brightness_at(curve, eval_minute);
    let target_kelvin = color_temp_at(curve, eval_minute);
    let curve_target = (target_brightness, target_kelvin);

    let mut publish_count = 0usize;
    for device in registry.list_all() {
        if device.state.on != Some(true) {
            continue;
        }
        if tracker.is_flagged(&device.id) {
            continue;
        }
        if claim_tracker.is_claimed(&device.id) {
            continue;
        }
        // A wake-up ramping or holding it.
        if wake_ups.is_some_and(|w| w.owns(&device.id)) {
            continue;
        }
        // Ambient lights sit out the curve and are held at one fixed
        // setting instead. With nothing configured they are left alone
        // entirely, which is how they behaved before.
        let target_state = if device.is_curve_driven(ambient) {
            build_curve_target(
                &device.state,
                device.capabilities,
                curve_target.0,
                curve_target.1,
            )
        } else if device.is_light() && device.room_light {
            match ambient_target {
                Some(want) => build_ambient_target(
                    &device.state,
                    device.capabilities,
                    // Unset brightness follows the curve rather than
                    // freezing wherever the light happened to be.
                    Some(want.brightness.unwrap_or(curve_target.0)),
                    want.kelvin,
                    want.rgb,
                ),
                None => None,
            }
        } else {
            None
        };
        let Some(target_state) = target_state else {
            continue;
        };
        // If we already published exactly this, skip — even if the
        // reported state hasn't caught up yet (Z2M acks lag the set
        // command).
        if last_published.get(&device.id) == Some(&target_state) {
            continue;
        }
        if let Some(commander) = router.commander_for(&device.id, &target_state) {
            let what = format!("[curve {minute_of_day}]");
            if command_directly(commander, &device.id, &target_state, dry_run, &what).await {
                last_published.insert(device.id.clone(), target_state);
            }
            continue;
        }
        let Some((topic, payload)) = router.format_fading(&device.id, &target_state, fade) else {
            continue;
        };
        debug_assert!(
            is_actionable(&target_state),
            "build_curve_target should only return actionable targets"
        );
        let ok = if dry_run {
            tracing::info!("[curve {minute_of_day}] [dry-run] {topic}  {payload}");
            true
        } else {
            match publisher.publish(&topic, payload.clone()).await {
                Ok(()) => {
                    tracing::info!("[curve {minute_of_day}] {topic}  {payload}");
                    true
                }
                Err(e) => {
                    tracing::warn!("[curve {minute_of_day}] {topic} failed: {e}");
                    false
                }
            }
        };
        if ok {
            // A WLED strip never reports its white balance, so nothing
            // would ever update the registry and the curve would find
            // it off-curve again next tick — every minute, for ever,
            // and over the top of anyone who set it by hand.
            if let Some(echo) = niles_mqtt::unechoed(&device.id, &target_state) {
                registry.merge_state(&device.id, echo);
            }
            last_published.insert(device.id.clone(), target_state);
        }
        publish_count += 1;
    }
    tracing::debug!(
        "curve tick at {minute_of_day} in {tz}: brightness={target_brightness}, \
         kelvin={target_kelvin}K, devices_touched={publish_count}"
    );
}

/// Whether a light's reported warmth is far enough from `want` to send,
/// by the curve's rule. A light that reports none gets it with the
/// brightness only.
pub(crate) fn kelvin_drifted(reported: Option<u16>, want: Option<u16>) -> bool {
    matches!((reported, want), (Some(now), Some(want)) if now.abs_diff(want) > KELVIN_DEBOUNCE_K)
}

/// One tick of the morning routine. Behaviour:
///
/// - At `morning_start` on a fire-day: claim every off target device
///   and publish `on: true, brightness: 0`.
/// - During the window: for each currently-claimed device, compute the
///   target via `routine_brightness_at` and publish a brightness update
///   if it differs from current state by strictly more than
///   `BRIGHTNESS_DEBOUNCE`, or its warmth has drifted from
///   `routine_kelvin_at`'s.
/// - At `morning_end` or after: release any device the routine still
///   has claimed.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_morning_routine_tick(
    registry: &DeviceRegistry,
    publisher: &MqttPublisher,
    router: &CommandRouter,
    routine: &MorningRoutineConfig,
    morning_start: MinuteOfDay,
    morning_end: MinuteOfDay,
    tz: chrono_tz::Tz,
    dry_run: bool,
    // Used by the ramp and by nothing else here: switching a light on
    // at zero, and the shove to full at the end, are both meant to
    // happen the moment they happen.
    fade: Duration,
    tracker: &ManualModeTracker,
    claim_tracker: &MorningClaimTracker,
    ambient: &HashSet<DeviceId>,
    lamp_plugs: &HashSet<DeviceId>,
    wake_ups: Option<&wake_up::WakeUps>,
    // The curve's warmth now, `None` with the curve off: what the lights
    // are on when the routine has none of its own.
    curve_kelvin: Option<u16>,
) {
    // Deliberately not gated on `lighting.enabled`: that switch governs
    // the curve, and the routine is opted into separately by having a
    // `[lighting.morning_routine]` at all. Turning the curve off and
    // silently losing a wake-up ramp would be a poor trade for whoever
    // relies on it to get up.
    let Some((minute_of_day, now)) = current_minute_of_day(tz) else {
        return;
    };
    let today = now.date_naive();
    let warmth = routine_kelvin_at(
        routine,
        minute_of_day,
        morning_start,
        morning_end,
        curve_kelvin,
    );

    // Effective target set: an empty `target_devices` means "every
    // curve-managed light" (all non-ambient lights, same set the curve
    // drives), resolved fresh each tick so newly-added lights are included.
    // A non-empty list is used verbatim. `exclude_devices` is then removed
    // from whichever base we picked — so empty target + exclude = "all
    // lights except these".
    let exclude: HashSet<&DeviceId> = routine.exclude_devices.iter().collect();
    let target_ids: Vec<DeviceId> = if routine.target_devices.is_empty() {
        registry
            .list_all()
            .into_iter()
            // Non-ambient lights (ramped) + lamps on plugs (switched on
            // at the end-minute, since they can't ramp).
            .filter(|d| d.is_lamp(lamp_plugs) && !ambient.contains(&d.id))
            .map(|d| d.id)
            .collect()
    } else {
        routine.target_devices.clone()
    };
    let target_ids: Vec<DeviceId> = target_ids
        .into_iter()
        .filter(|id| !exclude.contains(id))
        .collect();

    // Phase 1 — at exact end-minute, force 100% once then release.
    // After the end-minute, only release leftovers.
    if minute_of_day >= morning_end {
        // Outlets (plugs) can't ramp, so they switch ON once at the
        // end-minute — by now the dimmable lights have reached full, so the
        // plug joining in isn't a jarring jump. (An outlet is switchable but
        // not a light; the curve never touches it, so there's no claim.)
        if minute_of_day == morning_end {
            for id in &target_ids {
                let Some(device) = registry.get(id) else {
                    continue;
                };
                if device.is_light() || !device.is_switchable() {
                    continue; // lights handled below; non-switchables ignored
                }
                if tracker.is_flagged(id) || device.state.on == Some(true) {
                    continue;
                }
                let target = DeviceState {
                    on: Some(true),
                    ..Default::default()
                };
                if let Some(commander) = router.commander_for(id, &target) {
                    let what = format!("[routine {minute_of_day}]");
                    command_directly(commander, id, &target, dry_run, &what).await;
                    continue;
                }
                let Some((topic, payload)) = router.format(id, &target) else {
                    continue;
                };
                if dry_run {
                    tracing::info!("[routine {minute_of_day}] [dry-run] {topic}  {payload}");
                } else if let Err(e) = publisher.publish(&topic, payload.clone()).await {
                    tracing::warn!("[routine {minute_of_day}] {topic} failed: {e}");
                } else {
                    tracing::info!("[routine {minute_of_day}] {topic}  {payload}");
                }
            }
        }
        for id in &target_ids {
            if !claim_tracker.is_claimed(id) {
                continue;
            }

            if minute_of_day == morning_end {
                let Some(device) = registry.get(id) else {
                    claim_tracker.release(id);
                    tracing::info!("[routine {minute_of_day}] released {id}");
                    continue;
                };
                if device.is_curve_driven(ambient)
                    && (device.state.brightness != Some(100)
                        || kelvin_drifted(device.state.color_temp_kelvin, warmth))
                {
                    let target = DeviceState {
                        brightness: Some(100),
                        color_temp_kelvin: warmth,
                        ..Default::default()
                    };
                    if let Some(commander) = router.commander_for(id, &target) {
                        let what = format!("[routine {minute_of_day}]");
                        command_directly(commander, id, &target, dry_run, &what).await;
                    } else {
                        let Some((topic, payload)) = router.format(id, &target) else {
                            continue;
                        };
                        if dry_run {
                            tracing::info!(
                                "[routine {minute_of_day}] [dry-run] {topic}  {payload}"
                            );
                        } else if let Err(e) = publisher.publish(&topic, payload.clone()).await {
                            tracing::warn!("[routine {minute_of_day}] {topic} failed: {e}");
                        } else {
                            tracing::info!("[routine {minute_of_day}] {topic}  {payload}");
                        }
                    }
                }
            }

            claim_tracker.release(id);
            tracing::info!("[routine {minute_of_day}] released {id}");
        }
        return;
    }
    if minute_of_day < morning_start {
        return;
    }

    // Phase 2 — at start, on a fire-day: claim + kick-on devices that are off.
    let mut just_kicked_on = HashSet::new();
    let firing = should_fire_today(routine, today);
    if minute_of_day == morning_start && !firing {
        tracing::info!(
            "[routine {minute_of_day}] not a fire-day ({}), sitting this one out",
            today.weekday()
        );
    }
    if minute_of_day == morning_start && firing {
        for id in &target_ids {
            // The rule lives in `kick_on_skip`, where it can be tested;
            // this end only says what it decided. A wake-up that does
            // not happen is noticed hours later in a dark room, so the
            // reason has to survive to be read then — `info`, not
            // `debug`.
            let device = registry.get(id);
            if let Some(reason) = kick_on_skip(
                device.is_some(),
                device.as_ref().is_some_and(|d| d.is_curve_driven(ambient)),
                device.as_ref().and_then(|d| d.state.on),
                claim_tracker.is_claimed(id) || wake_ups.is_some_and(|w| w.owns(id)),
            ) {
                tracing::info!(
                    "[routine {minute_of_day}] skipped {id}: {}",
                    reason.reason()
                );
                continue;
            }
            let target = DeviceState {
                on: Some(true),
                brightness: Some(0),
                color_temp_kelvin: warmth,
                ..Default::default()
            };
            if let Some(commander) = router.commander_for(id, &target) {
                let what = format!("[routine {minute_of_day}]");
                if command_directly(commander, id, &target, dry_run, &what).await {
                    claim_tracker.claim(id);
                    just_kicked_on.insert(id.clone());
                }
                continue;
            }
            let Some((topic, payload)) = router.format(id, &target) else {
                continue;
            };
            let ok = if dry_run {
                tracing::info!("[routine {minute_of_day}] [dry-run] {topic}  {payload}");
                true
            } else {
                match publisher.publish(&topic, payload.clone()).await {
                    Ok(()) => {
                        tracing::info!("[routine {minute_of_day}] {topic}  {payload}");
                        true
                    }
                    Err(e) => {
                        tracing::warn!("[routine {minute_of_day}] {topic} failed: {e}");
                        false
                    }
                }
            };
            if ok {
                claim_tracker.claim(id);
                just_kicked_on.insert(id.clone());
            }
        }
    }

    // Phase 3 — during window: drive the ramp for claimed devices.
    let Some(target_brightness) = routine_brightness_at(minute_of_day, morning_start, morning_end)
    else {
        return;
    };
    for id in &target_ids {
        if tracker.is_flagged(id) {
            continue;
        }
        if !claim_tracker.is_claimed(id) {
            continue;
        }
        if just_kicked_on.contains(id) {
            continue;
        }
        let Some(device) = registry.get(id) else {
            continue;
        };
        if !device.is_curve_driven(ambient) {
            continue;
        }
        let should_publish = match device.state.brightness {
            Some(cur) => cur.abs_diff(target_brightness) > BRIGHTNESS_DEBOUNCE,
            None => true,
        } || kelvin_drifted(device.state.color_temp_kelvin, warmth);
        if !should_publish {
            continue;
        }
        let target = DeviceState {
            brightness: Some(target_brightness),
            color_temp_kelvin: warmth,
            ..Default::default()
        };
        if let Some(commander) = router.commander_for(id, &target) {
            let what = format!("[routine {minute_of_day}]");
            command_directly(commander, id, &target, dry_run, &what).await;
            continue;
        }
        let Some((topic, payload)) = router.format_fading(id, &target, fade) else {
            continue;
        };
        if dry_run {
            tracing::info!("[routine {minute_of_day}] [dry-run] {topic}  {payload}");
        } else {
            match publisher.publish(&topic, payload.clone()).await {
                Ok(()) => {
                    tracing::info!("[routine {minute_of_day}] {topic}  {payload}");
                }
                Err(e) => {
                    tracing::warn!("[routine {minute_of_day}] {topic} failed: {e}");
                }
            }
        }
    }
}
