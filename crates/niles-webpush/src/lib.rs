//! niles-webpush — notifications on a person's phone and computer, by
//! the browser's own Web Push.
//!
//! A person allows notifications in the app on each device they want
//! them on; the browser hands over a [`Subscription`] (an address at its push service
//! and the keys to encrypt for it), and Niles keeps it under the same
//! person key as their chat. When something of theirs needs them — a
//! document written in the background is ready — [`WebPush`] sends to
//! every device they have. What it tells them is kept too ([`inbox`]),
//! for their Notifications page in the app.
//!
//! Everything is outbound: Niles posts to Apple's, Google's or
//! Mozilla's push service and the service wakes the device. Nothing has
//! to reach Niles from outside.
//!
//! The two standards are small enough to be written here rather than
//! brought in: message encryption ([`encrypt`], RFC 8291) and the
//! signed token that says which server is sending ([`VapidKey`], RFC
//! 8292). Both are pure Rust on RustCrypto, so the build stays free of
//! OpenSSL.

pub mod encrypt;
mod error;
pub mod inbox;
mod send;
mod store;
mod vapid;

pub use error::{Error, Result};
pub use inbox::{Inbox, MemoryInbox, Notice, Notification};
pub use send::{Delivery, Message, TTL, WebPush};
pub use store::{MemoryPush, NewSubscription, PushStore, Subscription, device_label};
pub use vapid::{VapidKey, origin_of};
