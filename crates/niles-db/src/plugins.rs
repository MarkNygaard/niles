//! Third-party plugins, in Postgres: what is installed, each plugin's
//! private storage, the devices Niles holds for it, and what the
//! add-on library counted.
//!
//! Four tables, created on first use:
//!
//! - `plugins`: the approved manifest, where it came from (the repo, the
//!   release's tag and page), the component's bytes and sha256, the
//!   manifest's sha256, the sanitised logo, the version before it (kept
//!   for one rollback), whether it is enabled, and the newer release the
//!   last update check found, and the notice about the last automatic
//!   rollback. A restart never downloads anything: what was approved is
//!   what runs.
//! - `plugin_storage`: the plugin's own key-value store, 1 MiB each.
//! - `plugin_devices`: the devices it registered, so they stay in their
//!   rooms, unavailable, while the plugin is stopped, and the rules that
//!   name them stay valid.
//! - `plugin_installs_counted`: the repos whose install this house has
//!   counted, by downloading their release's `niles-installed` once.
//!   Not tied to `plugins`, so removing a plugin keeps it and
//!   installing it again is not counted again.
//!
//! The middle two reference the first with `on delete cascade`, so
//! removing a plugin removes its data with it.

use sha2::{Digest, Sha256};
use sqlx::{Row, postgres::PgPool};

const SCHEMA: &str = "
create table if not exists plugins (
    id                 text primary key,
    manifest           text not null,
    repo               text,
    version            text not null,
    component          bytea not null,
    sha256             text not null,
    previous_version   text,
    previous_manifest  text,
    previous_component bytea,
    previous_sha256    text,
    enabled            boolean not null default true,
    installed_at       timestamptz not null default now(),
    updated_at         timestamptz not null default now()
);
-- Added with installing from GitHub, after the table shipped.
alter table plugins add column if not exists release_tag text;
alter table plugins add column if not exists release_url text;
alter table plugins add column if not exists manifest_sha256 text;
alter table plugins add column if not exists logo text;
alter table plugins add column if not exists previous_release_tag text;
alter table plugins add column if not exists previous_release_url text;
alter table plugins add column if not exists previous_manifest_sha256 text;
alter table plugins add column if not exists previous_logo text;
alter table plugins add column if not exists update_available text;
-- Added with the plugin's panel in the app: an update that failed and
-- was rolled back, said until the next update or rollback by hand.
alter table plugins add column if not exists notice text;
create table if not exists plugin_storage (
    plugin_id text not null references plugins (id) on delete cascade,
    key       text not null,
    value     text not null,
    primary key (plugin_id, key)
);
create table if not exists plugin_devices (
    plugin_id    text not null references plugins (id) on delete cascade,
    device       text not null,
    class        text not null,
    color_temp   boolean not null default false,
    rgb          boolean not null default false,
    controllable boolean not null default false,
    primary key (plugin_id, device)
);
-- Added with interface 0.2.1: a light kept out of its room's lights,
-- and a device of any class that has reported an action.
alter table plugin_devices add column if not exists room_light boolean not null default true;
alter table plugin_devices add column if not exists reports_actions boolean not null default false;
-- Added with the add-on library: one row per repo whose install was
-- counted. No reference to plugins, so a removal keeps it.
create table if not exists plugin_installs_counted (
    repo       text primary key,
    counted_at timestamptz not null default now()
);
";

/// A plugin's manifest and component, to start it.
#[derive(Debug, Clone)]
pub struct PluginRow {
    pub id: String,
    pub manifest: String,
    pub version: String,
    pub component: Vec<u8>,
    pub sha256: String,
}

/// What an approved install or update writes.
#[derive(Debug, Clone, Copy)]
pub struct PluginInstall<'a> {
    pub id: &'a str,
    pub version: &'a str,
    pub manifest: &'a str,
    pub component: &'a [u8],
    /// The sanitised SVG.
    pub logo: Option<&'a str>,
    /// `https://github.com/<owner>/<repo>`; none for a plugin installed
    /// from a file.
    pub repo: Option<&'a str>,
    pub release_tag: Option<&'a str>,
    pub release_url: Option<&'a str>,
}

/// An installed plugin as listed: everything but the component's bytes.
#[derive(Debug, Clone)]
pub struct PluginMeta {
    pub id: String,
    pub manifest: String,
    pub version: String,
    pub repo: Option<String>,
    pub release_tag: Option<String>,
    pub release_url: Option<String>,
    pub sha256: String,
    pub manifest_sha256: Option<String>,
    pub has_logo: bool,
    pub enabled: bool,
    pub installed_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub previous_version: Option<String>,
    /// What the last update check found, as its caller wrote it.
    pub update_available: Option<String>,
    /// Why the version before came back, after an update failed.
    pub notice: Option<String>,
}

