//! The bucket: one private R2 bucket, reached path-style and signed
//! with the household's own API token.

use crate::error::{Error, Result};
use crate::{HTTP, Storage, sigv4};
use chrono::{DateTime, Utc};
use sha2::Digest;
use std::time::Duration;

const PUT_WITHIN: Duration = Duration::from_secs(120);
const DELETE_WITHIN: Duration = Duration::from_secs(20);
/// A presigned address lives at most a week, the S3 limit.
const MOST_VALID: u64 = 604_800;
/// A test object read back is tiny; a bigger answer is not the object.
const MOST_READ: usize = 1 << 20;
const MOST_ERROR: usize = 4096;
const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

pub struct BucketSettings {
    pub endpoint: String,
    pub bucket: String,
    pub region: String,
    pub access_key_id: String,
    pub secret_access_key: String,
}

#[derive(Debug)]
pub struct Bucket {
    scheme: String,
    host: String,
    bucket: String,
    region: String,
    access_key_id: String,
    secret: String,
}

impl Bucket {
    pub fn new(settings: BucketSettings) -> Result<Bucket> {
        let parsed = url::Url::parse(&settings.endpoint).map_err(|_| Error::Storage {
            action: "reach",
            reason: "the R2 endpoint is not an address Niles can use".into(),
        })?;
        if !matches!(parsed.scheme(), "http" | "https")
            || !matches!(parsed.path(), "" | "/")
            || parsed.host_str().is_none()
        {
            return Err(Error::Storage {
                action: "reach",
                reason: "the R2 endpoint is not an address Niles can use".into(),
            });
        }
        let host = match parsed.port() {
            Some(port) => format!("{}:{port}", parsed.host_str().expect("checked above")),
            None => parsed.host_str().expect("checked above").to_string(),
        };
        Ok(Bucket {
            scheme: parsed.scheme().to_string(),
            host,
            bucket: settings.bucket.trim().to_string(),
            region: settings.region.trim().to_string(),
            access_key_id: settings.access_key_id.trim().to_string(),
            secret: settings.secret_access_key.trim().to_string(),
        })
    }

    /// The bucket the config describes, or what the add-on still needs.
    pub fn from_config(cfg: &niles_config::PodcastsConfig) -> Result<Bucket> {
        let mut missing = Vec::new();
        if cfg.endpoint_address().is_none() {
            missing.push("R2 endpoint");
        }
        if cfg.bucket_name().is_none() {
            missing.push("bucket name");
        }
        if cfg.access_key_id().is_err() {
            missing.push("access key ID");
        }
        if cfg.secret_access_key().is_err() {
            missing.push("secret access key");
        }
        if !missing.is_empty() {
            return Err(Error::NotSetUp { missing });
        }
        Bucket::new(BucketSettings {
            endpoint: cfg.endpoint_address().expect("checked above"),
            bucket: cfg.bucket_name().expect("checked above").to_string(),
            region: cfg.signing_region().to_string(),
            access_key_id: cfg.access_key_id().expect("checked above"),
            secret_access_key: cfg.secret_access_key().expect("checked above"),
        })
    }

