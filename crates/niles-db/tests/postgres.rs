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
