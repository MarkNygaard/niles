//! Each person's own MCP servers, and how each is signed in.
//!
//! Two tables. `personal_tool_servers` holds what My profile lists: the
//! name, the address, how it signs in and each tool's setting. The key,
//! or a browser sign-in's client and tokens, live in
//! `personal_tool_server_secrets`, sealed to their person, server and
//! field, and go with their server (`on delete cascade`).

use crate::secrets::Sealer;
use async_trait::async_trait;
use niles_mcp::personal::free_slug;
use niles_mcp::{
    BrowserSignIn, Credential, Error, Key, Mode, NewServer, PersonalServer, PersonalServers,
    Result, ServerEdit, SignInKind,
};
use sqlx::{Row, postgres::PgPool, postgres::PgRow};
use std::collections::BTreeMap;

const SCHEMA: &str = "
create table if not exists personal_tool_servers (
    id bigserial primary key, person text not null, slug text not null,
    name text not null, url text not null, sign_in text not null,
    header text, modes jsonb not null default '{}',
    created_at timestamptz not null default now(),
    unique (person, slug));
create table if not exists personal_tool_server_secrets (
    server_id bigint primary key references personal_tool_servers (id) on delete cascade,
    key_enc bytea, browser_enc bytea);
";

/// Which of a server's sealed values a context names.
#[derive(Clone, Copy)]
enum Field {
    Key,
    Browser,
}

/// What a value is sealed to: its person, its server and its field.
fn sealed_to(person: &str, id: i64, field: Field) -> String {
    let field = match field {
        Field::Key => "key",
        Field::Browser => "browser sign-in",
    };
    format!("niles personal tool server\u{0}{person}\u{0}{id}\u{0}{field}")
}

pub struct PostgresServers {
    pool: PgPool,
    describe: String,
    sealer: Sealer,
    schema: tokio::sync::OnceCell<()>,
}

impl PostgresServers {
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

    /// Names the database, never the DSN, which carries a password.
    fn storage(&self, e: impl std::fmt::Display) -> Error {
        Error::Storage(format!("{}: {e}", self.describe))
    }

    fn seal(&self, value: &str, person: &str, id: i64, field: Field) -> Result<Vec<u8>> {
        self.sealer
            .seal_for(value, &sealed_to(person, id, field))
            .map_err(|e| self.storage(e))
    }

    fn open(&self, sealed: &[u8], person: &str, id: i64, field: Field) -> Result<String> {
        self.sealer
            .open_for(sealed, &sealed_to(person, id, field))
            .map_err(|e| self.storage(e))
    }

    fn server(row: &PgRow) -> PersonalServer {
        let modes: sqlx::types::Json<BTreeMap<String, Mode>> = row.get("modes");
        PersonalServer {
            id: row.get("id"),
            slug: row.get("slug"),
            name: row.get("name"),
            url: row.get("url"),
            sign_in: kind_of(row.get::<&str, _>("sign_in")),
            header: row.get("header"),
            modes: modes.0,
            signed_in: row.get("signed_in"),
        }
    }
}

fn kind_name(kind: SignInKind) -> &'static str {
    match kind {
        SignInKind::Key => "key",
        SignInKind::Browser => "browser",
    }
}

fn kind_of(name: &str) -> SignInKind {
    if name == "browser" {
        SignInKind::Browser
    } else {
        SignInKind::Key
    }
}

const SELECT: &str = "select s.id, s.person, s.slug, s.name, s.url, s.sign_in, s.header, s.modes,
     (k.key_enc is not null or k.browser_enc is not null) as signed_in
     from personal_tool_servers s
     left join personal_tool_server_secrets k on k.server_id = s.id";

