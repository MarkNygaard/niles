use crate::error::{Error, Result};
use crate::matching::{close_enough, normalize};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex, MutexGuard};

/// Longer than any product name, short enough that a sentence the
/// recogniser ran together is refused rather than shopped for.
const MAX_LEN: usize = 80;

/// One thing on the list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub id: u64,
    /// What goes in the basket: "Letmælk".
    pub name: String,
    /// The words it was asked for in — "milk" — when they are not its
    /// name. Becomes an alias of whatever the item is checked off as.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub said: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quantity: Option<String>,
    pub added_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<DateTime<Utc>>,
}

/// Something this household has bought.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Product {
    pub name: String,
    /// Other words it has been asked for by, normalized.
    #[serde(default)]
    pub aliases: Vec<String>,
    pub bought: u32,
    pub last_bought: DateTime<Utc>,
}

/// What adding did.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Added {
    pub item: Item,
    /// It was on the list already, and nothing new was added.
    pub already: bool,
}

/// A change to one item. Absent fields are left alone.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Edit {
    #[serde(default)]
    pub name: Option<String>,
    /// An empty string clears it.
    #[serde(default)]
    pub quantity: Option<String>,
    #[serde(default)]
    pub checked: Option<bool>,
}

/// Somewhere to keep the whole document after every change.
///
/// Handed the whole thing rather than the change, like scenes: a
/// household list is a few kilobytes, and one way to be written is one
/// way to be right.
pub trait GroceryPersistence: Send + Sync {
    fn store(&self, document: &str);
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Book {
    #[serde(default)]
    next_id: u64,
    #[serde(default)]
    items: Vec<Item>,
    #[serde(default)]
    products: Vec<Product>,
}

/// The list and the catalog, in memory, written through to a sink.
#[derive(Default)]
pub struct GroceryStore {
    book: Mutex<Book>,
    sink: Option<Arc<dyn GroceryPersistence>>,
}

impl GroceryStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Read a document a sink was handed.
    pub fn from_json(document: &str) -> serde_json::Result<Self> {
        Ok(Self {
            book: Mutex::new(serde_json::from_str(document)?),
            sink: None,
        })
    }

    pub fn with_sink(mut self, sink: Arc<dyn GroceryPersistence>) -> Self {
        self.sink = Some(sink);
        self
    }

    /// Everything on the list: still to buy in the order it was added,
    /// then what is already in the basket.
    pub fn list(&self) -> Vec<Item> {
        let book = self.lock();
        let mut items = book.items.clone();
        items.sort_by_key(|i| (i.checked_at.is_some(), i.checked_at, i.id));
        items
    }

    /// The products bought most, most first, that are not on the list
    /// right now — what a "the usual" row offers.
    pub fn usual(&self, limit: usize) -> Vec<String> {
        let book = self.lock();
        let wanted: Vec<String> = book
            .items
            .iter()
            .filter(|i| i.checked_at.is_none())
            .map(|i| normalize(&i.name))
            .collect();
        let mut products: Vec<&Product> = book
            .products
            .iter()
            .filter(|p| !wanted.contains(&normalize(&p.name)))
            .collect();
        products.sort_by_key(|p| std::cmp::Reverse(rank(p)));
        products
            .into_iter()
            .take(limit)
            .map(|p| p.name.clone())
            .collect()
    }

    /// Product names, most bought first.
    pub fn known_products(&self, limit: usize) -> Vec<String> {
        let book = self.lock();
        let mut products: Vec<&Product> = book.products.iter().collect();
        products.sort_by_key(|p| std::cmp::Reverse(rank(p)));
        products
            .into_iter()
            .take(limit)
            .map(|p| p.name.clone())
            .collect()
    }

