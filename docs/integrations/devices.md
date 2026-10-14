# Devices

> v0.1 of the interface. Niles 0.2.0 runs it: the runtime, `niles plugin
> check` and `niles plugin dev` are built. Not yet: installing from GitHub,
> the setup panel, and pages and widgets drawn in the app.

A plugin with `devices = true` can add devices to the house. Once added, Niles
treats them like its own: they appear on their room's card in the app, voice
commands reach them, rules can trigger on them and act on them, and lights
follow the daily lighting curve.

## Naming

A device id is `room/name`, both parts lowercase letters, digits and `_`:
`living_room/floor_lamp`, `garage/door`, `system/electricity`.

- **Room** is one of the house's rooms (`kitchen`, `living_room`). The plugin
  rarely knows which, so ask: a `room` setting gives the user a drop-down of the
  house's rooms. Three room names are reserved for things that are not in a
  room: `system` (virtual devices, like an electricity price), `outdoor`
  and `none`.
- **Name** is what people call it, in snake_case: `floor_lamp` is said "floor
  lamp". Niles's model and voice commands use these words, so pick the name a
  person would say, not the vendor's model number. If the device has a name in
  its own app, normalise that (lowercase, spaces to `_`, drop other characters)
  rather than inventing one.

Inside Niles your devices' ids are prefixed with your source, `x_<plugin-id>:`
(`x_spotprice:system/electricity`), which keeps them apart from Zigbee or WLED
devices with the same `room/name`. People and the model see `room/name`; you
never write the prefix.

To move a device to another room, `remove` the old id and `upsert` the new one.

## Classes