    fn creds(&self) -> sigv4::Credentials<'_> {
        sigv4::Credentials {
            access_key_id: &self.access_key_id,
            secret: &self.secret,
            region: &self.region,
        }
    }

    /// The canonical URI for `key`, the bucket in the path, path-style.
    fn uri(&self, key: &str) -> String {
        format!(
            "/{}/{}",
            sigv4::encode(&self.bucket, false),
            sigv4::encode(key, true)
        )
    }

    /// A time-limited GET address for `key`; never logged, never the
    /// secret — the access key id in it is not one.
    pub fn presign_get(&self, key: &str, valid_for: Duration, now: DateTime<Utc>) -> String {
        self.presign(key, valid_for, &[], now)
    }

    /// The same, but the bucket answers it as a download named
    /// `file_name`: a browser saves it rather than playing it, which the
    /// `download` attribute can't ask of another site's address.
    pub fn presign_download(
        &self,
        key: &str,
        file_name: &str,
        valid_for: Duration,
        now: DateTime<Utc>,
    ) -> String {
        self.presign(
            key,
            valid_for,
            &[("response-content-disposition", &attachment(file_name))],
            now,
        )
    }

    fn presign(
        &self,
        key: &str,
        valid_for: Duration,
        extra: &[(&str, &str)],
        now: DateTime<Utc>,
    ) -> String {
        sigv4::presign(
            &self.creds(),
            "GET",
            &self.scheme,
            &self.host,
            &self.uri(key),
            valid_for.as_secs().min(MOST_VALID),
            extra,
            now,
        )
    }

    /// GET a presigned address of `key`, for the connection test's
    /// read-back. At most [`MOST_READ`] bytes.
    pub(crate) async fn read(&self, key: &str, now: DateTime<Utc>) -> Result<Vec<u8>> {
        let url = self.presign_get(key, Duration::from_secs(60), now);
        let response = match HTTP.get(&url).timeout(Duration::from_secs(20)).send().await {
            Ok(response) => response,
            Err(e) => return Err(self.not_answered(&e, "read", 20)),
        };
        if !response.status().is_success() {
            return Err(self.refused(response, "read").await);
        }
        let body = response_prefix(response, MOST_READ + 1)
            .await
            .map_err(|_| Error::Storage {
                action: "read",
                reason: "R2 answered with unreadable data".into(),
            })?;
        if body.len() > MOST_READ {
            return Err(Error::Storage {
                action: "read",
                reason: "what came back is bigger than the test file can be".into(),
            });
        }
        Ok(body)
    }

    fn not_answered(&self, e: &reqwest::Error, action: &'static str, within: u64) -> Error {
        if e.is_timeout() {
            Error::Storage {
                action,
                reason: format!("R2 did not answer within {within} s"),
            }
        } else if e.is_connect() {
            Error::Storage {
                action,
                reason: "nothing answered at the R2 endpoint; check it, and that Niles can reach \
                         it"
                .into(),
            }
        } else {
            Error::Storage {
                action,
                reason: "the request to R2 failed before it answered".into(),
            }
        }
    }

    /// A non-success answer read into something a person can act on:
    /// the S3 error `<Code>` (sanitised) and what it means, never the
    /// body, never the address.
    async fn refused(&self, response: reqwest::Response, action: &'static str) -> Error {
        let status = response.status().as_u16();
        let body = response_prefix(response, MOST_ERROR)
            .await
            .unwrap_or_default();
        let text = String::from_utf8_lossy(&body);
        let code = text
            .split("<Code>")
            .nth(1)
            .and_then(|rest| rest.split("</Code>").next())
            .map(|raw| {
                raw.chars()
                    .filter(|c| c.is_ascii_alphanumeric())
                    .take(64)
                    .collect::<String>()
            })
            .filter(|code| !code.is_empty());
        let reason = match &code {
            Some(code) => {
                let advice = match code.as_str() {
                    "AccessDenied" => "the access key has no access to this bucket",
                    "InvalidAccessKeyId" => "R2 does not know that access key ID",
                    "SignatureDoesNotMatch" => {
                        "the secret access key does not belong to that access key ID"
                    }
                    "NoSuchBucket" => "there is no bucket by that name at this endpoint",
                    "AuthorizationHeaderMalformed" | "AuthorizationQueryParametersError" => {
                        "check the region (auto for R2)"
                    }
                    "RequestTimeTooSkewed" => "the clock where Niles runs is off",
                    other => other,
                };
                format!("R2 refused it (HTTP {status}, {code}): {advice}")
            }
            None => format!("R2 answered with HTTP {status}"),
        };
        Error::Storage { action, reason }
    }
}

async fn response_prefix(
    mut response: reqwest::Response,
    limit: usize,
) -> std::result::Result<Vec<u8>, reqwest::Error> {
    let mut body = Vec::new();
    while body.len() < limit {
        let Some(chunk) = response.chunk().await? else {
            break;
        };
        body.extend_from_slice(&chunk[..chunk.len().min(limit - body.len())]);
    }
    Ok(body)
}

