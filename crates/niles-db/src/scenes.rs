//! Saved scenes, in Postgres.
//!
//! They used to live in a JSON file under `[persistence].directory` —
//! which has no default, so a Niles without that section kept its
//! scenes in memory and lost every one of them on each restart. In a
//! cluster that is every deploy, silently.
//!
//! So they go where the credentials and the config overrides already
//! go: the database, which needs nothing in the config file beyond the
//! line that names it.
//!
//! One row holding the whole document. There are a handful of scenes
//! and they are written together; a row per scene would buy nothing and
//! cost a second way for the set to be wrong.

use sqlx::{Row, postgres::PgPool};

const SCHEMA: &str = "
create table if not exists scenes (
    id         smallint primary key default 1 check (id = 1),
    document   text not null,
    updated_at timestamptz not null default now()
);
";

pub struct PostgresScenes {
    pool: PgPool,
    describe: String,
    schema: tokio::sync::OnceCell<()>,
}

impl PostgresScenes {
    /// Share a pool with whatever else is already talking to this
    /// database — there is one Postgres, and one pool is enough.
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

    /// Errors name the database but never the DSN — the connection
    /// string carries a password.
    fn storage(&self, e: impl std::fmt::Display) -> String {
        format!("scene storage ({}): {e}", self.describe)
    }

    /// The stored document, or `None` when nothing has been saved yet.
    pub async fn load(&self) -> Result<Option<String>, String> {
        self.ensure_schema().await?;
        let row = sqlx::query("select document from scenes where id = 1")
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| self.storage(e))?;
        Ok(row.map(|r| r.get::<String, _>("document")))
    }

    /// Replace it.
    pub async fn store(&self, document: &str) -> Result<(), String> {
        self.ensure_schema().await?;
        sqlx::query(
            "insert into scenes (id, document, updated_at) values (1, $1, now())
             on conflict (id) do update set document = excluded.document, updated_at = now()",
        )
        .bind(document)
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(|e| self.storage(e))
    }
}
