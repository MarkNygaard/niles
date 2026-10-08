//! niles-nemlig — the household's account at nemlig.com.
//!
//! nemlig.com has no public API. This talks to the JSON endpoints its
//! own website uses, the way several open-source clients do — most
//! completely <https://github.com/AndersMollgaard/nemlig>, whose notes
//! this follows. It can break whenever the site changes, and everything
//! that uses it has to degrade to "the shopping list still works".
//!
//! Deliberately not here: placing an order or anything about payment.
//! Niles fills the basket and reserves a delivery time; paying happens
//! at nemlig.com, by a person — [`CHECKOUT`] is where.

/// Where a person reviews the basket and pays.
pub const CHECKOUT: &str = "https://www.nemlig.com/basket";

mod basket;
mod client;
mod error;
mod product;

pub use basket::{Basket, BasketLine, DeliveryDay, DeliverySlot};
pub use client::{Credentials, NemligClient};
pub use error::{Error, Result};
pub use product::Product;
