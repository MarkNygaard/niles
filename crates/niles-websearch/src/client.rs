//! SearXNG HTTP client.

use crate::error::{Error, Result};
use crate::model::{SearchRequest, SearchResponse, SearchResult};
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

        let url = format!(
            "{}?q={}&format=json",
            self.config.base_url,
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
}

#[derive(Deserialize)]
struct SearXngResp {
    results: Vec<SearchResult>,
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
}
