//! niles-websearch — SearXNG search client.

mod address;
pub mod client;
pub mod error;
pub mod fetch;
pub mod model;
pub mod page;
pub mod transport;

pub use client::{SearXngClient, SearXngConfig};
pub use error::{Error, Result};
pub use fetch::{Fetched, PublicFetcher};
pub use model::{ImageResult, SearchRequest, SearchResponse, SearchResult};
pub use page::{Page, PageReader};
pub use transport::{HttpTransport, SearXngTransport};

/// What Niles says it is to the search instance and to the pages and
/// pictures it fetches.
pub const USER_AGENT: &str = concat!(
    "niles/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/MarkNygaard/niles)"
);
