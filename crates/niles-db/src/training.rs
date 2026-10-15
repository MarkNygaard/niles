//! Each person's workouts, and their Strava. See `niles_training` for
//! what is kept and why.
//!
//! Saving a workout is an upsert that writes nothing when nothing
//! changed: the `where ... is distinct from` on the conflict is what
//! lets a save say how many were new.
//!
//! A person's Strava app secret and tokens are standing keys to their
//! activities, so they are sealed with the same key as the credentials
//! typed into the app (`Sealer`), and a database without that key keeps
//! no Strava at all.

use crate::secrets::Sealer;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use niles_training::{
    Activity, Athlete, Error, Kind, Overview, Result, Saved, StravaApp, StravaLink, StravaTokens,
    TrainingStore,
};
use sqlx::{Row, postgres::PgPool, postgres::PgRow};

const SCHEMA: &str = "
create table if not exists training_activities (
    id               bigserial primary key,
    person           text not null,
    source           text not null,
    external_id      text not null,
    started_at       timestamptz not null,
    kind             text not null,
    name             text not null,
    distance_m       double precision,
    duration_s       double precision,
    moving_s         double precision,
    avg_hr           double precision,
    max_hr           double precision,
    elevation_gain_m double precision,
    energy_kcal      double precision,
    raw              jsonb not null,
    updated_at       timestamptz not null default now(),
    unique (person, source, external_id)
);
create index if not exists training_activities_person
    on training_activities (person, started_at);
create table if not exists training_strava (
    person            text primary key,
    client_id         text not null,
    client_secret_enc bytea not null,
    access_token_enc  bytea,
    refresh_token_enc bytea,
    expires_at        timestamptz,
    athlete_id        bigint,
    athlete_name      text,
    last_sync         timestamptz,
    last_error        text,
    updated_at        timestamptz not null default now()
);
-- The iPhone route's tables, from before Strava. Dropped rather than
-- left behind: one held sleep, HRV and weight that nothing reads any
-- more and nobody could delete from the app.
drop table if exists training_keys;
drop table if exists training_wellness;
";

pub struct PostgresTraining {
    pool: PgPool,
    describe: String,
    sealer: Sealer,
    schema: tokio::sync::OnceCell<()>,
}

impl PostgresTraining {
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

    fn seal(&self, value: &str) -> Result<Vec<u8>> {
        self.sealer.seal(value).map_err(|e| self.storage(e))
    }

    fn open(&self, sealed: &[u8]) -> Result<String> {
        self.sealer.open(sealed).map_err(|e| self.storage(e))
    }
}

fn activity(row: &PgRow) -> Activity {
    Activity {
        source: row.get("source"),
        external_id: row.get("external_id"),
        started_at: row.get("started_at"),
        kind: Kind::parse(row.get::<String, _>("kind").as_str()).unwrap_or(Kind::Other),
        name: row.get("name"),
        distance_m: row.get("distance_m"),
        duration_s: row.get("duration_s"),
        moving_s: row.get("moving_s"),
        avg_hr: row.get("avg_hr"),
        max_hr: row.get("max_hr"),
        elevation_gain_m: row.get("elevation_gain_m"),
        energy_kcal: row.get("energy_kcal"),
        raw: serde_json::from_str(&row.get::<String, _>("raw")).unwrap_or_default(),
    }
}

