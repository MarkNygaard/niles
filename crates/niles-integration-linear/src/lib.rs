//! niles-integration-linear — Linear task-tracker integration.

pub mod client;
pub mod error;
pub mod model;
pub mod transport;
pub mod webhook;

pub use client::{LinearClient, LinearConfig};
pub use error::{Error, Result};
pub use model::{TaskDetail, TaskRef, TaskSummary};
pub use transport::{HttpTransport, LinearTransport};
pub use webhook::{
    WebhookNotification, WebhookPayload, notification_for, parse_webhook, verify_signature,
};
