//! niles-db — Postgres storage for Niles's configuration overrides.
//!
//! Implements [`OverrideBackend`] against the cluster's Postgres, so the
//! values changed away from the config file survive a restart without
//! pinning Niles to a node the way a local volume would.
//!
//! # What is stored
//!
//! Two small tables, both owned by Niles and created on first use:
//!
//! - `config_overrides` — a single row holding the override document.
//! - `config_revisions` — the capped change history that makes undo work.
//!
//! The document is stored as **TOML text**, not `jsonb`. JSONB would be
//! nicer to read in `psql`, but a TOML → JSON → TOML round-trip silently
//! degrades types: a TOML datetime comes back a string. No field is
//! affected today, which is exactly how that breaks quietly a year from
//! now when someone adds one. Text keeps this backend and the file
//! backend byte-identical in behaviour.
//!
//! # Failovers are normal here
//!
//! `postgres-rw` follows the primary through a switchover, which happens
//! on every node drain — so a dropped connection is an expected event,
//! not an incident. Two things make that survivable: the pool replaces
//! dead connections, and [`OverrideBackend::save`] is a whole-state
//! replacement, so a retry after a connection dies mid-write cannot apply
//! anything twice.

mod enrollments;
pub use enrollments::PostgresEnrollments;

use async_trait::async_trait;
use niles_config::{OverrideBackend, Revision, StoredState};
use sqlx::postgres::{PgPoolOptions, PgRow};
use sqlx::{Row, postgres::PgPool};

/// Idempotent schema bootstrap, run before the first read or write.
///
/// Niles owns its schema rather than depending on an external migration
/// job: the alternative couples every schema change to a separate deploy
/// that has to land first.
const SCHEMA: &str = "
create table if not exists config_overrides (
    id         smallint primary key default 1 check (id = 1),
    document   text not null,
    updated_at timestamptz not null default now()
);
create table if not exists config_revisions (
    id               bigint primary key,
    at               timestamptz not null,
    source           text not null,
    summary          text not null,
    changed_paths    text[] not null,
    overrides_before text not null
);
";

/// Config overrides kept in Postgres.
pub struct PostgresBackend {
    pool: PgPool,
    /// Host and database, with any credentials stripped. Used in logs and
    /// error messages, which is the whole reason it is kept separately —
    /// the DSN itself must never reach a log line.
    describe: String,
    /// Schema bootstrap, retried if it fails. `get_or_try_init` re-runs
    /// the initialiser after an error, so a database that was down at
    /// startup still gets its tables when it returns.
    schema: tokio::sync::OnceCell<()>,
}

impl PostgresBackend {
    /// Build a backend from a libpq-style DSN.
    ///
    /// Connects lazily: a database that is unreachable right now must not
    /// stop Niles from starting on its base config, and the pool will
    /// connect on first use instead.
    ///
    /// Must be called from within a Tokio runtime — the pool starts a
    /// background task to reap idle connections even before it has
    /// opened one.
    pub fn connect_lazy(dsn: &str, max_connections: u32) -> Result<Self, String> {
        let describe = redact(dsn);
        let pool = PgPoolOptions::new()
            .max_connections(max_connections)
            // A failover replaces the server under us. Without a bounded
            // acquire timeout, a write would hang on a dead pool for as
            // long as the switchover takes instead of failing and letting
            // the caller say so.
            .acquire_timeout(std::time::Duration::from_secs(10))
            .connect_lazy(dsn)
            .map_err(|e| format!("invalid database URL: {e}"))?;
        Ok(Self {
            pool,
            describe,
            schema: tokio::sync::OnceCell::new(),
        })
    }

    /// Where this backend points, with credentials stripped — safe to log.
    pub fn describe_target(&self) -> String {
        self.describe.clone()
    }

    /// The connection pool, so anything else that needs this database
    /// shares one rather than opening its own.
    pub fn pool(&self) -> sqlx::postgres::PgPool {
        self.pool.clone()
    }

    async fn ensure_schema(&self) -> Result<(), String> {
        self.schema
            .get_or_try_init(|| async {
                sqlx::raw_sql(SCHEMA)
                    .execute(&self.pool)
                    .await
                    .map(|_| ())
                    .map_err(|e| format!("could not create niles tables: {e}"))
            })
            .await
            .copied()
    }
}

#[async_trait]
impl OverrideBackend for PostgresBackend {
    async fn load(&self) -> Result<Option<StoredState>, String> {
        self.ensure_schema().await?;

        let document: Option<String> =
            sqlx::query_scalar("select document from config_overrides where id = 1")
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| format!("could not read config overrides: {e}"))?;

        // No row is not an error: it's a database nobody has written to.
        let Some(document) = document else {
            return Ok(None);
        };
        let overrides = toml::from_str(&document)
            .map_err(|e| format!("stored override document does not parse: {e}"))?;

        let revisions = sqlx::query(
            "select id, at, source, summary, changed_paths, overrides_before \
             from config_revisions order by id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| format!("could not read config history: {e}"))?;

        // A damaged journal costs history, not config: the override
        // document alone decides what is in force, so a revision that
        // won't parse is dropped rather than failing the load.
        let revisions = revisions.iter().filter_map(row_to_revision).collect();

        Ok(Some(StoredState {
            overrides,
            revisions,
        }))
    }

