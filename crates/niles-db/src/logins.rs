//! Each person's saved logins, and their browser profiles.
//!
//! Four tables. `saved_logins` holds what Vault lists: the label, the
//! sites and when the login was last used. The username, password and
//! authenticator key live in `saved_login_secrets`, a login's sealed
//! browser profile in `browser_profiles`, and what it was used for in
//! `login_activity`; all hang off the login with `on delete cascade`,
//! so removing a login takes them with it.
//!
//! The secrets have a table of their own so a select on the logins
//! never brings them along — the list page can't trip over ciphertext —
//! and so `psql` users don't either. Everything sealed answers to the
//! same key as the other credentials typed into the app (`Sealer`), and
//! a database without that key keeps no logins at all.

use crate::secrets::Sealer;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use niles_logins::{
    ACTIVITY_SHOWN, Activity, Authenticator, Context, Credentials, KeyChange, Login, LoginEdit,
    LoginStore, NewLogin, Origin, Result, Secret, Used,
};
use sqlx::{Row, postgres::PgPool, postgres::PgRow};

const SCHEMA: &str = "
create table if not exists saved_logins (
    id bigserial primary key, person text not null, label text not null,
    origins text[] not null, last_used timestamptz,
    created_at timestamptz not null default now(), updated_at timestamptz not null default now());
create index if not exists saved_logins_person on saved_logins (person);
create table if not exists saved_login_secrets (
    login_id bigint primary key references saved_logins (id) on delete cascade,
    username_enc bytea not null, password_enc bytea not null);
create table if not exists browser_profiles (
    login_id bigint primary key references saved_logins (id) on delete cascade,
    profile_enc bytea not null, saved_at timestamptz not null default now());
alter table saved_login_secrets add column if not exists authenticator_enc bytea;
create table if not exists login_activity (
    id bigserial primary key,
    login_id bigint not null references saved_logins (id) on delete cascade,
    at timestamptz not null default now(), used text not null,
    site text, conversation bigint);
create index if not exists login_activity_login on login_activity (login_id, at desc);
create index if not exists login_activity_at on login_activity (at);
";

/// Saved logins and browser profiles in Postgres. See
/// [`LoginStore`] for the contract.
pub struct PostgresLogins {
    pool: PgPool,
    describe: String,
    sealer: Sealer,
    schema: tokio::sync::OnceCell<()>,
}

impl PostgresLogins {
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

    /// Errors name the database but never the DSN — the connection
    /// string carries a password.
    fn storage(&self, e: impl std::fmt::Display) -> niles_logins::Error {
        niles_logins::Error::Storage(format!("{}: {e}", self.describe))
    }

    /// `value` sealed to its person, login and field.
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

    /// One row of the logins-with-secrets join as its owner sees it. A
    /// stored origin that no longer parses is skipped rather than
    /// failing the whole list; a username that won't open is a storage
    /// error, like `PostgresTraining`.
    async fn login(&self, row: &PgRow, person: &str) -> Result<Login> {
        let stored: Vec<String> = row.get("origins");
        let id: i64 = row.get("id");
        Ok(Login {
            id,
            label: row.get("label"),
            origins: stored
                .iter()
                .filter_map(|o| Origin::parse(o).ok())
                .collect(),
            username: self.open(
                &row.get::<Vec<u8>, _>("username_enc"),
                person,
                id,
                Field::Username,
            )?,
            last_used: row.get("last_used"),
            signed_in: row.get("signed_in"),
            authenticator: row.get("authenticator"),
        })
    }
}

/// Which of a login's sealed values a context names.
#[derive(Clone, Copy)]
enum Field {
    Username,
    Password,
    Profile,
    Authenticator,
}

/// What a login's value is sealed to: its person, its login and its
/// field. A value moved to another person, another login or another
/// column, such as a password copied into a username a list shows,
/// doesn't open there.
fn sealed_to(person: &str, id: i64, field: Field) -> String {
    let field = match field {
        Field::Username => "username",
        Field::Password => "password",
        Field::Profile => "browser profile",
        Field::Authenticator => "authenticator key",
    };
    format!("niles saved login\u{0}{person}\u{0}{id}\u{0}{field}")
}

