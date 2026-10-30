//! The one-shot subcommands: each connects what it needs, does one thing and exits.

use crate::chat_loop::{LoopOutcome, MAX_TOOL_ITERATIONS, run_tool_calling_chat};
use crate::cli::{
    ApiArgs, ChatArgs, ConfigValidateArgs, DiscoverArgs, EnrollArgs, MqttTapArgs, SetArgs,
    SynthesizeArgs, TranscribeArgs, VoiceTapArgs, WyomingTapArgs,
};
use crate::prompt::{
    SpeakerContext, assemble_system_prompt_with_optional_capabilities, build_capability_index,
    build_capability_loader, join_memory_entries,
};
use crate::voice::transcribe_session;
use crate::wiring::accounts::{build_tado_tokens, build_weather_client};
use crate::wiring::llm::{build_llm_client, build_tool_registry};
use crate::wiring::memory::{build_memory_store, build_skill_store};
use crate::wiring::mqtt::{build_command_router, connect_from_config, spawn_wled_source};
use crate::wiring::presence::{build_presence, build_tado, spawn_presence_poll_loop};
use crate::wiring::speech::{
    build_enrollment_backend, build_piper_client, build_stt_client, stt_base_url,
};
use anyhow::Context;
use niles_api::{AppState, DevicePublisher};
use niles_config::Config;
use niles_core::{DeviceId, DeviceRegistry, DeviceState, Event, EventBus};
use niles_mqtt::{Z2mSource, format_set_command, is_actionable};
use niles_recognition::{EcapaTdnnEmbedder, EmbedderConfig};
use niles_wyoming::{SessionTracker, WyomingServer};
use std::sync::Arc;
use std::time::Duration;

pub(crate) fn config_validate(args: ConfigValidateArgs) -> anyhow::Result<()> {
    let cfg = Config::load_from_path(&args.config)
        .with_context(|| format!("loading config from {}", args.config.display()))?;
    cfg.validate().context("validating config")?;
    println!("{} is valid.", args.config.display());
    Ok(())
}

pub(crate) async fn mqtt_tap(args: MqttTapArgs) -> anyhow::Result<()> {
    let (_cfg, mut client) = connect_from_config(&args.config).await?;

    client
        .subscribe(&args.topic)
        .await
        .with_context(|| format!("subscribing to '{}'", args.topic))?;

    eprintln!("Subscribed to '{}'. Press Ctrl-C to exit.\n", args.topic);

    loop {
        tokio::select! {
            msg = client.next_message() => match msg {
                Some(msg) => {
                    let body = msg.payload_str().unwrap_or("<binary payload>");
                    println!("[{}] {}", msg.topic, body);
                }
                None => {
                    let reason = client.last_error().await;
                    match reason {
                        Some(r) => eprintln!("\nMQTT event loop terminated: {r}"),
                        None => eprintln!("\nMQTT event loop terminated."),
                    }
                    break;
                }
            },
            _ = tokio::signal::ctrl_c() => {
                eprintln!("\nReceived Ctrl-C. Exiting.");
                break;
            }
        }
    }

    Ok(())
}

