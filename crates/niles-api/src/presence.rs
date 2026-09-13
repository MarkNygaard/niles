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
    /// Whether `[presence]` names tado at all. False means there is
    /// nothing to connect to yet.
    pub configured: bool,
    /// Whether somebody has already approved a code.
    pub authorised: bool,
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
    let Some(tado) = state.tado.as_ref() else {
        return Json(TadoStatus {
            configured: false,
            authorised: false,
            pending: None,
        });
    };
    Json(TadoStatus {
        configured: true,
        authorised: tado.is_authorised().await.unwrap_or(false),
        pending: tado.pending_activation().await.map(PendingDto::from),
    })
}

/// `POST /presence/tado/connect` — start an authorisation, or hand back
/// the one already running.
pub async fn tado_connect(State(state): State<AppState>) -> Result<Json<PendingDto>, Failure> {
    let Some(tado) = state.tado.as_ref() else {
        return Err((
            StatusCode::NOT_IMPLEMENTED,
            "tado is not configured — turn presence on first".into(),
        ));
    };
    if tado.is_authorised().await.unwrap_or(false) {
        return Err((StatusCode::CONFLICT, "already connected to tado".into()));
    }
    let pending = tado
        .ensure_activation()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("tado refused: {e}")))?;
    Ok(Json(PendingDto::from(pending)))
}
