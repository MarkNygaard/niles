//! SearXNG data models.

use serde::{Deserialize, Serialize};

/// A web search request.
#[derive(Debug, Clone, Serialize)]
pub struct SearchRequest {
    pub query: String,
    pub num_results: u8,
}

/// A single search result.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub content: String,
    pub engine: Option<String>,
}

/// Complete search response.
#[derive(Debug, Clone, Serialize)]
pub struct SearchResponse {
    pub query: String,
    pub results: Vec<SearchResult>,
}
