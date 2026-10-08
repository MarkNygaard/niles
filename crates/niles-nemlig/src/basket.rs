use serde::{Deserialize, Serialize};

/// The account's basket at nemlig.com, as much as Niles shows of it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Basket {
    pub lines: Vec<BasketLine>,
    /// Kroner, everything included: products, delivery, bags, deposits.
    pub total: f64,
    pub delivery_price: f64,
    /// "Torsdag 9. oktober kl. 7-9", when a time is reserved.
    pub delivery: Option<String>,
    /// The reserved slot, when there is one.
    pub slot_id: Option<i64>,
    /// nemlig's smallest order, and whether this basket is there yet.
    pub minimum_total: Option<f64>,
    pub meets_minimum: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BasketLine {
    pub product_id: String,
    pub name: String,
    pub quantity: u32,
    /// Kroner for the line, not each.
    pub total: f64,
}

/// One day nemlig.com delivers, and when.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DeliveryDay {
    /// "2026-10-09".
    pub date: String,
    pub slots: Vec<DeliverySlot>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DeliverySlot {
    pub id: i64,
    pub start_hour: u8,
    pub end_hour: u8,
    pub price: f64,
    /// Open to order: not past its deadline, not sold out.
    pub available: bool,
    /// The one this basket has reserved.
    pub selected: bool,
    /// When ordering for it closes, nemlig's local time: "2026-10-08T14:00:00".
    pub deadline: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct RawBasket {
    #[serde(default)]
    lines: Vec<RawLine>,
    #[serde(default)]
    total_price: f64,
    #[serde(default)]
    delivery_price: f64,
    #[serde(default)]
    formatted_delivery_time: Option<String>,
    #[serde(default)]
    delivery_time_slot: Option<RawReserved>,
    #[serde(default)]
    minimum_order_total: Option<f64>,
    #[serde(default = "yes")]
    is_min_total_valid: bool,
    /// What search prices against; it follows the reserved slot.
    #[serde(default)]
    pub(crate) timeslot_utc: Option<String>,
    #[serde(default)]
    pub(crate) delivery_zone_id: Option<i64>,
}

fn yes() -> bool {
    true
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawLine {
    id: serde_json::Value,
    #[serde(default)]
    name: String,
    #[serde(default)]
    quantity: u32,
    #[serde(default)]
    price: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawReserved {
    /// A string here, though the same slot is a number in the list of
    /// delivery days.
    #[serde(default)]
    id: Option<serde_json::Value>,
}

impl From<RawBasket> for Basket {
    fn from(b: RawBasket) -> Self {
        Self {
            lines: b
                .lines
                .into_iter()
                .map(|l| BasketLine {
                    // A number in one answer and a string in another.
                    product_id: match l.id {
                        serde_json::Value::String(s) => s,
                        other => other.to_string(),
                    },
                    name: l.name,
                    quantity: l.quantity,
                    total: l.price,
                })
                .collect(),
            total: b.total_price,
            delivery_price: b.delivery_price,
            delivery: b.formatted_delivery_time.filter(|t| !t.trim().is_empty()),
            slot_id: b
                .delivery_time_slot
                .and_then(|s| s.id)
                .and_then(|id| match id {
                    serde_json::Value::String(s) => s.parse().ok(),
                    other => other.as_i64(),
                })
                .filter(|id| *id > 0),
            minimum_total: b.minimum_order_total.filter(|m| *m > 0.0),
            meets_minimum: b.is_min_total_valid,
        }
    }
}

impl Basket {
    /// How many of a product are in the basket already.
    pub fn quantity_of(&self, product_id: &str) -> u32 {
        self.lines
            .iter()
            .filter(|l| l.product_id == product_id)
            .map(|l| l.quantity)
            .sum()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct RawDays {
    #[serde(default)]
    day_range_hours: Vec<RawDay>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawDay {
    date: String,
    #[serde(default)]
    day_hours: Vec<RawSlot>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawSlot {
    id: i64,
    start_hour: u8,
    end_hour: u8,
    #[serde(default)]
    delivery_price: Option<f64>,
    /// 0 open, 1 past its deadline, 2 sold out, 3 not active.
    #[serde(default)]
    availability: i64,
    #[serde(default)]
    is_selected: bool,
    #[serde(default)]
    deadline: Option<String>,
}

impl RawDays {
    pub(crate) fn into_days(self) -> Vec<DeliveryDay> {
        self.day_range_hours
            .into_iter()
            .map(|d| DeliveryDay {
                date: d.date.chars().take(10).collect(),
                slots: d
                    .day_hours
                    .into_iter()
                    .map(|s| DeliverySlot {
                        id: s.id,
                        start_hour: s.start_hour,
                        end_hour: s.end_hour,
                        price: s.delivery_price.unwrap_or(0.0),
                        available: s.availability == 0,
                        selected: s.is_selected,
                        deadline: s.deadline,
                    })
                    .collect(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_basket() {
        let raw: RawBasket = serde_json::from_str(
            r#"{"Lines": [
                    {"Id": 701012, "Name": "Letmælk 1,5% øko.", "Quantity": 2, "Price": 27.9},
                    {"Id": "5060220", "Name": "Surdejsrundstykker", "Quantity": 1, "Price": 14.95}
                ],
                "TotalPrice": 88.85, "DeliveryPrice": 46.0,
                "FormattedDeliveryTime": "Torsdag 9. oktober kl. 7-9",
                "DeliveryTimeSlot": {"Id": "2409254", "Reserved": true},
                "MinimumOrderTotal": 300.0, "IsMinTotalValid": false,
                "TimeslotUtc": "2026100905-120-1020", "DeliveryZoneId": 4}"#,
        )
        .unwrap();
        assert_eq!(raw.timeslot_utc.as_deref(), Some("2026100905-120-1020"));
        let basket = Basket::from(raw);
        assert_eq!(basket.lines.len(), 2);
        assert_eq!(basket.quantity_of("701012"), 2);
        assert_eq!(basket.quantity_of("5060220"), 1);
        assert_eq!(basket.quantity_of("1"), 0);
        assert_eq!(basket.slot_id, Some(2409254));
        assert_eq!(
            basket.delivery.as_deref(),
            Some("Torsdag 9. oktober kl. 7-9")
        );
        assert!(!basket.meets_minimum);
    }

    #[test]
    fn an_empty_basket_has_no_slot() {
        let raw: RawBasket =
            serde_json::from_str(r#"{"Lines": [], "DeliveryTimeSlot": {"Id": 0}}"#).unwrap();
        let basket = Basket::from(raw);
        assert_eq!(basket.slot_id, None);
        assert!(basket.meets_minimum);
    }

    #[test]
    fn reads_delivery_days() {
        let raw: RawDays = serde_json::from_str(
            r#"{"DayRangeHours": [{"Date": "2026-10-09T00:00:00", "DayHours": [
                {"Id": 2408315, "StartHour": 7, "EndHour": 8, "DeliveryPrice": 59.0,
                 "Deadline": "2026-10-08T14:00:00", "Availability": 0, "IsSelected": false},
                {"Id": 2409254, "StartHour": 7, "EndHour": 9, "DeliveryPrice": 46.0,
                 "Deadline": "2026-10-08T14:00:00", "Availability": 2, "IsSelected": true}
            ]}]}"#,
        )
        .unwrap();
        let days = raw.into_days();
        assert_eq!(days[0].date, "2026-10-09");
        assert_eq!(days[0].slots[0].price, 59.0);
        assert!(days[0].slots[0].available);
        assert!(!days[0].slots[1].available, "sold out");
        assert!(days[0].slots[1].selected);
    }
}
