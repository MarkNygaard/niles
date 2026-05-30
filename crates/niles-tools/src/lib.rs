//! niles-tools — LLM tool definitions, dispatch, and registry.
//!
//! Composes existing primitives (`DeviceRegistry`, `MqttPublisher`,
//! `format_set_command`, `niles_llm` wire types) into the concrete
//! tool surface the Tier-1 LLM can call. See `builtin` for the five
//! built-ins this crate ships with.

pub mod builtin;
pub mod error;
pub mod registry;
pub mod skill;
pub mod tool;
pub mod weather;
pub mod web_search;

pub use builtin::{
    CancelTimer, DeviceStateSnapshotAt, ExplainDeviceState, GetDeviceState, GetTimerRemaining,
    ListAllDevices, ListDevicesInRoom, ListTimers, LookUpCapability, MemoryTool,
    QueryCommandHistory, QueryDeviceStateHistory, SetDevice, default_registry,
    register_history_tools, register_memory_tools, register_state_history_tools,
    register_timer_tools,
};
pub use error::{Error, Result};
pub use registry::ToolRegistry;
pub use skill::{
    DeleteSkillTool, MintSkillTool, PatchSkillTool, ViewSkillTool, register_skill_tools,
};
pub use tool::{Tool, ToolDescriptor};
pub use weather::{WeatherTool, register_weather_tools};
pub use web_search::{WebSearchTool, register_web_search_tool};