pub(crate) async fn discover(args: DiscoverArgs) -> anyhow::Result<()> {
    let (cfg, client) = connect_from_config(&args.config).await?;

    let registry = Arc::new(DeviceRegistry::new());
    let bus = EventBus::default();
    let mut bus_rx = bus.subscribe();

    let source = Z2mSource::new(client, registry.clone(), bus.clone(), &cfg.mqtt.z2m_prefix);

    eprintln!(
        "Subscribed to {prefix}/bridge/devices and {prefix}/+/+. Press Ctrl-C to exit.\n",
        prefix = cfg.mqtt.z2m_prefix
    );

    let source_handle = tokio::spawn(async move { source.run().await });

    loop {
        tokio::select! {
            ev = bus_rx.recv() => match ev {
                Ok(Event::DeviceAdded { device }) => println!("+ {}", device.id),
                Ok(Event::DeviceRemoved { id }) => println!("- {id}"),
                Ok(Event::DeviceStateChanged { id, state }) => {
                    println!("~ {id} {state:?}");
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    eprintln!("(skipped {n} events — receiver fell behind)");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            },
            _ = tokio::signal::ctrl_c() => {
                eprintln!("\nReceived Ctrl-C.");
                break;
            }
        }
    }

    source_handle.abort();

    let devices = registry.list_all();
    println!("\nFinal registry ({} devices):", devices.len());
    for d in devices {
        println!("  {} state={:?}", d.id, d.state);
    }

    Ok(())
}

pub(crate) async fn set(args: SetArgs) -> anyhow::Result<()> {
    let id = DeviceId::parse(&format!("z2m:{}", args.device))
        .with_context(|| format!("parsing device {:?}", args.device))?;

    let target = DeviceState {
        on: args.on,
        brightness: args.brightness,
        color_temp_kelvin: args.kelvin,
        ..Default::default()
    };
    if !is_actionable(&target) {
        anyhow::bail!("nothing to set — pass at least one of --on / --brightness / --kelvin");
    }

    if args.dry_run {
        // Dry-run uses the config's z2m_prefix if available, but doesn't
        // need credentials or a connection. Default to "zigbee2mqtt"
        // if the config can't be loaded so the user can preview a
        // command without a working config.
        let prefix = Config::load_from_path(&args.config)
            .map(|cfg| cfg.mqtt.z2m_prefix)
            .unwrap_or_else(|_| "zigbee2mqtt".into());
        let (topic, payload) = format_set_command(&prefix, &id, &target);
        println!("[dry-run] {topic}");
        println!("[dry-run] {payload}");
        return Ok(());
    }

    let (cfg, client) = connect_from_config(&args.config).await?;
    let (topic, payload) = format_set_command(&cfg.mqtt.z2m_prefix, &id, &target);
    println!("Publishing {topic}");
    println!("          {payload}");
    client.publish(&topic, payload).await?;

    // QoS::AtLeastOnce: publish() returns once the message is queued,
    // not once it's confirmed on the wire. Give the event loop a
    // beat to flush before we drop the client (which aborts the
    // event-loop task). 250ms is overkill for a healthy broker.
    tokio::time::sleep(Duration::from_millis(250)).await;

    Ok(())
}

pub(crate) async fn api(args: ApiArgs) -> anyhow::Result<()> {
    let (cfg, client) = connect_from_config(&args.config).await?;
    let bind = cfg
        .api
        .socket_addr()
        .context("resolving api.bind_address")?;

    let registry = Arc::new(DeviceRegistry::new());
    let bus = EventBus::default();

    let publisher = client.publisher();
    let source = Z2mSource::new(client, registry.clone(), bus.clone(), &cfg.mqtt.z2m_prefix);
    let source_handle = tokio::spawn(async move {
        if let Err(e) = source.run().await {
            tracing::error!("Z2mSource exited: {e}");
        }
    });

    let state = AppState::new(
        registry.clone(),
        Arc::new(publisher) as Arc<dyn DevicePublisher>,
        build_command_router(&cfg),
        bus.clone(),
    );
    let api_handle = tokio::spawn(async move {
        // The dev subcommand's Ctrl-C below is its stop; the API itself
        // is asked for nothing here.
        if let Err(e) = niles_api::serve(bind, state, std::future::pending::<()>()).await {
            tracing::error!("API server exited: {e}");
        }
    });

    eprintln!(
        "Z2M source running on {prefix}/+/+; API listening on http://{bind}\n  GET  /devices   /rooms/<room>   /healthz\n  WS   /events/stream\n  POST /rooms/<room>   /rooms/<room>/<device>\nPress Ctrl-C to exit.",
        prefix = cfg.mqtt.z2m_prefix
    );

    tokio::signal::ctrl_c()
        .await
        .context("listening for Ctrl-C")?;
    eprintln!("\nReceived Ctrl-C, shutting down.");

    api_handle.abort();
    source_handle.abort();
    Ok(())
}

pub(crate) async fn wyoming_tap(args: WyomingTapArgs) -> anyhow::Result<()> {
    let cfg = Config::load_from_path(&args.config)
        .with_context(|| format!("loading config from {}", args.config.display()))?;
    cfg.validate().context("validating config")?;

    let bind = cfg
        .wyoming
        .socket_addr()
        .context("resolving wyoming.bind_address")?;

    // wyoming-tap doesn't track sessions, so disconnect events
    // have nothing to clean up — discard the receiver. The server
    // sends best-effort and won't block on a dropped receiver.
    let (server, mut rx, _disconnects) = WyomingServer::bind(bind)
        .await
        .with_context(|| format!("binding Wyoming server on {bind}"))?;

    eprintln!(
        "Wyoming server listening on tcp://{bind}\nPoint your satellite at it. Press Ctrl-C to exit.\n"
    );

    let server_handle = tokio::spawn(server.run());

    loop {
        tokio::select! {
            incoming = rx.recv() => match incoming {
                Some(incoming) => {
                    let kind = incoming.event.kind.as_wire_str();
                    let payload_len = incoming.event.payload.len();
                    if payload_len > 0 {
                        println!("[{}] {kind} (+ {payload_len}B payload)", incoming.from);
                    } else {
                        println!("[{}] {kind} {data}", incoming.from, data = incoming.event.data);
                    }
                }
                None => {
                    eprintln!("\nWyoming server stopped.");
                    break;
                }
            },
            _ = tokio::signal::ctrl_c() => {
                eprintln!("\nReceived Ctrl-C. Exiting.");
                break;
            }
        }
    }

    server_handle.abort();
    Ok(())
}

pub(crate) async fn transcribe(args: TranscribeArgs) -> anyhow::Result<()> {
    let cfg = Config::load_from_path(&args.config)
        .with_context(|| format!("loading config from {}", args.config.display()))?;
    cfg.validate().context("validating config")?;

    let audio = std::fs::read(&args.audio)
        .with_context(|| format!("reading audio file {}", args.audio.display()))?;
    let filename = args
        .audio
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_else(|| "audio".into());

    let client = build_stt_client(&cfg)?;

    eprintln!(
        "Transcribing {} ({} bytes) via {} ({}) ...",
        args.audio.display(),
        audio.len(),
        stt_base_url(&cfg),
        cfg.stt.model
    );
    let transcript = client
        .transcribe(audio, &filename)
        .await
        .context("transcribing audio")?;

    if let Some(lang) = &transcript.language {
        eprintln!("Detected language: {lang}");
    }
    if let Some(dur) = transcript.duration_seconds {
        eprintln!("Audio duration: {dur:.2}s");
    }
    println!("{}", transcript.text);
    Ok(())
}

pub(crate) async fn enroll(args: EnrollArgs) -> anyhow::Result<()> {
    let cfg = Config::load_from_path(&args.config)
        .with_context(|| format!("loading config from {}", args.config.display()))?;
    cfg.validate().context("validating config")?;

    if !cfg.recognition.enabled {
        anyhow::bail!("[recognition] must be enabled with model_path and enrollment_dir set");
    }
    let model_path = cfg
        .recognition
        .model_path
        .clone()
        .expect("model_path guaranteed by config validation");
    let mut reader = hound::WavReader::open(&args.audio)
        .with_context(|| format!("opening audio file {}", args.audio.display()))?;
    let spec = reader.spec();
    if spec.sample_rate != 16000 || spec.channels != 1 || spec.bits_per_sample != 16 {
        anyhow::bail!(
            "expected 16 kHz mono 16-bit WAV, got {} Hz {}-channel {}-bit",
            spec.sample_rate,
            spec.channels,
            spec.bits_per_sample
        );
    }
    let samples: Vec<i16> = reader
        .samples::<i16>()
        .collect::<Result<Vec<_>, _>>()
        .with_context(|| "decoding WAV samples")?;

    let embedder = EcapaTdnnEmbedder::new(&EmbedderConfig {
        model_path: model_path.clone(),
        use_gpu: cfg.recognition.use_gpu,
    })
    .with_context(|| format!("loading model from {}", model_path.display()))?;
    let embedding = embedder
        .extract(&samples, 16000)
        .with_context(|| "extracting speaker embedding")?;

    let store = build_enrollment_backend(&cfg)?.context(
        "nowhere to keep enrolled voices: configure [database], or          [recognition.matcher] enrollment_dir",
    )?;
    store
        .enroll(&args.name, &embedding)
        .await
        .context("enrolling speaker")?;

    let record = store
        .load(&args.name)
        .await
        .context("loading enrolled speaker")?;
    println!(
        "Enrolled speaker '{}' with {} clip(s) in {}.",
        record.display_name,
        record.clip_count,
        store.describe()
    );
    println!("A Niles that is already running picks this voice up when it restarts.");
    Ok(())
}

pub(crate) async fn synthesize(args: SynthesizeArgs) -> anyhow::Result<()> {
    let cfg = Config::load_from_path(&args.config)
        .with_context(|| format!("loading config from {}", args.config.display()))?;
    cfg.validate().context("validating config")?;

    let client = build_piper_client(&cfg)?;
    let voice = args.voice.as_deref();
    let voice_for_log = voice.unwrap_or(&cfg.tts.default_voice);

    eprintln!(
        "Synthesizing {} chars via {} (voice {}) ...",
        args.text.chars().count(),
        cfg.tts.base_url,
        voice_for_log
    );
    let synthesis = client
        .synthesize(&args.text, voice)
        .await
        .context("synthesizing speech")?;

    std::fs::write(&args.out, &synthesis.audio_wav)
        .with_context(|| format!("writing WAV to {}", args.out.display()))?;
    println!(
        "Wrote {} bytes to {}",
        synthesis.audio_wav.len(),
        args.out.display()
    );
    Ok(())
}

#[expect(
    clippy::too_many_lines,
    reason = "a dev subcommand's straight-line wiring of the Tier 1 stack"
)]
pub(crate) async fn chat(args: ChatArgs) -> anyhow::Result<()> {
    let (cfg, mqtt_client) = connect_from_config(&args.config).await?;
    let publisher = mqtt_client.publisher();
    let router = build_command_router(&cfg);
    let registry = Arc::new(DeviceRegistry::new());
    let bus = EventBus::default();
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
    tokio::time::sleep(Duration::from_secs(2)).await;

    let capability_loader = build_capability_loader();
    let capability_index = capability_loader.as_deref().map(build_capability_index);

    let memory_store = build_memory_store(&cfg).await;
    let skill_store = build_skill_store(&cfg).await;
    let weather_client = build_weather_client();
    let search = niles_tools::Search::of(&cfg.web_search);
    // Kept separately from the poll loop: the connection has to exist
    // for Settings to authorise it even when nothing polls it yet.
    let tado = build_tado(&cfg.presence, build_tado_tokens(&cfg)?);
    let presence = build_presence(&cfg.presence, tado.clone(), None);
    let presence_handle = presence.as_ref().map(|p| {
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
    let tools_registry = build_tool_registry(
        registry.clone(),
        publisher,
        router,
        capability_loader.clone(),
        memory_store.clone(),
        skill_store.clone(),
        weather_client.clone(),
        search,
        presence_agg,
        &cfg.home,
        None,
        None,
        false,
    );
    let client = build_llm_client(&cfg)?;
    eprintln!(
        "Chatting via {} ({}) ...",
        cfg.base_url_for(
            niles_config::Role::Llm,
            cfg.llm.provider.as_deref(),
            cfg.llm.base_url.as_deref(),
        )
        .unwrap_or_default(),
        cfg.llm.model
    );

    let user_mem = memory_store
        .as_ref()
        .map(|s| s.load(niles_memory::Target::User));
    let agent_mem = memory_store
        .as_ref()
        .map(|s| s.load(niles_memory::Target::Memory));
    let user_mem_str = user_mem.as_deref().and_then(join_memory_entries);
    let agent_mem_str = agent_mem.as_deref().and_then(join_memory_entries);

    let skill_summaries = skill_store.as_ref().map(|s| s.list_summaries());

    let system_prompt = assemble_system_prompt_with_optional_capabilities(
        &args.prompt,
        &cfg.home,
        capability_index.as_ref(),
        capability_loader.as_deref(),
        None,
        user_mem_str.as_deref(),
        agent_mem_str.as_deref(),
        skill_summaries.as_deref(),
        &SpeakerContext::Disabled,
    );

    // Surface the actual error chain on failure. Both loop exhaustion
    // and a real LLM/network error end up here, but they're distinct
    // failure modes — printing `{e:#}` keeps the cause honest instead
    // of mislabeling every error as "loop exhausted".
    match run_tool_calling_chat(
        &client,
        &tools_registry,
        &args.prompt,
        Some(system_prompt.as_str()),
        MAX_TOOL_ITERATIONS,
    )
    .await
    {
        Ok((LoopOutcome::Done(text), _trace)) => println!("{text}"),
        Ok((LoopOutcome::EscalateRequested { reason, .. }, _trace)) => {
            eprintln!(
                "[niles chat] Tier 1 requested escalation ({reason}) but Tier 2 is not available in chat mode"
            );
        }
        Ok((LoopOutcome::HandOff { .. }, _trace)) => unreachable!("nothing is handed off here"),
        Err(e) => eprintln!("[niles chat] {e:#}"),
    }

    source_handle.abort();
    if let Some((h, _publisher)) = wled_handle {
        h.abort();
    }
    if let Some(h) = presence_handle {
        h.abort();
    }
    Ok(())
}

pub(crate) async fn voice_tap(args: VoiceTapArgs) -> anyhow::Result<()> {
    let cfg = Config::load_from_path(&args.config)
        .with_context(|| format!("loading config from {}", args.config.display()))?;
    cfg.validate().context("validating config")?;

    let bind = cfg
        .wyoming
        .socket_addr()
        .context("resolving wyoming.bind_address")?;

    let client = Arc::new(build_stt_client(&cfg)?);
    let (server, mut rx, mut disconnects_rx) = WyomingServer::bind(bind)
        .await
        .with_context(|| format!("binding Wyoming server on {bind}"))?;

    eprintln!(
        "Wyoming server listening on tcp://{bind}\nTranscribing each utterance via {} ({}). Press Ctrl-C to exit.\n",
        stt_base_url(&cfg),
        cfg.stt.model
    );

    let server_handle = tokio::spawn(server.run());
    let mut tracker = SessionTracker::new();

    loop {
        tokio::select! {
            // Drain queued events before reacting to a disconnect:
            // an `audio-stop` already in `rx` should complete its
            // session before `drop_peer` clears the in-flight slot.
            biased;
            incoming = rx.recv() => match incoming {
                Some(incoming) => {
                    if let Some(session) = tracker.feed(incoming) {
                        // Hand transcription off to a task so a slow
                        // STT round-trip doesn't block the next
                        // satellite's audio from being buffered.
                        // Concurrency is unbounded by design for this
                        // dev tap — head-of-line blocking is worse
                        // than fan-out at small N. Prod dispatch will
                        // need a bounded worker pool.
                        let client = client.clone();
                        tokio::spawn(async move {
                            if let Some(heard) = transcribe_session(&client, session).await {
                                println!("[{}] \"{}\"", heard.peer, heard.text);
                                heard.timing.log(heard.peer, &heard.text);
                            }
                        });
                    }
                }
                None => {
                    eprintln!("\nWyoming server stopped.");
                    break;
                }
            },
            disconnect = disconnects_rx.recv() => {
                if let Some(peer) = disconnect {
                    // Free any half-buffered session for this peer
                    // immediately, rather than waiting on the 10-min
                    // idle reaper inside the server.
                    tracker.drop_peer(peer);
                }
            }
            _ = tokio::signal::ctrl_c() => {
                eprintln!("\nReceived Ctrl-C. Exiting.");
                break;
            }
        }
    }

    // Detached transcription tasks (if any) are dropped here without
    // awaiting — fine for a dev tap; a graceful-shutdown signal for
    // those in-flight tasks is a separate concern.
    server_handle.abort();
    Ok(())
}
