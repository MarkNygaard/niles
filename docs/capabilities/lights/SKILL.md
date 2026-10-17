---
name: lights
description: Lights, light, lamp, lamps, bulbs, brightness, dim, dimmer, brighter, warmer, cooler, colour, color, kelvin, WLED effects, manual mode, the daylight curve, ramp, sunset and ambient lighting.
version: 1.0.0
---
# Lights

```yaml
lights:
  on_off: "turn off the <room> lights" | "turn on the lights in the <room>" | "<room> lights off"
  whole_home: "turn off all the lights" | "all lights on" | "everything off" | "lights off everywhere"
  device: "turn on the <device>" | "turn off the <device> in the <room>"
  dim: "dim the <room> lights to <n>%" | "set the <device> to <n> percent"
  step: "<room> lights brighter" | "make the <room> dimmer"         # 10% per step
  white: "<room> lights warmer" | "make the <room> cooler"          # 200 K per step, 2000-6500 K
  named_white: "<room> lights warm white" | "cool white" | "daylight"  # 2200 / 4000 / 5500 K
  here: "lights off" | "brighter" | "warmer" | "dim the lights to 30%"  # room of the satellite
  again: "turn it back on" | "turn them off again"                  # what this room last switched
  normal: "back to normal" | "back to normal in the <room>" | "<room> back to normal"
  tools:
    set_room: every light in one room (on, brightness 0-100, color_temp_kelvin, rgb)
    set_device: one device by room/name id, e.g. kitchen/ceiling_light; wled:<id> when a name exists twice
    set_light_effect: WLED only; solid, blink, breathe, colorloop, rainbow, twinkle, sparkle, fire, candle, glitter
    list_devices_in_room / list_all_devices / explain_device_state: what exists and its state
    query_device_state_history / query_command_history: "when did the kitchen light turn off"
    get_config / update_config / reset_config / undo_config_change: retune the curve
  notes:
    - rooms are lower_snake ids (living_room); never invent a device, list them first
    - room and whole-home on/off include plugs marked as lamps, never other plugs
    - colours by name are not a fast phrase: use rgb on set_room / set_device
    - "solid" turns a WLED effect off
    - the curve sets brightness and colour temperature through the day, once a minute
    - manual mode: a spoken dim, step or white change, a device switched by name, a scene,
      a wall dimmer press, or a level, warmth, colour or effect you set with set_device,
      set_room or set_light_effect (or a rule sets) takes a light off the curve; on/off alone does not
    - a light leaves manual mode when switched off and on again, or on "back to normal"
    - ambient lights sit out the curve and the morning routine; held at lighting.ambient_* if set
    - curve settings: lighting.daytime_brightness, lighting.night_floor_brightness,
      lighting.morning_start / morning_end / sunset_start / sunset_end ("HH:MM")
    - relative curve changes ("make the evenings brighter"): get_config first, then update_config
    - the morning routine ramps lights from 0 to 100% on its days (lighting.morning_routine);
      a light set by hand during the ramp keeps its level at the end
  examples:
    - User: "make the kitchen lights blue" → set_room room=kitchen rgb=[0,0,255]
    - User: "put the fire effect on the shelf strip" → set_light_effect effect=fire
    - User: "the evenings are too dark" → get_config, then update_config lighting.<value>
    - User: "why is the bedroom light at 30%?" → explain_device_state, query_command_history
```
