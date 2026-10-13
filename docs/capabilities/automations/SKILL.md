---
name: automations
description: Automations, automation, rule, rules, whenever, trigger, triggers, condition, conditions, remind, reminder.
version: 1.0.0
---
# Automations

```yaml
automations:
  make: "when the <room> door opens, turn off the <room> lights" | "turn on the porch light at sunset"
        | "turn the lights off five minutes after the door closes" | "tell me if the garden door has been open for ten minutes"
  list: "what automations do I have"
  pause: "pause the <name> automation" | "switch the <name> automation back on"
  remove: "delete the <name> automation"
  why: "why didn't the office light turn off?" | "did the garden door rule run?"
  tools:
    propose_automation: first step; saves nothing; returns the rule as a sentence, problems, similar rules
    create_automation: save it, only after the person said yes to the read-back
    list_automations: every rule as a sentence, with id and paused; find ids here
    pause_automation: id + paused=true to pause, false to resume
    remove_automation: delete for good; read its sentence back and ask first
    automation_runs: the last runs of one rule since Niles started; trigger, outcome, reason
  triggers: contact (door/window open|closed), device_state (on|off), numeric (temperature|humidity|battery|brightness
            above|below), device_action (button press), presence (home|away), time (HH:MM, days), sun (sunrise|sunset
            + offset_minutes), timer_fired; contact/device_state/numeric/presence take `for` ("open for 10 minutes")
  conditions: time_of_day, sun, device_is, contact_is, numeric, presence_is, triggered_by; all must hold
  actions: set_room (on, brightness, kelvin), set_device, notify (spoken in a room with a satellite), delay, if/then/else (3 deep)
  notes:
    - a sentence with when / whenever / if / every time / as soon as is never acted on now; it is a rule to make
    - always propose_automation, read the sentence back, ask "Shall I?" — never say saved before a yes
    - a plain yes within 3 minutes of the read-back is saved by Niles itself; a no drops it
    - if propose returns problems (unknown device or room, a rule that could never run), say them and ask
    - "30 minutes after sunset" is ONE sun trigger with offset_minutes 30, not a sunset trigger plus a condition
    - a rule fires every time its trigger does; there is no one-off reminder
    - a new trigger while a run is still going (in a delay) replaces that run
    - rules can also be made and edited on the Automations page in the app, with several triggers and branches
  examples:
    - User: "when the office door opens, turn off the office lights" → propose_automation → "When the office door opens,
      I'll turn off the office lights. Shall I?" → User: "yes" → saved
    - User: "remind me to close the curtains 30 minutes after sunset" → sun trigger sunset offset 30, notify in the room
    - User: "stop the garden door rule for now" → list_automations, pause_automation paused=true
```
