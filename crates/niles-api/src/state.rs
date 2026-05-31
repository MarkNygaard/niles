//! Shared state plumbed through axum handlers.

use crate::publish::DevicePublisher;
use niles_core::{DeviceRegistry, EventBus};
use std::sync::Arc;

/// State shared with every request. `Clone` is cheap (just bumps
/// reference counts).
#[derive(Clone)]
pub struct AppState {
    pub registry: Arc<DeviceRegistry>,
    pub publisher: Arc<dyn DevicePublisher>,
    pub z2m_prefix: Arc<String>,
    pub event_bus: EventBus,
}

impl AppState {
    pub fn new(
        registry: Arc<DeviceRegistry>,
        publisher: Arc<dyn DevicePublisher>,
        z2m_prefix: Arc<String>,
        event_bus: EventBus,
    ) -> Self {
        Self {
            registry,
            publisher,
            z2m_prefix,
            event_bus,
        }
    }
}
