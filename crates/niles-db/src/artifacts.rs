//! Each person's artifacts and every version of them. See
//! ARCHITECTURE.md § Artifacts (per person).
//!
//! The content is JSONB, whose shape is the kind's business; it goes in
//! and out as text, cast in the statement, because the pool's sqlx has
//! no JSON feature and the conversation log already works this way.
//!
//! A page is a row of `artifact_pages`, kept once and named by id from
//! each version drawn with it, so a plan Niles marks a run on twenty
//! times a month keeps one page and not twenty copies. It goes when no
//! version kept draws with it any more.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use niles_artifacts::{
    Artifact, ArtifactStore, Author, Changes, Error, Filter, KEPT_REVISIONS, NewArtifact,
    NewRevision, PageChange, Result, Revision, RevisionMeta, Status, StatusChange,
};
use sqlx::postgres::{PgPool, PgRow};
use sqlx::{Postgres, Row, Transaction};

const SCHEMA: &str = "
create table if not exists artifacts (
    id               bigserial primary key,
    person           text not null,
    kind             text not null,
    title            text not null,
    summary          text not null default '',
    pinned           boolean not null default false,
    archived         boolean not null default false,
    notify           boolean not null default true,
    status           text not null default 'ready',
    status_text      text,
    error            text,
    job_id           text,
    brief            text,
    conversation_id  bigint,
    created_at       timestamptz not null default now(),
    updated_at       timestamptz not null default now(),
    current_revision integer not null default 0,
    preview          jsonb not null default '{\"lines\": []}'
);
create index if not exists artifacts_person on artifacts (person, updated_at desc);
create table if not exists artifact_revisions (
    artifact_id bigint not null references artifacts (id) on delete cascade,
    n           integer not null,
    content     jsonb not null,
    reason      text not null,
    author      text not null,
    at          timestamptz not null default now(),
    primary key (artifact_id, n)
);
create table if not exists artifact_pages (
    id          bigserial primary key,
    artifact_id bigint not null references artifacts (id) on delete cascade,
    html        text not null,
    at          timestamptz not null default now()
);
create index if not exists artifact_pages_artifact on artifact_pages (artifact_id);
-- Added after the tables shipped, so `add column if not exists` rather
-- than in the tables above.
alter table artifacts add column if not exists page bigint;
alter table artifact_revisions add column if not exists page bigint
    references artifact_pages (id);
";

const COLUMNS: &str = "id, person, kind, title, summary, pinned, archived, notify, status, \
    status_text, error, job_id, brief, conversation_id, created_at, updated_at, current_revision, \
    preview::text as preview, page";

pub struct PostgresArtifacts {
    pool: PgPool,
    describe: String,
    schema: tokio::sync::OnceCell<()>,
}

