//! The device state and command histories, so "how were the lights at
//! eight yesterday" outlives a restart.
//!
//! A device's state is kept as the JSON `DeviceState` serialises to, as
//! text: this crate stores it and hands it back, and what a state holds
//! is `niles-core`'s business. Its room is a column of its own, so a
//! room's history is found without reading every state.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use niles_core::DeviceId;
use niles_history::{
    CommandEntry, CommandHistory, CommandQuery, Error, Result, StateEntry, StateHistory, StateQuery,
};
use sqlx::Row;
use sqlx::postgres::{PgPool, PgRow};

const SCHEMA: &str = "
create table if not exists device_state_history (
    id        bigserial primary key,
    at        timestamptz not null,
    device_id text not null,
    room      text not null,
    state     text not null
);
create index if not exists device_state_history_device on device_state_history (device_id, at desc);
create index if not exists device_state_history_room on device_state_history (room, at desc);
create index if not exists device_state_history_at on device_state_history (at);

create table if not exists command_history (
    id              bigserial primary key,
    at              timestamptz not null,
    peer            text not null,
    origin_room     text,
    transcript      text not null,
    spoken_response text,
    speaker         text
);
create index if not exists command_history_at on command_history (at desc);
";

/// Both histories, on one pool.
pub struct PostgresHistory {
    pool: PgPool,
    describe: String,
    schema: tokio::sync::OnceCell<()>,
}

impl PostgresHistory {
    pub fn new(pool: PgPool, describe: String) -> Self {
        Self {
            pool,
            describe,
            schema: tokio::sync::OnceCell::new(),
        }
    }

    async fn ensure_schema(&self, kind: &'static str) -> Result<()> {
        self.schema
            .get_or_try_init(|| async {
                sqlx::raw_sql(SCHEMA)
                    .execute(&self.pool)
                    .await
                    .map(|_| ())
                    .map_err(|e| self.storage(kind, e))
            })
            .await
            .copied()
    }

    /// Errors name the database but never the DSN, which carries a
    /// password.
    fn storage(&self, kind: &'static str, e: impl std::fmt::Display) -> Error {
        Error::Storage {
            kind,
            reason: format!("{}: {e}", self.describe),
        }
    }
}

const STATE: &str = "device state";
const COMMAND: &str = "command";

fn state_entry(this: &PostgresHistory, row: &PgRow) -> Result<StateEntry> {
    let id: String = row.get("device_id");
    let state: String = row.get("state");
    Ok(StateEntry {
        ts: row.get("at"),
        device_id: DeviceId::parse(&id).map_err(|e| this.storage(STATE, e))?,
        state: serde_json::from_str(&state).map_err(|e| this.storage(STATE, e))?,
    })
}

fn command_entry(this: &PostgresHistory, row: &PgRow) -> Result<CommandEntry> {
    let peer: String = row.get("peer");
    Ok(CommandEntry {
        ts: row.get("at"),
        peer: peer.parse().map_err(|e| this.storage(COMMAND, e))?,
        origin_room: row.get("origin_room"),
        transcript: row.get("transcript"),
        spoken_response: row.get("spoken_response"),
        speaker: row.get("speaker"),
    })
}

/// How many rows a query may return, as Postgres takes it.
fn limit(n: usize) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

#[async_trait]
impl StateHistory for PostgresHistory {
    async fn record(&self, entry: &StateEntry) -> Result<()> {
        self.ensure_schema(STATE).await?;
        let state = serde_json::to_string(&entry.state).map_err(|e| self.storage(STATE, e))?;
        sqlx::query(
            "insert into device_state_history (at, device_id, room, state) values ($1, $2, $3, $4)",
        )
        .bind(entry.ts)
        .bind(entry.device_id.to_string())
        .bind(entry.device_id.room().as_str())
        .bind(state)
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(|e| self.storage(STATE, e))
    }

