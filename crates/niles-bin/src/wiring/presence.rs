//! Who is home: Tado and UniFi presence, and the loop that polls them.

use niles_core::{Event, EventBus};
use niles_presence::{PresenceAggregator, PresenceSource, TadoConfig, TadoSource};
use std::sync::Arc;
use std::time::Duration;

/// Bundle produced by `build_presence` so callers don't juggle a
/// complex tuple.
pub(crate) struct PresenceSetup {
    pub(crate) aggregator: Arc<PresenceAggregator>,
    pub(crate) sources: Vec<Arc<dyn PresenceSource>>,
    /// The UniFi console, when there is one — kept out of `sources` as
    /// well as in it, because the pairing route asks it a question no
    /// other source can answer: which device is making this request.
    pub(crate) unifi: Option<Arc<niles_presence::UnifiSource>>,
    /// The phones that count as somebody being home.
    ///
    /// Shared with the UniFi source and refreshed from config on every
    /// tick, so pairing a phone takes hold without a restart — which
    /// matters more here than anywhere: pairing is the first thing
    /// somebody does, and being told to restart afterwards would make
    /// the button look broken.
    pub(crate) watching: Arc<std::sync::RwLock<std::collections::HashSet<String>>>,
}

/// Build a `PresenceSetup` from the `[presence]` section.
/// Returns `None` when presence is disabled or no sources could be constructed.
/// The tado connection, built whenever there is somewhere to keep its
/// token — **not** gated on `[presence] enabled`.
///
/// Connecting and switching the feature on are separate things, and in
/// that order: authorising something nothing is using is harmless,
/// while turning presence on before anything is connected is a feature
/// that cannot work yet. So the connection is always available to be
/// made, and `enabled` only decides whether anybody polls it.
pub(crate) fn build_tado(
    cfg: &niles_config::PresenceConfig,
    tokens: Option<Arc<dyn niles_presence::TokenStore>>,
) -> Option<Arc<TadoSource>> {
    let tokens = tokens?;
    let tado_cfg = cfg.tado.clone().unwrap_or_default();
    match TadoSource::new(
        TadoConfig {
            home_id: tado_cfg.home_id,
            base_url: tado_cfg.base_url,
            ..TadoConfig::default()
        },
        tokens,
    ) {
        Ok(s) => Some(Arc::new(s)),
        Err(e) => {
            tracing::error!("Tado source build failed: {e}");
            None
        }
    }
}

/// Build a `PresenceSetup` from the `[presence]` section.
///
/// Returns `None` only when there is no source at all. Whether presence
/// is *on* is read live by the poll loop, so switching it on does not
/// need a restart.
pub(crate) fn build_presence(
    cfg: &niles_config::PresenceConfig,
    tado: Option<Arc<TadoSource>>,
    store: Option<Arc<niles_config::ConfigStore>>,
) -> Option<PresenceSetup> {
    let mut sources: Vec<Arc<dyn PresenceSource>> = Vec::new();
    // Polled only if the config actually names tado. The *connection*
    // is built whenever there is a database, so it can be authorised
    // before the feature is switched on — but being able to connect
    // something is not the same as having asked for it, and polling a
    // source nobody configured warns once a tick about a device code
    // nobody was waiting for.
    if let (Some(t), true) = (&tado, cfg.tado.is_some()) {
        sources.push(t.clone() as Arc<dyn PresenceSource>);
    }
    let watching = Arc::new(std::sync::RwLock::new(std::collections::HashSet::new()));
    // Not pushed into `sources`: it is polled separately, and only when
    // there is a console to ask and a phone to look for. Polled as an
    // ordinary source it would warn every ten seconds until somebody
    // paired — which is exactly the state everybody starts in.
    let unifi = build_unifi(cfg, store, watching.clone());

    if sources.is_empty() && unifi.is_none() {
        return None;
    }
    // The debounce is read once. It is a property of the aggregator
    // rather than of a tick, and re-reading it would mean rebuilding
    // the aggregator and losing what it has seen.
    let aggregator = Arc::new(PresenceAggregator::new(chrono::Duration::minutes(5)));
    Some(PresenceSetup {
        aggregator,
        sources,
        unifi,
        watching,
    })
}