impl PostgresArtifacts {
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
            .copied()
    }

    fn storage(&self, e: impl std::fmt::Display) -> Error {
        Error::Storage(format!("{}: {e}", self.describe))
    }

    async fn one(&self, person: &str, id: i64) -> Result<Option<Artifact>> {
        let row = sqlx::query(&format!(
            "select {COLUMNS} from artifacts where id = $1 and person = $2"
        ))
        .bind(id)
        .bind(person)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(row.as_ref().map(artifact))
    }

    /// The page a new version of artifact `id` draws with, when the
    /// current one draws with `current`.
    async fn page_for(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: i64,
        current: Option<i64>,
        change: &PageChange,
    ) -> Result<Option<i64>> {
        match change {
            PageChange::Keep => Ok(current),
            PageChange::Remove => Ok(None),
            PageChange::New(html) => Ok(Some(
                sqlx::query(
                    "insert into artifact_pages (artifact_id, html) values ($1, $2) returning id",
                )
                .bind(id)
                .bind(html)
                .fetch_one(&mut **tx)
                .await
                .map_err(|e| self.storage(e))?
                .get("id"),
            )),
            PageChange::Use(page) => {
                let kept =
                    sqlx::query("select id from artifact_pages where id = $1 and artifact_id = $2")
                        .bind(page)
                        .bind(id)
                        .fetch_optional(&mut **tx)
                        .await
                        .map_err(|e| self.storage(e))?;
                match kept {
                    Some(_) => Ok(Some(*page)),
                    None => Err(Error::Invalid {
                        kind: "page",
                        reason: format!("page {page} is not one this artifact keeps"),
                    }),
                }
            }
        }
    }

    /// `content` (JSON, as text) kept as the new current version of
    /// artifact `id`, numbered after the newest, drawn with `page`; the
    /// oldest past the limit goes, and with it any page no version kept
    /// draws with. The preview and summary change when given. The row
    /// must be locked already.
    #[allow(clippy::too_many_arguments)]
    async fn keep(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: i64,
        content: &str,
        page: Option<i64>,
        reason: &str,
        author: Author,
        preview: Option<String>,
        summary: Option<&str>,
    ) -> Result<()> {
        let n: i32 = sqlx::query(
            "insert into artifact_revisions (artifact_id, n, content, page, reason, author)
             values ($1,
                     coalesce((select max(n) from artifact_revisions where artifact_id = $1), 0) + 1,
                     $2::jsonb, $3, $4, $5)
             returning n",
        )
        .bind(id)
        .bind(content)
        .bind(page)
        .bind(reason)
        .bind(author.as_str())
        .fetch_one(&mut **tx)
        .await
        .map_err(|e| self.storage(e))?
        .get("n");
        sqlx::query("delete from artifact_revisions where artifact_id = $1 and n <= $2")
            .bind(id)
            .bind(n - KEPT_REVISIONS as i32)
            .execute(&mut **tx)
            .await
            .map_err(|e| self.storage(e))?;
        sqlx::query(
            "delete from artifact_pages p
             where p.artifact_id = $1
               and not exists (select 1 from artifact_revisions r
                               where r.artifact_id = $1 and r.page = p.id)",
        )
        .bind(id)
        .execute(&mut **tx)
        .await
        .map_err(|e| self.storage(e))?;
        sqlx::query(
            "update artifacts
             set current_revision = $2, page = $3, updated_at = now(),
                 preview = coalesce($4::jsonb, preview), summary = coalesce($5, summary)
             where id = $1",
        )
        .bind(id)
        .bind(n)
        .bind(page)
        .bind(preview)
        .bind(summary)
        .execute(&mut **tx)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(())
    }

    /// The kind and page of artifact `id` of `person`'s, and its current
    /// version, its row locked so two saves at once number their
    /// versions one after the other rather than both taking the same
    /// number.
    async fn lock(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        person: &str,
        id: i64,
    ) -> Result<Option<(String, Option<i64>, i32)>> {
        let row = sqlx::query(
            "select kind, page, current_revision from artifacts
             where id = $1 and person = $2 for update",
        )
        .bind(id)
        .bind(person)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(row.map(|r| (r.get("kind"), r.get("page"), r.get("current_revision"))))
    }
}

