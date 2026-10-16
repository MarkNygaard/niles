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

/// A picture an image search found: the picture, and the page it is on.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ImageResult {
    pub title: String,
    /// The picture itself, an https address.
    pub image_url: String,
    /// A small copy of it, when the engine has one.
    pub thumbnail_url: Option<String>,
    /// The page it is on.
    pub source: Option<String>,
    /// "1920 x 1080", when the engine says.
    pub size: Option<String>,
    /// Who made it, when the engine says.
    pub author: Option<String>,
    pub engine: Option<String>,
}
