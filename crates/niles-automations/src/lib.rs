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
pub mod warnings;

pub use engine::{AutomationEngine, DeviceSink, Notifier, Step, WaitStore, Waiting, Waker};
pub use error::{Error, Result};
pub use presence_lights::PresenceLights;
pub use rule::{
    Action, BlindTarget, Condition, Measure, MotionGroup, Priority, Rule, Trigger, WakePlan,
};
pub use runs::{Outcome, Run, RunLog};
pub use sentence::{describe, sentence};
pub use store::{AutomationPersistence, AutomationStore};
pub use warnings::warnings;