    async fn query(&self, filter: &StateQuery) -> Result<Vec<StateEntry>> {
        filter.check()?;
        self.ensure_schema(STATE).await?;
        let rows = sqlx::query(
            "select at, device_id, state from device_state_history
             where ($1::timestamptz is null or at >= $1)
               and ($2::timestamptz is null or at <= $2)
               and ($3::text is null or device_id = $3)
               and ($3::text is not null or $4::text is null or room = $4)
             order by at desc, id desc
             limit $5",
        )
        .bind(filter.since)
        .bind(filter.until)
        .bind(filter.device_id.as_ref().map(ToString::to_string))
        .bind(filter.room.as_deref())
        .bind(limit(filter.limit()))
        .fetch_all(&self.pool)
        .await
        .map_err(|e| self.storage(STATE, e))?;
        rows.iter().map(|r| state_entry(self, r)).collect()
    }

    async fn snapshot_at(&self, at: DateTime<Utc>, ids: &[DeviceId]) -> Result<Vec<StateEntry>> {
        self.ensure_schema(STATE).await?;
        let ids: Vec<String> = ids.iter().map(ToString::to_string).collect();
        let rows = sqlx::query(
            "select distinct on (device_id) at, device_id, state from device_state_history
             where device_id = any($1) and at <= $2
             order by device_id, at desc, id desc",
        )
        .bind(&ids)
        .bind(at)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| self.storage(STATE, e))?;
        rows.iter().map(|r| state_entry(self, r)).collect()
    }

    async fn earliest(&self) -> Result<Option<DateTime<Utc>>> {
        self.ensure_schema(STATE).await?;
        sqlx::query_scalar("select min(at) from device_state_history")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| self.storage(STATE, e))
    }

    async fn forget_before(&self, before: DateTime<Utc>) -> Result<()> {
        self.ensure_schema(STATE).await?;
        sqlx::query("delete from device_state_history where at < $1")
            .bind(before)
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(|e| self.storage(STATE, e))
    }
}

#[async_trait]
impl CommandHistory for PostgresHistory {
    async fn record(&self, entry: &CommandEntry) -> Result<()> {
        self.ensure_schema(COMMAND).await?;
        sqlx::query(
            "insert into command_history
                 (at, peer, origin_room, transcript, spoken_response, speaker)
             values ($1, $2, $3, $4, $5, $6)",
        )
        .bind(entry.ts)
        .bind(entry.peer.to_string())
        .bind(entry.origin_room.as_deref())
        .bind(&entry.transcript)
        .bind(entry.spoken_response.as_deref())
        .bind(entry.speaker.as_deref())
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(|e| self.storage(COMMAND, e))
    }

    async fn query(&self, filter: &CommandQuery) -> Result<Vec<CommandEntry>> {
        filter.check()?;
        self.ensure_schema(COMMAND).await?;
        let rows = sqlx::query(
            "select at, peer, origin_room, transcript, spoken_response, speaker
             from command_history
             where ($1::timestamptz is null or at >= $1)
               and ($2::timestamptz is null or at <= $2)
               and ($3::text is null or origin_room = $3)
             order by at desc, id desc
             limit $4",
        )
        .bind(filter.since)
        .bind(filter.until)
        .bind(filter.room.as_deref())
        .bind(limit(filter.limit()))
        .fetch_all(&self.pool)
        .await
        .map_err(|e| self.storage(COMMAND, e))?;
        rows.iter().map(|r| command_entry(self, r)).collect()
    }

    async fn earliest(&self) -> Result<Option<DateTime<Utc>>> {
        self.ensure_schema(COMMAND).await?;
        sqlx::query_scalar("select min(at) from command_history")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| self.storage(COMMAND, e))
    }

    async fn forget_before(&self, before: DateTime<Utc>) -> Result<()> {
        self.ensure_schema(COMMAND).await?;
        sqlx::query("delete from command_history where at < $1")
            .bind(before)
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(|e| self.storage(COMMAND, e))
    }
}
