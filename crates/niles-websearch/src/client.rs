//! SearXNG HTTP client.

use crate::error::{Error, Result};
use crate::model::{ImageResult, SearchRequest, SearchResponse, SearchResult};
use crate::transport::{HttpTransport, SearXngTransport};
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;

/// Client configuration.
#[derive(Debug, Clone)]
pub struct SearXngConfig {
    pub base_url: String,
    pub request_timeout: Duration,
    pub user_agent: String,
}

/// SearXNG search client.
pub struct SearXngClient {
    transport: Arc<dyn SearXngTransport>,
    config: SearXngConfig,
}

impl SearXngClient {
    /// Create a client using the default [`HttpTransport`].
    pub fn new(config: SearXngConfig) -> Result<Self> {
        let transport = Arc::new(HttpTransport::new(
            &config.user_agent,
            config.request_timeout,
        ));
        Ok(Self::with_transport(transport, config))
    }

    /// Create a client with a custom transport (useful for testing).
    pub fn with_transport(transport: Arc<dyn SearXngTransport>, config: SearXngConfig) -> Self {
        Self { transport, config }
    }

    /// Perform a web search.
    pub async fn search(&self, request: &SearchRequest) -> Result<SearchResponse> {
        if request.query.trim().is_empty() {
            return Err(Error::InvalidInput {
                reason: "query must not be empty".into(),
            });
        }
        if !(1..=20).contains(&request.num_results) {
            return Err(Error::InvalidInput {
                reason: "num_results must be between 1 and 20".into(),
            });
        }

        let sep = if self.config.base_url.contains('?') {
            '&'
        } else {
            '?'
        };
        let url = format!(
            "{}{}q={}&format=json",
            self.config.base_url,
            sep,
            urlencoding::encode(&request.query),
        );

        let body = self.transport.get(&url).await?;
        let parsed: SearXngResp = serde_json::from_str(&body).map_err(|e| Error::Parse {
            reason: e.to_string(),
        })?;

        let mut results = parsed.results;
        results.truncate(request.num_results as usize);

        Ok(SearchResponse {
            query: request.query.clone(),
            results,
        })
    }

    /// Pictures for `query`, from the instance's image engines: at most
    /// `num_results`, each with an https address to the picture. An
    /// instance with no image engine on finds none, which is an empty
    /// list rather than an error.
    pub async fn search_images(&self, query: &str, num_results: u8) -> Result<Vec<ImageResult>> {
        if query.trim().is_empty() {
            return Err(Error::InvalidInput {
                reason: "query must not be empty".into(),
            });
        }
        if !(1..=20).contains(&num_results) {
            return Err(Error::InvalidInput {
                reason: "num_results must be between 1 and 20".into(),
            });
        }
        let mut url =
            reqwest::Url::parse(&self.config.base_url).map_err(|e| Error::InvalidInput {
                reason: format!("the search address does not read: {e}"),
            })?;
        // The instance's address may name a category of its own; this is
        // a search for pictures whatever it names.
        let kept: Vec<(String, String)> = url
            .query_pairs()
            .filter(|(k, _)| !matches!(k.as_ref(), "q" | "format" | "categories"))
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        url.query_pairs_mut()
            .clear()
            .extend_pairs(kept)
            .append_pair("q", query)
            .append_pair("format", "json")
            .append_pair("categories", "images");

        let body = self.transport.get(url.as_str()).await?;
        let parsed: RawImages = serde_json::from_str(&body).map_err(|e| Error::Parse {
            reason: e.to_string(),
        })?;
        Ok(parsed
            .results
            .iter()
            .filter_map(image_result)
            .take(num_results as usize)
            .collect())
    }
}

/// A result of the images category, when it is a picture Niles could
/// fetch: an engine's general results, mixed in, have no `img_src`.
fn image_result(raw: &serde_json::Value) -> Option<ImageResult> {
    let text = |key: &str| {
        raw.get(key)
            .and_then(serde_json::Value::as_str)
            .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|s| !s.is_empty())
    };
    let address = |key: &str| text(key).and_then(|a| https(&a));
    Some(ImageResult {
        image_url: address("img_src")?,
        title: text("title").unwrap_or_default(),
        thumbnail_url: address("thumbnail_src"),
        source: address("url"),
        size: text("resolution"),
        author: text("author"),
        engine: text("engine"),
    })
}

