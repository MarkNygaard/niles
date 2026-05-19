//! Shared state plumbed through axum handlers.

use niles_core::DeviceRegistry;
use std::sync::Arc;

/// State shared with every request. `Clone` is cheap (just bumps
/// reference counts).
#[derive(Clone)]
pub struct AppState {
    pub registry: Arc<DeviceRegistry>,
}

impl AppState {
    pub fn new(registry: Arc<DeviceRegistry>) -> Self {
        Self { registry }
    }
}