/// Forgets a login's profile, noting it when there was one.
const DROP_PROFILE_SQL: &str = "with gone as (
         delete from browser_profiles where login_id = $1 returning login_id)
     insert into login_activity (login_id, used)
         select login_id, 'profile_dropped' from gone";

/// Notes a use the store itself makes, inside its transaction.
async fn note(
    tx: &mut sqlx::PgConnection,
    id: i64,
    used: Used,
) -> std::result::Result<(), sqlx::Error> {
    sqlx::query("insert into login_activity (login_id, used) values ($1, $2)")
        .bind(id)
        .bind(used.as_str())
        .execute(tx)
        .await
        .map(|_| ())
}

impl std::fmt::Debug for PostgresLogins {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostgresLogins")
            .field("database", &self.describe)
            .finish()
    }
}

const LIST_SQL: &str = "select l.id, l.label, l.origins, l.last_used, s.username_enc,
     s.authenticator_enc is not null as authenticator,
     exists (select 1 from browser_profiles p where p.login_id = l.id
         and coalesce(l.last_used, p.saved_at)
             >= now() - make_interval(days => $2)) as signed_in
     from saved_logins l
     join saved_login_secrets s on s.login_id = l.id
     where l.person = $1
     order by lower(l.label), l.id";

const GET_SQL: &str = "select l.id, l.label, l.origins, l.last_used, s.username_enc,
     s.authenticator_enc is not null as authenticator,
     exists (select 1 from browser_profiles p where p.login_id = l.id
         and coalesce(l.last_used, p.saved_at)
             >= now() - make_interval(days => $3)) as signed_in
     from saved_logins l
     join saved_login_secrets s on s.login_id = l.id
     where l.person = $1 and l.id = $2";