#[async_trait::async_trait]
impl Storage for Bucket {
    async fn put(&self, key: &str, body: Vec<u8>, content_type: &str) -> Result<()> {
        let payload_hash = hex::encode(sha2::Sha256::digest(&body));
        let now = Utc::now();
        let authorization = sigv4::authorization(
            &self.creds(),
            "PUT",
            &self.host,
            &self.uri(key),
            "",
            &[
                ("host", &self.host),
                ("x-amz-content-sha256", &payload_hash),
                ("x-amz-date", &now.format("%Y%m%dT%H%M%SZ").to_string()),
            ],
            &payload_hash,
            now,
        );
        let url = format!("{}://{}{}", self.scheme, self.host, self.uri(key));
        let response = match HTTP
            .put(&url)
            .header("authorization", authorization)
            .header("x-amz-content-sha256", &payload_hash)
            .header("x-amz-date", now.format("%Y%m%dT%H%M%SZ").to_string())
            .header("content-type", content_type)
            .body(body)
            .timeout(PUT_WITHIN)
            .send()
            .await
        {
            Ok(response) => response,
            Err(e) => {
                let within = PUT_WITHIN.as_secs();
                return Err(self.not_answered(&e, "upload", within));
            }
        };
        if response.status().as_u16() != 200 {
            return Err(self.refused(response, "upload").await);
        }
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<()> {
        let now = Utc::now();
        let authorization = sigv4::authorization(
            &self.creds(),
            "DELETE",
            &self.host,
            &self.uri(key),
            "",
            &[
                ("host", &self.host),
                ("x-amz-content-sha256", EMPTY_SHA256),
                ("x-amz-date", &now.format("%Y%m%dT%H%M%SZ").to_string()),
            ],
            EMPTY_SHA256,
            now,
        );
        let url = format!("{}://{}{}", self.scheme, self.host, self.uri(key));
        let response = match HTTP
            .delete(&url)
            .header("authorization", authorization)
            .header("x-amz-content-sha256", EMPTY_SHA256)
            .header("x-amz-date", now.format("%Y%m%dT%H%M%SZ").to_string())
            .timeout(DELETE_WITHIN)
            .send()
            .await
        {
            Ok(response) => response,
            Err(e) => {
                let within = DELETE_WITHIN.as_secs();
                return Err(self.not_answered(&e, "delete", within));
            }
        };
        let status = response.status().as_u16();
        if status == 204 || status == 200 {
            return Ok(());
        }
        // A key already gone is the state a delete wants.
        if status == 404 {
            let gone = response_prefix(response, MOST_ERROR)
                .await
                .map(|body| String::from_utf8_lossy(&body).contains("<Code>NoSuchKey</Code>"))
                .unwrap_or(false);
            if gone {
                return Ok(());
            }
            return Err(Error::Storage {
                action: "delete",
                reason: "R2 answered with HTTP 404".into(),
            });
        }
        Err(self.refused(response, "delete").await)
    }

    fn presign_get(&self, key: &str, valid_for: Duration, now: DateTime<Utc>) -> String {
        Bucket::presign_get(self, key, valid_for, now)
    }
}

/// A `Content-Disposition` that saves the file as `file_name`: a plain
/// ASCII `filename` for old browsers, and the exact name, UTF-8 and
/// percent-encoded, in `filename*` (RFC 6266) for the rest.
fn attachment(file_name: &str) -> String {
    let plain: String = file_name
        .chars()
        .map(|c| match c {
            '"' | '\\' => '_',
            c if c.is_ascii_graphic() || c == ' ' => c,
            _ => '_',
        })
        .collect();
    let exact: String = file_name
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect();
    format!("attachment; filename=\"{plain}\"; filename*=UTF-8''{exact}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_download_names_the_file_for_every_browser() {
        assert_eq!(
            attachment("Øresund \"bridge\".mp3"),
            "attachment; filename=\"_resund _bridge_.mp3\"; \
             filename*=UTF-8''%C3%98resund%20%22bridge%22.mp3"
        );
    }

    #[test]
    fn a_download_address_asks_the_bucket_to_save_it() {
        let b = bucket("https://account.r2.cloudflarestorage.com");
        let at = chrono::Utc::now();
        let url = b.presign_download("e/7.mp3", "Bridge.mp3", Duration::from_secs(600), at);
        assert!(
            url.contains(
                "response-content-disposition=attachment%3B%20filename%3D%22Bridge.mp3%22"
            ),
            "{url}"
        );
        assert!(
            !b.presign_get("e/7.mp3", Duration::from_secs(600), at)
                .contains("disposition")
        );
    }
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn bucket(endpoint: &str) -> Bucket {
        Bucket::new(BucketSettings {
            endpoint: endpoint.into(),
            bucket: "my-bucket".into(),
            region: "auto".into(),
            access_key_id: "AKIDEXAMPLE".into(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
        })
        .expect("a bucket")
    }

    #[test]
    fn a_bad_endpoint_is_not_an_address_niles_can_use() {
        for endpoint in [
            "not an address",
            "ftp://account.r2.cloudflarestorage.com",
            "https://account.r2.cloudflarestorage.com/bucket",
        ] {
            let error = Bucket::new(BucketSettings {
                endpoint: endpoint.into(),
                bucket: "b".into(),
                region: "auto".into(),
                access_key_id: "A".into(),
                secret_access_key: "S".into(),
            })
            .expect_err("refused");
            assert!(
                error
                    .to_string()
                    .contains("the R2 endpoint is not an address Niles can use"),
                "unexpected: {error}"
            );
        }
    }

    #[test]
    fn the_r2_oracle_for_auto_and_us_east_1() {
        // Cross-checked with an independent Python implementation while
        // planning; do not hand-edit the expected signatures.
        let endpoint = "https://0123456789abcdef0123456789abcdef.r2.cloudflarestorage.com";
        let now = DateTime::parse_from_rfc3339("2026-10-17T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let auto = Bucket::new(BucketSettings {
            endpoint: endpoint.into(),
            bucket: "niles-podcasts".into(),
            region: "auto".into(),
            access_key_id: "AKIDEXAMPLE".into(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
        })
        .expect("a bucket");
        assert_eq!(
            auto.presign_get("podcasts/42.mp3", Duration::from_secs(14_400), now),
            "https://0123456789abcdef0123456789abcdef.r2.cloudflarestorage.com/\
             niles-podcasts/podcasts/42.mp3?X-Amz-Algorithm=AWS4-HMAC-SHA256&\
             X-Amz-Credential=AKIDEXAMPLE%2F20261017%2Fauto%2Fs3%2Faws4_request&\
             X-Amz-Date=20261017T120000Z&X-Amz-Expires=14400&X-Amz-SignedHeaders=host&\
             X-Amz-Signature=56c9ab138ebcf2f7e5962cae4c4136054cd44f2fc59100a6d2892366941899e4"
        );
        let east = Bucket::new(BucketSettings {
            endpoint: endpoint.into(),
            bucket: "niles-podcasts".into(),
            region: "us-east-1".into(),
            access_key_id: "AKIDEXAMPLE".into(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
        })
        .expect("a bucket");
        let url = east.presign_get("podcasts/42.mp3", Duration::from_secs(14_400), now);
        assert!(
            url.contains("%2Fus-east-1%2Fs3%2Faws4_request")
                && url.contains(
                    "X-Amz-Signature=cd31774d0b37372c492cfd48bb8eaf3bacb9ea5f458277e691e5d5ce9e22f231"
                ),
            "unexpected: {url}"
        );
    }

    #[test]
    fn from_config_uses_auto_when_no_region_is_set() {
        // SAFETY: test-only variable names, unique to this test; no
        // other test reads them.
        unsafe {
            std::env::set_var("NILES_TEST_PODCASTS_AKID", "AKIDEXAMPLE");
            std::env::set_var("NILES_TEST_PODCASTS_SECRET", "secret");
        }
        let cfg: niles_config::PodcastsConfig = toml::from_str(
            "[podcasts]\nendpoint = 'https://account.r2.cloudflarestorage.com'\nbucket = 'episodes'\nregion = ''\naccess_key_id_env = 'NILES_TEST_PODCASTS_AKID'\nsecret_access_key_env = 'NILES_TEST_PODCASTS_SECRET'\n",
        )
        .map(|i: niles_config::IntegrationsConfig| i.podcasts.expect("the section"))
        .expect("the config parses");
        assert_eq!(cfg.signing_region(), "auto");
        let bucket = Bucket::from_config(&cfg).expect("a bucket");
        assert_eq!(bucket.region, "auto");
    }

    #[tokio::test]
    async fn a_put_is_signed_and_goes_to_the_bucket_path() {
        let server = MockServer::start().await;
        let body = b"an episode".to_vec();
        let hash = hex::encode(sha2::Sha256::digest(&body));
        Mock::given(method("PUT"))
            .and(path("/my-bucket/podcasts/42.mp3"))
            .and(header("x-amz-content-sha256", hash.as_str()))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        bucket(&server.uri())
            .put("podcasts/42.mp3", body, "audio/mpeg")
            .await
            .expect("uploaded");
        let requests = server.received_requests().await.expect("recorded");
        let authorization = requests[0]
            .headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .expect("an authorization header");
        assert!(
            authorization.starts_with("AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/"),
            "unexpected: {authorization}"
        );
        assert!(
            authorization.contains(
                "/auto/s3/aws4_request, SignedHeaders=host;x-amz-content-sha256;x-amz-date, Signature="
            ),
            "unexpected: {authorization}"
        );
    }

    #[tokio::test]
    async fn a_refusal_names_the_code_and_nothing_secret() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .respond_with(
                ResponseTemplate::new(403).set_body_string(
                    "<Error><Code>AccessDenied</Code><Message>no</Message></Error>",
                ),
            )
            .expect(1)
            .mount(&server)
            .await;
        let error = bucket(&server.uri())
            .put("podcasts/42.mp3", b"x".to_vec(), "audio/mpeg")
            .await
            .expect_err("refused");
        let message = error.to_string();
        assert!(message.contains("AccessDenied"), "unexpected: {message}");
        assert!(
            !message.contains("wJalrXUtn")
                && !message.contains("AKIDEXAMPLE")
                && !message.contains(&server.uri()),
            "the message leaks: {message}"
        );
    }

    #[tokio::test]
    async fn a_delete_is_fine_with_204_and_with_a_key_already_gone() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/my-bucket/podcasts/1.mp3"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/my-bucket/podcasts/2.mp3"))
            .respond_with(
                ResponseTemplate::new(404).set_body_string("<Error><Code>NoSuchKey</Code></Error>"),
            )
            .expect(1)
            .mount(&server)
            .await;
        let bucket = bucket(&server.uri());
        bucket.delete("podcasts/1.mp3").await.expect("deleted");
        bucket.delete("podcasts/2.mp3").await.expect("already gone");
    }

    #[tokio::test]
    async fn an_unreachable_endpoint_says_nothing_answered() {
        let bucket = bucket("http://127.0.0.1:9");
        let error = bucket
            .put("podcasts/42.mp3", b"x".to_vec(), "audio/mpeg")
            .await
            .expect_err("unreachable");
        assert!(
            error.to_string().contains("nothing answered"),
            "unexpected: {error}"
        );
    }

    #[test]
    fn the_uri_is_path_style_and_encoded() {
        let bucket = bucket("https://account.r2.cloudflarestorage.com");
        assert_eq!(bucket.uri("podcasts/42.mp3"), "/my-bucket/podcasts/42.mp3");
        assert_eq!(
            bucket.uri("2026/episode 7.mp3"),
            "/my-bucket/2026/episode%207.mp3"
        );
    }
}
