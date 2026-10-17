//! Memory, the journal, conversations, skills and device history, kept in Postgres.

use crate::{approvals, chat_log, conversation, review};
use anyhow::Context;
use chrono::Utc;
use niles_config::Config;
use niles_core::{DeviceId, DeviceState};
use niles_history::{StateEntry, StateReader, StateWriter};
use niles_memory::MemoryStore;
use niles_skills::SkillStore;
use std::sync::Arc;
use std::time::Duration;

/// Hands a changed memory document to the task that writes it, so the
/// tool that changed it never waits on the database.
pub(crate) struct QueuedMemory {
    pub(crate) tx: tokio::sync::mpsc::UnboundedSender<(&'static str, String)>,
}

impl QueuedMemory {
    pub(crate) fn send(&self, row: &'static str, document: &str) {
        if self.tx.send((row, document.to_string())).is_err() {
            tracing::warn!(
                "[memory] nothing is writing it any more; this change is in memory only"
            );
        }
    }
}

/// The row the forgotten notes are kept in, beside `user` and `agent`.
pub(crate) const FORGOTTEN_ROW: &str = "forgotten";

impl niles_memory::MemoryPersistence for QueuedMemory {
    fn store(&self, target: niles_memory::Target, document: &str) {
        self.send(target.as_str(), document);
    }

