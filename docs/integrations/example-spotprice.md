# Worked example: spot prices

> Interface 0.2, which Niles still runs: it connects no account and has no
> webhook, so 0.3 would add only empty exports (see
> [publishing](publishing.md#interface-versions) for moving it). The Rust
> below was built with `cargo build --release --target wasm32-wasip2` against
> the 0.2 WIT, now kept as
> [`niles-integration-0.2.wit`](../../schemas/niles-integration-0.2.wit), and
> `wit-bindgen` 0.63, passes `niles plugin check`, and runs under `niles plugin
> dev` on Niles 0.2.0 against the live service. The JavaScript and Python
> snippets were built with `jco componentize` and `componentize-py` against
> interface 0.1, have had the two MQTT exports of 0.2 added by hand since,
> and have not been run in Niles yet.

A complete, small plugin: Danish electricity spot prices from
[Energi Data Service](https://www.energidataservice.dk/), Energinet's free open
data API, which needs no key. It shows every part of the interface a typical
plugin uses:

- **settings**: the price area, and how many hours a day the page calls cheap;
- **HTTP** to one declared host;
- **storage**: the fetched prices, so a restart does not refetch and the tool
  answers without a network call;
- **a timer**: every five minutes, fetch when needed and update the device;
- **a device**: `system/electricity`, a sensor whose readings are the price
  now (`price`, in kr/kWh) and the hour's place among today's (`hour_rank`, 1
  the cheapest), so a rule can say "when the price drops below 1 kr, switch on
  the dishwasher plug";
- **a tool**: `spotprice_prices`, for "when is power cheapest tomorrow?";
- **a page**: Electricity in the menu, with the price now, today's prices as a
  bar chart and a **Refresh now** button.

The data: `GET https://api.energidataservice.dk/dataset/DayAheadPrices` returns
day-ahead prices in quarter hours, per price area (`DK1` west of the Great
Belt, `DK2` east), in DKK and EUR per MWh:

```json
{"records": [
  {"TimeUTC": "2026-10-13T22:00:00", "TimeDK": "2026-10-14T00:00:00",
   "PriceArea": "DK1", "DayAheadPriceEUR": 113.449997, "DayAheadPriceDKK": 848.072763}
]}
```

`start=StartOfDay&end=StartOfDay%2BP2D` asks for today and tomorrow, in Danish
time. Tomorrow's prices appear at about 13:00. These are spot prices only:
network tariffs, taxes and VAT come on top and are most of a Danish bill, which
the tool's description tells the model so it does not quote a spot price as
the cost of power.

## The repo

```
niles-spotprice/
├── Cargo.toml
├── niles-integration.toml
├── logo.svg
├── wit/niles-integration.wit      # copied from Niles's niles-integration-0.2.wit
├── src/lib.rs
└── .github/workflows/release.yml  # as in publishing.md, with CRATE: niles_spotprice
```

## The manifest

```toml
id = "spotprice"
name = "Spot prices"
version = "0.1.0"
description = "Hear what power costs hour by hour, and let rules run things when it is cheapest."
authors = ["Jane Doe <jane@example.com>"]
homepage = "https://github.com/jane/niles-spotprice"
min_niles = "0.2.0"
interface = "0.2"
# No group fits electricity, so it is left out and the plugin shows under Custom.
logo = "logo.svg"
keywords = [
    "electricity", "power", "price", "prices", "spot", "kwh", "cheap", "cheapest",
    "expensive", "dishwasher", "washing machine", "tumble dryer", "charge the car",
    "strøm", "strømmen", "elpris", "elprisen",
]

[permissions]
http = ["api.energidataservice.dk"]
devices = true
tools = true
ui = true

[[settings]]
key = "area"
label = "Price area"
type = "choice"
required = true
default = "DK1"
help = "Jutland and Funen are DK1; Zealand, Lolland-Falster and Bornholm are DK2."
options = [
    { value = "DK1", label = "West Denmark (DK1)" },
    { value = "DK2", label = "East Denmark (DK2)" },
]

[[settings]]
key = "cheap_hours"
label = "Cheap hours a day"
type = "number"
default = 4
min = 1
max = 12
help = "How many of the day's cheapest hours the Electricity page marks as cheap."

[[ui.menu]]
id = "electricity"
label = "Electricity"
icon = "zap"
refresh_every = 300
```

The household is shown:

> **Spot prices** by Jane Doe, version 0.1.0, from github.com/jane/niles-spotprice
>
> Hear what power costs hour by hour, and let rules run things when it is cheapest.
>
> It will be able to:
> - Connect to api.energidataservice.dk
> - Add devices to your home. They appear in rooms, in rules and by voice.
> - Offer 1 tool to the assistant: prices
> - Adds a page to the menu: Electricity
>
> It cannot see your conversations, the people in your house, other
> add-ons' passwords or Niles's database.

## `logo.svg`

```svg
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
  <path d="M13 2 4 14h7l-1 8 9-12h-7z" fill="#F5B400"/>
</svg>
```

## `Cargo.toml`

```toml
[package]
name = "niles-spotprice"
version = "0.1.0"
edition = "2024"
publish = false

[lib]
crate-type = ["cdylib"]

[dependencies]
wit-bindgen = "0.63"
serde = { version = "1", features = ["derive"] }
serde_json = "1"

[profile.release]
opt-level = "s"
lto = true
strip = true
```

## `src/lib.rs`

```rust
//! Danish electricity spot prices for Niles, from Energi Data Service.
//!
//! Fetches today's and tomorrow's day-ahead prices and keeps them in
//! storage. Offers them to the AI as a tool, reports the price now as a
//! reading on a `system/electricity` device that rules can follow, and
//! draws an Electricity page in the app's menu.

wit_bindgen::generate!({
    world: "plugin",
    path: "wit",
});

// The types the world's exports use (DeviceState, Event, ToolDef,
// RenderContext, UiTree, ActionValue, ActionResult, ConnectionStatus)
// are generated at the crate root; the rest are imported from their
// interfaces.
use niles::integration::types::{Device, DeviceClass, LightCapabilities, Reading};
use niles::integration::ui::{
    Badge, Button, ButtonStyle, Chart, ChartKind, ChartSeries, Node, NodeKind, TextBlock,
    Tone,
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
        self.slots.iter().filter(|s| s.local.starts_with(date)).collect()
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
        let (_, price) = hours.iter().find(|(hour, _)| *hour == current.local[11..13])?;
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
            tree.add(0, muted("No prices yet. They are fetched every five minutes."));
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
    let url = format!(
        "{API}?start=StartOfDay&end=StartOfDay%2BP2D&sort=TimeUTC%20asc&filter={filter}"
    );
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
```

## Walking through it

- **`init`** registers the device, sets the five-minute timer, and runs one
  tick so there is data straight away. It does not fail if the fetch fails:
  the plugin is working, the data is just late, and the device shows as
  unavailable until it arrives.
- **`tick`** fetches only when the stored prices no longer cover now, or
  tomorrow's are missing and the last fetch is over an hour old. That is at
  most a couple of dozen requests a day to a free public service, and none
  while nothing has changed. After a fetch it calls `ui::invalidate`, so an
  open Electricity page redraws.
- **Storage** holds the whole fetch as one JSON value, about 12 KB for two
  days, well under the 1 MiB quota. It survives restarts and updates, and it
  is what `call_tool` and `render` read, so neither touches the network.
- **The device** is a `sensor` with two [readings](devices.md#readings):
  `price` in kr/kWh, and `hour_rank`, the current hour's place among today's
  by price. The price is what a threshold rule follows; the rank is for "the
  four cheapest hours", which no fixed price picks out every day. Both are
  reported together each tick, and Niles merges them by name like any other
  state. It sits in the reserved `system` room, since it is not in any room.
- **The tool** averages quarter hours into hours (24 numbers instead of 96)
  and rounds to øre, which is what the model needs to answer "when is it
  cheapest". Its description says what it is for in people's words, and that
  the spot price is not the whole cost.
- **The page** is drawn from storage in well under the 3-second `render`
  limit. The **Refresh now** button fetches in `on_action`, within its
  10-second limit (the request's own timeout is 8 s), and asks for a redraw.
- **Time zones.** The sandbox has a clock but no time zones, so the plugin
  compares instants in UTC (`TimeUTC`, converted with a small calendar
  function) and uses the service's own Danish times (`TimeDK`) for days and
  hour labels.

Running it with `niles plugin dev .` (the area defaults to DK1), on the
evening of 14 October 2026:

```
+ device x_spotprice:system/electricity (Sensor)
~ x_spotprice:system/electricity hour_rank=18 price=1.335013797 kr/kWh
status: Working
tool spotprice_prices: Danish electricity spot prices in kr per kWh, hour by hour, ...
> tool prices {"day":"today"}
{"area":"DK1","date":"2026-10-14","hours":[{"hour":"00","kr_kwh":0.84},{"hour":"01","kr_kwh":0.88}, ...],"now":1.34,"unit":"kr per kWh, spot price before tariffs and taxes"}
> render electricity
Stack
  Row
    Reading  Now: 1.34 kr/kWh
    Badge    Not cheap (neutral)
  Card       "Today, hour by hour"
    Chart    bar, 24 points, kr/kWh
  Text       Spot price for DK1, before network tariffs and taxes. Fetched 0 minutes ago. (muted)
  Button     refresh "Refresh now" (secondary)
```

## Rules that follow it

Once installed, the household writes rules on the readings like on any
sensor, by voice ("when power drops below a krone, turn on the dishwasher
plug") or in the app, where the Automations editor offers *Price (kr/kWh)* and
*Hour rank* as readings to watch. The rule as Niles stores it:

```json
{
  "description": "Dishwasher on when power is cheap",
  "triggers": [{
    "type": "numeric", "device": "system/electricity", "measure": "price",
    "below": 1, "for": { "minutes": 15 }
  }],
  "conditions": [{ "kind": "time_of_day", "after": "22:00", "before": "06:00" }],
  "actions": [{ "do": "set_device", "device": "kitchen/dishwasher_plug", "on": true }]
}
```

Niles reads it back as: *When the price of the system electricity has been
below 1 for 15 minutes, if it is between 22:00 and 06:00, turn on the kitchen
dishwasher plug.* The `for` keeps a single cheap quarter hour from starting
it. For the day's four cheapest hours instead of a fixed price, watch
`hour_rank` below 5.

The readings also show on the device in the app (`Hour rank 3 · 0.91 kr/kWh`)
and in what the assistant is told about it ("hour rank 3, price 0.91 kr/kWh").

## The same tool in JavaScript and Python

The same `prices` tool, fetching on each call instead of from storage, to keep
the snippets short. Both build into components that target the same world;
every export must exist, so the unused ones are stubbed.

### JavaScript (`jco componentize`)

`jco` turns WIT names into camelCase: `call-tool` is `callTool`, `timeout-ms`
is `timeoutMs`. An export that returns `result` returns its `ok` value and
throws its `err`; an import that returns `result` throws on `err`, with the
error variant in `e.payload`.

```js
import { send } from "niles:integration/http@0.2.1";
import { get } from "niles:integration/config@0.2.1";

const API = "https://api.energidataservice.dk/dataset/DayAheadPrices";

export function tools() {
  return [
    {
      name: "prices",
      description:
        "Danish electricity spot prices in kr per kWh, hour by hour, for today " +
        "or tomorrow. Spot price only: network tariffs and taxes come on top.",
      schema: JSON.stringify({
        type: "object",
        properties: {
          day: { type: "string", enum: ["today", "tomorrow"] },
        },
        additionalProperties: false,
      }),
    },
  ];
}

// A returned string is the tool's `ok`; a thrown string is its `err`.
export function callTool(name, argsJson) {
  if (name !== "prices") throw `There is no tool called ${name}.`;
  const { day = "today" } = JSON.parse(argsJson);
  const area = get("area")?.val ?? "DK1";
  const start = day === "tomorrow" ? "StartOfDay%2BP1D" : "StartOfDay";
  const filter = encodeURIComponent(JSON.stringify({ PriceArea: [area] }));
  const url = `${API}?start=${start}&end=${start}%2BP1D&sort=TimeUTC%20asc&filter=${filter}`;

  let response;
  try {
    response = send({
      method: "get",
      url,
      headers: [["accept", "application/json"]],
      timeoutMs: 8000,
    });
  } catch (e) {
    // An import that returns `result` throws on `err`; the variant is e.payload.
    throw `Energi Data Service did not answer (${e.payload?.tag ?? e}).`;
  }
  if (response.status !== 200) throw `Energi Data Service answered ${response.status}.`;

  const { records } = JSON.parse(new TextDecoder().decode(response.body));
  if (records.length === 0) throw "Those prices are not out yet.";
  const hours = {};
  for (const r of records) {
    const hour = r.TimeDK.slice(11, 13);
    (hours[hour] ??= []).push(r.DayAheadPriceDKK / 1000);
  }
  return JSON.stringify({
    area,
    day,
    unit: "kr per kWh, spot price before tariffs and taxes",
    hours: Object.entries(hours).map(([hour, p]) => ({
      hour,
      kr_kwh: Math.round((p.reduce((a, b) => a + b) / p.length) * 100) / 100,
    })),
  });
}

// The other exports, empty in a plugin that only offers a tool.
export function init() {}
export function setDevice() {
  throw "Nothing here can be switched.";
}
export function onTimer() {}
export function onEvent() {}
export function onMqttMessage() {}
export function onMqttStatus() {}
export function render() {
  throw "No pages.";
}
export function onAction() {
  throw "No pages.";
}
export function stop() {}
```

```sh
npx @bytecodealliance/jco componentize plugin.js \
  --wit wit/niles-integration.wit --world-name plugin \
  --disable http --out integration.wasm
```

`--disable http` leaves out the JavaScript runtime's own `fetch`, which needs
`wasi:http/outgoing-handler`, which Niles does not provide. The result is
about 12 MB, with the JavaScript engine inside; set `memory_mb = 128`.

### Python (`componentize-py`)

`componentize-py` generates a `wit_world` package: implement the class
`WitWorld`, with snake_case methods. Return the `ok` value; raise
`componentize_py_types.Err(value)` for `err`. Imports that return `result`
raise `Err` too, with the error variant in `e.value`. A field that is a Python
keyword gets a trailing underscore: a device's `class` is `class_`.

```python
import json
from urllib.parse import quote

import wit_world
from componentize_py_types import Err
from wit_world.imports import config, http, types

API = "https://api.energidataservice.dk/dataset/DayAheadPrices"


class WitWorld(wit_world.WitWorld):
    def tools(self) -> list[types.ToolDef]:
        return [
            types.ToolDef(
                name="prices",
                description=(
                    "Danish electricity spot prices in kr per kWh, hour by hour, for today "
                    "or tomorrow. Spot price only: network tariffs and taxes come on top."
                ),
                schema=json.dumps({
                    "type": "object",
                    "properties": {"day": {"type": "string", "enum": ["today", "tomorrow"]}},
                    "additionalProperties": False,
                }),
            )
        ]

    # A returned str is the tool's `ok`; raising Err(str) is its `err`.
    def call_tool(self, name: str, args_json: str) -> str:
        if name != "prices":
            raise Err(f"There is no tool called {name}.")
        day = json.loads(args_json).get("day", "today")
        setting = config.get("area")
        area = setting.value if isinstance(setting, config.SettingValue_Text) else "DK1"
        start = "StartOfDay%2BP1D" if day == "tomorrow" else "StartOfDay"
        filter_ = quote(json.dumps({"PriceArea": [area]}))
        url = f"{API}?start={start}&end={start}%2BP1D&sort=TimeUTC%20asc&filter={filter_}"

        try:
            response = http.send(http.Request(
                method=http.Method.GET,
                url=url,
                headers=[("accept", "application/json")],
                body=None,
                timeout_ms=8000,
            ))
        except Err as e:
            # An import that returns `result` raises Err; the variant is e.value.
            raise Err(f"Energi Data Service did not answer ({type(e.value).__name__}).")
        if response.status != 200:
            raise Err(f"Energi Data Service answered {response.status}.")

        records = json.loads(response.body)["records"]
        if not records:
            raise Err("Those prices are not out yet.")
        hours: dict[str, list[float]] = {}
        for r in records:
            hours.setdefault(r["TimeDK"][11:13], []).append(r["DayAheadPriceDKK"] / 1000)
        return json.dumps({
            "area": area,
            "day": day,
            "unit": "kr per kWh, spot price before tariffs and taxes",
            "hours": [
                {"hour": h, "kr_kwh": round(sum(p) / len(p), 2)} for h, p in hours.items()
            ],
        })

    # The other exports, empty in a plugin that only offers a tool.
    def init(self) -> None:
        pass

    def set_device(self, id: str, desired: types.DeviceState) -> None:
        raise Err("Nothing here can be switched.")

    def on_timer(self, id: str) -> None:
        pass

    def on_event(self, event: types.Event) -> None:
        pass

    def on_mqtt_message(self, conn: int, topic: str, payload: bytes) -> None:
        pass

    def on_mqtt_status(self, conn: int, status) -> None:
        pass

    def render(self, surface_id, context):
        raise Err("No pages.")

    def on_action(self, surface_id, control_id, value, context):
        raise Err("No pages.")

    def stop(self) -> None:
        pass
```

```sh
pip install componentize-py
componentize-py -d wit/niles-integration.wit -w plugin componentize app -o integration.wasm
```

Pure-Python libraries are bundled from your environment; libraries with C
extensions do not build. The result is about 19 MB, with the interpreter
inside; set `memory_mb = 128`.
