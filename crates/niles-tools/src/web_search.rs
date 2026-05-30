//! Web search tool — expose SearXNG to the LLM as `web_search`.

use crate::error::{Error, Result};
use crate::registry::ToolRegistry;
use crate::tool::{Tool, ToolDescriptor};
use async_trait::async_trait;
use niles_websearch::{SearXngClient, SearchRequest};
use serde_json::{Value, json};
use std::sync::Arc;

fn map_websearch_err<T>(r: std::result::Result<T, niles_websearch::Error>) -> Result<T> {
    r.map_err(|e| Error::WebSearch(e.to_string()))
}

pub struct WebSearchTool {
    client: Arc<SearXngClient>,
    default_num_results: u8,
}

impl WebSearchTool {
    pub fn new(client: Arc<SearXngClient>, default_num_results: u8) -> Self {
        Self {
            client,
            default_num_results,
        }
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "web_search".into(),
            description: "Search the web for information using SearXNG. Returns a list of \
                relevant results with titles, URLs, and content snippets."
                .into(),
            parameters: json!({
                "type": "object",
                "required": ["query"],
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The search query to look up."
                    },
                    "num_results": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 20,
                        "description": "Maximum number of results to return (1–20)."
                    }
                }
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let query = match args.get("query") {
            Some(v) => v.as_str().ok_or_else(|| Error::InvalidArgs {
                tool: "web_search".into(),
                reason: "query must be a string".into(),
            })?,
            None => {
                return Err(Error::InvalidArgs {
                    tool: "web_search".into(),
                    reason: "query is required".into(),
                });
            }
        };

        if query.trim().is_empty() {
            return Err(Error::InvalidArgs {
                tool: "web_search".into(),
                reason: "query must not be empty".into(),
            });
        }

        let num_results = match args.get("num_results") {
            Some(v) => v.as_u64().ok_or_else(|| Error::InvalidArgs {
                tool: "web_search".into(),
                reason: "num_results must be an integer".into(),
            })?,
            None => self.default_num_results as u64,
        };

        if !(1..=20).contains(&num_results) {
            return Err(Error::InvalidArgs {
                tool: "web_search".into(),
                reason: "num_results must be between 1 and 20".into(),
            });
        }

        let response = map_websearch_err(
            self.client
                .search(&SearchRequest {
                    query: query.into(),
                    num_results: num_results as u8,
                })
                .await,
        )?;

        let results: Vec<Value> = response
            .results
            .iter()
            .map(|r| {
                json!({
                    "title": r.title,
                    "url": r.url,
                    "content": r.content,
                })
            })
            .collect();

        Ok(json!({
            "query": response.query,
            "results": results,
        }))
    }
}

