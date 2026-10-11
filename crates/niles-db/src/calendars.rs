//! Each person's calendar connections, in Postgres.
//!
//! The refresh token is a standing key to somebody's calendar, so it is
//! sealed with the same key as the credentials typed into the app, and
//! a database without that key keeps no calendars at all.

use crate::secrets::Sealer;
use async_trait::async_trait;
use niles_calendar::{Access, Connection, ConnectionStore, Error, Result};
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
            "select id, person, name, account, access, refresh_token
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
            let refresh_token = match self.sealer.open(&sealed) {
                Ok(token) => token,
                // The key changed under it. Left out with a word in the
                // log: the person sees it gone, and connects again.
                Err(e) => {
                    tracing::error!("[calendars] {id}: the stored token would not open ({e})");
                    continue;
                }
            };
            let access: String = row.try_get("access").map_err(|e| self.storage(e))?;
            out.push(Connection {
                id,
                person: row.try_get("person").map_err(|e| self.storage(e))?,
                name: row.try_get("name").map_err(|e| self.storage(e))?,
                account: row.try_get("account").map_err(|e| self.storage(e))?,
                access: Access::parse(&access).unwrap_or(Access::Read),
                refresh_token,
            });
        }
        Ok(out)
    }

    async fn save(&self, connection: &Connection) -> Result<()> {
        self.ensure_schema().await?;
        let sealed = self.seal(&connection.refresh_token)?;
        sqlx::query(
            "insert into calendar_connections (id, person, name, account, access, refresh_token)
             values ($1, $2, $3, $4, $5, $6)
             on conflict (id) do update set
                 name = excluded.name, account = excluded.account, access = excluded.access,
                 refresh_token = excluded.refresh_token, updated_at = now()
             where calendar_connections.person = excluded.person",
        )
        .bind(&connection.id)
        .bind(&connection.person)
        .bind(&connection.name)
        .bind(&connection.account)
        .bind(connection.access.as_str())
        .bind(sealed)
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(|e| self.storage(e))
    }

    async fn update_token(&self, id: &str, refresh_token: &str) -> Result<()> {
        self.ensure_schema().await?;
        let sealed = self.seal(refresh_token)?;
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