    fn store_forgotten(&self, document: &str) {
        self.send(FORGOTTEN_ROW, document);
    }
}

/// What Niles remembers, kept in Postgres when there is one. Always on:
/// it used to need a `[memory] directory`, and without one Niles forgot
/// everything it was told.
///
/// Without a database it lasts as long as the process, and says so.
/// Memory that cannot be read is left out rather than started empty:
/// the first thing remembered would otherwise overwrite the rest.
pub(crate) async fn build_memory_store(cfg: &Config) -> Option<Arc<MemoryStore>> {
    let store = MemoryStore::new(niles_memory::MemoryConfig {
        user_char_limit: cfg.memory.user_char_limit,
        agent_char_limit: cfg.memory.agent_char_limit,
    });
    let in_memory = |store| {
        tracing::warn!("[memory] no database; what Niles learns now will not survive a restart");
        Some(Arc::new(store))
    };
    let Some(database) = &cfg.database else {
        return in_memory(store);
    };
    let Ok(url) = database.resolve_url() else {
        return in_memory(store);
    };
    let Ok(backend) = niles_db::PostgresBackend::connect_lazy(&url, database.max_connections)
    else {
        return in_memory(store);
    };
    let table = Arc::new(niles_db::PostgresMemory::new(
        backend.pool(),
        backend.describe_target(),
    ));
    let user = niles_memory::Target::User;
    let agent = niles_memory::Target::Memory;
    let documents = match (
        table.load(user.as_str()).await,
        table.load(agent.as_str()).await,
        table.load(FORGOTTEN_ROW).await,
    ) {
        (Ok(u), Ok(a), Ok(f)) => store
            .with_documents(&u, &a)
            .and_then(|s| {
                if f.is_empty() {
                    Ok(s)
                } else {
                    s.with_forgotten(&f)
                }
            })
            .map_err(|e| e.to_string()),
        (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => Err(e),
    };
    let store = match documents {
        Ok(store) => store,
        Err(e) => {
            tracing::error!("[memory] could not be read ({e}); it is unavailable this run");
            return None;
        }
    };

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(&'static str, String)>();
    tokio::spawn(async move {
        while let Some((row, document)) = rx.recv().await {
            if let Err(e) = table.store(row, &document).await {
                tracing::error!("[memory] save failed: {e}");
            }
        }
    });
    Some(Arc::new(store.with_sink(Arc::new(QueuedMemory { tx }))))
}

/// What was said since the last review, kept in Postgres when there is
/// one, so a deploy between a conversation and its review loses neither.
pub(crate) async fn build_journal(cfg: &Config) -> Arc<review::Journal> {
    let journal = review::Journal::default();
    let Some(database) = &cfg.database else {
        return Arc::new(journal);
    };
    let Ok(url) = database.resolve_url() else {
        return Arc::new(journal);
    };
    let Ok(backend) = niles_db::PostgresBackend::connect_lazy(&url, database.max_connections)
    else {
        return Arc::new(journal);
    };
    let table = niles_db::PostgresJournal::new(backend.pool(), backend.describe_target());
    let journal = match table.load().await {
        Ok(Some(document)) => journal.with_document(&document),
        Ok(None) => journal,
        // Not read is not empty: started empty and saved, it would
        // overwrite what is there. This run keeps its own, unsaved.
        Err(e) => {
            tracing::warn!(
                "[learning] the journal could not be read ({e}); this run's is not saved"
            );
            return Arc::new(journal);
        }
    };
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        while let Some(mut document) = rx.recv().await {
            while let Ok(newer) = rx.try_recv() {
                document = newer;
            }
            if let Err(e) = table.store(&document).await {
                tracing::warn!("[learning] journal save failed: {e}");
            }
        }
    });
    Arc::new(journal.with_sink(tx))
}

pub(crate) fn build_chat_log(cfg: &Config) -> Option<Arc<chat_log::ChatLog>> {
    let database = cfg.database.as_ref()?;
    let url = database.resolve_url().ok()?;
    let backend = niles_db::PostgresBackend::connect_lazy(&url, database.max_connections).ok()?;
    Some(Arc::new(chat_log::ChatLog::new(
        niles_db::PostgresConversationLog::new(backend.pool(), backend.describe_target()),
    )))
}

/// What each person was asked to approve, and decided, kept for the
/// history under Approvals. Absent without a database, and the history
/// then says it keeps none.
pub(crate) fn build_approval_log(cfg: &Config) -> Option<Arc<dyn approvals::Ledger>> {
    let database = cfg.database.as_ref()?;
    let url = database.resolve_url().ok()?;
    let backend = niles_db::PostgresBackend::connect_lazy(&url, database.max_connections).ok()?;
    Some(Arc::new(niles_db::PostgresApprovalLog::new(
        backend.pool(),
        backend.describe_target(),
    )))
}

/// The conversations going on, kept in Postgres when there is one, so
/// a restart does not forget what was said a minute ago.
pub(crate) async fn build_conversation_memory(
    cfg: &Config,
) -> Arc<conversation::ConversationMemory> {
    let memory = conversation::ConversationMemory::default();
    let Some(database) = &cfg.database else {
        return Arc::new(memory);
    };
    let Ok(url) = database.resolve_url() else {
        return Arc::new(memory);
    };
    let Ok(backend) = niles_db::PostgresBackend::connect_lazy(&url, database.max_connections)
    else {
        return Arc::new(memory);
    };
    let table = niles_db::PostgresConversations::new(backend.pool(), backend.describe_target());
    let memory = match table.load().await {
        Ok(Some(document)) => memory.with_document(&document),
        Ok(None) => memory,
        Err(e) => {
            tracing::warn!("[conversation] could not be read ({e}); this run's is not saved");
            return Arc::new(memory);
        }
    };
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        while let Some(mut document) = rx.recv().await {
            // Each document is the whole of it; only the newest matters.
            while let Ok(newer) = rx.try_recv() {
                document = newer;
            }
            if let Err(e) = table.store(&document).await {
                tracing::warn!("[conversation] save failed: {e}");
            }
        }
    });
    Arc::new(memory.with_sink(tx))
}

/// Hands the changed set of skills to the task that writes it.
pub(crate) struct QueuedSkills {
    pub(crate) tx: tokio::sync::mpsc::UnboundedSender<String>,
}

impl niles_skills::SkillPersistence for QueuedSkills {
    fn store(&self, document: &str) {
        if self.tx.send(document.to_string()).is_err() {
            tracing::warn!(
                "[skills] nothing is writing them any more; this change is in memory only"
            );
        }
    }
}

