//! Danish electricity spot prices for Niles, from Energi Data Service.
//!
//! Fetches today's and tomorrow's day-ahead prices and keeps them in
//! storage. Offers them to the AI as a tool, reports the price now as a
//! reading on a `system/electricity` device that rules can follow, and
//! draws an Electricity page in the app's menu.

wit_bindgen::generate!({
    world: "plugin",
    path: "../../schemas/niles-integration-0.2.wit",
});

// The types the world's exports use (DeviceState, Event, ToolDef,
// RenderContext, UiTree, ActionValue, ActionResult, ConnectionStatus)
// are generated at the crate root; the rest are imported from their
// interfaces.
use niles::integration::types::{Device, DeviceClass, LightCapabilities, Reading};
use niles::integration::ui::{
    Badge, Button, ButtonStyle, Chart, ChartKind, ChartSeries, Node, NodeKind, TextBlock, Tone,
};
use niles::integration::{config, devices, http, log, storage, timers, ui};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};

const API: &str = "https://api.energidataservice.dk/dataset/DayAheadPrices";
const ELECTRICITY: &str = "system/electricity";
const PAGE: &str = "electricity";
const TICK: &str = "tick";
const PRICES: &str = "prices";
/// Prices come in quarter hours.
const SLOT_SECONDS: i64 = 900;

struct SpotPrice;

export!(SpotPrice);

/// One quarter hour's price, as kept in storage.
#[derive(Serialize, Deserialize)]
struct Slot {
    /// Start, in Unix seconds.
    start: i64,
    /// Start in Danish time, e.g. "2026-10-14T13:15:00".
    local: String,
    /// Spot price in kr per kWh.
    kr_kwh: f64,
}

/// Everything fetched, under the storage key `prices`.
#[derive(Serialize, Deserialize)]
struct Stored {
    area: String,
    fetched_at: i64,
    slots: Vec<Slot>,
}

impl Stored {
    fn current(&self, now: i64) -> Option<&Slot> {
        self.slots
            .iter()
            .find(|s| s.start <= now && now < s.start + SLOT_SECONDS)
    }

    /// The Danish dates covered, in order: today, then tomorrow once it is out.
    fn dates(&self) -> Vec<&str> {
        let mut dates: Vec<&str> = self.slots.iter().map(|s| &s.local[..10]).collect();
        dates.dedup();
        dates
    }

    fn day(&self, date: &str) -> Vec<&Slot> {
        self.slots
            .iter()
            .filter(|s| s.local.starts_with(date))
            .collect()
    }

    fn today(&self, now: i64) -> Option<&str> {
        self.current(now).map(|s| &s.local[..10])
    }

    /// Where the current hour stands among today's hours by price: 1 is
    /// the cheapest. A rule on "the four cheapest hours" follows this,
    /// since no fixed price picks them out every day.
    fn hour_rank(&self, now: i64) -> Option<usize> {
        let current = self.current(now)?;
        let hours = hourly(&self.day(&current.local[..10]));
        let (_, price) = hours
            .iter()
            .find(|(hour, _)| *hour == current.local[11..13])?;
        Some(1 + hours.iter().filter(|(_, p)| p < price).count())
    }
}

#[derive(Deserialize)]
struct Answer {
    records: Vec<Record>,
}

#[derive(Deserialize)]
struct Record {
    #[serde(rename = "TimeUTC")]
    time_utc: String,
    #[serde(rename = "TimeDK")]
    time_dk: String,
    #[serde(rename = "DayAheadPriceDKK")]
    price_dkk_mwh: Option<f64>,
}

impl Guest for SpotPrice {
    fn init() -> Result<(), String> {
        devices::upsert(&Device {
            id: ELECTRICITY.into(),
            class: DeviceClass::Sensor,
            light: LightCapabilities {
                color_temp: false,
                rgb: false,
            },
            controllable: false,
            state: empty_state(),
        })
        .map_err(|e| format!("Could not add the electricity device: {e:?}"))?;
        timers::every(TICK, 300).map_err(|e| format!("Could not set the timer: {e:?}"))?;
        tick();
        Ok(())
    }

    fn tools() -> Vec<ToolDef> {
        vec![ToolDef {
            name: "prices".into(),
            description: "Danish electricity spot prices in kr per kWh, hour by hour, \
                for today or tomorrow, and the price right now. Spot price only: \
                network tariffs and taxes come on top. Use it for questions like \
                \"when is power cheapest\" or \"when should I run the dishwasher\". \
                Tomorrow's prices are out from about 13:00."
                .into(),
            schema: json!({
                "type": "object",
                "properties": {
                    "day": {
                        "type": "string",
                        "enum": ["today", "tomorrow"],
                        "description": "Which day. Today if left out."
                    }
                },
                "additionalProperties": false
            })
            .to_string(),
        }]
    }

