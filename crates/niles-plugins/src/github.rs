//! Finding a plugin's release on GitHub and downloading its assets.
//!
//! The REST API without a token, so only public repos, and GitHub's
//! limit of 60 requests an hour per address applies: a preview costs
//! one or two, an update check one per plugin. Assets are downloaded
//! from the URLs the API gives, with a cap on each that is checked
//! against the size GitHub reports, the `Content-Length`, and the bytes
//! as they arrive, so an oversized asset is refused without being held.

use crate::install::Refusal;
use serde::Deserialize;
use std::time::Duration;
use url::Url;

/// Where the API is.
pub const API: &str = "https://api.github.com";

/// `niles-integration.toml`, at most.
pub const MAX_MANIFEST: usize = 64 * 1024;
/// An API answer (a release, or a page of them with their notes), at most.
pub const MAX_ANSWER: usize = 2 * 1024 * 1024;

/// How long one API request may take.
const API_TIMEOUT: Duration = Duration::from_secs(20);
/// How long one download may take: 64 MiB on a slow line.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// A GitHub repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    pub owner: String,
    pub name: String,
}

impl Repo {
    /// `https://github.com/<owner>/<repo>`, as it is kept and shown.
    pub fn url(&self) -> String {
        format!("https://github.com/{}/{}", self.owner, self.name)
    }

    /// `owner/repo`.
    pub fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

/// What somebody pasted, as a repo and the release tag the URL named,
/// if it named one: `https://github.com/jane/niles-spotprice`, with
/// `.git`, a trailing slash or `/releases/tag/v0.1.0`, or
/// `jane/niles-spotprice`.
pub fn parse_repo(input: &str) -> Result<(Repo, Option<String>), String> {
    let input = input.trim();
    let not_a_repo = || {
        format!(
            "{input:?} is not a GitHub repository. Paste its address, like \
             https://github.com/jane/niles-spotprice, or jane/niles-spotprice."
        )
    };
    let path = if input.contains("://") {
        let url = Url::parse(input).map_err(|_| not_a_repo())?;
        if !matches!(url.scheme(), "https" | "http")
            || !matches!(url.host_str(), Some("github.com" | "www.github.com"))
        {
            return Err(format!(
                "{input:?} is not on github.com; plugins are installed from GitHub releases."
            ));
        }
        url.path().to_string()
    } else {
        input
            .strip_prefix("www.github.com/")
            .or_else(|| input.strip_prefix("github.com/"))
            .unwrap_or(input)
            .to_string()
    };
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    let (owner, name, rest) = match parts.as_slice() {
        [owner, name, rest @ ..] => (*owner, *name, rest),
        _ => return Err(not_a_repo()),
    };
    let name = name.strip_suffix(".git").unwrap_or(name);
    let tag = match rest {
        [] => None,
        ["releases"] | ["releases", "latest"] => None,
        ["releases", "tag", tag] => Some(tag.to_string()),
        _ => return Err(not_a_repo()),
    };
    let owner_ok = (1..=39).contains(&owner.len())
        && owner.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    let name_ok = (1..=100).contains(&name.len())
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if !owner_ok || !name_ok {
        return Err(not_a_repo());
    }
    if let Some(tag) = &tag {
        check_tag(tag)?;
    }
    Ok((
        Repo {
            owner: owner.to_string(),
            name: name.to_string(),
        },
        tag,
    ))
}

/// A tag as a request may name one.
pub fn check_tag(tag: &str) -> Result<(), String> {
    if tag.is_empty() || tag.len() > 128 || tag.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(format!("{tag:?} is not a release tag"));
    }
    Ok(())
}

/// A release, as much of it as Niles reads.
#[derive(Debug, Clone, Deserialize)]
pub struct Release {
    pub tag_name: String,
    /// The release's page.
    pub html_url: String,
    /// The release notes.
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

/// A file attached to a release.
#[derive(Debug, Clone, Deserialize)]
pub struct Asset {
    pub name: String,
    pub size: u64,
    pub browser_download_url: String,
    /// `sha256:<hex>`, where GitHub reports one.
    #[serde(default)]
    pub digest: Option<String>,
}

impl Release {
    pub fn asset(&self, name: &str) -> Option<&Asset> {
        self.assets.iter().find(|a| a.name == name)
    }

