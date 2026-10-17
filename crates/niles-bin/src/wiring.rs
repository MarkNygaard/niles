//! Building the long-lived parts `serve` connects: one module per area, each a set of
//! constructors that read the config and hand back a ready part, or nothing.

pub(crate) mod accounts;
pub(crate) mod automations;
pub(crate) mod llm;
pub(crate) mod memory;
pub(crate) mod mqtt;
pub(crate) mod notifications;
pub(crate) mod presence;
pub(crate) mod speech;
pub(crate) mod stores;
