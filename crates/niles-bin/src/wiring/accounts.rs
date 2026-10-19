//! Secrets, sign-in sessions and the outside accounts: calendars, push, Strava, Tado, weather.

use anyhow::Context;
use niles_config::Config;
use std::sync::Arc;

/// Build an `OpenMeteoClient`.
/// Returns `None` when client construction fails.
pub(crate) fn build_weather_client() -> Option<Arc<niles_weather::OpenMeteoClient>> {
    niles_weather::OpenMeteoClient::new(niles_weather::OpenMeteoConfig::default())
        .inspect_err(|e| {
            tracing::warn!("Failed to build Open-Meteo client: {e}; weather tool disabled")
        })
        .ok()
        .map(Arc::new)
}

/// Choose where config overrides are kept.
///
/// A database wins over a directory, and a directory over memory. The
/// order matters on Kubernetes: a local volume pins the pod to one node,
/// so `[database]` is the option that lets Niles reschedule freely.
///
/// Only a *misconfigured* database is fatal — a DSN that will not parse,
/// or an env var that is not set. One that merely cannot be reached right
/// now is not: the store reports it and retries in the background.
/// Where enrolled voices live.
///
/// The database when there is one, because a voice print is under a
/// kilobyte and the alternative is a volume — node-local ones pin the
/// pod and block drains, network ones make recognition depend on a NAS.
/// Falls back to a directory, which is what a laptop has.
/// Where the tado refresh token lives.
///
/// Only the database will do. tado rotates the token on every use, so
/// this is written several times an hour — a Secret cannot hold it, and
/// a file would need a volume, which is the thing `[database]` exists
/// to avoid. No database means no tado presence, and the caller says so
/// rather than starting something that will ask for a browser on every
/// restart.
/// The encrypted secret store, when there is a database and a key.
///
/// Absent is normal and not an error: it means secrets come from
/// environment variables, which is what every install did before this
/// existed. What is *not* normal is a key that cannot be used, so that
/// is reported rather than quietly falling back — a Niles that ignores
/// a key somebody set would leave them wondering why Settings will not
/// save anything.
pub(crate) fn build_secret_store(cfg: &Config) -> Option<Arc<niles_db::PostgresSecrets>> {
    let database = cfg.database.as_ref()?;
    let key = std::env::var(&database.secret_key_env).ok()?;
    if key.trim().is_empty() {
        return None;
    }
    let url = database.resolve_url().ok()?;
    let backend = niles_db::PostgresBackend::connect_lazy(&url, database.max_connections).ok()?;
    let describe = backend.describe_target();
    match niles_db::PostgresSecrets::new(backend.pool(), describe, &key) {
        Ok(store) => Some(Arc::new(store)),
        Err(e) => {
            tracing::error!(
                "[secrets] {} is set but unusable, so secrets typed into the app                  cannot be read: {e}",
                database.secret_key_env
            );
            None
        }
    }
}

/// Delete the keys of providers Niles no longer ships, once, at startup.
pub(crate) async fn forget_retired_provider_keys(
    secrets: &niles_db::PostgresSecrets,
    keys: &[String],
) {
    if keys.is_empty() {
        return;
    }
    for key in keys {
        match secrets.clear(key).await {
            Ok(()) => tracing::info!("[secrets] deleted {key}: Niles no longer has that provider"),
            Err(e) => tracing::warn!("[secrets] could not delete {key}: {e}"),
        }
    }
    match secrets.load_all().await {
        Ok(values) => niles_config::secrets::load(values),
        Err(e) => tracing::error!("[secrets] deleted, but reloading failed: {e}"),
    }
}

/// The key the sign-in cookie is signed with, made once and kept sealed
/// when nobody has set one. It is a random string nobody needs to know,
/// and asking the household to invent it was the last thing the
/// Credentials page was for. Only after the stored secrets were read:
/// made over one that could not be read, it would sign everybody out.
pub(crate) async fn ensure_session_key(cfg: &Config, secrets: &niles_db::PostgresSecrets) {
    if cfg.auth.resolve_session_secret().is_ok() {
        return;
    }
    let key = format!(
        "{:032x}{:032x}",
        rand::random::<u128>(),
        rand::random::<u128>()
    );
    match secrets.set("auth.session_secret", &key).await {
        Ok(()) => match secrets.load_all().await {
            Ok(values) => {
                niles_config::secrets::load(values);
                tracing::info!("[secrets] made a session key and kept it");
            }
            Err(e) => tracing::error!("[secrets] made a session key, but reloading failed: {e}"),
        },
        Err(e) => tracing::error!("[secrets] could not keep a new session key: {e}"),
    }
}

