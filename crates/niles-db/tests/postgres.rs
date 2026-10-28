//! Round-trip tests against a real Postgres.
//!
//! Skipped unless `NILES_TEST_DATABASE_URL` names a database the test may
//! create tables in. There is no Docker on the development machine and the
//! cluster's Postgres isn't reachable from it, so this is opt-in rather
//! than part of the default run — the honest alternative to claiming
//! coverage that isn't there.
//!
//! Run with:
//!
//! ```text
//! NILES_TEST_DATABASE_URL=postgres://... cargo test -p niles-db -- --ignored
//! ```

#![allow(clippy::unwrap_used, reason = "test helpers: a failure fails the test")]

use niles_config::{ChangeSource, OverrideBackend, Revision, StoredState};
use niles_db::PostgresBackend;

fn backend() -> Option<PostgresBackend> {
    let url = std::env::var("NILES_TEST_DATABASE_URL").ok()?;
    Some(PostgresBackend::connect_lazy(&url, 2).expect("valid test DSN"))
}

fn state(brightness: i64) -> StoredState {
    StoredState {
        overrides: toml::from_str(&format!("[lighting]\ndaytime_brightness = {brightness}"))
            .unwrap(),
        revisions: vec![Revision {
            id: 1,
            at: chrono::Utc::now(),
            source: ChangeSource::Voice,
            summary: format!("lighting.daytime_brightness 100 → {brightness}"),
            changed_paths: vec!["lighting.daytime_brightness".into()],
            overrides_before: toml::Table::new(),
        }],
    }
}

#[tokio::test]
#[ignore = "needs NILES_TEST_DATABASE_URL"]
async fn creates_its_own_schema_and_round_trips() {
    let Some(backend) = backend() else {
        return;
    };
    // The first call creates the tables — the claim that matters, since
    // nothing else in the deployment will create them.
    backend.save(&state(85)).await.expect("save");

    let loaded = backend.load().await.expect("load").expect("some state");
    assert_eq!(
        loaded.overrides["lighting"]["daytime_brightness"].as_integer(),
        Some(85)
    );
    assert_eq!(loaded.revisions.len(), 1);
    assert_eq!(loaded.revisions[0].source, ChangeSource::Voice);
}

#[tokio::test]
#[ignore = "needs NILES_TEST_DATABASE_URL"]
async fn saving_twice_replaces_rather_than_accumulates() {
    // The property that makes a retry after a failover safe: saving twice
    // must leave exactly one document and one journal, not two.
    let Some(backend) = backend() else {
        return;
    };
    backend.save(&state(85)).await.expect("first save");
    backend.save(&state(70)).await.expect("second save");

    let loaded = backend.load().await.expect("load").expect("some state");
    assert_eq!(
        loaded.overrides["lighting"]["daytime_brightness"].as_integer(),
        Some(70)
    );
    assert_eq!(loaded.revisions.len(), 1, "journal replaced, not appended");
}

#[tokio::test]
#[ignore = "needs NILES_TEST_DATABASE_URL"]
async fn an_empty_document_round_trips_as_empty() {
    let Some(backend) = backend() else {
        return;
    };
    backend
        .save(&StoredState::default())
        .await
        .expect("save empty");
    let loaded = backend.load().await.expect("load").expect("a row exists");
    assert!(loaded.overrides.is_empty());
    assert!(loaded.revisions.is_empty());
}

// ---- enrolled voices -------------------------------------------------

fn enrollments() -> Option<niles_db::PostgresEnrollments> {
    let backend = backend()?;
    Some(niles_db::PostgresEnrollments::new(
        backend.pool(),
        "the test database".into(),
    ))
}

/// A normalized embedding pointing along one axis, so two different
/// `axis` values are orthogonal and cannot be mistaken for each other.
fn embedding(axis: usize) -> Vec<f32> {
    let mut e = vec![0.0_f32; 192];
    e[axis] = 1.0;
    e
}