/// The skills Niles has made, kept in Postgres when there is one. Always
/// on: they used to need a `[skills] directory`, and without one Niles
/// could not learn anything.
///
/// Without a database they last as long as the process, and say so. A
/// set that cannot be read is left out rather than started empty: the
/// first skill made would otherwise overwrite the rest.
pub(crate) async fn build_skill_store(cfg: &Config) -> Option<Arc<SkillStore>> {
    let max_chars = cfg.skills.skill_max_chars;
    let in_memory = || {
        tracing::warn!("[skills] no database; skills made now will not survive a restart");
        Some(Arc::new(SkillStore::new(max_chars)))
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
    let table = Arc::new(niles_db::PostgresSkills::new(
        backend.pool(),
        backend.describe_target(),
    ));
    let store = match table.load().await {
        Ok(Some(document)) => match SkillStore::from_json(&document, max_chars) {
            Ok(store) => store,
            Err(e) => {
                tracing::error!(
                    "[skills] the stored skills would not load ({e}); they are unavailable this run"
                );
                return None;
            }
        },
        Ok(None) => SkillStore::new(max_chars),
        Err(e) => {
            tracing::error!("[skills] could not be read ({e}); they are unavailable this run");
            return None;
        }
    };

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        while let Some(mut document) = rx.recv().await {
            // Each document is the whole set; only the newest matters.
            while let Ok(newer) = rx.try_recv() {
                document = newer;
            }
            if let Err(e) = table.store(&document).await {
                tracing::error!("[skills] save failed: {e}");
            }
        }
    });
    Some(Arc::new(store.with_sink(Arc::new(QueuedSkills { tx }))))
}

pub(crate) fn spawn_skill_curator(
    store: Arc<SkillStore>,
    curator_cfg: niles_config::SkillsCuratorConfig,
) -> Option<tokio::task::JoinHandle<()>> {
    if !curator_cfg.enabled {
        tracing::info!("skill curator disabled by config");
        return None;
    }
    let interval = Duration::from_secs(curator_cfg.interval_hours * 3600);
    let thresholds = niles_skills::curator::CuratorThresholds {
        stale_after: chrono::Duration::days(curator_cfg.stale_after_days as i64),
        archive_after: chrono::Duration::days(curator_cfg.archive_after_days as i64),
    };
    Some(tokio::spawn(async move {
        // Defer the first sweep so startup work isn't competed with.
        tokio::time::sleep(Duration::from_secs(60)).await;
        loop {
            let r =
                niles_skills::curator::apply_automatic_transitions(&store, Utc::now(), thresholds);
            tracing::info!(
                examined = r.examined,
                became_stale = r.became_stale,
                became_archived = r.became_archived,
                revived = r.revived,
                skipped_pinned = r.skipped_pinned,
                skipped_user_created = r.skipped_user_created,
                "skill curator swept",
            );
            tokio::time::sleep(interval).await;
        }
    }))
}

/// Build a `StateWriter` from the `[history]` section, pruning old
/// files on open. Returns a disabled no-op writer when no directory
/// is configured.
pub(crate) fn build_state_writer(cfg: &Config) -> anyhow::Result<Arc<StateWriter>> {
    let writer = match &cfg.history.directory {
        Some(dir) => {
            let w = StateWriter::new(dir).context("opening state history writer")?;
            if let Err(e) = w.prune(cfg.history.retention_days) {
                tracing::warn!("state history prune failed: {e:#}");
            }
            w
        }
        None => StateWriter::disabled(),
    };
    Ok(Arc::new(writer))
}

/// Build a `StateReader` from the `[history]` section. Returns a
/// disabled no-op reader when no directory is configured.
pub(crate) fn build_state_reader(cfg: &Config) -> Arc<StateReader> {
    match &cfg.history.directory {
        Some(dir) => Arc::new(StateReader::new(dir)),
        None => Arc::new(StateReader::disabled()),
    }
}

/// Append a device-state snapshot to the history log.
pub(crate) fn append_state_history(writer: &StateWriter, id: &DeviceId, state: &DeviceState) {
    if let Err(e) = writer.append(&StateEntry {
        ts: Utc::now(),
        device_id: id.clone(),
        state: state.clone(),
    }) {
        tracing::warn!("state history append failed: {e:#}");
    }
}
