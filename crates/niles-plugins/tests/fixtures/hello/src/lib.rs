//! A small plugin for the runtime's integration tests.
//!
//! Each export does one thing a test can see from outside: a device with
//! a reading, a tool per host interface, a timer that counts, a page with
//! a button. Some tools misbehave on purpose (`spin`, `panic`) so the
//! tests can watch Niles throw the instance away and start a new one.

wit_bindgen::generate!({
    world: "plugin",
    path: "../../../../../schemas/niles-integration.wit",
});

use niles::integration::types::{Device, DeviceClass, EventKind, LightCapabilities, Reading};
use niles::integration::ui::{Badge, Button, ButtonStyle, Node, NodeKind, Tone};
use niles::integration::{config, devices, events, http, log, notify, storage, timers, ui};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

struct Hello;

export!(Hello);

static TICKS: AtomicU32 = AtomicU32::new(0);
static PRESSES: AtomicU32 = AtomicU32::new(0);
static LAST_EVENT: Mutex<String> = Mutex::new(String::new());

fn greeting() -> String {
    match config::get("greeting") {
        Some(config::SettingValue::Text(text)) => text,
        _ => "Hello".into(),
    }
}

fn room() -> String {
    match config::get("room") {
        Some(config::SettingValue::Text(room)) => room,
        _ => "office".into(),
    }
}

fn lamp() -> String {
    format!("{}/lamp", room())
}

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

fn device(id: String, class: DeviceClass, controllable: bool) -> Device {
    Device {
        id,
        class,
        light: LightCapabilities {
            color_temp: true,
            rgb: false,
        },
        controllable,
        state: empty_state(),
    }
}

fn arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

impl Guest for Hello {
    fn init() -> Result<(), String> {
        let greeting = greeting();
        if greeting == "fail" {
            return Err("The greeting is fail.".into());
        }
        log::log(log::Level::Info, &format!("starting with {greeting:?}"));
        println!("printed to stdout");

        devices::upsert(&device(lamp(), DeviceClass::Light, true)).map_err(|e| format!("{e:?}"))?;
        devices::upsert(&device(format!("{}/button", room()), DeviceClass::Switch, false))
            .map_err(|e| format!("{e:?}"))?;
        let mut meter = device("system/meter".into(), DeviceClass::Sensor, false);
        meter.state.readings = vec![Reading {
            name: "power".into(),
            value: 340.0,
            unit: Some("W".into()),
        }];
        devices::upsert(&meter).map_err(|e| format!("{e:?}"))?;

        timers::every("tick", 5).map_err(|e| format!("{e:?}"))?;
        events::subscribe(EventKind::DeviceState, &["*/door".to_string()])
            .map_err(|e| format!("{e:?}"))?;
        events::subscribe(EventKind::Presence, &[]).map_err(|e| format!("{e:?}"))?;
        Ok(())
    }

    fn tools() -> Vec<ToolDef> {
        let tool = |name: &str, description: &str| ToolDef {
            name: name.into(),
            description: description.into(),
            schema: json!({ "type": "object", "properties": {} }).to_string(),
        };
        vec![
            tool("greet", "Say hello to someone. Use for 'say hello to Mark'."),
            tool("fetch", "Fetch a URL and say what status came back."),
            tool("spin", "Never returns."),
            tool("panic", "Panics."),
            tool("remember", "Store a value."),
            tool("recall", "Read a stored value."),
            tool("keys", "List stored keys."),
            tool("say", "Speak through the satellites."),
            tool("ticks", "How many times the timer fired."),
            tool("timer", "Set a one-off timer."),
            tool("press", "Press the button."),
            tool("reading", "Report a new power reading."),
            tool("last_event", "The last event received."),
            tool("invalidate", "Ask open pages to redraw."),
            tool("secret", "Whether the token is set."),
            tool("registered", "The devices Niles holds for this plugin."),
        ]
    }