#[async_trait]
impl LoginStore for PostgresLogins {
    async fn list(&self, person: &str) -> Result<Vec<Login>> {
        self.ensure_schema().await?;
        let rows = sqlx::query(LIST_SQL)
            .bind(person)
            .bind(niles_logins::PROFILE_DAYS as i32)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| self.storage(e))?;
        let mut logins = Vec::with_capacity(rows.len());
        for row in &rows {
            logins.push(self.login(row, person).await?);
        }
        Ok(logins)
    }

    async fn get(&self, person: &str, id: i64) -> Result<Option<Login>> {
        self.ensure_schema().await?;
        let Some(row) = sqlx::query(GET_SQL)
            .bind(person)
            .bind(id)
            .bind(niles_logins::PROFILE_DAYS as i32)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| self.storage(e))?
        else {
            return Ok(None);
        };
        Ok(Some(self.login(&row, person).await?))
    }

    async fn add(&self, person: &str, new: &NewLogin) -> Result<Login> {
        self.ensure_schema().await?;
        let origins: Vec<&str> = new.origins.iter().map(Origin::as_str).collect();
        let mut tx = self.pool.begin().await.map_err(|e| self.storage(e))?;
        let id: i64 = sqlx::query_scalar(
            "insert into saved_logins (person, label, origins) values ($1, $2, $3)
             returning id",
        )
        .bind(person)
        .bind(&new.label)
        .bind(&origins)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| self.storage(e))?;
        let authenticator = new
            .authenticator
            .as_ref()
            .map(|key| self.seal(key.expose(), person, id, Field::Authenticator))
            .transpose()?;
        sqlx::query(
            "insert into saved_login_secrets
                 (login_id, username_enc, password_enc, authenticator_enc)
             values ($1, $2, $3, $4)",
        )
        .bind(id)
        .bind(self.seal(
            new.credentials.username.expose(),
            person,
            id,
            Field::Username,
        )?)
        .bind(self.seal(
            new.credentials.password.expose(),
            person,
            id,
            Field::Password,
        )?)
        .bind(authenticator)
        .execute(&mut *tx)
        .await
        .map_err(|e| self.storage(e))?;
        note(&mut tx, id, Used::Added)
            .await
            .map_err(|e| self.storage(e))?;
        tx.commit().await.map_err(|e| self.storage(e))?;
        Ok(Login {
            id,
            label: new.label.clone(),
            origins: new.origins.clone(),
            username: new.credentials.username.expose().to_string(),
            last_used: None,
            signed_in: false,
            authenticator: new.authenticator.is_some(),
        })
    }

    async fn update(&self, person: &str, id: i64, edit: &LoginEdit) -> Result<Option<Login>> {
        self.ensure_schema().await?;
        let mut tx = self.pool.begin().await.map_err(|e| self.storage(e))?;
        // Lock the login row: the account-changed decision and the
        // writes below read and rewrite it together.
        let Some(row) = sqlx::query(
            "select l.origins, s.username_enc
             from saved_logins l
             join saved_login_secrets s on s.login_id = l.id
             where l.id = $1 and l.person = $2
             for update of l",
        )
        .bind(id)
        .bind(person)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| self.storage(e))?
        else {
            // Not the person's: the transaction drops and rolls back.
            return Ok(None);
        };
        // The profile was signed in as this account on these sites, so
        // a new account or a different set of sites forgets it; a
        // label-only or password-only edit keeps it.
        let stored: Vec<String> = row.get("origins");
        let mut before = stored.clone();
        before.sort();
        let mut after: Vec<String> = edit
            .origins
            .iter()
            .map(|o| o.as_str().to_string())
            .collect();
        after.sort();
        let account_changed = self.open(
            &row.get::<Vec<u8>, _>("username_enc"),
            person,
            id,
            Field::Username,
        )? != edit.username.expose()
            || before != after;
        let origins: Vec<&str> = edit.origins.iter().map(Origin::as_str).collect();
        sqlx::query(
            "update saved_logins set label = $3, origins = $4, updated_at = now()
             where id = $1 and person = $2",
        )
        .bind(id)
        .bind(person)
        .bind(&edit.label)
        .bind(&origins)
        .execute(&mut *tx)
        .await
        .map_err(|e| self.storage(e))?;
        // `coalesce` keeps the sealed password when the edit carries
        // none — an empty password field means "leave it".
        let password_enc = edit
            .password
            .as_ref()
            .map(|p| self.seal(p.expose(), person, id, Field::Password))
            .transpose()?;
        sqlx::query(
            "update saved_login_secrets set username_enc = $2,
                 password_enc = coalesce($3, password_enc)
             where login_id = $1",
        )
        .bind(id)
        .bind(self.seal(edit.username.expose(), person, id, Field::Username)?)
        .bind(password_enc)
        .execute(&mut *tx)
        .await
        .map_err(|e| self.storage(e))?;
        match &edit.authenticator {
            KeyChange::Keep => {}
            KeyChange::Set(key) => {
                sqlx::query(
                    "update saved_login_secrets set authenticator_enc = $2 where login_id = $1",
                )
                .bind(id)
                .bind(self.seal(key.expose(), person, id, Field::Authenticator)?)
                .execute(&mut *tx)
                .await
                .map_err(|e| self.storage(e))?;
            }
            KeyChange::Remove => {
                sqlx::query(
                    "update saved_login_secrets set authenticator_enc = null where login_id = $1",
                )
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(|e| self.storage(e))?;
            }
        }
        note(&mut tx, id, Used::Edited)
            .await
            .map_err(|e| self.storage(e))?;
        if account_changed {
            sqlx::query(DROP_PROFILE_SQL)
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(|e| self.storage(e))?;
        }
        tx.commit().await.map_err(|e| self.storage(e))?;
        self.get(person, id).await
    }

    async fn remove(&self, person: &str, id: i64) -> Result<bool> {
        self.ensure_schema().await?;
        // The cascade takes the sealed values and the browser profile.
        let done = sqlx::query("delete from saved_logins where id = $1 and person = $2")
            .bind(id)
            .bind(person)
            .execute(&self.pool)
            .await
            .map_err(|e| self.storage(e))?;
        Ok(done.rows_affected() == 1)
    }

    async fn sign_out(&self, person: &str, id: i64) -> Result<bool> {
        self.ensure_schema().await?;
        let found: Option<i64> =
            sqlx::query_scalar("select id from saved_logins where id = $1 and person = $2")
                .bind(id)
                .bind(person)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| self.storage(e))?;
        if found.is_none() {
            return Ok(false);
        }
        // Forgetting a profile that isn't there is still a sign-out.
        let mut tx = self.pool.begin().await.map_err(|e| self.storage(e))?;
        sqlx::query("delete from browser_profiles where login_id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|e| self.storage(e))?;
        note(&mut tx, id, Used::SignedOut)
            .await
            .map_err(|e| self.storage(e))?;
        tx.commit().await.map_err(|e| self.storage(e))?;
        Ok(true)
    }

    async fn credentials(&self, person: &str, id: i64) -> Result<Option<Credentials>> {
        self.ensure_schema().await?;
        let Some(row) = sqlx::query(
            "select s.username_enc, s.password_enc
             from saved_login_secrets s
             join saved_logins l on l.id = s.login_id
             where l.id = $1 and l.person = $2",
        )
        .bind(id)
        .bind(person)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| self.storage(e))?
        else {
            return Ok(None);
        };
        Ok(Some(Credentials {
            username: Secret::new(self.open(
                &row.get::<Vec<u8>, _>("username_enc"),
                person,
                id,
                Field::Username,
            )?),
            password: Secret::new(self.open(
                &row.get::<Vec<u8>, _>("password_enc"),
                person,
                id,
                Field::Password,
            )?),
        }))
    }

    async fn save_profile(&self, person: &str, id: i64, profile: &Secret) -> Result<bool> {
        self.ensure_schema().await?;
        // One statement, so the login is marked used exactly when the
        // profile lands.
        let done = sqlx::query(
            "with l as (update saved_logins set last_used = now()
                 where id = $1 and person = $2 returning id)
             insert into browser_profiles (login_id, profile_enc)
                 select id, $3 from l
             on conflict (login_id) do update set
                 profile_enc = excluded.profile_enc, saved_at = now()",
        )
        .bind(id)
        .bind(person)
        .bind(self.seal(profile.expose(), person, id, Field::Profile)?)
        .execute(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(done.rows_affected() == 1)
    }

    async fn profile(&self, person: &str, id: i64) -> Result<Option<Secret>> {
        self.ensure_schema().await?;
        // A stale profile is forgotten, not just refused: the sweep may
        // not have run yet.
        sqlx::query(
            "with gone as (
                 delete from browser_profiles p using saved_logins l
                 where p.login_id = l.id and l.id = $1 and l.person = $2
                     and coalesce(l.last_used, p.saved_at)
                         < now() - make_interval(days => $3)
                 returning p.login_id)
             insert into login_activity (login_id, used)
                 select login_id, 'profile_dropped' from gone",
        )
        .bind(id)
        .bind(person)
        .bind(niles_logins::PROFILE_DAYS as i32)
        .execute(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        let Some(row) = sqlx::query(
            "select p.profile_enc
             from browser_profiles p
             join saved_logins l on l.id = p.login_id
             where l.id = $1 and l.person = $2",
        )
        .bind(id)
        .bind(person)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| self.storage(e))?
        else {
            return Ok(None);
        };
        Ok(Some(Secret::new(self.open(
            &row.get::<Vec<u8>, _>("profile_enc"),
            person,
            id,
            Field::Profile,
        )?)))
    }

    async fn used(&self, person: &str, id: i64, at: DateTime<Utc>) -> Result<bool> {
        self.ensure_schema().await?;
        let done =
            sqlx::query("update saved_logins set last_used = $3 where id = $1 and person = $2")
                .bind(id)
                .bind(person)
                .bind(at)
                .execute(&self.pool)
                .await
                .map_err(|e| self.storage(e))?;
        Ok(done.rows_affected() == 1)
    }

    async fn drop_stale_profiles(&self, before: DateTime<Utc>) -> Result<u64> {
        self.ensure_schema().await?;
        // One row of activity per profile dropped, so the count of
        // rows inserted is the count of profiles that went.
        let done = sqlx::query(
            "with gone as (
                 delete from browser_profiles p using saved_logins l
                 where p.login_id = l.id
                     and coalesce(l.last_used, p.saved_at) < $1
                 returning p.login_id)
             insert into login_activity (login_id, used)
                 select login_id, 'profile_dropped' from gone",
        )
        .bind(before)
        .execute(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(done.rows_affected())
    }

    async fn authenticator(&self, person: &str, id: i64) -> Result<Option<Authenticator>> {
        self.ensure_schema().await?;
        let sealed: Option<Option<Vec<u8>>> = sqlx::query_scalar(
            "select s.authenticator_enc
             from saved_login_secrets s
             join saved_logins l on l.id = s.login_id
             where l.id = $1 and l.person = $2",
        )
        .bind(id)
        .bind(person)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        let Some(Some(sealed)) = sealed else {
            return Ok(None);
        };
        let key = self.open(&sealed, person, id, Field::Authenticator)?;
        Authenticator::parse(&key).map(Some)
    }

    async fn record(&self, person: &str, id: i64, used: Used, context: &Context) -> Result<bool> {
        self.ensure_schema().await?;
        let done = sqlx::query(
            "insert into login_activity (login_id, used, site, conversation)
             select id, $3, $4, $5 from saved_logins where id = $1 and person = $2",
        )
        .bind(id)
        .bind(person)
        .bind(used.as_str())
        .bind(context.site.as_deref())
        .bind(context.conversation)
        .execute(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(done.rows_affected() == 1)
    }

    async fn activity(&self, person: &str, id: i64) -> Result<Option<Vec<Activity>>> {
        self.ensure_schema().await?;
        let found: Option<i64> =
            sqlx::query_scalar("select id from saved_logins where id = $1 and person = $2")
                .bind(id)
                .bind(person)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| self.storage(e))?;
        if found.is_none() {
            return Ok(None);
        }
        let rows = sqlx::query(
            "select at, used, site, conversation from login_activity
             where login_id = $1 order by at desc, id desc limit $2",
        )
        .bind(id)
        .bind(ACTIVITY_SHOWN as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(Some(
            rows.iter()
                .filter_map(|row| {
                    Some(Activity {
                        at: row.get("at"),
                        used: Used::parse(row.get::<&str, _>("used"))?,
                        site: row.get("site"),
                        conversation: row.get("conversation"),
                    })
                })
                .collect(),
        ))
    }

    async fn drop_old_activity(&self, before: DateTime<Utc>) -> Result<u64> {
        self.ensure_schema().await?;
        let done = sqlx::query("delete from login_activity where at < $1")
            .bind(before)
            .execute(&self.pool)
            .await
            .map_err(|e| self.storage(e))?;
        Ok(done.rows_affected())
    }

    async fn secrets(&self, person: &str) -> Result<Vec<Credentials>> {
        self.ensure_schema().await?;
        let rows = sqlx::query(
            "select l.id, s.username_enc, s.password_enc
             from saved_login_secrets s
             join saved_logins l on l.id = s.login_id
             where l.person = $1",
        )
        .bind(person)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        let mut out = Vec::with_capacity(rows.len());
        for row in &rows {
            let id: i64 = row.get("id");
            out.push(Credentials {
                username: Secret::new(self.open(
                    &row.get::<Vec<u8>, _>("username_enc"),
                    person,
                    id,
                    Field::Username,
                )?),
                password: Secret::new(self.open(
                    &row.get::<Vec<u8>, _>("password_enc"),
                    person,
                    id,
                    Field::Password,
                )?),
            });
        }
        Ok(out)
    }
}
