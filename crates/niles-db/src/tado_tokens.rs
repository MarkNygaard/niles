//! The tado refresh token, in Postgres.
//!
//! It is not configuration and it does not belong in a Secret: tado
//! issues a new one every time the old one is used, so whatever holds
//! it has to be writable by Niles several times an hour. It goes beside
//! the config overrides and the enrolled voices, in the database that
//! is already there and already survives a failover.
//!
//! One row, replaced in place. There is one tado account.

use async_trait::async_trait;
use niles_presence::TokenStore;
use niles_presence::error::{Error, Result};
use sqlx::{Row, postgres::PgPool};

const SCHEMA: &str = "
create table if not exists tado_tokens (
    id            smallint primary key default 1 check (id = 1),
    refresh_token text not null,
    updated_at    timestamptz not null default now()
);
";

pub struct PostgresTadoTokens {
    pool: PgPool,
    describe: String,
    schema: tokio::sync::OnceCell<()>,
}

impl PostgresTadoTokens {
    /// Share a pool with whatever else is already talking to this
    /// database — there is one Postgres, and one pool is enough.
    pub fn new(pool: PgPool, describe: String) -> Self {
        Self {
            pool,
            describe,
            schema: tokio::sync::OnceCell::new(),
        }
    }

    async fn ensure_schema(&self) -> Result<()> {
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
    fn storage(&self, e: impl std::fmt::Display) -> Error {
        Error::Parse {
            reason: format!("tado token storage ({}): {e}", self.describe),
        }
    }
}

#[async_trait]
impl TokenStore for PostgresTadoTokens {
    async fn load(&self) -> Result<Option<String>> {
        self.ensure_schema().await?;
        let row = sqlx::query("select refresh_token from tado_tokens where id = 1")
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| self.storage(e))?;
        row.map(|r| r.try_get("refresh_token").map_err(|e| self.storage(e)))
            .transpose()
    }

    async fn save(&self, refresh_token: &str) -> Result<()> {
        self.ensure_schema().await?;
        // Whole-row replacement, so a retry after a connection dies
        // mid-write cannot apply anything twice — the same reason the
        // config backend replaces rather than appends.
        sqlx::query(
            "insert into tado_tokens (id, refresh_token, updated_at)
             values (1, $1, now())
             on conflict (id) do update
             set refresh_token = excluded.refresh_token,
                 updated_at = excluded.updated_at",
        )
        .bind(refresh_token)
        .execute(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(())
    }
}
