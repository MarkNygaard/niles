# Automations

An automation is "when this happens, do that": turn off the office lights
when the door opens, turn on the porch light at sunset, tell you when the
garden door has been open for ten minutes. You can make one by asking
Niles, or on the **Automations** page.

The page is in the avatar menu by default. **Settings → Menu** can move it
to the main navigation.

## Asking for one

Say or type the rule the way you would say it to a person:

- "when the office door opens, turn off the office lights"
- "turn on the porch light at sunset"
- "turn the lights off five minutes after the door closes"
- "tell me if the garden door has been open for ten minutes"

Niles reads the rule back before it saves anything: "When the office door
opens, I'll turn off the office lights. Shall I?" Say or type "yes" within
three minutes to save it, or "no" to drop it. In the chat you also get
the whole rule written out, with **Save** and **Don't save** buttons.

If something in the rule does not exist, such as a device with no such
name or a room with no lights, Niles says so instead of offering to save
it.

Working out a rule by voice takes the assistant when one is switched on,
so the satellite may take a few seconds to answer.

Other things you can ask:

- "what automations do I have"
- "pause the garden door automation", and later "resume" it
- "delete the garden door automation"
- "why didn't the office light turn off?", "did the garden door rule run?"

## The Automations page

Each rule shows as a card with its name, the rule as a sentence, and when
it last ran and how that went. A switch pauses or resumes it. A **Paused**
badge marks a paused rule, and **From config** a rule written in Niles's
config file, which can only be changed there.

**New automation** opens the editor. On a phone the editor shows the rule
but does not edit it: "Open on a wider screen to edit." You can still
pause a rule or delete it there.

### The editor

The rule is drawn as a flow from top to bottom: its triggers, the checks
under **Only if**, and the steps under **Then**. Add parts with **Add
trigger**, **Add condition** and **Add action**. Select a step to change
it in the side panel, move it with **Move up** and **Move down**, or take
it out with **Remove step**.

The toolbar has the rule's **Name**, an **Enabled** switch, **Delete**,
**Cancel** and **Save**.

| Triggers | Conditions | Actions |
|---|---|---|
| Door or window | Time of day | Lights in a room |
| Motion | Before or after sunset | One device |
| Device turns on or off | Device is on or off | Blinds |
| A reading changes | Door or window is | Wake-up |
| Button press | Motion sensor sees | Say something |
| Home or away | A reading is | Wait |
| Time of day | Home or away | If, then, else |
| Sunrise or sunset | Which trigger started it | |
| Timer goes off | | |

Some details worth knowing:

- **And has stayed so for** on a door, motion, device, reading or home or
  away trigger waits until the state has held that long. If it stops
  holding, the trigger does not fire.
- A motion trigger can watch one sensor or several, or every sensor in a
  room. With several, "none sees motion" means none of them has seen
  anybody.
- **Minutes after** on a sunrise or sunset trigger can be negative, for
  before. Sun rules need the house's location under **Settings → Home**.
- **On** limits a time trigger to some days. None picked is every day.
- **Say something** speaks in the room you pick under **In**, at a
  priority: Routine, Important or Urgent. Left on **Wherever Niles
  would**, it is an announcement for the house and is spoken in every
  room with a satellite. See [Notifications](notifications.md) for what
  quiet hours do with each priority.
- A brightness or warmth that a rule sets puts the light in manual mode,
  so the curve leaves it there. A rule that only switches a light on or
  off leaves it to the curve. See [Manual mode](lighting.md#manual-mode).
- **Wake-up** brings lights up from nothing over a number of minutes
  (45 unless you say), then holds them. See [Lighting and
  scenes](lighting.md#the-morning-routine).
- There is no scene action, and no way to change the heating.

### Limits

- 10 triggers at most, and at least one trigger and one action.
- A wait, or a **stayed so for**, is at most 24 hours.
- An if inside an if goes at most three deep.
- Brightness is 0 to 100%, warmth 2000 to 6500 K.

## Warnings

Niles warns you when a rule looks like it may never do what you meant,
though it still lets you save it. The common case is a check that only
happens after a wait. For example: a rule waits two minutes after the
hall's motion stops, then checks that the kitchen is empty too. If
someone is in the kitchen at that moment, the run ends, and nothing tries
again later. The warning says so, and suggests one motion trigger over
both sensors with **And has stayed so for** instead.

Warnings show on the rule's card, in the editor and above **Save** in the
chat.

## How rules run

- Conditions are checked once, when a run starts.
- If a rule fires again while it is still running, for example during a
  wait, the new run replaces the old one.
- Rules that fire at the same moment run in the order the page shows.
- A rule that is waiting when Niles restarts carries on afterwards with
  the time that was left. If you changed, paused or removed the rule in
  the meantime, the wait is dropped.
- A door that was already open when Niles started has not "just opened".
- Turning a light on from a rule brings it back to the daily curve, the
  same as a wall switch.

## Recent runs

Open a rule to see **Recent runs**: when each run started, which trigger
started it, how it ended (**Done**, **Stopped**, **Replaced**, **Failed**
or still **Running**), and its steps. Niles keeps the last 20 runs of each
rule until it restarts. When you ask why something did or did not happen,
Niles reads these.

## Renaming devices

A rule names devices by their name in Zigbee2MQTT. Rename a device and a
rule that uses it stops working. Its triggers never fire and its checks
read false. Rename devices before you make rules, or open the rule and
pick the device again. See [Naming your devices](naming.md).
