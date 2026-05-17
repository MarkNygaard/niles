# Niles — Neural Intelligence, Lightweight Edge System

> **About the name.** *Niles* (NYE-uls) is the project's name, the system's name, and the default wake word — three roles, one word. The acronym **N**eural **I**ntelligence, **L**ightweight **E**dge **S**ystem describes what it is: an AI-first home assistant that runs locally on small hardware. Users can configure a different wake word per satellite if they prefer; the project name stays Niles regardless.

## What this is

Niles is an open-source, AI-first home automation system designed to replace Home Assistant for users who:

- Have a relatively focused device ecosystem (mostly Zigbee via Z2M, maybe a few WiFi-native devices)
- Want sub-second voice interactions ("Alexa-fast" or faster)
- Prefer code/config over UI-driven setup
- Want a voice assistant with real LLM intelligence as a first-class citizen, not a bolted-on afterthought

Home Assistant is a fantastic project, but it solves a different problem: maximum protocol/vendor coverage for non-technical users. That makes it heavy, UI-driven, and slow to evolve in the directions that matter for an AI-first assistant. Niles is opinionated, code-first, and assumes the user is comfortable with infrastructure.

## Design principles

1. **Stream everything.** Audio in, tokens out, audio out. No batch operations in the voice path.
2. **Three tiers by trigger, not always-on.** Regex catches ~80% of commands instantly. Fast LLM catches ~19%. Smart LLM catches the rest.
3. **Tools, not RAG, for state.** The device registry is queryable structured data — that's tool calling, not retrieval.
4. **One protocol abstraction per category.** A `DeviceSource` trait now means adding Matter, Z-Wave, or Shelly later doesn't require a rewrite.
5. **Satellites are appliances.** Identical firmware, no per-room state, easily replaceable.
6. **Wake word is config.** Users pick their own, runtime-configurable per satellite. Supports microWakeWord custom-trained models.
7. **Voice responses come from the satellite, not the room speaker.** Sonos (or any room speaker) is a tool the LLM can use, not the answer surface — keeps voice latency low while preserving full music control.
8. **No admin UI in v1.** Configuration lives in code, config files, and naming conventions in the underlying source (Zigbee2MQTT, etc.). Adding a device means pairing it and naming it correctly upstream — Niles picks it up automatically. A UI may come later for monitoring and debugging, but it will never be required for normal use. Note: this principle applies to *routine operation*. One-time setup files (the canonical TOML config, an initial API token, Kubernetes manifests) are not "UI" and are explicitly allowed — anything you do once at install time can be text-based.
9. **Self-documenting through voice, without sacrificing speed.** Users can ask Niles how to use Niles ("how do I save a scene?", "what can you do?"), and the system answers from a single source-of-truth capability reference that also grounds command execution. Context is loaded in tiers so common commands never pay the cost of carrying full documentation.
10. **A personal infrastructure surface, not just home automation.** Niles controls lights and timers, but its real ambition is to be the voice surface for everything its user runs — coding workflows, calendars, email, deployments, monitoring. The architecture is built to accommodate external integrations as first-class citizens, not bolted-on plugins.
11. **One shared home, multiple recognized people.** The house has one state (one set of devices, one lighting curve, one set of scenes). People who live there are identified by voice and can have personal references ("my calendar", "wake me up"), but they don't have per-user *preferences* for shared things. Admin users can set rules controlling who is allowed to do what.

## Hardware

### Per room

- **reSpeaker XVF3800 with Case + XIAO ESP32-S3** (Seeed Studio SKU p-6628, ~$54 each)
  - XMOS XVF3800 DSP: AEC, beamforming, dereverberation, noise suppression, 360° pickup up to 5m
  - XIAO ESP32-S3: WiFi, Bluetooth 5.0, runs the satellite firmware
  - Pre-enclosed, no soldering, mount at head height
  - Order from Seeed's Germany warehouse for fast EU shipping
- **Existing room speaker** (Sonos, in this design) — used for music and whole-room announcements, ducked during voice responses
- **Small I2S speaker on the satellite itself** for voice responses

### Central infrastructure

- **SMLIGHT SLZB-06MU** — Zigbee coordinator, PoE/USB, placed centrally
- **Kubernetes cluster** (3x Miniforum MS-01 in this reference design, but anything that runs k8s works)
- No GPU needed for the reference cloud-based pipeline; optional for fully-local STT/LLM

## Network architecture

```
                          ┌──────────────────────┐
                          │  SLZB-06MU           │
                          │  (Zigbee coordinator)│
                          │  PoE, central        │
                          └──────────┬───────────┘
                                     │ TCP (zigbee2mqtt protocol)
                                     ↓
┌────────────────────────────────────────────────────────────────────┐
│                    Kubernetes cluster                              │
│                                                                    │
│  ┌─────────────┐  ┌──────────┐  ┌──────────────────────────────┐   │
│  │ Mosquitto   │←→│ Z2M      │  │  Niles (Rust service)         │   │
│  │ (MQTT)      │  │ Pod      │  │                              │   │
│  └─────────────┘  └──────────┘  │  - Wyoming protocol server   │   │
│         ↑                       │  - Device registry           │   │
│         │                       │  - Event bus (tokio)         │   │
│         │ MQTT                  │  - Intent router             │   │
│         └───────────────────────│  - Tool registry             │   │
│                                 │  - LLM clients               │   │
│                                 │  - TTS pipeline              │   │
│                                 │  - Speaker controllers       │   │
│                                 │  - HTTP API + WebSocket      │   │
│                                 │  - SQLite (state, history)   │   │
│                                 └──────────────────────────────┘   │
└────────────────────────────────────────────────────────────────────┘
        ↑                                ↑                ↑
        │ MQTT (other ESP devices)       │ Wyoming/WS     │ SOAP/UPnP
        │                                │ (audio)        │ (or vendor API)
   ESP32 sensors,                 ┌──────┴───────┐    ┌───┴──────┐
   Shellies, etc.                 │ XVF3800      │    │ Room     │
                                  │ satellites   │    │ speakers │
                                  │ (per room)   │    │ (Sonos)  │
                                  └──────────────┘    └──────────┘
```

## The voice pipeline

### Tier 0 — Fast-path regex (~400–500ms total, beats Alexa)

```
[1] User speaks
       ↓ audio captured by 4-mic array

[2] XMOS XVF3800 processes audio
       AEC removes speaker echo
       Beamforming focuses on the user
       Noise suppression cleans signal
       Dereverberation removes room echo
       ↓ clean voice over I2S

[3] XIAO ESP32-S3 (ESPHome firmware)
       microWakeWord detects wake word locally (~50ms)
       LED ring lights up
       Streams Opus audio over WiFi
       ↓ Wyoming protocol over TCP

[4] Niles service receives stream
       Tags with room, session_id, timestamp
       Forwards chunks to Groq Whisper Turbo as they arrive
       ↓

[5] Groq Whisper Turbo returns text (~200ms)
       e.g. "turn off the kitchen light"
       ↓

[6] Intent router (Tier 0)
       Regex match: "turn (off|on) the X (light|lights)" ✓
       Resolves device: kitchen_ceiling_light
       Publishes MQTT: zigbee2mqtt/kitchen_ceiling_light/set {"state":"OFF"}
       Plays pre-recorded "okay" through satellite speaker
       Total: ~400–500ms
```

Tier 0 patterns include common high-frequency commands where LLM latency would be unacceptable:

- Light control: "turn (off|on) the X light(s)", "dim X to N%"
- Timer setting: "set a timer for N minutes", "N minute timer", "timer for N minutes called X"
- Acknowledgments: "stop", "cancel", "okay" (during alarm playback)

Everything else falls through to Tier 1.

### Tier 1 — Fast LLM with tool calling (~900–1300ms total, matches Alexa)

When the regex router doesn't match, the request goes to a fast LLM:

```
[6b] Tier 1 LLM (e.g. Groq GPT-OSS 20B, ~300–500ms)
       Tools available:
         - get_device_state, set_device, list_devices_in_room
         - control_room_speaker, set_reminder
         - search_calendar, search_events
         - escalate_to_smart_model
       LLM picks tool, generates response
       Response streams into TTS as tokens arrive
       TTS audio streams to satellite as it's generated
```

### Tier 2 — Smart LLM for genuine reasoning (~2–5s, only when needed)

```
[6c] Tier 2 LLM (e.g. Claude Sonnet 4.6)
       Triggered when Tier 1 calls escalate_to_smart_model
       Tier 1 plays a holding response first ("let me think...")
       Smart model does the actual work with full tool access
       Streams response to TTS
```

Tier 2 is a rare-escalation path for individual hard requests — meal planning with constraints, complex multi-step reasoning, one-shot questions that genuinely benefit from a smarter model. It is **not** a sustained "discussion mode" for design work or project planning. Voice is a poor medium for that kind of work (linear, slow to absorb, hostile to code and structure) and Tier 2 via API is metered, which makes long discussion sessions expensive. Sit-down planning work belongs at the computer with Claude Code or similar, not via Niles. See "What Niles is not" for the explicit positioning.

## Model recommendations (May 2026)

These are the current recommendations; the system should be provider-agnostic so they can be swapped.

| Stage | Recommended | Why | Approx latency | Approx cost |
|---|---|---|---|---|
| STT | Groq Whisper Large v3 Turbo | Fastest hosted Whisper, EU endpoint | ~200ms | ~$0.04 / hour audio |
| Tier 1 LLM | Groq GPT-OSS 20B | ~885 t/s, ~770ms TTFT, tool calling | ~600–800ms | ~$0.13 / 1M blended |
| Tier 2 LLM | Claude Sonnet 4.6 | Best quality-to-speed for hard reasoning | ~2–3s | Standard Anthropic pricing |
| TTS | Piper (self-hosted) | Free, ~100ms first audio, decent voices | ~100ms | Free |
| TTS upgrade option | ElevenLabs Flash v2.5 or Cartesia Sonic | Better quality, ~75–90ms first audio | ~75–90ms | Paid |

**All-Anthropic alternative:** Claude Haiku 4.5 as Tier 1 (~690ms TTFT, ~95 t/s, excellent tool calling), Sonnet 4.6 as Tier 2.

**Fully-local alternative** (requires GPU node): whisper.cpp `base.en` or `distil-small.en` for STT, Llama 3.1 8B locally for Tier 1, escalate to a hosted Tier 2 only when needed.

**EU licensing note:** Meta's Llama 4 multimodal weights are not licensed to entities domiciled in the EU (text-only via hosted APIs like Groq is fine). Document this clearly in the README so users know.

## Device naming convention (the no-UI strategy)

To avoid building an admin UI in v1, Niles uses **device names in the underlying source as the single source of truth** for room/device structure. There is no separate Niles-side device database, no admin screens, no manual room assignment.

### The convention

Devices are named in their upstream source (Zigbee2MQTT, etc.) using:

```
<room>/<device>
```

Examples:

- `kitchen/ceiling_light`
- `kitchen/counter_light`
- `living_room/floor_lamp`
- `bedroom/window_sensor`
- `office/desk_lamp`

Rules:

- `/` separates room from device (matches MQTT topic semantics — clean data flow)
- `_` replaces spaces inside multi-word names (`living_room`, not `living-room` or `living room`)
- Lowercase only
- The room segment is everything before the first `/`; the device segment is everything after

### Devices not in a room

Some devices don't belong to a normal room: the Zigbee coordinator, repeaters, outdoor sensors, system-level things. Use reserved prefixes:

- `system/` — coordinators, repeaters, internal infrastructure
- `outdoor/` — garden, patio, anything physically outside
- `none/` — explicitly unassigned (use sparingly; usually means it should be named properly)

Niles treats devices under reserved prefixes as not-in-a-room. They're invisible to room-scoped commands like "turn off all the kitchen lights" but still accessible by full name.

### Fully-qualified names across sources

When Niles eventually supports multiple device sources (Z2M plus Shelly, Matter, etc.), names are namespaced by source internally:

```
z2m:kitchen/ceiling_light
shelly:kitchen/dishwasher_plug
matter:bedroom/thermostat
```

The source prefix is mostly invisible to users — it's an internal identifier used by Niles for routing and debugging. End users (and the LLM) see the unprefixed `kitchen/ceiling_light` form. The source prefix prevents naming collisions if two different sources happen to expose a device with the same room/device name.

### What Niles does with this

On startup and whenever the underlying source publishes a device list update:

