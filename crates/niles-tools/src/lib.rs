//! niles-tools — LLM tool definitions, dispatch, and registry.
//!
//! Composes existing primitives (`DeviceRegistry`, `MqttPublisher`,
//! `format_set_command`, `niles_llm` wire types) into the concrete
//! tool surface the Tier-1 LLM can call. See `builtin` for the five
//! built-ins this crate ships with.

pub mod builtin;
pub mod error;
pub mod registry;
pub mod tool;

pub use builtin::{
    CancelTimer, ExplainDeviceState, GetDeviceState, GetTimerRemaining, ListAllDevices,
    ListDevicesInRoom, ListTimers, LookUpCapability, QueryCommandHistory, SetDevice,
    default_registry, register_history_tools, register_timer_tools,
};
pub use error::{Error, Result};
pub use registry::ToolRegistry;
pub use tool::{Tool, ToolDescriptor};
