---
name: training
description: Training, running, run, runs, ran, pace, km, marathon, half marathon, workout, workouts, sleep, slept, HRV, resting heart rate, weight, body fat, Garmin, how was my week.
version: 1.0.0
---
# Training

```yaml
training:
  read: "how far did I run on Sunday" | "how was my week" | "what was my pace on my last run" |
        "how did I sleep" | "what's my resting heart rate doing" | "how much do I weigh"
  tools:
    training_summary: period week|month|quarter|year (or from/to); runs, distance, longest run, average pace,
      resting heart rate / HRV / sleep against the period before, weight change. Use it for "how was my week"
    training_activities: workouts, newest last; from/to as YYYY-MM-DD (to inclusive), type run|walk|ride|strength|other
    training_wellness: one row per day; resting heart rate, HRV (ms), sleep, weight, body fat
    current_datetime: turn "Sunday" or "last week" into dates first
  notes:
    - only the data of the person speaking; nobody can ask for someone else's
    - the person is known by their voice; in the app's chat, by their sign-in
    - an unrecognised voice has no training data: say you don't know whose it is
    - the data comes from their iPhone (Garmin Connect → Apple Health → Health Auto Export); if there is none,
      say so and that it is set up on their profile page in the app, under "Training: connect your iPhone"
    - numbers come from the tools; never add up distances or average paces yourself
    - say distances and paces in the house's units, as the tools give them; round for speech ("about 21 km")
    - a lower resting heart rate and a higher HRV usually mean better recovery; don't diagnose anything
    - there is no training plan yet; don't invent one or promise to follow one
  examples:
    - User: "how was my week?" → training_summary period=week
    - User: "how far did I run on Sunday?" → current_datetime, then training_activities from=<sunday> to=<sunday> type=run
    - User: "how did I sleep last night?" → training_wellness from=<today> to=<today>
```