1. Niles subscribes to the source's device-list topic (e.g. `zigbee2mqtt/bridge/devices` for Z2M)
2. Parses each device's `friendly_name`
3. Splits on `/` to extract room and device
4. Reads the source's existing metadata to determine device type/capabilities (Z2M's `definition.exposes` array already tells you whether a device has on/off, brightness, color, temperature reading, etc. — no separate config needed)
5. Builds an in-memory registry: `HashMap<RoomName, HashMap<DeviceName, DeviceInfo>>`
6. Subscribes to per-device state topics for live updates

To add a new device: pair it in the upstream source, name it `<room>/<device>`, and Niles picks it up automatically. To rename: change it upstream and Niles updates within seconds. **No Niles-side admin needed.**

### Why this works well for AI-first

The naming convention is self-documenting in a way that's directly useful for LLM tool calling. When `list_devices_in_room("kitchen")` returns `["ceiling_light", "counter_light", "under_cabinet"]`, the LLM has everything it needs to handle "dim all the kitchen lights" — the structure is in the data, not buried in a database.

For ambiguous references like "the big light" or "the reading lamp," Niles relies on the LLM to resolve them against the full device list rather than maintaining a separate alias system. This is what LLMs are good at. An alias file can be added later if specific cases consistently fail.

### Trade-offs of this approach

Honest about the downsides:

- **Renaming a room means renaming every device in it.** If "living_room" becomes "lounge," every device in that room needs a Z2M rename. Z2M supports bulk operations, but it's still friction. Rare in practice.
- **No native support for "this device is in two rooms."** Pick one. Z2M groups can be read separately for things like "all upstairs lights" later.
- **No place for device metadata that isn't in the name or upstream source.** "This light is above the dining table" or "this is the reading lamp" can't be expressed in the name. The LLM can usually infer from context; if it can't, a side-car `devices.toml` with per-device hints could be added in a future version. Not needed for v1.
- **Sensor names get verbose.** `bedroom/window_temperature_sensor`. Acceptable.

### Migration path if a UI is ever wanted

Because the registry is derived from upstream names at runtime, adding a UI later means adding a *view* of that registry — not migrating data into a new system. The naming convention remains canonical even with a UI on top.

## Ambient lighting (the always-on circadian system)

Adaptive lighting is a first-class concern in Niles, not a plugin. The goal is a home that smoothly shifts brightness and color temperature throughout the day to support natural circadian rhythms, with a gentle morning wake-up routine on selected days, and that respects manual control without fighting it.

This section defines the full lighting model. Six rules, all consistent.

### The universal daily curve

A single brightness-and-color-temperature function governs the home, every day. It is defined by anchors and ramps:

```
Night floor:    15% brightness, ~2000K color temp (very warm)
                Holds from sunset end → next morning's sunrise start.

Morning ramp:   05:45 → 06:30 (universal time, same every day)
                Brightness: 15% → 100% (continuous from the night floor)
                Color temp: ~2000K → ~2700K (warm daytime)

Daytime:        06:30 → sunset_start
                Brightness: 100%
                Color temp: continues curve (warm morning → cool midday
                            ~4500K → warm afternoon)

Sunset ramp:    sunset_start → sunset_start + 90min
                Brightness: 100% → 15%
                Color temp: ramps back toward ~2000K

Night floor:    resumes after sunset ramp completes
```

The curve only describes state for lights that are currently on. It does not turn lights on or off (with one exception — the morning routine, below). Lights are off because no one turned them on; the curve simply waits.

**The curve is continuous.** Brightness is well-defined at every minute and never jumps — a hallway light left on overnight at the 15% night floor drifts smoothly upward through the morning ramp to 100% by 06:30. The morning routine (next section) is a separate concern that turns *its* target lights on at 0% and runs its *own* ramp; it does not modify the curve that governs already-on lights.

### What varies day to day

Two things, only:

1. **Whether the morning routine fires.** On configured day patterns (e.g. Mon–Fri), bedroom lights are auto-turned-on at the start of the morning ramp so the user wakes up to a gentle sunrise. On other days, the curve runs the same ramp shape, but no lights are auto-turned-on — they stay off until the user manually activates them.

2. **Sunset start time.** Typical values: 21:30 on weeknights, 24:00 or 01:00 on weekend nights. The sunset ramp duration (~90 min) and end state (15% night floor) are universal.

Everything else — sunrise timing, ramp shape, color temperature curve, night floor — is identical every day.

### The morning routine

This is the only system-initiated turn-on event in Niles. It exists to wake the user gently.

**Behavior:**
- Configured per day pattern (e.g. Mon–Fri at 05:45) with a target set of lights (e.g. `bedroom/*`)
- At the trigger time, the routine checks each target light's current state
- For target lights that are **off**: routine turns them on at 0% and applies its **own** ramp from 0% → 100% over the morning window (05:45 → 06:30). The user doesn't perceive the difference between "off" and "0%" since the room was dark, so there is no visible jump. This routine ramp is distinct from the curve's morning ramp (which goes 15% → 100% for already-on lights, continuous from the night floor).
- For target lights that are **already on**: routine skips them. The user is already up. The light continues to follow the curve from its current value (typically the 15% night floor, smoothly ramping to 100%).
- The routine completes at the end of the ramp window; control hands back to the regular curve, which by then is at 100% anyway, so the handoff is seamless.

**Skip overrides:**
- Single-day skip: "Niles, skip tomorrow's wake-up" — sets a one-day skip flag
- Date-range skip: "Niles, no wake-up routine from July 14 to July 21" — for vacations

Both are stored in config and consulted by the routine at trigger time.

A skip override only disables the auto-on trigger for that day. **The curve itself is unchanged** — the curve's morning ramp still runs from 15% → 100% between 05:45 and 06:30, sunset still happens, color temperature still cycles. The only difference is that no lights are automatically turned on, so the bedroom stays dark while you sleep in. If you wake up later and manually turn on a light, you get the curve value for whatever time it currently is (typically 100% if you're up past 06:30).

### Manual interactions

Manual control always wins, immediately, and persists until the user signals release (an off→on cycle).

**Manual turn-on, first click:**
The light comes on at the current curve value for that moment.

| Time of day | Curve value | What you get |
|---|---|---|
| 3am toilet trip | 15%, ~2000K | Dim warm light. Not blinded. |
| Saturday 09:30 | 100%, ~4000K | Full bright daylight color. |
| Mid-sunset ramp at 22:15 | ~50%, ~2500K | Light matches the room's wind-down. |
| During morning ramp at 06:10 (weekend, no auto-on) | ~62%, ~2400K | Light comes on at current ramp value. |

**Manual escalation, subsequent clicks within the same on-session:**
Click 2 → 80%
Click 3 → 100%
Click 4 → back to curve value (and out of manual mode)

Once a light has been escalated past the curve value, it is in **manual mode**.

**Manual mode:**
- Curve no longer touches this light — neither brightness nor color temperature
- Sunset ramp does not dim this light
- Color temp curve does not adjust this light's warmth
- Manual mode is per-light, not per-home; other lights continue normally
- Manual mode is cleared on the next off→on cycle

Voice or dimmer adjustments to brightness or color temp also put the light in manual mode. Any explicit user adjustment is a signal of "I have a specific need; leave this one alone."