    /// The tag as a version, with or without its `v`.
    pub fn version(&self) -> Option<semver::Version> {
        semver::Version::parse(self.tag_name.strip_prefix('v').unwrap_or(&self.tag_name)).ok()
    }
}

/// The GitHub REST API, without a token.
#[derive(Clone)]
pub struct GitHub {
    client: reqwest::Client,
    api: Url,
}

impl Default for GitHub {
    fn default() -> Self {
        Self::at(API).expect("the API's own address parses")
    }
}

impl GitHub {
    /// The API at `base`: GitHub's own, or a stand-in in tests.
    pub fn at(base: &str) -> Result<Self, String> {
        let api = Url::parse(base).map_err(|e| format!("{base}: {e}"))?;
        let client = reqwest::Client::builder()
            .user_agent(concat!("niles/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self { client, api })
    }

    fn endpoint(&self, repo: &Repo, rest: &[&str]) -> Url {
        let mut url = self.api.clone();
        if let Ok(mut path) = url.path_segments_mut() {
            path.pop_if_empty()
                .extend(["repos", repo.owner.as_str(), repo.name.as_str()])
                .extend(rest);
        }
        url
    }

    /// The release tagged `tag`, or the latest: not a draft, not a
    /// pre-release.
    pub async fn release(&self, repo: &Repo, tag: Option<&str>) -> Result<Release, Refusal> {
        let url = match tag {
            Some(tag) => self.endpoint(repo, &["releases", "tags", tag]),
            None => self.endpoint(repo, &["releases", "latest"]),
        };
        if let Some(release) = self.get_json::<Release>(url).await? {
            return Ok(release);
        }
        // A 404 is no repo, a private one, or a repo without that
        // release; only asking for the repo tells them apart.
        let found = self
            .get_json::<serde_json::Value>(self.endpoint(repo, &[]))
            .await?;
        let slug = repo.slug();
        Err(Refusal::NotFound(match (found, tag) {
            (None, _) => format!(
                "There is no public repository at github.com/{slug}. Private repositories \
                 cannot be installed from yet."
            ),
            (Some(_), Some(tag)) => format!("{slug} has no release tagged {tag}."),
            (Some(_), None) => format!(
                "{slug} has no releases yet. Niles installs the latest published release; \
                 drafts and pre-releases do not count."
            ),
        }))
    }

    /// The newest releases, newest first: the first page of 30.
    pub async fn releases(&self, repo: &Repo) -> Result<Vec<Release>, Refusal> {
        let mut url = self.endpoint(repo, &["releases"]);
        url.query_pairs_mut().append_pair("per_page", "30");
        self.get_json(url).await?.ok_or_else(|| {
            Refusal::NotFound(format!(
                "There is no public repository at github.com/{} any more.",
                repo.slug()
            ))
        })
    }

    /// `None` for a 404.
    async fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        url: Url,
    ) -> Result<Option<T>, Refusal> {
        let response = self
            .client
            .get(url)
            .header("accept", "application/vnd.github+json")
            .header("x-github-api-version", "2022-11-28")
            .timeout(API_TIMEOUT)
            .send()
            .await
            .map_err(|e| Refusal::Upstream(format!("GitHub could not be reached: {e}")))?;
        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if let Some(why) = rate_limited(&response) {
            return Err(Refusal::Upstream(why));
        }
        if !status.is_success() {
            return Err(Refusal::Upstream(format!(
                "GitHub answered {status}; try again in a while."
            )));
        }
        let bytes = read_capped(response, MAX_ANSWER, "GitHub's answer").await?;
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| Refusal::Upstream(format!("GitHub's answer did not read: {e}")))
    }

