//! Third-party plugins, in Postgres: what is installed, each plugin's
//! private storage, and the devices Niles holds for it.
//!
//! Three tables, created on first use:
//!
//! - `plugins`: the approved manifest, where it came from, the
//!   component's bytes and sha256, the version before it (kept for one
//!   rollback), and whether it is enabled. A restart never downloads
//!   anything: what was approved is what runs.
//! - `plugin_storage`: the plugin's own key-value store, 1 MiB each.
//! - `plugin_devices`: the devices it registered, so they stay in their
//!   rooms, unavailable, while the plugin is stopped, and the rules that
//!   name them stay valid.
//!
//! The last two reference the first with `on delete cascade`, so
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
";

/// An installed plugin, as kept.
#[derive(Debug, Clone)]
pub struct PluginRow {
    pub id: String,
    pub manifest: String,
    pub repo: Option<String>,
    pub version: String,
    pub component: Vec<u8>,
    pub sha256: String,
    pub enabled: bool,
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
            "select id, manifest, repo, version, component, sha256, enabled
             from plugins where enabled order by installed_at, id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| self.storage(e))?;
        Ok(rows
            .iter()
            .map(|r| PluginRow {
                id: r.get("id"),
                manifest: r.get("manifest"),
                repo: r.get("repo"),
                version: r.get("version"),
                component: r.get("component"),
                sha256: r.get("sha256"),
                enabled: r.get("enabled"),
            })
            .collect())
    }

    /// Add a plugin, or replace one with the same id and keep what it
    /// replaced as the previous version. Enabled either way.
    ///
    /// Internal: what an approved install writes. Nothing here checks
    /// the manifest or the component; the caller has.
    pub async fn install(
        &self,
        id: &str,
        version: &str,
        manifest: &str,
        repo: Option<&str>,
        component: &[u8],
    ) -> Result<(), String> {
        self.ensure_schema().await?;
        let sha256 = hex_digest(component);
        sqlx::query(
            "insert into plugins (id, manifest, repo, version, component, sha256)
             values ($1, $2, $3, $4, $5, $6)
             on conflict (id) do update set
                 previous_version = plugins.version,
                 previous_manifest = plugins.manifest,
                 previous_component = plugins.component,
                 previous_sha256 = plugins.sha256,
                 manifest = excluded.manifest,
                 repo = excluded.repo,
                 version = excluded.version,
                 component = excluded.component,
                 sha256 = excluded.sha256,
                 enabled = true,
                 updated_at = now()",
        )
        .bind(id)
        .bind(manifest)
        .bind(repo)
        .bind(version)
        .bind(component)
        .bind(&sha256)
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
            "select device, class, color_temp, rgb, controllable
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
            })
            .collect())
    }

    pub async fn put_device(&self, plugin: &str, device: &PluginDeviceRow) -> Result<(), String> {
        self.ensure_schema().await?;
        sqlx::query(
            "insert into plugin_devices (plugin_id, device, class, color_temp, rgb, controllable)
             values ($1, $2, $3, $4, $5, $6)
             on conflict (plugin_id, device) do update set
                 class = excluded.class,
                 color_temp = excluded.color_temp,
                 rgb = excluded.rgb,
                 controllable = excluded.controllable",
        )
        .bind(plugin)
        .bind(&device.device)
        .bind(&device.class)
        .bind(device.color_temp)
        .bind(device.rgb)
        .bind(device.controllable)
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

/// The component's sha256, as lowercase hex: what an install pins.
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
    fn the_digest_is_lowercase_hex_sha256() {
        assert_eq!(
            hex_digest(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
