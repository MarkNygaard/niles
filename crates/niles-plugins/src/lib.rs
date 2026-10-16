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
pub mod github;
mod host;
pub mod http;
mod human;
pub mod images;
pub mod inbound;
pub mod install;
pub mod kept;
pub mod log;
pub mod manifest;
pub mod mqtt;
#[cfg(test)]
mod mqtt_tests;
mod output;
pub mod permissions;
pub mod policy;
pub mod runtime;
pub mod store;
pub mod tools;
pub mod ui;
mod worker;

pub use error::{Error, Result};
pub use github::GitHub;
pub use inbound::{HookAnswer, HookRequest, LastHook};
pub use install::{Inbound, Installer, Leftovers, Listed, Options, Preview, Refusal};
pub use kept::{KeptSecrets, MemoryKept};
pub use manifest::Manifest;
pub use runtime::{House, Limits, PluginInfo, Runtime, Status, SurfaceError};
pub use store::{MemoryStore, Package, PluginStore, Source, Update};
pub use ui::{ActionOutcome, ActionValue, Context, Surface};
