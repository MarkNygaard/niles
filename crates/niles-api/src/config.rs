//! HTTP surface over the [`ConfigStore`].
//!
//! Read the effective config, change values, put them back, and see what
//! changed. The UI is the intended client; `curl` is a perfectly good one
//! too, which is why the responses carry the same information a person
//! would want rather than the minimum a form needs.
//!
//! Everything here speaks JSON while the store speaks TOML. The two data
//! models line up for everything Niles configures — scalars, arrays,
//! tables — so the conversion is a re-serialization, not a translation.
//!
//! # What is deliberately absent
//!
//! No authentication. Niles is exposed on an internal route only, and
//! every endpoint here is already reachable as "control the lights" via
//! the device API next door.

use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use niles_config::{Applied, ChangeSource, ConfigStore, Reload, Revision, section_reload};
use serde::Serialize;
use std::sync::Arc;

/// The whole picture: what's in force, what was changed away from the
/// file, and whether any of it is actually live.
#[derive(Serialize)]
pub struct ConfigView {
    /// The effective config — base with overrides applied.
    effective: serde_json::Value,
    /// Only the values changed away from the base file.
    overrides: serde_json::Value,
    /// Per top-level section, whether a change takes effect without a
    /// restart. The UI needs this to avoid offering an edit that would
    /// silently do nothing.
    sections: Vec<SectionView>,
    /// False when there is no writable directory: changes apply but are
    /// lost on restart. Worth showing rather than discovering.
    persistent: bool,
}

#[derive(Serialize)]
pub struct SectionView {
    name: String,
    /// `"hot"` or `"boot"`.
    reload: &'static str,
    /// Whether this section has any override in force.
    overridden: bool,
}

/// What a write did, in the shape the caller needs to report it.
#[derive(Serialize)]
pub struct AppliedView {
    /// Revision id; 0 when nothing changed.
    revision: u64,
    /// One line per value that actually changed.
    changes: Vec<ChangeView>,
    /// Human-readable summary of the whole write.
    summary: String,
    /// True when every requested value already held that setting.
    noop: bool,
    /// Sections that changed but need a restart to take effect.
    needs_restart: Vec<String>,
}

#[derive(Serialize)]
pub struct ChangeView {
    path: String,
    from: Option<serde_json::Value>,
    to: serde_json::Value,
}

#[derive(Serialize)]
pub struct RevisionView {
    id: u64,
    at: String,
    source: String,
    summary: String,
}

/// `GET /config`
pub async fn get_config(State(state): State<AppState>) -> Response {
    let Some(store) = store(&state) else {
        return unconfigured();
    };
    let effective = store.effective_table();
    let overrides = store.overrides();

    let sections = effective
        .keys()
        .cloned()
        .map(|name| SectionView {
            reload: match section_reload(&name) {
                Reload::Hot => "hot",
                _ => "boot",
            },
            overridden: overrides.contains_key(&name),
            name,
        })
        .collect();

    match (to_json(&effective), to_json(&overrides)) {
        (Ok(effective), Ok(overrides)) => Json(ConfigView {
            effective,
            overrides,
            sections,
            persistent: store.is_persistent(),
        })
        .into_response(),
        _ => problem(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not serialize the current config",
        ),
    }
}

/// `PATCH /config` — merge a partial config document into the overrides.
///
/// The body is a partial config tree, not a list of operations:
/// `{"lighting": {"daytime_brightness": 85}}`. Anything not mentioned is
/// left alone.
pub async fn patch_config(
    State(state): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let Some(store) = store(&state) else {
        return unconfigured();
    };
    let patch: toml::Table = match serde_json::from_value(body) {
        Ok(patch) => patch,
        Err(e) => {
            return problem(
                StatusCode::BAD_REQUEST,
                &format!("body must be a config object: {e}"),
            );
        }
    };
    match store.apply(&patch, ChangeSource::Api) {
        Ok(applied) => applied_response(applied),
        // A rejected value is the caller's mistake, not a server fault,
        // and the message from `validate()` says which field and why.
        Err(e) => problem(StatusCode::UNPROCESSABLE_ENTITY, &e.to_string()),
    }
}

/// `DELETE /config/{path}` — drop one override, returning that value to
/// whatever the base file says. `path` is dotted:
/// `lighting.daytime_brightness`.
pub async fn reset_config(State(state): State<AppState>, Path(path): Path<String>) -> Response {
    let Some(store) = store(&state) else {
        return unconfigured();
    };
    match store.reset(&path, ChangeSource::Api) {
        Ok(applied) => applied_response(applied),
        Err(e) => problem(StatusCode::UNPROCESSABLE_ENTITY, &e.to_string()),
    }
}

/// `GET /config/history` — every recorded change, oldest first.
pub async fn get_history(State(state): State<AppState>) -> Response {
    let Some(store) = store(&state) else {
        return unconfigured();
    };
    let history: Vec<RevisionView> = store.history().iter().map(revision_view).collect();
    Json(history).into_response()
}

/// `POST /config/undo` — walk back one change.
///
/// 404 when there is nothing to undo: the caller asked to change
/// something and no change happened, which is not a success.
pub async fn undo_config(State(state): State<AppState>) -> Response {
    let Some(store) = store(&state) else {
        return unconfigured();
    };
    match store.undo() {
        Ok(Some(applied)) => applied_response(applied),
        Ok(None) => problem(StatusCode::NOT_FOUND, "no config changes to undo"),
        Err(e) => problem(StatusCode::UNPROCESSABLE_ENTITY, &e.to_string()),
    }
}

fn store(state: &AppState) -> Option<&Arc<ConfigStore>> {
    state.config.as_ref()
}

/// Subcommands that don't build a store still serve the device API, so
/// this is a missing capability rather than an error.
fn unconfigured() -> Response {
    problem(
        StatusCode::NOT_IMPLEMENTED,
        "this Niles instance was started without a config store",
    )
}

fn applied_response(applied: Applied) -> Response {
    let summary = applied.summary();
    let needs_restart = applied
        .needs_restart()
        .into_iter()
        .map(str::to_string)
        .collect();
    let changes = applied
        .changes
        .iter()
        .map(|c| ChangeView {
            path: c.path.clone(),
            from: c.from.as_ref().and_then(|v| to_json(v).ok()),
            to: to_json(&c.to).unwrap_or(serde_json::Value::Null),
        })
        .collect();
    Json(AppliedView {
        revision: applied.revision,
        noop: applied.is_noop(),
        changes,
        summary,
        needs_restart,
    })
    .into_response()
}

fn revision_view(r: &Revision) -> RevisionView {
    RevisionView {
        id: r.id,
        at: r.at.to_rfc3339(),
        source: match r.source {
            ChangeSource::Voice => "voice",
            _ => "api",
        }
        .to_string(),
        summary: r.summary.clone(),
    }
}

fn to_json<T: Serialize>(value: &T) -> Result<serde_json::Value, serde_json::Error> {
    serde_json::to_value(value)
}

fn problem(status: StatusCode, message: &str) -> Response {
    (status, Json(serde_json::json!({ "error": message }))).into_response()
}
