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
//!
//! A picture is a row of `artifact_images`: the bytes as Niles wrote
//! them again (see `niles_artifacts::images`), never as they were
//! fetched. It goes when its artifact goes, or when it is taken away.
//!
//! A share is a row of `artifact_shares`: who else in the household
//! has the artifact, to view or to edit, with their own pin on it. It
//! goes when its artifact goes, or when the owner stops sharing.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use niles_artifacts::{
    Artifact, ArtifactStore, Author, Changes, Error, Filter, Image, ImageData, KEPT_REVISIONS,
    Level, NewArtifact, NewImage, NewRevision, PageChange, Result, Revision, RevisionMeta, Share,
    Status, StatusChange,
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
create table if not exists artifact_images (
    id          bigserial primary key,
    artifact_id bigint not null references artifacts (id) on delete cascade,
    mime        text not null,
    bytes       bytea not null,
    width       integer not null,
    height      integer not null,
    alt         text not null,
    credit      text,
    source      text,
    created_at  timestamptz not null default now()
);
create index if not exists artifact_images_artifact on artifact_images (artifact_id);
-- Added after the tables shipped, so `add column if not exists` rather
-- than in the tables above.
alter table artifacts add column if not exists page bigint;
alter table artifact_revisions add column if not exists page bigint
    references artifact_pages (id);
-- Whose change a version is, when a person's: the owner's, or someone
-- it is shared with to edit.
alter table artifacts add column if not exists changed_by text;
alter table artifact_revisions add column if not exists by_person text;
create table if not exists artifact_shares (
    artifact_id bigint not null references artifacts (id) on delete cascade,
    person      text not null,
    level       text not null,
    pinned      boolean not null default false,
    shared_at   timestamptz not null default now(),
    primary key (artifact_id, person)
);
create index if not exists artifact_shares_person on artifact_shares (person);
";

/// Qualified, so a query joining `artifact_shares` (which has a `person`
/// and a `pinned` of its own) reads the artifact's.
const COLUMNS: &str = "artifacts.id, artifacts.person, artifacts.kind, artifacts.title, \
    artifacts.summary, artifacts.pinned, artifacts.archived, artifacts.notify, artifacts.status, \
    artifacts.status_text, artifacts.error, artifacts.job_id, artifacts.brief, \
    artifacts.conversation_id, artifacts.created_at, artifacts.updated_at, \
    artifacts.current_revision, artifacts.preview::text as preview, artifacts.page, \
    artifacts.changed_by, \
    array(select sw.person from artifact_shares sw where sw.artifact_id = artifacts.id \
          order by sw.shared_at, sw.person) as shared_with";

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
            "select {COLUMNS} from artifacts where artifacts.id = $1 and artifacts.person = $2"
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
        (author, by): (Author, Option<&str>),
        preview: Option<String>,
        summary: Option<&str>,
    ) -> Result<()> {
        let n: i32 = sqlx::query(
            "insert into artifact_revisions
                 (artifact_id, n, content, page, reason, author, by_person)
             values ($1,
                     coalesce((select max(n) from artifact_revisions where artifact_id = $1), 0) + 1,
                     $2::jsonb, $3, $4, $5, $6)
             returning n",
        )
        .bind(id)
        .bind(content)
        .bind(page)
        .bind(reason)
        .bind(author.as_str())
        .bind(by)
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
                 preview = coalesce($4::jsonb, preview), summary = coalesce($5, summary),
                 changed_by = $6
             where id = $1",
        )
        .bind(id)
        .bind(n)
        .bind(page)
        .bind(preview)
        .bind(summary)
        .bind(by)
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
        changed_by: r.get("changed_by"),
        shared_with: r.get("shared_with"),
    }
}

/// An artifact as someone it is shared with has it: with their pin.
fn shared(r: &PgRow) -> Option<(Artifact, Level)> {
    let level = Level::parse(r.get::<&str, _>("level"))?;
    let mut a = artifact(r);
    a.pinned = r.get("my_pin");
    Some((a, level))
}

