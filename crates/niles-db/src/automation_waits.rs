//! Automation runs waiting out a delay, in Postgres, so a wait outlives
//! a restart: "seven hours after, wake me" still wakes you after a
//! deploy in the night.
//!
//! One row per rule, since a rule runs once at a time. The document is
//! the engine's, kept as it hands it over.

use sqlx::{Row, postgres::PgPool};

const SCHEMA: &str = "
create table if not exists automation_waits (
    rule       text primary key,
    document   text not null,
    updated_at timestamptz not null default now()
);
";

pub struct PostgresAutomationWaits {
    pool: PgPool,
    describe: String,
    schema: tokio::sync::OnceCell<()>,
}

impl PostgresAutomationWaits {
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
        format!("automation wait storage ({}): {e}", self.describe)
    }

    /// Keep `rule`'s wait, in place of any it had.
    pub async fn keep(&self, rule: &str, document: &str) -> Result<(), String> {
        self.ensure_schema().await?;
        sqlx::query(
            "insert into automation_waits (rule, document, updated_at) values ($1, $2, now())
             on conflict (rule) do update set document = excluded.document, updated_at = now()",
        )
        .bind(rule)
        .bind(document)
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(|e| self.storage(e))
    }

    pub async fn forget(&self, rule: &str) -> Result<(), String> {
        self.ensure_schema().await?;
        sqlx::query("delete from automation_waits where rule = $1")
            .bind(rule)
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(|e| self.storage(e))
    }

    /// Every wait kept.
    pub async fn all(&self) -> Result<Vec<String>, String> {
        self.ensure_schema().await?;
        let rows = sqlx::query("select document from automation_waits")
            .fetch_all(&self.pool)
            .await
            .map_err(|e| self.storage(e))?;
        Ok(rows.into_iter().map(|r| r.get("document")).collect())
    }
}
