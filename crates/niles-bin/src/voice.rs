//! A spoken turn from the satellite: transcription, timing, and handing it to dispatch.

use crate::cli::VoiceDispatchArgs;
use crate::dispatch::{DispatchCtx, handle_transcript};
use crate::prompt::{
    SpeakerContext, build_capability_index, build_capability_loader, build_initial_device_index,
    speaker_context_from, supports_speaker_identification,
};
use crate::recognition::{build_speaker_identifier, pcm_bytes_to_i16};
use crate::satellites::SatelliteRegistry;
use crate::speech_stream::{
    ConversationLease, RoomArbiter, conversation_hold, spawn_conversation_task,
    take_conversation_after_audio,
};
use crate::timers::spawn_timer_driver;
use crate::wiring::accounts::{build_tado_tokens, build_weather_client};
use crate::wiring::llm::{build_llm_client, build_tier2_client, build_tool_registry};
use crate::wiring::memory::{
    build_conversation_memory, build_history, build_memory_store, build_skill_store,
    spawn_state_recorder,
};
use crate::wiring::mqtt::{build_command_router, connect_from_config, spawn_wled_source};
use crate::wiring::notifications::{WyomingDelivery, build_notification_center};
use crate::wiring::presence::{build_presence, build_tado, spawn_presence_poll_loop};
use crate::wiring::speech::{LivePiper, LiveStt, build_enrollment_backend, stt_base_url};
use crate::wiring::stores::build_room_music;
use crate::{
    assistant, courtesy, follow_up, last_target, music, profile, recognition, ringer, speakers, tv,
};
use anyhow::Context;
use niles_config::ConfigStore;
use niles_core::{DeviceRegistry, Event, EventBus, RoomName};
use niles_history::CommandEntry;
use niles_mqtt::Z2mSource;
use niles_scheduler::{ManualModeTracker, SceneStore, TimerStore};
use niles_stt::{Confidence, PcmFormat, SttClient, pcm_to_wav};
use niles_wyoming::{SessionTracker, WyomingSender, WyomingServer};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

/// Wrap a session's PCM in WAV and ship it to Whisper. Returns the
/// (trimmed) transcript on success, `None` on any failure — errors
/// are logged in place so a single bad request doesn't take the
/// listener down.
/// One wake, transcribed.
///
/// A struct rather than a wider tuple because the audio joined it: the
/// WAV is built for Whisper anyway, and keeping it is what lets a
/// false wake become a training sample instead of a log line.
pub(crate) struct Heard {
    pub(crate) peer: SocketAddr,
    pub(crate) text: String,
    pub(crate) confidence: Option<Confidence>,
    pub(crate) timing: TurnTiming,
    /// What was sent to Whisper, ready to write to disk unchanged.
    pub(crate) wav: Vec<u8>,
    pub(crate) sample_rate_hz: u32,
}

pub(crate) async fn transcribe_session(
    client: &SttClient,
    session: niles_wyoming::AudioSession,
) -> Option<Heard> {
    let pcm_format = PcmFormat {
        sample_rate_hz: session.format.sample_rate_hz,
        bits_per_sample: session.format.bits_per_sample,
        channels: session.format.channels,
    };
    let pcm_bytes = session.pcm.len();
    let wav = match pcm_to_wav(&session.pcm, pcm_format) {
        Ok(w) => w,
        Err(e) => {
            tracing::warn!(
                "{}: WAV wrap failed ({pcm_bytes} PCM bytes): {e}",
                session.from
            );
            return None;
        }
    };

    let audio_ms = wav_duration_ms(&session);
    let sample_rate_hz = session.format.sample_rate_hz;
    let wake_probability = session.wake_probability;
    let started = Instant::now();
    match client.transcribe(wav.clone(), "session.wav").await {
        Ok(t) => {
            let text = t.text.trim().to_string();
            // Logged for every utterance, including the ones dropped
            // below, because the thresholds worth using are the ones
            // read off a real room rather than off somebody's blog.
            // The wake probability rides along, so one line answers both
            // questions a false wake raises: was that speech, and was it
            // confident enough to be the wake word. Tuning the satellite
            // used to need the satellite on a desk. Logged whether or not
            // the provider scores itself — Scribe does so on a line of
            // its own — or a provider switch silently loses the record
            // of what was heard.
            let (no_speech_prob, avg_logprob) = t
                .confidence
                .map_or((f64::NAN, f64::NAN), |c| (c.no_speech_prob, c.avg_logprob));
            tracing::info!(
                stt = client.name(),
                no_speech_prob,
                avg_logprob,
                wake = wake_probability.unwrap_or(f32::NAN),
                "[{}] heard {text:?}",
                session.from
            );
            Some(Heard {
                peer: session.from,
                text,
                confidence: t.confidence,
                timing: TurnTiming {
                    audio_ms,
                    stt_ms: started.elapsed().as_millis(),
                    ..TurnTiming::default()
                },
                wav,
                sample_rate_hz,
            })
        }
        Err(e) => {
            tracing::warn!("{}: transcription failed: {e}", session.from);
            None
        }
    }
}

