//! niles-presence — presence sources, aggregation, home-state events.

pub mod aggregator;
pub mod error;
pub mod source;
pub mod state;
pub mod tado;
pub mod transport;

pub use aggregator::PresenceAggregator;
pub use error::{Error, Result};
pub use source::PresenceSource;
pub use state::{HomeState, Override, PresenceSignal, PresenceSnapshot, SourceReading};
pub use tado::{TadoConfig, TadoSource};
pub use transport::{HttpTadoTransport, TadoTransport};