    fn call_tool(name: String, args_json: String) -> Result<String, String> {
        #[derive(Deserialize)]
        struct Args {
            day: Option<String>,
        }

        if name != "prices" {
            return Err(format!("There is no tool called {name}."));
        }
        let args: Args =
            serde_json::from_str(&args_json).map_err(|e| format!("Bad arguments: {e}"))?;
        let stored = load().ok_or("I have no prices yet.")?;
        let now = now();
        let today = stored
            .today(now)
            .ok_or("The prices I have are out of date.")?;
        let date = match args.day.as_deref() {
            Some("tomorrow") => *stored
                .dates()
                .iter()
                .find(|d| **d > today)
                .ok_or("Tomorrow's prices are not out yet. They come at about 13:00.")?,
            _ => today,
        };
        let hours: Vec<_> = hourly(&stored.day(date))
            .into_iter()
            .map(|(hour, price)| json!({ "hour": hour, "kr_kwh": price }))
            .collect();
        Ok(json!({
            "area": stored.area,
            "date": date,
            "unit": "kr per kWh, spot price before tariffs and taxes",
            "now": stored.current(now).map(|s| round2(s.kr_kwh)),
            "hours": hours,
        })
        .to_string())
    }

    fn set_device(_id: String, _desired: DeviceState) -> Result<(), String> {
        Err("Nothing here can be switched.".into())
    }

    fn on_timer(id: String) {
        if id == TICK {
            tick();
        }
    }

    fn on_event(_event: Event) {}

    fn on_mqtt_message(_conn: u32, _topic: String, _payload: Vec<u8>) {}

    fn on_mqtt_status(_conn: u32, _status: ConnectionStatus) {}

    fn render(surface_id: String, _context: RenderContext) -> Result<UiTree, String> {
        if surface_id != PAGE {
            return Err(format!("There is no page called {surface_id}."));
        }
        let mut tree = Tree::new(NodeKind::Stack);
        let Some(stored) = load() else {
            tree.add(
                0,
                muted("No prices yet. They are fetched every five minutes."),
            );
            tree.add(0, refresh_button());
            return Ok(tree.done());
        };
        let now = now();

        let top = tree.add(0, NodeKind::Row);
        tree.add(
            top,
            NodeKind::Reading(ui::Reading {
                label: "Now".into(),
                value: stored
                    .current(now)
                    .map(|s| format!("{:.2}", s.kr_kwh))
                    .unwrap_or_else(|| "-".into()),
                unit: Some("kr/kWh".into()),
            }),
        );
        if let Some(rank) = stored.hour_rank(now) {
            let cheap = rank as f64 <= cheap_hours();
            tree.add(
                top,
                NodeKind::Badge(Badge {
                    label: if cheap { "Cheap now" } else { "Not cheap" }.into(),
                    tone: if cheap { Tone::Good } else { Tone::Neutral },
                }),
            );
        }

        if let Some(today) = stored.today(now) {
            let hours = hourly(&stored.day(today));
            let card = tree.add(0, NodeKind::Card(Some("Today, hour by hour".into())));
            tree.add(
                card,
                NodeKind::Chart(Chart {
                    kind: ChartKind::Bar,
                    labels: hours.iter().map(|(hour, _)| hour.clone()).collect(),
                    series: vec![ChartSeries {
                        name: "Spot price".into(),
                        values: hours.iter().map(|(_, price)| *price).collect(),
                    }],
                    unit: Some("kr/kWh".into()),
                }),
            );
        }

        tree.add(
            0,
            muted(&format!(
                "Spot price for {}, before network tariffs and taxes. Fetched {} minutes ago.",
                stored.area,
                (now - stored.fetched_at) / 60
            )),
        );
        tree.add(0, refresh_button());
        Ok(tree.done())
    }

    fn on_action(
        surface_id: String,
        control_id: String,
        _value: ActionValue,
        _context: RenderContext,
    ) -> Result<ActionResult, String> {
        match (surface_id.as_str(), control_id.as_str()) {
            (PAGE, "refresh") => {
                let fresh = fetch()?;
                save(&fresh);
                update_device(Some(&fresh), now());
                Ok(ActionResult {
                    toast: Some("Prices updated".into()),
                    rerender: true,
                })
            }
            _ => Err("That button does nothing.".into()),
        }
    }

    fn stop() {}
}

/// Every five minutes: fetch when the stored prices run out or tomorrow's
/// are missing, then update the electricity device.
fn tick() {
    let now = now();
    let mut stored = load().filter(|s| s.area == area());
    let stale = match &stored {
        None => true,
        Some(s) => s.current(now).is_none() || (s.dates().len() < 2 && now - s.fetched_at > 3600),
    };
    if stale {
        match fetch() {
            Ok(fresh) => {
                save(&fresh);
                stored = Some(fresh);
                ui::invalidate(PAGE);
            }
            Err(e) => warn(&e),
        }
    }
    update_device(stored.as_ref(), now);
}

/// Report the price now, and the hour's place among today's, as readings.
/// Niles keeps their units and merges them like any other state.
fn update_device(stored: Option<&Stored>, now: i64) {
    let (Some(slot), Some(rank)) = (
        stored.and_then(|s| s.current(now)),
        stored.and_then(|s| s.hour_rank(now)),
    ) else {
        let _ = devices::set_available(ELECTRICITY, false);
        return;
    };
    let _ = devices::set_available(ELECTRICITY, true);
    let mut state = empty_state();
    state.readings = vec![
        Reading {
            name: "price".into(),
            value: slot.kr_kwh,
            unit: Some("kr/kWh".into()),
        },
        Reading {
            name: "hour_rank".into(),
            value: rank as f64,
            unit: None,
        },
    ];
    if let Err(e) = devices::report_state(ELECTRICITY, &state) {
        warn(&format!("Could not report the price: {e:?}"));
    }
}

