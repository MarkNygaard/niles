//! Each person's schedules and their runs.
//!
//! `schedules` holds what My profile lists, `when` as JSON; `schedule_runs`
//! each run, going with its schedule (`on delete cascade`). Nothing here
//! is sealed: a schedule's instruction is the person's own words, kept as
//! their chat is.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use niles_schedules::{
    Error, NewSchedule, Outcome, RUNS_SHOWN, Result, Run, Schedule, ScheduleStore, When,
};
use sqlx::{Row, postgres::PgPool, postgres::PgRow};

const SCHEMA: &str = "
create table if not exists schedules (
    id bigserial primary key, person text not null, name text not null,
    instruction text not null, \"when\" jsonb not null,
    paused boolean not null default false, next_due timestamptz,
    created_at timestamptz not null default now());
create index if not exists schedules_person on schedules (person);
create index if not exists schedules_due on schedules (next_due) where not paused;
create table if not exists schedule_runs (
    id bigserial primary key,
    schedule_id bigint not null references schedules (id) on delete cascade,
    due timestamptz not null, finished timestamptz, outcome text not null,
    conversation bigint, reason text);
create index if not exists schedule_runs_schedule on schedule_runs (schedule_id, id desc);
alter table schedules add column if not exists acts_on bigint;
";

pub struct PostgresSchedules {
    pool: PgPool,
    describe: String,
    schema: tokio::sync::OnceCell<()>,
}

impl PostgresSchedules {
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

    /// Names the database, never the DSN, which carries a password.
    fn storage(&self, e: impl std::fmt::Display) -> Error {
        Error::Storage(format!("{}: {e}", self.describe))
    }

    fn schedule(&self, row: &PgRow) -> Result<Schedule> {
        let when: sqlx::types::Json<When> = row.try_get("when").map_err(|e| self.storage(e))?;
        Ok(Schedule {
            id: row.get("id"),
            name: row.get("name"),
            instruction: row.get("instruction"),
            when: when.0,
            paused: row.get("paused"),
            next_due: row.get("next_due"),
            acts_on: row.get("acts_on"),
        })
    }
}

const COLUMNS: &str = "id, person, name, instruction, \"when\", paused, next_due, acts_on";

