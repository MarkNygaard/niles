//! What Niles remembers, in Postgres.
//!
//! The two memory documents — facts about the household, and what Niles
//! has learned — used to be markdown files in `[memory].directory`. That
//! has no default, so a Niles without the section remembered nothing,
//! and said nothing about it.
//!
//! One row per document, keyed `user` or `agent` — and `forgotten`, the
//! notes taken out lately, as JSON, so they can be put back.

use sqlx::{Row, postgres::PgPool};

const SCHEMA: &str = "
create table if not exists memory (
    target     text primary key check (target in ('user', 'agent')),
    document   text not null,
    updated_at timestamptz not null default now()
);
alter table memory drop constraint if exists memory_target_check;
alter table memory add constraint memory_target_check
    check (target in ('user', 'agent', 'forgotten'));
";

pub struct PostgresMemory {
    pool: PgPool,
    describe: String,
    schema: tokio::sync::OnceCell<()>,
}

impl PostgresMemory {
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
        format!("memory storage ({}): {e}", self.describe)
    }

    /// The document for `target` (`user` or `agent`); empty when nothing
    /// has been kept yet.
    pub async fn load(&self, target: &str) -> Result<String, String> {
        self.ensure_schema().await?;
        let row = sqlx::query("select document from memory where target = $1")
            .bind(target)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| self.storage(e))?;
        Ok(row
            .map(|r| r.get::<String, _>("document"))
            .unwrap_or_default())
    }

    /// Replace it.
    pub async fn store(&self, target: &str, document: &str) -> Result<(), String> {
        self.ensure_schema().await?;
        sqlx::query(
            "insert into memory (target, document, updated_at) values ($1, $2, now())
             on conflict (target) do update set document = excluded.document, updated_at = now()",
        )
        .bind(target)
        .bind(document)
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(|e| self.storage(e))
    }
}
