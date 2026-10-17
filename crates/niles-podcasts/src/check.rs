//! The add-on's test connection: one part per dependency, each named
//! and capped, so a failure says what broke without ever echoing an
//! address or a key.

use crate::{Bucket, Kokoro, Speech, Storage, mp3};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::time::Duration;

const KOKORO_WITHIN: Duration = Duration::from_secs(45);
const STORAGE_WITHIN: Duration = Duration::from_secs(20);
/// What the storage part says to prove it end to end: upload, read
/// back through a presigned address, delete.
const TEST_BODY: &[u8] = b"niles";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Part {
    pub ok: bool,
    pub error: Option<String>,
}

impl Part {
    fn ok() -> Part {
        Part {
            ok: true,
            error: None,
        }
    }

    fn failed(e: impl std::fmt::Display) -> Part {
        Part {
            ok: false,
            error: Some(e.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Check {
    pub kokoro: Part,
    pub storage: Part,
}

/// Both parts at once; each has its own ceiling, and a part that blows
/// its ceiling is reported as failed, not hung.
pub async fn check(
    kokoro: &Kokoro,
    voices: &[String; 2],
    bucket: &Bucket,
    now: DateTime<Utc>,
) -> Check {
    let (kokoro, storage) = tokio::join!(
        tokio::time::timeout(KOKORO_WITHIN, check_kokoro(kokoro, voices),),
        tokio::time::timeout(STORAGE_WITHIN, check_storage(bucket, now),),
    );
    Check {
        kokoro: match kokoro {
            Ok(part) => part,
            Err(_) => Part::failed(format!(
                "did not finish within {} s",
                KOKORO_WITHIN.as_secs()
            )),
        },
        storage: match storage {
            Ok(part) => part,
            Err(_) => Part::failed(format!(
                "did not finish within {} s",
                STORAGE_WITHIN.as_secs()
            )),
        },
    }
}

async fn check_kokoro(kokoro: &Kokoro, voices: &[String; 2]) -> Part {
    for (which, voice) in ["first", "second"].iter().zip(voices.iter()) {
        match kokoro.say(voice, "Hello from Niles.").await {
            Ok(audio) => {
                if let Err(e) = mp3::frames(&audio) {
                    return Part::failed(format!("The {which} host's voice ({voice}): {e}"));
                }
            }
            Err(e) => {
                return Part::failed(format!("The {which} host's voice ({voice}): {e}"));
            }
        }
    }
    Part::ok()
}

async fn check_storage(bucket: &Bucket, now: DateTime<Utc>) -> Part {
    let key = format!("podcasts/niles-test-{}.txt", now.timestamp_millis());
    if let Err(e) = bucket.put(&key, TEST_BODY.to_vec(), "text/plain").await {
        return Part::failed(format!("Uploading a test file: {e}"));
    }
    // Whatever happens next, the delete is always tried, so a test file
    // never lives in the bucket past its test.
    let read_back = match bucket.read(&key, now).await {
        Ok(body) if body == TEST_BODY => None,
        Ok(_) => Some("what was read back is not what was written".to_string()),
        Err(e) => Some(e.to_string()),
    };
    let delete = bucket.delete(&key).await;
    if let Some(error) = read_back {
        return Part::failed(format!(
            "Reading it back through a time-limited address: {error}"
        ));
    }
    match delete {
        Ok(()) => Part::ok(),
        Err(e) => Part::failed(format!(
            "Deleting the test file: {e}. It stays until the bucket's lifecycle rule clears it."
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path as wire_path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// One whole silent frame: what `mp3::frames` accepts as an answer.
    fn mp3_answer() -> Vec<u8> {
        [0xFF, 0xF3, 0x84, 0xC0]
            .into_iter()
            .chain(std::iter::repeat_n(0u8, 188))
            .collect()
    }

    fn at_millis(n: i64) -> DateTime<Utc> {
        DateTime::from_timestamp_millis(n).expect("a time")
    }

    async fn bucket(server: &MockServer) -> Bucket {
        Bucket::new(crate::BucketSettings {
            endpoint: server.uri(),
            bucket: "b".into(),
            region: "auto".into(),
            access_key_id: "AKIDEXAMPLE".into(),
            secret_access_key: "secret".into(),
        })
        .expect("a bucket")
    }

    /// The three calls the storage part makes, against a key derived
    /// from a fixed `now`.
    async fn mount_storage(server: &MockServer, n: i64) {
        let key = format!("/b/podcasts/niles-test-{n}.txt");
        Mock::given(method("PUT"))
            .and(wire_path(key.as_str()))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(server)
            .await;
        Mock::given(method("GET"))
            .and(wire_path(key.as_str()))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(TEST_BODY))
            .expect(1)
            .mount(server)
            .await;
        Mock::given(method("DELETE"))
            .and(wire_path(key.as_str()))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(server)
            .await;
    }

    async fn kokoro_saying(server: &MockServer, status: u16, body: &[u8]) -> Kokoro {
        Mock::given(method("POST"))
            .and(wire_path("/v1/audio/speech"))
            .respond_with(ResponseTemplate::new(status).set_body_bytes(body))
            .mount(server)
            .await;
        Kokoro::new(&server.uri()).expect("a kokoro")
    }

    #[tokio::test]
    async fn both_parts_pass_against_a_quiet_house() {
        let kokoro_server = MockServer::start().await;
        let kokoro = kokoro_saying(&kokoro_server, 200, &mp3_answer()).await;
        let storage_server = MockServer::start().await;
        mount_storage(&storage_server, 1).await;
        let Check { kokoro, storage } = check(
            &kokoro,
            &["af_heart".to_string(), "am_michael".to_string()],
            &bucket(&storage_server).await,
            at_millis(1),
        )
        .await;
        assert!(kokoro.ok, "kokoro failed: {:?}", kokoro.error);
        assert!(storage.ok, "storage failed: {:?}", storage.error);
    }

    #[tokio::test]
    async fn kokoro_failing_names_the_first_hosts_voice() {
        let kokoro_server = MockServer::start().await;
        let kokoro = kokoro_saying(&kokoro_server, 500, &mp3_answer()).await;
        let storage_server = MockServer::start().await;
        mount_storage(&storage_server, 2).await;
        let Check { kokoro, storage } = check(
            &kokoro,
            &["af_heart".to_string(), "am_michael".to_string()],
            &bucket(&storage_server).await,
            at_millis(2),
        )
        .await;
        assert!(!kokoro.ok);
        let error = kokoro.error.expect("an error");
        assert!(
            error.contains("The first host's voice (af_heart)"),
            "unexpected: {error}"
        );
        assert!(storage.ok, "storage should pass on its own");
    }

    #[tokio::test]
    async fn kokoro_answering_html_is_not_mp3() {
        let kokoro_server = MockServer::start().await;
        let kokoro = kokoro_saying(&kokoro_server, 200, b"<html><body>oops</body></html>").await;
        let storage_server = MockServer::start().await;
        mount_storage(&storage_server, 3).await;
        let Check { kokoro, .. } = check(
            &kokoro,
            &["af_heart".to_string(), "am_michael".to_string()],
            &bucket(&storage_server).await,
            at_millis(3),
        )
        .await;
        let error = kokoro.error.expect("an error");
        assert!(error.contains("not MP3 audio"), "unexpected: {error}");
    }

    #[tokio::test]
    async fn a_refused_upload_is_reported_on_the_storage_part() {
        let kokoro_server = MockServer::start().await;
        let kokoro = kokoro_saying(&kokoro_server, 200, &mp3_answer()).await;
        let storage_server = MockServer::start().await;
        Mock::given(method("PUT"))
            .respond_with(
                ResponseTemplate::new(403)
                    .set_body_string("<Error><Code>AccessDenied</Code></Error>"),
            )
            .expect(1)
            .mount(&storage_server)
            .await;
        let Check { kokoro, storage } = check(
            &kokoro,
            &["af_heart".to_string(), "am_michael".to_string()],
            &bucket(&storage_server).await,
            at_millis(4),
        )
        .await;
        assert!(kokoro.ok, "kokoro should pass on its own");
        assert!(!storage.ok);
        let error = storage.error.expect("an error");
        assert!(
            error.starts_with("Uploading a test file:") && error.contains("AccessDenied"),
            "unexpected: {error}"
        );
    }

    #[tokio::test]
    async fn a_refused_delete_is_named_and_says_what_stays() {
        let kokoro_server = MockServer::start().await;
        let kokoro = kokoro_saying(&kokoro_server, 200, &mp3_answer()).await;
        let storage_server = MockServer::start().await;
        let key = "/b/podcasts/niles-test-5.txt";
        Mock::given(method("PUT"))
            .and(wire_path(key))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&storage_server)
            .await;
        Mock::given(method("GET"))
            .and(wire_path(key))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(TEST_BODY))
            .expect(1)
            .mount(&storage_server)
            .await;
        Mock::given(method("DELETE"))
            .and(wire_path(key))
            .respond_with(ResponseTemplate::new(403))
            .expect(1)
            .mount(&storage_server)
            .await;
        let Check { storage, .. } = check(
            &kokoro,
            &["af_heart".to_string(), "am_michael".to_string()],
            &bucket(&storage_server).await,
            at_millis(5),
        )
        .await;
        let error = storage.error.expect("an error");
        assert!(
            error.starts_with("Deleting the test file:") && error.contains("lifecycle rule"),
            "unexpected: {error}"
        );
    }
}
