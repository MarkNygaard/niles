//! Each person's notifications, for their Notifications page. What they
//! are is in `niles_webpush::inbox`.
//!
//! A notice with a tag already kept for that person takes over its row
//! with a new id, so the id order is the time order and reading "up to"
//! an id never marks one that arrived after it.

use async_trait::async_trait;
use niles_webpush::inbox::KEPT_PER_PERSON;
use niles_webpush::{Error, Inbox, Notice, Notification, Result};
use sqlx::{Row, postgres::PgPool, postgres::PgRow};

const SCHEMA: &str = "
create table if not exists person_notifications (
    id         bigserial primary key,
    person     text not null,
    kind       text not null,
    title      text not null,
    body       text not null,
    url        text not null,
    tag        text not null,
    created_at timestamptz not null default now(),
    read_at    timestamptz
);
create unique index if not exists person_notifications_tag on person_notifications (person, tag);
create index if not exists person_notifications_person on person_notifications (person, id desc);
";

const COLUMNS: &str = "id, kind, title, body, url, tag, created_at, read_at";

pub struct PostgresPersonNotifications {
    pool: PgPool,
    describe: String,
    schema: tokio::sync::OnceCell<()>,
}

impl PostgresPersonNotifications {
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
        Error::Storage(format!("notifications ({}): {e}", self.describe))
    }
}

fn notification(row: &PgRow) -> Notification {
    Notification {
        id: row.get("id"),
        kind: row.get("kind"),
        title: row.get("title"),
        body: row.get("body"),
        url: row.get("url"),
        tag: row.get("tag"),
        created_at: row.get("created_at"),
        read_at: row.get("read_at"),
    }
}

#[async_trait]
impl Inbox for PostgresPersonNotifications {
    async fn add(&self, person: &str, notice: &Notice) -> Result<Notification> {
        self.ensure_schema().await?;
        let sql = format!(
            "insert into person_notifications (person, kind, title, body, url, tag)
             values ($1, $2, $3, $4, $5, $6)
             on conflict (person, tag) do update set
                 id = nextval(pg_get_serial_sequence('person_notifications', 'id')),
                 kind = excluded.kind, title = excluded.title, body = excluded.body,
                 url = excluded.url, created_at = now(), read_at = null
             returning {COLUMNS}"
        );
        let row = sqlx::query(&sql)
            .bind(person)
            .bind(notice.kind.as_str())
            .bind(&notice.title)
            .bind(&notice.body)
            .bind(&notice.url)
            .bind(&notice.tag)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| self.storage(e))?;
        sqlx::query(
            "delete from person_notifications where person = $1 and id not in (
                 select id from person_notifications where person = $1 order by id desc limit $2)",
        )
        .bind(person)
        .bind(KEPT_PER_PERSON)
        .execute(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(notification(&row))
    }

    async fn page(
        &self,
        person: &str,
        before: Option<i64>,
        limit: i64,
    ) -> Result<Vec<Notification>> {
        self.ensure_schema().await?;
        let sql = format!(
            "select {COLUMNS} from person_notifications
             where person = $1 and ($2::bigint is null or id < $2)
             order by id desc limit $3"
        );
        let rows = sqlx::query(&sql)
            .bind(person)
            .bind(before)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| self.storage(e))?;
        Ok(rows.iter().map(notification).collect())
    }

    async fn unread(&self, person: &str) -> Result<i64> {
        self.ensure_schema().await?;
        sqlx::query_scalar(
            "select count(*) from person_notifications where person = $1 and read_at is null",
        )
        .bind(person)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| self.storage(e))
    }

    async fn read(&self, person: &str, up_to: Option<i64>) -> Result<u64> {
        self.ensure_schema().await?;
        let done = sqlx::query(
            "update person_notifications set read_at = now()
             where person = $1 and read_at is null and ($2::bigint is null or id <= $2)",
        )
        .bind(person)
        .bind(up_to)
        .execute(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(done.rows_affected())
    }
}