/// Each person's calendars, when there is a database and a secrets key
/// to seal their sign-ins with. Without both there are none: a refresh
/// token is a standing key to somebody's calendar, and is not kept in
/// the clear.
pub(crate) fn build_calendars(cfg: &Config) -> Option<Arc<niles_calendar::Calendars>> {
    let database = cfg.database.as_ref()?;
    let key = std::env::var(&database.secret_key_env).ok()?;
    if key.trim().is_empty() {
        return None;
    }
    let url = database.resolve_url().ok()?;
    let backend = niles_db::PostgresBackend::connect_lazy(&url, database.max_connections).ok()?;
    let describe = backend.describe_target();
    match niles_db::PostgresCalendars::new(backend.pool(), describe, &key) {
        Ok(store) => Some(Arc::new(niles_calendar::Calendars::new(
            Arc::new(store),
            Arc::new(niles_calendar::Network::new(reqwest::Client::new())),
        ))),
        Err(e) => {
            tracing::error!("[calendars] unavailable: {e}");
            None
        }
    }
}

/// Notifications on each person's devices (Web Push), when there is a
/// database and a secrets key to seal the server's push key with. A
/// person turning them on in the app is the switch.
pub(crate) fn build_push(cfg: &Config) -> Option<Arc<niles_webpush::WebPush>> {
    let database = cfg.database.as_ref()?;
    let key = std::env::var(&database.secret_key_env).ok()?;
    if key.trim().is_empty() {
        return None;
    }
    let url = database.resolve_url().ok()?;
    let backend = niles_db::PostgresBackend::connect_lazy(&url, database.max_connections).ok()?;
    // A push service that does not answer in this long is not going to;
    // the job that is waiting to say it finished should not wait longer.
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .ok()?;
    match niles_db::PostgresPush::new(backend.pool(), backend.describe_target(), &key) {
        Ok(store) => Some(Arc::new(niles_webpush::WebPush::new(Arc::new(store), http))),
        Err(e) => {
            tracing::error!("[push] unavailable: {e}");
            None
        }
    }
}

/// Telling each person something: kept for their Notifications page
/// whenever there is a database, and pushed when `push` is there too.
pub(crate) fn build_notify(
    cfg: &Config,
    push: Option<Arc<niles_webpush::WebPush>>,
    bus: niles_core::EventBus,
) -> Arc<crate::notify::Notify> {
    let inbox = cfg
        .database
        .as_ref()
        .and_then(|database| {
            let url = database.resolve_url().ok()?;
            niles_db::PostgresBackend::connect_lazy(&url, database.max_connections).ok()
        })
        .map(|backend| {
            Arc::new(niles_db::PostgresPersonNotifications::new(
                backend.pool(),
                backend.describe_target(),
            )) as Arc<dyn niles_webpush::Inbox>
        });
    if inbox.is_none() {
        tracing::warn!("[notify] no database; nobody's notifications are kept");
    }
    Arc::new(crate::notify::Notify::new(inbox, push, bus))
}

/// Each person's Strava and the workouts it brings, when there is a
/// database and a secrets key to seal their app secrets and tokens
/// with. A person connecting their own Strava is the switch.
///
/// What each sync brings goes to `arrivals`, for the training plans.
pub(crate) fn build_strava(
    cfg: &Config,
    arrivals: tokio::sync::mpsc::UnboundedSender<niles_training::Arrived>,
) -> Option<Arc<niles_training::Strava>> {
    let database = cfg.database.as_ref()?;
    let key = std::env::var(&database.secret_key_env).ok()?;
    if key.trim().is_empty() {
        return None;
    }
    let url = database.resolve_url().ok()?;
    let backend = niles_db::PostgresBackend::connect_lazy(&url, database.max_connections).ok()?;
    match niles_db::PostgresTraining::new(backend.pool(), backend.describe_target(), &key) {
        Ok(store) => Some(Arc::new(
            niles_training::Strava::new(
                Arc::new(store),
                niles_training::strava::Client::new(reqwest::Client::new()),
            )
            .with_arrivals(arrivals),
        )),
        Err(e) => {
            tracing::error!("[strava] unavailable: {e}");
            None
        }
    }
}

pub(crate) fn build_tado_tokens(
    cfg: &Config,
) -> anyhow::Result<Option<Arc<dyn niles_presence::TokenStore>>> {
    let Some(database) = &cfg.database else {
        return Ok(None);
    };
    let url = database
        .resolve_url()
        .context("resolving the database connection string")?;
    let backend =
        niles_db::PostgresBackend::connect_lazy(&url, database.max_connections).map_err(|_| {
            anyhow::anyhow!(
                "the connection string in {} is not a valid database URL",
                database.url_env
            )
        })?;
    let describe = backend.describe_target();
    Ok(Some(Arc::new(niles_db::PostgresTadoTokens::new(
        backend.pool(),
        describe,
    ))))
}
