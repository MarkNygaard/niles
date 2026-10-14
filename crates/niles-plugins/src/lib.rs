//! niles-plugins — third-party integrations, run as sandboxed
//! WebAssembly components.
//!
//! A plugin is a component targeting the `niles:integration/plugin`
//! world (`schemas/niles-integration.wit`), described by its
//! `niles-integration.toml`. The [`Runtime`] runs each one in wasmtime
//! with nothing but its imports: no files, no environment, no sockets,
//! HTTP only to the hosts its manifest declares, a memory cap, and a
//! deadline on every call. Through those imports it joins the house:
//! its devices go into the shared registry under the source
//! `x_<plugin-id>`, its tools into the tool registry, its events onto
//! the bus. The rest of Niles reaches it back through interfaces it
//! already has: the [`Runtime`] is a `niles_mqtt::DeviceCommander` for
//! `x_*` devices, and its tools are in a `niles_tools::DynamicTools`.
//!
//! Spec: ARCHITECTURE.md, "Third-party integrations (plugins)", and the
//! author guide in `docs/integrations/`.

mod bindings;
pub mod check;
pub mod error;
mod host;
pub mod http;
pub mod manifest;
mod output;
pub mod policy;
pub mod runtime;
pub mod store;
pub mod tools;
pub mod ui;
mod worker;

pub use error::{Error, Result};
pub use manifest::Manifest;
pub use runtime::{House, Limits, PluginInfo, Runtime, Status, SurfaceError};
pub use store::{MemoryStore, PluginStore};
pub use ui::{ActionOutcome, ActionValue, Context, Surface};
