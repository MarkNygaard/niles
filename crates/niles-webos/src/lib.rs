//! niles-webos — LG webOS TVs, over their local websocket API.
//!
//! Paired once with a prompt on the screen, then reached directly on the
//! local network: no LG account, no cloud. Woken with Wake-on-LAN, since
//! a TV in standby has nothing listening that could be asked to wake.

pub mod error;
mod session;
pub mod tv;
pub mod wake;

pub use error::{Error, Result};
pub use tv::{App, Input, PAIRING, Status, Tv, pair};
pub use wake::wake;