**Manual off:**
- Outside the morning routine: simply turns the light off. Next on returns to normal curve behavior.
- **During an in-progress morning routine ramp: cancels the routine for that light for today.** The routine does not resume a minute later, does not retry, does not come back on. Off means off. (This is the fix for the most common frustration with HA's adaptive lighting.)

### Why this works

Six rules, all consistent:

1. **The curve always defines state for on lights.** No exceptions, no fighting between layers.
2. **The curve never turns lights off.** Only manual actions and the morning routine's start affect on/off state.
3. **The morning routine is the only auto-on event.** It only fires when target lights are off.
4. **Manual turn-on = curve value, with click-to-escalate.** Same rule all day, every day.
5. **Any manual adjustment to brightness or color temp → manual mode until next off→on.**
6. **Manual off during a ramp cancels the routine.** No system override of explicit user intent.

The system has no UI for any of this. Configuration is a single YAML/TOML file (sunrise/sunset times, day patterns, target lights, color temp curve points) plus voice commands for ad-hoc adjustments ("skip tomorrow's wake-up," "set sunset 30 minutes later this week").

### How the curve is computed

Each day at midnight, Niles computes that day's curve by:

1. Loading the universal curve template
2. Looking up today's day pattern → determining if morning auto-on fires, and which sunset start time applies
3. Applying any skip overrides for today
4. Storing the resolved curve in memory for use during the day

The `niles-scheduler` crate handles this. The curve becomes a pure function of `(time_of_day) → (brightness, color_temp)` that runs on a tick (every 30–60 seconds during plateau periods, every 5–10 seconds during ramps to keep transitions smooth).

For each tick, the scheduler iterates over all currently-on lights that are not in manual mode and publishes new state commands to MQTT only when the value has changed enough to matter (debounce threshold to avoid spamming the network with sub-perceptible updates).

### What this section does not specify

These are implementation choices left to the build phase:

- Exact night floor percentage (15% is the working default; tunable per home)
- Color temperature curve shape (cubic interpolation? linear between anchors? — visually indistinguishable, pick the simplest)
- Tick rate during ramps
- Debounce threshold for "value has changed enough to send a new command"
- Behavior on edge cases like a light dropping off the network mid-ramp (probably: log it, skip until it reconnects, don't retry)

These will likely emerge from real-home testing during Phase 6.

## Timers and alarms

Timers are table-stakes for any voice assistant replacing Alexa. They are the most common reason people talk to a voice assistant after lights, so they ship in v1 alongside the voice loop, not as a later addition.

### Behavior

**Setting a timer:**
- Triggered by Tier 0 fast-path (regex), not the LLM — timer setting must feel instant
- Voice patterns: "set a timer for 8 minutes", "5 minute timer", "set the pasta timer for 12 minutes", "timer for an hour and a half"
- Optional name extracted from the phrase ("pasta timer" → `timer:pasta`)
- If unnamed, the duration becomes the disambiguation handle ("your 8 minute timer")
- Originating satellite is recorded with the timer
- Confirmation is short and immediate: "8 minute timer started"

**While running:**
- "How long left on the pasta timer?" → reads remaining time
- "Cancel the laundry timer" → removes the timer
- "List my timers" → enumerates active timers
- These are handled by the Tier 1 LLM since they involve name resolution

**When a timer expires:**

The alarm escalates in two stages:

1. **Stage 1 — originating satellite only.** The satellite that set the timer plays the alarm sound. If Sonos is playing music in that room, Niles ducks its volume during the alarm.

2. **Stage 2 — all satellites, after 10 seconds without acknowledgment.** If no acknowledgment arrives within 10 seconds, the alarm starts playing on every satellite in the home. Sonos in any room with music gets ducked.

**Acknowledgment** can be:
- A voice command on any satellite: "Niles, stop" / "Niles, got it" / "Niles, cancel"
- A physical button press on the originating satellite (if the hardware exposes one — the XVF3800 board has user-programmable buttons; one can be designated "ack")

Once acknowledged, the alarm stops everywhere and the timer is marked complete.

This two-stage design gives the Alexa-like reliability of "the kitchen keeps beeping until you stop it" while not spamming the whole house unless the user has actually left earshot.

### State and persistence

Timer state lives in the central Niles service, not on the satellites:

- Stored in SQLite immediately on creation, so a service restart picks up active timers
- The scheduler reloads active timers on startup; if a timer's expiry passed while the service was down, it fires immediately
- Satellites are dumb — they receive "play alarm" / "stop alarm" commands from the central service

This matches the broader architecture: Niles is the brain, satellites are appliances. Niles availability is already a hard dependency for voice, so timer availability inheriting that dependency does not change the failure mode.

### Alarm sound

- Pre-recorded audio cached on the satellites (not TTS-synthesized — too slow and inconsistent)
- A small set of bundled options (gentle bell, kitchen timer, urgent alarm); user picks a default in config
- The same sound is used in both escalation stages; volume and looping handle the urgency, not separate audio files

### Where this lives in the codebase

The `niles-scheduler` crate is the natural home, since it already handles time-driven behavior (the lighting curve, the morning routine). Timer expiry is just another scheduled event. The crate exposes:

- `set_timer(duration, name?, originating_satellite_id)` — returns a `TimerId`
- `cancel_timer(timer_id_or_name)`
- `list_timers(satellite_id?)`
- `get_timer_remaining(timer_id_or_name)`

These same functions are exposed as LLM tools (for the query and cancellation flows) and called directly by the Tier 0 fast-path (for timer setting, where latency matters).

### Edge cases

- **Service restart while a timer is running:** SQLite-backed, timer survives. If expiry passed during downtime, fires immediately on restart.
- **Two timers expiring simultaneously:** Each fires its alarm independently. Acknowledging "stop" stops the most recent one; "stop all" stops everything. The LLM resolves "stop" vs "stop all" based on intent.
- **Timer set in a room with no Sonos:** Just plays alarm on the satellite. No ducking needed.
- **Network partition between Niles service and originating satellite at expiry time:** Niles attempts the originating satellite, fails, immediately falls back to Stage 2 (all satellites). Better to over-alarm than miss the timer.

## Scenes

Light scenes let users save a set of light states under a name and recall them later. The interaction model is "set the lights how you want, then save" — no config files, no YAML, no UI. The lights themselves are the editor.

This feature reuses the lighting model's *manual mode* mechanism — scenes don't introduce a new override concept, they just bulk-apply manual mode to a saved set of lights.

### Save behavior

The scope of a scene is determined at save time by the phrasing:

- "Save this as cozy" → whole-home scope
- "Save this as cozy in the living room" → living-room scope
- "Save the living room scene as cozy" → living-room scope (same, different phrasing)

The scope determines what gets captured: a whole-home scene captures every light in every room as it currently is; a room-scoped scene captures every light in that room. **Off counts as a state** — a scene records each light's current state regardless of whether it's on, off, in manual mode, or following the curve. What you see is what you save.

This makes scenes deterministic on apply. "The kitchen should be off" becomes part of the scene, not an emergent property.

### Apply behavior

The scope determines what gets affected. A living-room scene only touches living-room lights; lights elsewhere are untouched, regardless of what they're doing. A whole-home scene touches everything.

Within scope, **every light gets asserted** to its saved state, including lights currently off (which may get turned on) and lights currently on (which may get turned off). The scope is the scene's domain.

Lights set by a scene enter manual mode, just like any other manual override. The curve won't drift them back. They release on the next off→on cycle, or via "back to normal."

### Apply phrasing

- "Cozy mode" / "Cozy" → applies the most specific matching scene
- "Cozy in the living room" → applies a living-room-scoped `cozy` scene if one exists; otherwise falls back to applying the living-room portion of a whole-home `cozy` scene

This fallback is a nicety: even with only a whole-home scene saved, room-scoped phrasing still works by filtering.

### Same name, different scopes

Multiple scenes can share a name when their scopes differ. The full identity of a scene is `(name, scope)`:

- `cozy` — whole-home
- `cozy/living_room` — living-room scoped
- `cozy/bedroom` — bedroom scoped

These coexist as three distinct scenes. The apply phrasing picks the right one based on whether a room is mentioned.

### Back to normal — the escape hatch

A dedicated voice command exits scene state and returns to curve control:

- "Niles, back to normal" / "Niles, normal lights" — clears manual mode on every light in the home, resuming curve behavior across the board
- "Niles, back to normal in the living room" — clears manual mode only for living-room lights

This is the deliberate escape from a scene. Without it, scenes risk becoming roach motels: easy to enter, hard to exit. Clearing manual mode is also what an off→on cycle does for a single light — "back to normal" is the home-wide voice equivalent.

### Storage

Scenes live in SQLite alongside timers. Minimal schema:

```
scenes
  id (uuid)
  name (e.g. "cozy")
  scope (room name or NULL for whole-home)
  created_at
  updated_at

scene_lights
  scene_id
  light_id
  state (on/off)
  brightness (nullable)
  color_temp (nullable)
  color (nullable, for RGB lights)
```

### LLM tools and Tier 0 fast-paths

Tier 1 LLM tools:

- `save_current_state_as_scene(name, scope?)` — snapshot lights in scope
- `apply_scene(name, scope?)` — load and apply
- `list_scenes(scope?)` — enumerate
- `delete_scene(name, scope?)`
- `update_scene(name, scope?)` — overwrite with current state
- `exit_scenes(scope?)` — clear manual mode in scope (the "back to normal" handler)

Tier 0 fast-path patterns for low-latency common cases:

- `"<name> mode"` / `"<name>"` → apply scene
- `"<name> in the <room>"` → apply room-scoped scene
- `"save (this|current) as <name>"` → save whole-home scene
- `"save (this|current) as <name> in the <room>"` → save room-scoped scene
- `"back to normal"` / `"normal lights"` → exit scenes (whole-home)
- `"back to normal in the <room>"` → exit scenes for that room

The LLM is still the right path for anything ambiguous (renaming, deleting, listing, "save this somewhere"), but the common save/apply/exit flows skip the LLM round-trip.

### What this section does not specify

- The exact disambiguation flow if multiple scenes match a query (e.g. user says "cozy" with no room and has both whole-home and bedroom-scoped versions): probably "ask which one" via short voice prompt, decided in Phase 6 testing
- Whether scenes can include non-light devices (Sonos volume, blinds, etc.): out of scope for v1 — keep scenes lighting-only initially, expand later if the abstraction holds up
- Transition smoothness: should applying a scene fade or snap? Probably snap by default with an optional "slowly" modifier ("cozy mode, slowly") — left to implementation

## Music and radio

Music playback is one of the most-used voice features in any home assistant. Niles handles it by leveraging Sonos (or any room speaker integration) rather than building per-service integrations. This is a deliberate architectural choice with concrete consequences.

### The principle: leverage Sonos, don't rebuild

Sonos already integrates natively with TuneIn, Spotify, Apple Music, Amazon Music, Tidal, Deezer, YouTube Music, SoundCloud, Pandora, SiriusXM, and many more. Each integration handles account auth, codec support, content search, DRM, and audio quality — work that took those services years to perfect.

Niles does not need its own integrations for any of these. When a user says "play TuneIn" or "play my Discover Weekly," Niles sends a SOAP/UPnP command to Sonos with the appropriate content URI, and Sonos handles the actual streaming.

The implication: **"music services supported by Niles" = "music services supported by Sonos."** A long list, maintained by Sonos.

The same principle generalizes. Niles is the voice surface over existing systems, not a replacement for them.

### Music intents, not source-specific tools

The LLM-facing tools think in terms of what the user wants, not which service. Each intent maps internally to source-specific Sonos actions, but users never need to know:

- `play_radio(station?, room?)` — TuneIn radio. "Last station" if none specified, named station otherwise.
- `play_music(query, source?, room?)` — search-based playback (Spotify, Apple Music, etc.)
- `play_podcast(query?, room?)` — most recent unplayed episode if no query
- `resume_in_room(room?)` — whatever was last playing, just resume
- `play_what_was_playing_in(source_room, target_room)` — for "play what's in the living room in the kitchen too"
- `pause(room?)`, `skip(room?)`, `set_volume(level, room?)` — basic transport
- `group_speakers(rooms)`, `ungroup_speaker(room)` — multi-room playback

Adding a new music service (when Sonos adds one, or for a future non-Sonos backend) means extending the implementations behind these tools, not adding new tools.

### Per-room music state

For "play TuneIn" without a station name to work like Alexa's, Niles tracks per-room music state:

```sql
room_music_state
  room_name (primary key)
  last_source       -- 'tunein', 'spotify', 'apple_music', etc.
  last_content      -- station URI, playlist URI, podcast URI
  last_played_at
  was_playing_when_stopped
```

Updated whenever music starts or stops via Niles. Manually-started playback (from the Sonos app directly) is also captured by polling Sonos's current state — so the user's intent persists regardless of how playback was triggered.

This makes "Niles, play TuneIn" resume the last TuneIn station for the current room. "Niles, play TuneIn in the kitchen" resumes the kitchen's last TuneIn station specifically.

### Per-room vs per-user

Music state is **per-room, not per-user**, by default. The kitchen radio is a kitchen thing — both household members in the kitchen probably want the same station resumed. If Majse was last in the kitchen listening to a different station, that's the current "kitchen state" because she changed it last. The room is the unit.

Per-user music only matters when the user explicitly says "my":

- "Niles, play my Discover Weekly" → uses the speaker's Spotify account
- "Niles, resume my podcast" → uses the speaker's podcast subscriptions

This works because Sonos S2 supports multiple Spotify accounts linked simultaneously (Spotify Connect), and similar for other services. When the user explicitly references "my," Niles routes to the speaker's account rather than the household default.

If you and your partner each have your own Spotify accounts linked to Sonos, "my Discover Weekly" routes correctly per speaker once user recognition is active.

### "What was already playing" semantics

If Sonos is playing music in the kitchen and the user says "play TuneIn," the new playback replaces the current one. "Play X" is a directive that overrides whatever's currently playing. To duck-and-resume instead, the user would say "pause" first, do the thing, then "resume."

This matches Alexa/Sonos voice control conventions and is the least surprising default.

### Voice command coverage

Common cases as Tier 0 fast-paths (sub-second response):

| Pattern | Action |
|---|---|
| "play (the )?radio" / "play tunein" | resume last TuneIn station in current room |
| "play <station name>" | play named station via TuneIn |
| "play <song or artist>" | Spotify/Apple Music search → play |
| "play my <playlist>" | per-user playlist via speaker's account |
| "pause" / "stop the music" | pause current room |
| "resume" / "keep playing" | resume current room |
| "louder" / "turn it up" | volume up 10% |
| "quieter" / "turn it down" | volume down 10% |
| "next" / "skip" | next track |
| any of above + "in the <room>" | target specific room |
| "play this in <room> too" | group current room with target room |

Search-based plays ("play that song with the heavy drums I like") are Tier 1 since they need LLM-driven music search.

### Rooms without a music speaker

Some rooms may have a voice satellite but no room speaker (small spaces, by choice, etc.). The behavior:

- Voice commands work fully — satellites handle their own voice responses
- Notification chimes and timer alarms play on the satellite
- Music requests gracefully degrade: "There's no music speaker in the bathroom. I could play it in the living room instead?"

This is honest about what the room can do without trying to fake music playback through the satellite's small voice speaker.

### Where this lives in the codebase

No new crate needed. Music extends:

- `niles-speakers` — adds source-aware playback methods to the Sonos implementation, music intent dispatchers, per-room state tracking in SQLite
- `niles-tools` — adds the music intent tools
- `niles-intent` — adds Tier 0 patterns for common music commands

### What this section does not specify

- The exact mechanism for Sonos to expose its linked-account info to Niles (which accounts are available, default selection logic) — explored in implementation
- Whether voice-driven Spotify search uses Sonos's search API or Spotify's Web API directly — implementation choice, start with Sonos's
- Multi-room grouping nuances when speakers are different generations or have different capabilities — Sonos handles most of this; document caveats as they emerge

## Self-documentation and explainability

Users forget phrasings. They want to know what the system can do. They want to understand why a light is at 30% when it "should" be at 100%. Traditional home automation answers these questions with external docs, forum posts, and history graphs in a UI. Niles answers them through voice, against a single source of truth that doubles as the LLM's grounding for command execution.

This is genuinely a feature that traditional home automation can't easily match: the system that *runs* your home is the same system that *explains* your home.

### The tiered context architecture

The Tier 1 LLM operates on context loaded in three tiers, assembled per-request:

**Tier A — Always loaded (~300 tokens)**
- Active rooms and device summary (auto-built from the registry)
- Currently active timers and applied scenes
- Today's curve summary in one line (e.g. "lights at 100% bright, sunset starts at 21:30")
- Compact tool catalog: tool names with one-line descriptions, no examples
- Minimal system prompt

This is what every Tier 1 call carries. Enough for the LLM to handle device commands, music control, and simple queries without paying any documentation overhead.

**Tier B — Topic-specific reference (~500-800 tokens per topic)**
- Loaded on-demand by the intent router when the user's utterance touches a specific subsystem
- Each subsystem has its own reference: lighting, scenes, timers, music, routines, etc.
- Includes phrasings, examples, edge cases

**Tier C — Full reference**
- Only loaded for genuinely meta questions ("what can you do?", "give me an overview")
- Rare; concatenation of all Tier B blocks

### How context tiers are selected

Two mechanisms work together to pick the right context for each request:

**Topic detection in the Tier 0 intent router.** The same router that handles fast-path commands also runs a sub-millisecond keyword scan to tag requests with relevant topic IDs:

```
"scene" / "<name> mode" / "save this" / "back to normal"  →  scenes
"timer" / "alarm" / "remind me"                            →  timers
"wake" / "sunrise" / "sunset" / "morning"                  →  lighting
"how" / "what can" / "explain" / "show me how"             →  also load matched topic
"play" / "skip" / "pause" / "louder"                       →  music
```

If a request matches one or more topics, the corresponding Tier B references are loaded before the LLM call. If multiple topics match, all of them load. If nothing matches, only Tier A loads.

**LLM-requested expansion via tool.** For utterances that don't trigger topic detection but turn out to need more context, the LLM has a fallback:

```
look_up_capability(topic) → returns the Tier B reference for that topic
```

The LLM only calls this if it can't answer with what it has. Most of the time, regex topic detection already loaded the right thing, so this tool stays unused. When it fires, it costs one extra round-trip (~500-700ms), but only for edge cases.

### Latency by request type

| Request type | Context loaded | Latency budget |
|---|---|---|
| Common command ("turn off X") | Tier A only | ~700ms |
| Topic-specific command ("cozy mode") | Tier A + 1× Tier B | ~750ms |
| How-to question ("how do I save a scene") | Tier A + 1× Tier B | ~900-1000ms |
| Edge case needing lookup | Tier A → look_up_capability → second call | ~1500ms |
| "What can you do" overview | Tier A + Tier C | ~1100ms |

The common case stays fast forever, regardless of how much the capability surface grows.

### The capability reference format

Reference files are deliberately terse and structured. The LLM is excellent at reading structured data; full prose paragraphs waste tokens. Example for scenes:

```yaml
scenes:
  save: "save this as <name>" | "save this as <name> in the <room>"
  apply: "<name>" | "<name> mode" | "<name> in the <room>"
  exit: "back to normal" | "back to normal in the <room>"
  list: "what scenes do I have"
  delete: "delete the <name> scene"
  notes:
    - scope set at save (whole-home unless room specified)
    - "off" counts as a state; whole-home scenes turn off uncaptured lights
    - applied scenes put lights in manual mode until off→on or "back to normal"
  examples:
    - User: "save this living room scene as cozy" → save scope=living_room
    - User: "cozy mode" → apply most-specific cozy scene
```

This compresses what would be paragraphs of prose into something the LLM parses instantly.

### Single source of truth

The reference files live in the repo at `docs/capabilities/<topic>.md`. They are:

- **Loaded by the runtime** for Tier B context assembly
- **Rendered into the user-facing mdBook docs** as part of the docs build

The runtime and the public documentation are generated from the same source. They cannot drift. When a phrasing changes, both update together.

### State explanation

How-to questions are answered from the capability reference. State questions — "why is the bedroom light at 30%?" — need different machinery: access to current state and the reasoning behind it.

The `explain_device_state(device_id)` tool returns structured data:

```json
{
  "device": "bedroom/ceiling_light",
  "current_state": { "on": true, "brightness": 30, "color_temp": 2400 },
  "control_source": "manual_mode_since_22:14",
  "curve_would_be": { "brightness": 45, "color_temp": 2300 },
  "active_routines": [],
  "applied_scene": null
}
```

The LLM reads this and phrases the answer naturally: "It's at 30% because you adjusted it manually at 22:14, which put it in manual mode. The curve would have it at 45% right now. Want me to put it back to normal?"

This kind of explanation is painful in a traditional UI (find the history graph, decode the override entry, interpret what it means) and natural in a voice interface.

### Discovering capabilities

A new user doesn't know what to ask. "Niles, what can you do?" loads Tier C and the LLM responds with a structured overview — short, scannable, with one or two example phrasings per area:

> I can control your lights, set timers, save and recall lighting scenes, and control your Sonos. I can also explain how things work or why your home is in a particular state. Try "turn on the kitchen lights" or "how do I save a scene" to get started.

Not the full reference — that's overwhelming when spoken. A summary that invites further questions.

### Where this lives in the codebase

A new crate, `niles-capabilities`:

- Loads all reference files at startup
- Exposes lookup by topic for the context-assembly layer
- Provides the keyword-to-topic mapping consumed by the intent router

Plus modifications to existing crates:

- `niles-intent` adds topic detection alongside fast-path command matching
- `niles-llm` assembles per-request system prompts from Tier A + matched Tier B
- `niles-tools` exposes `look_up_capability` and `explain_device_state`

### What this section does not specify

- The exact keyword sets per topic (will iterate from real usage)
- Whether to cache assembled prompts across similar requests
- Behavior when topic detection matches 4+ topics (probably cap at top 2-3 by relevance to avoid context bloat)
- Whether the reference format is YAML, TOML, or custom — pick whatever's easiest in Rust

## Notifications (Niles speaks unprompted)

Until now, every voice interaction has been reactive: user speaks, Niles responds. Notifications are the first case where **Niles needs to talk to the user without being asked** — a long-running task completed, a calendar event is approaching, a workflow needs attention.

This is a real subsystem with its own architectural concerns, separate from the voice request/response loop. Designing it well now means future integrations (calendar, email, GitHub, monitoring alerts, security events) all plug into the same delivery mechanism.

### Where notifications play

Niles needs to pick a satellite for each notification. Heuristics, in priority order:

1. **Originating-satellite affinity.** If the notification relates to something the user started from a specific satellite (e.g. "this Archon workflow you kicked off in the office is done"), play it on that satellite first.
2. **Last-active satellite.** Otherwise, the satellite the user most recently spoke to. Niles tracks this passively as a single mutable "last active" pointer.
3. **All satellites.** Only if the notification is genuinely urgent and the user must hear it regardless of location (rare; explicit `priority: urgent` flag).

Notifications never auto-escalate to all satellites the way timer alarms do. They're informational, not action-required. Missing one is acceptable.

### What notifications sound like

- A short, distinctive chime (different from timer alarms so users can tell them apart at a glance)
- Followed immediately by the voice message
- Voice message is concise — one or two sentences — and surfaces the most important fact first

Example: *(chime)* "The dark mode workflow on ticket0 just finished. Pull request is ready."

Not: *(chime)* "Hi Mark, I have an update for you about a coding workflow you started earlier today. The Archon workflow for the dark mode feature on the ticket0 project completed successfully and there is now a pull request available for your review at the following URL..."

The chime gives the user time to mentally orient before the content; the message respects their attention.

### Quiet hours and priority

Niles has a notion of *quiet hours* (default: 22:00 — 07:00), during which most notifications are deferred until morning. Three priority levels:

| Priority | Behavior in quiet hours |
|---|---|
| `low` | Silently queued; replayed in morning if still relevant |
| `normal` | Played at low volume; still respects routing rules |
| `urgent` | Played at normal volume; ignores quiet hours |

The integration emitting the notification picks the priority. Archon workflow completion is `normal`. Security camera motion at 03:00 would be `urgent`. Most things are `low` or `normal`.

### Recall

Notifications are not transient — Niles remembers recent ones. The user can ask:

- "What was the last notification?"
- "What did I miss this morning?"
- "Did Archon finish?"

Notifications are stored in SQLite for ~7 days, then garbage-collected. The Tier 1 LLM has a `list_recent_notifications` tool for these queries.

### Where this lives in the codebase

A new crate, `niles-notifications`:

- Exposes an internal API: `notify(message, priority, origin?, satellite_hint?)`
- Handles routing (which satellite), formatting (chime + message), persistence (SQLite)
- Consumes events from the internal event bus that other crates publish
- Provides the `list_recent_notifications` LLM tool

Integrations don't speak directly to satellites. They publish a notification event; `niles-notifications` decides how to deliver it. This separation means quiet hours, routing rules, and recall all work consistently across every integration without each one re-implementing them.

### Configuration

```toml
[notifications]
quiet_hours = { start = "22:00", end = "07:00" }
default_priority = "normal"
prefer_originating_satellite = true
retention_days = 7

[notifications.routing]
# Optional per-room overrides
"bedroom" = { quiet_after = "21:30" }  # bedroom goes quiet earlier
"office"  = { allow_priorities = ["normal", "urgent"] }
```

## External integrations

Niles's ambition is to be the voice surface for the user's whole personal infrastructure — coding tools, calendar, email, deployments, monitoring — not just lights and timers. The architecture supports this through a consistent integration pattern.

### The pattern

Each integration is its own crate (`niles-integration-<name>`) and follows the same shape:

1. **Talks to an external service** (HTTP API, webhook subscription, local socket, etc.)
2. **Exposes LLM tools** for actions the user might trigger by voice
3. **Publishes events to the internal event bus** for notifications, state updates, etc.
4. **Loads a capability reference file** so users can ask "how do I use [integration]?"
5. **Maintains its own short-lived state** (caches, subscriptions) but persistent data lives in the central Niles SQLite

Integrations are independent of each other and of the core. Adding a new integration is a self-contained piece of work that doesn't require touching anything else.

### Discovery and naming

Integrations register themselves at startup with a stable name (e.g. `archon`, `calendar`, `github`). The LLM sees them via Tier B context when relevant. Users refer to them by their everyday names ("run an Archon workflow," "check my calendar"), and the LLM maps to the right integration.

### First concrete integration: Archon

[Archon](https://github.com/coleam00/Archon) is a workflow engine for AI agents. It runs YAML-defined workflows in isolated git worktrees with fire-and-forget execution. Workflows commonly handle coding (planning, implementation, validation, PR creation), but they can do anything that can be scripted with AI involvement — video generation (Archon ships `archon-remotion-generate`), content drafting, research, data analysis, documentation updates. Whatever the user wires up as a workflow.

Perfect fit for voice triggering: kick off a long-running task, get notified when it's done.

**Why Archon specifically as the first integration:**

- It already has a platform-adapter architecture (Slack, Telegram, Discord, GitHub, Web UI). Niles becomes the voice adapter alongside these.
- Fire-and-forget execution maps naturally to voice: "start the workflow, tell me when it's done."
- Most users running Archon also self-host it, so it can live in the same cluster as Niles with clean internal networking.
- The workflows are inherently long-running (minutes to hours), which is exactly when voice triggering is more valuable than typing.
- Archon runs workflows through the user's existing Claude Code (or other AI assistant) subscription — there's no metered API cost for the actual AI work. Niles only pays Tier 1 LLM cost for parsing the trigger.

**Why this is more than "coding integration":**

The user doesn't have to know workflow names. The LLM maps natural-language intent to the right Archon workflow:

- "Create a task in ticket0: add summary generation to ticket conversations" → `archon-idea-to-pr` on `ticket0`
- "Fix issue 42 in ticket0" → `archon-fix-github-issue`
- "Review PR 47" → `archon-smart-pr-review`
- "Make a product video about the spring campaign" → `archon-remotion-generate` (or whatever video workflow is configured)
- "Research how other voice assistants handle multi-user setups, save it to my notes" → a configured research workflow

The integration shape is generic. Whatever workflows Archon has configured, Niles can trigger.

**Three interaction patterns:**

**Pattern 1 — Workflow-name driven** (user knows the workflow):
> "Niles, run the idea-to-pr workflow on ticket0 with brief: add summary generation."

Niles extracts project, workflow, brief; calls Archon; confirms.

**Pattern 2 — Intent driven** (user describes the goal, LLM picks the workflow):
> "Niles, create a task in ticket0: add summary generation to ticket conversations."

The LLM, seeing Archon's project list and workflow catalog (with descriptions) in Tier B context, maps "create a task" + "implement a feature" → `archon-idea-to-pr`. Calls Archon with the brief. This is the most common and most powerful pattern.

**Pattern 3 — Question driven** (user has a problem, not a task yet):
> "Niles, ticket0 has a bug where users see the wrong avatar."

LLM recognizes this is "problem reported, no task yet" and picks `archon-create-issue` (which investigates and files a GitHub issue), or asks: "Should I have Archon investigate and file an issue, or try to fix it directly?"

**Brief refinement (used sparingly):**

Before kicking off a workflow, Niles can ask one clarifying question if the brief is genuinely ambiguous:

> User: "Niles, create a task in ticket0: add summary generation."
> Niles: "Got it. Summaries for individual tickets, or for whole conversations?"
> User: "Whole conversations."
> Niles: "Starting the idea-to-pr workflow on ticket0: add conversation-summary generation. I'll let you know when there's a PR."

The LLM is conservative: only one clarifying question, only when genuinely ambiguous, default to letting Archon's planning step resolve ambiguity rather than asking. Niles is not a design conversation surface.

**Discoverability:**

- "Niles, what workflows do I have for ticket0?"
- "Niles, what coding projects do I have?"
- "Niles, what's Archon doing right now?"
- "Niles, cancel the dark mode workflow"

**Completion notifications:**

Unprompted notifications when workflows complete:

- *(chime)* "Your ticket0 conversation-summary workflow finished. Pull request is ready for review."
- *(chime)* "The spring campaign video just finished rendering. Output is in the notification log."

Notification content is concise. Where to find the result comes first; URLs are referenced via the notification log rather than read verbatim ("PR is ready" not "P R hash 1 2 3 slash you slash ticket zero slash...").

**Integration shape:**

The `niles-integration-archon` crate:

- Connects to Archon's HTTP API (or cluster-internal address when co-deployed)
- Subscribes to Archon's webhook events for workflow status updates
- Caches the list of projects and available workflows (including their YAML descriptions); refreshes on Archon-side changes
- Exposes LLM tools:
  - `archon_list_projects()`
  - `archon_list_workflows(project?)`
  - `archon_run_workflow(project, workflow, brief)`
  - `archon_get_status(workflow_run_id?)` — current activity, or status of a specific run
  - `archon_cancel_workflow(workflow_run_id)`
- Publishes notification events on workflow completion, failure, or human-approval-gate
- Surfaces Archon's interactive approval gates as voice prompts: "The dark mode workflow needs your approval to continue. Want me to summarize the changes?"

**Resolving natural language:**

The LLM does the work of mapping casual phrasings to specific projects and workflows. It has Archon's project list and workflow list (with `description` fields from the YAML) in Tier B context. Pattern-2 intent-driven invocations are the LLM's bread and butter — well within GPT-OSS 20B's capability, no Tier 2 needed.

**Authentication:**

For self-hosted, co-deployed Archon (e.g. both in the same k8s cluster), cluster-internal networking handles auth — no public exposure needed. For users with remote Archon instances, an API token in Niles's config provides access. Tokens never leave the central Niles service.

**Approval gates:**

Archon's workflows can include `interactive: true` nodes that pause for human input. Niles surfaces these as voice prompts via the notifications subsystem with `priority: normal`. The user can say "approve" / "reject" / "show me the diff" / "ask for more detail" — the LLM routes these back to the workflow.

This preserves the human-in-the-loop discipline Archon was designed for, even when interacting via voice.

### Future integrations (not v1)

The same pattern scales to:

- **Calendar** (Microsoft 365 / Google Calendar) — read events, schedule things, get reminders
- **Email** — summary digests, send drafts (with explicit confirmation)
- **GitHub** directly — repository status, recent activity, code review requests
- **Monitoring** — alerts from Grafana, Prometheus, Sentry surfaced as notifications
- **Deployments** — trigger or check status of CI/CD pipelines
- **Home Assistant** (ironic, but useful) — bridge to HA for users still running it for some devices, with Niles as the voice surface

Each is a self-contained crate. None require core architectural changes — that's the point of the pattern.

### What this section does not specify

- The exact protocol between Niles and Archon (HTTP+webhooks vs. WebSocket vs. NATS — pick whatever Archon exposes most cleanly)
- How to handle Archon API auth-token rotation
- Whether multiple Archon instances can be connected simultaneously (probably no for v1)
- Voice-fingerprint gating of who can trigger destructive workflows (deferred to the upcoming "user recognition" capability — see separate section)

## User recognition

Niles identifies the people in the household by voice. This enables personalization (greeting by name, routing "my X" references to the right person) and access control (rules about who can do what, covered in the next section).

Recognition is opt-in via introduction. There is no enrollment ceremony, no biometric setup wizard. The first time someone is identified, it's either because they introduced themselves or because Niles asked. Over time, their voice profile gets stronger from repeated successful matches.

### How it works technically

A small neural network (likely ECAPA-TDNN via ONNX Runtime, ~80MB) produces a fixed-length voice embedding (~192–512 floats) from a few seconds of audio. Embeddings from the same person cluster in vector space; different people separate.

Identification is a nearest-neighbor lookup against stored embeddings, using cosine similarity. Three outcomes:

- **High confidence** (similarity > 0.85): confident match → speaker identified
- **Low confidence** (< 0.65): unknown speaker
- **Ambiguous** (in between): "I think you're Mark — is that right?" before proceeding

Thresholds are tunable and may drift as more embeddings accumulate per user.

### Where it runs in the pipeline

Speaker recognition runs **in parallel with Whisper STT**, not serially. The audio is already in memory; the speaker model is small (~100–300ms inference). By the time the LLM is ready to run, the speaker ID result is available as part of the request context. This means recognition adds *zero* latency to the user-perceived response.

The speaker result is added to the Tier A context the LLM sees:

```json
{
  "speaker": { "id": "mark", "name": "Mark", "confidence": 0.94, "is_admin": true },
  ...
}
```

Or for an unknown speaker:

```json
{
  "speaker": { "id": null, "confidence": null },
  ...
}
```

### Introduction flow

Two ways someone becomes identified:

**Explicit introduction:**
- User: "Niles, I'm Mark."
- Niles: captures embeddings from this utterance, stores them under "Mark"
- Confirmation: "Got it, Mark. Good to meet you."

Repeated introductions ("hey, this is Mark") add embeddings to the existing cluster, refining the profile.

**Prompted introduction:**
- Unknown speaker makes a request that requires identity (e.g. "what's on my calendar?")
- Niles: "I don't recognize your voice — who am I speaking to?"
- User: "Mark."
- Niles: captures embeddings, stores, completes the original request

For requests that don't require identity (turn on a light, set a timer), Niles does not prompt — it just executes. Friction is proportional to need.

### Online refinement

When a speaker matches with high confidence, the new utterance's embedding is added to their cluster (tagged `source: high_confidence_match`). The profile improves over time, capturing voice variation across morning grogginess, illness, different rooms, distance from the satellite.

If a user explicitly corrects ("Niles, I'm Majse, not Mark"), the most recent embedding is reassigned to the correct user and the model effectively un-learns the mistake.

### Personal references, not preferences

Niles does *not* maintain per-user preferences for shared home state. The lighting curve, sunset time, scenes, and ambient settings are properties of the house, not of any individual. Multiple people share one house.

What Niles does maintain per user is **identity-routed references** to external resources:

```
users
  id (uuid)
  name (display name)
  is_admin (boolean)
  calendar_id (which calendar maps to this user, optional)
  github_username (optional)
  archon_user (optional)
  bedroom (which room is this user's bedroom, for "wake me up" routing)
  created_at
```

The LLM uses these when a personal reference appears:

- "What's on my calendar?" → looks up `speaker.calendar_id`, queries that calendar
- "Wake me up at 06:30 tomorrow" → looks up `speaker.bedroom`, creates a routine targeting that room
- "Did Archon finish my workflow?" → looks up `speaker.archon_user`, queries Archon scoped to that user

If the speaker is unknown and a personal reference is used, Niles prompts for identification before proceeding.

### Privacy posture

Voice fingerprints are biometric data. Niles is deliberate about handling:

- **Stored locally only.** Embeddings live in Niles's SQLite. They are never sent to Groq, Anthropic, or any cloud provider. Whisper receives audio for transcription, but Whisper does not do speaker ID.
- **Easy to delete.** "Niles, forget me" wipes that user's embeddings and identity entirely.
- **Opt-in.** Niles works for anonymous users — they just don't get personalized features.
- **No silent identification.** Niles only attempts to identify people who have been introduced. Random visitors who never give a name remain "unknown speaker."

### LLM tools

- `introduce_speaker(name)` — captures the current speaker's voice under this name
- `list_known_speakers()` — enumerates known users
- `forget_speaker(name)` — admin-only; deletes a user and all their embeddings
- `correct_speaker_identity(actual_name)` — reassigns the most recent match

### Where this lives in the codebase

A new crate, `niles-recognition`:

- Loads the speaker embedding model (ONNX Runtime)
- Exposes `identify(audio) → SpeakerResult`
- Maintains the embedding database
- Runs in parallel with STT in the request pipeline
- Provides the introduction and management tools

## Permissions and admin

Niles supports a simple rules system that lets admins control who can do what. Rules are created in natural language by an admin and enforced before tool execution.

This complements user recognition: recognition answers "who is speaking," permissions answer "what are they allowed to do."

### The admin concept

Users have an `is_admin` flag. Only admins can:

- Create, modify, or delete rules
- Add or remove other users
- Promote another user to admin
- Forget a user (delete their identity)

**Bootstrap:** the first user introduced on a fresh install becomes admin by default. "Niles, I'm Mark" on day one → Mark is admin. Subsequent users default to non-admin and must be promoted by an existing admin.

### Rule shape

Rules have a structured form, but are created and managed in natural language:

```yaml
rule:
  id: uuid
  natural_description: "Only Mark or Majse can change the temperature"
  scope:
    tools: ["set_temperature", "set_thermostat"]
    or: device_categories: ["thermostat"]
  allowed_users: ["mark", "majse"]
  denied_users: null  # allow-list form
  created_by: "mark"
  created_at: ...
```

Or for deny-list form:

```yaml
rule:
  natural_description: "The kids can't change the lights in our bedroom"
  scope:
    tools: ["set_device"]
    devices: ["bedroom/*"]
  allowed_users: null
  denied_users: ["emma", "lucas"]
```

Allow-list ("only these people can") is the more secure default. Deny-list is for "everyone except these specific people."

### Natural-language rule lifecycle

The LLM translates between natural language and the structured form:

- **Create:** "Only Mark or Majse can change the temperature" → admin LLM extracts the rule, stores it, confirms back in natural language
- **List:** "What rules are active?" → reads them back as their `natural_description` strings
- **Delete:** "Remove the temperature rule" → matches by description, confirms before deletion
- **Modify:** "Also let Emma change the temperature" → finds the rule, adds Emma to the allow-list, confirms

All of these are admin-only. Non-admins attempting them get refused with an explanation.

### Enforcement

Every tool execution passes through the rules engine before running:

1. LLM decides to call a tool with specific arguments
2. The permissions check evaluates all rules against `(speaker, tool, arguments)`
3. If any rule denies the action: tool fails with `permission_denied`, LLM is told and apologizes/explains
4. If allowed: tool executes normally
5. If the speaker is unknown and the tool has any active rules: prompt for identification first

The check is in-memory rule evaluation; no measurable latency added.

### Conflict resolution

Two rules can overlap. The resolution principle: **most-specific rule wins.**

- "Only Mark can change the lights" (whole-house) vs.
- "Majse can change the bedroom lights" (scoped to bedroom)

The bedroom-scoped rule wins for bedroom interactions; the whole-house rule applies everywhere else. Specificity is measured by scope size — narrower scopes beat broader ones.

When the LLM detects ambiguity at rule creation time, it asks the admin to clarify.

### Sensible defaults

Niles ships with a small set of default rules that any admin can override:

| Default rule | Rationale |
|---|---|
| Unknown speakers cannot run Archon workflows | Destructive; user must be known |
| Unknown speakers cannot access personal data ("my X") | No identity to route to |
| Unknown speakers cannot modify rules | Obvious |
| Unknown speakers cannot add or remove users | Obvious |
| Only admins can change the lighting curve | Affects the whole house long-term |
| Only admins can set sunset time | Affects the whole house long-term |

These defaults can be loosened or tightened. For example, a household that explicitly wants guests to be able to start music can override "unknown speakers cannot control music."

### LLM tools

Admin-only:

- `create_rule(natural_description)` — stores a structured rule extracted from description
- `delete_rule(rule_id_or_description)`
- `modify_rule(rule_id_or_description, change)`
- `make_admin(user_name)`
- `revoke_admin(user_name)`

Available to anyone:

- `list_rules()` — shows current rules in natural language

### Where this lives in the codebase

A new crate, `niles-permissions`:

- Stores rules in SQLite
- Evaluates `(speaker, tool, args) → allow | deny | requires_identity`
- Hooks into the tool dispatch path; every tool call goes through it
- Provides the rule-management LLM tools (admin-gated)

The split between `niles-recognition` (who) and `niles-permissions` (what they can do) keeps each crate small and testable.

### What these sections do not specify

- Specific embedding model and ONNX export procedure (resolves in Phase 9)
- Threshold tuning approach (probably start with conservative defaults, adjust from real-world data)
- Whether rule descriptions should support more complex conditions (time-of-day, "only after 18:00", etc.) — initially no, simple `(users, scope)` only; add complexity if real households need it
- Whether the satellites themselves can do speaker ID locally (faster, more private) vs. server-side (more accurate, simpler) — server-side initially, satellite-side as a future optimization

## Presence

Niles tracks whether the home is occupied and which household members are present. Presence is the trigger for a lot of useful behavior: turning on the hallway lights when someone arrives, switching off ambient music when everyone leaves, suppressing notifications when nobody's home.

Presence is different from device control. Devices are concrete and live in rooms; people move around. The room/device naming convention doesn't fit, and presence sources vary wildly in shape and accuracy. So presence needs its own subsystem with structured config.

### Sources

Niles supports multiple presence sources, each implemented as a small adapter. Users enable the ones that fit their setup:

| Source | How it works | Accuracy | Notes |
|---|---|---|---|
| Tado | Geofence around home coordinates | Good | API token required; reuses existing Tado setup |
| Phone (Home Assistant Companion, OwnTracks) | Phone reports GPS to a webhook | Good | Requires app on each phone |
| BLE beacons | Phone or wearable detected by stationary BLE receivers | Excellent (room-level) | Hardware investment |
| Router-based | Device on WiFi = person home | Mediocre | Easy but unreliable for sleeping phones |
| Motion sensor inference | "Recent motion in the home" as a proxy | Mediocre | Free if motion sensors already exist |
| Manual ("Niles, I'm leaving") | Voice command | Always correct when used | Useful supplement, not primary |

Multiple sources can be combined per user. Niles aggregates: if any source says "home," the user is home; "away" only when all sources agree.

### Home-state aggregation

From individual user presence, Niles derives the home-level state:

- **occupied** — at least one known user is home
- **unoccupied** — all known users are away
- **transitioning** — within hysteresis window after a state change

Hysteresis matters. If someone drives past the geofence edge, presence shouldn't flicker on/off. Default: 60 seconds of consistent state required before declaring a change. Tunable per source.

State transitions publish events on the internal event bus:

- `presence.user_arrived(user_id)`
- `presence.user_left(user_id)`
- `presence.home_became_occupied`
- `presence.home_became_unoccupied`

Automations and integrations subscribe to these events.

### Config shape

```toml
[presence]
home_latitude = 56.1572
home_longitude = 10.2107
geofence_radius_meters = 150
hysteresis_seconds = 60

[[presence.users]]
name = "mark"
sources = ["tado:mark", "phone:mark_iphone"]

[[presence.users]]
name = "majse"
sources = ["tado:majse", "phone:majse_iphone"]

[presence.sources.tado]
home_id = 12345
api_token_env = "TADO_API_TOKEN"

[presence.sources.phone]
webhook_path = "/presence/phone"
allowed_devices = ["mark_iphone", "majse_iphone"]
```

API tokens are referenced by env var name rather than embedded. The deployment layer (Compose `.env` or k8s `Secret`) provides the actual values.

### Where this lives in the codebase

A new crate, `niles-presence`:

- Loads source configuration at startup
- Each source is a small adapter (Tado HTTP client, phone webhook listener, etc.)
- Aggregates per-user state from sources with hysteresis
- Maintains home-level state
- Publishes presence events to the internal event bus
- Exposes LLM tools: `who_is_home`, `is_someone_home`, `mark_user_left(user)`, `mark_user_arrived(user)`

### LLM tools and voice queries

Users can ask Niles about presence:

- "Niles, who's home?"
- "Niles, is Majse home?"
- "Niles, I'm leaving" → manually marks the speaker as away

The manual override is useful when sources lag or fail. It's a soft override — sources continue to update, and if they consistently disagree, Niles reverts to source-derived state after a configurable interval.

## Automations

Several places in Niles benefit from "when X happens, do Y" logic: presence-triggered lights, sunset-triggered porch lamps, "if Majse opens the office door after 22:00 dim the lights." These are *automations* — a first-class concept in Niles, not bolted on later.

### Two shapes

Automations come in two forms:

**Code automations** — complex, foundational behaviors implemented as Rust. Examples: the ambient lighting curve, the morning routine, timer escalation. These are too rich (state, hysteresis, interactions with other subsystems) for declarative config and are part of the core.

**Config automations** — simple "when X, do Y" rules expressed as structured config. Examples: "when home becomes occupied and the sun is below the horizon, turn on the hallway lights." These cover the long tail of user-specific behaviors.

The split keeps complex behavior testable in code while letting users define their own automations without writing Rust.

### Config automation shape

```toml
[[automations]]
name = "hallway light on arrival"
when = "presence.home_became_occupied"
conditions = ["sun.is_dark"]
do = [
  { tool = "set_device", args = { device = "hallway/ceiling_light", state = "on" } }
]

[[automations]]
name = "porch on at sunset"
when = "schedule.sunset"
do = [
  { tool = "set_device", args = { device = "outdoor/porch_light", state = "on" } }
]

[[automations]]
name = "everyone gone, music off"
when = "presence.home_became_unoccupied"
do = [
  { tool = "control_room_speaker", args = { room = "*", action = "stop" } }
]
```

The format is intentionally narrow: triggers are well-known event names, conditions are a small set of allowed predicates (sun state, time-of-day, user identity), actions are tool calls. No general scripting language — that would be a security and complexity disaster. The set of allowed conditions and triggers expands over time as new ones are needed.

### Voice-created automations

This is where AI-first shines. Users create automations by speaking them:

> User: "Niles, when we get home and it's dark, turn on the hallway light."

The LLM (admin-only, since automations affect everyone) extracts:
- trigger: `presence.home_became_occupied`
- condition: `sun.is_dark`
- action: `set_device("hallway/ceiling_light", { state: "on" })`

Stores it as a new automation, confirms back in natural language. The structured form exists because the runtime needs it; the user never edits it.

Voice-created automations are stored in SQLite, alongside config-defined ones. Both are evaluated identically at runtime.

### Automation lifecycle

- **List:** "Niles, what automations do I have?" → reads them back as their natural-language descriptions
- **Disable temporarily:** "Niles, disable the hallway automation for tonight" → marks it skipped until tomorrow
- **Delete:** "Niles, remove the hallway light automation" → matches by description, confirms, deletes

Same lifecycle pattern as scenes, rules, and timers. Consistent across the system.

### Conflicts and ordering

Automations don't pre-empt manual control. If you've put a light into manual mode, an automation that would change it skips that light. This is the same rule as lighting curve manual mode — manual user intent always wins.

When multiple automations fire on the same event, they execute in declaration order. Most automations don't conflict in practice (different rooms, different devices). If two genuinely conflict, the last-declared wins for that device — which the user can see and reason about by listing automations.

### Where this lives in the codebase

A new crate, `niles-automations`:

- Loads config-defined automations at startup
- Loads voice-defined automations from SQLite at startup
- Subscribes to the internal event bus
- Evaluates conditions against current state
- Dispatches actions through the tool system (which respects permissions, manual mode, etc.)
- Provides LLM tools for the voice-creation lifecycle

### Permissions interaction

Voice-created automations are admin-only (they affect everyone). Listing and disabling for a single day can be done by anyone. Deletion of a voice-created automation is admin-only. Config-defined automations (in the TOML) are deployed by whoever deployed Niles — out-of-band from runtime permissions.

## Deployment

Niles supports two deployment paths as first-class options: Docker Compose and Kubernetes. Most users will choose Compose; Kubernetes is for users running multi-node clusters.

Both paths consume the same canonical Niles config (TOML), the same API keys (env vars), and produce identical runtime behavior. The Niles service has no knowledge of which deployment style launched it.

### Docker Compose (default path)

For users with a single host running Docker. The repo includes a reference `docker-compose.yml`:

```yaml
services:
  mosquitto:
    image: eclipse-mosquitto:2
    volumes:
      - ./mosquitto-data:/mosquitto/data
      - ./mosquitto.conf:/mosquitto/config/mosquitto.conf:ro
    restart: unless-stopped

  zigbee2mqtt:
    image: koenkk/zigbee2mqtt:latest
    volumes:
      - ./z2m-data:/app/data
    environment:
      TZ: Europe/Copenhagen
    devices:
      # SLZB-06MU at network address; no host device passthrough needed
    restart: unless-stopped

  piper-tts:
    image: rhasspy/wyoming-piper:latest
    command: --voice en_US-amy-medium
    restart: unless-stopped

  niles:
    image: ghcr.io/<org>/niles:latest
    volumes:
      - ./niles-config.toml:/etc/niles/config.toml:ro
      - ./niles-data:/var/lib/niles
    environment:
      GROQ_API_KEY: ${GROQ_API_KEY}
      ANTHROPIC_API_KEY: ${ANTHROPIC_API_KEY}
      TADO_API_TOKEN: ${TADO_API_TOKEN}
    ports:
      - "10300:10300"  # Wyoming protocol
      - "8080:8080"    # HTTP API
    depends_on:
      - mosquitto
      - piper-tts
    restart: unless-stopped
```

Plus a `.env` file (gitignored) with the actual secret values:

```
GROQ_API_KEY=gsk_...
ANTHROPIC_API_KEY=sk-ant-...
TADO_API_TOKEN=...
```

Setup is: clone the repo, edit `niles-config.toml` with home coordinates and feature toggles, create `.env` with API keys, `docker compose up -d`. Done.

The repo's `deploy/compose/` directory holds the reference Compose file, example config, and a `.env.example` users copy.

### Kubernetes

For users running a cluster. The repo includes a Kustomize base in `deploy/kubernetes/base/`:

- `mosquitto/` — Deployment + Service + PVC
- `zigbee2mqtt/` — Deployment + Service + PVC
- `piper-tts/` — Deployment + Service
- `niles/` — Deployment + Service + PVC, references a `niles-config` ConfigMap and a `niles-secrets` Secret

Users create overlays under `deploy/kubernetes/overlays/<their-home>/` that patch the base with their values:

```yaml
# niles-config.toml mounted from a ConfigMap
apiVersion: v1
kind: ConfigMap
metadata:
  name: niles-config
data:
  config.toml: |
    [presence]
    home_latitude = 56.1572
    home_longitude = 10.2107
    # ... etc
---
apiVersion: v1
kind: Secret
metadata:
  name: niles-secrets
stringData:
  GROQ_API_KEY: gsk_...
  ANTHROPIC_API_KEY: sk-ant-...
  TADO_API_TOKEN: ...
```

The Niles Deployment references both. Setup is: fork the repo (or clone and create an overlay), edit values, `kubectl apply -k deploy/kubernetes/overlays/<your-home>`. Same config content as the Compose path, different delivery mechanism.

### What lives where

The Niles config TOML is the canonical configuration. Everything user-tunable lives there:

- Presence (sources, geofence, users)
- Lighting curve (sunrise/sunset times, night floor, color temp anchors)
- Feature toggles (which modules are enabled)
- Integration endpoints (Archon URL, Sonos discovery hint, etc.)
- Notification preferences (quiet hours, routing)
- Automations defined at deploy time

Secrets (API tokens) live in env vars referenced by the config. Deployment-specific delivery: `.env` for Compose, `Secret` for k8s.

Runtime state (scenes, voice-created automations, timers, user embeddings, permission rules, notification history) lives in Niles's SQLite, in a persistent volume.

### Choosing between Compose and Kubernetes

The README should be opinionated:

- **Default recommendation: Docker Compose.** Single host, simpler, fits most home setups.
- **Use Kubernetes if:** you already have a cluster, want multi-node resilience, or specifically prefer the k8s workflow.

Both are equally well-supported. The repo has CI for both. Neither is "the legacy option."

## LLM-facing documentation

Niles is open source. New users discover it, want to know if it fits their home, and need to walk through setup decisions. The traditional path is: read the README, scan the wiki, ask in Discord, hope for the best.

Niles takes a different approach: a structured capability manifest that LLMs read and use to guide users through setup conversationally.

### The capability manifest

A file at the repo root — `MANIFEST.md` (or potentially `.well-known/llms.txt` if the proposed standard gains traction) — describes every feature, requirement, and setup decision in a format optimized for LLMs to read and reason about.

This is *not* a README. The regular `README.md` exists for humans, with marketing-style intro, quick-start, links. `MANIFEST.md` is a different artifact with different goals:

- **Completeness over brevity.** Every feature, every requirement, every option — the LLM surfaces only what's relevant to the asker.
- **Structured over prosey.** Tables and YAML-like blocks. The LLM parses these reliably and uses them as ground truth.
- **Decision-tree friendly.** Each feature has explicit `requires`, `optional`, `default` flags so the LLM can walk a user through choices.
- **No hype.** The LLM doesn't need to be sold; it needs facts.
- **Self-describing setup.** Given the manifest, an LLM can compose a Niles config TOML stub for a user based on their answers.

### Usage pattern

A user discovers Niles, copies the URL of `MANIFEST.md`, pastes it into Claude/ChatGPT/their AI of choice, and says "I have a 3-bedroom apartment with Philips Hue and a Sonos; what would Niles give me, and how do I set it up?"

The AI:

1. Reads the manifest (which the user provided as context)
2. Identifies relevant features (lighting, scenes, music control)
3. Asks clarifying questions (do they have a voice satellite? Tado? Groq account?)
4. Generates a tailored Niles config TOML and Docker Compose file
5. Walks the user through deploying it

No human handholding required. The friction from "interested" to "running" drops significantly.

### Single source of truth

The manifest is generated from a canonical `features.toml` in the repo. The same file:

1. Is read by the Niles service at startup to know which features exist and what their config schema looks like
2. Is the source for the generated `MANIFEST.md` (via a build script)
3. Is the source for the human `README.md`'s feature table
4. Is the source for the mdBook docs site's per-feature pages

Documentation cannot drift from runtime behavior because they share a source. Updating a feature means updating `features.toml`; everything regenerates.

### Manifest format sketch

```markdown
# Niles — Capability Manifest

This document describes Niles in a format optimized for LLMs.
Humans should read README.md instead.

## Identity
- name: niles
- license: MIT
- runtime: Rust on Linux
- deployment: Docker Compose (default) or Kubernetes

## Core Requirements

- A Linux host (or k8s cluster) running Docker or k8s
- A Zigbee2MQTT instance with devices named per `<room>/<device>` convention
- An MQTT broker (Mosquitto bundled)
- At least one ESPHome voice satellite
- Groq API key for STT and Tier 1 LLM
  - Alternative: self-hosted Whisper + local LLM (requires GPU)

## Feature Modules

### lighting
  description: Always-on circadian lighting with morning/sunset ramps
  requires: at least one dimmable Zigbee light
  default: enabled

### scenes
  description: Save/recall named light configurations by voice
  requires: lighting module
  default: enabled

### presence
  description: Track home/away state, fire arrival/departure events
  requires_one_of: [tado, phone_app, ble_beacons, manual_only]
  api_keys: { tado: required if using Tado source }
  default: disabled

### archon_integration
  description: Trigger Archon workflows (coding, content, video, research) by voice
  requires: running Archon instance
  api_keys: { archon: required if remote }
  default: disabled

# ... etc, every feature
```

### Where this lives in the codebase

The canonical `features.toml` lives at the repo root. A small build script under `scripts/generate-manifest.sh` produces `MANIFEST.md` from it. CI ensures the generated manifest is committed and up to date.

The `niles-config` crate reads `features.toml` at startup so the service knows what features exist and what their config schemas are. Unknown features in the user's config produce clear errors; missing required config for an enabled feature produces clear errors.

### Why this matters for the project

For an open-source project replacing something as broad as Home Assistant, discoverability and adoption-friction matter enormously. Most users won't read a 500-page wiki. They will paste a URL into their AI assistant and ask. Designing for that pattern is genuinely competitive — it dramatically lowers the barrier to evaluating and trying Niles.

## Repository layout (monorepo)

Niles is developed as a monorepo. Everything that needs to evolve together lives in one place: backend, firmware, deployment manifests, schemas, and docs. A single commit can atomically update Rust types, regenerate frontend bindings, bump firmware config schema, and update the deployment manifest.

```
niles/
├── README.md                  # human-facing intro and quick-start
├── MANIFEST.md                # LLM-facing capability manifest (generated)
├── features.toml              # canonical feature catalog (source for MANIFEST.md and runtime)
├── LICENSE
├── CONTRIBUTING.md
├── ARCHITECTURE.md
├── .github/workflows/         # CI: rust, firmware, docs, manifest-regen, release
│
├── crates/                    # Cargo workspace
│   ├── niles-core/             # event bus, registry, types
│   ├── niles-wyoming/          # satellite protocol server
│   ├── niles-mqtt/             # MQTT + Z2M device source
│   ├── niles-stt/              # STT trait + providers
│   ├── niles-llm/              # LLM trait + providers
│   ├── niles-tts/              # TTS trait + providers
│   ├── niles-speakers/         # room speaker trait + Sonos impl
│   ├── niles-intent/           # Tier 0 regex router
│   ├── niles-tools/            # tool definitions for LLMs
│   ├── niles-capabilities/     # capability reference files for self-documentation
│   ├── niles-scheduler/        # time-driven behaviors (lighting curve, routines, timers)
│   ├── niles-notifications/    # unprompted speech: routing, chimes, quiet hours, recall
│   ├── niles-integration-archon/ # Archon workflow engine integration (first integration)
│   ├── niles-recognition/      # speaker identification via voice embeddings
│   ├── niles-permissions/      # rules engine and admin concept; enforces tool access
│   ├── niles-presence/         # presence sources, aggregation, home-state events
│   ├── niles-automations/      # when-X-do-Y rules, voice-creatable and config-defined
│   ├── niles-api/              # HTTP/WebSocket API
│   ├── niles-config/           # config loading and validation
│   └── niles-bin/              # main binary, wiring (produces `niles`)
│
├── firmware/
│   ├── esphome/               # reference ESPHome satellite config
│   └── esp-rs/                # future: native Rust firmware
│
├── frontends/                 # added when needed
│   ├── desktop/               # future Tauri app
│   └── mobile/                # future Tauri Mobile or RN
│
├── deploy/
│   ├── compose/               # docker-compose.yml + .env.example (default path)
│   ├── kubernetes/            # Kustomize manifests (for multi-node setups)
│   │   ├── base/              # mosquitto, z2m, piper-tts, niles
│   │   └── overlays/          # example-home, dev
│   └── docker/                # multi-stage Dockerfile (used by both paths)
│
├── docs/                      # mdBook source
│   └── src/                   # architecture, hardware, install, dev, reference
│
├── schemas/                   # shared, language-agnostic schemas
│   ├── wyoming-events.json
│   ├── tool-definitions.json
│   └── device-types.json
│
├── examples/                  # example tools, configs, integrations
├── scripts/                   # dev-up, flash-satellite, migrate-from-ha
│
├── Cargo.toml                 # workspace root
├── rust-toolchain.toml
└── .rustfmt.toml
```

### Single binary, subcommands

`niles-bin` produces one binary named `niles` with subcommands:

```bash
niles serve              # main service
niles migrate-from-ha    # one-shot migration helper
niles flash-satellite    # firmware flashing helper
niles config validate    # config sanity check
niles tools list         # show registered tools
```

Rationale: single binary, single Docker image, single thing to install and update. If any subcommand grows enough to warrant its own crate, the binary still wraps it.

### Why monorepo, not multi-repo

The project has several languages and artifact types that must stay in sync: Rust backend, ESPHome firmware, future frontends, Kubernetes manifests, JSON schemas, and docs. Splitting these into separate repos creates constant version-coordination overhead. A monorepo lets a single PR update everything atomically.

The classic monorepo objection is build performance, which is a problem at Google scale, not at this project's scale. Cargo workspaces handle Rust beautifully, and the other languages are small enough that their builds are fine standalone.

If a crate ever outgrows the monorepo, `git filter-repo --subdirectory-filter crates/niles-X` extracts it cleanly with full history.

### Core trait for device extensibility

```rust
#[async_trait]
trait DeviceSource: Send + Sync {
    fn name(&self) -> &str;
    async fn discover(&self) -> Result<Vec<Device>>;
    async fn subscribe(&self, tx: EventSender) -> Result<()>;
    async fn set_state(&self, device_id: &str, state: DeviceState) -> Result<()>;
}
```

Each source produces devices whose names follow the `<room>/<device>` convention (see "Device naming convention" above). Internally Niles prefixes names with the source identifier (`z2m:`, `shelly:`, etc.) to avoid collisions; users and the LLM see the unprefixed form.

First implementation is Zigbee2MQTT. Later additions: Shelly (local HTTP), Matter, Z-Wave, vendor cloud APIs as needed.

### Core trait for speakers

```rust
#[async_trait]
trait RoomSpeaker: Send + Sync {
    async fn play_url(&self, url: &str) -> Result<()>;
    async fn duck(&self, level: u8) -> Result<()>;
    async fn restore(&self) -> Result<()>;
    async fn current_state(&self) -> Result<SpeakerState>;
}
```

First implementation is Sonos via SOAP/UPnP. Later: AirPlay, Snapcast, generic UPnP.

### Tools exposed to the LLM

This is where home logic lives. Tools should be small, composable, and well-documented (the LLM reads the descriptions).

- `get_device_state(device_id)` — read current state
- `set_device(device_id, state)` — change a device
- `list_devices_in_room(room)` — discover what's available
- `control_room_speaker(room, action)` — play/pause/skip/volume/set source
- `set_reminder(time, message)` — schedule something
- `search_calendar(query)` — calendar integration (Outlook, Google, etc.)
- `search_events(query, time_range)` — query event log (SQL)
- `get_sensor_value(sensor_id)` — temperature, humidity, etc.
- `escalate_to_smart_model(reason)` — hand off to Tier 2

## Satellite firmware

Default firmware: **ESPHome + Wyoming protocol**. Already exists, community-maintained for the XVF3800 board, speaks to the Niles service over TCP.

Configuration (per satellite, set via ESPHome YAML or runtime config):

```yaml
niles:
  room: "kitchen"
  wake_word: "niles"          # configurable, supports custom microWakeWord models
  server: "niles.local:10300"
  satellite_id: "kitchen-01"
```

**Future option:** custom Rust firmware via `esp-hal` / `embassy` for users who want the satellite written in the same language as the backend. ESPHome is the recommended starting point for everyone.

## Wake word

- microWakeWord runs on the ESP32-S3, fully local, ~50ms detection
- Default models shipped: a few common options (a project-themed name, plus generic options like "computer", "assistant")
- Users can train custom models via microWakeWord's tooling and drop them into the firmware config
- Multiple wake words can be active simultaneously per satellite

## Build phases

### Phase 0: Hardware prep (1 evening)
- Order one reSpeaker XVF3800 with Case + XIAO ESP32-S3 (Seeed p-6628)
- Order SMLIGHT SLZB-06MU
- Confirm EU shipping to your address

### Phase 1: Infrastructure (1 weekend)
- Deploy Mosquitto with persistent volume for retained messages (Docker Compose or Kubernetes)
- Deploy Zigbee2MQTT pointing at the SLZB-06MU's IP
- Migrate Zigbee devices (re-pair if needed)
- **Rename all devices in Z2M to follow the `<room>/<device>` convention** (this is what Niles will auto-discover in Phase 2)
- Verify devices report and respond via MQTT
- Decommission Home Assistant

### Phase 2: Rust backend skeleton (1–2 weekends)
- Cargo workspace with the crate structure above
- Event bus and device registry in `niles-core`
- Z2M MQTT subscriber that parses `<room>/<device>` names and auto-populates the registry
- HTTP API exposing device list and state (read-only is fine for now)
- Verify: "I can read sensor values and toggle lights via curl, and the room/device structure matches what's in Z2M without any extra config"

### Phase 3: Tier 0 + Tier 1 voice loop (1–2 weekends)
- Wyoming protocol server in `niles-wyoming`
- Flash the satellite with ESPHome + Wyoming firmware
- Groq Whisper streaming integration
- Regex intent router with 10–20 common commands
- Piper TTS pod on k8s
- Verify: "I can say a command and it executes quickly"

### Phase 4: LLM tier + timers + capability reference (2 weekends)
- Groq client with tool calling
- `niles-capabilities` crate loading reference files at startup
- Tiered context architecture: Tier A always, Tier B on-demand via topic detection, Tier C for overviews
- Topic detection added to `niles-intent` (keyword sets per topic)
- `look_up_capability` tool for LLM-requested expansion
- `explain_device_state` tool for state questions
- Define core LLM tools (get/set device, list, speaker basics)
- Fallback from Tier 0 to Tier 1
- `niles-scheduler` crate scaffolding (will be expanded for lighting in Phase 6)
- Timers: set via Tier 0 fast-path, query/cancel via Tier 1 LLM tools
- Two-stage alarm escalation (originating satellite → all satellites after 10s)
- Pre-recorded alarm audio on satellites
- SQLite persistence so timers survive service restarts
- Verify: common commands stay <800ms; how-to questions answer correctly; timers reliable

### Phase 5: Room speaker integration and music (1–2 weekends)
- SOAP/UPnP client for local Sonos control
- Sonos as both an LLM tool AND a ducking target during voice responses
- Multi-room awareness ("play music in living room")
- Music intent tools: `play_radio`, `play_music`, `play_podcast`, `resume_in_room`, transport controls, grouping
- Per-room music state in SQLite (last source, last content) including polling for app-initiated playback
- Tier 0 fast-paths for common music commands ("play TuneIn", "pause", "louder", etc.)
- Graceful degradation in rooms without a Sonos
- Verify: "play TuneIn" resumes the last station in current room; "play my Discover Weekly" routes to the speaker's Spotify account once recognition is active

### Phase 6: Ambient lighting + scenes (2 weekends)
- `niles-scheduler` crate with daily curve computation at midnight
- Universal curve (night floor, morning ramp, daytime, sunset ramp) implemented
- Morning routine with day-pattern config and skip-if-already-on logic
- Manual mode detection (brightness, color temp, manual-off-during-ramp cancellation)
- Two-click escalation for manual turn-ons
- Scenes: save / apply / list / delete / update / exit, with room and whole-home scopes
- Voice tools for the LLM: `skip_morning_routine_tomorrow`, `set_sunset_time`, scene tools
- Tier 0 fast-path patterns for common scene phrasings and "back to normal"
- Verify: full lighting model works correctly across a real week with weekday/weekend patterns; scenes save and recall reliably

### Phase 7: User recognition and permissions (1–2 weekends)
- `niles-recognition` crate with ECAPA-TDNN (or similar) via ONNX Runtime
- Parallel-with-STT speaker identification in the request pipeline
- Introduction flow (explicit and prompted)
- Online refinement on high-confidence matches
- `niles-permissions` crate with rule storage, evaluation, admin concept
- Default rules (unknown-speaker restrictions, admin-only operations)
- Natural-language rule lifecycle via LLM
- Tier A context updated to include speaker identity and admin flag
- Verify: two household members can be recognized reliably, rules enforce correctly, unknown speakers prompted only when needed

### Phase 8: Notifications subsystem (1 weekend)
- `niles-notifications` crate: routing rules, chime + voice formatting, SQLite persistence
- Quiet hours config with priority-aware handling
- "Last active satellite" tracking in `niles-core`
- `list_recent_notifications` LLM tool
- Verify: programmatic test notifications route correctly across rooms and respect quiet hours

### Phase 9: Presence and automations (2 weekends)
- `niles-presence` crate with adapter pattern for sources
- First adapter: Tado (HTTP API client, geofence-derived per-user state)
- Second adapter: manual ("Niles, I'm leaving") as supplement
- Home-state aggregation with hysteresis; presence events published to event bus
- `niles-automations` crate: config-defined automation loader, event subscription, condition evaluation, action dispatch
- Voice-creatable automations: LLM extracts trigger/condition/action from natural language, stores in SQLite
- Admin-only creation; anyone can list and temporarily disable
- Concrete first automation: "when home becomes occupied and it's dark, turn on hallway lights"
- Verify: arrival lights work reliably; voice-created automation persists and fires correctly

### Phase 10: First external integration — Archon (1–2 weekends)
- `niles-integration-archon` crate
- Connect to Archon's HTTP API + webhook events (cluster-internal where possible)
- Cache projects and workflows; refresh on Archon-side changes
- LLM tools: list projects, list workflows, run workflow, get status, cancel
- Workflow-completion events flow through `niles-notifications`
- Approval-gate handling: surface Archon interactive nodes as voice prompts
- Capability reference for Archon, loaded into Tier B
- Default rule: only admins can run destructive Archon workflows
- Verify: full voice loop for kicking off and being notified about workflows on a real personal project

### Phase 11: LLM-facing documentation and deployment polish (1 weekend)
- `features.toml` canonical feature catalog at repo root
- Build script to generate `MANIFEST.md` from `features.toml`
- CI check that MANIFEST.md is up to date with features.toml
- Reference `docker-compose.yml` in `deploy/compose/` with `.env.example`
- Reference Kustomize base in `deploy/kubernetes/base/` with ConfigMap and Secret patterns
- Documentation pass: README aimed at humans, MANIFEST aimed at LLMs
- Verify: a fresh user can paste MANIFEST.md into an AI assistant and get a working Niles setup walkthrough

### Phase 12: Polish (ongoing)
- Order remaining satellites once one room is proven
- Add Tier 2 escalation
- Event log + SQL search tool
- Conversation memory (short-term in context, long-term in SQLite)

### Phase 13: Additional integrations and extensions
- Additional presence sources (phone-based, BLE)
- Calendar (Microsoft 365 / Google Calendar)
- Email integration
- GitHub direct integration
- Monitoring/alerting bridge
- RAG layer for documents/notes
- Frontends (Tauri 2 covers desktop + mobile from one codebase)
- Custom Rust firmware on the satellites
- More device sources (Shelly, Matter, Z-Wave)
- GPU node for fully-local STT/LLM
- Satellite-side speaker ID (faster, more private)

## What Niles is not

To keep scope honest:

- **Not a Home Assistant replacement for everyone.** If your home has lots of vendor-specific WiFi devices, complex Z-Wave/Matter setups, or you want a polished UI for non-technical family members, stay on HA.
- **Not a UI-driven system in v1.** Configuration is code, config files, and naming conventions in upstream sources. Adding a device means pairing and naming it correctly in Z2M — not clicking through Niles screens. If a UI is added later, it will be optional and used for monitoring/debugging, not required for setup.
- **Not a design or planning surface.** Niles is voice. Voice is a poor medium for design discussions, code review, long-form planning, anything that benefits from reading, scanning, jumping between sections, or seeing code and structured content. That work belongs at a computer (with Claude Code, a real editor, etc.). Niles's role in project work is *triggering* (kick off an Archon workflow, file a task) and *being notified* (workflow finished, PR ready) — not thinking through the work itself.
- **Not a voice OS.** Niles is the brain; satellites are dedicated hardware that talk to it. It is not a Linux audio stack you bolt onto existing speakers (though that's a reasonable future contribution).
- **Not an LLM training framework.** Niles uses LLMs; it doesn't train them.

## Open source considerations

- **License:** MIT or Apache 2.0 recommended for maximum adoption. Avoid GPL for the core service since users embed it in their homes and may want to mix proprietary integrations.
- **Repository structure:** monorepo with the Cargo workspace as the root; separate repos for firmware images and example configs if they grow.
- **CI:** GitHub Actions, with at minimum format/clippy/test and a build-all-crates job.
- **Documentation:** mdBook for the main docs site, plus inline rustdoc for the crates.
- **Community:** Matrix or Discord, and a discussions tab for design proposals.
- **Contribution model:** RFC process for major changes (new tier, new transport, new core trait); smaller PRs welcome directly.
- **Hardware support docs:** maintain a compatibility list — what's tested vs. community-reported vs. theoretical.
- **No telemetry by default.** Local-first is core to the appeal; if telemetry is ever added, opt-in only and clearly documented.

## Key external services

| Service | Used for | Cost (typical household) | Latency |
|---|---|---|---|
| Groq Whisper Large v3 Turbo | STT | ~$0.04 / month | ~200ms |
| Groq GPT-OSS 20B | Tier 1 LLM | ~$1–2 / month | ~600–800ms |
| Anthropic Sonnet 4.6 | Tier 2 LLM | ~$1–3 / month | ~2–3s |
| Piper TTS (self-hosted) | Voice synthesis | Free | ~100ms first audio |

Total cloud spend for a typical home: roughly $3–6 / month.

## Open decisions

These are intentionally left open for the project owner / community to decide:

- License (MIT vs. Apache 2.0)
- TTS voice selection for the default Piper model
- Whether to maintain a hosted demo instance or keep it BYO-infra
- Whether to add a read-only monitoring UI in v2 (not required, but possibly useful for debugging)

## Glossary

- **Satellite** — a per-room device with mic array + small speaker that streams audio to the Niles service and plays back responses
- **Tier 0 / 1 / 2** — escalating layers of intent handling, from local regex to fast LLM to smart LLM
- **Wyoming protocol** — open line-delimited JSON-over-TCP protocol for streaming voice between satellites and a server, originally from the Home Assistant ecosystem
- **AEC** — Acoustic Echo Cancellation, lets the mic hear you while audio is playing
- **DSP** — Digital Signal Processor, the dedicated chip (XMOS XVF3800) doing real-time audio cleanup
- **DoA** — Direction of Arrival, knowing which direction the speaker is in
- **microWakeWord** — small, on-device wake word detection model
- **Device naming convention** — Niles's `<room>/<device>` naming scheme set in the upstream source (e.g. Z2M), used as the canonical source of truth for the home's structure with no separate database
- **The curve** — the universal daily brightness-and-color-temperature function that governs all on lights in the home; defined by ramps and anchors, recomputed each day at midnight
- **Manual mode** — per-light state entered when a user explicitly adjusts brightness or color temperature; the curve stops touching that light until the next off→on cycle
- **Morning routine** — the only system-initiated turn-on event in Niles; auto-turns-on target lights at sunrise start on configured day patterns, with skip-if-already-on and skip-day-override logic
- **Timer escalation** — the two-stage alarm pattern for expired timers: originating satellite first, all satellites after 10 seconds without acknowledgment
- **Scene** — a named, scoped snapshot of light states (brightness, color temp, on/off) for one room or the whole home; applies via voice, puts affected lights into manual mode, exits via "back to normal"
- **Capability reference** — single source of truth for what Niles can do, stored as terse structured files in the repo; used both to ground LLM command execution and to answer user how-to questions
- **Tiered context (A/B/C)** — Niles's strategy for assembling LLM system prompts: Tier A always loaded for fast common commands, Tier B loaded on-demand for topic-specific requests, Tier C loaded only for full-system overviews
- **Notification** — an unprompted message Niles delivers to the user via voice, with chime + content, routed by satellite affinity and quiet-hours rules
- **External integration** — a self-contained crate (`niles-integration-<name>`) connecting Niles to an outside service (Archon, calendar, GitHub, etc.); exposes LLM tools and publishes notification events
- **Last-active satellite** — the most recently-used satellite by the user; default target for notifications without other routing affinity
- **Speaker recognition** — Niles's identification of who is speaking via voice embeddings; runs in parallel with STT, adds zero perceived latency
- **Voice embedding** — fixed-length numerical representation of a person's voice (~192–512 floats), produced by a small neural model and stored locally for nearest-neighbor identity matching
- **Admin** — a user with elevated privileges; can create rules, manage other users, and modify shared home settings like the lighting curve
- **Rule** — an admin-defined permission constraint (e.g. "only Mark or Majse can change the temperature") evaluated before every tool execution
- **Personal reference** — a phrase like "my calendar" or "wake me up" that the LLM resolves to the speaker's specific external resource (their calendar account, their bedroom, etc.); distinct from per-user preferences, which Niles does not maintain for shared home state
- **Presence source** — an adapter that reports whether a specific user is home (Tado, phone GPS, BLE beacons, manual voice command, etc.); multiple sources can be combined per user
- **Home state** — Niles's aggregated occupancy status (`occupied` / `unoccupied` / `transitioning`), derived from per-user presence with hysteresis
- **Automation** — a "when X, do Y" rule. Code automations are foundational behaviors implemented in Rust; config automations are simple rules in TOML or voice-created via the LLM, stored in SQLite
- **Capability manifest** — `MANIFEST.md`, the LLM-facing structured description of Niles's features, requirements, and setup decisions; generated from the canonical `features.toml`
- **features.toml** — canonical feature catalog at the repo root; single source of truth for the runtime's feature awareness, the generated MANIFEST.md, the README's feature table, and the docs site
- **Music intent** — high-level LLM tool (`play_radio`, `play_music`, etc.) that maps to source-specific Sonos actions internally; insulates the LLM from knowing which music service is being used
- **Per-room music state** — Niles's SQLite-stored memory of what each room was last playing (source, content URI, playing/paused), enabling "play TuneIn" to resume that room's last station

---

*This document is a living architecture spec for Niles, an open-source AI-first home automation system. Contributions and forks welcome.*
