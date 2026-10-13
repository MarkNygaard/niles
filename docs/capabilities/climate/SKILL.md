---
name: climate
description: Climate, heating, heat, thermostat, radiators, temperature, degrees, cold, chilly, tado zones.
version: 1.0.0
---
# Climate (tado heating)

```yaml
climate:
  set: "set the heating to 21" | "set the <room> heating to 21 and a half" | "set the <room> to 21 degrees" | "heat the <room> to 20.5 degrees"
  step: "turn the heating up" | "turn down the <room> heating"            # one degree from the target
  off: "turn off the heating" | "turn the <room> heating off"
  schedule: "put the heating back on the schedule" | "resume the heating" | "turn the heating on"
  read: "what's the temperature in the <room>" | "how warm is it in here" | "what's the <room> temperature"
  everywhere: "... everywhere" | "... in the whole house"                  # every zone
  notes:
    - these are fixed phrasings answered without the LLM; no tool reads or changes the heating
    - if a heating request reaches you, tell the person a phrasing that works
    - no room said: the room of the satellite; from the app's chat there is none, so name the room
    - tado's range is 5-25 °C; anything outside is refused
    - a set temperature or off holds until the schedule is resumed; "turn the heating on" means back to the schedule
    - a zone is found by the room it was paired to in the app, or by its own name
    - "set the <room> to <n>" needs "degrees" after the number; "set the <room> heating to <n>" does not
    - a bare "what's the temperature" is the outdoor weather, not a room
    - in the app: a dial per zone in the room drawer, and a boost button that warms the whole house for a while, then stops
    - automations cannot change the heating; they can trigger on a sensor's temperature (numeric trigger)
  examples:
    - User: "set the bedroom to 19 degrees" → bedroom zone held at 19 °C
    - User: "it's freezing in here" → suggest "turn the heating up" or "set the heating to <n>"
    - User: "turn on the heating everywhere" → every zone back on its schedule
```
