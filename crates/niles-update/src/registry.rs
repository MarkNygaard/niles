//! The newest published image of Niles, asked of GHCR without signing
//! in.
//!
//! The check is HEAD first: a quiet check is two requests (a token, a
//! HEAD) and pulls nothing. A new digest means the manifest, the
//! linux/amd64 image's config blob, and the commit, date and version
//! labels metadata-action bakes into it.
//!
//! The pull token is never logged or returned: it names nothing, but a
//! bearer for a public repo is still a credential.

use crate::build_info::Commit;
use crate::error::{Error, Result};
use crate::http;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::collections::HashMap;
use url::Url;

/// Niles's public image, the only one this checks.
pub const IMAGE: &str = "ghcr.io/marknygaard/niles";
/// GHCR itself, or a stand-in in tests.
pub(crate) const REGISTRY: &str = "https://ghcr.io";
const REPOSITORY: &str = "marknygaard/niles";
const TAG: &str = "latest";
/// How every error names the registry in its reason.
const WHAT: &str = "GitHub's container registry";
/// A pull token, at most: it is a short-lived JWT, not a document.
const MAX_TOKEN: usize = 64 * 1024;
/// A manifest or index, at most.
const MAX_MANIFEST: usize = 4 * 1024 * 1024;
/// An image config blob, at most.
const MAX_CONFIG: usize = 4 * 1024 * 1024;
/// Sent with every manifest request so GHCR answers with the digest a
/// `HEAD` names, whatever the stored media type.
const ACCEPT_MANIFESTS: &str = "application/vnd.oci.image.index.v1+json, \
 application/vnd.docker.distribution.manifest.list.v2+json, \
 application/vnd.oci.image.manifest.v1+json, \
 application/vnd.docker.distribution.manifest.v2+json";

/// A content-addressed image id: `sha256:` plus 64 lowercase hex.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct ImageDigest(String);

impl ImageDigest {
    /// `text` accepted only as `sha256:` and 64 lowercase hex digits.
    pub fn parse(text: &str) -> Option<Self> {
        let hex = text.strip_prefix("sha256:")?;
        let lowercase = hex
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        (hex.len() == 64 && lowercase).then(|| Self(text.to_string()))
    }

    /// The digest of bytes as GHCR would compute it.
    fn of(bytes: &[u8]) -> Self {
        Self(format!(
            "sha256:{}",
            hex::encode(sha2::Sha256::digest(bytes))
        ))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ImageDigest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The newest published build of Niles: what `latest` points at and
/// what its config labels say.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Newest {
    /// The digest of the index or manifest `latest` resolves to, the
    /// one GHCR shows and the one a pin would name.
    pub digest: ImageDigest,
    /// The image by digest: `ghcr.io/marknygaard/niles@sha256:…`.
    pub image: String,
    /// The commit the image was built from, from its `revision` label.
    pub commit: Commit,
    /// The image's own `created` label, else its config's `created`.
    pub created: Option<DateTime<Utc>>,
    /// The `version` label without a leading `v`, when it is a version
    /// (`latest` is not).
    pub version: Option<String>,
}

/// GHCR, asked anonymously. No `Default`: building the client can fail,
/// and an `expect` there would panic `serve`.
#[derive(Clone)]
pub struct Registry {
    client: reqwest::Client,
    base: Url,
}

impl Registry {
    /// The registry at `base`: GHCR's own address, or a stand-in in
    /// tests.
    pub fn at(base: &str) -> Result<Self> {
        let base = Url::parse(base).map_err(|e| Error::Malformed {
            what: WHAT,
            reason: format!("{base}: {e}"),
        })?;
        Ok(Self {
            client: http::client()?,
            base,
        })
    }