/// A device Niles holds for a plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginDeviceRow {
    /// `room/name`, without the plugin's source.
    pub device: String,
    /// `light`, `outlet`, `switch`, `contact`, `motion`, `cover`, `sensor`.
    pub class: String,
    pub color_temp: bool,
    pub rgb: bool,
    pub controllable: bool,
    /// False for a light kept out of its room's lights.
    pub room_light: bool,
    /// Whether it has reported an action.
    pub reports_actions: bool,
}

pub struct PostgresPlugins {
    pool: PgPool,
    describe: String,
    schema: tokio::sync::OnceCell<()>,
}

impl PostgresPlugins {
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
        format!("plugin storage ({}): {e}", self.describe)
    }

    /// Every enabled plugin, oldest install first.
    pub async fn enabled(&self) -> Result<Vec<PluginRow>, String> {
        self.ensure_schema().await?;
        let rows = sqlx::query(
            "select id, manifest, version, component, sha256
             from plugins where enabled order by installed_at, id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(rows.iter().map(plugin_row).collect())
    }

    /// Every installed plugin, enabled or not, oldest install first.
    pub async fn all(&self) -> Result<Vec<PluginMeta>, String> {
        self.ensure_schema().await?;
        let rows = sqlx::query(ALL)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| self.storage(e))?;
        Ok(rows
            .iter()
            .map(|r| PluginMeta {
                id: r.get("id"),
                manifest: r.get("manifest"),
                version: r.get("version"),
                repo: r.get("repo"),
                release_tag: r.get("release_tag"),
                release_url: r.get("release_url"),
                sha256: r.get("sha256"),
                manifest_sha256: r.get("manifest_sha256"),
                has_logo: r.get("has_logo"),
                enabled: r.get("enabled"),
                installed_at: r.get("installed_at"),
                updated_at: r.get("updated_at"),
                previous_version: r.get("previous_version"),
                update_available: r.get("update_available"),
                notice: r.get("notice"),
            })
            .collect())
    }

    /// One plugin's manifest and component, enabled or not.
    pub async fn package(&self, id: &str) -> Result<Option<PluginRow>, String> {
        self.ensure_schema().await?;
        let row = sqlx::query(
            "select id, manifest, version, component, sha256 from plugins where id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(row.as_ref().map(plugin_row))
    }

    /// Add a plugin, or replace one with the same id and keep what it
    /// replaced as the previous version. Enabled either way, and the
    /// update the last check found is forgotten: this is it, or newer.
    ///
    /// Internal: what an approved install writes. Nothing here checks
    /// the manifest or the component; the caller has.
    pub async fn install(&self, plugin: &PluginInstall<'_>) -> Result<(), String> {
        self.ensure_schema().await?;
        sqlx::query(INSTALL)
            .bind(plugin.id)
            .bind(plugin.manifest)
            .bind(plugin.repo)
            .bind(plugin.version)
            .bind(plugin.component)
            .bind(hex_digest(plugin.component))
            .bind(hex_digest(plugin.manifest.as_bytes()))
            .bind(plugin.logo)
            .bind(plugin.release_tag)
            .bind(plugin.release_url)
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(|e| self.storage(e))
    }

    /// Put the previous version back in place of the current one, which
    /// is not kept: a rollback happens once. `None` when there is no
    /// previous version.
    pub async fn roll_back(&self, id: &str) -> Result<Option<PluginRow>, String> {
        self.ensure_schema().await?;
        let row = sqlx::query(ROLL_BACK)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| self.storage(e))?;
        Ok(row.as_ref().map(plugin_row))
    }

    /// False when there is no such plugin.
    pub async fn set_enabled(&self, id: &str, enabled: bool) -> Result<bool, String> {
        self.ensure_schema().await?;
        sqlx::query("update plugins set enabled = $2 where id = $1")
            .bind(id)
            .bind(enabled)
            .execute(&self.pool)
            .await
            .map(|done| done.rows_affected() > 0)
            .map_err(|e| self.storage(e))
    }

    /// Record what the last update check found, or that it found nothing.
    pub async fn set_update(&self, id: &str, update: Option<&str>) -> Result<(), String> {
        self.ensure_schema().await?;
        sqlx::query("update plugins set update_available = $2 where id = $1")
            .bind(id)
            .bind(update)
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(|e| self.storage(e))
    }

    /// Say what happened to the plugin, or forget it.
    pub async fn set_notice(&self, id: &str, notice: Option<&str>) -> Result<(), String> {
        self.ensure_schema().await?;
        sqlx::query(SET_NOTICE)
            .bind(id)
            .bind(notice)
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(|e| self.storage(e))
    }

    /// Note that an install from `repo` was counted: true the first time.
    pub async fn count_install(&self, repo: &str) -> Result<bool, String> {
        self.ensure_schema().await?;
        sqlx::query(COUNT_INSTALL)
            .bind(repo)
            .execute(&self.pool)
            .await
            .map(|done| done.rows_affected() == 1)
            .map_err(|e| self.storage(e))
    }

    pub async fn logo(&self, id: &str) -> Result<Option<String>, String> {
        self.ensure_schema().await?;
        sqlx::query_scalar::<_, Option<String>>("select logo from plugins where id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map(Option::flatten)
            .map_err(|e| self.storage(e))
    }

    /// Delete the plugin, and with it its storage and its devices.
    pub async fn remove(&self, id: &str) -> Result<(), String> {
        self.ensure_schema().await?;
        sqlx::query("delete from plugins where id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(|e| self.storage(e))
    }

    pub async fn get(&self, plugin: &str, key: &str) -> Result<Option<String>, String> {
        self.ensure_schema().await?;
        sqlx::query_scalar("select value from plugin_storage where plugin_id = $1 and key = $2")
            .bind(plugin)
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| self.storage(e))
    }

    /// Store `value` under `key` unless the plugin's keys and values
    /// would then come to more than `quota` bytes. False when it would.
    ///
    /// One statement, so the sum and the write cannot disagree.
    pub async fn set(
        &self,
        plugin: &str,
        key: &str,
        value: &str,
        quota: usize,
    ) -> Result<bool, String> {
        self.ensure_schema().await?;
        let written = sqlx::query(
            "insert into plugin_storage (plugin_id, key, value)
             select $1, $2, $3
             where (select coalesce(sum(octet_length(key) + octet_length(value)), 0)
                    from plugin_storage where plugin_id = $1 and key <> $2)
                   + octet_length($2) + octet_length($3) <= $4
             on conflict (plugin_id, key) do update set value = excluded.value",
        )
        .bind(plugin)
        .bind(key)
        .bind(value)
        .bind(quota as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(written.rows_affected() > 0)
    }

    pub async fn delete(&self, plugin: &str, key: &str) -> Result<(), String> {
        self.ensure_schema().await?;
        sqlx::query("delete from plugin_storage where plugin_id = $1 and key = $2")
            .bind(plugin)
            .bind(key)
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(|e| self.storage(e))
    }

    /// Keys starting with `prefix`, in byte order, so date-stamped keys
    /// list oldest first whatever the database's collation.
    pub async fn keys(&self, plugin: &str, prefix: &str) -> Result<Vec<String>, String> {
        self.ensure_schema().await?;
        sqlx::query_scalar(
            "select key from plugin_storage
             where plugin_id = $1 and left(key, length($2)) = $2
             order by key collate \"C\"",
        )
        .bind(plugin)
        .bind(prefix)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| self.storage(e))
    }

    pub async fn devices(&self, plugin: &str) -> Result<Vec<PluginDeviceRow>, String> {
        self.ensure_schema().await?;
        let rows = sqlx::query(
            "select device, class, color_temp, rgb, controllable, room_light, reports_actions
             from plugin_devices where plugin_id = $1 order by device",
        )
        .bind(plugin)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(rows
            .iter()
            .map(|r| PluginDeviceRow {
                device: r.get("device"),
                class: r.get("class"),
                color_temp: r.get("color_temp"),
                rgb: r.get("rgb"),
                controllable: r.get("controllable"),
                room_light: r.get("room_light"),
                reports_actions: r.get("reports_actions"),
            })
            .collect())
    }

    pub async fn put_device(&self, plugin: &str, device: &PluginDeviceRow) -> Result<(), String> {
        self.ensure_schema().await?;
        sqlx::query(
            "insert into plugin_devices
                 (plugin_id, device, class, color_temp, rgb, controllable, room_light, reports_actions)
             values ($1, $2, $3, $4, $5, $6, $7, $8)
             on conflict (plugin_id, device) do update set
                 class = excluded.class,
                 color_temp = excluded.color_temp,
                 rgb = excluded.rgb,
                 controllable = excluded.controllable,
                 room_light = excluded.room_light,
                 reports_actions = excluded.reports_actions",
        )
        .bind(plugin)
        .bind(&device.device)
        .bind(&device.class)
        .bind(device.color_temp)
        .bind(device.rgb)
        .bind(device.controllable)
        .bind(device.room_light)
        .bind(device.reports_actions)
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(|e| self.storage(e))
    }

    pub async fn remove_device(&self, plugin: &str, device: &str) -> Result<(), String> {
        self.ensure_schema().await?;
        sqlx::query("delete from plugin_devices where plugin_id = $1 and device = $2")
            .bind(plugin)
            .bind(device)
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(|e| self.storage(e))
    }
}