fn artifact(r: &PgRow) -> Artifact {
    Artifact {
        id: r.get("id"),
        person: r.get("person"),
        kind: r.get("kind"),
        title: r.get("title"),
        summary: r.get("summary"),
        pinned: r.get("pinned"),
        archived: r.get("archived"),
        notify: r.get("notify"),
        // A status this build does not know is one it cannot be
        // working on.
        status: Status::parse(r.get::<&str, _>("status")).unwrap_or(Status::Failed),
        status_text: r.get("status_text"),
        error: r.get("error"),
        job_id: r.get("job_id"),
        brief: r.get("brief"),
        conversation_id: r.get("conversation_id"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
        current_revision: r.get("current_revision"),
        // A preview that will not read costs the card its lines, not the
        // artifact.
        preview: serde_json::from_str(r.get::<&str, _>("preview")).unwrap_or_default(),
        page: r.get("page"),
    }
}

fn preview_json(kind: &str, content: &serde_json::Value) -> String {
    serde_json::to_string(&niles_artifacts::kinds::preview(kind, content))
        .unwrap_or_else(|_| "{\"lines\": []}".into())
}

fn author(r: &PgRow) -> Author {
    Author::parse(r.get::<&str, _>("author")).unwrap_or(Author::Niles)
}

#[async_trait]
impl ArtifactStore for PostgresArtifacts {
    async fn create(&self, person: &str, new: NewArtifact) -> Result<Artifact> {
        self.ensure_schema().await?;
        let mut tx = self.pool.begin().await.map_err(|e| self.storage(e))?;
        let summary = new
            .first
            .as_ref()
            .and_then(|f| f.summary.clone())
            .unwrap_or(new.summary);
        let id: i64 = sqlx::query(
            "insert into artifacts
                 (person, kind, title, summary, notify, status, status_text, job_id, brief,
                  conversation_id)
             values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) returning id",
        )
        .bind(person)
        .bind(&new.kind)
        .bind(&new.title)
        .bind(&summary)
        .bind(new.notify)
        .bind(new.status.as_str())
        .bind(&new.status_text)
        .bind(&new.job_id)
        .bind(&new.brief)
        .bind(new.conversation_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| self.storage(e))?
        .get("id");
        if let Some(first) = &new.first {
            let page = self.page_for(&mut tx, id, None, &first.page).await?;
            let content = serde_json::to_string(&first.content).map_err(|e| self.storage(e))?;
            self.keep(
                &mut tx,
                id,
                &content,
                page,
                &first.reason,
                first.author,
                Some(preview_json(&new.kind, &first.content)),
                None,
            )
            .await?;
        }
        tx.commit().await.map_err(|e| self.storage(e))?;
        self.one(person, id)
            .await?
            .ok_or_else(|| self.storage("the artifact just made is not there"))
    }

    async fn list(&self, person: &str, filter: Filter) -> Result<Vec<Artifact>> {
        self.ensure_schema().await?;
        let rows = sqlx::query(&format!(
            "select {COLUMNS} from artifacts
             where person = $1
               and ($2::boolean is null or pinned = $2)
               and ($3::boolean is null or archived = $3)
             order by pinned desc, updated_at desc, id desc"
        ))
        .bind(person)
        .bind(filter.pinned)
        .bind(filter.archived)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(rows.iter().map(artifact).collect())
    }

    async fn get(&self, person: &str, id: i64) -> Result<Option<Artifact>> {
        self.ensure_schema().await?;
        self.one(person, id).await
    }

    async fn revision(&self, person: &str, id: i64, n: i32) -> Result<Option<Revision>> {
        self.ensure_schema().await?;
        let row = sqlx::query(
            "select r.n, r.content::text as content, r.page, r.reason, r.author, r.at
             from artifact_revisions r join artifacts a on a.id = r.artifact_id
             where a.id = $1 and a.person = $2 and r.n = $3",
        )
        .bind(id)
        .bind(person)
        .bind(n)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        row.map(|r| {
            Ok(Revision {
                n: r.get("n"),
                content: serde_json::from_str(r.get::<&str, _>("content"))
                    .map_err(|e| self.storage(e))?,
                page: r.get("page"),
                reason: r.get("reason"),
                author: author(&r),
                at: r.get("at"),
            })
        })
        .transpose()
    }

    async fn revisions(&self, person: &str, id: i64) -> Result<Vec<RevisionMeta>> {
        self.ensure_schema().await?;
        let rows = sqlx::query(
            "select r.n, r.page, r.reason, r.author, r.at
             from artifact_revisions r join artifacts a on a.id = r.artifact_id
             where a.id = $1 and a.person = $2
             order by r.n desc",
        )
        .bind(id)
        .bind(person)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(rows
            .iter()
            .map(|r| RevisionMeta {
                n: r.get("n"),
                page: r.get("page"),
                reason: r.get("reason"),
                author: author(r),
                at: r.get("at"),
            })
            .collect())
    }

    async fn revise(
        &self,
        person: &str,
        id: i64,
        revision: NewRevision,
    ) -> Result<Option<Artifact>> {
        self.ensure_schema().await?;
        let mut tx = self.pool.begin().await.map_err(|e| self.storage(e))?;
        let Some((kind, current, _)) = self.lock(&mut tx, person, id).await? else {
            return Ok(None);
        };
        let page = self.page_for(&mut tx, id, current, &revision.page).await?;
        let content = serde_json::to_string(&revision.content).map_err(|e| self.storage(e))?;
        self.keep(
            &mut tx,
            id,
            &content,
            page,
            &revision.reason,
            revision.author,
            Some(preview_json(&kind, &revision.content)),
            revision.summary.as_deref(),
        )
        .await?;
        tx.commit().await.map_err(|e| self.storage(e))?;
        self.one(person, id).await
    }

    async fn set_page(
        &self,
        person: &str,
        id: i64,
        page: PageChange,
        reason: String,
        author: Author,
    ) -> Result<Option<Artifact>> {
        self.ensure_schema().await?;
        let mut tx = self.pool.begin().await.map_err(|e| self.storage(e))?;
        let Some((_, current, n)) = self.lock(&mut tx, person, id).await? else {
            return Ok(None);
        };
        let content: String = sqlx::query(
            "select content::text as content from artifact_revisions
             where artifact_id = $1 and n = $2",
        )
        .bind(id)
        .bind(n)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| self.storage(e))?
        .ok_or_else(niles_artifacts::page::nothing_to_draw)?
        .get("content");
        let page = self.page_for(&mut tx, id, current, &page).await?;
        self.keep(&mut tx, id, &content, page, &reason, author, None, None)
            .await?;
        tx.commit().await.map_err(|e| self.storage(e))?;
        self.one(person, id).await
    }

    async fn page(&self, person: &str, id: i64, page: i64) -> Result<Option<String>> {
        self.ensure_schema().await?;
        let row = sqlx::query(
            "select p.html from artifact_pages p join artifacts a on a.id = p.artifact_id
             where a.id = $1 and a.person = $2 and p.id = $3",
        )
        .bind(id)
        .bind(person)
        .bind(page)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(row.map(|r| r.get("html")))
    }

    async fn change(&self, person: &str, id: i64, changes: Changes) -> Result<Option<Artifact>> {
        self.ensure_schema().await?;
        let row = sqlx::query(&format!(
            "update artifacts set
                 title = coalesce($3, title),
                 updated_at = case when $3 is null then updated_at else now() end,
                 pinned = coalesce($4, pinned),
                 archived = coalesce($5, archived),
                 notify = coalesce($6, notify)
             where id = $1 and person = $2
             returning {COLUMNS}"
        ))
        .bind(id)
        .bind(person)
        .bind(&changes.title)
        .bind(changes.pinned)
        .bind(changes.archived)
        .bind(changes.notify)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(row.as_ref().map(artifact))
    }

    async fn set_status(
        &self,
        person: &str,
        id: i64,
        status: StatusChange,
    ) -> Result<Option<Artifact>> {
        self.ensure_schema().await?;
        let row = sqlx::query(&format!(
            "update artifacts set status = $3, status_text = $4, error = $5, job_id = $6,
                 updated_at = now()
             where id = $1 and person = $2
             returning {COLUMNS}"
        ))
        .bind(id)
        .bind(person)
        .bind(status.status.as_str())
        .bind(&status.status_text)
        .bind(&status.error)
        .bind(&status.job_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(row.as_ref().map(artifact))
    }

    async fn set_conversation(&self, person: &str, id: i64, conversation: i64) -> Result<bool> {
        self.ensure_schema().await?;
        let done =
            sqlx::query("update artifacts set conversation_id = $3 where id = $1 and person = $2")
                .bind(id)
                .bind(person)
                .bind(conversation)
                .execute(&self.pool)
                .await
                .map_err(|e| self.storage(e))?;
        Ok(done.rows_affected() == 1)
    }

    async fn delete(&self, person: &str, id: i64) -> Result<bool> {
        self.ensure_schema().await?;
        let done = sqlx::query("delete from artifacts where id = $1 and person = $2")
            .bind(id)
            .bind(person)
            .execute(&self.pool)
            .await
            .map_err(|e| self.storage(e))?;
        Ok(done.rows_affected() == 1)
    }

    async fn fail_abandoned(&self, reason: &str, before: DateTime<Utc>) -> Result<u64> {
        self.ensure_schema().await?;
        let done = sqlx::query(
            "update artifacts set status = 'failed', status_text = null, error = $1, job_id = null
             where status = 'working' and updated_at < $2",
        )
        .bind(reason)
        .bind(before)
        .execute(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(done.rows_affected())
    }
}