    /// Download `asset`, refusing it past `cap` bytes, and check it
    /// against the digest GitHub reports for it, where it reports one.
    pub async fn download(&self, asset: &Asset, cap: usize) -> Result<Vec<u8>, Refusal> {
        let name = &asset.name;
        if asset.size > cap as u64 {
            return Err(too_big(name, asset.size, cap));
        }
        let response = self
            .client
            .get(&asset.browser_download_url)
            .timeout(DOWNLOAD_TIMEOUT)
            .send()
            .await
            .map_err(|e| Refusal::Upstream(format!("{name} could not be downloaded: {e}")))?;
        if let Some(why) = rate_limited(&response) {
            return Err(Refusal::Upstream(why));
        }
        if !response.status().is_success() {
            return Err(Refusal::Upstream(format!(
                "{name} could not be downloaded: GitHub answered {}",
                response.status()
            )));
        }
        let bytes = read_capped(response, cap, name).await?;
        if let Some(digest) = &asset.digest
            && let Some(expected) = digest.strip_prefix("sha256:")
            && !expected.eq_ignore_ascii_case(&niles_db::hex_digest(&bytes))
        {
            return Err(Refusal::Invalid(format!(
                "{name} does not match the sha256 GitHub reports for it, so it is not the file \
                 the release describes."
            )));
        }
        Ok(bytes)
    }
}

/// Why GitHub refused, when it refused for the rate limit.
fn rate_limited(response: &reqwest::Response) -> Option<String> {
    let status = response.status();
    let header = |name: &str| {
        response
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    let exhausted = header("x-ratelimit-remaining").as_deref() == Some("0");
    if !(status == reqwest::StatusCode::TOO_MANY_REQUESTS
        || (status == reqwest::StatusCode::FORBIDDEN && exhausted))
    {
        return None;
    }
    let reset = header("x-ratelimit-reset")
        .and_then(|s| s.parse::<i64>().ok())
        .and_then(|secs| chrono::DateTime::from_timestamp(secs, 0))
        .map(|at| format!(" It resets at {} UTC.", at.format("%H:%M")))
        .unwrap_or_default();
    Some(format!(
        "GitHub's limit of 60 requests an hour without a token is used up for now.{reset}"
    ))
}

async fn read_capped(
    mut response: reqwest::Response,
    cap: usize,
    name: &str,
) -> Result<Vec<u8>, Refusal> {
    if let Some(length) = response.content_length()
        && length > cap as u64
    {
        return Err(too_big(name, length, cap));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| Refusal::Upstream(format!("{name} could not be downloaded: {e}")))?
    {
        if bytes.len() + chunk.len() > cap {
            return Err(too_big(name, (bytes.len() + chunk.len()) as u64, cap));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn too_big(name: &str, size: u64, cap: usize) -> Refusal {
    Refusal::Invalid(format!(
        "{name} is {}; at most {} is accepted.",
        human_size(size),
        human_size(cap as u64)
    ))
}

/// `64 MiB`, `32 KiB`, `1.5 MiB`.
pub fn human_size(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * 1024;
    let scaled = |unit: u64, name: &str| {
        if bytes.is_multiple_of(unit) {
            format!("{} {name}", bytes / unit)
        } else {
            format!("{:.1} {name}", bytes as f64 / unit as f64)
        }
    };
    match bytes {
        b if b >= MIB => scaled(MIB, "MiB"),
        b if b >= KIB => scaled(KIB, "KiB"),
        b => format!("{b} bytes"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(input: &str) -> (String, Option<String>) {
        let (repo, tag) = parse_repo(input).unwrap_or_else(|e| panic!("{input}: {e}"));
        (repo.slug(), tag)
    }

    #[test]
    fn the_ways_people_paste_a_repo() {
        for input in [
            "https://github.com/jane/niles-spotprice",
            "https://github.com/jane/niles-spotprice/",
            "https://github.com/jane/niles-spotprice.git",
            "http://www.github.com/jane/niles-spotprice",
            "github.com/jane/niles-spotprice",
            "jane/niles-spotprice",
            "  jane/niles-spotprice\n",
            "https://github.com/jane/niles-spotprice/releases",
            "https://github.com/jane/niles-spotprice/releases/latest",
        ] {
            assert_eq!(
                parsed(input),
                ("jane/niles-spotprice".into(), None),
                "{input}"
            );
        }
        assert_eq!(
            parsed("https://github.com/jane/niles-spotprice/releases/tag/v0.2.0"),
            ("jane/niles-spotprice".into(), Some("v0.2.0".into()))
        );
        assert_eq!(
            parse_repo("jane/niles.spot_price").unwrap().0.url(),
            "https://github.com/jane/niles.spot_price"
        );
    }

    #[test]
    fn what_is_not_a_repo_says_so() {
        for input in [
            "",
            "spotprice",
            "https://gitlab.com/jane/niles-spotprice",
            "ftp://github.com/jane/niles-spotprice",
            "https://github.com/jane",
            "https://github.com/jane/niles-spotprice/issues/3",
            "jane/../etc",
            "ja ne/x",
            "jane/x/releases/tag/two words",
        ] {
            assert!(parse_repo(input).is_err(), "{input:?}");
        }
    }

    #[test]
    fn sizes_read_as_people_say_them() {
        assert_eq!(human_size(64 * 1024 * 1024), "64 MiB");
        assert_eq!(human_size(32 * 1024), "32 KiB");
        assert_eq!(human_size(1536 * 1024), "1.5 MiB");
        assert_eq!(human_size(12), "12 bytes");
    }

    #[test]
    fn a_release_is_a_version_with_or_without_its_v() {
        let release = |tag: &str| Release {
            tag_name: tag.into(),
            html_url: String::new(),
            body: None,
            draft: false,
            prerelease: false,
            assets: Vec::new(),
        };
        assert_eq!(release("v1.2.3").version().unwrap().to_string(), "1.2.3");
        assert_eq!(release("1.2.3").version().unwrap().to_string(), "1.2.3");
        assert!(release("latest").version().is_none());
    }
}
