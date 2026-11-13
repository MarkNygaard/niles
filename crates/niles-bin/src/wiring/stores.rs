//! The Postgres-backed stores the house keeps: timers, scenes, groceries, artifacts and more.

use crate::{artifacts, chat_log, training};
use anyhow::Context;
use niles_config::Config;
use niles_core::EventBus;
use niles_scheduler::{MorningClaimTracker, SceneStore, TimerStore};
use niles_tools::ToolRegistry;
use std::sync::Arc;

/// Every conversation, kept per person for the app's chat. Nothing
/// without a database: the chat then remembers only the one going on.
/// Each person's artifacts, whenever there is a database: no switch, a
/// person asking for one is the switch.
pub(crate) fn build_artifacts(
    cfg: &Config,
    bus: EventBus,
    runner: Arc<artifacts::AssistantRunner>,
    chat_log: Option<Arc<chat_log::ChatLog>>,
    search: niles_tools::Search,
    training: Option<Arc<dyn niles_training::TrainingStore>>,
    notify: Arc<crate::notify::Notify>,
) -> Option<Arc<artifacts::Artifacts>> {
    // The household is who the settings allow in: the same store the
    // assistant is chosen from.
    let settings = runner.settings.clone();
    let database = cfg.database.as_ref()?;
    let url = database.resolve_url().ok()?;
    let backend = niles_db::PostgresBackend::connect_lazy(&url, database.max_connections).ok()?;
    let store = niles_db::PostgresArtifacts::new(backend.pool(), backend.describe_target());
    let timezone = cfg.home.timezone.clone();
    let web = search.clone();
    // What writing needs and nothing of the house's: a job cannot turn
    // on a light.
    let extras: artifacts::Extras = Box::new(move |reg: &mut ToolRegistry| {
        niles_tools::register_web_tools(reg, web.clone());
        niles_tools::register_datetime_tool(reg, &timezone);
    });
    // Pictures are found with the house's search, while it has one, and
    // fetched from the public internet only.
    let pictures = artifacts::Pictures {
        search,
        download: Arc::new(niles_websearch::PublicFetcher::new(
            niles_artifacts::images::MAX_DOWNLOAD,
            artifacts::DOWNLOAD_WITHIN,
            niles_websearch::USER_AGENT,
        )),
    };
    let artifacts = artifacts::Artifacts::new(
        Arc::new(store),
        bus,
        runner,
        notify as Arc<dyn artifacts::Notifier>,
        chat_log.map(|log| log as Arc<dyn artifacts::Conversations>),
        extras,
    )
    .with_pictures(pictures)
    .with_podcasts(artifacts::Podcasts::live(settings.clone()))
    .with_household(settings as Arc<dyn artifacts::Household>);
    let artifacts = match (training, cfg.home.timezone.parse::<chrono_tz::Tz>()) {
        (Some(store), Ok(tz)) => {
            let imperial = cfg.home.resolved_units() == niles_config::Units::Imperial;
            artifacts.with_training(
                Box::new(move |reg: &mut ToolRegistry| {
                    training::register(reg, store.clone(), tz, imperial);
                }),
                tz,
            )
        }
        _ => artifacts,
    };
    Some(Arc::new(artifacts))
}

/// Running timers, kept in Postgres when there is one, so a deploy no
/// longer drops the one somebody has going. A timer that ran out while
/// Niles was down comes back ringing.
///
/// Without a database they last as long as the process, and say so. A
/// stored document that will not parse is logged and dropped: timers
/// are minutes long, and starting with none costs less than refusing to.
pub(crate) async fn build_timer_store(cfg: &Config) -> Arc<TimerStore> {
    let in_memory = || {
        tracing::warn!("[timers] no database; timers will not survive a restart");
        Arc::new(TimerStore::new())
    };
    let Some(database) = &cfg.database else {
        return in_memory();
    };
    let Ok(url) = database.resolve_url() else {
        return in_memory();
    };
    let Ok(backend) = niles_db::PostgresBackend::connect_lazy(&url, database.max_connections)
    else {
        return in_memory();
    };
    let table = Arc::new(niles_db::PostgresTimers::new(
        backend.pool(),
        backend.describe_target(),
    ));
    let store = match table.load().await {
        Ok(Some(document)) => TimerStore::from_json(&document).unwrap_or_else(|e| {
            tracing::error!("[timers] the stored document would not parse ({e}); starting empty");
            TimerStore::new()
        }),
        Ok(None) => TimerStore::new(),
        Err(e) => {
            // Saving over a set that could not be read would lose it for
            // good; leave it for the next run instead.
            tracing::error!("[timers] could not be read ({e}); this run's timers are not saved");
            return Arc::new(TimerStore::new());
        }
    };
    let sink = queue_document_writes("timers", move |document| {
        let table = table.clone();
        async move { table.store(&document).await }
    });
    Arc::new(store.with_sink(sink))
}