/// `address` as an https one: a protocol-relative `//host/…` is taken
/// as https; plain http, `data:` and anything else is nothing.
fn https(address: &str) -> Option<String> {
    let address = match address.strip_prefix("//") {
        Some(rest) => format!("https://{rest}"),
        None => address.to_string(),
    };
    let url = reqwest::Url::parse(&address).ok()?;
    (url.scheme() == "https" && url.host_str().is_some()).then(|| url.to_string())
}

#[derive(Deserialize)]
struct SearXngResp {
    results: Vec<SearchResult>,
}

#[derive(Deserialize)]
struct RawImages {
    #[serde(default)]
    results: Vec<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;

    #[derive(Clone)]
    struct MockTransport {
        last_url: Arc<Mutex<Option<String>>>,
        responses: Arc<Mutex<Vec<Result<String>>>>,
    }

    impl MockTransport {
        fn new(responses: Vec<Result<String>>) -> Self {
            Self {
                last_url: Arc::new(Mutex::new(None)),
                responses: Arc::new(Mutex::new(responses)),
            }
        }

        fn last_url(&self) -> Option<String> {
            self.last_url.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl SearXngTransport for MockTransport {
        async fn get(&self, url: &str) -> Result<String> {
            *self.last_url.lock().unwrap() = Some(url.to_string());
            self.responses.lock().unwrap().remove(0)
        }
    }

    fn client_with(responses: Vec<Result<String>>) -> (MockTransport, SearXngClient) {
        let mock = MockTransport::new(responses);
        let config = SearXngConfig {
            base_url: "https://search.example.com/search".into(),
            request_timeout: Duration::from_secs(10),
            user_agent: "test".into(),
        };
        let client = SearXngClient::with_transport(Arc::new(mock.clone()), config);
        (mock, client)
    }

    fn search_json() -> String {
        r#"{
            "query": "rust async",
            "results": [
                {
                    "title": "Asynchronous Programming in Rust",
                    "url": "https://rust-lang.github.io/async-book/",
                    "content": "Asynchronous programming in Rust.",
                    "engine": "google"
                },
                {
                    "title": "async-std",
                    "url": "https://async.rs/",
                    "content": "Async version of the Rust standard library.",
                    "engine": "bing"
                },
                {
                    "title": "Tokio",
                    "url": "https://tokio.rs/",
                    "content": "A runtime for writing reliable network applications.",
                    "engine": "duckduckgo"
                }
            ]
        }"#
        .into()
    }

    #[tokio::test]
    async fn search_happy_path() {
        let (_, client) = client_with(vec![Ok(search_json())]);
        let resp = client
            .search(&SearchRequest {
                query: "rust async".into(),
                num_results: 5,
            })
            .await
            .unwrap();
        assert_eq!(resp.query, "rust async");
        assert_eq!(resp.results.len(), 3);
        assert_eq!(resp.results[0].title, "Asynchronous Programming in Rust");
        assert_eq!(
            resp.results[0].url,
            "https://rust-lang.github.io/async-book/"
        );
        assert_eq!(resp.results[0].content, "Asynchronous programming in Rust.");
        assert_eq!(resp.results[0].engine, Some("google".into()));
    }

    #[tokio::test]
    async fn search_truncates_to_num_results() {
        let (_, client) = client_with(vec![Ok(search_json())]);
        let resp = client
            .search(&SearchRequest {
                query: "rust async".into(),
                num_results: 2,
            })
            .await
            .unwrap();
        assert_eq!(resp.results.len(), 2);
        assert_eq!(resp.results[0].title, "Asynchronous Programming in Rust");
        assert_eq!(resp.results[1].title, "async-std");
    }

    #[tokio::test]
    async fn bad_status_propagates() {
        let (_, client) = client_with(vec![Err(Error::BadStatus {
            status: 500,
            body: "oops".into(),
        })]);
        let err = client
            .search(&SearchRequest {
                query: "test".into(),
                num_results: 5,
            })
            .await
            .unwrap_err();
        assert!(matches!(err, Error::BadStatus { status: 500, body } if body == "oops"));
    }

