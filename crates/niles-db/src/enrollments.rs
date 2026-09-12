//! Enrolled voices in Postgres.
//!
//! A voice print is 192 floats — under a kilobyte per clip, a few
//! kilobytes for a household. That is far too little to justify a
//! volume, and a volume is the thing we can least afford: a node-local
//! one pins the pod to a node and blocks drains, and an NFS one makes
//! voice recognition depend on a NAS being up.
//!
//! So they live beside the config overrides, in the database that is
//! already there and already survives a failover.

use async_trait::async_trait;
use niles_recognition::error::{Error, Result};
use niles_recognition::{
    EnrolledSpeaker, EnrollmentBackend, EnrollmentEntry, default_display_name,
    validate_speaker_slug,
};
use sqlx::postgres::{PgPool, PgRow};
use sqlx::{Row, types::Json};

/// One row per speaker, embeddings included.
///
/// The clips are a `jsonb` array rather than their own table: they are
/// only ever read and written as a whole speaker, and nothing queries
/// inside them. A join would buy nothing and cost a migration.
const SCHEMA: &str = "
create table if not exists enrolled_speakers (
    speaker       text primary key,
    display_name  text not null,
    created_at    timestamptz not null,
    last_seen_at  timestamptz,
    embeddings    jsonb not null
);
";

pub struct PostgresEnrollments {
    pool: PgPool,
    describe: String,
    schema: tokio::sync::OnceCell<()>,
}

impl PostgresEnrollments {
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
                    .map_err(storage)
            })
            .await
            .copied()
    }
}

fn storage(e: impl std::fmt::Display) -> Error {
    Error::Storage(e.to_string())
}

fn row_to_speaker(row: &PgRow) -> Result<EnrolledSpeaker> {
    let embeddings: Json<Vec<EnrollmentEntry>> = row.try_get("embeddings").map_err(storage)?;
    let embeddings = embeddings.0;
    Ok(EnrolledSpeaker {
        speaker: row.try_get("speaker").map_err(storage)?,
        display_name: row.try_get("display_name").map_err(storage)?,
        created_at: row.try_get("created_at").map_err(storage)?,
        last_seen_at: row.try_get("last_seen_at").map_err(storage)?,
        clip_count: embeddings.len(),
        embeddings,
    })
}

#[async_trait]
impl EnrollmentBackend for PostgresEnrollments {
    async fn load_all(&self) -> Result<Vec<EnrolledSpeaker>> {
        self.ensure_schema().await?;
        let rows = sqlx::query("select * from enrolled_speakers order by speaker")
            .fetch_all(&self.pool)
            .await
            .map_err(storage)?;
        rows.iter().map(row_to_speaker).collect()
    }

    async fn enroll(&self, speaker: &str, embedding: &[f32]) -> Result<()> {
        self.ensure_schema().await?;
        validate_speaker_slug(speaker)?;
        if embedding.len() != 192 {
            return Err(Error::WrongEmbeddingDim {
                actual: embedding.len(),
            });
        }
        // The file store normalises before writing and the matcher
        // assumes it. Skipping it here would not fail — it would just
        // quietly stop recognising people.
        let mut embedding = embedding.to_vec();
        niles_recognition::similarity::l2_normalize(&mut embedding);
        let entry = EnrollmentEntry {
            recorded_at: chrono::Utc::now(),
            embedding,
        };

        // Appending inside the statement keeps a second enrollment of
        // the same speaker from racing a read-modify-write.
        sqlx::query(
            "insert into enrolled_speakers \
             (speaker, display_name, created_at, last_seen_at, embeddings) \
             values ($1, $2, now(), null, $3) \
             on conflict (speaker) do update \
             set embeddings = enrolled_speakers.embeddings || excluded.embeddings",
        )
        .bind(speaker)
        .bind(default_display_name(speaker))
        .bind(Json(vec![entry]))
        .execute(&self.pool)
        .await
        .map_err(storage)?;
        Ok(())
    }

    async fn load(&self, speaker: &str) -> Result<EnrolledSpeaker> {
        self.ensure_schema().await?;
        let row = sqlx::query("select * from enrolled_speakers where speaker = $1")
            .bind(speaker)
            .fetch_optional(&self.pool)
            .await
            .map_err(storage)?
            .ok_or_else(|| Error::NotFound {
                speaker: speaker.to_string(),
            })?;
        row_to_speaker(&row)
    }

    async fn list(&self) -> Result<Vec<String>> {
        self.ensure_schema().await?;
        sqlx::query_scalar("select speaker from enrolled_speakers order by speaker")
            .fetch_all(&self.pool)
            .await
            .map_err(storage)
    }

    async fn delete(&self, speaker: &str) -> Result<()> {
        self.ensure_schema().await?;
        let done = sqlx::query("delete from enrolled_speakers where speaker = $1")
            .bind(speaker)
            .execute(&self.pool)
            .await
            .map_err(storage)?;
        if done.rows_affected() == 0 {
            return Err(Error::NotFound {
                speaker: speaker.to_string(),
            });
        }
        Ok(())
    }

    async fn bump_last_seen(&self, speaker: &str) -> Result<()> {
        self.ensure_schema().await?;
        sqlx::query("update enrolled_speakers set last_seen_at = now() where speaker = $1")
            .bind(speaker)
            .execute(&self.pool)
            .await
            .map_err(storage)?;
        Ok(())
    }

    fn describe(&self) -> String {
        self.describe.clone()
    }
}
