//! niles-core — event bus, device registry, shared types.
//!
//! This crate has no business logic. It provides the type system other
//! crates compose against: device identifiers, the runtime registry,
//! and the internal event bus.

pub mod device;
pub mod error;
pub mod event;
pub mod registry;

pub use device::{Device, DeviceClass, DeviceId, DeviceName, DeviceState, RoomName};
pub use error::{Error, Result};
pub use event::{Event, EventBus};
pub use registry::DeviceRegistry;
