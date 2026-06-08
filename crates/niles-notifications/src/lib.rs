//! niles-notifications — unprompted speech: routing, chimes, quiet hours, recall.

pub mod center;
pub mod error;
pub mod log;
pub mod model;
pub mod quiet;

pub use center::{NotificationCenter, NotificationDelivery};
pub use error::{Error, Result};
pub use log::NotificationLog;
pub use model::{DeliveryOutcome, Notification, Priority};
pub use quiet::{QuietHoursConfig, QuietWindow};
