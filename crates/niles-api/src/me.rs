//! The signed-in person's own page: what Niles knows about them.
//!
//! Theirs alone. Everything here is reached through the session, never
//! through a name in the path, so there is no URL that shows somebody
//! else's notes — and `/voices`, which everybody signed in can read,
//! does not carry them.
//!
//! Niles knows people by voice, so the profile lives on the voice the
//! allowlist links them to (`auth.allowed[].speaker`). Somebody without
//! one has a page with only their phone on it, and is told why.

use crate::state::AppState;
use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use serde::{Deserialize, Serialize};

type Failure = (StatusCode, String);

#[derive(Debug, Clone, Serialize)]
pub struct MeDto {
    pub email: String,
    /// The voice they are linked to. `None` means there is no profile
    /// yet: nobody has linked this address to a voice under People.
    pub speaker: Option<String>,
    pub display_name: Option<String>,
    pub spoken_as: Option<String>,
    pub address_as: Option<String>,
    /// Their own USER.md.
    pub notes: Option<String>,
    /// "MM-DD".
    pub birthday: Option<String>,
    /// The phone presence follows them by, if one is paired.
    pub phone: Option<String>,
}

/// Each field: absent leaves it, empty clears it.
#[derive(Debug, Default, Deserialize)]
pub struct Update {
    #[serde(default)]
    pub spoken_as: Option<String>,
    #[serde(default)]
    pub address_as: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub birthday: Option<String>,
}

/// Who is asking, and the config that says who they are.
fn caller(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<(std::sync::Arc<niles_config::Config>, String), Failure> {
    let store = state.config.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "this Niles instance has no people configured".to_string(),
    ))?;
    let config = store.current();
    let who = crate::auth::signed_in_as(headers, &config).ok_or((
        StatusCode::UNAUTHORIZED,
        "sign in to see your own page".to_string(),
    ))?;
    Ok((config, who))
}

fn roster(state: &AppState) -> Result<&dyn niles_recognition::VoiceRoster, Failure> {
    state.voices.as_deref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "speaker recognition is not configured".into(),
    ))
}

fn unreachable(what: &str) -> impl Fn(niles_recognition::Error) -> Failure + '_ {
    move |e| (StatusCode::BAD_GATEWAY, format!("could not {what}: {e}"))
}

/// "MM-DD" for a day that exists, or why not.
pub fn valid_birthday(birthday: &str) -> Result<String, String> {
    let bad = || format!("{birthday:?} is not a birthday — give it as MM-DD");
    let (m, d) = birthday.split_once('-').ok_or_else(bad)?;
    let (m, d): (u32, u32) = (m.parse().map_err(|_| bad())?, d.parse().map_err(|_| bad())?);
    // A leap year, so 29 February is somebody's birthday.
    chrono::NaiveDate::from_ymd_opt(2024, m, d).ok_or_else(bad)?;
    Ok(format!("{m:02}-{d:02}"))
}

