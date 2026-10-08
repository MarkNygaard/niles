//! Shopping-list tools — `add_to_grocery_list`, `remove_from_grocery_list`
//! and `read_grocery_list`.
//!
//! The model is the translator, not the memory. It turns "milk" into
//! what a Danish shop calls it the first time; the catalog in
//! [`niles_groceries`] remembers what this house actually bought, and
//! overrules the translation from then on.

use crate::error::{Error, Result};
use crate::registry::ToolRegistry;
use crate::tool::{Tool, ToolDescriptor};
use async_trait::async_trait;
use niles_groceries::GroceryStore;
use serde_json::{Value, json};
use std::sync::Arc;

/// How many of the household's products the model is shown. Enough
/// for a weekly shop; the rest are still found by the store itself.
const KNOWN: usize = 40;

pub struct AddToGroceryList {
    store: Arc<GroceryStore>,
    country: Option<String>,
}

#[async_trait]
impl Tool for AddToGroceryList {
    fn descriptor(&self) -> ToolDescriptor {
        let shop = match &self.country {
            Some(code) => format!("a shop in the country with ISO code {code}"),
            None => "a local shop".to_string(),
        };
        let mut description = format!(
            "Put things on the household's shopping list: 'add milk to the list', \
             'we need eggs', 'we're out of coffee'. Pass each thing as `said`, in the \
             person's own words. When that is not what {shop} calls it, also pass \
             `product`: one of this household's products below if one fits, otherwise \
             the ordinary name {shop} would use. When confirming out loud, repeat the \
             person's own words rather than the product name — the voice cannot \
             pronounce every language."
        );
        let known = self.store.known_products(KNOWN);
        if !known.is_empty() {
            description.push_str(" Products this household buys: ");
            description.push_str(&known.join(", "));
            description.push('.');
        }
        ToolDescriptor {
            name: "add_to_grocery_list".into(),
            description,
            parameters: json!({
                "type": "object",
                "properties": {
                    "items": {
                        "type": "array",
                        "minItems": 1,
                        "items": {
                            "type": "object",
                            "properties": {
                                "said": {
                                    "type": "string",
                                    "description": "The thing, exactly in the person's words."
                                },
                                "product": {
                                    "type": "string",
                                    "description": "What to put on the list, when it differs from `said`."
                                },
                                "quantity": {
                                    "type": "string",
                                    "description": "How much, if they said: '2', '1 kg'."
                                }
                            },
                            "required": ["said"],
                            "additionalProperties": false
                        }
                    }
                },
                "required": ["items"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let items = items(&args, "add_to_grocery_list")?;
        let results: Vec<Value> = items
            .iter()
            .map(|entry| {
                let said = entry
                    .get("said")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let product = entry.get("product").and_then(Value::as_str);
                let quantity = entry.get("quantity").and_then(Value::as_str);
                match self.store.add(said, product, quantity) {
                    Ok(added) => json!({
                        "said": said,
                        "on_list_as": added.item.name,
                        "already_on_list": added.already,
                    }),
                    Err(e) => json!({ "said": said, "error": e.to_string() }),
                }
            })
            .collect();
        Ok(json!({ "items": results }))
    }
}

pub struct RemoveFromGroceryList {
    store: Arc<GroceryStore>,
}

#[async_trait]
impl Tool for RemoveFromGroceryList {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "remove_from_grocery_list".into(),
            description: "Take things off the shopping list: 'take milk off the list', \
                'we don't need eggs after all'. Pass each thing in the person's own words."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "items": {
                        "type": "array",
                        "minItems": 1,
                        "items": { "type": "string" }
                    }
                },
                "required": ["items"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let items = items(&args, "remove_from_grocery_list")?;
        let results: Vec<Value> = items
            .iter()
            .filter_map(Value::as_str)
            .map(|said| match self.store.remove_named(said) {
                Some(item) => json!({ "said": said, "removed": item.name }),
                None => json!({ "said": said, "removed": null, "reason": "not on the list" }),
            })
            .collect();
        Ok(json!({ "items": results }))
    }
}

pub struct ReadGroceryList {
    store: Arc<GroceryStore>,
}

#[async_trait]
impl Tool for ReadGroceryList {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "read_grocery_list".into(),
            description: "What is on the shopping list still to be bought.".into(),
            parameters: json!({ "type": "object", "properties": {} }),
        }
    }

    async fn execute(&self, _args: Value) -> Result<Value> {
        let to_buy: Vec<Value> = self
            .store
            .list()
            .into_iter()
            .filter(|i| i.checked_at.is_none())
            .map(|i| json!({ "name": i.name, "quantity": i.quantity, "asked_for_as": i.said }))
            .collect();
        Ok(json!({ "to_buy": to_buy }))
    }
}

fn items<'a>(args: &'a Value, tool: &str) -> Result<&'a Vec<Value>> {
    args.get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::InvalidArgs {
            tool: tool.into(),
            reason: "missing required 'items' array".into(),
        })
}

