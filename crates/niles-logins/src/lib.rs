//! niles-logins — each person's saved logins for Niles's browser.
//!
//! A login is a label, one or more sites as their whole origin, and a
//! sealed username and password, all checked before anything is kept.
//! Why they are kept this way is in ARCHITECTURE.md's "Saved logins".
//!
//! Everything here belongs to one person, named by the same key as
//! their chat history, so what they save in the app and what a card in
//! the chat asked for reach the same data, and nobody else's.

mod activity;
mod error;
mod model;
mod origin;
mod store;
mod totp;

pub use activity::{ACTIVITY_DAYS, ACTIVITY_SHOWN, Activity, Context, Used};
pub use error::{Error, Result};
pub use model::{
    Credentials, KeyChange, Login, LoginEdit, LoginRef, MAX_CODE, MAX_LABEL, MAX_ORIGINS,
    MAX_PASSWORD, MAX_USERNAME, NewLogin, Secret, code, label, origins,
};
pub use origin::Origin;
pub use store::{LoginStore, MemoryLogins, PROFILE_DAYS};
pub use totp::{Authenticator, MAX_AUTHENTICATOR, authenticator};
