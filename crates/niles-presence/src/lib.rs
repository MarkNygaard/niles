//! niles-presence — presence sources, aggregation, home-state events.

pub mod aggregator;
pub mod error;
pub mod source;
pub mod state;
pub mod tado;
pub mod tokens;
pub mod transport;
pub mod zones;

pub use aggregator::PresenceAggregator;
pub use error::{Error, Result};
pub use source::PresenceSource;
pub use state::{HomeState, Override, PresenceSignal, PresenceSnapshot, SourceReading};
pub use tado::{DEVICE_CLIENT_ID, DeviceActivation, TadoConfig, TadoSource};
pub use tokens::{MemoryTokenStore, TokenStore};
pub use transport::{HttpTadoTransport, TadoTransport};
pub use zones::{Placed, Zone};
