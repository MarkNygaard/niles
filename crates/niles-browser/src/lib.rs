//! Niles's own Chrome DevTools Protocol client, and the proxy the browser
//! reaches the internet through.
//!
//! Headless Chromium runs in a container of its own (see ARCHITECTURE.md),
//! and this crate is what talks to it: a websocket connection to its
//! DevTools, [`Browser`] opening one browser context per session,
//! [`Session`] reading a page as text with `[n]` references, scrolling,
//! aiming and acting, and [`Proxy`], a CONNECT-only proxy that applies
//! `niles-websearch`'s address rules to every connection.
//!
//! Niles has its own client rather than a third-party one so the page
//! script, the reading budget, the references and the held-step checks
//! stay exactly the ones Niles means (ARCHITECTURE.md, Big decisions).
//!
//! The CDP client never logs a call's parameters or results: a page's
//! text, text a model types and later a password pass through it.
//!
//! The page script (`src/page.js`) runs in an isolated world the page
//! can't see, so the references it hands out can't be moved or faked by
//! page code.

mod browser;
mod cdp;
mod error;
mod picture;
mod proxy;
mod session;
mod tab;

pub use browser::Browser;
pub use error::{Error, Result};
pub use picture::picture;
pub use proxy::Proxy;
pub use session::Session;
pub use tab::{
    Aim, Choice, Driver, Element, Kind, NodeId, Point, Reading, Rect, Reference, Scroll, Tab,
};

/// Where Niles looks for the browser: the name both the Kustomize Service
/// and the Compose service have.
pub const HOST: &str = "niles-browser";

/// The port Chromium's DevTools listens on in the browser container.
pub const DEVTOOLS_PORT: u16 = 9222;

/// The port Niles's proxy listens on, which the NetworkPolicy lets the
/// browser reach.
pub const PROXY_PORT: u16 = 3128;
