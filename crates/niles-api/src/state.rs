//! Shared state plumbed through axum handlers.

use crate::publish::DevicePublisher;
use niles_core::{DeviceRegistry, EventBus};
use niles_notifications::NotificationCenter;
use std::sync::Arc;

/// State required to verify and route Linear webhooks.
pub struct LinearWebhookState {
    pub secret: Vec<u8>,
    pub team: String,
    pub notify_room: Option<String>,
    pub center: Arc<NotificationCenter>,
}

/// State shared with every request. `Clone` is cheap (just bumps
/// reference counts).
#[derive(Clone)]
pub struct AppState {
    pub registry: Arc<DeviceRegistry>,
    pub publisher: Arc<dyn DevicePublisher>,
    pub z2m_prefix: Arc<String>,
    pub event_bus: EventBus,
    pub linear_webhook: Option<Arc<LinearWebhookState>>,
    /// Absent for subcommands that serve the device API without a config
    /// store; the `/config` routes report that rather than 500ing.
    pub config: Option<Arc<niles_config::ConfigStore>>,
    /// Recent log lines, when the binary installed a buffer. Absent for
    /// subcommands that don't, and `/logs` says so rather than 500ing.
    pub logs: Option<crate::logs::LogBuffer>,
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
            linear_webhook: None,
            config: None,
            logs: None,
        }
    }

    pub fn with_logs(mut self, logs: Option<crate::logs::LogBuffer>) -> Self {
        self.logs = logs;
        self
    }

    pub fn with_config_store(mut self, store: Option<Arc<niles_config::ConfigStore>>) -> Self {
        self.config = store;
        self
    }

    pub fn with_linear_webhook(mut self, w: Option<Arc<LinearWebhookState>>) -> Self {
        self.linear_webhook = w;
        self
    }
}