/// Where a voice turn's time went, in milliseconds.
///
/// Every number here is measured rather than assumed. The architecture
/// quotes a 400-500 ms Tier 0 budget, and until this existed nobody
/// could say which part of it was real — the satellite's own
/// end-of-speech hangover does not even appear in that figure.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct TurnTiming {
    /// How long the person spoke, for reading the rest against.
    pub(crate) audio_ms: u128,
    /// Upload, transcribe, and the round trip to wherever STT lives.
    pub(crate) stt_ms: u128,
    /// Matching the transcript and acting on it — MQTT, tools, an LLM
    /// call if Tier 0 could not answer.
    pub(crate) dispatch_ms: u128,
    /// Synthesising the reply and streaming it back to the satellite.
    pub(crate) speak_ms: u128,
}

impl TurnTiming {
    pub(crate) fn log(&self, peer: SocketAddr, text: &str) {
        tracing::info!(
            audio_ms = self.audio_ms,
            stt_ms = self.stt_ms,
            dispatch_ms = self.dispatch_ms,
            speak_ms = self.speak_ms,
            heard_to_spoken_ms = self.stt_ms + self.dispatch_ms + self.speak_ms,
            "[{peer}] turn timing for {text:?}"
        );
    }
}

/// Milliseconds of audio in a session, from the PCM length and format.
pub(crate) fn wav_duration_ms(session: &niles_wyoming::AudioSession) -> u128 {
    let bytes_per_sample = u128::from(session.format.bits_per_sample / 8).max(1);
    let channels = u128::from(session.format.channels).max(1);
    let rate = u128::from(session.format.sample_rate_hz).max(1);
    let frames = session.pcm.len() as u128 / (bytes_per_sample * channels);
    frames * 1000 / rate
}

/// Run `work` while telling the satellite at `peer` every few seconds
/// that an answer is still coming. It gives up after ten silent
/// seconds, and a slow model call — seventeen seconds, once — or a turn
/// handed to the assistant took longer than that: the answer was made and
/// spoken to nobody.
pub(crate) async fn keep_waiting<T>(
    sender: &WyomingSender,
    peer: SocketAddr,
    work: impl std::future::Future<Output = T>,
) -> T {
    const EVERY: Duration = Duration::from_secs(4);
    tokio::pin!(work);
    let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + EVERY, EVERY);
    loop {
        tokio::select! {
            out = &mut work => return out,
            _ = tick.tick() => {
                // Gone is fine: the satellite hung up or was asked
                // something else, and the reply will say so when it
                // fails.
                let _ = sender.still_thinking(peer).await;
            }
        }
    }
}

