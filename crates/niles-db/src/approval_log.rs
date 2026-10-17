//! What each person was asked to approve, and what they decided: the
//! history under Approvals in Niles's panel and on My profile.
//!
//! A row is written when Niles starts holding a call, as `pending`, and
//! written again with the answer. The held call itself lives only in
//! the binary's memory, so a restart lapses it without anything there
//! to say so: the first use of this table after a start marks every
//! row still pending as lapsed. One Niles runs against a database, so
//! nothing pending then can still be waiting anywhere.
//!
//! What is kept is what the card showed, already cut and redacted by
//! the binary: a title, one summary line, never the call's arguments.

use chrono::{DateTime, Utc};
use sqlx::{Row, postgres::PgPool, postgres::PgRow};

const SCHEMA: &str = "
create table if not exists approval_log (
    id              bigserial primary key,
    approval        text not null unique,
    person          text not null,
    tool            text not null,
    icon            text,
    title           text not null,
    summary         text not null,
    status          text not null,
    decision        text,
    outcome         text,
    asked_via       text not null,
    decided_via     text,
    asked_at        timestamptz not null,
    decided_at      timestamptz,
    conversation_id bigint
);
create index if not exists approval_log_person on approval_log (person, id desc);
";

/// What a restart did to everything that was waiting.
const LAPSE_THE_LEFTOVERS: &str = "
update approval_log set status = 'lapsed', decided_at = now(), decided_via = null
where status = 'pending'
";

/// How many decisions one person keeps. Older ones go as new ones come.
pub const APPROVALS_KEPT_PER_PERSON: i64 = 500;

/// One approval, as the log keeps it.
#[derive(Debug, Clone, PartialEq)]
pub struct ApprovalEntry {
    /// The card's id.
    pub approval: String,
    pub person: String,
    /// The add-on it was for, as a person reads it: "Linear", "Mail".
    pub tool: String,
    pub icon: Option<String>,
    pub title: String,
    pub summary: String,
    /// `pending`, `allowed`, `denied`, `lapsed` or `failed`.
    pub status: String,
    /// `once`, `conversation` or `deny`; none for a lapse.
    pub decision: Option<String>,
    /// What became of it: "Done", the error.
    pub outcome: Option<String>,
    /// `app` or `voice`.
    pub asked_via: String,
    pub decided_via: Option<String>,
    pub asked_at: DateTime<Utc>,
    pub decided_at: Option<DateTime<Utc>>,
    pub conversation_id: Option<i64>,
}

/// An entry and its place in the log, which is what a page is cut by.
#[derive(Debug, Clone, PartialEq)]
pub struct ApprovalLogged {
    pub id: i64,
    pub entry: ApprovalEntry,
}

pub struct PostgresApprovalLog {
    pool: PgPool,
    describe: String,
    schema: tokio::sync::OnceCell<()>,
}

impl PostgresApprovalLog {
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
                    .map_err(|e| self.storage(e))?;
                sqlx::raw_sql(LAPSE_THE_LEFTOVERS)
                    .execute(&self.pool)
                    .await
                    .map(|_| ())
                    .map_err(|e| self.storage(e))
            })
            .await
            .copied()
    }

    fn storage(&self, e: impl std::fmt::Display) -> String {
        format!("approval log ({}): {e}", self.describe)
    }

    /// A call Niles has started holding. Written once: if the answer
    /// got here first, it stands.
    pub async fn asked(&self, entry: &ApprovalEntry) -> Result<(), String> {
        self.ensure_schema().await?;
        self.write(entry, "on conflict (approval) do nothing").await
    }

    /// What became of one, over whatever was written when it was asked.
    pub async fn settled(&self, entry: &ApprovalEntry) -> Result<(), String> {
        self.ensure_schema().await?;
        self.write(
            entry,
            "on conflict (approval) do update set
                 status = excluded.status, decision = excluded.decision,
                 outcome = excluded.outcome, decided_via = excluded.decided_via,
                 decided_at = excluded.decided_at,
                 conversation_id = coalesce(excluded.conversation_id, approval_log.conversation_id)",
        )
        .await?;
        sqlx::query(
            "delete from approval_log where person = $1 and id not in (
                 select id from approval_log where person = $1 order by id desc limit $2)",
        )
        .bind(&entry.person)
        .bind(APPROVALS_KEPT_PER_PERSON)
        .execute(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(())
    }

    async fn write(&self, e: &ApprovalEntry, conflict: &str) -> Result<(), String> {
        let sql = format!(
            "insert into approval_log
                 (approval, person, tool, icon, title, summary, status, decision, outcome,
                  asked_via, decided_via, asked_at, decided_at, conversation_id)
             values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
             {conflict}"
        );
        sqlx::query(&sql)
            .bind(&e.approval)
            .bind(&e.person)
            .bind(&e.tool)
            .bind(&e.icon)
            .bind(&e.title)
            .bind(&e.summary)
            .bind(&e.status)
            .bind(&e.decision)
            .bind(&e.outcome)
            .bind(&e.asked_via)
            .bind(&e.decided_via)
            .bind(e.asked_at)
            .bind(e.decided_at)
            .bind(e.conversation_id)
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(|e| self.storage(e))
    }

    /// The person's decided ones, newest first: at most `limit`, all
    /// before the entry `before` when it is given. Never anybody else's,
    /// and never one still waiting, which is not history yet.
    pub async fn page(
        &self,
        person: &str,
        before: Option<i64>,
        limit: i64,
    ) -> Result<Vec<ApprovalLogged>, String> {
        self.ensure_schema().await?;
        let rows = sqlx::query(
            "select id, approval, person, tool, icon, title, summary, status, decision, outcome,
                    asked_via, decided_via, asked_at, decided_at, conversation_id
             from approval_log
             where person = $1 and status <> 'pending' and ($2::bigint is null or id < $2)
             order by id desc limit $3",
        )
        .bind(person)
        .bind(before)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(rows.iter().map(logged).collect())
    }
}

fn logged(row: &PgRow) -> ApprovalLogged {
    ApprovalLogged {
        id: row.get("id"),
        entry: ApprovalEntry {
            approval: row.get("approval"),
            person: row.get("person"),
            tool: row.get("tool"),
            icon: row.get("icon"),
            title: row.get("title"),
            summary: row.get("summary"),
            status: row.get("status"),
            decision: row.get("decision"),
            outcome: row.get("outcome"),
            asked_via: row.get("asked_via"),
            decided_via: row.get("decided_via"),
            asked_at: row.get("asked_at"),
            decided_at: row.get("decided_at"),
            conversation_id: row.get("conversation_id"),
        },
    }
}