| Class | What it is | Fields that matter | Controllable |
|---|---|---|---|
| `light` | A light, dimmable or not | `on`, `brightness`, `color_temp_kelvin`, `rgb` | usually |
| `outlet` | An on/off plug | `on` | usually |
| `switch` | A wall switch or button | none: it reports actions | no |
| `contact` | A door or window sensor | `open`, `battery_percent` | no |
| `motion` | A motion sensor | `motion`, `illuminance_lux`, `battery_percent` | no |
| `cover` | A blind, curtain or shutter | `position` | usually |
| `sensor` | Anything that only reports | any field, and [readings](#readings) | no |

The class decides how Niles treats a device: only `light`s are "the lights"
of a room and follow the curve; `outlet`s switch but never dim; `contact` and
`motion` have their own rule triggers; a `cover` is never one of the lights.
Choose by what the device *is*, not what it reports right now.

For a `light`, `LightCapabilities` says what it can be told beyond on/off and
brightness: `color_temp` (takes a colour temperature) and `rgb` (takes a
colour). Take them from what the device is, not what it last reported: a
strip in white mode is still an RGB strip.

## State fields and units

The same fields and units as everywhere in Niles:

| Field | Type | Unit and range |
|---|---|---|
| `on` | bool | |
| `brightness` | u8 | percent, 0–100 |
| `color_temp_kelvin` | u16 | kelvin, e.g. 2200 (warm) to 6500 (cool) |
| `rgb` | `{r, g, b}` | 0–255 each |
| `temperature_celsius` | f32 | °C |
| `humidity_percent` | f32 | relative humidity, 0–100 |
| `battery_percent` | u8 | 0–100 |
| `open` | bool | `true` is open |
| `motion` | bool | `true` while it sees movement |
| `illuminance_lux` | f32 | lux |
| `position` | u8 | percent, 0 shut, 100 all the way open |
| `readings` | list of `{name, value, unit}` | any other number; see [Readings](#readings) |

Convert at the edge. If your device reports brightness as 0–254, or colour
temperature in mireds (1,000,000 ÷ mireds = kelvin), or a contact sensor's
"contact: true" meaning *closed*, translate before reporting. The fields mean
the same thing for every source, which is what lets rules and the app treat
every device alike.

**Every field is optional, and `None` means "not reported", never "off".**
Niles merges a report field by field into what it knows, so report what
changed and leave the rest `None`. Reporting `{on: Some(false)}` keeps the
known brightness; it does not clear it.

## Readings

Any other number goes in `readings`: a list of `{name, value, unit}`. A price,
a power draw, CO₂, a tank level.

```rust
use niles::integration::types::Reading;

let mut state = empty_state();
state.readings = vec![
    Reading { name: "power".into(), value: 340.0, unit: Some("W".into()) },
    Reading { name: "energy".into(), value: 12.4, unit: Some("kWh".into()) },
];
devices::report_state("kitchen/dishwasher", &state).map_err(|e| format!("{e:?}"))?;
```

- **Names** are `[a-z0-9_]+`, the word a person would use: `price`, `power`,
  `co2`, `soil_moisture`. Rules and the assistant use the name ("when the
  power of the kitchen dishwasher drops below 5"), so keep it stable across
  versions.
- **Not for the fixed fields.** A temperature is `temperature_celsius`, never a
  reading called `temperature`: rules look for it in one place. The names
  `temperature`, `humidity`, `battery`, `brightness` and `illuminance` are
  taken.
- **The unit is yours**, sent with every report and passed on as given: `W`,
  `kWh`, `kr/kWh`, `ppm`, `µg/m³`. Niles never converts it, so pick one and keep
  it. `None` for a plain number, such as a rank or a count.
- **Merged by name**, like the fields: reporting `power` leaves `energy` as it
  was, and an empty list says nothing. There is no way to clear one; report a
  new value, or `set_available(false)` when the device stops answering.
- **Measured, never set.** `set_device` never carries readings, and scenes do
  not keep them.
- At most 32 per device.

Rules follow readings with the same numeric trigger and condition as a
temperature: `"measure": "price", "below": 1`, with `for` if it must hold a
while. The app shows them on the device's row (`340 W · 12.4 kWh`) and lists
them in the automation editor with their units; the assistant sees them with
the device's state. The [worked example](example-spotprice.md) reports an
electricity price this way.

## Adding and reporting

```rust
use niles::integration::devices;
use niles::integration::types::{Device, DeviceClass, LightCapabilities};

devices::upsert(&Device {
    id: "living_room/floor_lamp".into(),
    class: DeviceClass::Light,
    light: LightCapabilities { color_temp: true, rgb: false },
    controllable: true,
    state: empty_state(),
})
.map_err(|e| format!("{e:?}"))?;

let mut state = empty_state();
state.on = Some(true);
state.brightness = Some(60);
devices::report_state("living_room/floor_lamp", &state).map_err(|e| format!("{e:?}"))?;
```

- **`upsert`** adds a device or replaces its class, capabilities and
  controllability; its `state` is merged like a report. Call it in `init` for
  every device, every time: it is cheap, and it is how a changed class reaches
  Niles.
- **`report_state`** merges a change. Report when something changed, not on
  every poll if nothing did; Niles coalesces reports for one device that come
  faster than every 100 ms.
- **`report_action`** is for `switch` devices: a press, not a state.
  `single`, `double`, `hold`, `release`, or Zigbee2MQTT's names where they fit
  (`on_press`, `off_press`, `up_hold`). Actions fire rules once and are never
  replayed.
- **`set_available(id, false)`** when the device cannot be reached: it is
  greyed out in the app and rules skip it. `true` once it answers again.
  Devices start available.
- **`remove`** takes a device out of the house. Rules that name it are kept
  and shown as broken, so remove only devices that are really gone.
- **`registered()`** lists the devices Niles holds for you, including ones
  remembered from before a restart. Compare it with what you found in `init`
  and remove the leftovers.

Niles remembers your devices while your plugin is stopped, shown as
unavailable, so rules that name them stay valid through a restart or an update.

At most 256 devices per plugin.

## Controllable devices

Mark a device `controllable: true` and Niles sends it commands through your
`set_device` export: from voice ("turn the floor lamp down to 30%"), the app,
rules, scenes and, for lights, the lighting curve.

```rust
fn set_device(id: String, desired: DeviceState) -> Result<(), String> {
    let lamp = find_lamp(&id).ok_or("I don't know that lamp.")?;
    if let Some(on) = desired.on {
        lamp.switch(on)?;
    }
    if let Some(brightness) = desired.brightness {
        lamp.dim(brightness)?; // 0-100; convert to the device's own scale here
    }
    if let Some(kelvin) = desired.color_temp_kelvin {
        lamp.white(kelvin)?;
    }
    // Report what the device now says, not what was asked for.
    devices::report_state(&id, &lamp.read_state()?).map_err(|e| format!("{e:?}"))?;
    Ok(())
}
```

- `desired` holds only the fields to change. Ignore fields the device cannot
  do, rather than failing the whole command.
- **Report the result.** The app shows the change straight away and settles
  it when your report arrives; a command that is never confirmed rolls back.
  If the device confirms asynchronously, report from the timer or event that
  sees it.
- Return `Err` with a sentence when it did not work ("The pump did not
  answer."); voice reads it, the app shows it.
- **Be quick.** You have 10 s, but someone is waiting, and a light on the
  curve gets a `set_device` once a minute. Send the command and return; do not
  poll until it settles.

## Buttons

A wall switch or remote is a `switch` device that never takes commands and
reports actions:

```rust
devices::report_action("hall/remote", "double").map_err(|e| format!("{e:?}"))?;
```

The household then writes a rule on it ("when the hall remote is pressed
twice, turn everything off").