const INSTALL: &str = "insert into plugins (id, manifest, repo, version, component, sha256,
                                  manifest_sha256, logo, release_tag, release_url)
             values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
             on conflict (id) do update set
                 previous_version = plugins.version,
                 previous_manifest = plugins.manifest,
                 previous_component = plugins.component,
                 previous_sha256 = plugins.sha256,
                 previous_manifest_sha256 = plugins.manifest_sha256,
                 previous_logo = plugins.logo,
                 previous_release_tag = plugins.release_tag,
                 previous_release_url = plugins.release_url,
                 manifest = excluded.manifest,
                 repo = excluded.repo,
                 version = excluded.version,
                 component = excluded.component,
                 sha256 = excluded.sha256,
                 manifest_sha256 = excluded.manifest_sha256,
                 logo = excluded.logo,
                 release_tag = excluded.release_tag,
                 release_url = excluded.release_url,
                 update_available = null,
                 notice = null,
                 enabled = true,
                 updated_at = now()";

const ROLL_BACK: &str = "update plugins set
                 version = previous_version,
                 manifest = previous_manifest,
                 component = previous_component,
                 sha256 = previous_sha256,
                 manifest_sha256 = previous_manifest_sha256,
                 logo = previous_logo,
                 release_tag = previous_release_tag,
                 release_url = previous_release_url,
                 previous_version = null,
                 previous_manifest = null,
                 previous_component = null,
                 previous_sha256 = null,
                 previous_manifest_sha256 = null,
                 previous_logo = null,
                 previous_release_tag = null,
                 previous_release_url = null,
                 update_available = null,
                 notice = null,
                 updated_at = now()
             where id = $1 and previous_component is not null
             returning id, manifest, version, component, sha256";

