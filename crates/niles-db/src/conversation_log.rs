//! Every conversation with Niles, spoken or typed, kept for the person
//! who had it — so the app's chat can show what was said and what was
//! said yes to, and pick a conversation up again.
//!
//! One row per conversation; its turns are a JSON array appended to in
//! place, so a turn is one statement rather than a rewrite. What a turn
//! holds is the binary's business: here it is opaque JSON.

use chrono::{DateTime, Utc};
use sqlx::{Row, postgres::PgPool};

const SCHEMA: &str = "
create table if not exists conversation_log (
    id         bigserial primary key,
    person     text not null,
    started_at timestamptz not null default now(),
    last_at    timestamptz not null default now(),
    title      text not null,
    spoken     boolean not null default false,
    turns      jsonb not null default '[]'
);
create index if not exists conversation_log_person on conversation_log (person, last_at desc);
";

/// How many conversations one person keeps. Older ones go as new ones
/// start.
pub const KEPT_PER_PERSON: i64 = 100;

/// A conversation as the list shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct ConversationRow {
    pub id: i64,
    pub title: String,
    pub started_at: DateTime<Utc>,
    pub last_at: DateTime<Utc>,
    pub turns: usize,
    pub spoken: bool,
}

pub struct PostgresConversationLog {
    pool: PgPool,
    describe: String,
    schema: tokio::sync::OnceCell<()>,
}

impl PostgresConversationLog {
    pub fn new(pool: PgPool, describe: String) -> Self {
        Self {
            pool,
            describe,
            schema: tokio::sync::OnceCell::new(),
        }
    }

    async fn ensure_schema(&self) -> Result<(), String> {
        self.schema
            .get_or_try_init(|| async {
                sqlx::raw_sql(SCHEMA)
                    .execute(&self.pool)
                    .await
                    .map(|_| ())
                    .map_err(|e| self.storage(e))
            })
            .await
            .copied()
    }

    fn storage(&self, e: impl std::fmt::Display) -> String {
        format!("conversation storage ({}): {e}", self.describe)
    }

    /// The person's conversation still going: one with a turn in the
    /// last `gap_seconds`.
    pub async fn active(&self, person: &str, gap_seconds: i64) -> Result<Option<i64>, String> {
        self.ensure_schema().await?;
        let row = sqlx::query(
            "select id from conversation_log
             where person = $1 and last_at > now() - make_interval(secs => $2)
             order by last_at desc limit 1",
        )
        .bind(person)
        .bind(gap_seconds as f64)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(row.map(|r| r.get::<i64, _>("id")))
    }

    /// Add `turn` (JSON) to conversation `into`, or start a new one
    /// titled `title`. Answers which conversation it went into; `None`
    /// when `into` is not one of this person's.
    pub async fn append(
        &self,
        person: &str,
        into: Option<i64>,
        title: &str,
        turn: &str,
        spoken: bool,
    ) -> Result<Option<i64>, String> {
        self.ensure_schema().await?;
        if let Some(id) = into {
            let updated = sqlx::query(
                "update conversation_log
                 set turns = turns || jsonb_build_array($3::jsonb), last_at = now(),
                     spoken = spoken or $4
                 where id = $1 and person = $2",
            )
            .bind(id)
            .bind(person)
            .bind(turn)
            .bind(spoken)
            .execute(&self.pool)
            .await
            .map_err(|e| self.storage(e))?;
            return Ok((updated.rows_affected() == 1).then_some(id));
        }
        let id: i64 = sqlx::query(
            "insert into conversation_log (person, title, spoken, turns)
             values ($1, $2, $3, jsonb_build_array($4::jsonb)) returning id",
        )
        .bind(person)
        .bind(title)
        .bind(spoken)
        .bind(turn)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| self.storage(e))?
        .get("id");
        sqlx::query(
            "delete from conversation_log where person = $1 and id not in (
                 select id from conversation_log where person = $1
                 order by last_at desc limit $2)",
        )
        .bind(person)
        .bind(KEPT_PER_PERSON)
        .execute(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(Some(id))
    }

    /// The person's conversations, newest first.
    pub async fn list(&self, person: &str) -> Result<Vec<ConversationRow>, String> {
        self.ensure_schema().await?;
        let rows = sqlx::query(
            "select id, title, started_at, last_at, jsonb_array_length(turns) as n, spoken
             from conversation_log where person = $1 order by last_at desc",
        )
        .bind(person)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(rows
            .into_iter()
            .map(|r| ConversationRow {
                id: r.get("id"),
                title: r.get("title"),
                started_at: r.get("started_at"),
                last_at: r.get("last_at"),
                turns: r.get::<i32, _>("n").max(0) as usize,
                spoken: r.get("spoken"),
            })
            .collect())
    }

    /// One of the person's conversations, with its turns as JSON text.
    pub async fn get(
        &self,
        person: &str,
        id: i64,
    ) -> Result<Option<(ConversationRow, String)>, String> {
        self.ensure_schema().await?;
        let row = sqlx::query(
            "select id, title, started_at, last_at, jsonb_array_length(turns) as n, spoken,
                    turns::text as turns
             from conversation_log where id = $1 and person = $2",
        )
        .bind(id)
        .bind(person)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(row.map(|r| {
            (
                ConversationRow {
                    id: r.get("id"),
                    title: r.get("title"),
                    started_at: r.get("started_at"),
                    last_at: r.get("last_at"),
                    turns: r.get::<i32, _>("n").max(0) as usize,
                    spoken: r.get("spoken"),
                },
                r.get::<String, _>("turns"),
            )
        }))
    }

    /// Forget one of the person's conversations. False when there was
    /// none of theirs by that id.
    pub async fn delete(&self, person: &str, id: i64) -> Result<bool, String> {
        self.ensure_schema().await?;
        let deleted = sqlx::query("delete from conversation_log where id = $1 and person = $2")
            .bind(id)
            .bind(person)
            .execute(&self.pool)
            .await
            .map_err(|e| self.storage(e))?;
        Ok(deleted.rows_affected() == 1)
    }
}
