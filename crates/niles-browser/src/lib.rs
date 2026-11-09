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
//! page code. The same script also fills a saved login or a code in an
//! isolated world in the field's own frame, only on the login's origins,
//! checking the frame's origin as it sets each value; and a signed-in
//! session's page is held to those origins by request interception
//! (`Fetch`), so no main-frame navigation leaves them.

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
    Aim, Asks, Choice, Driver, Element, Filled, Kind, NodeId, Point, Reading, Rect, Reference,
    Scroll, Tab, on_sites,
};

/// Where Niles looks for the browser: the name both the Kustomize Service
/// and the Compose service have.
pub const HOST: &str = "niles-browser";

/// The port Chromium's DevTools listens on in the browser container.
pub const DEVTOOLS_PORT: u16 = 9222;

/// The port Niles's proxy listens on, which the NetworkPolicy lets the
/// browser reach.
pub const PROXY_PORT: u16 = 3128;
