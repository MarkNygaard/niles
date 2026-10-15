//! niles-calendar — each person's calendars: Microsoft (Graph), Google
//! (the Calendar API) and iCloud (CalDAV).
//!
//! A person connects their own accounts from their profile; the voice
//! tools read and write them only for that person, and only as far as
//! each connection's access — which Microsoft and Google grant as a
//! scope, and which for iCloud is a rule kept here.

pub mod caldav;
pub mod error;
pub mod gmail;
pub mod google;
pub mod graph;
pub mod graph_mail;
pub mod icloud_mail;
pub mod ics;
pub mod mail;
pub mod model;
pub mod service;
pub mod store;

pub use error::{Error, Result};
pub use google::GoogleApp;
pub use graph::{MicrosoftApp, Tokens};
pub use model::{Access, Connection, Event, MailAccess, NewEvent, Provider, Services};
pub use service::{Apps, Backend, Calendars, Change, Listed, Made, MailListed, MailRead, Network};
pub use store::{ConnectionStore, MemoryConnections};
