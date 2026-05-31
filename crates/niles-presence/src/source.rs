//! Presence source trait.

use crate::error::Result;
use crate::state::PresenceSignal;
use async_trait::async_trait;

/// A source of presence readings (e.g. Tado, router-attached devices,
/// Bluetooth proximity, etc.).
#[async_trait]
pub trait PresenceSource: Send + Sync {
    /// Poll the source for the latest presence signal.
    async fn poll(&self) -> Result<PresenceSignal>;

    /// Human-readable source name (used in logs).
    fn name(&self) -> &str;
}
