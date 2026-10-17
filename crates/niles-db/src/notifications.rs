//! What Niles said unprompted, so "what did you say?" outlives a
//! restart.
//!
//! One row per notification, each the JSON document the binary wrote:
//! this crate does not need to know what a notification holds. The
//! newest few hundred are kept; the centre only ever reads back its own
//! ring's worth.

use chrono::{DateTime, Utc};
use sqlx::{Row, postgres::PgPool};

const SCHEMA: &str = "
create table if not exists notifications (
    id         text primary key,
    created_at timestamptz not null,
    document   text not null
);
create index if not exists notifications_newest on notifications (created_at desc);
";

/// How many are kept. Older ones go as new ones come.
pub const NOTIFICATIONS_KEPT: i64 = 500;

pub struct PostgresNotifications {
    pool: PgPool,
    describe: String,
    schema: tokio::sync::OnceCell<()>,
}

impl PostgresNotifications {
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

    /// Errors name the database but never the DSN, which carries a
    /// password.
    fn storage(&self, e: impl std::fmt::Display) -> String {
        format!("notification history ({}): {e}", self.describe)
    }

    /// Keep one, and let the oldest beyond [`NOTIFICATIONS_KEPT`] go.
    pub async fn keep(
        &self,
        id: &str,
        created_at: DateTime<Utc>,
        document: &str,
    ) -> Result<(), String> {
        self.ensure_schema().await?;
        sqlx::query(
            "insert into notifications (id, created_at, document) values ($1, $2, $3)
             on conflict (id) do nothing",
        )
        .bind(id)
        .bind(created_at)
        .bind(document)
        .execute(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        sqlx::query(
            "delete from notifications where id not in (
                 select id from notifications order by created_at desc limit $1)",
        )
        .bind(NOTIFICATIONS_KEPT)
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(|e| self.storage(e))
    }

    /// The newest `limit`, newest first.
    pub async fn recent(&self, limit: i64) -> Result<Vec<String>, String> {
        self.ensure_schema().await?;
        let rows = sqlx::query(
            "select document from notifications order by created_at desc, id desc limit $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(rows
            .iter()
            .map(|r| r.get::<String, _>("document"))
            .collect())
    }
}
