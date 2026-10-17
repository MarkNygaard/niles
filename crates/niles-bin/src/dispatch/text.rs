//! Acting on a recognised intent.

use crate::dispatch::hearing::{addressed, is_overheard, without_vocative};
use crate::dispatch::media::{
    Reach, media_dispatch, media_result_to_response, music_reply, music_room, room_volume_set,
    room_volume_step, stopped_music, tv_volume,
};
use crate::dispatch::targets::{
    RoomResolve, dispatch_to_targets, intent_room_to_canonical, publish_single,
    resolve_room_targets,
};
use crate::dispatch::{
    DispatchCtx, answer_to_approval, answer_to_proposal, dispatch_tier1, enroll_by_voice,
    format_intent,
};
use crate::prompt::SpeakerContext;
use crate::timers::{timer_label, timer_label_for};
use crate::wiring::automations::command_directly;
use crate::{climate, conversation, music, response};
use chrono::Utc;
use niles_config::FULLY_OPEN;
use niles_core::{BlindsLevel, Device, DeviceState};
use niles_intent::{Intent, IntentRouter, RouterContext};
use niles_webos::VolumeChange;
use std::net::SocketAddr;
use std::time::Instant;

/// Act on words somebody meant for Niles, however they arrived.
///
/// Everything before this decides whether a sound in a room was a
/// command — the noise gate, the name, the known-voices lock. Typed
/// words from the app have already answered all of that by being typed
/// by somebody signed in, so they start here.
pub(crate) async fn dispatch_text(
    ctx: &DispatchCtx,
    peer: SocketAddr,
    text: &str,
    thread: &conversation::Thread,
    speaker: &SpeakerContext,
    voice: Option<&[f32]>,
    // Known to be meant for Niles — typed in the app, or an answer to its
    // question — so a lone word is a reply, not something overheard.
    meant: bool,
) -> Option<String> {
    let origin_room = ctx.satellites.room_for(peer);

    if let Some(reply) = answer_to_approval(ctx, peer, text, speaker).await {
        return Some(reply);
    }
    if let Some(reply) = answer_to_proposal(ctx, peer, text) {
        return Some(reply);
    }

    // IntentRouter is a zero-sized unit struct; the regexes are
    // compiled once into a static OnceLock, so constructing one
    // per call is free.
    let parsed = {
        let idx = ctx.device_index.read().unwrap_or_else(|e| e.into_inner());
        // Read per utterance rather than cached: a scene saved a
        // sentence ago has to be callable by name in the next one.
        let scene_names = ctx.scenes.names();
        let router_ctx = RouterContext {
            device_index: &idx,
            origin_room,
            scenes: &scene_names,
        };
        let router = IntentRouter::new();
        router.parse_with_context(text, router_ctx).or_else(|| {
            without_vocative(text).and_then(|rest| router.parse_with_context(rest, router_ctx))
        })
    };

    let intent = match parsed {
        Some(i) => i,
        // Tier 0 miss — hand the raw transcript to the LLM, unless it
        // is too thin to be an instruction.
        None if !meant && is_overheard(text) => {
            tracing::info!("[{peer}] not acting on {text:?}: too little to be an instruction");
            return Some(response::didnt_catch_that());
        }
        None => return dispatch_tier1(ctx, peer, text, origin_room, thread, speaker).await,
    };

    tracing::info!("[{peer}] \"{text}\" -> {}", format_intent(&intent));
    // "Lights off, Sir." — but not on the replies that are about who
    // somebody is, which have to be the name.
    let address = match speaker {
        SpeakerContext::Identified { name, known } => known
            .address
            .as_ref()
            .map(|address| (name.clone(), address.clone())),
        _ => None,
    }
    .filter(|_| !matches!(intent, Intent::WhoAmI | Intent::EnrollSpeaker { .. }));
    let reply = match intent {
        Intent::WeatherQuery { day, rain } => Some(climate::weather(ctx, day, rain).await),
        heating @ (Intent::HeatingQuery { .. }
        | Intent::HeatingSet { .. }
        | Intent::HeatingStep { .. }
        | Intent::HeatingOff { .. }
        | Intent::HeatingResume { .. }) => Some(climate::heating(ctx, peer, &heating).await),
        Intent::LightSet { room, on } => {
            // Filter on device class, not observed state: a light can be
            // switched on/off whether or not niles has seen its state yet.
            // Filtering on `state.on.is_some()` made a freshly-started
            // `serve` report "no devices support this action" until Z2M
            // retained state arrived.
            //
            // Lamps rather than lights, which is what a wall switch in
            // the same room already does: a lamp on a smart plug is a
            // lamp, and leaving it lit is not what "turn off the lights
            // in here" meant.
            let lamps = ctx.lamp_plugs();
            let (_canonical, targets) =
                match resolve_room_targets(ctx, peer, &room, |d| d.is_lamp(&lamps)) {
                    RoomResolve::Found(c, t) => (c, t),
                    RoomResolve::BadName | RoomResolve::NoDevices => {
                        // Tier 0 matched a light intent but found no target
                        // (e.g. "living room ceiling light" parsed as room
                        // "living room ceiling"). Let the LLM resolve it
                        return dispatch_tier1(ctx, peer, text, origin_room, thread, speaker).await;
                    }
                    RoomResolve::WarmingUp => return Some(response::room_warming_up()),
                };
            let desired = DeviceState {
                on: Some(on),
                ..Default::default()
            };
            ctx.last_target
                .remember(origin_room, &response::spoken_room(&room), &targets);
            dispatch_to_targets(ctx, peer, &targets, &desired).await;
            Some(response::light_set(&room, on))
        }
        Intent::BlindsSet {
            room,
            everywhere,
            level,
        } => {
            let targets: Vec<Device> = match (&room, origin_room) {
                _ if everywhere => ctx
                    .registry
                    .list_all()
                    .into_iter()
                    .filter(|d| d.is_cover())
                    .collect(),
                (Some(room), _) => match resolve_room_targets(ctx, peer, room, |d| d.is_cover()) {
                    RoomResolve::Found(_, targets) => targets,
                    RoomResolve::WarmingUp => return Some(response::room_warming_up()),
                    // "the living room left blind" reads as a room called
                    // "living room left": one blind, which the LLM finds.
                    RoomResolve::BadName | RoomResolve::NoDevices => {
                        return dispatch_tier1(ctx, peer, text, origin_room, thread, speaker).await;
                    }
                },
                (None, Some(here)) => ctx
                    .registry
                    .list_room(here)
                    .into_iter()
                    .filter(|d| d.is_cover())
                    .collect(),
                (None, None) => Vec::new(),
            };
            if targets.is_empty() {
                return dispatch_tier1(ctx, peer, text, origin_room, thread, speaker).await;
            }
            // "open" is not a position: resolved per blind against the
            // live config — a second press, every blind already at its
            // normal level, goes all the way.
            let snapshot = ctx.settings.as_ref().map(|s| s.current());
            let fallback = niles_config::BlindsConfig::default();
            let cfg = snapshot.as_deref().map(|c| &c.blinds).unwrap_or(&fallback);
            let resolved = cfg.targets(level, targets.iter().map(|d| (&d.id, d.state.position)));
            for (device, position) in targets.iter().zip(&resolved.positions) {
                let desired = DeviceState {
                    position: Some(*position),
                    ..Default::default()
                };
                dispatch_to_targets(ctx, peer, std::slice::from_ref(device), &desired).await;
            }
            let said = match level {
                BlindsLevel::Exact(0) => response::BlindsSaid::Closing,
                BlindsLevel::Exact(FULLY_OPEN) => response::BlindsSaid::Fully,
                BlindsLevel::Exact(p) => response::BlindsSaid::Level(p),
                BlindsLevel::Open if resolved.all_the_way => response::BlindsSaid::Fully,
                BlindsLevel::Open => response::BlindsSaid::Opening,
            };
            Some(response::blinds(
                room.as_deref(),
                everywhere,
                targets.len(),
                said,
            ))
        }
        Intent::LightSetLast { on } => {
            let Some((spoken, targets)) = ctx.last_target.resolve(origin_room) else {
                // Nothing recent to point at. The LLM has the
                // conversation history and may do better than we can.
                return dispatch_tier1(ctx, peer, text, origin_room, thread, speaker).await;
            };
            let desired = DeviceState {
                on: Some(on),
                ..Default::default()
            };
            dispatch_to_targets(ctx, peer, &targets, &desired).await;
            Some(response::light_set_last(&spoken, on))
        }
        Intent::DateTimeQuery { date } => Some(response::datetime_now(&ctx.home.timezone, date)),
        Intent::EnrollSpeaker { name } => Some(enroll_by_voice(ctx, peer, &name, voice).await),
        Intent::WhoAmI => {
            // Answered from what the turn already knows. The voice was
            // matched in parallel with transcription, so this costs a
            // string rather than a round trip — and it still answers
            // when the language model is unreachable, which is when
            // somebody is most likely to be asking.
            // Resolved from the voice rather than from `speaker`,
            // which carries the *display* name for the LLM to read.
            // This answer is spoken, and the two are different strings
            // whenever somebody has had to respell a name to make Piper
            // say it.
            let spoken = match (ctx.identifier.as_ref(), voice) {
                (Some(id), Some(voice)) => {
                    id.whose_voice(voice).and_then(|slug| id.how_to_say(&slug))
                }
                _ => match speaker {
                    SpeakerContext::Identified { name, .. } => Some(name.clone()),
                    _ => None,
                },
            };
            Some(response::who_you_are(
                spoken.as_deref(),
                ctx.identifier.is_some(),
                ctx.identifier.as_ref().is_some_and(|i| i.knows_anybody()),
            ))
        }
        Intent::LightSetAll { on } => {
            // Lamps, not lights: "turn everything off" includes the
            // lamp that happens to be on a plug, and not the fridge.
            let lamps = ctx.lamp_plugs();
            let targets: Vec<Device> = ctx
                .registry
                .list_all()
                .into_iter()
                .filter(|d| d.is_lamp(&lamps))
                .collect();
            if targets.is_empty() {
                if ctx.registry.is_empty() {
                    println!(
                        "[{peer}] registry is still warming up (no devices yet) — try again in a moment"
                    );
                    return Some(response::room_warming_up());
                }
                tracing::info!("[{peer}] no lights in registry — nothing to dispatch");
                return Some(response::no_lights());
            }
            let desired = DeviceState {
                on: Some(on),
                ..Default::default()
            };
            dispatch_to_targets(ctx, peer, &targets, &desired).await;
            Some(response::all_lights(on))
        }
        Intent::LightDim { room, percent } => {
            let (_canonical, targets) = match resolve_room_targets(ctx, peer, &room, |d| {
                d.room_light && d.state.brightness.is_some()
            }) {
                RoomResolve::Found(c, t) => (c, t),
                RoomResolve::BadName | RoomResolve::NoDevices => {
                    // Tier 0 matched a light intent but found no target
                    // (e.g. "living room ceiling light" parsed as room
                    // "living room ceiling"). Let the LLM resolve it
                    // rather than dead-ending. See dispatch_tier1.
                    return dispatch_tier1(ctx, peer, text, origin_room, thread, speaker).await;
                }
                RoomResolve::WarmingUp => return Some(response::room_warming_up()),
            };
            // Flag each dimmable target *before* publishing so the
            // curve driver can't race a tick in between and overwrite
            // the dim we're about to send.
            for device in &targets {
                ctx.tracker.flag(&device.id);
            }
            let desired = DeviceState {
                brightness: Some(percent),
                ..Default::default()
            };
            dispatch_to_targets(ctx, peer, &targets, &desired).await;
            Some(response::light_dim(&room, percent))
        }
        Intent::LightStep {
            room,
            delta_percent,
        } => {
            let (_canonical, targets) = match resolve_room_targets(ctx, peer, &room, |d| {
                d.room_light && d.state.brightness.is_some()
            }) {
                RoomResolve::Found(c, t) => (c, t),
                RoomResolve::BadName | RoomResolve::NoDevices => {
                    // Tier 0 matched a light intent but found no target
                    // (e.g. "living room ceiling light" parsed as room
                    // "living room ceiling"). Let the LLM resolve it
                    // rather than dead-ending. See dispatch_tier1.
                    return dispatch_tier1(ctx, peer, text, origin_room, thread, speaker).await;
                }
                RoomResolve::WarmingUp => return Some(response::room_warming_up()),
            };
            // `resolve_room_targets` already filtered for devices with
            // brightness, so every target has a known value.
            let base = (targets
                .iter()
                .map(|d| d.state.brightness.unwrap() as i32)
                .sum::<i32>()
                / targets.len() as i32) as i16;
            let new = (base + delta_percent).clamp(0, 100) as u8;
            for device in &targets {
                ctx.tracker.flag(&device.id);
            }
            let desired = DeviceState {
                brightness: Some(new),
                ..Default::default()
            };
            dispatch_to_targets(ctx, peer, &targets, &desired).await;
            Some(response::light_dim(&room, new))
        }
        Intent::LightKelvinStep { room, delta_kelvin } => {
            let (_canonical, targets) = match resolve_room_targets(ctx, peer, &room, |d| {
                d.is_light() && d.room_light && d.supports_color_temperature()
            }) {
                RoomResolve::Found(c, t) => (c, t),
                RoomResolve::BadName | RoomResolve::NoDevices => {
                    return dispatch_tier1(ctx, peer, text, origin_room, thread, speaker).await;
                }
                RoomResolve::WarmingUp => return Some(response::room_warming_up()),
            };
            // `resolve_room_targets` already filtered for devices with
            // color temperature, so every target has a known value.
            let base = (targets
                .iter()
                .map(|d| d.state.color_temp_kelvin.unwrap() as u32)
                .sum::<u32>()
                / targets.len() as u32) as u16;
            let new = base.saturating_add_signed(delta_kelvin).clamp(2000, 6500);
            for device in &targets {
                ctx.tracker.flag(&device.id);
            }
            let desired = DeviceState {
                color_temp_kelvin: Some(new),
                ..Default::default()
            };
            dispatch_to_targets(ctx, peer, &targets, &desired).await;
            Some(response::light_kelvin_step(&room, new))
        }
        Intent::LightKelvinSet { room, kelvin } => {
            let (_canonical, targets) = match resolve_room_targets(ctx, peer, &room, |d| {
                d.is_light() && d.room_light && d.supports_color_temperature()
            }) {
                RoomResolve::Found(c, t) => (c, t),
                RoomResolve::BadName | RoomResolve::NoDevices => {
                    // No Tier 0 target — escalate to the LLM (see dispatch_tier1).
                    return dispatch_tier1(ctx, peer, text, origin_room, thread, speaker).await;
                }
                RoomResolve::WarmingUp => return Some(response::room_warming_up()),
            };
            let target_kelvin = kelvin.clamp(2000, 6500);
            for device in &targets {
                ctx.tracker.flag(&device.id);
            }
            let desired = DeviceState {
                color_temp_kelvin: Some(target_kelvin),
                ..Default::default()
            };
            dispatch_to_targets(ctx, peer, &targets, &desired).await;
            Some(response::light_kelvin_set(&room, target_kelvin))
        }
        Intent::DeviceSet { device_id, on } => {
            let Some(device) = ctx.registry.get(&device_id) else {
                return Some(response::device_not_found(&device_id));
            };
            let desired = DeviceState {
                on: Some(on),
                ..Default::default()
            };
            publish_single(ctx, peer, &device, &desired).await;
            ctx.tracker.flag(&device.id);
            Some(response::device_set(&device_id, on))
        }
        Intent::DeviceDim { device_id, percent } => {
            let Some(device) = ctx.registry.get(&device_id) else {
                return Some(response::device_not_found(&device_id));
            };
            // Flag before publishing so the curve driver can't race
            // a tick in between and overwrite the dim we're about to send.
            ctx.tracker.flag(&device.id);
            let desired = DeviceState {
                brightness: Some(percent),
                ..Default::default()
            };
            publish_single(ctx, peer, &device, &desired).await;
            Some(response::device_dim(&device_id, percent))
        }
        Intent::SceneSave { name, room } => {
            let canonical = match room.as_deref().map(intent_room_to_canonical) {
                Some(Ok(r)) => Some(r),
                Some(Err(reason)) => {
                    let room_name = room.as_deref().unwrap_or("");
                    tracing::warn!(
                        "[{peer}] room {room:?} is not a valid registry name: {reason}",
                        room = room_name,
                    );
                    return Some(response::room_not_found(room_name));
                }
                None => None,
            };
            let n = ctx
                .scenes
                .save(&name, &ctx.registry, canonical.as_ref(), &ctx.lamp_plugs());
            match &canonical {
                Some(r) => tracing::info!("[{peer}] saved scene {name:?} with {n} devices in {r}"),
                None => {
                    tracing::info!("[{peer}] saved scene {name:?} with {n} devices (whole home)")
                }
            }
            Some(response::scene_saved(&name))
        }
        Intent::SceneApply { name } => {
            let Some(entries) = ctx.scenes.get(&name) else {
                tracing::info!("[{peer}] no scene named {name:?}");
                return Some(response::scene_not_found(&name));
            };
            if entries.is_empty() {
                tracing::info!("[{peer}] scene {name:?} is empty — nothing to apply");
                return Some(response::scene_empty(&name));
            }
            for entry in entries {
                if let Some(commander) = ctx.router.commander_for(&entry.device_id, &entry.state) {
                    let what = format!("[{peer}]");
                    command_directly(
                        commander,
                        &entry.device_id,
                        &entry.state,
                        ctx.dry_run,
                        &what,
                    )
                    .await;
                    ctx.tracker.flag(&entry.device_id);
                    continue;
                }
                let Some((topic, payload)) = ctx.router.format(&entry.device_id, &entry.state)
                else {
                    continue;
                };
                if ctx.dry_run {
                    tracing::info!("[{peer}] [dry-run] {topic}  {payload}");
                } else {
                    match ctx.publisher.publish(&topic, payload.clone()).await {
                        Ok(()) => tracing::info!("[{peer}] published {topic}  {payload}"),
                        Err(e) => tracing::warn!("[{peer}] publish to {topic} failed: {e}"),
                    }
                }
                // Scene-applied lights enter manual mode until the
                // user explicitly clears them.
                ctx.tracker.flag(&entry.device_id);
            }
            tracing::info!("[{peer}] applied scene {name:?}");
            Some(response::scene_applied(&name))
        }
        Intent::SceneList => {
            let names = ctx.scenes.names();
            if names.is_empty() {
                tracing::info!("[{peer}] no scenes saved yet");
            } else {
                tracing::info!("[{peer}] {} scenes: {}", names.len(), names.join(", "));
            }
            Some(response::scene_list(&names))
        }
        Intent::SceneDelete { name } => {
            if ctx.scenes.delete(&name) {
                tracing::info!("[{peer}] deleted scene {name:?}");
                Some(response::scene_deleted(&name))
            } else {
                tracing::info!("[{peer}] no scene named {name:?}");
                Some(response::scene_not_found(&name))
            }
        }
        Intent::ClearManualMode { room } => match room {
            None => {
                let n = ctx.tracker.clear_all();
                tracing::info!("[{peer}] back to normal -> cleared manual flag on {n} devices");
                Some(response::cleared_manual(None))
            }
            Some(name) => {
                let canonical = match intent_room_to_canonical(&name) {
                    Ok(r) => r,
                    Err(reason) => {
                        tracing::warn!(
                            "[{peer}] room {name:?} is not a valid registry name: {reason}"
                        );
                        return Some(response::room_not_found(&name));
                    }
                };
                let n = ctx.tracker.clear_room(&canonical);
                println!(
                    "[{peer}] back to normal in {name} -> cleared manual flag on {n} devices in {canonical}"
                );
                Some(response::cleared_manual(Some(&name)))
            }
        },
        Intent::MediaPause { room } => {
            let canonical = match intent_room_to_canonical(&room) {
                Ok(r) => r,
                Err(reason) => {
                    tracing::warn!("[{peer}] room {room:?} is not a valid registry name: {reason}");
                    return Some(response::room_not_found(&room));
                }
            };
            Some(match ctx.music.pause(&canonical).await {
                music::Outcome::Paused => response::media_pause(&room),
                music::Outcome::NoSpeaker => response::no_speaker_in_room(canonical.as_str()),
                other => {
                    tracing::warn!("[{peer}] pausing {canonical}: {other:?}");
                    response::speaker_unreachable(canonical.as_str())
                }
            })
        }
        // "Play the kitchen" resumes it; "play P4" and "play John Mayer"
        // are things to find. Only a room Niles knows tells them apart.
        Intent::MediaPlay { room } => {
            let known = match intent_room_to_canonical(&room) {
                Ok(r) => {
                    !ctx.speakers.in_room(&r).await.is_empty()
                        || !ctx.registry.list_room(&r).is_empty()
                }
                Err(_) => false,
            };
            if known {
                media_result_to_response(
                    peer,
                    &room,
                    media_dispatch(
                        ctx,
                        &room,
                        Reach::Leaders,
                        |c| async move { c.play().await },
                    )
                    .await,
                    response::media_play(&room),
                    "play",
                )
            } else {
                let target = match music_room(None, origin_room) {
                    Ok(r) => r,
                    Err(say) => return Some(say),
                };
                match music_reply(ctx.music.play(&target, &room, false).await, &target) {
                    Some(say) => Some(say),
                    None => {
                        return dispatch_tier1(ctx, peer, text, origin_room, thread, speaker).await;
                    }
                }
            }
        }
        Intent::TvPower { on } => Some(ctx.tv.power(on).await.unwrap_or_else(|e| e)),
        Intent::TvOpen { app } => Some(ctx.tv.open(&app).await.unwrap_or_else(|e| e)),
        Intent::TvInput { input } => Some(ctx.tv.input(&input).await.unwrap_or_else(|e| e)),
        Intent::TvPlayback { play } => Some(ctx.tv.playback(play).await.unwrap_or_else(|e| e)),
        Intent::TvVolumeSet { percent } => {
            Some(tv_volume(ctx, peer, VolumeChange::To(percent)).await)
        }
        Intent::TvVolumeStep { delta } => Some(tv_volume(ctx, peer, VolumeChange::By(delta)).await),
        // A Sonos has no mute to give, so muting the TV is always the TV:
        // one behind a soundbar passes it on over HDMI.
        Intent::TvMute { mute } => Some(
            ctx.tv
                .volume(VolumeChange::Mute(mute))
                .await
                .unwrap_or_else(|e| e),
        ),
        Intent::PlayRadio { station, room } => {
            let target = match music_room(room.as_deref(), origin_room) {
                Ok(r) => r,
                Err(say) => return Some(say),
            };
            match music_reply(ctx.music.radio(&target, station.as_deref()).await, &target) {
                Some(say) => Some(say),
                None => return dispatch_tier1(ctx, peer, text, origin_room, thread, speaker).await,
            }
        }
        Intent::PlayMusic {
            query,
            room,
            spotify,
        } => {
            let target = match music_room(room.as_deref(), origin_room) {
                Ok(r) => r,
                Err(say) => return Some(say),
            };
            match music_reply(ctx.music.play(&target, &query, spotify).await, &target) {
                Some(say) => Some(say),
                None => return dispatch_tier1(ctx, peer, text, origin_room, thread, speaker).await,
            }
        }
        Intent::PlayElsewhere { room } => {
            let to = match room.as_deref().map(intent_room_to_canonical) {
                Some(Ok(r)) => Some(r),
                Some(Err(_)) => {
                    return Some(response::room_not_found(
                        room.as_deref().unwrap_or_default(),
                    ));
                }
                None => None,
            };
            Some(match ctx.music.spread(origin_room, to.as_ref()).await {
                music::Outcome::Spread => match &to {
                    Some(r) => response::playing_too(r.as_str()),
                    None => response::playing_everywhere(),
                },
                music::Outcome::NothingPlaying => response::nothing_to_share(),
                music::Outcome::TvStaysPut => response::tv_stays_put(),
                music::Outcome::NoSpeaker => {
                    response::no_speaker_in_room(to.as_ref().map_or("house", |r| r.as_str()))
                }
                other => {
                    tracing::warn!("[{peer}] spreading the music: {other:?}");
                    response::music_failed(to.as_ref().map_or("house", |r| r.as_str()))
                }
            })
        }
        Intent::MediaNext { room } => media_result_to_response(
            peer,
            &room,
            media_dispatch(
                ctx,
                &room,
                Reach::Leaders,
                |c| async move { c.next().await },
            )
            .await,
            response::media_next(&room),
            "next",
        ),
        Intent::MediaPrevious { room } => media_result_to_response(
            peer,
            &room,
            media_dispatch(
                ctx,
                &room,
                Reach::Leaders,
                |c| async move { c.previous().await },
            )
            .await,
            response::media_previous(&room),
            "previous",
        ),
        Intent::MediaVolumeSet { room, percent } => {
            Some(room_volume_set(ctx, peer, &room, percent).await)
        }
        Intent::MediaVolumeStep { room, delta } => {
            Some(room_volume_step(ctx, peer, &room, delta).await)
        }
        Intent::TimerSet { duration, name } => {
            let id = ctx.timers.set(duration, name.clone(), peer, Utc::now());
            let label = timer_label_for(name.as_deref(), duration);
            tracing::info!("[{peer}] {label} started (id={})", id.0);
            Some(response::timer_started(duration, name.as_deref()))
        }
        Intent::StopMusic => Some(stopped_music(ctx, peer, origin_room).await),
        Intent::Stop | Intent::Cancel => {
            let stop = matches!(intent, Intent::Stop);
            // Prefer stopping a ringing alarm; otherwise cancel the soonest
            // counting-down timer, so "stop the timer" works before it fires
            // (previously this only stopped a *ringing* timer).
            if let Some(entry) = ctx.timers.stop_most_recent_ringing() {
                tracing::info!("[{peer}] stopped {}", timer_label(&entry));
                Some(response::timer_stopped(
                    entry.name.as_deref(),
                    entry.duration,
                ))
            } else if let Some(entry) = ctx.timers.cancel_soonest_pending() {
                tracing::info!("[{peer}] cancelled pending {}", timer_label(&entry));
                Some(response::stop_outcome(
                    response::StopOutcome::CancelledPending,
                ))
            } else if stop {
                // No timer to stop: "stop" means the music, as it would
                // to anybody standing in the room with it.
                Some(stopped_music(ctx, peer, origin_room).await)
            } else {
                tracing::info!("[{peer}] nothing to stop");
                Some(response::stop_outcome(response::StopOutcome::Nothing))
            }
        }
        Intent::TimerCancel { name } => {
            let n = ctx.timers.cancel_by_name(&name);
            if n == 0 {
                tracing::info!("[{peer}] no timer named {name:?}");
            } else {
                tracing::info!("[{peer}] cancelled {n} timer(s) named {name:?}");
            }
            Some(response::timer_cancelled(&name, n))
        }
        Intent::TimerList => {
            let entries = ctx.timers.list();
            if entries.is_empty() {
                tracing::info!("[{peer}] no active timers");
            } else {
                tracing::info!("[{peer}] {} active timer(s):", entries.len());
                for e in &entries {
                    println!(
                        "  - id={} {} (state={:?}, expires_at={})",
                        e.id.0,
                        timer_label(e),
                        e.state,
                        e.expires_at,
                    );
                }
            }
            Some(response::timer_list(entries.len()))
        }
        Intent::TimerRemaining => {
            let now = Utc::now();
            let entries = ctx.timers.list(); // sorted soonest-first
            let seconds_left = if let Some(e) = entries.iter().find(|e| e.is_pending()) {
                let secs = (e.expires_at - now).num_seconds().max(0) as u64;
                tracing::info!("[{peer}] {} has {secs}s left", timer_label(e));
                Some(secs)
            } else if entries.iter().any(|e| e.is_ringing()) {
                tracing::info!("[{peer}] timer is ringing (0s left)");
                Some(0)
            } else {
                tracing::info!("[{peer}] no timer for remaining query");
                None
            };
            Some(response::timer_remaining(seconds_left))
        }
        _ => {
            tracing::info!("{peer}: unknown intent variant, skipping dispatch");
            Some(response::fallback())
        }
    };
    // Once in a while rather than every time: see `courtesy`.
    reply.map(|r| match &address {
        Some((name, a)) if ctx.courtesy.address_now(name, Instant::now()) => addressed(&r, Some(a)),
        _ => r,
    })
}
