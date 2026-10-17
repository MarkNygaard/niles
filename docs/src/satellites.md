# Satellites and hardware

A satellite is the microphone and speaker in a room. It listens for
"Niles" itself, sends what you say to Niles, and plays the answer. This
page covers what a house needs, how to set up a satellite, and the voice
settings.

## What a house needs

- **A Zigbee coordinator and Zigbee2MQTT.** Every Zigbee light, sensor and
  switch reaches Niles through Zigbee2MQTT. The coordinator Niles was built
  with is the SMLIGHT SLZB-06MU, placed centrally.
- **An MQTT broker**, such as Mosquitto, that Zigbee2MQTT and Niles both
  connect to. Its address and login go under **Settings → Devices**.
- **Piper**, which speaks Niles's replies. It runs in the house, not in a
  cloud.
- **A Postgres database and a secrets key.** Niles runs without them, but
  then it cannot keep keys typed into the app, conversations, Saved items,
  approval history, phone notifications, calendars, mail, Strava or the
  ChatGPT sign-in.
- **An ElevenLabs account** for hearing and **a Cerebras account** for
  thinking, each with an API key. Cerebras has a free tier.
- **One satellite per room** you want to talk in.

No GPU is needed. Niles listens on port 8080 for the app and on 10300 for
the satellites, and calls each satellite on 10301 to ring timers and speak
announcements. [Deployment](deployment.md) covers running Niles itself.

## The satellite

The current satellite is a reSpeaker XVF3800 with a XIAO ESP32-S3 on its
back, which Seeed sells as the "ReSpeaker XVF3800 USB 4-Mic Array". Add:

- a speaker on its 3.5 mm jack or its speaker terminal
- a USB-C cable that carries data
- Wi-Fi on 2.4 GHz

It runs Niles's own firmware. The wake word runs on the satellite, so
nothing leaves the room until it hears "Niles". Then it streams what you
say to Niles and plays the reply.

The ring of lights shows what it is doing:

| Ring | Meaning |
|---|---|
| Off | Waiting for "Niles" |
| Steady white | Listening |
| A light running round | Thinking |
| Steady green | Speaking |

Saying "Niles" while it is speaking or chiming cuts it off, so "Niles,
stop" works over a ringing timer.

## Setting one up

The full steps, with commands, are in the
[satellite firmware README](https://github.com/MarkNygaard/niles/blob/main/firmware/xiao-wyoming/satellite-idf/README.md).
In short:

1. **Flash the microphone board once.** The board has two USB-C ports.
   Through the one next to the 3.5 mm jack, flash Seeed's
   `i2s_master_v1.0.9_48k` image. Other Seeed images leave the satellite
   hearing nothing.
2. **Fill in the satellite's settings.** Copy `main/secrets.example.h` to
   `main/secrets.h` and set the Wi-Fi name and password and Niles's
   address. The address must be an IP address, not a host name.
3. **Build and flash the firmware** with ESP-IDF 6.0.1, through the XIAO's
   own USB-C port.
4. **Give the satellite a fixed IP address** with a DHCP reservation in
   your router. Niles knows which room a satellite is in by its address.
5. **Open the ports** if the satellite is on another network or VLAN:
   10300 from the satellite to Niles, and 10301 from Niles to the
   satellite.
6. **Add it under Settings → Satellites**, then restart Niles.
7. **Try it**: "Niles, what time is it?"

If it wakes when nobody said "Niles", or misses you when you did, the
README explains how to tune how sure it has to be.

## Settings → Satellites

"The microphones around the house. Which room one stands in is what makes
'turn the lights off' mean this room." Changes here take a restart.

Add a satellite with a name, such as `kitchen_echo`, and its IP address,
and press **Add**. It goes into the first room Niles knows; change it with
**Room** on its row. Rooms come from your devices, so a room appears once
something in it is paired.

Each row has:

- **Address**: its IP address.
- **Room**: the room it stands in.
- **Music**: the Sonos in the same room, when Sonos is set up, or "No
  speaker in this room".
- **Volume**: 10 to 100%. The board has no volume control of its own, so
  Niles turns its replies and chimes down before sending them.
- a bin to remove it.

A satellite that is not listed still works. It just cannot know which
room "here" is, and it cannot ring timers.

## Voice & AI

**Settings → Voice & AI** chooses who hears, who thinks and who speaks.
A checklist at the top says what is still missing.

**Hearing** turns what a satellite heard into words. Pick the provider,
give it its key and press **Save**. ElevenLabs is the one Niles ships with.

- **Only when it hears its name** ignores a wake unless what was heard
  starts with "Niles". A television that set off the satellite almost
  never says it. This works best with ElevenLabs, which spells the name
  right. Off by default.
- **Language** is what the house speaks, as a two-letter code such as
  `en`. Empty lets the transcriber work it out each time, which costs a
  little accuracy on short commands.

**Thinking** answers anything a fixed phrasing cannot, while you wait.
Pick the provider and the **Model**, give it its key, and choose a
**Reasoning effort**: the provider's default, low, medium or high.
Cerebras is the one Niles ships with.

**Speaking** is Piper. **Piper** is the address of the Piper server, and
**Voice** which of its voice models speaks.

**Assistant** chooses who answers the chat, works out new rules and
reviews the day: **Niles's own model**, **Claude** or **ChatGPT**. Claude
and ChatGPT only show here once they are set up under **Settings →
Add-ons**. See [Add-ons and accounts](add-ons.md#claude-and-chatgpt).

A key saved here is kept encrypted in Niles's database. A key set by an
environment variable shows "from the environment" and is changed there.

## Keeping recordings

**Settings → People → Keep what it hears** has a switch, **Keep wake
recordings**. On, Niles keeps the recording of every wake, the last 500,
in its database. That is how a wake word gets better at ignoring the
television, because the false wakes are the training data. It is off unless you
switch it on. **Delete all** removes what has been kept.

## The next satellite

A second satellite is being designed: a custom board with a bigger
speaker in its own chamber, presence radar, temperature and air sensors,
and power over USB-C with Wi-Fi or over Ethernet. It is not available yet.
