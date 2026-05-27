//! Shared state plumbed through axum handlers.

use crate::publish::DevicePublisher;
use niles_core::DeviceRegistry;
use std::sync::Arc;

/// State shared with every request. `Clone` is cheap (just bumps
/// reference counts).
#[derive(Clone)]
pub struct AppState {
    pub registry: Arc<DeviceRegistry>,
    pub publisher: Arc<dyn DevicePublisher>,
    pub z2m_prefix: Arc<String>,
}

impl AppState {
    pub fn new(
        registry: Arc<DeviceRegistry>,
        publisher: Arc<dyn DevicePublisher>,
        z2m_prefix: Arc<String>,
    ) -> Self {
        Self {
            registry,
            publisher,
            z2m_prefix,
        }
    }
}