/// Register the web search tool onto an existing registry.
pub fn register_web_search_tool(
    reg: &mut ToolRegistry,
    client: Arc<SearXngClient>,
    default_num_results: u8,
) {
    reg.register(Box::new(WebSearchTool::new(client, default_num_results)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use niles_websearch::{SearXngConfig, SearXngTransport};
    use serde_json::json;
    use std::sync::Mutex;
    use std::time::Duration;

    #[derive(Clone)]
    struct MockTransport {
        last_url: Arc<Mutex<Option<String>>>,
        responses: Arc<Mutex<Vec<std::result::Result<String, niles_websearch::Error>>>>,
    }

    impl MockTransport {
        fn new(responses: Vec<std::result::Result<String, niles_websearch::Error>>) -> Self {
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
        async fn get(&self, url: &str) -> std::result::Result<String, niles_websearch::Error> {
            *self.last_url.lock().unwrap() = Some(url.to_string());
            self.responses.lock().unwrap().remove(0)
        }
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
                },
                {
                    "title": "Futures",
                    "url": "https://docs.rs/futures/",
                    "content": "Zero-cost asynchronous programming in Rust.",
                    "engine": "google"
                },
                {
                    "title": "Rust async book",
                    "url": "https://book.async.rs/",
                    "content": "Learn async programming with Rust.",
                    "engine": "bing"
                }
            ]
        }"#
        .into()
    }

    fn tool_with(
        responses: Vec<std::result::Result<String, niles_websearch::Error>>,
    ) -> (MockTransport, WebSearchTool) {
        let mock = MockTransport::new(responses);
        let config = SearXngConfig {
            base_url: "https://search.example.com/search".into(),
            request_timeout: Duration::from_secs(10),
            user_agent: "test".into(),
        };
        let client = Arc::new(SearXngClient::with_transport(
            Arc::new(mock.clone()),
            config,
        ));
        let tool = WebSearchTool::new(client, 5);
        (mock, tool)
    }

    #[tokio::test]
    async fn happy_path() {
        let (_mock, tool) = tool_with(vec![Ok(search_json())]);
        let result = tool.execute(json!({"query": "rust async"})).await.unwrap();
        assert_eq!(result["query"], "rust async");
        let results = result["results"].as_array().unwrap();
        assert_eq!(results.len(), 5);
        assert_eq!(results[0]["title"], "Asynchronous Programming in Rust");
        assert_eq!(results[0]["url"], "https://rust-lang.github.io/async-book/");
        assert_eq!(results[0]["content"], "Asynchronous programming in Rust.");
        assert!(results[0].get("engine").is_none());
    }

    #[tokio::test]
    async fn default_num_results_applied() {
        let (mock, tool) = tool_with(vec![Ok(search_json())]);
        let result = tool.execute(json!({"query": "rust async"})).await.unwrap();
        let results = result["results"].as_array().unwrap();
        assert_eq!(results.len(), 5);
        let url = mock.last_url().unwrap();
        assert!(!url.contains("num_results"));
    }

    #[tokio::test]
    async fn explicit_num_results_honored() {
        let (_mock, tool) = tool_with(vec![Ok(search_json())]);
        let result = tool
            .execute(json!({"query": "rust async", "num_results": 3}))
            .await
            .unwrap();
        let results = result["results"].as_array().unwrap();
        assert_eq!(results.len(), 3);
    }

    #[tokio::test]
    async fn missing_query_errors() {
        let (_mock, tool) = tool_with(vec![]);
        let err = tool.execute(json!({})).await.unwrap_err();
        assert!(
            matches!(err, Error::InvalidArgs { tool, reason } if tool == "web_search" && reason.contains("query is required"))
        );
    }

    #[tokio::test]
    async fn empty_query_errors() {
        let (_mock, tool) = tool_with(vec![]);
        let err = tool.execute(json!({"query": ""})).await.unwrap_err();
        assert!(
            matches!(err, Error::InvalidArgs { tool, reason } if tool == "web_search" && reason.contains("query must not be empty"))
        );
    }

    #[tokio::test]
    async fn non_string_query_errors() {
        let (_mock, tool) = tool_with(vec![]);
        let err = tool.execute(json!({"query": 42})).await.unwrap_err();
        assert!(
            matches!(err, Error::InvalidArgs { tool, reason } if tool == "web_search" && reason.contains("query must be a string"))
        );
    }

    #[tokio::test]
    async fn num_results_zero_errors() {
        let (_mock, tool) = tool_with(vec![]);
        let err = tool
            .execute(json!({"query": "test", "num_results": 0}))
            .await
            .unwrap_err();
        assert!(
            matches!(err, Error::InvalidArgs { tool, reason } if tool == "web_search" && reason.contains("num_results must be between 1 and 20"))
        );
    }

    #[tokio::test]
    async fn num_results_too_large_errors() {
        let (_mock, tool) = tool_with(vec![]);
        let err = tool
            .execute(json!({"query": "test", "num_results": 100}))
            .await
            .unwrap_err();
        assert!(
            matches!(err, Error::InvalidArgs { tool, reason } if tool == "web_search" && reason.contains("num_results must be between 1 and 20"))
        );
    }

    #[tokio::test]
    async fn whitespace_only_query_errors() {
        let (_mock, tool) = tool_with(vec![]);
        let err = tool.execute(json!({"query": "   "})).await.unwrap_err();
        assert!(
            matches!(err, Error::InvalidArgs { tool, reason } if tool == "web_search" && reason.contains("query must not be empty"))
        );
    }

    #[tokio::test]
    async fn empty_results_from_upstream() {
        let (_mock, tool) = tool_with(vec![Ok(r#"{"results": []}"#.into())]);
        let result = tool.execute(json!({"query": "test"})).await.unwrap();
        assert_eq!(result["query"], "test");
        let results = result["results"].as_array().unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn client_error_propagates() {
        let (_mock, tool) = tool_with(vec![Err(niles_websearch::Error::BadStatus {
            status: 500,
            body: "server error".into(),
        })]);
        let err = tool.execute(json!({"query": "test"})).await.unwrap_err();
        assert!(matches!(err, Error::WebSearch(reason) if reason.contains("500")));
    }
}
