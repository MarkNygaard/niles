---
name: training
description: Training, running, run, runs, ran, pace, km, marathon, half marathon, workout, workouts, ride, cycling, heart rate, Strava, Garmin, how was my week.
version: 2.0.0
---
# Training

```yaml
training:
  read: "how far did I run on Sunday" | "how was my week" | "what was my pace on my last run" |
        "how many workouts did I do this month" | "what was my heart rate on that run"
  tools:
    training_summary: period week|month|quarter|year (or from/to); runs, distance, longest run, average pace,
      all workouts. Use it for "how was my week"
    training_activities: workouts, newest last; from/to as YYYY-MM-DD (to inclusive), type run|walk|ride|strength|other;
      distance, time, pace, heart rate, climb, calories
    current_datetime: turn "Sunday" or "last week" into dates first
  notes:
    - only the data of the person speaking; nobody can ask for someone else's
    - the person is known by their voice; in the app's chat, by their sign-in
    - an unrecognised voice has no training data: say you don't know whose it is
    - the workouts come from the person's own Strava; if there are none, say so and that it is
      connected on their profile page in the app, under "Strava"
    - there is no sleep, HRV, resting heart rate or weight: Strava does not have them. Say so if asked
    - numbers come from the tools; never add up distances or average paces yourself
    - say distances and paces in the house's units, as the tools give them; round for speech ("about 21 km")
    - there is no training plan yet; don't invent one or promise to follow one
  examples:
    - User: "how was my week?" → training_summary period=week
    - User: "how far did I run on Sunday?" → current_datetime, then training_activities from=<sunday> to=<sunday> type=run
    - User: "how did I sleep last night?" → say Niles only has their workouts from Strava, not their sleep
```