/// Spawn a fire-and-forget task that transcribes `session`, dispatches
/// the transcript, and speaks the response back to the satellite.
#[expect(
    clippy::too_many_lines,
    reason = "one voice turn from transcript to spoken reply, in a task that owns its clones"
)]
pub(crate) fn spawn_dispatch_task(
    whisper: &Arc<LiveStt>,
    ctx: &DispatchCtx,
    piper: &Arc<LivePiper>,
    sender: &WyomingSender,
    session: niles_wyoming::AudioSession,
    conversation: Option<ConversationLease>,
) {
    let whisper = whisper.clone();
    let ctx = ctx.clone();
    let piper = piper.clone();
    let sender = sender.clone();
    spawn_conversation_task(conversation, async move {
        let follow_up = session.follow_up;
        let from = session.from;
        let mut asked = false;
        let mut playing = Duration::ZERO;
        let attempted = ctx.identifier.is_some() && supports_speaker_identification(session.format);
        let id_handle = attempted.then(|| {
            let id = ctx
                .identifier
                .clone()
                .expect("attempted implies identifier present");
            let pcm = pcm_bytes_to_i16(&session.pcm);
            let rate = session.format.sample_rate_hz;
            // One print, two uses: recognising who spoke, and — if they
            // were introducing themselves — enrolling them.
            tokio::task::spawn_blocking(move || recognition::identify(id.as_ref(), &pcm, rate))
        });
        if let Some(heard) = transcribe_session(&whisper.current(), session).await {
            let Heard {
                peer,
                text,
                confidence,
                mut timing,
                wav,
                sample_rate_hz,
            } = heard;
            let (ident, voice) = match id_handle {
                Some(h) => match h.await {
                    Ok(result) => result,
                    Err(e) => {
                        tracing::warn!("[{peer}] speaker identification task failed: {e}");
                        (None, None)
                    }
                },
                None => (None, None),
            };
            if attempted {
                match &ident {
                    Some((name, score)) => {
                        tracing::info!("[{peer}] speaker: {name} (score {score:.3})");
                    }
                    None => {
                        tracing::info!("[{peer}] speaker: not recognized");
                    }
                }
            }
            // By the voice, like "who am I": the slug is the key the
            // profile is kept under, and the display name is not.
            let known = match (ctx.identifier.as_ref(), voice.as_deref()) {
                (Some(id), Some(v)) => id.whose_voice(v).map_or_else(Default::default, |slug| {
                    let (notes, birthday) = id.profile(&slug);
                    profile::Known {
                        address: id.address_as(&slug),
                        notes,
                        birthday,
                        slug: Some(slug),
                    }
                }),
                _ => profile::Known::default(),
            };
            let speaker = speaker_context_from(attempted, ident, known);
            let speaker_name = match &speaker {
                SpeakerContext::Identified { name, .. } => Some(name.clone()),
                _ => None,
            };
            // An answer to a question, heard without the wake word: taken
            // only when a question is waiting, and from the voice asked.
            let question = follow_up.then(|| ctx.follow_ups.take(peer)).flatten();
            let asker = question
                .as_ref()
                .and_then(|a| a.asker.clone().zip(a.known.clone()));
            let answering = if follow_up {
                match follow_up::judge(question, speaker_name.as_deref(), attempted) {
                    follow_up::Verdict::Answer { round } => Some(round),
                    follow_up::Verdict::Unasked => {
                        tracing::info!(
                            "[{peer}] not acting on {text:?}: an answer, and no question is waiting"
                        );
                        None
                    }
                    follow_up::Verdict::OtherVoice => {
                        tracing::info!(
                            "[{peer}] not acting on {text:?}: answered by another voice than the one asked"
                        );
                        None
                    }
                }
            } else {
                None
            };
            // An answer too short to know the voice in — "yes" — is taken
            // as the answer of whoever was asked. Otherwise the lock on
            // unknown voices threw away the yes to "shall I save it?",
            // and a calendar question's yes had nobody to belong to.
            let speaker = match (answering, speaker, asker) {
                (Some(_), SpeakerContext::Unknown, Some((name, known))) => {
                    SpeakerContext::Identified { name, known }
                }
                (_, speaker, _) => speaker,
            };
            let speaker_name = match &speaker {
                SpeakerContext::Identified { name, .. } => Some(name.clone()),
                _ => None,
            };
            let dispatch_started = Instant::now();
            let say = if follow_up && answering.is_none() {
                None
            } else {
                keep_waiting(
                    &sender,
                    peer,
                    handle_transcript(
                        &ctx,
                        peer,
                        &text,
                        confidence,
                        &speaker,
                        voice.as_deref(),
                        answering.is_some(),
                    ),
                )
                .await
            };
            timing.dispatch_ms = dispatch_started.elapsed().as_millis();
            // A question gets the microphone back after it, for one more
            // round — but only for somebody Niles knows, or when it is not
            // trying to know anybody: a question asked of an unrecognised
            // voice is as likely put to the television.
            let round = answering.map_or(1, |r| r + 1);
            let listen = say.as_deref().is_some_and(follow_up::asks)
                && round <= follow_up::MAX_ROUNDS
                && (speaker_name.is_some() || !attempted);
            if listen {
                let known = match &speaker {
                    SpeakerContext::Identified { known, .. } => Some(known.clone()),
                    _ => None,
                };
                ctx.follow_ups
                    .expect(peer, speaker_name.clone(), known, round);
            }
            asked = listen;

            // Kept before anything else looks at it, because what makes
            // this worth keeping is precisely the turns nothing else
            // wanted. `say` is the label: nothing said back means Niles
            // decided this was not addressed to it, which is the
            // negative a wake word needs to be trained on.
            if let Some(captures) = ctx.captures.clone() {
                let live = ctx.settings.as_ref().map(|s| s.current().capture);
                if let Some(cfg) = live.filter(|c| c.enabled) {
                    let outcome = if say.is_some() { "answered" } else { "dropped" };
                    let text = text.clone();
                    tokio::spawn(async move {
                        if let Err(e) = captures
                            .keep(&text, outcome, sample_rate_hz, &wav, i64::from(cfg.keep))
                            .await
                        {
                            tracing::warn!("keeping the wake audio failed: {e}");
                        }
                    });
                }
            }
            let entry = CommandEntry {
                ts: chrono::Utc::now(),
                peer,
                origin_room: ctx
                    .satellites
                    .room_for(peer)
                    .map(|r| r.as_str().to_string()),
                transcript: text.clone(),
                spoken_response: say.clone(),
                speaker: match &speaker {
                    SpeakerContext::Identified { name, .. } => Some(name.clone()),
                    _ => None,
                },
            };
            if let Some(history) = &ctx.history {
                history.record_command(entry);
            }
            if let Some(say) = say {
                tracing::info!("[{peer}] say: {say}");
                let speak_started = Instant::now();
                let spoken = crate::speak::speak_back(
                    &piper.current(),
                    &sender,
                    peer,
                    &say,
                    &ctx.speakers,
                    &ctx.satellites,
                    listen,
                )
                .await;
                match spoken {
                    Ok(played) => playing = played,
                    Err(e) => tracing::warn!("[{peer}] speak-back failed: {e:#}"),
                }
                timing.speak_ms = speak_started.elapsed().as_millis();
            }
            timing.log(peer, &text);
        }
        ctx.hush.finish(from, asked).await;
        tokio::time::sleep(conversation_hold(playing, asked && !playing.is_zero())).await;
    });
}

