//! A small interface 0.2.1 plugin for the tests of what 0.2.1 added.
//!
//! `init` registers `office/printer`, a sensor, and
//! `office/chamber_light`, a light it keeps out of the office's lights.
//! Tools let a test hear the house's time zone, have the printer report
//! an action, and put the light back among the room's.

wit_bindgen::generate!({
    world: "plugin",
    path: "../../../../../schemas/niles-integration.wit",
});

use niles::integration::types::{Device, DeviceClass, LightCapabilities};
use niles::integration::{config, devices};
use serde_json::Value;

struct Printer;

export!(Printer);

const PRINTER: &str = "office/printer";
const LIGHT: &str = "office/chamber_light";

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

fn device(id: &str, class: DeviceClass, controllable: bool) -> Device {
    Device {
        id: id.into(),
        class,
        light: LightCapabilities {
            color_temp: false,
            rgb: false,
        },
        controllable,
        state: empty_state(),
    }
}

fn failed(e: niles::integration::types::HostError) -> String {
    format!("{e:?}")
}

impl Guest for Printer {
    fn init() -> Result<(), String> {
        devices::upsert(&device(PRINTER, DeviceClass::Sensor, false)).map_err(failed)?;
        devices::upsert(&device(LIGHT, DeviceClass::Light, true)).map_err(failed)?;
        devices::set_room_light(LIGHT, false).map_err(failed)?;
        Ok(())
    }

    fn tools() -> Vec<ToolDef> {
        let tool = |name: &str, description: &str| ToolDef {
            name: name.into(),
            description: description.into(),
            schema: r#"{"type":"object","properties":{}}"#.into(),
        };
        vec![
            tool("time_zone", "The house's time zone."),
            tool("finish", "The printer reports that the print finished."),
            tool("room_light", "Whether the chamber light is one of the room's."),
        ]
    }

    fn call_tool(name: String, args_json: String) -> Result<String, String> {
        let args: Value = serde_json::from_str(&args_json).map_err(|e| e.to_string())?;
        match name.as_str() {
            "time_zone" => Ok(config::time_zone()),
            "finish" => devices::report_action(PRINTER, "finished")
                .map(|()| "finished".to_string())
                .map_err(failed),
            "room_light" => {
                let on = args.get("on").and_then(Value::as_bool).unwrap_or(true);
                devices::set_room_light(LIGHT, on)
                    .map(|()| on.to_string())
                    .map_err(failed)
            }
            other => Err(format!("There is no tool called {other}.")),
        }
    }

    fn set_device(id: String, desired: DeviceState) -> Result<(), String> {
        let mut state = empty_state();
        state.on = desired.on;
        devices::report_state(&id, &state).map_err(failed)
    }

    fn on_timer(_id: String) {}

    fn on_event(_event: Event) {}

    fn render(surface_id: String, _context: RenderContext) -> Result<UiTree, String> {
        Err(format!("There is nothing to draw for {surface_id}."))
    }

    fn on_action(
        _surface_id: String,
        _control_id: String,
        _value: ActionValue,
        _context: RenderContext,
    ) -> Result<ActionResult, String> {
        Err("There are no controls.".into())
    }

    fn on_mqtt_message(_conn: u32, _topic: String, _payload: Vec<u8>) {}

    fn on_mqtt_status(_conn: u32, _status: ConnectionStatus) {}

    fn stop() {}
}
