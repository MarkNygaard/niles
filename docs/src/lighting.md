# Lighting and scenes

Niles can set the brightness and warmth of your lights through the day,
wake you with light in the morning, and keep scenes you save. All of it
is under **Settings → Lighting**, and all of it works alongside voice, the
dashboard and the wall switches.

## The daily curve

The curve is a brightness and a colour temperature for every minute of
the day. Once a minute Niles moves every light that is on towards the
curve's value. It never turns a light on or off. It only changes lights
that are already on, and a light you switch on gets the curve's value at
once.

The curve is off in a new install. Turn on **Follow the curve** on the
**Daily curve** card to start it. Off leaves lights exactly where you put
them.

With it on, the card shows a chart of the day and these settings:

| Setting | What it does | Shipped value |
|---|---|---|
| **Morning ramp** | Lights come up from the night level to the day level across this window. | 05:45 to 06:30 |
| **Sunset ramp** | They wind back down across this one. | 21:30 to 23:00 |
| **Brightness** | The night and day levels, held flat outside the ramps. | 15% night, 100% day |
| **Fade** | How long a light takes to reach each new level. | 45 seconds |
| **Curve pause** | A weekly window when the curve stays where it was when the pause began. | none |

The sunset ramp runs at the clock times you give it, not at the real
sunset. The colour temperature follows its own points through the day,
which you can edit on the same card. The shipped points are 2000 K
through the night, 2700 K at the end of the morning ramp, 4500 K at
midday, 3500 K at 17:00, 2700 K at the start of the sunset ramp and 2000 K
at its end.

The **Curve pause** takes a weekday and a time at each end, such as
`fri 12:00` to `mon 06:00`. Inside it, lights you switch on get the level
the curve had when the pause began, so a weekend keeps Friday's light.

The fade only applies to the curve and the morning routine. Voice, the
dashboard and the wall dimmer change a light at once.

All times are read in the house's timezone, set under **Settings → Home**.
A wrong timezone shifts the whole day.

You can also change the curve by asking: "the evenings are too dark" or
"make the morning start at seven". Niles changes the setting, and the
change shows under **Settings → History**, where you can undo it.

## Manual mode

When you set a light yourself, Niles stops moving it with the curve. That
light is in manual mode. These put a light in manual mode:

- a spoken level, step or warmth ("dim the kitchen to 30%", "brighter",
  "warmer", "warm white")
- switching one light on or off by its name ("turn on the floor lamp")
- applying a scene
- the brighter or dimmer buttons on a wall dimmer
- setting brightness, warmth or colour from the dashboard
- a brightness, warmth, colour or effect that Niles sets when it answers
  a request with its own tools ("make the kitchen a bit cosier")
- a brightness or warmth that an automation sets

Turning a light on or off from the dashboard, by an automation, or a room
on by voice, does not. Turning a light on is how you ask for the curve
back.

A light leaves manual mode when:

- it is switched off and on again, or
- you say "back to normal" (every light), "back to normal in the kitchen"
  or "kitchen back to normal".

A level sent to a light that is off also turns it on, and that off-to-on
clears manual mode again, so the light gets the curve's value. Manual mode
keeps a level only on a light that was already on.

## Ambient lights

Some lights should stay dim and warm whatever the time, such as a shelf
strip or a night light. Pick them on the **Ambient lights** card. The
curve and the morning routine leave them alone. Voice, scenes and the
switch still control them.

Under **Held at** you can give them a brightness, a warmth and, for colour
lights, a colour. Leave these unset and Niles does not touch ambient lights
at all.

## Lamps on plugs

A smart plug does not say what is plugged into it. On the **Lamps on
plugs** card, pick the plugs that have a lamp on them. Those then count as
lights: "turn off the lights", room switches and scenes include them.
Every other plug is switched only when you name it, so the fridge stays on.

## The morning routine

The **Morning routine** card turns lights on at nothing and ramps them to
full across the morning window, whether or not the curve is running.
Switch on **Wake up with light**, then choose:

- **Days**: the days it runs. With none picked it never runs.
- **Morning window**: the same window as the curve's morning ramp.
  Changing it here changes it there.
- **Lights**: leave empty for every light the curve drives. Plugs in the
  list switch on at the end instead of ramping.
- **Except these**: lights to leave out. An empty **Lights** list plus a
  few here means every light but these.
- **Warmth**: empty, the lights follow the curve's warmth. Set an end
  warmth to hold one colour, and a start warmth too to go from one to the
  other across the window, like a sunrise.

At the start of the window, the routine only takes lights that are off. A
light that is already on is left as it is, and so is an ambient light.

A light you set yourself during the window, by voice, a dimmer or the
dashboard, is in manual mode. The routine stops ramping it, and at the end
of the window it keeps your level instead of going to full.

Turning the routine off keeps all its settings for when you turn it on
again.

An automation can also start a wake-up, at any time and with its own
lights and length. See [Automations](automations.md).

## Scenes

A scene is a snapshot of your lights that you can put back later.

| To | Say |
|---|---|
| Save one room | "save the living room as cosy", "save the kitchen scene as dinner", "save this as cosy in the living room" |
| Save the whole house | "save this as movie night", "save movie night" |
| Apply one | "cosy scene", "apply cosy", "scene cosy", "turn on cosy", "set the lights to cosy", "activate cosy" |
| Apply one room's lights from it | "apply cosy in the living room", "turn on cosy in the living room" |
| List them | "list my scenes", "what scenes do I have" |
| Delete one | "delete the cosy scene", "remove scene cosy" |

Saving works by voice or in the chat. There is no tool for it, so
the phrasings above are the ones that work.

A scene keeps every light and lamp plug in the room, or in the whole
house. It keeps whether each one was on or off, so applying it also
turns off the lights that were off when you saved it. Saving under a name
you already used replaces that scene, whichever room it was for. Names
ignore upper case and spacing, so "Movie Night" and "movie night" are the
same scene.

"In the living room" at the end of a name means the room when your house
has a room by that name. "Save this as cosy in the living room" saves a
scene called cosy with the living room's lights. "Save this as dinner in
paris" saves a scene called "dinner in paris", because there is no room
called paris. Applying "cosy in the living room" puts back only the
living room's lights from cosy.

"Turn on cosy" means the scene only when a scene called cosy exists and
no room or device has that name. A room or a device always wins.

Applying a scene puts its lights in manual mode, so the curve leaves them
until you say "back to normal" or switch them off and on.

Saved scenes show as buttons at the top of the **Home** page. One press
applies the scene. The buttons only appear once you have saved a scene.

To get back how the lights were at some earlier time without a scene,
ask: "make it like last night". Niles looks up what the lights were then
and sets them again.

## Blinds

The **Blinds** card sets how far a plain "open" opens. **Every blind**
is the house's level; below it each blind can have its own. Saying "open"
to a blind already at its level opens it all the way. A whole room only
goes all the way when every blind in it is already at its own level.

## WLED strips

WLED strips are added on the **WLED strips** card under **Settings →
Lighting**, by the MQTT topic each one publishes on (the one set under Sync
Interfaces in WLED itself). The card also says what each strip is made of,
because WLED does not report it. The curve warms the white channels of the
strips listed and leaves the rest their colour. A change there takes a
restart. Names follow the same `room/device` rule as the Zigbee lights;
see [Naming your devices](naming.md). Effects such as fire, candle or
rainbow can be asked for by voice.

## Lights and who is home

With presence set up, Niles can turn off whatever is on when the last
person leaves, and turn on chosen lights (the hall, say) when somebody
comes home. Both are off until you switch them on. See
[People and voices](people.md#who-is-home).