#[tokio::test]
#[ignore = "needs NILES_TEST_DATABASE_URL"]
async fn an_enrolled_voice_round_trips() {
    use niles_recognition::EnrollmentBackend;
    let Some(store) = enrollments() else { return };
    let _ = store.delete("test_speaker").await;

    store
        .enroll("test_speaker", &embedding(0))
        .await
        .expect("enroll");
    let record = store.load("test_speaker").await.expect("load");
    assert_eq!(record.display_name, "Test_speaker");
    assert_eq!(record.clip_count, 1);
    assert_eq!(record.embeddings[0].embedding.len(), 192);
    assert!(record.last_seen_at.is_none());

    store.delete("test_speaker").await.expect("delete");
}

#[tokio::test]
#[ignore = "needs NILES_TEST_DATABASE_URL"]
async fn a_second_clip_is_added_rather_than_replacing_the_first() {
    use niles_recognition::EnrollmentBackend;
    let Some(store) = enrollments() else { return };
    let _ = store.delete("test_speaker").await;

    store.enroll("test_speaker", &embedding(0)).await.unwrap();
    store.enroll("test_speaker", &embedding(1)).await.unwrap();

    let record = store.load("test_speaker").await.unwrap();
    assert_eq!(record.clip_count, 2, "enrolling again adds a clip");

    store.delete("test_speaker").await.unwrap();
}

#[tokio::test]
#[ignore = "needs NILES_TEST_DATABASE_URL"]
async fn being_heard_is_recorded() {
    use niles_recognition::EnrollmentBackend;
    let Some(store) = enrollments() else { return };
    let _ = store.delete("test_speaker").await;

    store.enroll("test_speaker", &embedding(0)).await.unwrap();
    store.bump_last_seen("test_speaker").await.unwrap();

    let record = store.load("test_speaker").await.unwrap();
    assert!(record.last_seen_at.is_some());

    store.delete("test_speaker").await.unwrap();
}

#[tokio::test]
#[ignore = "needs NILES_TEST_DATABASE_URL"]
async fn a_voice_nobody_enrolled_is_not_found() {
    use niles_recognition::EnrollmentBackend;
    let Some(store) = enrollments() else { return };
    let err = store.load("nobody_here").await.unwrap_err();
    assert!(err.to_string().contains("not found"), "{err}");
}