    /// What `latest` resolves to now. `known` short-circuits the check:
    /// a digest it already has means no manifest is pulled, so a quiet
    /// check costs a token and a HEAD.
    pub async fn newest(&self, known: Option<&Newest>) -> Result<Newest> {
        let token = self.token().await?;
        if let Some(digest) = self.head_digest(&token).await?
            && known.is_some_and(|k| k.digest == digest)
        {
            return Ok(known.expect("is_some_and above").clone());
        }
        let head = self.manifest(&token, TAG).await?;
        let digest = ImageDigest::of(&head.bytes);
        if let Some(header) = head.digest_header
            && header != digest
        {
            return Err(Error::Malformed {
                what: "the newest image",
                reason: "its digest doesn't match its content".to_string(),
            });
        }
        if known.is_some_and(|k| k.digest == digest) {
            return Ok(known.expect("is_some_and above").clone());
        }
        let parsed: ManifestHead =
            serde_json::from_slice(&head.bytes).map_err(|e| Error::Malformed {
                what: WHAT,
                reason: format!("its manifest isn't JSON: {e}"),
            })?;
        let config_digest = match kind_of(head.content_type.as_deref(), &parsed) {
            Kind::Index => self.child_config_digest(&token, &parsed).await?,
            Kind::Manifest => parsed
                .config
                .as_ref()
                .and_then(|c| c.digest.as_deref())
                .and_then(ImageDigest::parse)
                .ok_or_else(|| Error::Malformed {
                    what: WHAT,
                    reason: "its manifest names no config".to_string(),
                })?,
        };
        let config = self.config_of(&token, &config_digest).await?;
        Self::labels(digest, &config)
    }

    /// The anonymous pull token. GHCR answers a token or an
    /// `access_token`; both are the bearer.
    async fn token(&self) -> Result<String> {
        let mut url = self.base.clone();
        if let Ok(mut path) = url.path_segments_mut() {
            path.pop_if_empty().push("token");
        }
        url.query_pairs_mut()
            .append_pair("scope", &format!("repository:{REPOSITORY}:pull"))
            .append_pair("service", "ghcr.io");
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| http::send_error(WHAT, &e))?;
        let status = response.status();
        if !status.is_success() {
            return Err(Error::Status {
                what: WHAT,
                status: status.as_u16(),
            });
        }
        let bytes = http::read_capped(response, MAX_TOKEN, WHAT).await?;
        #[derive(Deserialize)]
        struct TokenAnswer {
            #[serde(alias = "access_token")]
            token: String,
        }
        let answer: TokenAnswer = serde_json::from_slice(&bytes).map_err(|e| Error::Malformed {
            what: WHAT,
            reason: format!("its token answer doesn't read: {e}"),
        })?;
        Ok(answer.token)
    }

