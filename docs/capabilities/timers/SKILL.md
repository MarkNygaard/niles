---
name: timers
description: Timers, timer, countdown, alarm, alarms, ringing, chime.
version: 1.0.0
---
# Timers

```yaml
timers:
  set: "set a timer for <n> minutes" | "timer for <n> seconds" | "<n> minute timer" | "<n> hour timer called <name>"
  cancel: "cancel the <name> timer" | "stop the <name> timer"
  stop: "stop" | "stop the timer" | "stop the alarm" | "cancel" | "cancel the timer"
  list: "list my timers" | "what timers do I have"
  remaining: "how long is left" | "how much time is left on the timer" | "how much longer"
  tools:
    list_timers: every active timer with id, name, duration, state, remaining seconds
    cancel_timer: by name; cancels every timer sharing that name; 0 means no match
    get_timer_remaining: by name; the soonest when several share it
  notes:
    - setting a timer is a fixed phrasing answered without the LLM; no tool sets one
    - if a timer request reaches you, give the person the phrasing: "set a timer for 5 minutes"
    - units: seconds, minutes, hours (sec, min, hr); a whole number, no "and a half"
    - a name is optional and comes after "called"
    - a finished timer rings with a chime in the room it was set from until stopped; it gives up after 10 minutes
    - "stop" stops a ringing timer first; with none ringing it cancels the soonest timer still counting down;
      with no timer at all it stops the music
    - "how long is left" without a name answers for the soonest timer
    - timers are not reminders at a clock time; no tool takes "at 17:00"
    - an automation can start on a timer finishing (trigger type=timer_fired)
  examples:
    - User: "10 minute timer called pasta" → pasta timer, 10 minutes
    - User: "how long on the pasta timer?" → get_timer_remaining name=pasta
    - User: "cancel all my timers" → list_timers, then cancel_timer per name
```
