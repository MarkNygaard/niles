//! niles-speakers — room speaker integration. Sonos SOAP/UPnP client.

pub mod client;
pub mod error;
pub mod household;
pub mod transport;

pub use client::{SonosClient, TransportState};
pub use error::{Error, Result};
pub use household::{SonosRoom, household};
pub use transport::{HttpTransport, SonosTransport};
