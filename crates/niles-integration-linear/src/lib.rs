//! niles-integration-linear — Linear task-tracker integration.

pub mod client;
pub mod error;
pub mod model;
pub mod transport;

pub use client::{LinearClient, LinearConfig};
pub use error::{Error, Result};
pub use model::{TaskDetail, TaskRef, TaskSummary};
pub use transport::{HttpTransport, LinearTransport};
