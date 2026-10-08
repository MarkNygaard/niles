use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};

/// The status nemlig gives an order once it has been delivered.
const DELIVERED: i64 = 3;

/// An order placed at nemlig.com, as much as "when is it coming" needs.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Order {
    pub id: i64,
    pub status: i64,
    /// Kroner.
    pub total: Option<f64>,
    /// The delivery window, in Danish time: nemlig delivers in Denmark.
    pub delivery_start: Option<NaiveDateTime>,
    pub delivery_end: Option<NaiveDateTime>,
}

impl Order {
    /// Not delivered yet, and its window not over.
    pub fn is_coming(&self, now: NaiveDateTime) -> bool {
        self.status != DELIVERED && self.delivery_end.is_some_and(|end| end > now)
    }
}

/// Now, the way nemlig's delivery times are written.
pub fn danish_now() -> NaiveDateTime {
    Utc::now()
        .with_timezone(&chrono_tz::Europe::Copenhagen)
        .naive_local()
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct RawOrders {
    #[serde(default)]
    orders: Vec<RawOrder>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawOrder {
    id: i64,
    #[serde(default)]
    status: Option<i64>,
    #[serde(default)]
    total: Option<f64>,
    #[serde(default)]
    delivery_time: Option<RawTime>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawTime {
    #[serde(default)]
    start: Option<String>,
    #[serde(default)]
    end: Option<String>,
}

impl RawOrders {
    pub(crate) fn into_orders(self) -> Vec<Order> {
        self.orders
            .into_iter()
            .map(|o| {
                let (start, end) = o
                    .delivery_time
                    .map(|t| (t.start, t.end))
                    .unwrap_or_default();
                Order {
                    id: o.id,
                    status: o.status.unwrap_or(0),
                    total: o.total,
                    delivery_start: start.as_deref().and_then(danish_time),
                    delivery_end: end.as_deref().and_then(danish_time),
                }
            })
            .collect()
    }
}

/// A time from nemlig, in Danish time: as written when it has no zone,
/// converted when it is UTC or carries an offset, and nothing for the
/// "0001-01-01" that stands in for "no time".
fn danish_time(value: &str) -> Option<NaiveDateTime> {
    if value.starts_with("0001-01-01") {
        return None;
    }
    if let Ok(at) = DateTime::parse_from_rfc3339(value) {
        return Some(
            at.with_timezone(&chrono_tz::Europe::Copenhagen)
                .naive_local(),
        );
    }
    let plain = value.get(..19).unwrap_or(value);
    NaiveDateTime::parse_from_str(plain, "%Y-%m-%dT%H:%M:%S").ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    fn orders(json: &str) -> Vec<Order> {
        serde_json::from_str::<RawOrders>(json)
            .unwrap()
            .into_orders()
    }

    #[test]
    fn reads_an_order_in_danish_time() {
        let o = &orders(
            r#"{"Orders": [{"Id": 123, "Status": 1, "Total": 512.5,
                "DeliveryTime": {"Start": "2026-10-10T07:00:00", "End": "2026-10-10T09:00:00"}}],
                "NumberOfPages": 1}"#,
        )[0];
        assert_eq!(o.delivery_start, Some(at("2026-10-10 07:00")));
        assert_eq!(o.delivery_end, Some(at("2026-10-10 09:00")));
        assert_eq!(o.total, Some(512.5));
    }

    #[test]
    fn a_utc_time_is_brought_to_danish_time() {
        // October is summer time: UTC+2.
        assert_eq!(
            danish_time("2026-10-10T05:00:00Z"),
            Some(at("2026-10-10 07:00"))
        );
    }

    #[test]
    fn no_time_is_nothing() {
        assert_eq!(danish_time("0001-01-01T00:00:00"), None);
    }

    #[test]
    fn an_empty_history_is_no_orders() {
        assert!(orders(r#"{"Orders": [], "NumberOfPages": 0}"#).is_empty());
    }

    #[test]
    fn coming_is_undelivered_with_its_window_ahead() {
        let order = Order {
            id: 1,
            status: 1,
            total: None,
            delivery_start: Some(at("2026-10-10 07:00")),
            delivery_end: Some(at("2026-10-10 09:00")),
        };
        assert!(order.is_coming(at("2026-10-10 08:00")));
        assert!(!order.is_coming(at("2026-10-10 09:30")));
        let delivered = Order {
            status: DELIVERED,
            ..order
        };
        assert!(!delivered.is_coming(at("2026-10-10 08:00")));
    }
}
