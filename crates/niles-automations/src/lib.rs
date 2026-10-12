//! niles-automations — when-X-do-Y rules, from the config file or made by
//! voice and in the app, driven by EventBus.

pub mod engine;
pub mod error;
pub mod presence_lights;
pub mod rule;
pub mod runs;
pub mod sentence;
pub mod store;
pub mod sun;

pub use engine::{AutomationEngine, DeviceSink, Notifier};
pub use error::{Error, Result};
pub use presence_lights::PresenceLights;
pub use rule::{Action, Condition, Priority, Rule, Trigger};
pub use runs::{Outcome, Run, RunLog};
pub use sentence::sentence;
pub use store::{AutomationPersistence, AutomationStore};