/// The lights the morning routine holds, kept in Postgres when there is
/// one. A restart mid-ramp would otherwise hand them straight back to
/// the curve.
///
/// Without a database the claims last as long as the process, and say
/// so. A stored document that will not parse is logged and dropped: the
/// worst a lost claim does is end one morning's ramp early.
pub(crate) async fn build_morning_claims(cfg: &Config) -> Arc<MorningClaimTracker> {
    let in_memory = || {
        tracing::warn!("[morning] no database; claims will not survive a restart");
        Arc::new(MorningClaimTracker::new())
    };
    let Some(database) = &cfg.database else {
        return in_memory();
    };
    let Ok(url) = database.resolve_url() else {
        return in_memory();
    };
    let Ok(backend) = niles_db::PostgresBackend::connect_lazy(&url, database.max_connections)
    else {
        return in_memory();
    };
    let table = Arc::new(niles_db::PostgresMorningClaims::new(
        backend.pool(),
        backend.describe_target(),
    ));
    let tracker = match table.load().await {
        Ok(Some(document)) => MorningClaimTracker::from_json(&document).unwrap_or_else(|e| {
            tracing::error!("[morning] the stored claims would not parse ({e}); starting empty");
            MorningClaimTracker::new()
        }),
        Ok(None) => MorningClaimTracker::new(),
        Err(e) => {
            tracing::error!("[morning] claims could not be read ({e}); this run's are not saved");
            return Arc::new(MorningClaimTracker::new());
        }
    };
    let sink = queue_document_writes("morning", move |document| {
        let table = table.clone();
        async move { table.store(&document).await }
    });
    Arc::new(tracker.with_sink(sink))
}

/// The database the wake-ups and the automations' waits are kept in.
pub(crate) fn database(cfg: &Config) -> Option<niles_db::PostgresBackend> {
    let database = cfg.database.as_ref()?;
    let url = database.resolve_url().ok()?;
    niles_db::PostgresBackend::connect_lazy(&url, database.max_connections).ok()
}

/// Where wake-ups under way are kept. Without a database they last as
/// long as Niles runs.
pub(crate) fn wake_up_store(cfg: &Config) -> Option<Arc<niles_db::PostgresWakeUps>> {
    match database(cfg) {
        Some(db) => Some(Arc::new(niles_db::PostgresWakeUps::new(
            db.pool(),
            db.describe_target(),
        ))),
        None => {
            tracing::warn!("[wake-up] no database; a wake-up will not survive a restart");
            None
        }
    }
}

/// Where kept wake audio goes, when there is a database.
///
/// Built whether or not capture is switched on: the switch lives in
/// the config and moves while Niles is running, and a store that only
/// exists if it was on at boot would make flipping it do nothing —
/// which is the bug that let a television talk to an empty house all
/// day.
pub(crate) fn build_capture_store(cfg: &Config) -> Option<Arc<niles_db::PostgresCaptures>> {
    let database = cfg.database.as_ref()?;
    let url = database.resolve_url().ok()?;
    let backend = niles_db::PostgresBackend::connect_lazy(&url, database.max_connections).ok()?;
    let describe = backend.describe_target();
    Some(Arc::new(niles_db::PostgresCaptures::new(
        backend.pool(),
        describe,
    )))
}

/// Hands a scheduler store's changed document to a task that writes it.
///
/// The scene, timer and morning-claim stores are sync — they are read on
/// paths that cannot await — so they cannot do a database round trip
/// themselves. They hand the document to this instead, and a task
/// drains the queue. The last document wins, which is what "the whole
/// set, replaced" already means.
///
/// The window where a change is made and the process dies before the
/// write lands is real and small. A change that blocked a voice reply
/// on a database is worse.
pub(crate) struct QueuedDocument {
    pub(crate) what: &'static str,
    pub(crate) tx: tokio::sync::mpsc::UnboundedSender<String>,
}

impl niles_scheduler::DocumentSink for QueuedDocument {
    fn store(&self, document: &str) {
        // A closed channel means the writer task is gone, which is
        // worth one line rather than one per change.
        if self.tx.send(document.to_string()).is_err() {
            tracing::warn!(
                "[{}] nothing is writing them any more; this change is in memory only",
                self.what
            );
        }
    }
}