#[async_trait]
impl TrainingStore for PostgresTraining {
    async fn save_activities(&self, person: &str, activities: &[Activity]) -> Result<Saved> {
        self.ensure_schema().await?;
        let mut saved = Saved::default();
        let mut tx = self.pool.begin().await.map_err(|e| self.storage(e))?;
        for a in activities {
            // `xmax = 0` is how Postgres tells an insert from an update
            // in `returning`; no row back means nothing changed.
            let row = sqlx::query(
                "insert into training_activities
                     (person, source, external_id, started_at, kind, name, distance_m,
                      duration_s, moving_s, avg_hr, max_hr, elevation_gain_m, energy_kcal, raw)
                 values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14::jsonb)
                 on conflict (person, source, external_id) do update set
                     started_at = excluded.started_at, kind = excluded.kind,
                     name = excluded.name, distance_m = excluded.distance_m,
                     duration_s = excluded.duration_s, moving_s = excluded.moving_s,
                     avg_hr = excluded.avg_hr, max_hr = excluded.max_hr,
                     elevation_gain_m = excluded.elevation_gain_m,
                     energy_kcal = excluded.energy_kcal, raw = excluded.raw,
                     updated_at = now()
                 where (training_activities.started_at, training_activities.kind,
                        training_activities.name, training_activities.distance_m,
                        training_activities.duration_s, training_activities.moving_s,
                        training_activities.avg_hr, training_activities.max_hr,
                        training_activities.elevation_gain_m,
                        training_activities.energy_kcal, training_activities.raw)
                   is distinct from
                       (excluded.started_at, excluded.kind, excluded.name,
                        excluded.distance_m, excluded.duration_s, excluded.moving_s,
                        excluded.avg_hr, excluded.max_hr, excluded.elevation_gain_m,
                        excluded.energy_kcal, excluded.raw)
                 returning (xmax = 0) as inserted",
            )
            .bind(person)
            .bind(&a.source)
            .bind(&a.external_id)
            .bind(a.started_at)
            .bind(a.kind.as_str())
            .bind(&a.name)
            .bind(a.distance_m)
            .bind(a.duration_s)
            .bind(a.moving_s)
            .bind(a.avg_hr)
            .bind(a.max_hr)
            .bind(a.elevation_gain_m)
            .bind(a.energy_kcal)
            .bind(a.raw.to_string())
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| self.storage(e))?;
            match row.map(|r| r.get::<bool, _>("inserted")) {
                Some(true) => saved.added += 1,
                Some(false) => saved.updated += 1,
                None => {}
            }
        }
        tx.commit().await.map_err(|e| self.storage(e))?;
        Ok(saved)
    }

    async fn activities(
        &self,
        person: &str,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
        kind: Option<Kind>,
    ) -> Result<Vec<Activity>> {
        self.ensure_schema().await?;
        let rows = sqlx::query(
            "select source, external_id, started_at, kind, name, distance_m, duration_s,
                    moving_s, avg_hr, max_hr, elevation_gain_m, energy_kcal, raw::text as raw
             from training_activities
             where person = $1
               and ($2::timestamptz is null or started_at >= $2)
               and ($3::timestamptz is null or started_at < $3)
               and ($4::text is null or kind = $4)
             order by started_at",
        )
        .bind(person)
        .bind(from)
        .bind(to)
        .bind(kind.map(Kind::as_str))
        .fetch_all(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(rows.iter().map(activity).collect())
    }

    async fn overview(&self, person: &str) -> Result<Overview> {
        self.ensure_schema().await?;
        let row = sqlx::query(
            "select count(*) filter (where kind = 'run') as runs, count(*) as activities
             from training_activities where person = $1",
        )
        .bind(person)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(Overview {
            runs: row.get::<i64, _>("runs").max(0) as u64,
            activities: row.get::<i64, _>("activities").max(0) as u64,
        })
    }

    async fn latest(&self, person: &str, source: &str) -> Result<Option<DateTime<Utc>>> {
        self.ensure_schema().await?;
        sqlx::query_scalar(
            "select max(started_at) from training_activities where person = $1 and source = $2",
        )
        .bind(person)
        .bind(source)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| self.storage(e))
    }

    async fn strava(&self, person: &str) -> Result<Option<StravaLink>> {
        self.ensure_schema().await?;
        let Some(row) = sqlx::query(
            "select client_id, client_secret_enc, access_token_enc, refresh_token_enc,
                    expires_at, athlete_id, athlete_name, last_sync, last_error
             from training_strava where person = $1",
        )
        .bind(person)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| self.storage(e))?
        else {
            return Ok(None);
        };
        // A row whose secret will not open — the key changed under it —
        // reads as an error rather than as no app: saving the app again
        // is the way out, and the card says so.
        let app = StravaApp {
            client_id: row.get("client_id"),
            client_secret: self.open(&row.get::<Vec<u8>, _>("client_secret_enc"))?,
        };
        let access: Option<Vec<u8>> = row.get("access_token_enc");
        let refresh: Option<Vec<u8>> = row.get("refresh_token_enc");
        let expires_at: Option<DateTime<Utc>> = row.get("expires_at");
        let tokens = match (access, refresh, expires_at) {
            (Some(access), Some(refresh), Some(expires_at)) => Some(StravaTokens {
                access_token: self.open(&access)?,
                refresh_token: self.open(&refresh)?,
                expires_at,
            }),
            _ => None,
        };
        let athlete_id: Option<i64> = row.get("athlete_id");
        let athlete_name: Option<String> = row.get("athlete_name");
        Ok(Some(StravaLink {
            app,
            tokens,
            athlete: athlete_id.map(|id| Athlete {
                id,
                name: athlete_name.unwrap_or_default(),
            }),
            last_sync: row.get("last_sync"),
            last_error: row.get("last_error"),
        }))
    }

    async fn strava_connected(&self) -> Result<Vec<String>> {
        self.ensure_schema().await?;
        sqlx::query_scalar(
            "select person from training_strava where refresh_token_enc is not null
             order by person",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| self.storage(e))
    }

    async fn set_strava_app(&self, person: &str, app: &StravaApp) -> Result<()> {
        self.ensure_schema().await?;
        let sealed = self.seal(&app.client_secret)?;
        sqlx::query(
            "insert into training_strava (person, client_id, client_secret_enc)
             values ($1, $2, $3)
             on conflict (person) do update set client_id = excluded.client_id,
                 client_secret_enc = excluded.client_secret_enc, updated_at = now()",
        )
        .bind(person)
        .bind(&app.client_id)
        .bind(sealed)
        .execute(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(())
    }

    async fn remove_strava(&self, person: &str) -> Result<bool> {
        self.ensure_schema().await?;
        let done = sqlx::query("delete from training_strava where person = $1")
            .bind(person)
            .execute(&self.pool)
            .await
            .map_err(|e| self.storage(e))?;
        Ok(done.rows_affected() == 1)
    }

    async fn set_strava_tokens(
        &self,
        person: &str,
        tokens: &StravaTokens,
        athlete: Option<&Athlete>,
    ) -> Result<()> {
        self.ensure_schema().await?;
        sqlx::query(
            "update training_strava set access_token_enc = $2, refresh_token_enc = $3,
                 expires_at = $4,
                 athlete_id = coalesce($5, athlete_id),
                 athlete_name = coalesce($6, athlete_name),
                 updated_at = now()
             where person = $1",
        )
        .bind(person)
        .bind(self.seal(&tokens.access_token)?)
        .bind(self.seal(&tokens.refresh_token)?)
        .bind(tokens.expires_at)
        .bind(athlete.map(|a| a.id))
        .bind(athlete.map(|a| a.name.as_str()))
        .execute(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(())
    }

    async fn clear_strava_tokens(&self, person: &str) -> Result<()> {
        self.ensure_schema().await?;
        sqlx::query(
            "update training_strava set access_token_enc = null, refresh_token_enc = null,
                 expires_at = null, athlete_id = null, athlete_name = null, updated_at = now()
             where person = $1",
        )
        .bind(person)
        .execute(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(())
    }

    async fn strava_synced(
        &self,
        person: &str,
        at: Option<DateTime<Utc>>,
        error: Option<&str>,
    ) -> Result<()> {
        self.ensure_schema().await?;
        sqlx::query(
            "update training_strava set last_sync = coalesce($2, last_sync), last_error = $3
             where person = $1",
        )
        .bind(person)
        .bind(at)
        .bind(error)
        .execute(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(())
    }
}
