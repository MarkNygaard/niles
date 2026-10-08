//! nemlig.com tools — what is in the basket, when deliveries can come,
//! what is on offer, and when the next order arrives.
//!
//! Read-only on purpose. Filling the basket and reserving a time happen
//! in the app, where the "finish at nemlig.com" button is: a delivery
//! time reserved by voice is let go after twenty minutes unless the order
//! is paid for, which nobody standing in a kitchen will do in time.

use crate::error::Result;
use crate::registry::ToolRegistry;
use crate::tool::{Tool, ToolDescriptor};
use async_trait::async_trait;
use niles_config::ConfigStore;
use niles_nemlig::{Credentials, NemligClient};
use serde_json::{Value, json};
use std::sync::Arc;

/// What every nemlig tool needs: the session, and the config that says
/// whether it is switched on and how to log in.
#[derive(Clone)]
struct Nemlig {
    client: Arc<NemligClient>,
    config: Arc<ConfigStore>,
}

impl Nemlig {
    /// The login, or the reason there is none — said to the model as a
    /// result, not raised as an error, so it can say it rather than fail.
    fn credentials(&self) -> std::result::Result<Credentials, Value> {
        let cfg = self.config.current();
        let nemlig = cfg
            .integrations
            .nemlig
            .clone()
            .filter(|n| n.enabled)
            .ok_or_else(|| json!({ "error": "nemlig.com is not switched on in Niles" }))?;
        let (username, password) = nemlig
            .resolve_credentials()
            .map_err(|e| json!({ "error": e.to_string() }))?;
        Ok(Credentials { username, password })
    }
}

fn failed(e: niles_nemlig::Error) -> Value {
    json!({ "error": e.to_string() })
}

/// "07:00".
fn clock(hour: u8) -> String {
    format!("{hour:02}:00")
}

pub struct NemligBasket(Nemlig);

#[async_trait]
impl Tool for NemligBasket {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "nemlig_basket".into(),
            description: "What is in the household's nemlig.com basket right now, what it costs, \
                and the delivery time reserved for it, if any."
                .into(),
            parameters: json!({ "type": "object", "properties": {} }),
        }
    }

    async fn execute(&self, _args: Value) -> Result<Value> {
        let credentials = match self.0.credentials() {
            Ok(c) => c,
            Err(reason) => return Ok(reason),
        };
        Ok(match self.0.client.basket(&credentials).await {
            Ok(basket) => json!({
                "lines": basket.lines.iter().map(|l| json!({
                    "name": l.name, "quantity": l.quantity, "kroner": l.total,
                })).collect::<Vec<_>>(),
                "total_kroner": basket.total,
                "delivery": basket.delivery,
                "minimum_order_kroner": basket.minimum_total,
                "meets_minimum": basket.meets_minimum,
            }),
            Err(e) => failed(e),
        })
    }
}

pub struct NemligDeliveryTimes(Nemlig);

#[async_trait]
impl Tool for NemligDeliveryTimes {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "nemlig_delivery_times".into(),
            description: "When nemlig.com can deliver in the coming week, with prices and the \
                deadline to order by. Give `date` (YYYY-MM-DD) for one day's open times; leave it \
                out for a summary of each day. Read-only: times are reserved in the Niles app."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "date": { "type": "string", "description": "YYYY-MM-DD, for one day's times." }
                },
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let credentials = match self.0.credentials() {
            Ok(c) => c,
            Err(reason) => return Ok(reason),
        };
        let days = match self.0.client.delivery_days(&credentials, 7).await {
            Ok(days) => days,
            Err(e) => return Ok(failed(e)),
        };
        let wanted = args.get("date").and_then(Value::as_str);
        if let Some(date) = wanted {
            let open: Vec<Value> = days
                .iter()
                .filter(|d| d.date == date)
                .flat_map(|d| d.slots.iter())
                .filter(|s| s.available)
                .map(|s| {
                    json!({
                        "from": clock(s.start_hour), "to": clock(s.end_hour),
                        "kroner": s.price, "order_by": s.deadline,
                    })
                })
                .collect();
            return Ok(json!({ "date": date, "open": open }));
        }
        let summary: Vec<Value> = days
            .iter()
            .map(|d| {
                let open: Vec<_> = d.slots.iter().filter(|s| s.available).collect();
                json!({
                    "date": d.date,
                    "open_times": open.len(),
                    "earliest": open.iter().map(|s| s.start_hour).min().map(clock),
                    "cheapest_kroner": open.iter().map(|s| s.price).fold(None, |m: Option<f64>, p| Some(m.map_or(p, |m| m.min(p)))),
                })
            })
            .collect();
        Ok(json!({ "days": summary }))
    }
}