const IMAGE_COLUMNS: &str = "i.id, i.mime, i.width, i.height, octet_length(i.bytes) as size, \
    i.alt, i.credit, i.source, i.created_at";

fn image(r: &PgRow) -> Image {
    Image {
        id: r.get("id"),
        mime: r.get("mime"),
        width: r.get::<i32, _>("width").max(0) as u32,
        height: r.get::<i32, _>("height").max(0) as u32,
        size: r.get::<i32, _>("size").max(0) as usize,
        alt: r.get("alt"),
        credit: r.get("credit"),
        source: r.get("source"),
        created_at: r.get("created_at"),
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
                (first.author, first.by.as_deref()),
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
             where artifacts.person = $1
               and ($2::boolean is null or artifacts.pinned = $2)
               and ($3::boolean is null or artifacts.archived = $3)
             order by artifacts.pinned desc, artifacts.updated_at desc, artifacts.id desc"
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
            "select r.n, r.content::text as content, r.page, r.reason, r.author, r.by_person, r.at
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
                by: r.get("by_person"),
                at: r.get("at"),
            })
        })
        .transpose()
    }

    async fn revisions(&self, person: &str, id: i64) -> Result<Vec<RevisionMeta>> {
        self.ensure_schema().await?;
        let rows = sqlx::query(
            "select r.n, r.page, r.reason, r.author, r.by_person, r.at
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
                by: r.get("by_person"),
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
            (revision.author, revision.by.as_deref()),
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
        by: Option<String>,
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
        self.keep(
            &mut tx,
            id,
            &content,
            page,
            &reason,
            (author, by.as_deref()),
            None,
            None,
        )
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
             where artifacts.id = $1 and artifacts.person = $2
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
             where artifacts.id = $1 and artifacts.person = $2
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

    async fn add_image(&self, person: &str, id: i64, new: NewImage) -> Result<Option<Image>> {
        self.ensure_schema().await?;
        let mut tx = self.pool.begin().await.map_err(|e| self.storage(e))?;
        // Locked, so two pictures added at once are counted one after
        // the other against the artifact's room.
        if self.lock(&mut tx, person, id).await?.is_none() {
            return Ok(None);
        }
        let kept = sqlx::query(
            "select count(*) as n, coalesce(sum(octet_length(bytes)), 0)::bigint as total
             from artifact_images where artifact_id = $1",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| self.storage(e))?;
        niles_artifacts::images::check_room(
            kept.get::<i64, _>("n").max(0) as usize,
            kept.get::<i64, _>("total").max(0) as usize,
            new.picture.bytes.len(),
        )?;
        let row = sqlx::query(&format!(
            "insert into artifact_images as i
                 (artifact_id, mime, bytes, width, height, alt, credit, source)
             values ($1, $2, $3, $4, $5, $6, $7, $8)
             returning {IMAGE_COLUMNS}"
        ))
        .bind(id)
        .bind(new.picture.mime)
        .bind(&new.picture.bytes)
        .bind(new.picture.width as i32)
        .bind(new.picture.height as i32)
        .bind(&new.alt)
        .bind(&new.credit)
        .bind(&new.source)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| self.storage(e))?;
        tx.commit().await.map_err(|e| self.storage(e))?;
        Ok(Some(image(&row)))
    }

    async fn images(&self, person: &str, id: i64) -> Result<Vec<Image>> {
        self.ensure_schema().await?;
        let rows = sqlx::query(&format!(
            "select {IMAGE_COLUMNS}
             from artifact_images i join artifacts a on a.id = i.artifact_id
             where a.id = $1 and a.person = $2
             order by i.id"
        ))
        .bind(id)
        .bind(person)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(rows.iter().map(image).collect())
    }

    async fn image(&self, person: &str, id: i64, picture: i64) -> Result<Option<ImageData>> {
        self.ensure_schema().await?;
        let row = sqlx::query(&format!(
            "select {IMAGE_COLUMNS}, i.bytes
             from artifact_images i join artifacts a on a.id = i.artifact_id
             where a.id = $1 and a.person = $2 and i.id = $3"
        ))
        .bind(id)
        .bind(person)
        .bind(picture)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(row.map(|r| ImageData {
            image: image(&r),
            bytes: r.get("bytes"),
        }))
    }

    async fn remove_image(&self, person: &str, id: i64, picture: i64) -> Result<bool> {
        self.ensure_schema().await?;
        let done = sqlx::query(
            "delete from artifact_images i using artifacts a
             where a.id = i.artifact_id and a.id = $1 and a.person = $2 and i.id = $3",
        )
        .bind(id)
        .bind(person)
        .bind(picture)
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

    async fn shared(&self, person: &str, id: i64) -> Result<Option<(Artifact, Level)>> {
        self.ensure_schema().await?;
        let row = sqlx::query(&format!(
            "select {COLUMNS}, s.level, s.pinned as my_pin
             from artifacts join artifact_shares s on s.artifact_id = artifacts.id
             where artifacts.id = $1 and s.person = $2"
        ))
        .bind(id)
        .bind(person)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(row.as_ref().and_then(shared))
    }

    async fn shared_with(&self, person: &str, filter: Filter) -> Result<Vec<(Artifact, Level)>> {
        self.ensure_schema().await?;
        let rows = sqlx::query(&format!(
            "select {COLUMNS}, s.level, s.pinned as my_pin
             from artifacts join artifact_shares s on s.artifact_id = artifacts.id
             where s.person = $1
               and ($2::boolean is null or s.pinned = $2)
               and ($3::boolean is null or artifacts.archived = $3)
             order by s.pinned desc, artifacts.updated_at desc, artifacts.id desc"
        ))
        .bind(person)
        .bind(filter.pinned)
        .bind(filter.archived)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(rows.iter().filter_map(shared).collect())
    }

    async fn shares(&self, owner: &str, id: i64) -> Result<Vec<Share>> {
        self.ensure_schema().await?;
        let rows = sqlx::query(
            "select s.person, s.level, s.shared_at
             from artifact_shares s join artifacts a on a.id = s.artifact_id
             where a.id = $1 and a.person = $2
             order by s.shared_at, s.person",
        )
        .bind(id)
        .bind(owner)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(rows
            .iter()
            .filter_map(|r| {
                Some(Share {
                    person: r.get("person"),
                    level: Level::parse(r.get::<&str, _>("level"))?,
                    shared_at: r.get("shared_at"),
                })
            })
            .collect())
    }

    async fn set_share(
        &self,
        owner: &str,
        id: i64,
        person: &str,
        level: Option<Level>,
    ) -> Result<bool> {
        self.ensure_schema().await?;
        if self.one(owner, id).await?.is_none() {
            return Ok(false);
        }
        if person == owner {
            return Err(niles_artifacts::share::with_the_owner());
        }
        let query = match level {
            Some(level) => sqlx::query(
                "insert into artifact_shares (artifact_id, person, level) values ($1, $2, $3)
                 on conflict (artifact_id, person) do update set level = excluded.level",
            )
            .bind(id)
            .bind(person)
            .bind(level.as_str()),
            None => {
                sqlx::query("delete from artifact_shares where artifact_id = $1 and person = $2")
                    .bind(id)
                    .bind(person)
            }
        };
        query
            .execute(&self.pool)
            .await
            .map_err(|e| self.storage(e))?;
        Ok(true)
    }

    async fn pin_shared(&self, person: &str, id: i64, pinned: bool) -> Result<bool> {
        self.ensure_schema().await?;
        let done = sqlx::query(
            "update artifact_shares set pinned = $3 where artifact_id = $1 and person = $2",
        )
        .bind(id)
        .bind(person)
        .bind(pinned)
        .execute(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(done.rows_affected() == 1)
    }
}