/// Register the three shopping-list tools. `country` is the household's
/// ISO 3166 code, which decides what language a product is named in.
pub fn register_grocery_tools(
    reg: &mut ToolRegistry,
    store: Arc<GroceryStore>,
    country: Option<String>,
) {
    reg.register(Box::new(AddToGroceryList {
        store: store.clone(),
        country,
    }));
    reg.register(Box::new(RemoveFromGroceryList {
        store: store.clone(),
    }));
    reg.register(Box::new(ReadGroceryList { store }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use niles_groceries::Edit;

    fn tools() -> (Arc<GroceryStore>, AddToGroceryList) {
        let store = Arc::new(GroceryStore::new());
        let add = AddToGroceryList {
            store: store.clone(),
            country: Some("DK".into()),
        };
        (store, add)
    }

    #[tokio::test]
    async fn adds_the_models_translation_and_keeps_the_words() {
        let (store, add) = tools();
        let out = add
            .execute(json!({ "items": [{ "said": "milk", "product": "Mælk" }] }))
            .await
            .unwrap();
        assert_eq!(out["items"][0]["on_list_as"], "Mælk");
        assert_eq!(store.list()[0].said.as_deref(), Some("milk"));
    }

    #[tokio::test]
    async fn names_the_households_products_once_there_are_some() {
        let (store, add) = tools();
        assert!(
            !add.descriptor()
                .description
                .contains("Products this household buys")
        );

        let item = store.add("Letmælk", None, None).unwrap().item;
        store
            .update(
                item.id,
                Edit {
                    checked: Some(true),
                    ..Edit::default()
                },
            )
            .unwrap();
        let description = add.descriptor().description;
        assert!(
            description.contains("Products this household buys: Letmælk."),
            "{description}"
        );
        assert!(description.contains("ISO code DK"), "{description}");
    }

    #[tokio::test]
    async fn a_bad_item_does_not_sink_the_rest() {
        let (_, add) = tools();
        let long = "x".repeat(200);
        let out = add
            .execute(json!({ "items": [{ "said": long }, { "said": "æg" }] }))
            .await
            .unwrap();
        assert!(out["items"][0]["error"].is_string());
        assert_eq!(out["items"][1]["on_list_as"], "Æg");
    }

    #[tokio::test]
    async fn removes_by_the_persons_words() {
        let store = Arc::new(GroceryStore::new());
        store.add("milk", Some("Mælk"), None).unwrap();
        let remove = RemoveFromGroceryList {
            store: store.clone(),
        };
        let out = remove
            .execute(json!({ "items": ["milk", "bananas"] }))
            .await
            .unwrap();
        assert_eq!(out["items"][0]["removed"], "Mælk");
        assert!(out["items"][1]["removed"].is_null());
        assert!(store.list().is_empty());
    }

    #[tokio::test]
    async fn reads_only_what_is_left_to_buy() {
        let store = Arc::new(GroceryStore::new());
        store.add("Smør", None, Some("2")).unwrap();
        let bought = store.add("Æg", None, None).unwrap().item;
        store
            .update(
                bought.id,
                Edit {
                    checked: Some(true),
                    ..Edit::default()
                },
            )
            .unwrap();
        let read = ReadGroceryList { store };
        let out = read.execute(json!({})).await.unwrap();
        assert_eq!(
            out["to_buy"],
            json!([{ "name": "Smør", "quantity": "2", "asked_for_as": null }])
        );
    }

    #[tokio::test]
    async fn refuses_a_call_without_items() {
        let (_, add) = tools();
        assert!(add.execute(json!({})).await.is_err());
    }
}