/// At a satellite's wake word, quiet the room's speakers for what is
/// about to be said — see [`speak::Hush`].
pub(crate) fn hush_on_wake(
    ctx: &DispatchCtx,
    arbiter: &Arc<RoomArbiter>,
    incoming: &niles_wyoming::server::IncomingEvent,
) -> Option<ConversationLease> {
    if incoming.event.kind != niles_wyoming::EventKind::AudioStart {
        return None;
    }
    let conversation = ctx
        .satellites
        .room_for(incoming.from)
        .map(|room| arbiter.enter_conversation(room.clone()));
    let ctx = ctx.clone();
    let peer = incoming.from;
    tokio::spawn(async move {
        ctx.hush.begin(&ctx.speakers, &ctx.satellites, peer).await;
    });
    conversation
}

#[expect(
    clippy::too_many_lines,
    reason = "a dev subcommand's straight-line wiring of the voice stack"
)]
pub(crate) async fn voice_dispatch(args: VoiceDispatchArgs) -> anyhow::Result<()> {
    // MQTT + config: connect_from_config validates and gives us a
    // live MqttClient. We grab a publisher handle *before* handing
    // the client to Z2mSource (which consumes it) so spawned
    // dispatch tasks can publish on the same connection.
    let (cfg, mqtt_client) = connect_from_config(&args.config).await?;
    // Read live by the speakers, which follow `[speakers]` as it is
    // asked rather than as it was at startup. Nothing writes it here.
    let (store, _) = ConfigStore::open_in_memory(&args.config)
        .await
        .context("opening the config for the speakers")?;
    let store = Arc::new(store);
    let publisher = mqtt_client.publisher();
    let router = build_command_router(&cfg);
    let bind = cfg
        .wyoming
        .socket_addr()
        .context("resolving wyoming.bind_address")?;
    let whisper = Arc::new(LiveStt::new(None, &cfg)?);
    let piper = Arc::new(LivePiper::new(None, &cfg)?);
    // Registry populated by Z2mSource. Dispatch tasks look up
    // devices in a room from this shared snapshot.
    let registry = Arc::new(DeviceRegistry::new());
    // Z2mSource requires a bus; we subscribe before spawning the
    // source so the device-index observer can't miss initial
    // DeviceAdded events while the registry warms up.
    let bus = EventBus::default();
    let mut bus_rx = bus.subscribe();
    let device_index = Arc::new(RwLock::new(build_initial_device_index(&registry)));
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

    // In-memory timer store, shared by the driver task (below) and
    // the dispatch context. The driver's behavior is documented on
    // `spawn_timer_driver` — see that helper for the 60 s sleep-cap
    // trade-off and the Pending → Ringing transition rules.
    let timers = Arc::new(TimerStore::new());
    let timer_handle = spawn_timer_driver(Arc::clone(&timers), bus.clone());

    // Build the LLM + tool registry once for the lifetime of the
    // server. Both go into DispatchCtx wrapped in Arc — they're cloned
    // (Arc::clone) into every spawned dispatch task. No Z2M warm-up
    // needed here: the first transcript arrives many seconds after
    // startup (wake-word + speech + STT round-trip), so Z2M has plenty
    // of time to populate before any tool call hits the registry.
    let llm = Arc::new(build_llm_client(&cfg)?);
    let capability_loader = build_capability_loader();
    let capability_index = capability_loader
        .as_deref()
        .map(build_capability_index)
        .map(Arc::new);

    let memory_store = build_memory_store(&cfg).await;
    let skill_store = build_skill_store(&cfg).await;
    let weather_client = build_weather_client();
    let search = niles_tools::Search::live(store.clone());
    // Kept separately from the poll loop: the connection has to exist
    // for Settings to authorise it even when nothing polls it yet.
    let tado = build_tado(&cfg.presence, build_tado_tokens(&cfg)?);
    let presence = build_presence(&cfg.presence, tado.clone(), None);
    let _presence_handle = presence.as_ref().map(|p| {
        spawn_presence_poll_loop(
            p.aggregator.clone(),
            p.sources.clone(),
            p.unifi.clone(),
            p.watching.clone(),
            cfg.presence.clone(),
            None,
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
        search,
        presence_agg.clone(),
        &cfg.home,
        Some(store.clone()),
        // No curve runs here to leave anything alone.
        None,
        args.dry_run,
    );
    niles_tools::register_timer_tools(&mut tools, timers.clone());

    let history = build_history(&cfg);
    if let Some(history) = &history {
        history.register_tools(&mut tools, registry.clone());
    }

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
    if let Some(roster) = recogniser {
        profile::register(&mut tools, roster);
    }

    let (server, mut rx, mut disconnects_rx) = WyomingServer::bind(bind)
        .await
        .with_context(|| format!("binding Wyoming server on {bind}"))?;

    let satellites = Arc::new(SatelliteRegistry::from_config(&cfg.satellites));
    let speech_arbiter = Arc::new(RoomArbiter::default());
    let speakers = Arc::new(speakers::SpeakerRegistry::new(store.clone()));
    let music =
        Arc::new(music::Music::new(speakers.clone(), store.clone(), build_room_music(&cfg)).await);
    let tv = Arc::new(tv::TvControl::new(store.clone()));
    music::register(&mut tools, music.clone(), tv.clone());
    tv::register(&mut tools, tv.clone());

    // Notification center
    let mut notifications = build_notification_center(&cfg).await;
    let peer_index: Arc<Mutex<HashMap<RoomName, SocketAddr>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let wyoming_delivery = Arc::new(WyomingDelivery {
        piper: piper.clone(),
        sender: server.sender(),
        speakers: speakers.clone(),
        satellites: satellites.clone(),
        peer_index: peer_index.clone(),
        tv: tv.clone(),
        speech_arbiter: speech_arbiter.clone(),
    });
    notifications.set_delivery(wyoming_delivery);
    let notifications = Arc::new(notifications);
    niles_tools::register_notification_tools(&mut tools, notifications.clone());

    // A fired timer rings in the room it was set from until stopped.
    let _timer_alarms = ringer::spawn_timer_alarms(
        &bus,
        timers.clone(),
        satellites.clone(),
        peer_index.clone(),
        notifications.clone(),
        speech_arbiter.clone(),
    );

    let tools = Arc::new(tools);
    // No app here, so no chat for an assistant to answer.
    let assistant: Option<Arc<assistant::Assistant>> = None;

    let mode_note = if args.dry_run {
        " (dry-run: nothing will be published)"
    } else {
        ""
    };
    eprintln!(
        "Wyoming server listening on tcp://{bind}\nZ2M source running on {prefix}/+/+; STT via {stt_url} ({model}).\nMatched intents trigger MQTT publish{mode}. Press Ctrl-C to exit.\n",
        prefix = cfg.mqtt.z2m_prefix,
        stt_url = stt_base_url(&cfg),
        model = cfg.stt.model,
        mode = mode_note,
    );

    let wyoming_sender = server.sender();
    let server_handle = tokio::spawn(server.run());
    let mut tracker = SessionTracker::new();
    let speech_arbiter = Arc::new(RoomArbiter::default());
    let mut conversations: HashMap<SocketAddr, ConversationLease> = HashMap::new();
    let ctx = DispatchCtx {
        publisher,
        registry: registry.clone(),
        router,
        dry_run: args.dry_run,
        tracker: Arc::new(ManualModeTracker::new()),
        scenes: Arc::new(SceneStore::new()),
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
        device_index: Arc::clone(&device_index),
        history: history.map(Arc::new),
        memory: memory_store,
        skill_store,
        home: Arc::new(cfg.home.clone()),
        captures: None,
        // The same store the tool registry above resolves "open"
        // against: a voice Open here must not answer a different
        // level than a set_blinds call in this same process.
        settings: Some(store.clone()),
        journal: Arc::default(),
        conversation: build_conversation_memory(&cfg).await,
        chat_log: None,
        last_target: Arc::new(last_target::LastTarget::default()),
        weather: weather_client.clone(),
        tado: tado.clone(),
        courtesy: Arc::new(courtesy::Courtesy::default()),
        assistant: assistant.clone(),
        follow_ups: Arc::default(),
        automations: None,
        approvals: Arc::default(),
        artifacts: None,
    };

    // Keep the device index in sync so Tier-0 device-name matchers
    // work in voice-dispatch mode too (same pattern as `serve`).
    let observer_device_index = Arc::clone(&ctx.device_index);
    let observer_recorder = ctx
        .history
        .as_ref()
        .map(|h| spawn_state_recorder(h.states.clone()));
    let observer_registry = registry.clone();
    let _device_index_handle = tokio::spawn(async move {
        loop {
            match bus_rx.recv().await {
                Ok(Event::DeviceAdded { device }) if device.is_light() => {
                    observer_device_index
                        .write()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(device.id.clone());
                }
                Ok(Event::DeviceRemoved { id }) => {
                    observer_device_index
                        .write()
                        .unwrap_or_else(|e| e.into_inner())
                        .remove(&id);
                }
                Ok(Event::DeviceStateChanged { id, .. }) => {
                    if let (Some(recorder), Some(device)) =
                        (&observer_recorder, observer_registry.get(&id))
                    {
                        let _ = recorder.send(niles_history::StateEntry {
                            ts: chrono::Utc::now(),
                            device_id: id.clone(),
                            state: device.state,
                        });
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                _ => {}
            }
        }
    });

    loop {
        tokio::select! {
            // See voice-tap: drain events before disconnects so a
            // trailing `audio-stop` isn't dropped by the race.
            biased;
            incoming = rx.recv() => match incoming {
                Some(incoming) => {
                    let audio_stop = incoming.event.kind == niles_wyoming::EventKind::AudioStop;
                    let peer = incoming.from;
                    if let Some(room) = satellites.room_for(incoming.from) {
                        peer_index.lock().unwrap_or_else(|e| e.into_inner()).insert(room.clone(), incoming.from);
                    }
                    if let Some(conversation) = hush_on_wake(&ctx, &speech_arbiter, &incoming) {
                        conversations.insert(incoming.from, conversation);
                    }
                    let session = tracker.feed(incoming);
                    let conversation = take_conversation_after_audio(
                        &mut conversations,
                        peer,
                        audio_stop,
                        session.is_some(),
                    );
                    if let Some(session) = session {
                        // Same unbounded fan-out as voice-tap — fine
                        // for a dev tool, replaced by a bounded
                        // worker pool when this becomes prod dispatch.
                        spawn_dispatch_task(&whisper, &ctx, &piper, &wyoming_sender, session, conversation);
                    }
                }
                None => {
                    eprintln!("\nWyoming server stopped.");
                    break;
                }
            },
            disconnect = disconnects_rx.recv() => {
                if let Some(peer) = disconnect {
                    tracker.drop_peer(peer);
                    conversations.remove(&peer);
                    let mut index = peer_index.lock().unwrap_or_else(|e| e.into_inner());
                    index.retain(|_, addr| *addr != peer);
                }
            }
            _ = tokio::signal::ctrl_c() => {
                eprintln!("\nReceived Ctrl-C. Exiting.");
                break;
            }
        }
    }

    server_handle.abort();
    source_handle.abort();
    if let Some(h) = wled_handle {
        h.abort();
    }
    timer_handle.abort();
    if let Some(h) = _presence_handle {
        h.abort();
    }
    Ok(())
}