#[async_trait]
impl ScheduleStore for PostgresSchedules {
    async fn list(&self, person: &str) -> Result<Vec<Schedule>> {
        self.ensure_schema().await?;
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
            "select {COLUMNS} from schedules where person = $1 order by lower(name), id"
        )))
        .bind(person)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        rows.iter().map(|r| self.schedule(r)).collect()
    }

    async fn get(&self, person: &str, id: i64) -> Result<Option<Schedule>> {
        self.ensure_schema().await?;
        let row = sqlx::query(sqlx::AssertSqlSafe(format!(
            "select {COLUMNS} from schedules where person = $1 and id = $2"
        )))
        .bind(person)
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        row.as_ref().map(|r| self.schedule(r)).transpose()
    }

    async fn add(
        &self,
        person: &str,
        new: &NewSchedule,
        next: Option<DateTime<Utc>>,
    ) -> Result<Schedule> {
        self.ensure_schema().await?;
        let row = sqlx::query(sqlx::AssertSqlSafe(format!(
            "insert into schedules (person, name, instruction, \"when\", next_due, acts_on)
             values ($1, $2, $3, $4, $5, $6) returning {COLUMNS}"
        )))
        .bind(person)
        .bind(&new.name)
        .bind(&new.instruction)
        .bind(sqlx::types::Json(&new.when))
        .bind(next)
        .bind(new.acts_on)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        self.schedule(&row)
    }

    async fn update(
        &self,
        person: &str,
        id: i64,
        new: &NewSchedule,
        next: Option<DateTime<Utc>>,
    ) -> Result<Option<Schedule>> {
        self.ensure_schema().await?;
        let row = sqlx::query(sqlx::AssertSqlSafe(format!(
            "update schedules set name = $3, instruction = $4, \"when\" = $5,
                 next_due = case when paused then null else $6 end, acts_on = $7
             where person = $1 and id = $2 returning {COLUMNS}"
        )))
        .bind(person)
        .bind(id)
        .bind(&new.name)
        .bind(&new.instruction)
        .bind(sqlx::types::Json(&new.when))
        .bind(next)
        .bind(new.acts_on)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        row.as_ref().map(|r| self.schedule(r)).transpose()
    }

    async fn remove(&self, person: &str, id: i64) -> Result<bool> {
        self.ensure_schema().await?;
        let done = sqlx::query("delete from schedules where person = $1 and id = $2")
            .bind(person)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| self.storage(e))?;
        Ok(done.rows_affected() == 1)
    }

    async fn set_paused(
        &self,
        person: &str,
        id: i64,
        paused: bool,
        next: Option<DateTime<Utc>>,
    ) -> Result<bool> {
        self.ensure_schema().await?;
        let done = sqlx::query(
            "update schedules set paused = $3, next_due = case when $3 then null else $4 end
             where person = $1 and id = $2",
        )
        .bind(person)
        .bind(id)
        .bind(paused)
        .bind(next)
        .execute(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(done.rows_affected() == 1)
    }

    async fn due(&self, now: DateTime<Utc>) -> Result<Vec<(String, Schedule)>> {
        self.ensure_schema().await?;
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
            "select {COLUMNS} from schedules where not paused and next_due <= $1 order by next_due"
        )))
        .bind(now)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        rows.iter()
            .map(|r| Ok((r.get::<String, _>("person"), self.schedule(r)?)))
            .collect()
    }

    async fn set_next(&self, id: i64, next: Option<DateTime<Utc>>) -> Result<()> {
        self.ensure_schema().await?;
        sqlx::query("update schedules set next_due = $2 where id = $1")
            .bind(id)
            .bind(next)
            .execute(&self.pool)
            .await
            .map_err(|e| self.storage(e))?;
        Ok(())
    }

    async fn start_run(
        &self,
        person: &str,
        id: i64,
        due: DateTime<Utc>,
        outcome: Outcome,
    ) -> Result<Option<i64>> {
        self.ensure_schema().await?;
        sqlx::query_scalar(
            "insert into schedule_runs (schedule_id, due, outcome, finished)
             select id, $3, $4, case when $4 = 'running' then null else now() end
             from schedules where person = $1 and id = $2
             returning id",
        )
        .bind(person)
        .bind(id)
        .bind(due)
        .bind(outcome.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| self.storage(e))
    }

    async fn finish_run(
        &self,
        run: i64,
        outcome: Outcome,
        conversation: Option<i64>,
        reason: Option<&str>,
    ) -> Result<()> {
        self.ensure_schema().await?;
        sqlx::query(
            "update schedule_runs set outcome = $2, conversation = $3, reason = $4, finished = now()
             where id = $1",
        )
        .bind(run)
        .bind(outcome.as_str())
        .bind(conversation)
        .bind(reason)
        .execute(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(())
    }

    async fn runs(&self, person: &str, id: i64) -> Result<Option<Vec<Run>>> {
        self.ensure_schema().await?;
        if self.get(person, id).await?.is_none() {
            return Ok(None);
        }
        let rows = sqlx::query(
            "select id, due, finished, outcome, conversation, reason from schedule_runs
             where schedule_id = $1 order by id desc limit $2",
        )
        .bind(id)
        .bind(RUNS_SHOWN as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(Some(
            rows.iter()
                .map(|row| Run {
                    id: row.get("id"),
                    due: row.get("due"),
                    finished: row.get("finished"),
                    outcome: Outcome::parse(row.get::<&str, _>("outcome")),
                    conversation: row.get("conversation"),
                    reason: row.get("reason"),
                })
                .collect(),
        ))
    }

    async fn abandon_running(&self, reason: &str) -> Result<()> {
        self.ensure_schema().await?;
        sqlx::query(
            "update schedule_runs set outcome = 'failed', finished = now(), reason = $1
             where outcome = 'running'",
        )
        .bind(reason)
        .execute(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(())
    }
}

impl std::fmt::Debug for PostgresSchedules {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostgresSchedules")
            .field("database", &self.describe)
            .finish()
    }
}