/// Start the task that writes `what` with `save`, and return the sink
/// that feeds it.
pub(crate) fn queue_document_writes<F, Fut>(what: &'static str, save: F) -> Arc<QueuedDocument>
where
    F: Fn(String) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<(), String>> + Send,
{
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        while let Some(mut document) = rx.recv().await {
            // Each document is the whole state; only the newest matters.
            while let Ok(newer) = rx.try_recv() {
                document = newer;
            }
            if let Err(e) = save(document).await {
                tracing::error!("[{what}] save failed: {e}");
            }
        }
    });
    Arc::new(QueuedDocument { what, tx })
}

/// Where each room's last station is kept: the database, when there is
/// one. Without it, rooms remember their stations until a restart.
pub(crate) fn build_room_music(cfg: &Config) -> Option<Arc<niles_db::PostgresRoomMusic>> {
    let database = cfg.database.as_ref()?;
    let url = database.resolve_url().ok()?;
    let backend = niles_db::PostgresBackend::connect_lazy(&url, database.max_connections).ok()?;
    let describe = backend.describe_target();
    Some(Arc::new(niles_db::PostgresRoomMusic::new(
        backend.pool(),
        describe,
    )))
}

/// The scene store, kept in Postgres when there is one: it needs
/// nothing in the config file beyond the line that names the database,
/// which is the point.
///
/// Without a database scenes live until the process does — and say so,
/// because losing them silently is how this was found.
pub(crate) async fn build_scene_store(cfg: &Config) -> Arc<SceneStore> {
    let in_memory = || {
        tracing::warn!("[scenes] no database; scenes will not survive a restart");
        Arc::new(SceneStore::new())
    };
    let Some(database) = &cfg.database else {
        return in_memory();
    };
    let Ok(url) = database.resolve_url() else {
        return in_memory();
    };
    let Ok(backend) = niles_db::PostgresBackend::connect_lazy(&url, database.max_connections)
    else {
        return in_memory();
    };
    let describe = backend.describe_target();
    let scenes = Arc::new(niles_db::PostgresScenes::new(backend.pool(), describe));

    let store = match scenes.load().await {
        Ok(Some(document)) => SceneStore::from_json(&document).unwrap_or_else(|e| {
            tracing::error!("[scenes] the stored document would not parse ({e}); starting empty");
            SceneStore::new()
        }),
        Ok(None) => SceneStore::new(),
        Err(e) => {
            // Starting empty here would look like the scenes were
            // deleted, and the next save would make that true.
            tracing::error!("[scenes] could not be read ({e}); they are unavailable this run");
            return Arc::new(SceneStore::new());
        }
    };

    let sink = queue_document_writes("scenes", move |document| {
        let scenes = scenes.clone();
        async move { scenes.store(&document).await }
    });
    Arc::new(store.with_sink(sink))
}

/// Hands each new grocery document to a task that writes it, the way
/// [`QueuedDocument`] does for scenes: checking something off in the shop
/// must not wait on the database.
pub(crate) struct QueuedGroceries {
    pub(crate) tx: tokio::sync::mpsc::UnboundedSender<String>,
}

impl niles_groceries::GroceryPersistence for QueuedGroceries {
    fn store(&self, document: &str) {
        if self.tx.send(document.to_string()).is_err() {
            tracing::warn!(
                "[groceries] nothing is writing the list any more; this change is in memory only"
            );
        }
    }
}

/// The shopping list, kept in Postgres when there is one.
///
/// Without a database it lives as long as the process, and says so. A
/// list that cannot be read is left out entirely rather than started
/// empty: the first item added would otherwise overwrite the real one.
pub(crate) async fn build_grocery_store(
    cfg: &Config,
) -> Option<Arc<niles_groceries::GroceryStore>> {
    let in_memory = || {
        tracing::warn!("[groceries] no database; the shopping list will not survive a restart");
        Some(Arc::new(niles_groceries::GroceryStore::new()))
    };
    let Some(database) = &cfg.database else {
        return in_memory();
    };
    let Ok(url) = database.resolve_url() else {
        return in_memory();
    };
    let Ok(backend) = niles_db::PostgresBackend::connect_lazy(&url, database.max_connections)
    else {
        return in_memory();
    };
    let describe = backend.describe_target();
    let groceries = Arc::new(niles_db::PostgresGroceries::new(backend.pool(), describe));

    let store = match groceries.load().await {
        Ok(Some(document)) => match niles_groceries::GroceryStore::from_json(&document) {
            Ok(store) => store,
            Err(e) => {
                tracing::error!(
                    "[groceries] the stored list would not parse ({e}); it is unavailable this run"
                );
                return None;
            }
        },
        Ok(None) => niles_groceries::GroceryStore::new(),
        Err(e) => {
            tracing::error!(
                "[groceries] could not be read ({e}); the list is unavailable this run"
            );
            return None;
        }
    };

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        while let Some(document) = rx.recv().await {
            if let Err(e) = groceries.store(&document).await {
                tracing::error!("[groceries] save failed: {e}");
            }
        }
    });
    Some(Arc::new(store.with_sink(Arc::new(QueuedGroceries { tx }))))
}