/// `GET /me`
pub async fn get(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<MeDto>, Failure> {
    let (config, email) = caller(&state, &headers)?;
    let person = config.auth.person_for(&email);
    let speaker = person.and_then(|p| p.speaker.clone());
    let voice = match (&speaker, state.voices.as_deref()) {
        (Some(slug), Some(roster)) => roster
            .voices()
            .await
            .map_err(unreachable("read your voice"))?
            .into_iter()
            .find(|v| &v.speaker == slug),
        _ => None,
    };
    Ok(Json(MeDto {
        email,
        // Only a voice that exists: a link to one since forgotten is no
        // profile, and a page offering to edit it would fail every save.
        speaker: voice.as_ref().map(|v| v.speaker.clone()),
        display_name: voice.as_ref().map(|v| v.display_name.clone()),
        spoken_as: voice.as_ref().and_then(|v| v.spoken_as.clone()),
        address_as: voice.as_ref().and_then(|v| v.address_as.clone()),
        notes: voice.as_ref().and_then(|v| v.notes.clone()),
        birthday: voice.as_ref().and_then(|v| v.birthday.clone()),
        phone: person.and_then(|p| p.device_mac.clone()),
    }))
}

/// `PUT /me`
pub async fn update(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Update>,
) -> Result<StatusCode, Failure> {
    let (config, email) = caller(&state, &headers)?;
    let speaker = config
        .auth
        .person_for(&email)
        .and_then(|p| p.speaker.clone())
        .ok_or((
            StatusCode::CONFLICT,
            "your sign-in is not linked to a voice yet — link it under People".to_string(),
        ))?;
    let roster = roster(&state)?;

    // Every field checked before any is written, so a bad birthday does
    // not leave the notes saved and the page saying it failed.
    let birthday = match body.birthday.as_deref().map(str::trim) {
        None => None,
        Some("") => Some(None),
        Some(b) => Some(Some(
            valid_birthday(b).map_err(|e| (StatusCode::BAD_REQUEST, e))?,
        )),
    };
    if let Some(notes) = &body.notes
        && notes.trim().chars().count() > niles_recognition::NOTES_LIMIT
    {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "the notes are over {} characters — Niles reads them on every turn, so keep them to a page",
                niles_recognition::NOTES_LIMIT
            ),
        ));
    }

    let cleared = |s: &str| {
        let s = s.trim();
        (!s.is_empty()).then(|| s.to_string())
    };
    if let Some(spoken) = &body.spoken_as {
        roster
            .set_spoken_as(&speaker, cleared(spoken).as_deref())
            .await
            .map_err(unreachable("set how to say your name"))?;
    }
    if let Some(address) = &body.address_as {
        roster
            .set_address_as(&speaker, cleared(address).as_deref())
            .await
            .map_err(unreachable("set how to address you"))?;
    }
    if let Some(notes) = &body.notes {
        roster
            .set_notes(&speaker, cleared(notes).as_deref())
            .await
            .map_err(unreachable("save your notes"))?;
    }
    if let Some(birthday) = birthday {
        roster
            .set_birthday(&speaker, birthday.as_deref())
            .await
            .map_err(unreachable("save your birthday"))?;
    }
    tracing::info!("{email} updated their page");
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /me/phone` — this is not my phone any more.
pub async fn forget_phone(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, Failure> {
    let (config, email) = caller(&state, &headers)?;
    let store = state.config.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "this Niles instance has nowhere to save it".to_string(),
    ))?;
    store
        .apply(
            &crate::presence::with_phone(&config, &email, None),
            niles_config::ChangeSource::Api,
        )
        .await
        .map_err(|e| (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;
    tracing::info!("{email} unpaired their phone");
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::session::{self, Session};
    use crate::publish::DevicePublisher;
    use axum::body::Body;
    use axum::http::{Request, header};
    use http_body_util::BodyExt;
    use niles_recognition::{EnrolledSpeaker, VoiceRoster};
    use serde_json::{Value, json};
    use std::sync::{Arc, Mutex};
    use tower::ServiceExt;

    const SECRET_VAR: &str = "NILES_TEST_ME_SESSION_SECRET";
    const SECRET: &str = "the-me-page-secret";

    #[derive(Clone)]
    struct NoopPublisher;

    #[async_trait::async_trait]
    impl DevicePublisher for NoopPublisher {
        async fn publish(&self, _topic: String, _payload: Vec<u8>) -> Result<(), String> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct Roster(Mutex<Vec<EnrolledSpeaker>>);

    impl Roster {
        fn with(names: &[&str]) -> Self {
            Self(Mutex::new(
                names
                    .iter()
                    .map(|n| EnrolledSpeaker {
                        speaker: n.to_string(),
                        display_name: n.to_string(),
                        spoken_as: None,
                        address_as: None,
                        notes: Some(format!("- private to {n}")),
                        birthday: None,
                        created_at: chrono::Utc::now(),
                        last_seen_at: None,
                        clip_count: 3,
                        embeddings: Vec::new(),
                    })
                    .collect(),
            ))
        }

        fn edit(&self, speaker: &str, f: impl FnOnce(&mut EnrolledSpeaker)) {
            let mut all = self.0.lock().unwrap();
            if let Some(v) = all.iter_mut().find(|v| v.speaker == speaker) {
                f(v);
            }
        }
    }

    #[async_trait::async_trait]
    impl VoiceRoster for Roster {
        async fn voices(&self) -> niles_recognition::Result<Vec<EnrolledSpeaker>> {
            Ok(self.0.lock().unwrap().clone())
        }
        async fn forget(&self, _speaker: &str) -> niles_recognition::Result<()> {
            Ok(())
        }
        async fn rename(&self, _speaker: &str, _name: &str) -> niles_recognition::Result<()> {
            Ok(())
        }
        async fn set_spoken_as(&self, s: &str, v: Option<&str>) -> niles_recognition::Result<()> {
            self.edit(s, |r| r.spoken_as = v.map(str::to_string));
            Ok(())
        }
        async fn set_address_as(&self, s: &str, v: Option<&str>) -> niles_recognition::Result<()> {
            self.edit(s, |r| r.address_as = v.map(str::to_string));
            Ok(())
        }
        async fn set_notes(&self, s: &str, v: Option<&str>) -> niles_recognition::Result<()> {
            self.edit(s, |r| r.notes = v.map(str::to_string));
            Ok(())
        }
        async fn set_birthday(&self, s: &str, v: Option<&str>) -> niles_recognition::Result<()> {
            self.edit(s, |r| r.birthday = v.map(str::to_string));
            Ok(())
        }
    }

    fn app(roster: Arc<Roster>) -> axum::Router {
        // SAFETY: test-only variable names nothing else reads.
        unsafe {
            std::env::set_var(SECRET_VAR, SECRET);
            std::env::set_var("NILES_TEST_ME_CLIENT_ID", "id");
            std::env::set_var("NILES_TEST_ME_CLIENT_SECRET", "secret");
        }
        let toml = format!(
            r#"{}
[auth]
github_client_id_env = "NILES_TEST_ME_CLIENT_ID"
github_client_secret_env = "NILES_TEST_ME_CLIENT_SECRET"
session_secret_env = "{SECRET_VAR}"
allowed = [
  {{ email = "mark@example.com", speaker = "mark", device_mac = "aa:bb:cc:dd:ee:ff" }},
  {{ email = "majse@example.com", speaker = "majse" }},
  {{ email = "guest@example.com" }},
]
"#,
            crate::config_tests::base_toml()
        );
        let store = Arc::new(niles_config::ConfigStore::from_str_in_memory(&toml).unwrap());
        let state = AppState::new(
            Arc::new(niles_core::DeviceRegistry::new()),
            Arc::new(NoopPublisher) as Arc<dyn DevicePublisher>,
            Arc::new(niles_mqtt::CommandRouter::z2m_only("zigbee2mqtt")),
            niles_core::EventBus::default(),
        )
        .with_config_store(Some(store))
        .with_voices(Some(roster as Arc<dyn VoiceRoster>));
        crate::server::router(state)
    }

    async fn send(app: &axum::Router, req: Request<Body>) -> (StatusCode, Value) {
        let res = app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    fn by(email: &str, method: &str, uri: &str, body: Option<Value>) -> Request<Body> {
        let token = session::sign(SECRET, &Session::new(email, None));
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::COOKIE, format!("{}={token}", session::COOKIE));
        match body {
            Some(b) => req
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(b.to_string()))
                .unwrap(),
            None => req.body(Body::empty()).unwrap(),
        }
    }

    #[tokio::test]
    async fn the_page_is_the_callers_own() {
        let app = app(Arc::new(Roster::with(&["mark", "majse"])));
        let (status, me) = send(&app, by("mark@example.com", "GET", "/me", None)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(me["speaker"], "mark");
        assert_eq!(me["notes"], "- private to mark");
        assert_eq!(me["phone"], "aa:bb:cc:dd:ee:ff");

        let (_, majse) = send(&app, by("majse@example.com", "GET", "/me", None)).await;
        assert_eq!(majse["notes"], "- private to majse");
    }

    #[tokio::test]
    async fn the_voice_list_does_not_carry_anybodys_notes() {
        let app = app(Arc::new(Roster::with(&["mark", "majse"])));
        let (status, voices) = send(&app, by("majse@example.com", "GET", "/voices", None)).await;
        assert_eq!(status, StatusCode::OK);
        assert!(!voices.to_string().contains("private"), "{voices}");
    }

    #[tokio::test]
    async fn saving_writes_to_the_callers_voice_only() {
        let roster = Arc::new(Roster::with(&["mark", "majse"]));
        let app = app(roster.clone());
        let body = json!({ "notes": "- Prefers tea", "birthday": "10-03", "address_as": "Sir" });
        let (status, _) = send(&app, by("mark@example.com", "PUT", "/me", Some(body))).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let all = roster.voices().await.unwrap();
        let mark = all.iter().find(|v| v.speaker == "mark").unwrap();
        let majse = all.iter().find(|v| v.speaker == "majse").unwrap();
        assert_eq!(mark.notes.as_deref(), Some("- Prefers tea"));
        assert_eq!(mark.birthday.as_deref(), Some("10-03"));
        assert_eq!(mark.address_as.as_deref(), Some("Sir"));
        assert_eq!(majse.notes.as_deref(), Some("- private to majse"));
    }

    #[tokio::test]
    async fn a_bad_birthday_saves_nothing() {
        let roster = Arc::new(Roster::with(&["mark"]));
        let app = app(roster.clone());
        let body = json!({ "notes": "- changed", "birthday": "02-30" });
        let (status, _) = send(&app, by("mark@example.com", "PUT", "/me", Some(body))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let notes = roster.voices().await.unwrap()[0].notes.clone();
        assert_eq!(notes.as_deref(), Some("- private to mark"));
    }

    #[tokio::test]
    async fn somebody_with_no_voice_has_no_profile_to_save() {
        let app = app(Arc::new(Roster::with(&["mark"])));
        let (status, me) = send(&app, by("guest@example.com", "GET", "/me", None)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(me["speaker"], Value::Null);
        let body = json!({ "notes": "- hello" });
        let (status, _) = send(&app, by("guest@example.com", "PUT", "/me", Some(body))).await;
        assert_eq!(status, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn unpairing_forgets_only_the_callers_phone() {
        let app = app(Arc::new(Roster::with(&["mark"])));
        let (status, _) = send(&app, by("mark@example.com", "DELETE", "/me/phone", None)).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (_, me) = send(&app, by("mark@example.com", "GET", "/me", None)).await;
        assert_eq!(me["phone"], Value::Null);
        assert_eq!(me["speaker"], "mark", "the voice link survives");
    }

    #[test]
    fn a_birthday_is_month_and_day() {
        assert_eq!(valid_birthday("10-03").unwrap(), "10-03");
        assert_eq!(valid_birthday("2-29").unwrap(), "02-29");
        assert!(valid_birthday("02-30").is_err());
        assert!(valid_birthday("3 October").is_err());
        assert!(valid_birthday("13-01").is_err());
    }
}
