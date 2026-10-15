//! Each person's workouts and days, and the key their phone sends them
//! with. See `niles_training` for what is kept and why.
//!
//! Saving is an upsert that writes nothing when nothing changed: the
//! `where ... is distinct from` on each conflict is what makes Health
//! Auto Export's habit of re-sending the last week every hour cost a
//! read rather than a write, and what lets a save say how many were new.

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use niles_training::{
    Activity, Error, Kind, Overview, Result, Saved, TrainingStore, Wellness, same_hash,
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
create table if not exists training_wellness (
    person       text not null,
    date         date not null,
    resting_hr   double precision,
    hrv_ms       double precision,
    sleep_s      double precision,
    weight_kg    double precision,
    body_fat_pct double precision,
    updated_at   timestamptz not null default now(),
    primary key (person, date)
);
create table if not exists training_keys (
    person           text primary key,
    key_hash         text unique,
    made_at          timestamptz,
    last_received_at timestamptz
);
";

pub struct PostgresTraining {
    pool: PgPool,
    describe: String,
    schema: tokio::sync::OnceCell<()>,
}

impl PostgresTraining {
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
            .map(|_| ())
    }

    fn storage(&self, e: impl std::fmt::Display) -> Error {
        Error::Storage(format!("{}: {e}", self.describe))
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

    async fn save_wellness(&self, person: &str, days: &[Wellness]) -> Result<Saved> {
        self.ensure_schema().await?;
        let mut saved = Saved::default();
        let mut tx = self.pool.begin().await.map_err(|e| self.storage(e))?;
        for d in days.iter().filter(|d| !d.is_empty()) {
            let row = sqlx::query(
                "insert into training_wellness
                     (person, date, resting_hr, hrv_ms, sleep_s, weight_kg, body_fat_pct)
                 values ($1, $2, $3, $4, $5, $6, $7)
                 on conflict (person, date) do update set
                     resting_hr = coalesce(excluded.resting_hr, training_wellness.resting_hr),
                     hrv_ms = coalesce(excluded.hrv_ms, training_wellness.hrv_ms),
                     sleep_s = coalesce(excluded.sleep_s, training_wellness.sleep_s),
                     weight_kg = coalesce(excluded.weight_kg, training_wellness.weight_kg),
                     body_fat_pct = coalesce(excluded.body_fat_pct, training_wellness.body_fat_pct),
                     updated_at = now()
                 where (training_wellness.resting_hr, training_wellness.hrv_ms,
                        training_wellness.sleep_s, training_wellness.weight_kg,
                        training_wellness.body_fat_pct)
                   is distinct from
                       (coalesce(excluded.resting_hr, training_wellness.resting_hr),
                        coalesce(excluded.hrv_ms, training_wellness.hrv_ms),
                        coalesce(excluded.sleep_s, training_wellness.sleep_s),
                        coalesce(excluded.weight_kg, training_wellness.weight_kg),
                        coalesce(excluded.body_fat_pct, training_wellness.body_fat_pct))
                 returning (xmax = 0) as inserted",
            )
            .bind(person)
            .bind(d.date)
            .bind(d.resting_hr)
            .bind(d.hrv_ms)
            .bind(d.sleep_s)
            .bind(d.weight_kg)
            .bind(d.body_fat_pct)
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

    async fn wellness(
        &self,
        person: &str,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<Wellness>> {
        self.ensure_schema().await?;
        let rows = sqlx::query(
            "select date, resting_hr, hrv_ms, sleep_s, weight_kg, body_fat_pct
             from training_wellness
             where person = $1 and date >= $2 and date <= $3
             order by date",
        )
        .bind(person)
        .bind(from)
        .bind(to)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(rows
            .iter()
            .map(|r| Wellness {
                date: r.get("date"),
                resting_hr: r.get("resting_hr"),
                hrv_ms: r.get("hrv_ms"),
                sleep_s: r.get("sleep_s"),
                weight_kg: r.get("weight_kg"),
                body_fat_pct: r.get("body_fat_pct"),
            })
            .collect())
    }

    async fn overview(&self, person: &str) -> Result<Overview> {
        self.ensure_schema().await?;
        let row = sqlx::query(
            "select
                 (select made_at from training_keys
                   where person = $1 and key_hash is not null) as made_at,
                 (select last_received_at from training_keys where person = $1) as received,
                 (select count(*) from training_activities
                   where person = $1 and kind = 'run') as runs,
                 (select count(*) from training_activities where person = $1) as activities,
                 (select count(*) from training_wellness where person = $1) as days",
        )
        .bind(person)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(Overview {
            key_made_at: row.get("made_at"),
            last_received_at: row.get("received"),
            runs: row.get::<i64, _>("runs").max(0) as u64,
            activities: row.get::<i64, _>("activities").max(0) as u64,
            wellness_days: row.get::<i64, _>("days").max(0) as u64,
        })
    }

    async fn set_key(&self, person: &str, hash: &str) -> Result<()> {
        self.ensure_schema().await?;
        sqlx::query(
            "insert into training_keys (person, key_hash, made_at) values ($1, $2, now())
             on conflict (person) do update set key_hash = excluded.key_hash,
                 made_at = excluded.made_at",
        )
        .bind(person)
        .bind(hash)
        .execute(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(())
    }

    async fn revoke_key(&self, person: &str) -> Result<bool> {
        self.ensure_schema().await?;
        let done = sqlx::query(
            "update training_keys set key_hash = null, made_at = null
             where person = $1 and key_hash is not null",
        )
        .bind(person)
        .execute(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(done.rows_affected() == 1)
    }

    async fn person_for_key(&self, hash: &str) -> Result<Option<String>> {
        self.ensure_schema().await?;
        let row = sqlx::query("select person, key_hash from training_keys where key_hash = $1")
            .bind(hash)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| self.storage(e))?;
        // The index found it; the comparison that decides is ours, and
        // does not stop at the first byte that differs.
        Ok(row
            .filter(|r| same_hash(&r.get::<String, _>("key_hash"), hash))
            .map(|r| r.get("person")))
    }

    async fn received(&self, person: &str, at: DateTime<Utc>) -> Result<()> {
        self.ensure_schema().await?;
        sqlx::query(
            "insert into training_keys (person, last_received_at) values ($1, $2)
             on conflict (person) do update set last_received_at = excluded.last_received_at",
        )
        .bind(person)
        .bind(at)
        .execute(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(())
    }
}
