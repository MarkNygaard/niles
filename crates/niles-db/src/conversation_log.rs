//! Every conversation with Niles, spoken or typed, kept for the person
//! who had it — so the app's chat can show what was said and what was
//! said yes to, and pick a conversation up again.
//!
//! One row per conversation; its turns are a JSON array appended to in
//! place, so a turn is one statement rather than a rewrite. What a turn
//! holds is the binary's business: here it is opaque JSON.
//!
//! The files on a message live in `chat_attachments`, created in the
//! same `ensure_schema` so one pass makes the conversation log and its
//! child table in order.

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
create table if not exists chat_attachments (
    id              bigserial primary key,
    person          text not null,
    conversation_id bigint references conversation_log (id) on delete cascade,
    turn            integer,
    kind            text not null,
    mime            text not null,
    name            text not null,
    bytes           bytea not null,
    width           integer,
    height          integer,
    created_at      timestamptz not null default now()
);
create index if not exists chat_attachments_conversation
    on chat_attachments (conversation_id);
create index if not exists chat_attachments_unbound
    on chat_attachments (created_at) where conversation_id is null;
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

/// One attachment as it is served: what it is, and the bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct AttachmentBytes {
    pub kind: String,
    pub mime: String,
    pub name: String,
    pub bytes: Vec<u8>,
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

    /// Replace one field of one turn: an approval card that has been
    /// answered says so where it was asked. False when there was no such
    /// conversation of theirs, or no such turn.
    pub async fn set_turn_field(
        &self,
        person: &str,
        id: i64,
        turn: usize,
        field: &str,
        value: &str,
    ) -> Result<bool, String> {
        self.ensure_schema().await?;
        let updated = sqlx::query(
            "update conversation_log
             set turns = jsonb_set(turns, array[$3::text, $4::text], $5::jsonb)
             where id = $1 and person = $2 and jsonb_array_length(turns) > $6",
        )
        .bind(id)
        .bind(person)
        .bind(turn.to_string())
        .bind(field)
        .bind(value)
        .bind(turn as i32)
        .execute(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(updated.rows_affected() == 1)
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

    /// Keep one file, owned by `person` from the moment it is stored.
    /// The turn it belongs to does not exist yet, so `conversation_id`
    /// and `turn` stay null until `bind` ties them. Answers the row id.
    pub async fn attach(
        &self,
        person: &str,
        kind: &str,
        mime: &str,
        name: &str,
        bytes: &[u8],
        dims: Option<(i32, i32)>,
    ) -> Result<i64, String> {
        self.ensure_schema().await?;
        sqlx::query(
            "delete from chat_attachments
             where conversation_id is null and created_at < now() - interval '1 day'",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        let row = sqlx::query(
            "insert into chat_attachments (person, kind, mime, name, bytes, width, height)
             values ($1, $2, $3, $4, $5, $6, $7) returning id",
        )
        .bind(person)
        .bind(kind)
        .bind(mime)
        .bind(name)
        .bind(bytes)
        .bind(dims.map(|(w, _)| w))
        .bind(dims.map(|(_, h)| h))
        .fetch_one(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(row.get("id"))
    }

    /// Tie stored files to the conversation's most recent turn — the
    /// one that was just appended. Files already bound, or another
    /// person's, are left alone.
    pub async fn bind(&self, person: &str, ids: &[i64], conversation: i64) -> Result<(), String> {
        self.ensure_schema().await?;
        sqlx::query(
            "update chat_attachments
             set conversation_id = $1,
                 turn = (select jsonb_array_length(turns) - 1
                           from conversation_log where id = $1)
             where id = any($2) and person = $3 and conversation_id is null",
        )
        .bind(conversation)
        .bind(ids)
        .bind(person)
        .execute(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(())
    }

    /// One attachment's bytes, when it is the person's, in that
    /// conversation. Anything else is "no such attachment".
    pub async fn attachment(
        &self,
        person: &str,
        conversation: i64,
        id: i64,
    ) -> Result<Option<AttachmentBytes>, String> {
        self.ensure_schema().await?;
        let row = sqlx::query(
            "select kind, mime, name, bytes from chat_attachments
             where id = $1 and conversation_id = $2 and person = $3",
        )
        .bind(id)
        .bind(conversation)
        .bind(person)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(row.map(|r| AttachmentBytes {
            kind: r.get("kind"),
            mime: r.get("mime"),
            name: r.get("name"),
            bytes: r.get("bytes"),
        }))
    }
}