    /// The digest `latest` points at, by HEAD. `None` when the registry
    /// doesn't say (an older registry, a stand-in); the GET then
    /// decides from the bytes.
    async fn head_digest(&self, token: &str) -> Result<Option<ImageDigest>> {
        let response = self
            .client
            .head(self.manifest_url(TAG))
            .bearer_auth(token)
            .header("accept", ACCEPT_MANIFESTS)
            .send()
            .await
            .map_err(|e| http::send_error(WHAT, &e))?;
        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(Error::Status {
                what: WHAT,
                status: 404,
            });
        }
        if !status.is_success() {
            return Err(Error::Status {
                what: WHAT,
                status: status.as_u16(),
            });
        }
        Ok(response
            .headers()
            .get("docker-content-digest")
            .and_then(|v| v.to_str().ok())
            .and_then(ImageDigest::parse))
    }

    /// A manifest by reference (`latest` or a digest), read at most
    /// [`MAX_MANIFEST`] bytes, with whatever digest and content type
    /// the response carried.
    async fn manifest(&self, token: &str, reference: &str) -> Result<ManifestBody> {
        let response = self
            .client
            .get(self.manifest_url(reference))
            .bearer_auth(token)
            .header("accept", ACCEPT_MANIFESTS)
            .send()
            .await
            .map_err(|e| http::send_error(WHAT, &e))?;
        let status = response.status();
        if !status.is_success() {
            return Err(Error::Status {
                what: WHAT,
                status: status.as_u16(),
            });
        }
        let digest_header = response
            .headers()
            .get("docker-content-digest")
            .and_then(|v| v.to_str().ok())
            .and_then(ImageDigest::parse);
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let bytes = http::read_capped(response, MAX_MANIFEST, WHAT).await?;
        Ok(ManifestBody {
            bytes,
            digest_header,
            content_type,
        })
    }

    /// The config digest of the linux/amd64 image out of an index.
    /// Attestation manifests are skipped; an index with nothing else is
    /// refused rather than guessed at.
    async fn child_config_digest(&self, token: &str, index: &ManifestHead) -> Result<ImageDigest> {
        let manifests = index.manifests.as_deref().unwrap_or_default();
        let usable = manifests.iter().find(|m| {
            m.platform.as_ref().is_some_and(|p| {
                p.os.as_deref() == Some("linux") && p.architecture.as_deref() == Some("amd64")
            })
        });
        let fallback = manifests.iter().find(|m| {
            let unknown = m.platform.as_ref().is_some_and(|p| {
                p.os.as_deref() == Some("unknown") && p.architecture.as_deref() == Some("unknown")
            });
            let attestation = m
                .annotations
                .as_ref()
                .and_then(|a| a.get("vnd.docker.reference.type"))
                .is_some_and(|v| v == "attestation-manifest");
            !unknown && !attestation
        });
        let child = usable
            .or(fallback)
            .and_then(|m| m.digest.as_deref())
            .ok_or_else(|| Error::Malformed {
                what: WHAT,
                reason: "it has no image for linux/amd64".to_string(),
            })?;
        let head = self.manifest(token, child).await?;
        let parsed: ManifestHead =
            serde_json::from_slice(&head.bytes).map_err(|e| Error::Malformed {
                what: WHAT,
                reason: format!("its image manifest isn't JSON: {e}"),
            })?;
        parsed
            .config
            .as_ref()
            .and_then(|c| c.digest.as_deref())
            .and_then(ImageDigest::parse)
            .ok_or_else(|| Error::Malformed {
                what: WHAT,
                reason: "its image manifest names no config".to_string(),
            })
    }

    /// The image config blob behind `digest`. The registry answers
    /// with a 307 to GitHub's CDN; the default redirect policy follows
    /// it, dropping the token cross-host on purpose.
    async fn config_of(&self, token: &str, digest: &ImageDigest) -> Result<Config> {
        let mut url = self.base.clone();
        if let Ok(mut path) = url.path_segments_mut() {
            path.pop_if_empty()
                .extend(["v2", "marknygaard", "niles", "blobs", digest.as_str()]);
        }
        let response = self
            .client
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| http::send_error(WHAT, &e))?;
        let status = response.status();
        if !status.is_success() {
            return Err(Error::Status {
                what: WHAT,
                status: status.as_u16(),
            });
        }
        let bytes = http::read_capped(response, MAX_CONFIG, WHAT).await?;
        serde_json::from_slice(&bytes).map_err(|e| Error::Malformed {
            what: WHAT,
            reason: format!("its image config isn't JSON: {e}"),
        })
    }

    /// What the config's labels say, as a [`Newest`]. A missing
    /// `revision` label is a failure, never a guess: without it there
    /// is no honest answer to "is this newer".
    fn labels(digest: ImageDigest, config: &Config) -> Result<Newest> {
        let labels = config.config.as_ref().and_then(|c| c.labels.as_ref());
        let commit = labels
            .and_then(|l| l.get("org.opencontainers.image.revision"))
            .and_then(|v| Commit::parse(v))
            .ok_or_else(|| Error::Malformed {
                what: "the newest image",
                reason: "it doesn't say which commit it was built from".to_string(),
            })?;
        let created = labels
            .and_then(|l| l.get("org.opencontainers.image.created"))
            .or(config.created.as_ref())
            .and_then(|v| DateTime::parse_from_rfc3339(v).ok())
            .map(|at| at.with_timezone(&Utc));
        let version = labels
            .and_then(|l| l.get("org.opencontainers.image.version"))
            .map(|v| v.strip_prefix('v').unwrap_or(v))
            .filter(|v| semver::Version::parse(v).is_ok())
            .map(str::to_string);
        Ok(Newest {
            image: format!("{IMAGE}@{digest}"),
            digest,
            commit,
            created,
            version,
        })
    }

    fn manifest_url(&self, reference: &str) -> Url {
        let mut url = self.base.clone();
        if let Ok(mut path) = url.path_segments_mut() {
            path.pop_if_empty()
                .extend(["v2", "marknygaard", "niles", "manifests", reference]);
        }
        url
    }
}

