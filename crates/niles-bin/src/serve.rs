//! `niles serve`: everything connected and running.

use crate::app::{AppChat, AppMusic};
use crate::cli::ServeArgs;
use crate::dispatch::DispatchCtx;
use crate::dispatch::targets::{is_wall_switch, switch_targets};
use crate::lighting::{
    Lighting, current_minute_of_day, house_location, run_curve_tick, run_morning_routine_tick,
    spawn_override_retry,
};
use crate::prompt::{build_capability_index, build_capability_loader, build_initial_device_index};
use crate::recognition::build_speaker_identifier;
use crate::satellites::SatelliteRegistry;
use crate::timers::spawn_timer_driver;
use crate::voice::{hush_on_wake, spawn_dispatch_task};
use crate::wiring::accounts::{
    build_calendars, build_push, build_secret_store, build_strava, build_tado_tokens,
    build_weather_client, ensure_session_key, forget_retired_provider_keys,
};
use crate::wiring::automations::{
    CenterNotifier, MqttDeviceSink, automation_rules, automation_waits, build_automation_store,
    command_directly,
};
use crate::wiring::llm::{build_llm_client, build_tier2_client, build_tool_registry};
use crate::wiring::memory::{
    append_state_history, build_chat_log, build_conversation_memory, build_journal,
    build_memory_store, build_skill_store, build_state_reader, build_state_writer,
    spawn_skill_curator,
};
use crate::wiring::mqtt::{command_router, connect_with_config, spawn_wled_source};
use crate::wiring::notifications::{WyomingDelivery, build_notification_center};
use crate::wiring::presence::{build_presence, build_tado, spawn_presence_poll_loop};
use crate::wiring::speech::{LiveStt, build_enrollment_backend, build_piper_client, stt_base_url};
use crate::wiring::stores::{
    build_artifacts, build_capture_store, build_grocery_store, build_morning_claims,
    build_override_backend, build_room_music, build_scene_store, build_timer_store, wake_up_store,
};
use crate::{
    LOG_BUFFER, approvals, artifacts, assistant, calendar, courtesy, grocery_delivery, last_target,
    mail, music, plugins, profile, review, ringer, speakers, tool_servers, training, tv,
    vocabulary, wake_up,
};
use anyhow::Context;
use chrono::{Datelike, Utc};
use niles_api::{AppState, DevicePublisher};
use niles_automations::AutomationEngine;
use niles_config::{Config, ConfigStore, LoadOutcome};
use niles_core::{DeviceId, DeviceRegistry, DeviceState, Event, EventBus, RoomName};
use niles_history::{CommandReader, CommandWriter};
use niles_mqtt::Z2mSource;
use niles_scheduler::{
    ManualModeTracker, SwitchEffect, WeekInstant, brightness_at, classify_action, color_temp_at,
    effective_minute,
};
use niles_tools::ToolRegistry;
use niles_wyoming::{SessionTracker, WyomingServer};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