    /// Put something on the list.
    ///
    /// `said` is the request in the asker's words. What goes on the
    /// list is, in order: the product those words have meant before;
    /// else `product`, the asker's own guess (the LLM's, by voice),
    /// matched against the catalog in turn; else the words themselves.
    pub fn add(&self, said: &str, product: Option<&str>, quantity: Option<&str>) -> Result<Added> {
        let said = said.trim();
        let product = product.map(str::trim).filter(|p| !p.is_empty());
        let quantity = cleaned(quantity);
        check_len("item", said)?;
        if let Some(product) = product {
            check_len("product", product)?;
        }

        self.mutate(|book| {
            let name = book
                .resolve(said)
                .map(|p| p.name.clone())
                .or_else(|| {
                    product.map(|p| {
                        book.resolve(p)
                            .map_or_else(|| capitalized(p), |p| p.name.clone())
                    })
                })
                .unwrap_or_else(|| capitalized(said));
            if name.is_empty() {
                return Err(Error::Invalid {
                    kind: "item",
                    reason: "there is nothing to add".into(),
                });
            }

            let key = normalize(&name);
            if let Some(item) = book
                .items
                .iter_mut()
                .find(|i| i.checked_at.is_none() && normalize(&i.name) == key)
            {
                if quantity.is_some() {
                    item.quantity = quantity;
                }
                return Ok(Added {
                    item: item.clone(),
                    already: true,
                });
            }

            book.next_id += 1;
            let item = Item {
                id: book.next_id,
                name,
                said: (!said.is_empty() && normalize(said) != key).then(|| said.to_string()),
                quantity,
                added_at: Utc::now(),
                checked_at: None,
            };
            book.items.push(item.clone());
            Ok(Added {
                item,
                already: false,
            })
        })
    }

    /// Rename, requantify, check off or uncheck one item.
    ///
    /// Checking off is a purchase: it counts toward the product, and
    /// teaches the catalog that the words the item was asked for in
    /// mean this. Unchecking takes the purchase back.
    pub fn update(&self, id: u64, edit: Edit) -> Result<Item> {
        if let Some(name) = &edit.name {
            check_len("name", name.trim())?;
            if name.trim().is_empty() {
                return Err(Error::Invalid {
                    kind: "name",
                    reason: "an item needs a name".into(),
                });
            }
        }
        if let Some(quantity) = &edit.quantity {
            check_len("quantity", quantity.trim())?;
        }

        self.mutate(|book| {
            let index = book
                .items
                .iter()
                .position(|i| i.id == id)
                .ok_or(Error::NotFound { id })?;

            if edit.checked == Some(false) && book.items[index].checked_at.is_some() {
                book.items[index].checked_at = None;
                let name = book.items[index].name.clone();
                book.unrecord(&name);
            }

            if let Some(name) = &edit.name {
                if book.items[index].checked_at.is_some() {
                    // The purchase was counted under the old name.
                    return Err(Error::Invalid {
                        kind: "edit",
                        reason: "uncheck it before renaming it".into(),
                    });
                }
                let item = &mut book.items[index];
                // What it was called is what it was asked for, if
                // nothing else was: "Milk" renamed to Letmælk should
                // teach the same as "milk" heard and bought as Letmælk.
                if item.said.is_none() {
                    item.said = Some(item.name.clone());
                }
                item.name = name.trim().to_string();
            }
            if let Some(quantity) = &edit.quantity {
                book.items[index].quantity = cleaned(Some(quantity));
            }

            if edit.checked == Some(true) && book.items[index].checked_at.is_none() {
                let now = Utc::now();
                book.items[index].checked_at = Some(now);
                let item = book.items[index].clone();
                book.record(&item.name, item.said.as_deref(), now);
            }
            Ok(book.items[index].clone())
        })
    }

    /// Take one item off, checked or not.
    pub fn remove(&self, id: u64) -> Result<Item> {
        self.mutate(|book| {
            let index = book
                .items
                .iter()
                .position(|i| i.id == id)
                .ok_or(Error::NotFound { id })?;
            Ok(book.items.remove(index))
        })
    }

    /// Take something off by what it is called — "take the milk off".
    ///
    /// Only what is still to be bought: something already in the basket
    /// is not what anybody means.
    pub fn remove_named(&self, text: &str) -> Option<Item> {
        let wanted = normalize(text);
        if wanted.is_empty() {
            return None;
        }
        self.mutate(|book| {
            let resolved = book.resolve(text).map(|p| normalize(&p.name));
            let index = book.items.iter().position(|i| {
                i.checked_at.is_none()
                    && (normalize(&i.name) == wanted
                        || i.said.as_deref().map(normalize) == Some(wanted.clone())
                        || Some(normalize(&i.name)) == resolved)
            });
            Ok(index.map(|i| book.items.remove(i)))
        })
        .ok()
        .flatten()
    }

