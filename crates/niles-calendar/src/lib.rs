//! niles-calendar — each person's calendars, through Microsoft Graph.
//!
//! A person connects their own accounts (an Outlook.com one, a work one
//! on Entra ID) from their profile; the voice tools read and write them
//! only for that person, and only as far as each connection's access —
//! which is the scope Microsoft granted, not a switch kept here.

pub mod error;
pub mod graph;
pub mod model;
pub mod service;
pub mod store;

pub use error::{Error, Result};
pub use graph::{MicrosoftApp, Tokens};
pub use model::{Access, Connection, Event, NewEvent};
pub use service::{Backend, Calendars, Change, Graph, Listed};
pub use store::{ConnectionStore, MemoryConnections};