#[tokio::test]
#[ignore = "needs NILES_TEST_DATABASE_URL"]
async fn a_conversation_is_appended_to_and_kept_for_its_person_only() {
    let Some(backend) = backend() else {
        return;
    };
    let log = niles_db::PostgresConversationLog::new(backend.pool(), "test".into());
    let person = format!("person:test-{}", std::process::id());
    let id = log
        .append(&person, None, "hello", r#"{"said":"hello"}"#, true)
        .await
        .expect("start")
        .expect("an id");
    assert_eq!(log.active(&person, 300).await.expect("active"), Some(id));
    let into = log
        .append(&person, Some(id), "", r#"{"said":"again"}"#, false)
        .await
        .expect("append");
    assert_eq!(into, Some(id));

    let (row, turns) = log.get(&person, id).await.expect("get").expect("theirs");
    assert_eq!(row.turns, 2);
    assert!(row.spoken);
    assert!(turns.contains("again"), "{turns}");

    // Somebody else neither sees it nor adds to it.
    assert!(
        log.get("person:someone-else", id)
            .await
            .expect("get")
            .is_none()
    );
    assert_eq!(
        log.append("person:someone-else", Some(id), "", "{}", false)
            .await
            .expect("append"),
        None
    );

    assert!(log.delete(&person, id).await.expect("delete"));
    assert!(log.list(&person).await.expect("list").is_empty());
}

#[tokio::test]
#[ignore = "needs NILES_TEST_DATABASE_URL"]
async fn a_plugin_is_installed_its_storage_holds_its_quota_and_removal_takes_its_data() {
    let Some(backend) = backend() else {
        return;
    };
    let plugins = niles_db::PostgresPlugins::new(backend.pool(), "test".into());
    let id = format!("test{}", std::process::id());
    let release = |version, component| niles_db::PluginInstall {
        id: &id,
        version,
        manifest: "id = \"x\"",
        component,
        logo: Some("<svg/>"),
        repo: Some("https://github.com/jane/x"),
        addon: Some("x"),
        release_tag: Some(version),
        release_url: None,
    };
    plugins
        .install(&release("0.1.0", b"\0asm one"))
        .await
        .expect("install");
    plugins
        .install(&release("0.2.0", b"\0asm two"))
        .await
        .expect("update");
    let row = plugins
        .enabled()
        .await
        .expect("enabled")
        .into_iter()
        .find(|p| p.id == id)
        .expect("installed");
    assert_eq!(row.version, "0.2.0");
    assert_eq!(row.sha256, niles_db::hex_digest(b"\0asm two"));
    let meta = plugins.all().await.expect("all");
    let meta = meta.iter().find(|p| p.id == id).expect("listed");
    assert_eq!(meta.previous_version.as_deref(), Some("0.1.0"));
    assert_eq!(meta.addon.as_deref(), Some("x"));
    assert!(meta.has_logo);

    plugins.set_update(&id, Some("{}")).await.expect("update");
    let back = plugins
        .roll_back(&id)
        .await
        .expect("roll back")
        .expect("a previous");
    assert_eq!(back.version, "0.1.0");
    assert!(
        plugins.roll_back(&id).await.expect("again").is_none(),
        "once"
    );
    assert!(plugins.set_enabled(&id, false).await.expect("disable"));
    assert!(
        plugins
            .enabled()
            .await
            .expect("enabled")
            .iter()
            .all(|p| p.id != id)
    );

    assert!(plugins.set(&id, "day:2", "b", 100).await.expect("set"));
    assert!(plugins.set(&id, "day:1", "a", 100).await.expect("set"));
    assert_eq!(
        plugins.keys(&id, "day:").await.expect("keys"),
        vec!["day:1", "day:2"]
    );
    let big = "x".repeat(90);
    assert!(
        !plugins.set(&id, "big", &big, 100).await.expect("set"),
        "over the quota"
    );
    assert_eq!(
        plugins.get(&id, "day:1").await.expect("get").as_deref(),
        Some("a")
    );

    let lamp = niles_db::PluginDeviceRow {
        device: "office/lamp".into(),
        class: "light".into(),
        color_temp: true,
        rgb: false,
        controllable: true,
        room_light: false,
        reports_actions: true,
    };
    plugins.put_device(&id, &lamp).await.expect("device");
    assert_eq!(plugins.devices(&id).await.expect("devices"), vec![lamp]);

    plugins.remove(&id).await.expect("remove");
    assert!(plugins.package(&id).await.expect("package").is_none());
    assert!(plugins.keys(&id, "").await.expect("keys").is_empty());
    assert!(plugins.devices(&id).await.expect("devices").is_empty());
}

#[tokio::test]
#[ignore = "needs NILES_TEST_DATABASE_URL"]
async fn an_install_is_counted_once() {
    let Some(backend) = backend() else {
        return;
    };
    let plugins = niles_db::PostgresPlugins::new(backend.pool(), "test".into());
    let repo = format!("test/counted-{}", chrono::Utc::now().timestamp_micros());
    assert!(
        plugins.count_install(&repo).await.expect("first claim"),
        "the first install from a repo is counted"
    );
    assert!(
        !plugins.count_install(&repo).await.expect("second claim"),
        "ever after, even once the plugin is removed, it is not"
    );
}

#[tokio::test]
#[ignore = "needs NILES_TEST_DATABASE_URL"]
async fn training_saved_twice_is_saved_once() {
    use niles_training::TrainingStore;
    let Some(backend) = backend() else {
        return;
    };
    let store = niles_db::PostgresTraining::new(
        backend.pool(),
        backend.describe_target(),
        &niles_db::Sealer::generate_key(),
    )
    .expect("store");
    let person = format!("email:test-{}@example.com", std::process::id());
    let run = niles_training::strava::activity(&serde_json::json!({
        "id": 1, "name": "Morning Run", "sport_type": "Run",
        "start_date": "2026-10-14T05:00:00Z", "distance": 5000.0,
        "moving_time": 1500, "elapsed_time": 1560
    }))
    .unwrap();

    let first = store
        .save_activities(&person, std::slice::from_ref(&run))
        .await
        .expect("save");
    assert_eq!(first.added, 1);
    let again = store
        .save_activities(&person, &[run])
        .await
        .expect("save again");
    assert_eq!((again.added, again.updated), (0, 0));
    assert_eq!(
        store.latest(&person, "strava").await.expect("latest"),
        Some("2026-10-14T05:00:00Z".parse().unwrap())
    );
}

#[tokio::test]
#[ignore = "needs NILES_TEST_DATABASE_URL"]
async fn a_strava_secret_and_tokens_are_kept_sealed() {
    use niles_training::{Athlete, StravaApp, StravaTokens, TrainingStore};
    let Some(backend) = backend() else {
        return;
    };
    let key = niles_db::Sealer::generate_key();
    let store = niles_db::PostgresTraining::new(backend.pool(), backend.describe_target(), &key)
        .expect("store");
    let person = format!("email:strava-{}@example.com", std::process::id());
    let app = StravaApp {
        client_id: "1001".into(),
        client_secret: "plain-client-secret".into(),
    };
    store.set_strava_app(&person, &app).await.expect("app");
    let tokens = StravaTokens {
        access_token: "plain-access-token".into(),
        refresh_token: "plain-refresh-token".into(),
        expires_at: chrono::Utc::now(),
    };
    let athlete = Athlete {
        id: 42,
        name: "Majse".into(),
    };
    store
        .set_strava_tokens(&person, &tokens, Some(&athlete))
        .await
        .expect("tokens");

    let raw: (Vec<u8>, Vec<u8>, Vec<u8>) = sqlx::query_as(
        "select client_secret_enc, access_token_enc, refresh_token_enc
         from training_strava where person = $1",
    )
    .bind(&person)
    .fetch_one(&backend.pool())
    .await
    .expect("raw row");
    for (sealed, plain) in [
        (&raw.0, "plain-client-secret"),
        (&raw.1, "plain-access-token"),
        (&raw.2, "plain-refresh-token"),
    ] {
        assert!(
            !sealed.windows(plain.len()).any(|w| w == plain.as_bytes()),
            "{plain} is in the clear"
        );
    }

    let link = store.strava(&person).await.expect("read").expect("a link");
    assert_eq!(link.app, app);
    assert_eq!(link.tokens.unwrap().refresh_token, "plain-refresh-token");
    assert_eq!(link.athlete, Some(athlete));
    assert!(
        store
            .strava_connected()
            .await
            .expect("connected")
            .contains(&person)
    );

    store.clear_strava_tokens(&person).await.expect("clear");
    assert!(
        store
            .strava(&person)
            .await
            .unwrap()
            .unwrap()
            .tokens
            .is_none()
    );
    assert!(store.remove_strava(&person).await.expect("remove"));
}

#[tokio::test]
#[ignore = "needs NILES_TEST_DATABASE_URL"]
async fn an_artifact_s_page_is_kept_once_and_follows_its_versions() {
    use niles_artifacts::{ArtifactStore, Author, NewArtifact, NewRevision, PageChange, Status};
    let Some(backend) = backend() else {
        return;
    };
    let store = niles_db::PostgresArtifacts::new(backend.pool(), "test".into());
    let person = format!("person:test-pages-{}", std::process::id());
    let version = |markdown: &str, page: PageChange| NewRevision {
        content: serde_json::json!({ "markdown": markdown }),
        reason: "because".into(),
        author: Author::Claude,
        by: None,
        summary: None,
        page,
    };
    let a = store
        .create(
            &person,
            NewArtifact {
                kind: "document".into(),
                title: "Plan".into(),
                summary: String::new(),
                notify: true,
                status: Status::Ready,
                status_text: None,
                job_id: None,
                brief: None,
                conversation_id: None,
                first: Some(version("# first", PageChange::New("<h1>one</h1>".into()))),
            },
        )
        .await
        .expect("create");
    let page = a.page.expect("made with a page");
    let marked = store
        .revise(&person, a.id, version("# marked", PageChange::Keep), None)
        .await
        .expect("revise")
        .expect("theirs");
    assert_eq!(marked.page, Some(page), "a data change keeps the page");
    assert_eq!(
        store
            .page(&person, a.id, page)
            .await
            .expect("page")
            .as_deref(),
        Some("<h1>one</h1>")
    );
    assert!(
        store
            .page("person:someone-else", a.id, page)
            .await
            .expect("page")
            .is_none()
    );

    let gone = store
        .set_page(
            &person,
            a.id,
            PageChange::Remove,
            "No page".into(),
            Author::Person,
            None,
        )
        .await
        .expect("set_page")
        .expect("theirs");
    assert_eq!(gone.page, None);
    let r = store
        .revision(&person, a.id, gone.current_revision)
        .await
        .expect("revision")
        .expect("kept");
    assert_eq!(r.content["markdown"], "# marked", "the content as it was");
    let back = niles_artifacts::restore(&store, &person, a.id, marked.current_revision, &person)
        .await
        .expect("restore")
        .expect("theirs");
    assert_eq!(back.page, Some(page));

    for i in 0..niles_artifacts::KEPT_REVISIONS {
        store
            .set_page(
                &person,
                a.id,
                PageChange::Remove,
                format!("{i}"),
                Author::Person,
                None,
            )
            .await
            .expect("set_page");
    }
    assert!(
        store
            .page(&person, a.id, page)
            .await
            .expect("page")
            .is_none(),
        "no version kept draws with it"
    );
    assert!(store.delete(&person, a.id).await.expect("delete"));
}

#[tokio::test]
#[ignore = "needs NILES_TEST_DATABASE_URL"]
#[expect(
    clippy::too_many_lines,
    reason = "one stale-edit scenario from create to delete; each step depends on the one before"
)]
async fn stale_artifact_edits_cannot_discard_another_writer() {
    use niles_artifacts::{ArtifactStore, Author, NewArtifact, NewRevision, PageChange, Status};
    let Some(backend) = backend() else {
        return;
    };
    let store = niles_db::PostgresArtifacts::new(backend.pool(), "test".into());
    let person = format!("person:test-stale-edits-{}", std::process::id());
    let version = |markdown: &str, page| NewRevision {
        content: serde_json::json!({ "markdown": markdown }),
        reason: "Edit".into(),
        author: Author::Claude,
        by: None,
        summary: None,
        page,
    };
    let a = store
        .create(
            &person,
            NewArtifact {
                kind: "document".into(),
                title: "Plan".into(),
                summary: String::new(),
                notify: true,
                status: Status::Ready,
                status_text: None,
                job_id: None,
                brief: None,
                conversation_id: None,
                first: Some(version(
                    "alpha beta",
                    PageChange::New("<p>alpha beta</p>".into()),
                )),
            },
        )
        .await
        .unwrap();
    let first = version("ALPHA beta", PageChange::Keep);
    let second = version("alpha BETA", PageChange::Keep);
    let kept = store
        .revise(&person, a.id, first, Some(a.current_revision))
        .await
        .unwrap()
        .unwrap();
    assert!(
        store
            .revise(&person, a.id, second.clone(), Some(a.current_revision))
            .await
            .is_err()
    );
    let now = store.get(&person, a.id).await.unwrap().unwrap();
    assert_eq!(now.current_revision, kept.current_revision);
    let current = store
        .revision(&person, a.id, now.current_revision)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.content["markdown"], "ALPHA beta");
    let replaced = store
        .revise(&person, a.id, second, None)
        .await
        .unwrap()
        .unwrap();
    let current = store
        .revision(&person, a.id, replaced.current_revision)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.content["markdown"], "alpha BETA");
    let from = a.page.unwrap();
    let edit = |html: &str| PageChange::Edit {
        from,
        html: html.into(),
    };
    let kept = store
        .set_page(
            &person,
            a.id,
            edit("<p>ALPHA beta</p>"),
            "Edit".into(),
            Author::Claude,
            None,
        )
        .await
        .unwrap()
        .unwrap();
    assert!(
        store
            .set_page(
                &person,
                a.id,
                edit("<p>alpha BETA</p>"),
                "Edit".into(),
                Author::Claude,
                None
            )
            .await
            .is_err()
    );
    let now = store.get(&person, a.id).await.unwrap().unwrap();
    assert_eq!(now.current_revision, kept.current_revision);
    assert_eq!(
        store
            .page(&person, a.id, now.page.unwrap())
            .await
            .unwrap()
            .unwrap(),
        "<p>ALPHA beta</p>"
    );
    let current = store
        .revision(&person, a.id, now.current_revision)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.content["markdown"], "alpha BETA");
    assert!(store.delete(&person, a.id).await.unwrap());
}

