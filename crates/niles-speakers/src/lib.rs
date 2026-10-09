//! niles-speakers — room speaker integration. Sonos SOAP/UPnP client.

pub mod client;
pub mod error;
pub mod favorites;
pub mod household;
pub mod spotify;
pub mod transport;
pub mod tunein;

pub use client::{Media, SonosClient, TransportState, is_radio};
pub use error::{Error, Result};
pub use favorites::Favorite;
pub use household::{SonosRoom, household};
pub use transport::{HttpTransport, SonosTransport};
