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
    propose_automation: first step; saves nothing; returns the rule as a sentence, rule_in_full (every part as it will
                        run), problems, warnings, similar rules
    create_automation: save it, only after the person said yes to the read-back; returns rule_in_full and warnings
    list_automations: every rule as a sentence, with id and paused; find ids here
    pause_automation: id + paused=true to pause, false to resume
    remove_automation: delete for good; read its sentence back and ask first
    automation_runs: the last runs of one rule since Niles started; trigger, outcome, reason
  triggers: contact (door/window open|closed), motion (detected true|false), device_state (on|off), numeric
            (temperature|humidity|battery|brightness above|below), device_action (button press), presence (home|away),
            time (HH:MM, days), sun (sunrise|sunset + offset_minutes), timer_fired; contact/motion/device_state/numeric/
            presence take `for` ("open for 10 minutes", "no motion for 2 minutes")
  motion groups: a motion trigger or motion_is takes `devices` (several sensors) and/or `rooms` (every motion sensor in
            them) in place of `device`; detected=false holds once NONE of them sees anybody, true while ANY does; with
            `for`, the wait starts when the last one clears and motion in any starts it over
  conditions: time_of_day, sun, device_is, contact_is, motion_is, numeric, presence_is, triggered_by; all must hold
  actions: set_room (on, brightness, kelvin), set_device, notify (spoken in a room with a satellite), delay, if/then/else (3 deep)
  notes:
    - a sentence with when / whenever / if / every time / as soon as is never acted on now; it is a rule to make
    - always propose_automation, read the sentence back, ask "Shall I?" — never say saved before a yes
    - a plain yes within 3 minutes of the read-back is saved by Niles itself; a no drops it
    - if propose returns problems (unknown device or room, a rule that could never run), say them and ask
    - if propose returns warnings, the rule will not do what it reads like; fix it unless that is meant
    - say what the rule does only as rule_in_full has it, never your own paraphrase of it
    - ask before proposing when a request reads two ways: "not if there is motion in the kitchen" is either "don't turn
      it ON while the kitchen has somebody" or "don't turn it OFF until the kitchen is clear too"; "no motion for 2
      minutes" is not "on for at least 2 minutes"
    - lights shared by rooms ("keep both on while I move between them", "off when both are clear"): ONE motion trigger
      over all the rooms' sensors, detected=false with `for`, for the off; the on triggers stay one per room
    - never "off only if the other room is clear" as a condition on a `for` trigger or after a delay: it is checked
      once, when the wait ends, and if it fails nothing tries again
    - "30 minutes after sunset" is ONE sun trigger with offset_minutes 30, not a sunset trigger plus a condition
    - a rule fires every time its trigger does; there is no one-off reminder
    - a new trigger while a run is still going (in a delay) replaces that run
    - rules can also be made and edited on the Automations page in the app, with several triggers and branches
  examples:
    - User: "when the office door opens, turn off the office lights" → propose_automation → "When the office door opens,
      I'll turn off the office lights. Shall I?" → User: "yes" → saved
    - User: "remind me to close the curtains 30 minutes after sunset" → sun trigger sunset offset 30, notify in the room
    - User: "stop the garden door rule for now" → list_automations, pause_automation paused=true
    - User: "hallway motion turns on the hallway light, kitchen motion the kitchen light, but not if there is motion in
      the kitchen" → ask: "Should the hallway light stay off while somebody is in the kitchen, or stay on until both
      rooms are clear?" → User: "kitchen motion turns on the hallway light too; both off once both rooms have been clear
      for 2 minutes" → triggers hall-seen (motion hallway/motion detected=true), kitchen-seen (motion kitchen/motion
      detected=true), all-clear (motion devices [hallway/motion, kitchen/motion] detected=false for 2 minutes); actions:
      if triggered_by hall-seen → set_room hallway on; if kitchen-seen → set_room kitchen on, set_room hallway on; if
      all-clear → set_room kitchen off, set_room hallway off. No motion_is conditions.
```