#[tokio::test]
#[ignore = "needs NILES_TEST_DATABASE_URL"]
#[expect(
    clippy::too_many_lines,
    reason = "one sharing scenario from create to delete; each step depends on the one before"
)]
async fn an_artifact_is_shared_with_its_own_pin_and_goes_with_it() {
    use niles_artifacts::{
        Access, ArtifactStore, Author, Changes, Filter, Level, NewArtifact, NewRevision,
        PageChange, Status,
    };
    let Some(backend) = backend() else {
        return;
    };
    let store = niles_db::PostgresArtifacts::new(backend.pool(), "test".into());
    let owner = format!("person:test-owner-{}", std::process::id());
    let her = format!("person:test-shared-{}", std::process::id());
    let a = store
        .create(
            &owner,
            NewArtifact {
                kind: "document".into(),
                title: "Thailand trip".into(),
                summary: String::new(),
                notify: true,
                status: Status::Ready,
                status_text: None,
                job_id: None,
                brief: None,
                conversation_id: None,
                first: Some(NewRevision {
                    content: serde_json::json!({ "markdown": "- [ ] passport" }),
                    reason: "First draft".into(),
                    author: Author::Claude,
                    by: Some(owner.clone()),
                    summary: None,
                    page: PageChange::Keep,
                }),
            },
        )
        .await
        .expect("create");
    assert_eq!(a.changed_by.as_deref(), Some(owner.as_str()));
    assert!(
        store
            .set_share(&owner, a.id, &her, Some(Level::View))
            .await
            .expect("share")
    );
    assert!(
        store
            .set_share(&owner, a.id, &owner, Some(Level::View))
            .await
            .is_err()
    );
    assert!(
        !store
            .set_share(&her, a.id, "person:nobody", Some(Level::View))
            .await
            .expect("share"),
        "only the owner shares"
    );
    let opened = niles_artifacts::open(&store, &her, a.id)
        .await
        .expect("open")
        .expect("shared");
    assert_eq!(opened.access, Access::View);
    assert_eq!(opened.artifact.shared_with, vec![her.clone()]);

    store
        .set_share(&owner, a.id, &her, Some(Level::Edit))
        .await
        .expect("share");
    store
        .change(
            &owner,
            a.id,
            Changes {
                pinned: Some(true),
                ..Changes::default()
            },
        )
        .await
        .expect("pin");
    assert!(store.pin_shared(&her, a.id, false).await.expect("pin"));
    let listed = store
        .shared_with(&her, Filter::default())
        .await
        .expect("shared with");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].1, Level::Edit);
    assert!(!listed[0].0.pinned, "her pin, not his");
    assert_eq!(store.shares(&owner, a.id).await.expect("shares").len(), 1);

    let kept = store
        .revise(
            &owner,
            a.id,
            NewRevision {
                content: serde_json::json!({ "markdown": "- [x] passport" }),
                reason: "Ticked passport".into(),
                author: Author::Person,
                by: Some(her.clone()),
                summary: None,
                page: PageChange::Keep,
            },
            None,
        )
        .await
        .expect("revise")
        .expect("his");
    assert_eq!(kept.changed_by.as_deref(), Some(her.as_str()));
    let meta = store.revisions(&owner, a.id).await.expect("revisions");
    assert_eq!(meta[0].by.as_deref(), Some(her.as_str()));

    assert!(store.delete(&owner, a.id).await.expect("delete"));
    assert!(store.shared(&her, a.id).await.expect("shared").is_none());
}

