//! The household's automations, in Postgres.
//!
//! One row holding the whole ordered set, as scenes and the shopping
//! list are kept: there are a handful, they are read together, and the
//! order is part of the record — the later of two conflicting rules
//! wins. The document is the JSON the app sends, which is also what the
//! LLM writes.

use sqlx::{Row, postgres::PgPool};

const SCHEMA: &str = "
create table if not exists automations (
    id         smallint primary key default 1 check (id = 1),
    document   text not null,
    updated_at timestamptz not null default now()
);
";

pub struct PostgresAutomations {
    pool: PgPool,
    describe: String,
    schema: tokio::sync::OnceCell<()>,
}

impl PostgresAutomations {
    /// Share a pool with whatever else is already talking to this
    /// database — there is one Postgres, and one pool is enough.
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

    /// Errors name the database but never the DSN — the connection
    /// string carries a password.
    fn storage(&self, e: impl std::fmt::Display) -> String {
        format!("automation storage ({}): {e}", self.describe)
    }

    /// The stored document, or `None` when nothing has been saved yet.
    pub async fn load(&self) -> Result<Option<String>, String> {
        self.ensure_schema().await?;
        let row = sqlx::query("select document from automations where id = 1")
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| self.storage(e))?;
        Ok(row.map(|r| r.get::<String, _>("document")))
    }

    /// Replace it.
    pub async fn store(&self, document: &str) -> Result<(), String> {
        self.ensure_schema().await?;
        sqlx::query(
            "insert into automations (id, document, updated_at) values (1, $1, now())
             on conflict (id) do update set document = excluded.document, updated_at = now()",
        )
        .bind(document)
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(|e| self.storage(e))
    }
}