const ALL: &str = "select id, manifest, version, repo, release_tag, release_url, sha256,
                    manifest_sha256, logo is not null as has_logo, enabled,
                    installed_at, updated_at, previous_version, update_available, notice
             from plugins order by installed_at, id";

const SET_NOTICE: &str = "update plugins set notice = $2 where id = $1";

const COUNT_INSTALL: &str =
    "insert into plugin_installs_counted (repo) values ($1) on conflict (repo) do nothing";

fn plugin_row(r: &sqlx::postgres::PgRow) -> PluginRow {
    PluginRow {
        id: r.get("id"),
        manifest: r.get("manifest"),
        version: r.get("version"),
        component: r.get("component"),
        sha256: r.get("sha256"),
    }
}

/// A sha256 as lowercase hex: what an install pins.
pub fn hex_digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_notice_has_a_column_and_is_listed() {
        assert!(SCHEMA.contains("add column if not exists notice text"));
        assert!(ALL.contains("notice"));
        assert!(SET_NOTICE.contains("set notice = $2 where id = $1"));
    }

    #[test]
    fn the_count_of_installs_has_its_own_table_and_outlives_the_plugin() {
        let marker = "create table if not exists plugin_installs_counted";
        assert!(SCHEMA.contains(marker));
        let start = SCHEMA.find(marker).unwrap();
        let end = SCHEMA[start..].find(");").map(|i| start + i).unwrap();
        assert!(
            !SCHEMA[start..end].contains("references"),
            "no reference to plugins, so a removal keeps the count"
        );
        assert!(COUNT_INSTALL.contains("on conflict (repo) do nothing"));
    }

    #[test]
    fn an_update_and_a_rollback_forget_the_notice() {
        // An automatic rollback writes its notice after the rollback,
        // so the rollback clearing it only clears an older one.
        assert!(INSTALL.contains("notice = null"), "{INSTALL}");
        assert!(ROLL_BACK.contains("notice = null"), "{ROLL_BACK}");
    }

    #[test]
    fn the_digest_is_lowercase_hex_sha256() {
        assert_eq!(
            hex_digest(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