    async fn save(&self, state: &StoredState) -> Result<(), String> {
        self.ensure_schema().await?;

        let document = toml::to_string_pretty(&state.overrides)
            .map_err(|e| format!("could not serialize the override document: {e}"))?;

        // One transaction, so a reader never sees a journal that
        // disagrees with the document it accompanies.
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| format!("could not begin a transaction: {e}"))?;

        sqlx::query(
            "insert into config_overrides (id, document, updated_at) \
             values (1, $1, now()) \
             on conflict (id) do update set document = excluded.document, \
             updated_at = excluded.updated_at",
        )
        .bind(&document)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("could not write config overrides: {e}"))?;

        // Replace the journal wholesale rather than appending. The store
        // caps and can pop entries, so "what changed" is not expressible
        // as an insert; a full replacement is also what makes a retry
        // after a failover safe.
        sqlx::query("delete from config_revisions")
            .execute(&mut *tx)
            .await
            .map_err(|e| format!("could not clear config history: {e}"))?;

        for revision in &state.revisions {
            let before = toml::to_string_pretty(&revision.overrides_before)
                .map_err(|e| format!("could not serialize a revision: {e}"))?;
            sqlx::query(
                "insert into config_revisions \
                 (id, at, source, summary, changed_paths, overrides_before) \
                 values ($1, $2, $3, $4, $5, $6)",
            )
            .bind(revision.id as i64)
            .bind(revision.at)
            .bind(source_name(revision.source))
            .bind(&revision.summary)
            .bind(&revision.changed_paths)
            .bind(&before)
            .execute(&mut *tx)
            .await
            .map_err(|e| format!("could not write config history: {e}"))?;
        }

        tx.commit()
            .await
            .map_err(|e| format!("could not commit config changes: {e}"))
    }

    fn describe(&self) -> String {
        self.describe.clone()
    }
}

fn row_to_revision(row: &PgRow) -> Option<Revision> {
    Some(Revision {
        id: row.try_get::<i64, _>("id").ok()? as u64,
        at: row.try_get("at").ok()?,
        source: parse_source(&row.try_get::<String, _>("source").ok()?),
        summary: row.try_get("summary").ok()?,
        changed_paths: row.try_get("changed_paths").ok()?,
        overrides_before: toml::from_str(&row.try_get::<String, _>("overrides_before").ok()?)
            .ok()?,
    })
}

fn source_name(source: niles_config::ChangeSource) -> &'static str {
    match source {
        niles_config::ChangeSource::Voice => "voice",
        _ => "api",
    }
}

fn parse_source(name: &str) -> niles_config::ChangeSource {
    match name {
        "voice" => niles_config::ChangeSource::Voice,
        _ => niles_config::ChangeSource::Api,
    }
}

/// Strip credentials from a DSN, leaving something safe to log.
///
/// `postgres://niles:hunter2@host:5432/niles?sslmode=require` becomes
/// `the niles database at host:5432/niles`. Called once at construction
/// so no later code path has to remember to do it.
fn redact(dsn: &str) -> String {
    let without_scheme = dsn.split_once("://").map_or(dsn, |(_, rest)| rest);
    // Everything before '@' is userinfo — the part that must not be
    // logged. A DSN with no '@' has no credentials in it to begin with.
    let host_and_path = without_scheme
        .rsplit_once('@')
        .map_or(without_scheme, |(_, rest)| rest);
    // Query parameters can carry a password too (`?password=`), so they
    // are dropped rather than filtered.
    let host_and_path = host_and_path
        .split_once('?')
        .map_or(host_and_path, |(head, _)| head);
    format!("the niles database at {host_and_path}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_removes_credentials() {
        assert_eq!(
            redact(
                "postgres://niles:hunter2@postgres-rw.database.svc.cluster.local:5432/niles?sslmode=require"
            ),
            "the niles database at postgres-rw.database.svc.cluster.local:5432/niles"
        );
    }

    #[test]
    fn redact_drops_query_parameters_which_can_also_hold_a_password() {
        assert_eq!(
            redact("postgres://host/niles?password=hunter2"),
            "the niles database at host/niles"
        );
    }

    #[test]
    fn redact_handles_a_dsn_without_credentials() {
        assert_eq!(
            redact("postgres://host:5432/niles"),
            "the niles database at host:5432/niles"
        );
    }

    #[test]
    fn redact_never_leaks_a_password_it_does_not_recognise() {
        // An '@' inside the password is legal and would fool a
        // left-to-right split; rsplit is what keeps this honest.
        let dsn = "postgres://niles:p@ss@host:5432/niles";
        let out = redact(dsn);
        assert!(!out.contains("p@ss"), "{out}");
        assert!(!out.contains("niles:"), "{out}");
    }

    #[test]
    fn an_invalid_dsn_is_refused_at_construction() {
        assert!(PostgresBackend::connect_lazy("not a url", 5).is_err());
    }

    #[tokio::test]
    async fn a_valid_dsn_builds_without_connecting() {
        // The point of `connect_lazy`: an unreachable database must not
        // stop Niles from starting on its base config.
        let backend =
            PostgresBackend::connect_lazy("postgres://niles:pw@nowhere.invalid:5432/niles", 5)
                .expect("valid DSN");
        assert_eq!(
            backend.describe(),
            "the niles database at nowhere.invalid:5432/niles"
        );
    }
}