pub(crate) async fn serve(args: ServeArgs) -> anyhow::Result<()> {
    // Where the override document lives is deliberately *not* itself
    // overridable — it's how we find the overrides in the first place — so
    // it comes from a plain read of the base file.
    let bootstrap = match &args.config {
        Some(path) => Config::load_from_path(path)
            .with_context(|| format!("loading config from {}", path.display()))?,
        None => Config::load_from_str("").context("Niles's own defaults")?,
    };
    let backend = build_override_backend(&bootstrap)?;
    let base = args.config.as_deref();
    let (store, outcome) = ConfigStore::open(base, backend)
        .await
        .with_context(|| match base {
            Some(path) => format!("loading config from {}", path.display()),
            None => "starting from Niles's defaults".to_string(),
        })?;
    let store = Arc::new(store);
    // Never fatal: the base still loaded. But say it loudly — the user's
    // tuning just silently reverted to file defaults.
    tracing::info!("config state: {}", store.backend_description());
    match &outcome {
        LoadOutcome::Clean => {}
        LoadOutcome::OverridesRejected(why) => {
            tracing::warn!("config overrides rejected, using base config alone: {why}");
        }
        LoadOutcome::BackendUnavailable(why) => {
            tracing::error!(
                "could not load config overrides ({why}); running on base config.                  Retrying in the background — tuning will be picked up when the                  backend returns."
            );
            spawn_override_retry(store.clone());
        }
        other => tracing::warn!("config overrides not applied ({other:?})"),
    }
    if !store.is_persistent() {
        tracing::warn!(
            "config changes apply immediately but are lost on restart —              no persistent backend is configured"
        );
    }

    let cfg = store.current();
    for retired in &cfg.retired {
        tracing::warn!("config names something Niles no longer has: {retired:?}");
    }

    // Before the broker, because the broker password may be one of
    // these. Everything that reads a credential goes through
    // `require_secret`, and until this runs that only sees the
    // environment.
    let secret_store = build_secret_store(&cfg);
    let captures = build_capture_store(&cfg);
    if let Some(secrets) = &secret_store {
        match secrets.load_all().await {
            Ok(values) => {
                let n = values.len();
                let retired = niles_config::secrets::retired_provider_keys(values.keys());
                niles_config::secrets::load(values);
                tracing::info!("[secrets] {n} loaded from the database");
                forget_retired_provider_keys(secrets, &retired).await;
                ensure_session_key(&cfg, secrets).await;
                niles_api::api_token::ensure(&cfg, secrets.as_ref()).await;
            }
            // Not fatal: an install whose credentials are all env vars
            // does not need these, and refusing to start would make a
            // database blip take the house down with it.
            Err(e) => tracing::error!("[secrets] could not be read: {e}"),
        }
    }

    let mqtt_client = connect_with_config(&cfg).await?;
    let publisher = mqtt_client.publisher();
    let api_bind = cfg
        .api
        .socket_addr()
        .context("resolving api.bind_address")?;
    let wyoming_bind = cfg
        .wyoming
        .socket_addr()
        .context("resolving wyoming.bind_address")?;

    let tz: chrono_tz::Tz = cfg.home.timezone.parse().map_err(|e| {
        anyhow::anyhow!(
            "home.timezone '{}' is not a valid IANA zone: {e}",
            cfg.home.timezone
        )
    })?;
    let mut lighting = Lighting::derive(&cfg)?;
    match lighting.routine() {
        Some(r) if r.target_devices.is_empty() => tracing::info!(
            "morning routine: ALL non-ambient lights + plugs, fires {:?}",
            r.fire_days
        ),
        Some(r) => tracing::info!(
            "morning routine: {} device(s), fires {:?}",
            r.target_devices.len(),
            r.fire_days
        ),
        None => tracing::info!("morning routine: off (no wake-up light)"),
    }

    let whisper = Arc::new(LiveStt::new(Some(store.clone()), &cfg)?);
    let piper = Arc::new(build_piper_client(&cfg)?);

    let registry = Arc::new(DeviceRegistry::new());
    let bus = EventBus::default();
    let tracker = Arc::new(ManualModeTracker::new());
    let claim_tracker = build_morning_claims(&cfg).await;

    // Third-party plugins. Their devices are commanded through the
    // runtime rather than MQTT, so the router learns of it before
    // anything that sends a command is built; their tools join the
    // registry below, and they are started once everything they reach
    // exists.
    let plugin_tools = niles_tools::DynamicTools::new();
    let plugin_host = plugins::start_runtime(
        &cfg,
        store.clone(),
        registry.clone(),
        bus.clone(),
        plugin_tools.clone(),
        secret_store.clone(),
    );
    // Remote tool servers put their tools beside the plugins'. After the
    // secrets are loaded, since each connects with its key.
    let approvals = Arc::new(approvals::Approvals::default());
    let tool_servers =
        tool_servers::ToolServers::start(store.clone(), plugin_tools.clone(), approvals.clone());
    let plugin_database = plugin_host.as_ref().is_some_and(|(_, database)| *database);
    let plugin_runtime = plugin_host.map(|(runtime, _)| runtime);
    let router = Arc::new(match &plugin_runtime {
        Some(runtime) => command_router(&cfg).with_commander(Arc::new(runtime.clone())),
        None => command_router(&cfg),
    });
    // Wake-ups automations start, picked up where they were.
    let wake_ups = wake_up::WakeUps::load(
        wake_up_store(&cfg),
        registry.clone(),
        publisher.clone(),
        router.clone(),
        store.clone(),
        tracker.clone(),
        claim_tracker.clone(),
        args.dry_run,
    )
    .await;

    // Device index: built from the registry at startup and updated
    // on DeviceAdded / DeviceRemoved so Tier 0 matchers can resolve
    // spoken device names without an LLM round-trip.
    let device_index = Arc::new(RwLock::new(build_initial_device_index(&registry)));

    // Subscribe to the bus *before* spawning the source so we can't miss
    // the early DeviceStateChanged events that seed the observer's
    // last-seen on/off map — broadcast channels only deliver messages
    // sent after a receiver is bound.
    let state_writer = build_state_writer(&cfg)?;
    // Off→on transitions spotted by the observer are forwarded here so the
    // curve loop can re-assert the curve on the light straight away instead
    // of leaving it at its restored level until the next tick.
    let (curve_nudge_tx, mut curve_nudge_rx) = tokio::sync::mpsc::unbounded_channel::<DeviceId>();
    let observer_tracker = tracker.clone();
    let observer_claim_tracker = claim_tracker.clone();
    let observer_wake_ups = wake_ups.clone();
    let observer_registry = registry.clone();
    let observer_config = store.clone();
    let observer_device_index = Arc::clone(&device_index);
    let observer_publisher = publisher.clone();
    let observer_router = router.clone();
    let observer_dry_run = args.dry_run;
    let observer_state_writer = state_writer.clone();
    let mut bus_rx = bus.subscribe();
    let source = Z2mSource::new(
        mqtt_client,
        registry.clone(),
        bus.clone(),
        cfg.mqtt.z2m_prefix.as_str(),
    );
    let source_handle = tokio::spawn(async move {
        if let Err(e) = source.run().await {
            tracing::error!("Z2mSource exited: {e}");
        }
    });
    let wled_handle = spawn_wled_source(&cfg, registry.clone(), bus.clone()).await;

    // EventBus observer — feeds tracker.observe() for off→on
    // auto-clear, and tracker.forget() so removed devices don't
    // leave stale tracker entries behind (the maps would otherwise
    // grow monotonically over a long-running service).
    // Also releases morning-routine claims when a device is turned
    // off mid-ramp, and forgets claims on device removal.
    let observer_handle = tokio::spawn(async move {
        loop {
            match bus_rx.recv().await {
                Ok(Event::DeviceStateChanged { id, state }) => {
                    if observer_tracker.observe(&id, &state) {
                        // Turned on: ask the curve loop for an immediate pass.
                        // A closed receiver just means we're shutting down.
                        let _ = curve_nudge_tx.send(id.clone());
                    }
                    // Mid-ramp off cancels the routine for the rest of today.
                    if state.on == Some(false) && observer_claim_tracker.is_claimed(&id) {
                        observer_claim_tracker.release(&id);
                    }
                    // And a wake-up's, or its hold: off means off.
                    if state.on == Some(false) && observer_wake_ups.owns(&id) {
                        observer_wake_ups.let_go(&id).await;
                    }
                    append_state_history(&observer_state_writer, &id, &state);
                }
                Ok(Event::DeviceAdded { device }) => {
                    if device.is_light() {
                        observer_device_index
                            .write()
                            .unwrap_or_else(|e| e.into_inner())
                            .insert(device.id.clone());
                    }
                }
                Ok(Event::DeviceRemoved { id }) => {
                    observer_tracker.forget(&id);
                    observer_claim_tracker.forget(&id);
                    observer_wake_ups.let_go(&id).await;
                    observer_device_index
                        .write()
                        .unwrap_or_else(|e| e.into_inner())
                        .remove(&id);
                }
                Ok(Event::DeviceAction { id, action }) => {
                    if !is_wall_switch(&observer_registry, &id) {
                        continue;
                    }
                    let Some(effect) = classify_action(&action) else {
                        // Includes _release variants and unknown strings.
                        continue;
                    };
                    let lamps = observer_config.current().lamp_plugs.ids().clone();
                    let targets = switch_targets(&observer_registry, &id, &lamps);
                    if targets.is_empty() {
                        tracing::debug!(
                            "switch {id} pressed but no actionable lights for room {}",
                            id.room()
                        );
                        continue;
                    }
                    let desired = match effect {
                        SwitchEffect::TurnOnRoom => DeviceState {
                            on: Some(true),
                            ..Default::default()
                        },
                        SwitchEffect::TurnOffRoom => DeviceState {
                            on: Some(false),
                            ..Default::default()
                        },
                        SwitchEffect::StepBrightness { delta_percent } => {
                            let known: Vec<u8> =
                                targets.iter().filter_map(|d| d.state.brightness).collect();
                            let base: i16 = if known.is_empty() {
                                50
                            } else {
                                (known.iter().copied().map(|b| b as i32).sum::<i32>()
                                    / known.len() as i32) as i16
                            };
                            let next = (base + delta_percent).clamp(0, 100) as u8;
                            DeviceState {
                                brightness: Some(next),
                                ..Default::default()
                            }
                        }
                    };
                    for d in &targets {
                        observer_tracker.flag(&d.id);
                    }
                    for d in &targets {
                        if let Some(commander) = observer_router.commander_for(&d.id, &desired) {
                            command_directly(
                                commander,
                                &d.id,
                                &desired,
                                observer_dry_run,
                                "[switch]",
                            )
                            .await;
                            continue;
                        }
                        let Some((topic, payload)) = observer_router.format(&d.id, &desired) else {
                            continue;
                        };
                        if observer_dry_run {
                            println!("[switch] [dry-run] {topic}  {payload}");
                        } else {
                            match observer_publisher.publish(&topic, payload.clone()).await {
                                Ok(()) => println!("[switch] published {topic}  {payload}"),
                                Err(e) => tracing::warn!("[switch] publish to {topic} failed: {e}"),
                            }
                        }
                    }
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("ManualModeTracker observer lagged by {n} events");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // No Z2M warm-up needed here: tool calls only fire after a
    // transcript arrives, by which point Z2M has had seconds to
    // populate. See voice_dispatch for the same reasoning.
    let llm = Arc::new(build_llm_client(&cfg)?);
    let timers = build_timer_store(&cfg).await;
    let scenes = build_scene_store(&cfg).await;
    let groceries = build_grocery_store(&cfg).await;
    let automations = build_automation_store(&cfg).await;
    // What each rule did lately: written by the engine, read by the app
    // and by voice.
    let automation_runs = Arc::new(niles_automations::RunLog::new());
    // The rule Niles last read back, saved by a plain yes.
    let proposals = Arc::new(niles_tools::automations::Proposals::new());
    let calendars = build_calendars(&cfg);
    let capability_loader = build_capability_loader();
    let capability_index = capability_loader
        .as_deref()
        .map(build_capability_index)
        .map(Arc::new);

    let memory_store = build_memory_store(&cfg).await;
    let skill_store = build_skill_store(&cfg).await;
    let weather_client = build_weather_client();
    let _skill_curator_handle = skill_store
        .clone()
        .and_then(|store| spawn_skill_curator(store, cfg.skills.curator.clone()));
    let search = niles_tools::Search::live(store.clone());
    // Kept separately from the poll loop: the connection has to exist
    // for Settings to authorise it even when nothing polls it yet.
    let tado = build_tado(&cfg.presence, build_tado_tokens(&cfg)?);
    let presence = build_presence(&cfg.presence, tado.clone(), Some(store.clone()));
    let _presence_handle = presence.as_ref().map(|p| {
        spawn_presence_poll_loop(
            p.aggregator.clone(),
            p.sources.clone(),
            p.unifi.clone(),
            p.watching.clone(),
            cfg.presence.clone(),
            Some(store.clone()),
            bus.clone(),
        )
    });
    let presence_agg = presence.as_ref().map(|p| p.aggregator.clone());
    let mut tools = build_tool_registry(
        registry.clone(),
        publisher.clone(),
        router.clone(),
        capability_loader.clone(),
        memory_store.clone(),
        skill_store.clone(),
        weather_client.clone(),
        search.clone(),
        presence_agg.clone(),
        &cfg.home,
        Some(store.clone()),
        args.dry_run,
    );
    niles_tools::register_timer_tools(&mut tools, timers.clone());
    // Lets the user retune the curve by voice: "make the evenings
    // brighter", "start the morning ramp half an hour later".
    niles_tools::register_config_tools(&mut tools, store.clone());
    if let Some(automations) = &automations {
        niles_tools::register_automation_tools(
            &mut tools,
            automations.clone(),
            registry.clone(),
            automation_runs.clone(),
            proposals.clone(),
            tz,
        );
    }
    if let (Some(calendars), Ok(tz)) = (&calendars, cfg.home.timezone.parse::<chrono_tz::Tz>()) {
        let shared = calendar::register(&mut tools, calendars.clone(), store.clone(), tz);
        mail::register(&mut tools, shared, approvals.clone());
    }
    let (arrivals, mut arrived) = tokio::sync::mpsc::unbounded_channel();
    let strava = build_strava(&cfg, arrivals);
    let chat_log = build_chat_log(&cfg);
    let push = build_push(&cfg);
    // Made by the assistant, which runs their jobs: the runner is handed
    // the assistant once it has started serving the tools.
    let artifact_runner = Arc::new(artifacts::AssistantRunner {
        assistant: std::sync::OnceLock::new(),
        settings: store.clone(),
    });
    let artifacts = build_artifacts(
        &cfg,
        bus.clone(),
        artifact_runner.clone(),
        chat_log.clone(),
        search.clone(),
        strava.as_ref().map(|s| s.store().clone()),
        push.clone(),
    );
    if let (Some(strava), Ok(tz)) = (&strava, cfg.home.timezone.parse::<chrono_tz::Tz>()) {
        let imperial = cfg.home.resolved_units() == niles_config::Units::Imperial;
        training::register(&mut tools, strava.store().clone(), tz, imperial);
        if let Some(artifacts) = &artifacts {
            training::register_plan(
                &mut tools,
                strava.store().clone(),
                artifacts.store().clone(),
                tz,
                imperial,
            );
        }
    }
    // The plans follow the runs from the first sync on, so this listens
    // before the poll begins.
    if let Some(artifacts) = &artifacts {
        let artifacts = artifacts.clone();
        tokio::spawn(async move {
            while let Some(runs) = arrived.recv().await {
                artifacts.runs_arrived(&runs).await;
            }
        });
    }
    if let Some(strava) = &strava {
        tokio::spawn(strava.clone().poll());
    }
    if let Some(groceries) = &groceries {
        niles_tools::register_grocery_tools(
            &mut tools,
            groceries.clone(),
            cfg.home.resolved_country(),
        );
    }
    // One session for the app, the tools and the delivery reminder:
    // nemlig sees one login, not three.
    let nemlig = match niles_nemlig::NemligClient::new() {
        Ok(client) => Some(Arc::new(client)),
        Err(e) => {
            tracing::warn!("[nemlig] unavailable: {e}");
            None
        }
    };
    if let Some(client) = &nemlig {
        niles_tools::register_nemlig_tools(&mut tools, client.clone(), store.clone());
    }

    let command_writer = match &cfg.history.directory {
        Some(dir) => {
            let w = CommandWriter::new(dir).context("opening command history writer")?;
            if let Err(e) = w.prune(cfg.history.retention_days) {
                tracing::warn!("history prune failed: {e:#}");
            }
            w
        }
        None => CommandWriter::disabled(),
    };
    let command_writer = Arc::new(command_writer);
    let command_reader = match &cfg.history.directory {
        Some(dir) => Arc::new(CommandReader::new(dir)),
        None => Arc::new(CommandReader::disabled()),
    };
    niles_tools::register_history_tools(&mut tools, command_reader);

    let state_reader = build_state_reader(&cfg);
    niles_tools::register_state_history_tools(&mut tools, state_reader, registry.clone());

    let tier2 = build_tier2_client(&cfg)?;
    if tier2.is_some() {
        niles_tools::register_escalate_tool(&mut tools);
    }
    // Concrete, so it can be handed to dispatch as a `SpeakerIdentifier`
    // and to the app as a `VoiceRoster` — the same object answering two
    // questions, rather than two views of the roster that can disagree.
    let recogniser = build_speaker_identifier(&cfg.recognition, build_enrollment_backend(&cfg)?)
        .await
        .context("initializing speaker recognition")?;
    let identifier = recogniser
        .clone()
        .map(|r| r as Arc<dyn crate::recognition::SpeakerIdentifier>);
    let voices = recogniser.map(|r| r as Arc<dyn niles_recognition::VoiceRoster>);
    if let Some(roster) = &voices {
        profile::register(&mut tools, roster.clone());
    }
    let (server, mut rx, mut disconnects_rx) = WyomingServer::bind(wyoming_bind)
        .await
        .with_context(|| format!("binding Wyoming server on {wyoming_bind}"))?;

    let satellites = Arc::new(SatelliteRegistry::from_config(&cfg.satellites));
    let speakers = Arc::new(speakers::SpeakerRegistry::new(store.clone()));
    let music =
        Arc::new(music::Music::new(speakers.clone(), store.clone(), build_room_music(&cfg)).await);
    let tv = Arc::new(tv::TvControl::new(store.clone()));
    music::register(&mut tools, music.clone(), tv.clone());
    tv::register(&mut tools, tv.clone());
    let _vocabulary = vocabulary::spawn_refresh(
        whisper.vocabulary(),
        registry.clone(),
        store.clone(),
        scenes.clone(),
        music.clone(),
    );

    // Notification center
    let mut notifications = build_notification_center(&cfg.notifications, &cfg.home.timezone);
    let peer_index: Arc<Mutex<HashMap<RoomName, SocketAddr>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let wyoming_delivery = Arc::new(WyomingDelivery {
        piper: piper.clone(),
        sender: server.sender(),
        speakers: speakers.clone(),
        satellites: satellites.clone(),
        peer_index: peer_index.clone(),
        tv: tv.clone(),
    });
    notifications.set_delivery(wyoming_delivery);
    let notifications = Arc::new(notifications);

    // Automation engine
    let automation_sink = Arc::new(MqttDeviceSink {
        publisher: publisher.clone(),
        router: router.clone(),
        dry_run: args.dry_run,
    });

    // Lights that follow the house emptying and filling. Read at
    // startup like the rules beside it: both build a task around the
    // settings, and neither rebuilds it underneath itself.
    let presence_lights = Arc::new(niles_automations::PresenceLights::new(
        registry.clone(),
        automation_sink.clone(),
        cfg.presence.lights_off_when_away,
        cfg.presence
            .arrival_lights()
            .inspect_err(|e| tracing::warn!("presence lights disabled: {e}"))
            .unwrap_or_default(),
    ));
    let _presence_lights_handle = presence_lights
        .wanted()
        .then(|| presence_lights.clone().spawn(bus.clone()));
    let automation_notifier = Arc::new(CenterNotifier {
        center: notifications.clone(),
    });
    // Always running, even with no rules: one made by voice or in the
    // app starts working without a restart.
    let automation_engine = Arc::new(
        AutomationEngine::new(
            automation_rules(automations.as_deref(), &cfg.automations),
            registry.clone(),
            tz,
            automation_sink,
            Some(automation_notifier),
        )
        .with_location(house_location(&cfg))
        .with_log(automation_runs.clone())
        .with_waker(Some(wake_ups.clone() as Arc<dyn niles_automations::Waker>))
        .with_waits(automation_waits(&cfg))
        .with_settings(Some(store.clone())),
    );
    let _automation_handle = automation_engine.clone().spawn(bus.clone());
    if let Some(store) = automations.clone() {
        let engine = automation_engine.clone();
        let from_config = cfg.automations.clone();
        let mut changes = store.subscribe();
        tokio::spawn(async move {
            while changes.changed().await.is_ok() {
                engine
                    .replace(automation_rules(Some(&store), &from_config))
                    .await;
            }
        });
    }

    niles_tools::register_notification_tools(&mut tools, notifications.clone());
    // Taken before the plugins' tools join: the names no plugin may use.
    let builtin_tool_names = tools.builtin_names();
    tools.attach_dynamic(plugin_tools);

    let tools = Arc::new(tools);
    if let Some(artifacts) = &artifacts {
        let artifacts = artifacts.clone();
        tokio::spawn(async move { artifacts.recover().await });
    }
    let assistant_only = artifacts
        .as_ref()
        .map_or_else(ToolRegistry::new, |a| a.tools());
    // Started whether or not an assistant is switched on: the switch is
    // in the app and read per message, and a port bound only at startup
    // would leave turning it on waiting for a restart.
    let assistant = match assistant::Assistant::start(
        tools.clone(),
        assistant_only,
        secret_store.clone(),
        search.clone(),
    )
    .await
    {
        Ok(assistant) => Some(Arc::new(assistant)),
        Err(e) => {
            tracing::warn!("[assistant] unavailable: {e:#}");
            None
        }
    };
    if let Some(assistant) = &assistant {
        let _ = artifact_runner.assistant.set(assistant.clone());
    }

    // What Niles learns: the assistant reviews what was said once the
    // house is quiet, and tidies memory and skills weekly. Which one, if
    // any, is read each time, like the chat's.
    let journal = build_journal(&cfg).await;
    let _learning = assistant.clone().map(|assistant| {
        review::Learning {
            assistant,
            settings: store.clone(),
            journal: journal.clone(),
            memory: memory_store.clone(),
            skills: skill_store.clone(),
            automations: automations.clone(),
            voices: voices.clone(),
            tidy_tried: Mutex::default(),
        }
        .spawn()
    });

    // Timer driver: shares the `timers` Arc registered with the LLM
    // tools (above) and threaded into `DispatchCtx` (below) so
    // timers set via voice in `serve` actually fire. See
    // `spawn_timer_driver` for behavior + the 60 s sleep-cap caveat.
    let timer_handle = spawn_timer_driver(Arc::clone(&timers), bus.clone());

    // A fired timer rings in the room it was set from until stopped.
    let _timer_alarms = ringer::spawn_timer_alarms(
        &bus,
        timers.clone(),
        satellites.clone(),
        peer_index.clone(),
        notifications.clone(),
    );
    // Half an hour before a nemlig.com delivery, the satellites say so.
    let _grocery_reminder = nemlig.clone().map(|client| {
        grocery_delivery::spawn_reminder(client, store.clone(), notifications.clone())
    });

    // Installing, updating and removing plugins, from the app.
    let plugin_installer = plugin_runtime.clone().map(|runtime| {
        niles_plugins::Installer::new(
            runtime,
            niles_plugins::GitHub::default(),
            niles_plugins::Options::default(),
        )
        .with_database(plugin_database)
        .with_reserved_tools(builtin_tool_names)
        .with_leftovers(Arc::new(plugins::Forget {
            settings: store.clone(),
            secrets: secret_store.clone(),
        }))
    });

    // HTTP API
    let chat_ctx = Arc::new(std::sync::OnceLock::new());
    let api_state = AppState::new(
        registry.clone(),
        Arc::new(publisher.clone()) as Arc<dyn DevicePublisher>,
        router.clone(),
        bus.clone(),
    )
    .with_config_store(Some(store.clone()))
    .with_logs(LOG_BUFFER.get().cloned())
    .with_manual_mode(Some(tracker.clone()))
    .with_scenes(Some(scenes.clone()))
    .with_groceries(groceries.clone())
    .with_automations(automations.clone())
    .with_skills(skill_store.clone())
    .with_memory(memory_store.clone())
    .with_automation_runs(Some(automation_runs.clone()))
    .with_calendars(calendars.clone())
    .with_strava(strava.clone())
    .with_push(push.clone())
    .with_artifacts(
        artifacts.as_ref().map(|a| a.store().clone()),
        artifacts
            .clone()
            .map(|a| Arc::new(artifacts::AppJobs(a)) as Arc<dyn niles_api::artifacts::Jobs>),
    )
    .with_nemlig(nemlig.clone())
    .with_music(Some(Arc::new(AppMusic(music.clone()))))
    .with_chat(Some(Arc::new(AppChat {
        ctx: chat_ctx.clone(),
        stt: whisper.clone(),
    })))
    .with_tado(tado.clone())
    .with_voices(voices.clone())
    .with_captures(captures.clone())
    .with_unifi(presence.as_ref().and_then(|p| p.unifi.clone()))
    .with_secrets(
        secret_store
            .clone()
            .map(|s| s as Arc<dyn niles_api::SecretStore>),
    )
    .with_plugins(plugin_installer.clone().map(|installer| {
        Arc::new(plugins::AppPlugins(installer)) as Arc<dyn niles_api::plugins::Plugins>
    }))
    .with_tool_servers(Some(
        Arc::new(tool_servers::AppToolServers(tool_servers.clone()))
            as Arc<dyn niles_api::tool_servers::ToolServers>,
    ));

    // A deployment that mounts its secrets under names the platform
    // silently drops leaves `[auth]` naming variables that do not
    // resolve. Sign-in then reads as "off, waiting for its first
    // person" — which is what a correct install looks like too, except
    // Starting no longer proves Niles is configured — every section has
    // a default, which is what lets the app be the way in. So say what
    // is still unanswered, once, where somebody will see it.
    for gap in cfg.setup_gaps() {
        // A severity added later is louder than it needs to be rather
        // than silently dropped: a new kind of gap nobody is told about
        // is the failure this whole report exists to prevent.
        if matches!(gap.severity, niles_config::Severity::Degraded) {
            tracing::info!("[setup] {} is not set. {}", gap.path, gap.consequence);
        } else {
            tracing::warn!("[setup] {} is not set. {}", gap.path, gap.consequence);
        }
    }

    // that one is open to anybody who can reach it. Worth one loud line
    // rather than a discovery.
    if cfg.auth.github_client_id_env.is_some() && !cfg.auth.is_configured() {
        tracing::warn!(
            "[auth] names secrets that cannot be read, so signing in is OFF and this              Niles is open to anyone who can reach it — check the env vars actually              reach the process"
        );
    }
    let api_handle = tokio::spawn(async move {
        if let Err(e) = niles_api::serve(api_bind, api_state).await {
            tracing::error!("API server exited: {e}");
        }
    });

    // Last, with everything they reach in place. Each compiles and starts
    // on its own thread; one that fails shows as failing and stops
    // nothing else.
    let _plugin_settings = match &plugin_runtime {
        Some(runtime) => {
            runtime.connect_notifications(notifications.clone());
            let started = runtime.load_installed().await;
            if started > 0 {
                tracing::info!("[plugins] starting {started}");
            }
            if let Some(installer) = &plugin_installer {
                installer.spawn_update_checks();
            }
            Some(plugins::restart_on_settings(runtime.clone(), store.clone()))
        }
        None => None,
    };

    // Wyoming + STT + Intent dispatch
    let wyoming_sender = server.sender();
    let server_handle = tokio::spawn(server.run());

    let mode_note = if args.dry_run { " (dry-run)" } else { "" };
    eprintln!(
        "niles serve\n  Z2M:     {prefix}/+/+\n  API:     http://{api_bind}\n  \
         WS:      ws://{api_bind}/events/stream\n  \
         Wyoming: tcp://{wyoming_bind}\n  STT:     {stt_url} ({model})\n  \
         Curve:   tick every {tick}s in {tz}{mode}\nPress Ctrl-C to exit.\n",
        prefix = cfg.mqtt.z2m_prefix,
        stt_url = stt_base_url(&cfg),
        model = cfg.stt.model,
        tick = args.tick_seconds.max(1),
        mode = mode_note,
    );

    let n_timers = timers.list().len();
    let n_scenes = scenes.names().len();
    let n_claims = claim_tracker.claimed_count();
    tracing::info!(
        target: "niles::persistence",
        "loaded {n_timers} timers, {n_scenes} scenes, {n_claims} morning claims",
    );

    let mut session_tracker = SessionTracker::new();
    let ctx = DispatchCtx {
        publisher: publisher.clone(),
        registry: registry.clone(),
        router: router.clone(),
        dry_run: args.dry_run,
        tracker: tracker.clone(),
        scenes,
        timers,
        speakers,
        music,
        tv,
        hush: Arc::default(),
        llm,
        tier2,
        identifier,
        tools,
        capability_loader,
        capability_index,
        presence: presence_agg,
        satellites: satellites.clone(),
        device_index,
        history: command_writer,
        memory: memory_store,
        skill_store,
        home: Arc::new(cfg.home.clone()),
        captures: captures.clone(),
        settings: Some(store.clone()),
        journal: journal.clone(),
        conversation: build_conversation_memory(&cfg).await,
        chat_log: chat_log.clone(),
        last_target: Arc::new(last_target::LastTarget::default()),
        weather: weather_client.clone(),
        tado: tado.clone(),
        courtesy: Arc::new(courtesy::Courtesy::default()),
        assistant: assistant.clone(),
        follow_ups: Arc::default(),
        automations: automations.clone().map(|store| (store, proposals.clone())),
        approvals: approvals.clone(),
        artifacts: artifacts.clone(),
    };
    // The API is already up; from here on the chat has something to
    // answer with.
    let _ = chat_ctx.set(ctx.clone());

    // Curve loop: driven inline with select! so we share Ctrl-C handling.
    let mut config_changes = store.subscribe();
    let mut last_published: HashMap<DeviceId, DeviceState> = HashMap::new();
    let mut ticker = tokio::time::interval(Duration::from_secs(args.tick_seconds.max(1)));

    let serve_result = loop {
        tokio::select! {
            biased;
            incoming = rx.recv() => match incoming {
                Some(incoming) => {
                    if let Some(room) = satellites.room_for(incoming.from) {
                        peer_index.lock().unwrap_or_else(|e| e.into_inner()).insert(room.clone(), incoming.from);
                    }
                    hush_on_wake(&ctx, &incoming);
                    if let Some(session) = session_tracker.feed(incoming) {
                        spawn_dispatch_task(&whisper, &ctx, &piper, &wyoming_sender, session);
                    }
                }
                None => {
                    eprintln!("\nWyoming server stopped.");
                    break Ok(());
                }
            },
            disconnect = disconnects_rx.recv() => {
                if let Some(peer) = disconnect {
                    session_tracker.drop_peer(peer);
                    let mut index = peer_index.lock().unwrap_or_else(|e| e.into_inner());
                    index.retain(|_, addr| *addr != peer);
                }
            }
            Ok(()) = config_changes.changed() => {
                // A tuning change should land on the lights now, not up
                // to a tick later. What was published before was for the
                // old target, so none of it is a reason to stay quiet.
                last_published.clear();
                lighting.refresh(&store);
                run_curve_tick(
                    &registry,
                    &publisher,
                    &router,
                    &lighting.curve,
                    lighting.snapshot.lighting.ambient_target(),
                    lighting.ambient(),
                    tz,
                    args.dry_run,
                    Duration::ZERO,
                    &mut last_published,
                    &tracker,
                    &claim_tracker,
                    Some(wake_ups.as_ref()),
                ).await;
            }
            Some(id) = curve_nudge_rx.recv() => {
                // A light just came on. Drop its last-published entry so the
                // tick can't mistake the stale target for "already applied",
                // then run a pass immediately. Drain the queue first so a
                // whole-room turn-on costs one pass, not one per light.
                last_published.remove(&id);
                while let Ok(extra) = curve_nudge_rx.try_recv() {
                    last_published.remove(&extra);
                }
                lighting.refresh(&store);
                run_curve_tick(
                    &registry,
                    &publisher,
                    &router,
                    &lighting.curve,
                    lighting.snapshot.lighting.ambient_target(),
                    lighting.ambient(),
                    tz,
                    args.dry_run,
                    Duration::ZERO,
                    &mut last_published,
                    &tracker,
                    &claim_tracker,
                    Some(wake_ups.as_ref()),
                ).await;
            }
            _ = ticker.tick() => {
                if source_handle.is_finished() {
                    break Err(anyhow::anyhow!(
                        "Z2mSource task has exited; the device registry is no longer \
                         being updated, so the curve would publish blindly. Bailing."
                    ));
                }
                lighting.refresh(&store);
                // Wake-ups ramp on, follow the curve's warmth where they
                // have none of their own, and let go of what the curve
                // has caught up with. With the curve off, nothing catches
                // up: a hold then lasts until the light is turned off.
                let curve_now = match current_minute_of_day(tz) {
                    Some((minute, now)) if lighting.curve.enabled => {
                        let at = effective_minute(
                            &lighting.curve,
                            WeekInstant::new(now.weekday(), minute),
                        );
                        Some(wake_up::CurveNow {
                            brightness: brightness_at(&lighting.curve, at),
                            kelvin: color_temp_at(&lighting.curve, at),
                        })
                    }
                    _ => None,
                };
                wake_ups.tick(Utc::now(), curve_now, lighting.transition()).await;
                if let Some(routine) = lighting.routine() {
                    run_morning_routine_tick(
                        &registry,
                        &publisher,
                        &router,
                        routine,
                        lighting.curve.morning_start,
                        lighting.curve.morning_end,
                        tz,
                        args.dry_run,
                        lighting.transition(),
                        &tracker,
                        &claim_tracker,
                        lighting.ambient(),
                        lighting.lamp_plugs(),
                        Some(wake_ups.as_ref()),
                        curve_now.map(|c| c.kelvin),
                    ).await;
                }
                run_curve_tick(
                    &registry,
                    &publisher,
                    &router,
                    &lighting.curve,
                    lighting.snapshot.lighting.ambient_target(),
                    lighting.ambient(),
                    tz,
                    args.dry_run,
                    lighting.transition(),
                    &mut last_published,
                    &tracker,
                    &claim_tracker,
                    Some(wake_ups.as_ref()),
                ).await;
            }
            _ = tokio::signal::ctrl_c() => {
                eprintln!("\nReceived Ctrl-C. Exiting.");
                break Ok(());
            }
        }
    };

    server_handle.abort();
    source_handle.abort();
    if let Some(h) = wled_handle {
        h.abort();
    }
    api_handle.abort();
    observer_handle.abort();
    timer_handle.abort();
    if let Some(h) = _presence_handle {
        h.abort();
    }
    if let Some(runtime) = &plugin_runtime {
        runtime.shutdown().await;
    }
    serve_result
}