/// The UniFi console, read from the live config every time it is asked.
///
/// Built whether or not anything is configured yet. The host and the key
/// are typed into the app while Niles is running, and a source built from
/// them at boot went on using whatever was there at boot: saving the key
/// reported success and changed nothing until a restart, so the pairing
/// button never appeared for somebody who had done everything right.
///
/// Without a store — the subcommands that serve no app — it reads the
/// config it was started with, which is all there is.
pub(crate) fn build_unifi(
    cfg: &niles_config::PresenceConfig,
    store: Option<Arc<niles_config::ConfigStore>>,
    watching: Arc<std::sync::RwLock<std::collections::HashSet<String>>>,
) -> Option<Arc<niles_presence::UnifiSource>> {
    let transport = match niles_presence::HttpUnifiTransport::new(Duration::from_secs(10)) {
        Ok(t) => Arc::new(t),
        Err(e) => {
            tracing::warn!("UniFi presence unavailable: {e}");
            return None;
        }
    };
    let startup = cfg.unifi.clone();
    let settings = move || {
        let live = store.as_ref().map(|s| s.current().presence.unifi.clone());
        let unifi = live.unwrap_or_else(|| startup.clone());
        if !unifi.is_configured() {
            return None;
        }
        // A host without a key is a console that will refuse every
        // request, so it is reported as not set up rather than asked.
        let api_key = unifi.resolve_api_key().ok()?;
        Some(niles_presence::UnifiSettings {
            host: unifi.host.clone(),
            site: unifi.site.clone(),
            api_key,
        })
    };
    Some(Arc::new(niles_presence::UnifiSource::live(
        settings, transport, watching,
    )))
}

/// The phones on the people list, as the network writes them.
pub(crate) fn paired_macs(cfg: &niles_config::Config) -> std::collections::HashSet<String> {
    cfg.auth
        .allowed
        .iter()
        .filter_map(|p| p.device_mac.as_deref())
        .map(|m| m.trim().to_lowercase())
        .filter(|m| !m.is_empty())
        .collect()
}

