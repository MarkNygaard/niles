//! niles-groceries — the household's shopping list, and what it buys.
//!
//! Two things are kept:
//!
//! - **The list**: what is to be bought, each item with the words it
//!   was asked for in, until it is checked off and cleared away.
//! - **The catalog**: every product that has actually been bought, how
//!   often, and the other words it has been asked for by.
//!
//! The catalog is what turns "milk" into *Letmælk*. A house that buys
//! one kind of milk should not have to say which every time, and
//! nobody should have to teach Niles that up front either — so it
//! learns from the checkbox. An item added as "milk" and checked off as
//! Letmælk makes "milk" an alias of Letmælk. Adding is only a guess;
//! buying is the answer, so nothing is learned until then.

mod error;
mod matching;
mod store;

pub use error::{Error, Result};
pub use matching::normalize;
pub use niles_nemlig::Product as NemligProduct;
pub use store::{Added, Edit, GroceryPersistence, GroceryStore, Item, Product};