/// Today and tomorrow, in Danish time, for the configured area.
fn fetch() -> Result<Stored, String> {
    let area = area();
    let filter = format!("%7B%22PriceArea%22:%5B%22{area}%22%5D%7D");
    let url =
        format!("{API}?start=StartOfDay&end=StartOfDay%2BP2D&sort=TimeUTC%20asc&filter={filter}");
    let response = http::send(&http::Request {
        method: http::Method::Get,
        url,
        headers: vec![("accept".into(), "application/json".into())],
        body: None,
        timeout_ms: Some(8_000),
    })
    .map_err(|e| format!("Energi Data Service did not answer: {e:?}"))?;
    if response.status != 200 {
        return Err(format!("Energi Data Service answered {}.", response.status));
    }
    let answer: Answer = serde_json::from_slice(&response.body)
        .map_err(|e| format!("Unexpected answer from Energi Data Service: {e}"))?;
    let slots = answer
        .records
        .into_iter()
        .filter_map(|r| {
            Some(Slot {
                start: unix_seconds(&r.time_utc)?,
                kr_kwh: r.price_dkk_mwh? / 1000.0,
                local: r.time_dk,
            })
        })
        .collect();
    Ok(Stored {
        area,
        fetched_at: now(),
        slots,
    })
}

fn load() -> Option<Stored> {
    serde_json::from_str(&storage::get(PRICES)?).ok()
}

fn save(stored: &Stored) {
    let text = serde_json::to_string(stored).expect("prices serialise");
    if let Err(e) = storage::set(PRICES, &text) {
        warn(&format!("Could not store the prices: {e:?}"));
    }
}

fn area() -> String {
    match config::get("area") {
        Some(config::SettingValue::Text(area)) => area,
        _ => "DK1".into(),
    }
}

fn cheap_hours() -> f64 {
    match config::get("cheap_hours") {
        Some(config::SettingValue::Number(hours)) => hours.clamp(1.0, 12.0),
        _ => 4.0,
    }
}

/// The average price of each Danish hour, as ("13", 1.23).
fn hourly(slots: &[&Slot]) -> Vec<(String, f64)> {
    let mut hours: Vec<(String, f64, u32)> = Vec::new();
    for slot in slots {
        let hour = &slot.local[11..13];
        match hours.last_mut() {
            Some((h, sum, n)) if h == hour => {
                *sum += slot.kr_kwh;
                *n += 1;
            }
            _ => hours.push((hour.to_string(), slot.kr_kwh, 1)),
        }
    }
    hours
        .into_iter()
        .map(|(hour, sum, n)| (hour, round2(sum / f64::from(n))))
        .collect()
}

/// "2026-10-14T13:15:00", in UTC, as Unix seconds.
fn unix_seconds(s: &str) -> Option<i64> {
    let field = |from: usize, to: usize| s.get(from..to)?.parse::<i64>().ok();
    let (year, month, day) = (field(0, 4)?, field(5, 7)?, field(8, 10)?);
    let (hour, minute) = (field(11, 13)?, field(14, 16)?);
    // Howard Hinnant's days_from_civil: there is no calendar in the sandbox.
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    Some(days * 86_400 + hour * 3_600 + minute * 60)
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn warn(message: &str) {
    log::log(log::Level::Warn, message);
}

/// wit-bindgen does not derive `Default` for records.
fn empty_state() -> DeviceState {
    DeviceState {
        on: None,
        brightness: None,
        color_temp_kelvin: None,
        rgb: None,
        temperature_celsius: None,
        humidity_percent: None,
        battery_percent: None,
        open: None,
        motion: None,
        illuminance_lux: None,
        position: None,
        readings: Vec::new(),
    }
}

fn muted(text: &str) -> NodeKind {
    NodeKind::Text(TextBlock {
        value: text.into(),
        muted: true,
    })
}

fn refresh_button() -> NodeKind {
    NodeKind::Button(Button {
        id: "refresh".into(),
        label: "Refresh now".into(),
        style: ButtonStyle::Secondary,
    })
}

/// Builds the flat node list a `ui-tree` is. Node 0 is the root, and a
/// child is always added after its parent, as the interface requires.
struct Tree(Vec<Node>);

impl Tree {
    fn new(root: NodeKind) -> Self {
        Tree(vec![Node {
            kind: root,
            children: Vec::new(),
        }])
    }

    /// Add `kind` as the last child of `parent`; returns its index.
    fn add(&mut self, parent: u32, kind: NodeKind) -> u32 {
        let index = self.0.len() as u32;
        self.0.push(Node {
            kind,
            children: Vec::new(),
        });
        self.0[parent as usize].children.push(index);
        index
    }

    fn done(self) -> UiTree {
        UiTree { nodes: self.0 }
    }
}
