//! The server's Web Push key and each person's devices. See
//! `niles_webpush` for what they are for.
//!
//! The private key signs every push, so whoever holds it can send to
//! every device in the house: it is sealed with the secrets key
//! (`Sealer`), the same as the credentials typed into the app, and a
//! database without that key keeps no push at all.

use crate::secrets::Sealer;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use niles_webpush::{Error, NewSubscription, PushStore, Result, Subscription};
use sqlx::{Row, postgres::PgPool, postgres::PgRow};

const SCHEMA: &str = "
create table if not exists push_keys (
    id              smallint primary key default 1 check (id = 1),
    private_key_enc bytea not null,
    created_at      timestamptz not null default now()
);
create table if not exists push_subscriptions (
    endpoint   text primary key,
    person     text not null,
    p256dh     text not null,
    auth       text not null,
    device     text not null,
    origin     text not null,
    created_at timestamptz not null default now(),
    last_ok_at timestamptz
);
create index if not exists push_subscriptions_person on push_subscriptions (person);
";

pub struct PostgresPush {
    pool: PgPool,
    describe: String,
    sealer: Sealer,
    schema: tokio::sync::OnceCell<()>,
}

impl PostgresPush {
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
            .map(|_| ())
    }

    /// Errors name the database but never the DSN — the connection
    /// string carries a password.
    fn storage(&self, e: impl std::fmt::Display) -> Error {
        Error::Storage(format!("{}: {e}", self.describe))
    }
}

fn subscription(row: &PgRow) -> Subscription {
    Subscription {
        endpoint: row.get("endpoint"),
        p256dh: row.get("p256dh"),
        auth: row.get("auth"),
        device: row.get("device"),
        origin: row.get("origin"),
        created_at: row.get("created_at"),
        last_ok_at: row.get("last_ok_at"),
    }
}

#[async_trait]
impl PushStore for PostgresPush {
    async fn vapid_key(&self) -> Result<Option<String>> {
        self.ensure_schema().await?;
        let sealed: Option<Vec<u8>> =
            sqlx::query_scalar("select private_key_enc from push_keys where id = 1")
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| self.storage(e))?;
        sealed
            .map(|sealed| self.sealer.open(&sealed).map_err(|e| self.storage(e)))
            .transpose()
    }

    async fn keep_vapid_key(&self, key: &str) -> Result<String> {
        self.ensure_schema().await?;
        let sealed = self.sealer.seal(key).map_err(|e| self.storage(e))?;
        sqlx::query("insert into push_keys (id, private_key_enc) values (1, $1) on conflict (id) do nothing")
            .bind(sealed)
            .execute(&self.pool)
            .await
            .map_err(|e| self.storage(e))?;
        self.vapid_key()
            .await?
            .ok_or_else(|| self.storage("the push key was not kept"))
    }

    async fn subscribe(&self, person: &str, s: &NewSubscription) -> Result<()> {
        self.ensure_schema().await?;
        sqlx::query(
            "insert into push_subscriptions (endpoint, person, p256dh, auth, device, origin)
             values ($1, $2, $3, $4, $5, $6)
             on conflict (endpoint) do update set
                 person = excluded.person, p256dh = excluded.p256dh, auth = excluded.auth,
                 device = excluded.device, origin = excluded.origin",
        )
        .bind(&s.endpoint)
        .bind(person)
        .bind(&s.p256dh)
        .bind(&s.auth)
        .bind(&s.device)
        .bind(&s.origin)
        .execute(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(())
    }

    async fn unsubscribe(&self, person: &str, endpoint: &str) -> Result<bool> {
        self.ensure_schema().await?;
        let done =
            sqlx::query("delete from push_subscriptions where person = $1 and endpoint = $2")
                .bind(person)
                .bind(endpoint)
                .execute(&self.pool)
                .await
                .map_err(|e| self.storage(e))?;
        Ok(done.rows_affected() > 0)
    }

    async fn subscriptions(&self, person: &str) -> Result<Vec<Subscription>> {
        self.ensure_schema().await?;
        let rows = sqlx::query(
            "select endpoint, p256dh, auth, device, origin, created_at, last_ok_at
             from push_subscriptions where person = $1 order by created_at desc",
        )
        .bind(person)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(rows.iter().map(subscription).collect())
    }

    async fn delivered(&self, endpoint: &str, at: DateTime<Utc>) -> Result<()> {
        self.ensure_schema().await?;
        sqlx::query("update push_subscriptions set last_ok_at = $2 where endpoint = $1")
            .bind(endpoint)
            .bind(at)
            .execute(&self.pool)
            .await
            .map_err(|e| self.storage(e))?;
        Ok(())
    }

    async fn gone(&self, endpoint: &str) -> Result<()> {
        self.ensure_schema().await?;
        sqlx::query("delete from push_subscriptions where endpoint = $1")
            .bind(endpoint)
            .execute(&self.pool)
            .await
            .map_err(|e| self.storage(e))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_wrong_secrets_key_is_refused_at_construction() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://niles:pw@nowhere.invalid:5432/niles")
            .unwrap();
        assert!(PostgresPush::new(pool.clone(), "db".into(), "short").is_err());
        assert!(PostgresPush::new(pool, "db".into(), &Sealer::generate_key()).is_ok());
    }
}