struct ManifestBody {
    bytes: Vec<u8>,
    digest_header: Option<ImageDigest>,
    content_type: Option<String>,
}

/// Both manifest shapes read in one pass; every field optional so one
/// stranger's document never panics the check.
#[derive(Deserialize)]
struct ManifestHead {
    #[serde(default, rename = "mediaType")]
    media_type: Option<String>,
    #[serde(default)]
    manifests: Option<Vec<Descriptor>>,
    #[serde(default)]
    config: Option<Descriptor>,
}

#[derive(Deserialize)]
struct Descriptor {
    #[serde(default)]
    digest: Option<String>,
    #[serde(default)]
    platform: Option<Platform>,
    #[serde(default)]
    annotations: Option<HashMap<String, String>>,
}

#[derive(Deserialize)]
struct Platform {
    #[serde(default)]
    os: Option<String>,
    #[serde(default)]
    architecture: Option<String>,
}

/// The config blob: only the labels and the top-level `created`.
#[derive(Deserialize)]
struct Config {
    config: Option<ConfigField>,
    #[serde(default)]
    created: Option<String>,
}

#[derive(Deserialize)]
struct ConfigField {
    /// OCI's spelling: Docker wrote this key, and metadata-action
    /// keeps it.
    #[serde(rename = "Labels")]
    labels: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Index,
    Manifest,
}