    fn call_tool(name: String, args_json: String) -> Result<String, String> {
        let args: Value = serde_json::from_str(&args_json).map_err(|e| e.to_string())?;
        match name.as_str() {
            "greet" => Ok(format!(
                "{}, {}!",
                greeting(),
                arg(&args, "name").unwrap_or("world")
            )),
            "fetch" => {
                let url = arg(&args, "url").ok_or("Which URL?")?;
                http::send(&http::Request {
                    method: http::Method::Get,
                    url: url.into(),
                    headers: Vec::new(),
                    body: None,
                    timeout_ms: Some(2_000),
                })
                .map(|r| format!("status {}", r.status))
                .map_err(|e| format!("{e:?}"))
            }
            "spin" => {
                // black_box, or the compiler works out where the loop
                // ends and skips it.
                let mut n: u64 = 0;
                loop {
                    n = std::hint::black_box(n.wrapping_add(1));
                }
            }
            "panic" => panic!("on purpose"),
            "remember" => {
                let key = arg(&args, "key").ok_or("Which key?")?;
                let value = match args.get("size").and_then(Value::as_u64) {
                    Some(size) => "x".repeat(size as usize),
                    None => arg(&args, "value").unwrap_or_default().to_string(),
                };
                storage::set(key, &value)
                    .map(|()| "stored".to_string())
                    .map_err(|e| format!("{e:?}"))
            }
            "recall" => {
                let key = arg(&args, "key").ok_or("Which key?")?;
                Ok(storage::get(key).unwrap_or_else(|| "nothing".into()))
            }
            "keys" => Ok(json!(storage::list_keys(arg(&args, "prefix").unwrap_or(""))).to_string()),
            "say" => {
                let priority = if args.get("urgent").and_then(Value::as_bool) == Some(true) {
                    notify::Priority::Urgent
                } else {
                    notify::Priority::Normal
                };
                notify::say(arg(&args, "message").unwrap_or("Hello."), priority, None)
                    .map(|()| "said".to_string())
                    .map_err(|e| format!("{e:?}"))
            }
            "ticks" => Ok(TICKS.load(Ordering::SeqCst).to_string()),
            "timer" => {
                let seconds = args.get("seconds").and_then(Value::as_u64).unwrap_or(1) as u32;
                let every = args.get("every").and_then(Value::as_bool) == Some(true);
                let set = if every {
                    timers::every("tick", seconds)
                } else {
                    timers::after("once", seconds)
                };
                set.map(|()| "set".to_string())
                    .map_err(|e| format!("{e:?}"))
            }
            "press" => devices::report_action(&format!("{}/button", room()), "single")
                .map(|()| "pressed".to_string())
                .map_err(|e| format!("{e:?}")),
            "reading" => {
                let mut state = empty_state();
                state.readings = vec![Reading {
                    name: "power".into(),
                    value: args.get("watts").and_then(Value::as_f64).unwrap_or(5.0),
                    unit: Some("W".into()),
                }];
                devices::report_state("system/meter", &state)
                    .map(|()| "reported".to_string())
                    .map_err(|e| format!("{e:?}"))
            }
            "last_event" => Ok(LAST_EVENT.lock().unwrap().clone()),
            "invalidate" => {
                ui::invalidate("page");
                Ok("invalidated".into())
            }
            "secret" => Ok(config::secret("token").is_some().to_string()),
            "registered" => Ok(json!(devices::registered()).to_string()),
            other => Err(format!("There is no tool called {other}.")),
        }
    }

    fn set_device(id: String, desired: DeviceState) -> Result<(), String> {
        if id != lamp() {
            return Err("I don't know that device.".into());
        }
        devices::report_state(&id, &desired).map_err(|e| format!("{e:?}"))
    }

    fn on_timer(id: String) {
        if id == "tick" || id == "once" {
            let n = TICKS.fetch_add(1, Ordering::SeqCst) + 1;
            let mut state = empty_state();
            state.readings = vec![Reading {
                name: "ticks".into(),
                value: f64::from(n),
                unit: None,
            }];
            let _ = devices::report_state("system/meter", &state);
        }
    }

    fn on_event(event: Event) {
        let text = match event {
            Event::DeviceState(change) => format!("state {} open={:?}", change.id, change.state.open),
            Event::DeviceAction(press) => format!("action {} {}", press.id, press.action),
            Event::Presence(state) => format!("presence {state:?}"),
        };
        *LAST_EVENT.lock().unwrap() = text;
    }

    fn render(surface_id: String, context: RenderContext) -> Result<UiTree, String> {
        let node = |kind: NodeKind, children: Vec<u32>| Node { kind, children };
        match surface_id.as_str() {
            "page" => Ok(UiTree {
                nodes: vec![
                    node(NodeKind::Stack, vec![1, 2, 3, 4]),
                    node(
                        NodeKind::Reading(ui::Reading {
                            label: "Greeting".into(),
                            value: greeting(),
                            unit: None,
                        }),
                        Vec::new(),
                    ),
                    node(
                        NodeKind::Text(ui::TextBlock {
                            value: format!(
                                "Pressed {} times, for {}.",
                                PRESSES.load(Ordering::SeqCst),
                                context.person.unwrap_or_else(|| "nobody".into())
                            ),
                            muted: true,
                        }),
                        Vec::new(),
                    ),
                    node(
                        NodeKind::Button(Button {
                            id: "hello".into(),
                            label: "Say hello".into(),
                            style: ButtonStyle::Primary,
                        }),
                        Vec::new(),
                    ),
                    node(NodeKind::Card(Some("Inside".into())), vec![5]),
                    node(
                        NodeKind::Badge(Badge {
                            label: "Fine".into(),
                            tone: Tone::Good,
                        }),
                        Vec::new(),
                    ),
                ],
            }),
            "card" => Ok(UiTree {
                nodes: vec![node(
                    NodeKind::Heading(format!(
                        "{} ticks in {}",
                        TICKS.load(Ordering::SeqCst),
                        context.room.unwrap_or_default()
                    )),
                    Vec::new(),
                )],
            }),
            // A node that is its own child: Niles must refuse it.
            "broken" => Ok(UiTree {
                nodes: vec![node(NodeKind::Stack, vec![0])],
            }),
            other => Err(format!("There is no page called {other}.")),
        }
    }

    fn on_action(
        surface_id: String,
        control_id: String,
        value: ActionValue,
        _context: RenderContext,
    ) -> Result<ActionResult, String> {
        match (surface_id.as_str(), control_id.as_str(), value) {
            ("page", "hello", ActionValue::Pressed) => {
                PRESSES.fetch_add(1, Ordering::SeqCst);
                Ok(ActionResult {
                    toast: Some("Hello pressed".into()),
                    rerender: true,
                })
            }
            _ => Err("That button does nothing.".into()),
        }
    }

    fn stop() {
        log::log(log::Level::Info, "stopping");
    }
}