fn approval(approval: &str, person: &str) -> niles_db::ApprovalEntry {
    niles_db::ApprovalEntry {
        approval: approval.into(),
        person: person.into(),
        tool: "Linear".into(),
        icon: Some("linear".into()),
        title: "Let Linear create issue?".into(),
        summary: "Title: WLED presets".into(),
        status: "pending".into(),
        decision: None,
        outcome: None,
        asked_via: "app".into(),
        decided_via: None,
        asked_at: chrono::Utc::now(),
        decided_at: None,
        conversation_id: Some(7),
    }
}

#[tokio::test]
#[ignore = "needs NILES_TEST_DATABASE_URL"]
async fn an_approval_is_logged_when_asked_and_again_when_decided() {
    let Some(backend) = backend() else {
        return;
    };
    let log = niles_db::PostgresApprovalLog::new(backend.pool(), "test".into());
    let person = format!("person:approvals-{}", std::process::id());
    let tag = |n: u32| format!("{}-{n}", std::process::id());

    let asked = approval(&tag(1), &person);
    log.asked(&asked).await.expect("asked");
    assert!(
        log.page(&person, None, 10).await.expect("page").is_empty(),
        "waiting is not history yet"
    );
    let decided = niles_db::ApprovalEntry {
        status: "allowed".into(),
        decision: Some("once".into()),
        outcome: Some("Done".into()),
        decided_via: Some("app".into()),
        decided_at: Some(chrono::Utc::now()),
        ..asked.clone()
    };
    log.settled(&decided).await.expect("settled");
    // Asked again after the answer, the answer stands.
    log.asked(&asked).await.expect("asked twice");
    let page = log.page(&person, None, 10).await.expect("page");
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].entry.status, "allowed");
    assert_eq!(page[0].entry.decision.as_deref(), Some("once"));

    for n in 2..5 {
        let mut denied = approval(&tag(n), &person);
        denied.status = "denied".into();
        log.settled(&denied).await.expect("denied");
    }
    log.settled(&approval(&tag(9), "person:someone-else"))
        .await
        .expect("another's");
    let first = log.page(&person, None, 2).await.expect("first page");
    assert_eq!(first.len(), 2);
    assert!(first[0].id > first[1].id, "newest first");
    let rest = log
        .page(&person, Some(first[1].id), 10)
        .await
        .expect("rest");
    assert_eq!(rest.len(), 2);
    assert!(rest.iter().all(|r| r.entry.person == person));

    // A Niles starting up finds what it cannot still be holding.
    log.asked(&approval(&tag(5), &person)).await.expect("asked");
    let restarted = niles_db::PostgresApprovalLog::new(backend.pool(), "test".into());
    let after = restarted.page(&person, None, 10).await.expect("page");
    assert_eq!(after[0].entry.approval, tag(5));
    assert_eq!(after[0].entry.status, "lapsed");
}

