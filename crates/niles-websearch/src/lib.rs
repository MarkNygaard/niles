//! niles-websearch — SearXNG search client.

pub mod client;
pub mod error;
pub mod model;
pub mod page;
pub mod transport;

pub use client::{SearXngClient, SearXngConfig};
pub use error::{Error, Result};
pub use model::{SearchRequest, SearchResponse, SearchResult};
pub use page::{Page, PageReader};
pub use transport::{HttpTransport, SearXngTransport};