pub(crate) fn spawn_presence_poll_loop(
    aggregator: Arc<PresenceAggregator>,
    sources: Vec<Arc<dyn PresenceSource>>,
    unifi: Option<Arc<niles_presence::UnifiSource>>,
    // Shared with the UniFi source; refreshed from config each tick.
    watching: Arc<std::sync::RwLock<std::collections::HashSet<String>>>,
    startup: niles_config::PresenceConfig,
    store: Option<Arc<niles_config::ConfigStore>>,
    bus: EventBus,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut last_published: Option<niles_presence::HomeState> = None;
        // When each source may next be asked, and whether its login was
        // refused — see `retry_after`.
        let mut due: Vec<std::time::Instant> = vec![std::time::Instant::now(); sources.len()];
        let mut refused: Vec<bool> = vec![false; sources.len()];
        loop {
            // Read live rather than captured, so turning presence on in
            // Settings takes hold on the next tick instead of at the
            // next restart. `[presence]` is `Reload::Hot` for this
            // reason, and this loop is the thing that makes it true.
            let snapshot = store.as_ref().map(|s| s.current());
            let cfg = snapshot
                .as_ref()
                .map(|c| c.presence.clone())
                .unwrap_or_else(|| startup.clone());
            // Who counts as home, read live. Pairing a phone writes one
            // of these, and being told to restart afterwards would make
            // the button look like it had not worked.
            if let Some(current) = &snapshot {
                *watching.write().unwrap_or_else(|e| e.into_inner()) = paired_macs(current);
            }

            if !cfg.enabled {
                // Idle, but not asleep: checking every half minute is
                // what makes switching it on feel immediate without
                // spending anything upstream while it is off.
                tokio::time::sleep(Duration::from_secs(30)).await;
                continue;
            }

            // Each at its own pace. The loop ticks at the shortest
            // interval anyone wants — UniFi's seconds — and asking tado
            // on every one of those ticks spent its daily allowance in a
            // morning and got the login refused.
            let interval = Duration::from_secs(cfg.poll_seconds);
            for (i, src) in sources.iter().enumerate() {
                let now = std::time::Instant::now();
                if now < due[i] {
                    continue;
                }
                match src.poll().await {
                    Ok(signal) => {
                        if refused[i] {
                            tracing::info!("presence source '{}' is answering again", src.name());
                        }
                        refused[i] = false;
                        aggregator.ingest(signal);
                        due[i] = now + interval;
                    }
                    Err(e) => {
                        let auth = matches!(e, niles_presence::Error::Auth { .. });
                        // Once, not every attempt: the cure is a person
                        // reconnecting it in Settings, and repeating the
                        // news does not bring that sooner.
                        if !(auth && refused[i]) {
                            tracing::warn!("presence source '{}' poll failed: {e}", src.name());
                        }
                        refused[i] = auth;
                        due[i] = now + retry_after(auth, interval);
                    }
                }
            }
            // Asked only when it can answer: a console set up in the app,
            // and a phone paired to look for. Either missing is the normal
            // state of a house that has not finished setting up, not a
            // fault worth a warning per tick.
            let anyone_paired = !watching
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .is_empty();
            if let Some(u) = &unifi
                && u.is_configured()
                && anyone_paired
            {
                match u.poll().await {
                    Ok(signal) => aggregator.ingest(signal),
                    Err(e) => tracing::warn!("presence source 'unifi' poll failed: {e}"),
                }
            }
            let resolved = aggregator.resolve(chrono::Utc::now());
            if let Some(state) = presence_transition(&mut last_published, resolved) {
                bus.publish(Event::PresenceChanged {
                    state: state.into(),
                });
            }

            // The shortest interval any source asks for. UniFi is on
            // the LAN and wants seconds; tado is rate-limited by the
            // day and wants minutes. Polling both at the slower rate
            // would throw away the only reason UniFi is here.
            let wait = if unifi.as_ref().is_some_and(|u| u.is_configured()) {
                cfg.poll_seconds.min(cfg.unifi.poll_seconds).max(2)
            } else {
                cfg.poll_seconds.max(10)
            };
            tokio::time::sleep(Duration::from_secs(wait)).await;
        }
    })
}

/// How long before asking a presence source again after it failed.
///
/// A refused login does not fix itself — somebody has to reconnect it —
/// so it is asked rarely rather than at every interval: tado counts
/// requests against a daily allowance, refused ones included.
pub(crate) fn retry_after(auth_refused: bool, interval: Duration) -> Duration {
    if auth_refused {
        interval.max(Duration::from_secs(30 * 60))
    } else {
        interval
    }
}

/// Pure transition detector — returns `Some(state)` when `resolved`
/// differs from `*last` (and updates `*last`), `None` otherwise.
/// First call (last is None) always returns Some(resolved).
/// Decide whether a freshly-resolved presence state warrants
/// publishing a `PresenceChanged` event.
///
/// The FIRST resolve after startup seeds `last` WITHOUT publishing:
/// `PresenceChanged` is an edge-triggered transition between two
/// known states, not the initial unknown→known resolution. Without
/// this, every service restart would re-fire presence-triggered
/// automations ("welcome home", "everyone left") even though nobody
/// actually arrived or left. niles has no persisted pre-restart
/// state to diff against, so suppressing the one initial edge is the
/// safe choice — on-demand `get_presence` still reflects reality
/// immediately; only the edge-triggered automation is skipped for
/// that first resolution.
pub(crate) fn presence_transition(
    last: &mut Option<niles_presence::HomeState>,
    resolved: niles_presence::HomeState,
) -> Option<niles_presence::HomeState> {
    match *last {
        // First resolve after startup: seed, don't publish.
        None => {
            *last = Some(resolved);
            None
        }
        Some(prev) if prev == resolved => None,
        Some(_) => {
            *last = Some(resolved);
            Some(resolved)
        }
    }
}
