//! niles-automations — config-defined when-X-do-Y rules driven by EventBus.

pub mod engine;
pub mod error;
pub mod rule;

pub use engine::{AutomationEngine, DeviceSink, Notifier};
pub use error::{Error, Result};
pub use rule::{Action, Condition, Priority, Rule, Trigger};
