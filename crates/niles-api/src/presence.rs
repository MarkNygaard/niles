//! Getting tado connected, from the app rather than from a ConfigMap.
//!
//! The device flow needs a person with a browser. A service has
//! neither, so these two routes hand the job to whoever is looking at
//! Settings: one says what the state is, the other starts an
//! authorisation and gives back the code to approve.
//!
//! The pending activation lives on the source, not here, so the code
//! shown on the page is the same one the background task is polling
//! for. Two codes in flight means approving one and watching the other
//! stay pending.

use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;

type Failure = (StatusCode, String);

#[derive(serde::Serialize)]
pub struct TadoStatus {
    /// Whether there is anywhere to keep a token — which is to say,
    /// whether connecting is possible at all. False means no database.
    pub connectable: bool,
    /// Whether somebody has already approved a code.
    pub authorised: bool,
    /// Whether the feature is switched on. Separate from being
    /// connected, and second: authorising something nothing uses is
    /// harmless, turning it on before anything is connected is not.
    pub presence_enabled: bool,
    /// The code currently waiting to be approved, if any.
    pub pending: Option<PendingDto>,
}

#[derive(serde::Serialize)]
pub struct PendingDto {
    pub verification_uri: String,
    pub user_code: String,
    pub expires_at: String,
}

impl From<niles_presence::DeviceActivation> for PendingDto {
    fn from(p: niles_presence::DeviceActivation) -> Self {
        Self {
            verification_uri: p.verification_uri,
            user_code: p.user_code,
            expires_at: p.expires_at.to_rfc3339(),
        }
    }
}

/// `GET /presence/tado` — what the Settings card needs to draw itself.
///
/// Deliberately does not start an authorisation: opening a page must
/// not spend tado's daily quota.
pub async fn tado_status(State(state): State<AppState>) -> Json<TadoStatus> {
    let enabled = state
        .config
        .as_ref()
        .map(|c| c.current().presence.enabled)
        .unwrap_or(false);
    let Some(tado) = state.tado.as_ref() else {
        return Json(TadoStatus {
            connectable: false,
            authorised: false,
            presence_enabled: enabled,
            pending: None,
        });
    };
    Json(TadoStatus {
        connectable: true,
        authorised: tado.is_authorised().await.unwrap_or(false),
        presence_enabled: enabled,
        pending: tado.pending_activation().await.map(PendingDto::from),
    })
}

/// `POST /presence/tado/connect` — start an authorisation, or hand back
/// the one already running.
pub async fn tado_connect(State(state): State<AppState>) -> Result<Json<PendingDto>, Failure> {
    let Some(tado) = state.tado.as_ref() else {
        return Err((
            StatusCode::NOT_IMPLEMENTED,
            "no database, so there is nowhere to keep tado's token".into(),
        ));
    };
    if tado.is_authorised().await.unwrap_or(false) {
        return Err((StatusCode::CONFLICT, "already connected to tado".into()));
    }
    let (pending, is_new) = tado
        .ensure_activation()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("tado refused: {e}")))?;

    // Whoever started it waits on it. Nothing else polls tado in the
    // background any more: an authorisation is something a person asks
    // for, and a service quietly asking for codes nobody approves
    // spends a daily quota on nothing.
    if is_new {
        wait_for_approval(tado.clone(), pending.clone());
    }
    Ok(Json(PendingDto::from(pending)))
}

/// Poll tado until the code is approved, gives up, or runs out.
///
/// The page watches our own status rather than tado's, so this is the
/// only thing that touches tado while a code is outstanding — at the
/// interval tado asked for, and not faster.
fn wait_for_approval(
    tado: std::sync::Arc<niles_presence::TadoSource>,
    pending: niles_presence::DeviceActivation,
) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(pending.interval).await;
            match tado.finish_activation(&pending).await {
                Ok(true) => {
                    tado.clear_activation().await;
                    tracing::info!("[tado] authorised — presence can be switched on");
                    return;
                }
                Ok(false) if chrono::Utc::now() < pending.expires_at => continue,
                Ok(false) => {
                    tado.clear_activation().await;
                    tracing::warn!("[tado] the code expired unapproved");
                    return;
                }
                Err(e) => {
                    tado.clear_activation().await;
                    tracing::warn!("[tado] authorisation failed: {e}");
                    return;
                }
            }
        }
    });
}
