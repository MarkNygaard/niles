//! Each person's calendar connections, in Postgres.
//!
//! The credential — a refresh token, or iCloud's app-specific password —
//! is a standing key to somebody's calendar, so it is sealed with the
//! same key as the credentials typed into the app, and a database
//! without that key keeps no calendars at all.

use crate::secrets::Sealer;
use async_trait::async_trait;
use niles_calendar::{Access, Connection, ConnectionStore, Error, Provider, Result};
use sqlx::{Row, postgres::PgPool};

const SCHEMA: &str = "
create table if not exists calendar_connections (
    id            text primary key,
    person        text not null,
    name          text not null,
    account       text not null,
    access        text not null,
    refresh_token bytea not null,
    created_at    timestamptz not null default now(),
    updated_at    timestamptz not null default now()
);
create index if not exists calendar_connections_person on calendar_connections (person);
-- Added with Google and iCloud. The column keeps its first name: it
-- holds a refresh token, or iCloud's password.
alter table calendar_connections add column if not exists provider text not null default 'microsoft';
alter table calendar_connections add column if not exists endpoint text;
";

pub struct PostgresCalendars {
    pool: PgPool,
    describe: String,
    sealer: Sealer,
    schema: tokio::sync::OnceCell<()>,
}

impl PostgresCalendars {
    /// `key_b64` is the secrets key: 32 bytes, base64.
    pub fn new(pool: PgPool, describe: String, key_b64: &str) -> niles_config::Result<Self> {
        Ok(Self {
            pool,
            describe,
            sealer: Sealer::new(key_b64)?,
            schema: tokio::sync::OnceCell::new(),
        })
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
        Error::Storage {
            reason: format!("{}: {e}", self.describe),
        }
    }

    fn seal(&self, token: &str) -> Result<Vec<u8>> {
        self.sealer.seal(token).map_err(|e| self.storage(e))
    }
}

#[async_trait]
impl ConnectionStore for PostgresCalendars {
    async fn list(&self, person: &str) -> Result<Vec<Connection>> {
        self.ensure_schema().await?;
        let rows = sqlx::query(
            "select id, person, provider, name, account, access, refresh_token, endpoint
             from calendar_connections where person = $1 order by created_at",
        )
        .bind(person)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let id: String = row.try_get("id").map_err(|e| self.storage(e))?;
            let sealed: Vec<u8> = row.try_get("refresh_token").map_err(|e| self.storage(e))?;
            let credential = match self.sealer.open(&sealed) {
                Ok(credential) => credential,
                // The key changed under it. Left out with a word in the
                // log: the person sees it gone, and connects again.
                Err(e) => {
                    tracing::error!("[calendars] {id}: the stored token would not open ({e})");
                    continue;
                }
            };
            let access: String = row.try_get("access").map_err(|e| self.storage(e))?;
            let provider: String = row.try_get("provider").map_err(|e| self.storage(e))?;
            let Some(provider) = Provider::parse(&provider) else {
                tracing::error!("[calendars] {id}: unknown provider {provider:?}");
                continue;
            };
            out.push(Connection {
                id,
                person: row.try_get("person").map_err(|e| self.storage(e))?,
                provider,
                name: row.try_get("name").map_err(|e| self.storage(e))?,
                account: row.try_get("account").map_err(|e| self.storage(e))?,
                access: Access::parse(&access).unwrap_or(Access::Read),
                credential,
                endpoint: row.try_get("endpoint").map_err(|e| self.storage(e))?,
            });
        }
        Ok(out)
    }

    async fn save(&self, connection: &Connection) -> Result<()> {
        self.ensure_schema().await?;
        let sealed = self.seal(&connection.credential)?;
        sqlx::query(
            "insert into calendar_connections
                 (id, person, provider, name, account, access, refresh_token, endpoint)
             values ($1, $2, $3, $4, $5, $6, $7, $8)
             on conflict (id) do update set
                 provider = excluded.provider, name = excluded.name,
                 account = excluded.account, access = excluded.access,
                 refresh_token = excluded.refresh_token, endpoint = excluded.endpoint,
                 updated_at = now()
             where calendar_connections.person = excluded.person",
        )
        .bind(&connection.id)
        .bind(&connection.person)
        .bind(connection.provider.as_str())
        .bind(&connection.name)
        .bind(&connection.account)
        .bind(connection.access.as_str())
        .bind(sealed)
        .bind(&connection.endpoint)
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(|e| self.storage(e))
    }

    async fn update_credential(&self, id: &str, credential: &str) -> Result<()> {
        self.ensure_schema().await?;
        let sealed = self.seal(credential)?;
        sqlx::query(
            "update calendar_connections set refresh_token = $2, updated_at = now() where id = $1",
        )
        .bind(id)
        .bind(sealed)
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(|e| self.storage(e))
    }

    async fn delete(&self, person: &str, id: &str) -> Result<bool> {
        self.ensure_schema().await?;
        sqlx::query("delete from calendar_connections where person = $1 and id = $2")
            .bind(person)
            .bind(id)
            .execute(&self.pool)
            .await
            .map(|done| done.rows_affected() > 0)
            .map_err(|e| self.storage(e))
    }
}
