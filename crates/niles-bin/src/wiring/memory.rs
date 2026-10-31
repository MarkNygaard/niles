//! Memory, the journal, conversations, skills and device history, kept in Postgres.

use crate::{approvals, chat_log, conversation, review};
use chrono::Utc;
use niles_config::Config;
use niles_core::{DeviceId, DeviceState};
use niles_history::{CommandEntry, CommandHistory, KEPT_DAYS, StateEntry, StateHistory};
use niles_memory::MemoryStore;
use niles_skills::SkillStore;
use std::collections::HashMap;
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

/// The device state and command histories, both in Postgres.
pub(crate) struct History {
    pub(crate) states: Arc<dyn StateHistory>,
    pub(crate) commands: Arc<dyn CommandHistory>,
}

impl History {
    /// Offer the tools that read the histories.
    pub(crate) fn register_tools(
        &self,
        tools: &mut niles_tools::ToolRegistry,
        registry: Arc<niles_core::DeviceRegistry>,
    ) {
        niles_tools::register_history_tools(tools, self.commands.clone());
        niles_tools::register_state_history_tools(tools, self.states.clone(), registry);
    }

    /// Keep a spoken turn, away from the turn: it is spoken without
    /// waiting on the database.
    pub(crate) fn record_command(&self, entry: CommandEntry) {
        let commands = self.commands.clone();
        tokio::spawn(async move {
            if let Err(e) = commands.record(&entry).await {
                tracing::warn!("[history] a spoken turn was not kept: {e}");
            }
        });
    }
}

/// The histories, when there is a database, and a task that lets go
/// of what is older than [`KEPT_DAYS`]. Without a database none is
/// kept, and the tools that read them are not offered.
pub(crate) fn build_history(cfg: &Config) -> Option<History> {
    let Some(db) = super::stores::database(cfg) else {
        tracing::info!("[history] no database; nothing is kept of how the house was");
        return None;
    };
    let history = Arc::new(niles_db::PostgresHistory::new(
        db.pool(),
        db.describe_target(),
    ));
    let pruned = history.clone();
    tokio::spawn(async move {
        loop {
            let before = Utc::now() - chrono::Duration::days(KEPT_DAYS);
            if let Err(e) = StateHistory::forget_before(&*pruned, before).await {
                tracing::warn!("[history] old device states not let go: {e}");
            }
            if let Err(e) = CommandHistory::forget_before(&*pruned, before).await {
                tracing::warn!("[history] old spoken turns not let go: {e}");
            }
            tokio::time::sleep(Duration::from_secs(60 * 60)).await;
        }
    });
    Some(History {
        states: history.clone(),
        commands: history,
    })
}

/// Where the event loop sends each device's whole state after a change:
/// a task of its own keeps it, so a slow database never holds up the
/// loop that the lights answer to.
pub(crate) fn spawn_state_recorder(
    states: Arc<dyn StateHistory>,
) -> tokio::sync::mpsc::UnboundedSender<StateEntry> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(record_states(states, rx));
    tx
}

/// Keep each state that differs from the last one kept for its device:
/// devices report the same state again and again, and a reading that
/// did not move is not a change.
async fn record_states(
    states: Arc<dyn StateHistory>,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<StateEntry>,
) {
    let mut last: HashMap<DeviceId, DeviceState> = HashMap::new();
    while let Some(entry) = rx.recv().await {
        if last.get(&entry.device_id) == Some(&entry.state) {
            continue;
        }
        match states.record(&entry).await {
            Ok(()) => {
                last.insert(entry.device_id, entry.state);
            }
            Err(e) => tracing::warn!("[history] {} not kept: {e}", entry.device_id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use niles_history::{MemoryStateHistory, StateQuery};

    fn entry(minute: u32, brightness: u8) -> StateEntry {
        use chrono::TimeZone;
        StateEntry {
            ts: Utc.with_ymd_and_hms(2026, 10, 30, 20, minute, 0).unwrap(),
            device_id: DeviceId::parse("z2m:living_room/table").unwrap(),
            state: DeviceState {
                on: Some(true),
                brightness: Some(brightness),
                ..Default::default()
            },
        }
    }

    #[tokio::test]
    async fn a_state_reported_again_unchanged_is_kept_once() {
        let states = Arc::new(MemoryStateHistory::new());
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        for e in [entry(0, 40), entry(1, 40), entry(2, 20), entry(3, 40)] {
            tx.send(e).unwrap();
        }
        drop(tx);
        record_states(states.clone(), rx).await;
        let kept = states.query(&StateQuery::default()).await.unwrap();
        let brightness: Vec<_> = kept.iter().rev().map(|e| e.state.brightness).collect();
        assert_eq!(brightness, [Some(40), Some(20), Some(40)]);
    }
}