pub struct NemligOffers(Nemlig);

#[async_trait]
impl Tool for NemligOffers {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "nemlig_offers".into(),
            description: "Whether something is on offer at nemlig.com: 'is coffee on offer?'. \
                Search in Danish, the way the shop names things (kaffe, mælk, rugbrød)."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "What to look for, in Danish." }
                },
                "required": ["query"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let credentials = match self.0.credentials() {
            Ok(c) => c,
            Err(reason) => return Ok(reason),
        };
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if query.is_empty() {
            return Ok(json!({ "error": "what should I look for?" }));
        }
        Ok(match self.0.client.search(&credentials, query, 12).await {
            Ok(products) => {
                let on_offer: Vec<Value> = products
                    .iter()
                    .filter(|p| p.offer.is_some() && p.available)
                    .map(|p| {
                        json!({
                            "name": p.name, "size": p.description,
                            "normal_kroner": p.price, "offer": p.offer,
                        })
                    })
                    .collect();
                json!({ "query": query, "searched": products.len(), "on_offer": on_offer })
            }
            Err(e) => failed(e),
        })
    }
}

pub struct NemligNextDelivery(Nemlig);

#[async_trait]
impl Tool for NemligNextDelivery {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "nemlig_next_delivery".into(),
            description: "When the next nemlig.com order arrives: 'when are my groceries coming?'."
                .into(),
            parameters: json!({ "type": "object", "properties": {} }),
        }
    }

    async fn execute(&self, _args: Value) -> Result<Value> {
        let credentials = match self.0.credentials() {
            Ok(c) => c,
            Err(reason) => return Ok(reason),
        };
        let now = niles_nemlig::danish_now();
        Ok(match self.0.client.next_delivery(&credentials, now).await {
            Ok(Some(order)) => json!({
                "coming": {
                    "from": order.delivery_start.map(|t| t.format("%Y-%m-%d %H:%M").to_string()),
                    "to": order.delivery_end.map(|t| t.format("%H:%M").to_string()),
                    "total_kroner": order.total,
                },
                "now": now.format("%Y-%m-%d %H:%M").to_string(),
            }),
            Ok(None) => json!({ "coming": null }),
            Err(e) => failed(e),
        })
    }
}

/// Register the four nemlig.com tools. Each checks per call whether
/// nemlig.com is switched on, so turning it on needs no restart.
pub fn register_nemlig_tools(
    reg: &mut ToolRegistry,
    client: Arc<NemligClient>,
    config: Arc<ConfigStore>,
) {
    let shared = Nemlig { client, config };
    reg.register(Box::new(NemligBasket(shared.clone())));
    reg.register(Box::new(NemligDeliveryTimes(shared.clone())));
    reg.register(Box::new(NemligOffers(shared.clone())));
    reg.register(Box::new(NemligNextDelivery(shared)));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tools() -> Vec<Box<dyn Tool>> {
        let shared = Nemlig {
            client: Arc::new(NemligClient::new().unwrap()),
            // Nothing in it: nemlig.com is not switched on.
            config: Arc::new(
                ConfigStore::from_str_in_memory("").expect("an empty config is valid"),
            ),
        };
        vec![
            Box::new(NemligBasket(shared.clone())),
            Box::new(NemligDeliveryTimes(shared.clone())),
            Box::new(NemligOffers(shared.clone())),
            Box::new(NemligNextDelivery(shared)),
        ]
    }

    #[tokio::test]
    async fn each_says_so_when_nemlig_is_off() {
        for tool in tools() {
            let out = tool.execute(json!({ "query": "kaffe" })).await.unwrap();
            assert!(
                out["error"].as_str().unwrap().contains("not switched on"),
                "{}: {out}",
                tool.descriptor().name
            );
        }
    }

    #[test]
    fn none_of_them_reserves_anything() {
        // Reserving belongs in the app, beside the button that pays.
        for tool in tools() {
            assert!(!tool.descriptor().name.contains("reserve"));
        }
    }

    #[test]
    fn hours_read_as_a_clock() {
        assert_eq!(clock(7), "07:00");
        assert_eq!(clock(17), "17:00");
    }
}
