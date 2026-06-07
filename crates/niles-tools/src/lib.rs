//! niles-tools — LLM tool definitions, dispatch, and registry.
//!
//! Composes existing primitives (`DeviceRegistry`, `MqttPublisher`,
//! `format_set_command`, `niles_llm` wire types) into the concrete
//! tool surface the Tier-1 LLM can call. See `builtin` for the five
//! built-ins this crate ships with.

pub mod announce;
pub mod builtin;
pub mod datetime;
pub mod error;
pub mod escalate;
pub mod linear;
pub mod list_recent_notifications;
pub mod presence;
pub mod registry;
pub mod skill;
pub mod tool;
pub mod weather;
pub mod web_search;

pub use announce::{AnnounceTool, register_announce_tool};
pub use builtin::{
    CancelTimer, DeviceStateSnapshotAt, ExplainDeviceState, GetDeviceState, GetTimerRemaining,
    ListAllDevices, ListDevicesInRoom, ListTimers, LookUpCapability, MemoryTool,
    QueryCommandHistory, QueryDeviceStateHistory, SetDevice, SetLightEffect, default_registry,
    register_history_tools, register_memory_tools, register_state_history_tools,
    register_timer_tools, restricted_registry_for_review,
};
pub use datetime::{CurrentDatetimeTool, register_datetime_tool};
pub use error::{Error, Result};
pub use escalate::{EscalateToTier2Tool, register_escalate_tool};
pub use list_recent_notifications::{
    ListRecentNotificationsTool, register_list_recent_notifications_tool,
};
pub use presence::{GetPresenceTool, SetPresenceTool, register_presence_tools};

/// Register both notification tools (announce + list_recent) on a registry.
pub fn register_notification_tools(
    reg: &mut ToolRegistry,
    center: std::sync::Arc<niles_notifications::NotificationCenter>,
) {
    register_announce_tool(reg, center.clone());
    register_list_recent_notifications_tool(reg, center);
}
pub use linear::{CreateTaskTool, GetTaskTool, ListTasksTool, register_linear_tools};
pub use registry::ToolRegistry;
pub use skill::{
    DeleteSkillTool, MintSkillTool, PatchSkillTool, ViewSkillTool, register_skill_tools,
};
pub use tool::{Tool, ToolDescriptor};
pub use weather::{WeatherTool, register_weather_tools};
pub use web_search::{WebSearchTool, register_web_search_tool};