#[async_trait]
impl PersonalServers for PostgresServers {
    async fn list(&self, person: &str) -> Result<Vec<PersonalServer>> {
        self.ensure_schema().await?;
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
            "{SELECT} where s.person = $1 order by lower(s.name), s.id"
        )))
        .bind(person)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(rows.iter().map(Self::server).collect())
    }

    async fn get(&self, person: &str, id: i64) -> Result<Option<PersonalServer>> {
        self.ensure_schema().await?;
        let row = sqlx::query(sqlx::AssertSqlSafe(format!(
            "{SELECT} where s.person = $1 and s.id = $2"
        )))
        .bind(person)
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(row.as_ref().map(Self::server))
    }

    async fn add(&self, person: &str, new: &NewServer) -> Result<PersonalServer> {
        self.ensure_schema().await?;
        let mut tx = self.pool.begin().await.map_err(|e| self.storage(e))?;
        let taken: Vec<String> = sqlx::query_scalar(
            "select slug from personal_tool_servers where person = $1 for update",
        )
        .bind(person)
        .fetch_all(&mut *tx)
        .await
        .map_err(|e| self.storage(e))?;
        let slug = free_slug(&new.name, &taken);
        let id: i64 = sqlx::query_scalar(
            "insert into personal_tool_servers (person, slug, name, url, sign_in, header)
             values ($1, $2, $3, $4, $5, $6) returning id",
        )
        .bind(person)
        .bind(&slug)
        .bind(&new.name)
        .bind(&new.url)
        .bind(kind_name(new.sign_in))
        .bind(&new.header)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| self.storage(e))?;
        let key = new
            .key
            .as_ref()
            .map(|k| self.seal(k.expose(), person, id, Field::Key))
            .transpose()?;
        sqlx::query(
            "insert into personal_tool_server_secrets (server_id, key_enc) values ($1, $2)",
        )
        .bind(id)
        .bind(key)
        .execute(&mut *tx)
        .await
        .map_err(|e| self.storage(e))?;
        tx.commit().await.map_err(|e| self.storage(e))?;
        Ok(PersonalServer {
            id,
            slug,
            name: new.name.clone(),
            url: new.url.clone(),
            sign_in: new.sign_in,
            header: new.header.clone(),
            modes: BTreeMap::new(),
            signed_in: new.key.is_some(),
        })
    }

    async fn update(
        &self,
        person: &str,
        id: i64,
        edit: &ServerEdit,
    ) -> Result<Option<PersonalServer>> {
        self.ensure_schema().await?;
        let mut tx = self.pool.begin().await.map_err(|e| self.storage(e))?;
        let kind: Option<String> = sqlx::query_scalar(
            "update personal_tool_servers set name = $3,
                 header = case when sign_in = 'key' then $4 else header end
             where id = $1 and person = $2 returning sign_in",
        )
        .bind(id)
        .bind(person)
        .bind(&edit.name)
        .bind(&edit.header)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| self.storage(e))?;
        let Some(kind) = kind else {
            return Ok(None);
        };
        if kind_of(&kind) == SignInKind::Key
            && let Some(key) = &edit.key
        {
            sqlx::query(
                "update personal_tool_server_secrets set key_enc = $2 where server_id = $1",
            )
            .bind(id)
            .bind(self.seal(key.expose(), person, id, Field::Key)?)
            .execute(&mut *tx)
            .await
            .map_err(|e| self.storage(e))?;
        }
        tx.commit().await.map_err(|e| self.storage(e))?;
        self.get(person, id).await
    }

    async fn remove(&self, person: &str, id: i64) -> Result<bool> {
        self.ensure_schema().await?;
        let done = sqlx::query("delete from personal_tool_servers where id = $1 and person = $2")
            .bind(id)
            .bind(person)
            .execute(&self.pool)
            .await
            .map_err(|e| self.storage(e))?;
        Ok(done.rows_affected() == 1)
    }

    async fn set_mode(&self, person: &str, id: i64, tool: &str, mode: Mode) -> Result<bool> {
        self.ensure_schema().await?;
        let done = sqlx::query(
            "update personal_tool_servers set modes = modes || jsonb_build_object($3::text, $4::text)
             where id = $1 and person = $2",
        )
        .bind(id)
        .bind(person)
        .bind(tool)
        .bind(match mode {
            Mode::On => "on",
            Mode::Ask => "ask",
            Mode::Off => "off",
        })
        .execute(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(done.rows_affected() == 1)
    }

    async fn credential(&self, person: &str, id: i64) -> Result<Option<Credential>> {
        self.ensure_schema().await?;
        let Some(row) = sqlx::query(
            "select k.key_enc, k.browser_enc
             from personal_tool_server_secrets k
             join personal_tool_servers s on s.id = k.server_id
             where s.id = $1 and s.person = $2",
        )
        .bind(id)
        .bind(person)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| self.storage(e))?
        else {
            return Ok(None);
        };
        if let Some(sealed) = row.get::<Option<Vec<u8>>, _>("browser_enc") {
            let opened = self.open(&sealed, person, id, Field::Browser)?;
            let sign_in: BrowserSignIn =
                serde_json::from_str(&opened).map_err(|e| self.storage(e))?;
            return Ok(Some(Credential::Browser(Box::new(sign_in))));
        }
        match row.get::<Option<Vec<u8>>, _>("key_enc") {
            Some(sealed) => Ok(Some(Credential::Key(Key::new(self.open(
                &sealed,
                person,
                id,
                Field::Key,
            )?)))),
            None => Ok(None),
        }
    }

    async fn signed_in(&self, person: &str, id: i64, sign_in: &BrowserSignIn) -> Result<bool> {
        self.ensure_schema().await?;
        let json = serde_json::to_string(sign_in).map_err(|e| self.storage(e))?;
        let done = sqlx::query(
            "update personal_tool_server_secrets k set browser_enc = $3
             from personal_tool_servers s
             where k.server_id = s.id and s.id = $1 and s.person = $2 and s.sign_in = 'browser'",
        )
        .bind(id)
        .bind(person)
        .bind(self.seal(&json, person, id, Field::Browser)?)
        .execute(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(done.rows_affected() == 1)
    }

    async fn all(&self) -> Result<Vec<(String, PersonalServer)>> {
        self.ensure_schema().await?;
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
            "{SELECT} order by s.person, s.id"
        )))
        .fetch_all(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(rows
            .iter()
            .map(|row| {
                let person: String = row.get("person");
                (person, Self::server(row))
            })
            .collect())
    }
}

impl std::fmt::Debug for PostgresServers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostgresServers")
            .field("database", &self.describe)
            .finish()
    }
}