    /// Clear away what is in the basket. Returns how many went.
    pub fn clear_checked(&self) -> usize {
        self.mutate(|book| {
            let before = book.items.len();
            book.items.retain(|i| i.checked_at.is_none());
            Ok(before - book.items.len())
        })
        .unwrap_or(0)
    }

    fn lock(&self) -> MutexGuard<'_, Book> {
        // A panic mid-change leaves a list that is at worst one edit
        // short, which is better than no list.
        self.book.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn mutate<T>(&self, change: impl FnOnce(&mut Book) -> Result<T>) -> Result<T> {
        let mut book = self.lock();
        let out = change(&mut book)?;
        if let Some(sink) = &self.sink {
            match serde_json::to_string(&*book) {
                Ok(document) => sink.store(&document),
                Err(e) => tracing::error!("[groceries] could not serialize the list: {e}"),
            }
        }
        Ok(out)
    }
}

impl Book {
    /// The product these words mean, if the catalog knows.
    ///
    /// Exact first — the name or a learned alias — and only then a near
    /// miss. When several fit equally, the one bought most wins: a house
    /// that says "milk" for two kinds means the one it buys more of.
    fn resolve(&self, text: &str) -> Option<&Product> {
        let wanted = normalize(text);
        if wanted.is_empty() {
            return None;
        }
        let exact = self
            .products
            .iter()
            .filter(|p| normalize(&p.name) == wanted || p.aliases.contains(&wanted))
            .max_by_key(|p| rank(p));
        if exact.is_some() {
            return exact;
        }
        self.products
            .iter()
            .filter_map(|p| {
                std::iter::once(normalize(&p.name))
                    .chain(p.aliases.iter().cloned())
                    .filter_map(|known| close_enough(&wanted, &known))
                    .min()
                    .map(|d| (d, p))
            })
            .min_by(|(da, a), (db, b)| da.cmp(db).then(rank(b).cmp(&rank(a))))
            .map(|(_, p)| p)
    }

    fn record(&mut self, name: &str, said: Option<&str>, now: DateTime<Utc>) {
        let key = normalize(name);
        let index = match self.products.iter().position(|p| normalize(&p.name) == key) {
            Some(i) => i,
            None => {
                self.products.push(Product {
                    name: name.to_string(),
                    aliases: Vec::new(),
                    bought: 0,
                    last_bought: now,
                });
                self.products.len() - 1
            }
        };
        let product = &mut self.products[index];
        product.bought += 1;
        product.last_bought = now;
        if let Some(said) = said.map(normalize)
            && !said.is_empty()
            && said != key
            && !product.aliases.contains(&said)
        {
            product.aliases.push(said);
        }
    }

    /// Take back one purchase. A product nobody has bought after all is
    /// not in the catalog: a mis-tap on the first purchase must not
    /// teach anything.
    fn unrecord(&mut self, name: &str) {
        let key = normalize(name);
        if let Some(index) = self.products.iter().position(|p| normalize(&p.name) == key) {
            let product = &mut self.products[index];
            product.bought = product.bought.saturating_sub(1);
            if product.bought == 0 {
                self.products.remove(index);
            }
        }
    }
}

fn rank(p: &Product) -> (u32, DateTime<Utc>) {
    (p.bought, p.last_bought)
}

fn cleaned(text: Option<&str>) -> Option<String> {
    text.map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}

fn check_len(kind: &'static str, text: &str) -> Result<()> {
    if text.chars().count() > MAX_LEN {
        return Err(Error::Invalid {
            kind,
            reason: format!("longer than {MAX_LEN} characters"),
        });
    }
    Ok(())
}

