//! The audio behind a wake, kept for training a better wake word.
//!
//! A satellite woke fifty-three times in one day and meant it twice.
//! Every one of those was a recording of the room — the television, a
//! word that sounded enough like "nyles" — streamed to Niles,
//! transcribed, and thrown away. They are the exact distribution the
//! wake word is failing on, which makes them worth more as training
//! negatives than any number of synthesised ones.
//!
//! Postgres rather than a volume, for the reason voice prints are:
//! the container has a read-only root and one emptyDir, and an
//! emptyDir loses everything on the next deploy. A week of a real
//! house is a few tens of megabytes, which is small enough that a
//! table is the cheap answer and a PVC is not.
//!
//! Off by default and capped, because this is a microphone writing
//! down a living room.

use sqlx::{Row, postgres::PgPool};

const SCHEMA: &str = "
create table if not exists wake_captures (
    id          bigserial primary key,
    heard_at    timestamptz not null default now(),
    transcript  text not null,
    outcome     text not null,
    sample_rate integer not null,
    wav         bytea not null
);
create index if not exists wake_captures_heard_at on wake_captures (heard_at);
";

/// One kept recording, without its audio.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Capture {
    pub id: i64,
    pub heard_at: chrono::DateTime<chrono::Utc>,
    /// What Whisper made of it. Empty when it made nothing.
    pub transcript: String,
    /// What Niles did with it — `acted`, `dropped: …`, `escalated`.
    /// The label a training set needs, written at the moment it is
    /// known rather than guessed at afterwards.
    pub outcome: String,
    pub bytes: i64,
}

pub struct PostgresCaptures {
    pool: PgPool,
    describe: String,
    schema: tokio::sync::OnceCell<()>,
}

impl PostgresCaptures {
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
                    .map_err(|e| format!("{}: {e}", self.describe))
            })
            .await
            .copied()
    }

    /// Keep one, and evict the oldest beyond `keep`.
    ///
    /// Trimmed on write rather than on a timer: the cap exists so a
    /// microphone cannot quietly fill a database, and a cap enforced
    /// somewhere else is a cap that is not enforced while that
    /// somewhere else is not running.
    pub async fn keep(
        &self,
        transcript: &str,
        outcome: &str,
        sample_rate: u32,
        wav: &[u8],
        keep: i64,
    ) -> Result<(), String> {
        self.ensure_schema().await?;
        sqlx::query(
            "insert into wake_captures (transcript, outcome, sample_rate, wav) \
             values ($1, $2, $3, $4)",
        )
        .bind(transcript)
        .bind(outcome)
        .bind(sample_rate as i32)
        .bind(wav)
        .execute(&self.pool)
        .await
        .map_err(|e| format!("keeping a capture: {e}"))?;

        sqlx::query(
            "delete from wake_captures where id not in \
             (select id from wake_captures order by heard_at desc limit $1)",
        )
        .bind(keep)
        .execute(&self.pool)
        .await
        .map_err(|e| format!("trimming captures: {e}"))?;
        Ok(())
    }

    /// Everything kept, newest first, without the audio.
    pub async fn list(&self) -> Result<Vec<Capture>, String> {
        self.ensure_schema().await?;
        let rows = sqlx::query(
            "select id, heard_at, transcript, outcome, octet_length(wav) as bytes \
             from wake_captures order by heard_at desc",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| format!("listing captures: {e}"))?;

        rows.iter()
            .map(|r| {
                Ok(Capture {
                    id: r.try_get("id").map_err(|e| e.to_string())?,
                    heard_at: r.try_get("heard_at").map_err(|e| e.to_string())?,
                    transcript: r.try_get("transcript").map_err(|e| e.to_string())?,
                    outcome: r.try_get("outcome").map_err(|e| e.to_string())?,
                    bytes: r.try_get::<i32, _>("bytes").map_err(|e| e.to_string())? as i64,
                })
            })
            .collect()
    }

    /// One recording's audio.
    pub async fn wav(&self, id: i64) -> Result<Option<Vec<u8>>, String> {
        self.ensure_schema().await?;
        let row = sqlx::query("select wav from wake_captures where id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| format!("reading a capture: {e}"))?;
        row.map(|r| r.try_get("wav").map_err(|e| e.to_string()))
            .transpose()
    }

    /// Throw them all away.
    pub async fn clear(&self) -> Result<(), String> {
        self.ensure_schema().await?;
        sqlx::query("delete from wake_captures")
            .execute(&self.pool)
            .await
            .map_err(|e| format!("clearing captures: {e}"))?;
        Ok(())
    }
}
