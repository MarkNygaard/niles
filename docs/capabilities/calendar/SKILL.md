---
name: calendar
description: Calendar, calendars, meeting, meetings, appointment, appointments, event, events, agenda, schedule, busy, free, dentist, kalender, aftale.
version: 1.0.0
---
# Calendar

```yaml
calendar:
  read: "what's on my calendar tomorrow" | "when is my next meeting" | "am I free on Friday afternoon"
  create: "add the dentist on Thursday at ten" | "put lunch with Anna in my calendar on Monday at noon"
  change: "move the dentist to eleven" | "rename that meeting"
  delete: "delete the dentist on Thursday"
  tools:
    calendar_events: the speaker's events, all their connected calendars together; from/to as YYYY-MM-DD or YYYY-MM-DDTHH:MM local
    create_calendar_event: title, start, end?, all_day?, location?, calendar? (by name, when several can be written)
    update_calendar_event: by ref (e1, e2…) from calendar_events; only what changes; a new start keeps the length
    delete_calendar_event: by ref
    current_datetime: turn "Thursday" or "next week" into a date first
  notes:
    - only the calendars of the person speaking; nobody can ask for someone else's
    - the speaker is known by their voice; in the app's chat, by the voice their sign-in is linked to
    - an unrecognised voice has no calendar: say you don't know whose it is
    - create, update and delete: read it back and ask first ("Dentist, Thursday 15 October at 10:00, in your
      Private calendar. Shall I?"); call the tool only after a yes
    - writing needs a calendar connected with write access; if several can be written and none was said, ask which
    - a repeating event in an iCloud calendar can only be changed or deleted on the phone
    - providers: Microsoft, Google and iCloud, each connected by the person from their profile in the app
    - read titles briefly and naturally; times are local; name the calendar only when it helps
  examples:
    - User: "what do I have tomorrow?" → calendar_events from=<tomorrow> to=<day after>
    - User: "cancel my dentist appointment" → calendar_events, find its ref, ask "Delete the dentist on Thursday?",
      then delete_calendar_event on yes
```
