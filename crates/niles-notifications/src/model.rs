//! Data models for notifications: [`Notification`], [`Priority`], [`DeliveryOutcome`].

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Priority level of a notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    /// Low-priority informational messages.
    Routine,
    /// Should be delivered unless quiet hours are active.
    Important,
    /// Always delivered, bypassing quiet hours.
    Urgent,
}

impl Priority {
    /// Returns true if this priority should be suppressed during quiet hours.
    pub fn is_suppressed_by_quiet_hours(&self) -> bool {
        matches!(self, Priority::Routine)
    }

    /// Floor priority during quiet hours: Routine → Important.
    pub fn quiet_floor(&self) -> Self {
        match self {
            Priority::Routine => Priority::Important,
            other => *other,
        }
    }
}

/// Outcome of attempting to deliver a notification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryOutcome {
    /// Delivered successfully.
    Delivered,
    /// Suppressed due to quiet hours.
    Suppressed,
    /// Delivery mechanism failed.
    Failed,
}

/// A single notification record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub id: String,
    pub text: String,
    pub priority: Priority,
    pub room: Option<String>,
    pub outcome: DeliveryOutcome,
    pub created_at: DateTime<Utc>,
}