    #[tokio::test]
    async fn junk_json_returns_parse_error() {
        let (_, client) = client_with(vec![Ok("not json".into())]);
        let err = client
            .search(&SearchRequest {
                query: "test".into(),
                num_results: 5,
            })
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Parse { .. }));
    }

    #[tokio::test]
    async fn empty_query_rejected() {
        let (_, client) = client_with(vec![]);
        let err = client
            .search(&SearchRequest {
                query: "".into(),
                num_results: 5,
            })
            .await
            .unwrap_err();
        assert!(
            matches!(err, Error::InvalidInput { reason } if reason.contains("query must not be empty"))
        );
    }

    #[tokio::test]
    async fn whitespace_only_query_rejected() {
        let (_, client) = client_with(vec![]);
        let err = client
            .search(&SearchRequest {
                query: "   ".into(),
                num_results: 5,
            })
            .await
            .unwrap_err();
        assert!(
            matches!(err, Error::InvalidInput { reason } if reason.contains("query must not be empty"))
        );
    }

    #[tokio::test]
    async fn zero_num_results_rejected() {
        let (_, client) = client_with(vec![]);
        let err = client
            .search(&SearchRequest {
                query: "test".into(),
                num_results: 0,
            })
            .await
            .unwrap_err();
        assert!(
            matches!(err, Error::InvalidInput { reason } if reason.contains("num_results must be between 1 and 20"))
        );
    }

    #[tokio::test]
    async fn too_many_num_results_rejected() {
        let (_, client) = client_with(vec![]);
        let err = client
            .search(&SearchRequest {
                query: "test".into(),
                num_results: 21,
            })
            .await
            .unwrap_err();
        assert!(
            matches!(err, Error::InvalidInput { reason } if reason.contains("num_results must be between 1 and 20"))
        );
    }

    #[tokio::test]
    async fn extra_fields_ignored() {
        let json = r#"{
            "query": "test",
            "results": [
                {
                    "title": "T",
                    "url": "https://example.com",
                    "content": "C",
                    "engine": "google",
                    "category": "general",
                    "score": 1.0,
                    "engines": ["google", "bing"],
                    "pretty_url": "example.com"
                }
            ],
            "infoboxes": [],
            "answers": [],
            "suggestions": []
        }"#
        .into();
        let (_, client) = client_with(vec![Ok(json)]);
        let resp = client
            .search(&SearchRequest {
                query: "test".into(),
                num_results: 5,
            })
            .await
            .unwrap();
        assert_eq!(resp.results.len(), 1);
        assert_eq!(resp.results[0].title, "T");
    }

    #[tokio::test]
    async fn missing_optional_fields_default() {
        let json = r#"{
            "query": "test",
            "results": [
                {
                    "title": "T",
                    "url": "https://example.com",
                    "content": "C"
                }
            ]
        }"#
        .into();
        let (_, client) = client_with(vec![Ok(json)]);
        let resp = client
            .search(&SearchRequest {
                query: "test".into(),
                num_results: 5,
            })
            .await
            .unwrap();
        assert_eq!(resp.results[0].engine, None);
    }

    #[tokio::test]
    async fn url_contains_format_json() {
        let (mock, client) = client_with(vec![Ok(search_json())]);
        client
            .search(&SearchRequest {
                query: "test".into(),
                num_results: 5,
            })
            .await
            .unwrap();
        let url = mock.last_url().unwrap();
        assert!(url.contains("format=json"));
    }

    #[tokio::test]
    async fn query_is_percent_encoded() {
        let (mock, client) = client_with(vec![Ok(search_json())]);
        client
            .search(&SearchRequest {
                query: "rust async".into(),
                num_results: 5,
            })
            .await
            .unwrap();
        let url = mock.last_url().unwrap();
        assert!(url.contains("rust%20async"));
    }

    #[tokio::test]
    async fn url_with_existing_query_params() {
        let mock = MockTransport::new(vec![Ok(search_json())]);
        let config = SearXngConfig {
            base_url: "https://search.example.com/search?categories=general".into(),
            request_timeout: Duration::from_secs(10),
            user_agent: "test".into(),
        };
        let client = SearXngClient::with_transport(Arc::new(mock.clone()), config);
        client
            .search(&SearchRequest {
                query: "rust".into(),
                num_results: 5,
            })
            .await
            .unwrap();
        let url = mock.last_url().unwrap();
        assert!(url.contains("categories=general"));
        assert!(url.contains("q=rust"));
        assert!(url.contains("format=json"));
        // Verify there is exactly one '?' separating path and query.
        assert_eq!(url.matches('?').count(), 1);
    }

    #[tokio::test]
    async fn empty_results_array_ok() {
        let (_, client) = client_with(vec![Ok(r#"{"results": []}"#.into())]);
        let resp = client
            .search(&SearchRequest {
                query: "test".into(),
                num_results: 5,
            })
            .await
            .unwrap();
        assert_eq!(resp.results.len(), 0);
    }

    fn images_json() -> String {
        r#"{
            "query": "grand palace bangkok",
            "results": [
                {
                    "title": "Grand  Palace, Bangkok",
                    "url": "https://commons.wikimedia.org/wiki/File:Grand_Palace.jpg",
                    "img_src": "https://upload.wikimedia.org/grand_palace.jpg",
                    "thumbnail_src": "//upload.wikimedia.org/thumb/grand_palace.jpg",
                    "resolution": "4000 x 2667",
                    "author": "J. Doe",
                    "engine": "wikicommons.images"
                },
                { "title": "A page, not a picture", "url": "https://x.example/", "content": "c" },
                { "title": "Plain http", "url": "https://y.example/", "img_src": "http://y.example/a.jpg" },
                { "title": "Inline", "url": "https://z.example/", "img_src": "data:image/png;base64,AAAA" },
                {
                    "title": "Protocol relative",
                    "url": "https://w.example/p",
                    "img_src": "//w.example/b.jpg",
                    "thumbnail_src": "data:image/jpeg;base64,AAAA",
                    "engine": "bing images"
                }
            ]
        }"#
        .into()
    }

    #[tokio::test]
    async fn an_image_search_asks_for_images_and_keeps_the_pictures_it_could_fetch() {
        let mock = MockTransport::new(vec![Ok(images_json())]);
        let config = SearXngConfig {
            base_url: "https://search.example.com/search?categories=general&language=da".into(),
            request_timeout: Duration::from_secs(10),
            user_agent: "test".into(),
        };
        let client = SearXngClient::with_transport(Arc::new(mock.clone()), config);
        let found = client
            .search_images("grand palace bangkok", 5)
            .await
            .unwrap();
        let url = mock.last_url().unwrap();
        assert!(url.contains("categories=images"), "{url}");
        assert!(!url.contains("categories=general"), "{url}");
        assert!(url.contains("language=da"), "{url}");
        assert!(url.contains("format=json"), "{url}");
        assert!(url.contains("q=grand+palace+bangkok"), "{url}");

        assert_eq!(found.len(), 2);
        assert_eq!(found[0].title, "Grand Palace, Bangkok");
        assert_eq!(
            found[0].image_url,
            "https://upload.wikimedia.org/grand_palace.jpg"
        );
        assert_eq!(
            found[0].thumbnail_url.as_deref(),
            Some("https://upload.wikimedia.org/thumb/grand_palace.jpg")
        );
        assert_eq!(
            found[0].source.as_deref(),
            Some("https://commons.wikimedia.org/wiki/File:Grand_Palace.jpg")
        );
        assert_eq!(found[0].size.as_deref(), Some("4000 x 2667"));
        assert_eq!(found[0].author.as_deref(), Some("J. Doe"));
        assert_eq!(found[1].image_url, "https://w.example/b.jpg");
        assert_eq!(found[1].thumbnail_url, None);
    }

    #[tokio::test]
    async fn an_instance_without_image_engines_finds_nothing() {
        let (_, client) = client_with(vec![Ok(
            r#"{"results": [], "unresponsive_engines": []}"#.into()
        )]);
        assert!(client.search_images("palace", 5).await.unwrap().is_empty());
        let (_, client) = client_with(vec![]);
        assert!(client.search_images(" ", 5).await.is_err());
        assert!(client.search_images("palace", 0).await.is_err());
    }

    #[tokio::test]
    async fn missing_results_field_returns_parse_error() {
        let (_, client) = client_with(vec![Ok(r#"{"query": "test"}"#.into())]);
        let err = client
            .search(&SearchRequest {
                query: "test".into(),
                num_results: 5,
            })
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Parse { .. }));
    }
}