/// "mælk" → "Mælk", the way a list is written. Nothing else changes:
/// "iPhone-oplader" keeps its own capitals.
fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Add, then check off as bought.
    fn buy(store: &GroceryStore, said: &str, product: Option<&str>) -> Item {
        let added = store.add(said, product, None).unwrap();
        store
            .update(
                added.item.id,
                Edit {
                    checked: Some(true),
                    ..Edit::default()
                },
            )
            .unwrap()
    }

    fn check(store: &GroceryStore, id: u64, checked: bool) -> Item {
        store
            .update(
                id,
                Edit {
                    checked: Some(checked),
                    ..Edit::default()
                },
            )
            .unwrap()
    }

    #[test]
    fn a_new_item_is_written_the_way_a_list_is() {
        let store = GroceryStore::new();
        let added = store.add("mælk", None, None).unwrap();
        assert_eq!(added.item.name, "Mælk");
        // Its own name, so there is nothing to learn from it.
        assert_eq!(added.item.said, None);
    }

    #[test]
    fn a_guess_is_used_when_the_catalog_has_nothing() {
        let store = GroceryStore::new();
        let added = store.add("milk", Some("mælk"), None).unwrap();
        assert_eq!(added.item.name, "Mælk");
        assert_eq!(added.item.said.as_deref(), Some("milk"));
    }

    #[test]
    fn checking_off_teaches_what_the_words_meant() {
        let store = GroceryStore::new();
        let added = store.add("milk", Some("Mælk"), None).unwrap();
        // Bought as Letmælk, not the guess.
        store
            .update(
                added.item.id,
                Edit {
                    name: Some("Letmælk".into()),
                    checked: Some(true),
                    ..Edit::default()
                },
            )
            .unwrap();
        store.clear_checked();

        let again = store.add("milk", Some("Mælk"), None).unwrap();
        assert_eq!(again.item.name, "Letmælk");
    }

    #[test]
    fn nothing_is_learned_until_it_is_bought() {
        let store = GroceryStore::new();
        let added = store.add("milk", None, None).unwrap();
        store
            .update(
                added.item.id,
                Edit {
                    name: Some("Letmælk".into()),
                    ..Edit::default()
                },
            )
            .unwrap();
        store.remove(added.item.id).unwrap();

        assert_eq!(store.add("milk", None, None).unwrap().item.name, "Milk");
        assert!(store.known_products(10).is_empty());
    }

    #[test]
    fn renaming_what_was_typed_teaches_it_too() {
        let store = GroceryStore::new();
        let added = store.add("milk", None, None).unwrap();
        assert_eq!(added.item.name, "Milk");
        store
            .update(
                added.item.id,
                Edit {
                    name: Some("Letmælk".into()),
                    checked: Some(true),
                    ..Edit::default()
                },
            )
            .unwrap();
        store.clear_checked();
        assert_eq!(store.add("milk", None, None).unwrap().item.name, "Letmælk");
    }

    #[test]
    fn unchecking_a_first_purchase_forgets_the_product() {
        let store = GroceryStore::new();
        let item = buy(&store, "milk", Some("Letmælk"));
        check(&store, item.id, false);
        assert!(store.known_products(10).is_empty());
    }

    #[test]
    fn unchecking_a_repeat_purchase_keeps_the_product() {
        let store = GroceryStore::new();
        buy(&store, "Letmælk", None);
        store.clear_checked();
        let item = buy(&store, "Letmælk", None);
        check(&store, item.id, false);
        assert_eq!(store.known_products(10), vec!["Letmælk"]);
    }

    #[test]
    fn a_mishearing_finds_the_product_it_was_near() {
        let store = GroceryStore::new();
        buy(&store, "Letmælk", None);
        store.clear_checked();
        assert_eq!(
            store.add("let melk", None, None).unwrap().item.name,
            "Letmælk"
        );
    }

    #[test]
    fn the_catalog_beats_a_guess() {
        // A learned alias is the household's own answer; the LLM's
        // translation is only a guess.
        let store = GroceryStore::new();
        buy(&store, "milk", Some("Letmælk"));
        store.clear_checked();
        assert_eq!(
            store.add("milk", Some("Mælk"), None).unwrap().item.name,
            "Letmælk"
        );
    }

    #[test]
    fn a_guess_is_matched_against_the_catalog_too() {
        let store = GroceryStore::new();
        buy(&store, "Letmælk", None);
        store.clear_checked();
        assert_eq!(
            store
                .add("semi-skimmed", Some("letmælk"), None)
                .unwrap()
                .item
                .name,
            "Letmælk"
        );
    }

    #[test]
    fn the_kind_bought_more_wins_a_shared_word() {
        let store = GroceryStore::new();
        for _ in 0..3 {
            buy(&store, "milk", Some("Letmælk"));
            store.clear_checked();
        }
        // Once, asked for as milk and bought as Sødmælk instead.
        let item = store.add("milk", None, None).unwrap().item;
        store
            .update(
                item.id,
                Edit {
                    name: Some("Sødmælk".into()),
                    checked: Some(true),
                    ..Edit::default()
                },
            )
            .unwrap();
        store.clear_checked();
        assert_eq!(store.add("milk", None, None).unwrap().item.name, "Letmælk");
    }

    #[test]
    fn asking_twice_does_not_list_it_twice() {
        let store = GroceryStore::new();
        let first = store.add("Rugbrød", None, None).unwrap();
        let second = store.add("rugbrød", None, Some("2")).unwrap();
        assert!(second.already);
        assert_eq!(second.item.id, first.item.id);
        assert_eq!(second.item.quantity.as_deref(), Some("2"));
        assert_eq!(store.list().len(), 1);
    }

    #[test]
    fn something_in_the_basket_can_be_asked_for_again() {
        let store = GroceryStore::new();
        buy(&store, "Rugbrød", None);
        assert!(!store.add("Rugbrød", None, None).unwrap().already);
    }

    #[test]
    fn the_list_puts_what_is_left_to_buy_first() {
        let store = GroceryStore::new();
        let a = store.add("Æg", None, None).unwrap().item;
        store.add("Smør", None, None).unwrap();
        check(&store, a.id, true);
        let names: Vec<String> = store.list().into_iter().map(|i| i.name).collect();
        assert_eq!(names, vec!["Smør", "Æg"]);
    }

    #[test]
    fn a_checked_item_cannot_be_renamed() {
        let store = GroceryStore::new();
        let item = buy(&store, "Smør", None);
        let renamed = store.update(
            item.id,
            Edit {
                name: Some("Bregott".into()),
                ..Edit::default()
            },
        );
        assert!(matches!(renamed, Err(Error::Invalid { .. })));
    }

    #[test]
    fn an_empty_quantity_clears_it() {
        let store = GroceryStore::new();
        let item = store.add("Æg", None, Some("12")).unwrap().item;
        let item = store
            .update(
                item.id,
                Edit {
                    quantity: Some(" ".into()),
                    ..Edit::default()
                },
            )
            .unwrap();
        assert_eq!(item.quantity, None);
    }

    #[test]
    fn removing_by_name_finds_it_by_its_words_too() {
        let store = GroceryStore::new();
        buy(&store, "milk", Some("Letmælk"));
        store.clear_checked();
        store.add("Letmælk", None, None).unwrap();
        assert_eq!(store.remove_named("milk").unwrap().name, "Letmælk");
        assert!(store.list().is_empty());
    }

    #[test]
    fn the_usual_leaves_out_what_is_already_wanted() {
        let store = GroceryStore::new();
        buy(&store, "Letmælk", None);
        buy(&store, "Rugbrød", None);
        store.clear_checked();
        store.add("Rugbrød", None, None).unwrap();
        assert_eq!(store.usual(5), vec!["Letmælk"]);
    }

    #[test]
    fn a_run_on_sentence_is_refused() {
        let store = GroceryStore::new();
        let long = "and then we also need ".repeat(10);
        assert!(store.add(&long, None, None).is_err());
    }

    #[test]
    fn an_unknown_id_is_not_found() {
        let store = GroceryStore::new();
        assert!(matches!(store.remove(7), Err(Error::NotFound { id: 7 })));
    }

    #[test]
    fn every_change_reaches_the_sink_and_reads_back() {
        struct Keep(Mutex<Option<String>>);
        impl GroceryPersistence for Keep {
            fn store(&self, document: &str) {
                *self.0.lock().unwrap() = Some(document.to_string());
            }
        }
        let sink = Arc::new(Keep(Mutex::new(None)));
        let store = GroceryStore::new().with_sink(sink.clone());
        buy(&store, "milk", Some("Letmælk"));
        store.add("Æg", None, Some("12")).unwrap();

        let document = sink.0.lock().unwrap().clone().unwrap();
        let restored = GroceryStore::from_json(&document).unwrap();
        assert_eq!(restored.list(), store.list());
        assert_eq!(
            restored.add("milk", None, None).unwrap().item.name,
            "Letmælk"
        );
        // Ids carry on rather than starting again and colliding.
        assert!(restored.add("Smør", None, None).unwrap().item.id > 2);
    }
}