/// Whether the bytes are an index or a single manifest: the content
/// type first, then the JSON's own `mediaType`, then which keys it
/// has. An index answers the `manifest.list` check before anything can
/// match `manifest.v2` inside the same media type.
fn kind_of(content_type: Option<&str>, head: &ManifestHead) -> Kind {
    if let Some(media) = content_type {
        let media = media.split(';').next().unwrap_or("").trim();
        if media.contains("manifest.list") || media.contains("image.index") {
            return Kind::Index;
        }
        if media.contains("image.manifest") || media.contains("manifest.v2") {
            return Kind::Manifest;
        }
    }
    if let Some(media) = head.media_type.as_deref() {
        if media.contains("manifest.list") || media.contains("image.index") {
            return Kind::Index;
        }
        if media.contains("image.manifest") || media.contains("manifest.v2") {
            return Kind::Manifest;
        }
    }
    if head.manifests.is_some() {
        Kind::Index
    } else {
        Kind::Manifest
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";
    const CREATED: &str = "2026-11-05T09:30:00Z";

    fn sha256(bytes: &[u8]) -> String {
        format!("sha256:{}", hex::encode(sha2::Sha256::digest(bytes)))
    }

    /// An OCI index holding an amd64 image manifest and an attestation.
    fn index_bytes(child_digest: &str) -> Vec<u8> {
        serde_json::json!({
            "schemaVersion": 2,
            "mediaType": "application/vnd.oci.image.index.v1+json",
            "manifests": [
                {
                    "digest": child_digest,
                    "mediaType": "application/vnd.oci.image.manifest.v1+json",
                    "platform": {"os": "linux", "architecture": "amd64"}
                },
                {
                    "digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                    "mediaType": "application/vnd.oci.image.manifest.v1+json",
                    "platform": {"os": "unknown", "architecture": "unknown"},
                    "annotations": {"vnd.docker.reference.type": "attestation-manifest"}
                }
            ]
        })
        .to_string()
        .into_bytes()
    }

    fn attestation_only_index_bytes() -> Vec<u8> {
        serde_json::json!({
            "schemaVersion": 2,
            "mediaType": "application/vnd.oci.image.index.v1+json",
            "manifests": [
                {
                    "digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                    "mediaType": "application/vnd.oci.image.manifest.v1+json",
                    "platform": {"os": "unknown", "architecture": "unknown"},
                    "annotations": {"vnd.docker.reference.type": "attestation-manifest"}
                }
            ]
        })
        .to_string()
        .into_bytes()
    }

    fn manifest_bytes(config_digest: &str) -> Vec<u8> {
        serde_json::json!({
            "schemaVersion": 2,
            "mediaType": "application/vnd.oci.image.manifest.v1+json",
            "config": {
                "mediaType": "application/vnd.oci.image.config.v1+json",
                "digest": config_digest,
                "size": 3
            }
        })
        .to_string()
        .into_bytes()
    }

    fn config_bytes(revision: &str, created: &str, version: &str) -> Vec<u8> {
        serde_json::json!({
            "architecture": "amd64",
            "created": created,
            "config": {
                "Labels": {
                    "org.opencontainers.image.revision": revision,
                    "org.opencontainers.image.created": created,
                    "org.opencontainers.image.version": version
                }
            }
        })
        .to_string()
        .into_bytes()
    }

    const INDEX_MEDIA: &str = "application/vnd.oci.image.index.v1+json";
    const MANIFEST_MEDIA: &str = "application/vnd.oci.image.manifest.v1+json";

    /// Serve a token, HEAD `latest` by the body's digest, the manifest
    /// at `reference`, and its config blob; return the registry pointed
    /// at the stand-in. Tests that need a HEAD saying nothing reset and
    /// mount their own.
    async fn serving(
        server: &MockServer,
        reference: &str,
        body: Vec<u8>,
        media_type: &str,
        config: Vec<u8>,
    ) -> Registry {
        Mock::given(method("GET"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "token": "t"
            })))
            .mount(server)
            .await;
        Mock::given(method("HEAD"))
            .and(path(format!("/v2/marknygaard/niles/manifests/{reference}")))
            .and(header("authorization", "Bearer t"))
            .respond_with(
                ResponseTemplate::new(200).insert_header("docker-content-digest", sha256(&body)),
            )
            .mount(server)
            .await;
        let digest = sha256(&body);
        Mock::given(method("GET"))
            .and(path(format!("/v2/marknygaard/niles/manifests/{reference}")))
            .and(header("authorization", "Bearer t"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(body, media_type)
                    .insert_header("docker-content-digest", digest),
            )
            .mount(server)
            .await;
        let config_digest = sha256(&config);
        Mock::given(method("GET"))
            .and(path(format!("/v2/marknygaard/niles/blobs/{config_digest}")))
            .and(header("authorization", "Bearer t"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(config, "application/octet-stream"),
            )
            .mount(server)
            .await;
        Registry::at(&server.uri()).expect("the stand-in parses")
    }

    async fn index_registry(server: &MockServer, revision: &str, version: &str) -> Registry {
        let config = config_bytes(revision, CREATED, version);
        let child = manifest_bytes(&sha256(&config));
        let child_digest = sha256(&child);
        let index = index_bytes(&child_digest);
        let registry = serving(server, "latest", index, INDEX_MEDIA, config).await;
        Mock::given(method("GET"))
            .and(path(format!(
                "/v2/marknygaard/niles/manifests/{child_digest}"
            )))
            .and(header("authorization", "Bearer t"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(child, MANIFEST_MEDIA)
                    .insert_header("docker-content-digest", child_digest),
            )
            .mount(server)
            .await;
        registry
    }

    #[tokio::test]
    async fn an_index_resolves_to_the_amd64_image() {
        let server = MockServer::start().await;
        let registry = index_registry(&server, SHA, "0.7.0").await;
        let newest = registry.newest(None).await.expect("the check works");
        // Two answers carry no digest header back in this flow: the
        // computed one stands. Only equality with `known` matters.
        assert_eq!(newest.commit.as_str(), SHA);
        assert_eq!(
            newest.created.map(|c| c.to_rfc3339()),
            Some("2026-11-05T09:30:00+00:00".to_string())
        );
        assert_eq!(newest.version.as_deref(), Some("0.7.0"));
        assert_eq!(newest.image, format!("{IMAGE}@{}", newest.digest.as_str()));
        assert!(ImageDigest::parse(newest.digest.as_str()).is_some());
    }

    #[tokio::test]
    async fn a_single_manifest_works() {
        let server = MockServer::start().await;
        let config = config_bytes(SHA, CREATED, "0.7.0");
        let manifest = manifest_bytes(&sha256(&config));
        let registry = serving(&server, "latest", manifest, MANIFEST_MEDIA, config).await;
        let newest = registry.newest(None).await.expect("the check works");
        assert_eq!(newest.commit.as_str(), SHA);
        assert_eq!(newest.version.as_deref(), Some("0.7.0"));
    }

    #[tokio::test]
    async fn a_known_digest_short_circuits_the_check() {
        let server = MockServer::start().await;
        let registry = index_registry(&server, SHA, "0.7.0").await;
        let known = registry.newest(None).await.expect("the check works");
        server.reset().await;
        Mock::given(method("GET"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "token": "t"
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("HEAD"))
            .and(path("/v2/marknygaard/niles/manifests/latest"))
            .and(header("authorization", "Bearer t"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("docker-content-digest", known.digest.as_str()),
            )
            .expect(1)
            .mount(&server)
            .await;
        // Nothing else is mounted: a GET would 404 and fail the check.
        let again = registry
            .newest(Some(&known))
            .await
            .expect("the known digest answers");
        assert_eq!(again, known);
    }

    #[tokio::test]
    async fn a_head_without_a_digest_header_falls_back_to_get() {
        let server = MockServer::start().await;
        let config = config_bytes(SHA, CREATED, "0.7.0");
        let manifest = manifest_bytes(&sha256(&config));
        Mock::given(method("GET"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "token": "t"
            })))
            .mount(&server)
            .await;
        // The HEAD says nothing: the GET decides from the bytes.
        Mock::given(method("HEAD"))
            .and(path("/v2/marknygaard/niles/manifests/latest"))
            .and(header("authorization", "Bearer t"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v2/marknygaard/niles/manifests/latest"))
            .and(header("authorization", "Bearer t"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(manifest.clone(), MANIFEST_MEDIA)
                    .insert_header("docker-content-digest", sha256(&manifest)),
            )
            .mount(&server)
            .await;
        let config_digest = sha256(&config);
        Mock::given(method("GET"))
            .and(path(format!("/v2/marknygaard/niles/blobs/{config_digest}")))
            .and(header("authorization", "Bearer t"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(config, "application/octet-stream"),
            )
            .mount(&server)
            .await;
        let registry = Registry::at(&server.uri()).expect("the stand-in parses");
        let newest = registry.newest(None).await.expect("the GET decides");
        assert_eq!(newest.commit.as_str(), SHA);
    }

    #[tokio::test]
    async fn a_failing_token_endpoint_is_a_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let registry = Registry::at(&server.uri()).expect("the stand-in parses");
        let why = registry.newest(None).await.unwrap_err();
        assert!(
            matches!(why, Error::Status { status: 500, .. }),
            "the token failure is named: {why}"
        );
    }

    #[tokio::test]
    async fn a_missing_latest_is_a_404() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "token": "t"
            })))
            .mount(&server)
            .await;
        Mock::given(method("HEAD"))
            .and(path("/v2/marknygaard/niles/manifests/latest"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let registry = Registry::at(&server.uri()).expect("the stand-in parses");
        let why = registry.newest(None).await.unwrap_err();
        assert!(
            matches!(why, Error::Status { status: 404, .. }),
            "404 is named: {why}"
        );
    }

    #[tokio::test]
    async fn a_config_without_a_revision_label_is_malformed() {
        let server = MockServer::start().await;
        let config = config_bytes("", CREATED, "0.7.0");
        let manifest = manifest_bytes(&sha256(&config));
        let registry = serving(&server, "latest", manifest, MANIFEST_MEDIA, config).await;
        let why = registry.newest(None).await.unwrap_err();
        let text = why.to_string();
        assert!(
            matches!(why, Error::Malformed { .. }),
            "no guessing without a revision: {text}"
        );
        assert!(
            text.contains("commit"),
            "the reason says what is missing: {text}"
        );
    }

    #[tokio::test]
    async fn an_index_with_only_attestations_is_malformed() {
        let server = MockServer::start().await;
        let config = config_bytes(SHA, CREATED, "0.7.0");
        let index = attestation_only_index_bytes();
        let registry = serving(&server, "latest", index, INDEX_MEDIA, config).await;
        let why = registry.newest(None).await.unwrap_err();
        assert!(
            matches!(why, Error::Malformed { .. }),
            "nothing usable in the index: {why}"
        );
    }

    #[tokio::test]
    async fn a_redirected_blob_is_followed() {
        let server = MockServer::start().await;
        let config = config_bytes(SHA, CREATED, "0.7.0");
        let manifest = manifest_bytes(&sha256(&config));
        Mock::given(method("GET"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "token": "t"
            })))
            .mount(&server)
            .await;
        Mock::given(method("HEAD"))
            .and(path("/v2/marknygaard/niles/manifests/latest"))
            .and(header("authorization", "Bearer t"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("docker-content-digest", sha256(&manifest)),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v2/marknygaard/niles/manifests/latest"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(manifest.clone(), MANIFEST_MEDIA)
                    .insert_header("docker-content-digest", sha256(&manifest)),
            )
            .mount(&server)
            .await;
        let config_digest = sha256(&config);
        Mock::given(method("GET"))
            .and(path(format!("/v2/marknygaard/niles/blobs/{config_digest}")))
            .respond_with(
                ResponseTemplate::new(307)
                    .insert_header("location", format!("{}/cdn/{config_digest}", server.uri())),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/cdn/{config_digest}")))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(config, "application/octet-stream"),
            )
            .mount(&server)
            .await;
        let registry = Registry::at(&server.uri()).expect("the stand-in parses");
        let newest = registry.newest(None).await.expect("the blob arrives");
        assert_eq!(newest.commit.as_str(), SHA);
    }

    #[tokio::test]
    async fn a_latest_version_label_is_no_version() {
        let server = MockServer::start().await;
        let registry = index_registry(&server, SHA, "latest").await;
        let newest = registry.newest(None).await.expect("the check works");
        assert_eq!(newest.version, None);
    }

    #[tokio::test]
    async fn a_version_label_keeps_its_digits() {
        let server = MockServer::start().await;
        let registry = index_registry(&server, SHA, "v0.7.0").await;
        let newest = registry.newest(None).await.expect("the check works");
        assert_eq!(newest.version.as_deref(), Some("0.7.0"));
    }

    #[test]
    fn a_digest_is_sha256_and_64_lowercase_hex() {
        let good = format!("sha256:{}", "a".repeat(64));
        assert!(ImageDigest::parse(&good).is_some());
        for bad in [
            "sha256:",
            &format!("sha256:{}", "A".repeat(64)),
            &format!("sha256:{}", "a".repeat(63)),
            &format!("sha1:{}", "a".repeat(64)),
            &good[..70],
        ] {
            assert!(ImageDigest::parse(bad).is_none(), "{bad} is not a digest");
        }
    }
}