fn notice(tag: &str) -> niles_webpush::Notice {
    niles_webpush::Notice {
        kind: niles_webpush::inbox::Kind::Workout,
        title: "Hellerup plan".into(),
        body: "Your 6.2 km run today was counted as the easy run.".into(),
        url: "/#/chat?artifact=1".into(),
        tag: tag.into(),
    }
}

#[tokio::test]
#[ignore = "needs NILES_TEST_DATABASE_URL"]
async fn a_persons_notifications_are_theirs_capped_and_replaced_by_tag() {
    use niles_webpush::Inbox;
    let Some(backend) = backend() else {
        return;
    };
    let inbox = niles_db::PostgresPersonNotifications::new(backend.pool(), "test".into());
    let person = format!("person:notified-{}", std::process::id());
    let other = format!("person:other-{}", std::process::id());

    let first = inbox.add(&person, &notice("a")).await.expect("add");
    let second = inbox.add(&person, &notice("b")).await.expect("add");
    inbox.add(&other, &notice("a")).await.expect("another's");
    assert_eq!(inbox.unread(&person).await.unwrap(), 2);

    assert_eq!(inbox.read(&person, Some(first.id)).await.unwrap(), 1);
    assert_eq!(inbox.unread(&person).await.unwrap(), 1);

    // The same tag again: one row, newest, unread.
    let again = inbox.add(&person, &notice("a")).await.expect("again");
    assert!(again.id > second.id);
    let page = inbox.page(&person, None, 50).await.unwrap();
    assert_eq!(page.len(), 2);
    assert_eq!(page[0].id, again.id);
    assert!(page[0].read_at.is_none());

    let older = inbox.page(&person, Some(again.id), 50).await.unwrap();
    assert_eq!(older.len(), 1);
    assert_eq!(older[0].id, second.id);

    for n in 0..niles_webpush::inbox::KEPT_PER_PERSON {
        inbox.add(&person, &notice(&format!("w{n}"))).await.unwrap();
    }
    let all = inbox.page(&person, None, 1_000).await.unwrap();
    assert_eq!(all.len() as i64, niles_webpush::inbox::KEPT_PER_PERSON);
    assert_eq!(inbox.page(&other, None, 50).await.unwrap().len(), 1);
    assert_eq!(inbox.read(&other, None).await.unwrap(), 1);
}