pub(crate) fn build_override_backend(
    cfg: &Config,
) -> anyhow::Result<Box<dyn niles_config::OverrideBackend>> {
    if let Some(database) = &cfg.database {
        let url = database
            .resolve_url()
            .context("resolving the database connection string")?;
        let backend = niles_db::PostgresBackend::connect_lazy(&url, database.max_connections)
            // The DSN parser's error can quote the DSN, which holds a
            // password. Say only that it was rejected.
            .map_err(|_| {
                anyhow::anyhow!(
                    "the connection string in {} is not a valid database URL",
                    database.url_env
                )
            })?;
        return Ok(Box::new(backend));
    }
    Ok(Box::new(niles_config::MemoryBackend::new()))
}

#[cfg(test)]
mod override_backend_tests {
    use super::*;

    const BASE: &str = r#"
[home]
name = "test home"
latitude = 56.1572
longitude = 10.2107
timezone = "Europe/Copenhagen"

[mqtt]
host = "192.168.42.16"
port = 1883
username_env = "NILES_MQTT_USERNAME"
password_env = "NILES_MQTT_PASSWORD"

[api]
bind_address = "0.0.0.0:8080"

[wyoming]
bind_address = "0.0.0.0:10300"

[stt]
api_key_env = "ELEVENLABS_API_KEY"

[tts]

[llm]
api_key_env = "CEREBRAS_API_KEY"

[lighting]
morning_start = "05:45"
morning_end = "06:30"
sunset_start = "21:30"
sunset_end = "23:00"
night_floor_brightness = 15
daytime_brightness = 100

[[lighting.color_temp_anchors]]
time = "00:00"
kelvin = 2000

[[lighting.color_temp_anchors]]
time = "23:59"
kelvin = 2000
"#;

    fn config_with(extra: &str) -> Config {
        Config::load_from_str(&format!("{BASE}{extra}")).expect("fixture parses")
    }

    #[tokio::test]
    async fn a_database_section_selects_the_postgres_backend() {
        // The bug this test exists for: niles-db compiled, tested and
        // shipped while nothing ever constructed it, so config lived in
        // memory and the database stayed empty.
        // SAFETY: a test-only variable name nothing else reads.
        unsafe {
            std::env::set_var(
                "NILES_TEST_WIRING_DSN",
                "postgres://niles:pw@postgres-rw.database.svc.cluster.local:5432/niles",
            );
        }
        let cfg = config_with(
            "
[database]
url_env = \"NILES_TEST_WIRING_DSN\"
",
        );
        let backend = build_override_backend(&cfg).expect("backend builds");
        let described = backend.describe();
        assert!(
            described.contains("niles database"),
            "expected the postgres backend, got: {described}"
        );
        assert!(
            !described.contains("pw@"),
            "credentials must not appear in the description: {described}"
        );
    }

    #[tokio::test]
    async fn a_missing_env_var_is_fatal_rather_than_silently_falling_back() {
        // Falling back to memory here is how a database quietly goes unused.
        let cfg = config_with(
            "
[database]
url_env = \"NILES_TEST_DEFINITELY_NOT_SET\"
",
        );
        // `Box<dyn OverrideBackend>` has no Debug, so unwrap the Result by hand.
        let err = match build_override_backend(&cfg) {
            Ok(backend) => panic!("must refuse to start, got {}", backend.describe()),
            Err(e) => e,
        };
        assert!(err.to_string().contains("connection string"));
    }

    #[tokio::test]
    async fn without_a_database_it_falls_back_to_memory_and_says_so() {
        let described = build_override_backend(&config_with("")).unwrap().describe();
        assert!(described.contains("memory"), "got: {described}");
    }
}
