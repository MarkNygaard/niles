# Niles — Neural Intelligence, Lightweight Edge System

> **About the name.** *Niles* (NYE-uls) is the project's name, the system's name, and the default wake word — three roles, one word. The acronym **N**eural **I**ntelligence, **L**ightweight **E**dge **S**ystem describes what it is: an AI-first home assistant that runs locally on small hardware. Users can configure a different wake word per satellite if they prefer; the project name stays Niles regardless.

## What this is

Niles is an open-source, AI-first home automation system designed to replace Home Assistant for users who:

- Have a relatively focused device ecosystem (mostly Zigbee via Z2M, maybe a few WiFi-native devices)
- Want sub-second voice interactions ("Alexa-fast" or faster)
- Prefer code/config over UI-driven setup (see [Runtime configuration](#runtime-configuration) for where that line actually falls, and [Signing in](#signing-in) for who is allowed to use either)
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
10. **A personal infrastructure surface, not just home automation.** Niles controls lights and timers, but its real ambition is to be the voice surface for everything its user runs — coding workflows, calendars, email, deployments, monitoring. The integrations Niles ships are first-class citizens, not bolted-on plugins: crates in this repository, held to the same tests and conventions as the core. For the long tail no project can carry, third-party integrations run as sandboxed WebAssembly plugins installed from GitHub (see [Third-party integrations](#third-party-integrations-plugins)); once installed their devices, tools and pages join Niles exactly as a built-in's do, but they reach only what the household approved.
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
│                                 └──────────────────────────────┘   │
│                                                ↕ SQL (TCP)         │
│                                 ┌──────────────────────────────┐   │
│                                 │  Postgres (CNPG + pgvecto.rs)│   │
│                                 │  - relational state          │   │
│                                 │  - voice embeddings (vector) │   │
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

**Tier 1 alternative — Cerebras GPT-OSS 120B:** a larger model that is also faster (~3000 t/s), and a free tier with 90K TPM against Groq's 8K. The free tier caps requests at 5/minute rather than tokens, so it rewards a turn that asks for a room in one call instead of one call per device.

**All-Anthropic alternative:** Claude Haiku 4.5 as Tier 1 (~690ms TTFT, ~95 t/s, excellent tool calling), Sonnet 4.6 as Tier 2.

**Fully-local alternative** (requires GPU node): whisper.cpp `base.en` or `distil-small.en` for STT, Llama 3.1 8B locally for Tier 1, escalate to a hosted Tier 2 only when needed.

**EU licensing note:** Meta's Llama 4 multimodal weights are not licensed to entities domiciled in the EU (text-only via hosted APIs like Groq is fine). Document this clearly in the README so users know.

## Device naming convention (the no-UI strategy)

To avoid building an admin UI in v1, Niles uses **device names in the underlying source as the single source of truth** for room/device structure. There is no separate Niles-side device database, no admin screens, no manual room assignment.

This still holds with the web UI in place. What that UI does is edit
*values* and *press switches*; what it cannot do is describe the house.
Where it names devices — choosing which lights are ambient, listing the
rooms on the dashboard — it reads the registry rather than a list
anyone maintains, and what it stores is the id the source gave them.
Rename a device in Z2M and Niles follows; add one and it appears.
There is nowhere else for the name to disagree, and no screen where a
room is created.

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
4. Reads the source's existing metadata to determine device type/capabilities (Z2M's `definition.exposes` array already tells you whether a device has on/off, brightness, color, temperature reading, etc. — no separate config needed). Numeric exposes with no field of their own (`power`, `energy`, `voltage`, `co2`, `pm25`) become named readings, with the unit the expose declares
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

### WLED as a second device source (MQTT)

WLED (WiFi addressable-LED controllers) is the first concrete *second* device source, and it reuses the MQTT layer Niles already speaks — no new transport. Conceptually it sits beside Z2M: a `WledSource` feeds the same `DeviceRegistry`, so a WLED strip behaves like any other light.

- **Namespacing.** WLED devices are internally `wled:<room>/<device>` (per the fully-qualified-names model above). Users and the LLM see the unprefixed `<room>/<device>`; the prefix only disambiguates collisions with Z2M.
- **Lights-only in v1.** Each configured WLED instance is exposed as a single light supporting **on/off, brightness, and RGB color** (and color temperature where the strip emulates it). It joins the `DeviceRegistry` exactly like a Z2M bulb, so room-scoped commands ("turn off the office lights") and the **ambient lighting curve drive it with no special-casing**. WLED's own effects, presets, segments, and per-segment control are explicitly **out of scope for v1** — a later slice can add an effects surface.
- **Discovery is config, not auto.** Unlike Z2M, WLED publishes no `bridge/devices` list, so instances are declared in Niles config (a `[wled]` section: each entry names the device `<room>/<device>` and its base MQTT topic). This is one-time install-time text, consistent with the no-UI principle. Adding a strip = one config entry, not a code change.
- **Commands + state over MQTT.** Niles publishes set-commands to the device's WLED MQTT topic and subscribes to its state topic for live updates feeding the registry (the lighting curve and manual-mode tracking both read that state, same as Z2M).
- **Unit translation lives in the source crate.** WLED brightness is `0..=255`; the `WledSource` translates to/from Niles' canonical `0..=100` percent, mirroring the rule that Z2M's `0..=254` translation happens in its source crate, never in core types. RGB ↔ any internal color representation, and any mired↔Kelvin handling, likewise live in the WLED source.

This keeps WLED a thin, self-contained addition: a new MQTT source + a config section, with the registry, intents, lighting curve, and voice paths all unchanged.

### WLED effects (v1.1)

Beyond on/off/brightness/color, WLED's signature feature is its animated **effects** (fire, rainbow, twinkle, …). v1.1 adds the ability to set one by voice, deliberately scoped small:

- **The LLM picks the effect, niles maps it to a number.** A Tier 1 tool `set_light_effect { device, effect }` advertises a curated set of effect *names*; the model resolves a fuzzy request ("make the tv light a cozy fire", "rainbow") to one of those names — its strength — and niles maps the name → WLED's FX index, publishing `{"seg":[{"fx":<index>}]}` to `<topic>/api` (reusing the same MQTT command path as the rest of WLED). Tier 0 stays out of effects; fuzzy → effect mapping belongs in the LLM.
- **Curated, not exhaustive.** WLED ships 100+ effects; niles supports a hand-picked, voice-natural subset (e.g. solid, breathe, blink, rainbow, colorloop, fire, candle, twinkle). The name→index table is pinned to WLED's standard FX indices and documented as firmware-coupled — a new effect is one table entry, not new machinery. `solid` (index 0) is how you turn an effect *off* (back to a static color).
- **Write-only.** The simple `/g` `/c` state topics don't carry the active effect (it's only in the XML `/v` payload, which niles doesn't parse), so niles sends effects fire-and-forget and doesn't track "current effect." Acceptable for v1.1.
- **WLED-only.** The tool/command routes through the source-aware `CommandRouter`; a non-WLED target is rejected (Z2M lights have no effect concept here).
- **Out of scope:** WLED presets (`ps`), segments / per-segment control, custom palettes, and effect speed/intensity tuning — all clean later slices.

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
- For target lights that are **already on**: routine skips them. The user is already up. The light continues at its current state — if it's in manual mode (the user clicked to escalate to 80% / 100%), that's respected and the curve does not touch it; otherwise it follows the curve from its current value (typically the 15% night floor, smoothly ramping to 100%).
- The routine completes at the end of the ramp window; control hands back to the regular curve, which by then is at 100% anyway, so the handoff is seamless.

**Worked example — early-riser scenario.** You wake up before the routine, go to the bathroom, click the bathroom light on and click again to escalate it to 100%. The bathroom light is now in manual mode. At 05:45 the routine fires:

- *Bathroom light*: already on → routine skips it. In manual mode → curve also leaves it alone. Stays at 100% until you turn it off.
- *Your partner's bedroom light* (still off, in the routine's target set): routine turns it on at 0% and rides the routine ramp to 100% by 06:30. Your partner wakes up gently.
- *Hallway light left on overnight at the 15% night floor* (not in manual mode, not a routine target): routine doesn't touch it, but the curve's morning ramp does — it drifts smoothly from 15% to 100% over the window.

Three lights, three behaviors, one rule set.

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

The curve's shape lives in the config file (sunrise/sunset times, day
patterns, target lights, color temp curve points). Its values can also
be changed while Niles runs — by voice ("make the evenings warmer"), or
in the config UI — and take effect on the next tick. See
[Runtime configuration](#runtime-configuration) for which settings are
tunable that way and which are not.

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

- Stored in Postgres immediately on creation, so a service restart picks up active timers
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

- **Service restart while a timer is running:** Postgres-backed, timer survives. If expiry passed during downtime, fires immediately on restart.
- **Two timers expiring simultaneously:** Each fires its alarm independently. Acknowledging "stop" stops the most recent one; "stop all" stops everything. The LLM resolves "stop" vs "stop all" based on intent.
- **Timer set in a room with no Sonos:** Just plays alarm on the satellite. No ducking needed.
- **Network partition between Niles service and originating satellite at expiry time:** Niles attempts the originating satellite, fails, immediately falls back to Stage 2 (all satellites). Better to over-alarm than miss the timer.

## Scenes

Light scenes let users save a set of light states under a name and recall them later. The interaction model is "set the lights how you want, then save" — no config files, no YAML, no UI. The lights themselves are the editor.

**What counts as a light.** A light, or a smart plug listed in `[lamp_plugs]` as having a lamp on it. A plug reports itself as an outlet and says nothing about what is plugged in, so Niles cannot tell a corner lamp from a fridge without being told. Listed plugs are lights everywhere lights are acted on as a group: scenes, "turn off the lights", a room's lights, wall switches and the morning routine. A plug in a scene records and restores only on/off. An unlisted plug is never part of any of those, and is switched only when asked for by name.

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

Scenes live in Postgres alongside timers. Minimal schema:

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

### Finding the speakers, and which room each is in

Sonos is an integration like the others: switched on and off under Integrations, and configured by the address of **any one** speaker. Automatic discovery (SSDP) is a multicast on the local network, and it does not reach a service running in a cluster network. One speaker is enough, because every Sonos can describe the whole household: its rooms, the speakers in each, and their current addresses.

- **A speaker is remembered by its Sonos identity, not its address.** Addresses come from DHCP and change. The household layout is read again from the speaker, so a new address is simply followed.
- **One row per Sonos room.** A home-theatre set (soundbar, rear speakers, sub) or a stereo pair is one Sonos room and plays as one. Niles never addresses the speakers inside it separately.
- **Rooms are set under Settings → Speakers**, the way satellites are: each Sonos room found is given a Niles room. A Sonos room without one is listed as not in a room, so a new speaker is noticed rather than lost. A portable speaker carried to another room is moved here. The satellites page shows each satellite's room speaker, read-only.
- **Several Sonos rooms can belong to one Niles room**, for example a soundbar at the front of a living room and a speaker at the back. The Niles room is the unit, as everywhere else. Music asked for in it plays on all of them, grouped, and pause, skip, volume and ducking act on all of them.

### The TV

A soundbar playing the TV is a Sonos that is playing, and Niles treats it as one:

- **While Niles speaks**, the TV is turned down like music, and turned back up afterwards.
- **Volume and pause** act on the TV sound.
- **Music asked for in the room replaces the TV sound.** "Play X" replaces whatever is playing (see below), and the TV is no exception. The music plays on the whole room, grouped.
- **Niles never sends TV sound to another Sonos room.** The TV plays on the soundbar and its own home-theatre speakers, as Sonos does by itself.

### Music intents, not source-specific tools

The LLM-facing tools think in terms of what the user wants, not which service. Each intent maps internally to source-specific Sonos actions, but users never need to know:

- `play_radio(station?, room?)` — TuneIn radio. "Last station" if none specified, named station otherwise. Stations are found with TuneIn's open station search and played on Sonos by station id; no account is needed.
- `play_favorite(name, room?)` — anything starred as a favourite in the Sonos app (a station, a playlist, an album), played by name. Local only, and it reaches content no public API exposes.
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

- `niles-speakers` — adds source-aware playback methods to the Sonos implementation, music intent dispatchers, per-room state tracking in Postgres
- `niles-tools` — adds the music intent tools
- `niles-intent` — adds Tier 0 patterns for common music commands

### Spotify

Playing and finding are separate:

- **Playing goes through Sonos.** Spotify is linked to the household in the Sonos app. Niles puts a Spotify track, album or playlist in the Sonos queue over the local API, and Sonos streams it.
- **Finding uses Spotify's Web API.** Sonos's local API cannot search a linked music service, so "play the new Lukas Graham album" needs Spotify's own search. Since February 2026, a Web API app in development mode requires its owner to have Spotify Premium, allows five users and returns at most ten search results. That is enough for a household.
- **Spotify's own playlists** (Discover Weekly, Daily Mix) cannot be read by new Web API apps. They are played as Sonos favourites (`play_favorite`).

### What this section does not specify

- The exact mechanism for Sonos to expose its linked-account info to Niles (which accounts are available, default selection logic) — explored in implementation
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

The reference files live in the repo at `docs/capabilities/<topic>/SKILL.md` (YAML frontmatter with `name`, `description`, `version`, then the reference). They are:

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

- Compiles every reference file into the binary, so no directory or config is needed at runtime
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

1. **Originating-satellite affinity.** If the notification relates to something the user started from a specific satellite (e.g. "that task you filed in the office has a PR ready"), play it on that satellite first.
2. **Last-active satellite.** Otherwise, the satellite the user most recently spoke to. Niles tracks this passively as a single mutable "last active" pointer.
3. **All satellites.** Only if the notification is genuinely urgent and the user must hear it regardless of location (rare; explicit `priority: urgent` flag).

Notifications never auto-escalate to all satellites the way timer alarms do. They're informational, not action-required. Missing one is acceptable.

### What notifications sound like

- A short, distinctive chime (different from timer alarms so users can tell them apart at a glance)
- Followed immediately by the voice message
- Voice message is concise — one or two sentences — and surfaces the most important fact first

Example: *(chime)* "The dark mode workflow on ticket0 just finished. Pull request is ready."

Not: *(chime)* "Hi Mark, I have an update for you about a coding task you filed earlier today. The dark mode task on the ticket0 project completed successfully and there is now a pull request available for your review at the following URL..."

The chime gives the user time to mentally orient before the content; the message respects their attention.

### Quiet hours and priority

Niles has a notion of *quiet hours* (default: 22:00 — 07:00), during which most notifications are deferred until morning. Three priority levels:

| Priority | Behavior in quiet hours |
|---|---|
| `low` | Silently queued; replayed in morning if still relevant |
| `normal` | Played at low volume; still respects routing rules |
| `urgent` | Played at normal volume; ignores quiet hours |

The integration emitting the notification picks the priority. A task's PR-ready notification is `normal`. Security camera motion at 03:00 would be `urgent`. Most things are `low` or `normal`.

### Recall

Notifications are not transient — Niles remembers recent ones. The user can ask:

- "What was the last notification?"
- "What did I miss this morning?"
- "Did that task get a PR yet?"

Notifications are stored in Postgres for ~7 days, then garbage-collected. The Tier 1 LLM has a `list_recent_notifications` tool for these queries.

### Where this lives in the codebase

A new crate, `niles-notifications`:

- Exposes an internal API: `notify(message, priority, origin?, satellite_hint?)`
- Handles routing (which satellite), formatting (chime + message), persistence (Postgres)
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
5. **Maintains its own short-lived state** (caches, subscriptions) but persistent data lives in the shared cluster Postgres alongside Niles's own tables

Integrations are independent of each other and of the core. Adding a new integration is a self-contained piece of work that doesn't require touching anything else.

### Discovery and naming

Integrations register themselves at startup with a stable name (e.g. `linear`, `calendar`, `github`). The LLM sees them via Tier B context when relevant. Users refer to them by their everyday names ("add a task," "check my calendar"), and the LLM maps to the right integration.

### First concrete integration: Linear (with the ai-harness as executor)

Niles's first coding-workflow integration is **Linear**, the task tracker — *not* a direct connection to a workflow engine. The user runs a self-hosted Rust **ai-harness** that watches Linear: any issue in the team's **"Todo"** column carrying the **"AI Eligible"** label is picked up, run through an idea-to-pr pipeline (plan → implement → review → PR) in an isolated worktree, and turned into a GitHub pull request linked back to the issue. Linear is also wired to GitHub, so every task and its PR are visible in one place.

This decouples Niles cleanly: **Niles just files and queries Linear issues.** It does not talk to the harness, doesn't know its internals, and doesn't track workflow runs — the harness reacts to Linear declaratively. Triggering work is "create a well-formed issue with the right label," not "imperatively poke an engine." (An earlier draft of this spec made Archon the first integration and had Niles call it directly; that was dropped in favor of this Linear-driven model — cleaner, single source of truth, and the task/PR overview comes for free.)

**Why Linear as the surface:**

- **Declarative trigger.** Label + column *is* the contract. No run IDs, no engine-specific API in Niles.
- **Single source of truth.** Every voice-created task lands as a normal Linear issue alongside hand-created ones; the user sees them all, with linked PRs, in Linear's UI.
- **Decoupled.** The harness can change, gain workflows, or be swapped entirely without touching Niles. Niles's contract is just Linear's API.
- **Fits voice.** Coding tasks are long-running; "file it and tell me when the PR's up" is exactly the fire-and-forget shape voice is good at.

**Interaction patterns:**

The LLM turns a spoken request into a well-formed Linear issue — a clear title plus a description that reads as a task brief — and files it in the right team, in "Todo", with "AI Eligible" so the harness runs it.

- **Create a task** (most common): "Niles, add a task: WLED effects should support presets." → files an issue (Todo + AI Eligible). Niles confirms: "Filed it. The harness will open a PR."
- **Status / query:** "What's in my todo?" / "Did the WLED effects task get a PR yet?" → lists issues or reports an issue's state + its linked PR.
- **Brief refinement (sparingly):** one clarifying question only when genuinely ambiguous, then file. Niles writes the brief; it does not hold a design conversation — the harness's planning step resolves the rest.

**Completion notifications:**

When an issue's linked PR opens or its state changes (e.g. → "In Review"), Niles can surface it unprompted via the notifications subsystem:

- *(chime)* "The WLED-effects task has a pull request ready for review."

Notification content is concise; the result location comes first, URLs go to the notification log rather than being read verbatim.

**Integration shape:**

The `niles-integration-linear` crate:

- Talks to **Linear's GraphQL API** with an API token (env var; never leaves the central Niles service).
- Resolves the configured team, the trigger **label** ("AI Eligible"), and the initial **state** ("Todo") by name at startup, so config stays human-readable.
- Exposes LLM tools:
  - `create_task(title, description)` — files an issue in the team, in "Todo", with "AI Eligible" (so the harness runs it). Returns the issue identifier/URL.
  - `list_tasks(status?)` — open tasks and their states.
  - `get_task(id)` — one issue's state plus its linked PR, if any.
- (Later) subscribes to Linear webhooks to publish notification events on state changes / PR-linked, for the completion chimes above.

**Configuration:**

```toml
[integrations.linear]
api_key_env = "LINEAR_API_KEY"
team        = "niles"          # team key or name
trigger_label = "AI Eligible"  # label the harness watches for
todo_state    = "Todo"         # column new tasks land in
```

**Resolving natural language:**

The LLM maps a casual spoken request to a clean issue title + brief — well within GPT-OSS 20B's range, no Tier 2 needed. It does not need to know workflow names or the harness at all; "make it AI-eligible in Todo" is the whole contract.

### Calendars

"What's on my calendar tomorrow?", "when's my next meeting?", "put the dentist in on Thursday at ten." A calendar is the clearest case of a *personal reference*: "my calendar" means a different calendar for each person who says it, so this integration is built around the person, not the house.

**Whose calendar.** Each person connects their own calendars, from their profile in the web UI. A calendar is only ever read or written for the person it belongs to: by voice, when [recognition](#user-recognition) has identified that person; in the app's chat, when they are signed in. An unrecognised voice gets no calendar at all ("I don't know who's asking"), and one person's calendar is never read out for another, whoever asks.

**Three providers, one set of tools.** A household's calendars live where its people's phones and work put them:

| | Reached through | Connected by |
|---|---|---|
| **Microsoft** — Outlook.com, Hotmail, and work or school accounts (Entra ID) | Microsoft Graph | Microsoft's sign-in (OAuth 2.0), redirect `/calendars/microsoft/callback` |
| **Google** — Gmail and Google Workspace | Google Calendar API | Google's sign-in (OAuth 2.0), redirect `/calendars/google/callback` |
| **iCloud** — the calendar on an iPhone | CalDAV, at `caldav.icloud.com` | the Apple ID and an **app-specific password** the person makes at appleid.apple.com |

A person may connect several, of any kind; each is one connection, with a name they choose ("Private", "Work"). The tools merge them.

**Access.** A connection is either **read only** or **read and write**.

- **Microsoft and Google grant it.** That is what Niles asks them for — `Calendars.Read` or `Calendars.ReadWrite` (with `offline_access` and `User.Read`) from Microsoft; `calendar.readonly` or `calendar.events` from Google. A read-only connection holds a token that *cannot* write, so a work calendar connected read-only stays read-only even if Niles misbehaves. Changing a connection's access means signing in to it again; the old token is discarded.
- **iCloud cannot.** Apple has no narrower credential than an app-specific password, and that password opens the whole account's calendars (and contacts and mail). For an iCloud connection "read only" is a rule Niles keeps, not one Apple enforces, and the profile says so where it is chosen. The password can be revoked at appleid.apple.com at any time, which ends the connection.

**Which calendar.** Microsoft: the account's default calendar. Google: its primary calendar. iCloud: every calendar on the account that holds events, read together; new events go to the first of them, which on an iPhone is the default one ("Calendar" or "Home").

**What is stored.** Per connection: the person, its provider, its name, the account's address, its access, and its credential — a refresh token, or iCloud's app-specific password — encrypted with the same key as the credentials typed into the app, in a `calendar_connections` table. Disconnecting deletes the row and its credential. Access tokens are kept in memory only.

**The household's app registrations.** Microsoft and Google each need one, made once by whoever runs Niles, with its client ID and secret entered under Settings → Integrations:

- *Microsoft:* the Entra admin center, "accounts in any organizational directory and personal Microsoft accounts", the redirect above. A work tenant may still refuse a person's consent to an outside app; that is the employer's call, and the profile says so when Microsoft refuses.
- *Google:* a Google Cloud project with the Calendar API enabled and a web OAuth client with the redirect above. The app must be **published** rather than left in testing, where Google expires its sign-ins after seven days; unverified, it shows "Google hasn't verified this app" on the way in, which is expected for a household's own app.

iCloud needs no registration.

**By voice (LLM tools).**

- `calendar_events(from, to)` — the speaker's events in a window, every connection merged and labelled with the connection's name. Times are the house's time zone.
- `create_calendar_event`, `update_calendar_event`, `delete_calendar_event` — only on a read-and-write connection. When the person has more than one, Niles asks which (by its name) unless they said. Every write is read back first and made only on a yes ("Dentist, Thursday at 10:00 in your private calendar — shall I?"), as [rules](#voice-created-rules) are.

These tools exist only when the person asking has a connection; the LLM is told whose calendar it is looking at.

**What leaves the house.** Answering a calendar question sends the events it needs — titles, times, places — to the LLM provider in the request that writes the answer, as any tool result is. Nothing is sent ahead of a question, and nothing is kept outside Niles's own database.

**Not in v1:** reminders Niles speaks unprompted ("your meeting is in ten minutes"), which belong to [notifications](#notifications-niles-speaks-unprompted); inviting other people; recurring-series edits beyond a single occurrence; choosing which of an account's calendars to read or write.

### Future integrations (not v1)

The same pattern scales to:

- **Calendar reminders** — spoken unprompted before what is coming up, from the [calendars](#calendars) already connected
- **Email** — summary digests, send drafts (with explicit confirmation)
- **GitHub** directly — repository status, recent activity, code review requests
- **Monitoring** — alerts from Grafana, Prometheus, Sentry surfaced as notifications
- **Deployments** — trigger or check status of CI/CD pipelines
- **Home Assistant** (ironic, but useful) — bridge to HA for users still running it for some devices, with Niles as the voice surface

Each is a self-contained crate. None require core architectural changes — that's the point of the pattern.

What the project will never carry in-tree — one household's inverter, a regional price feed, a niche gadget — can be written by anyone as a [third-party integration](#third-party-integrations-plugins), a sandboxed plugin installed from a GitHub repo.

### What this section does not specify

- Whether Niles polls Linear for status or relies solely on webhooks (webhooks preferred; polling is the fallback)
- How to handle Linear API-token rotation
- Voice-fingerprint gating of who can file/trigger tasks (deferred to the upcoming "user recognition" capability — see separate section)

## Third-party integrations (plugins)

> **Status: v0.2 of the interface (v0.1 plus [MQTT](#mqtt)), with v0.1 plugins still run beside it; built (Niles 0.2.0).** Built: the runtime and its sandbox, every host interface, devices, tools, timers, events, notifications, the pages' render and action routes, loading the enabled plugins from the `plugins` table at startup, `niles plugin check` (a directory, a `.wasm` or a GitHub repo) and `niles plugin dev`; installing from GitHub with the approval screen, update checks, updates with the permission difference and automatic rollback, turning off and removing ([Install](#install) onwards), with their routes; and in the app, a plugin's row with its Custom badge in its group, its panel with the generated setup panel, its log and where its widgets go, its pages in the menu, its widgets on the front page and the rooms, and the image fetch behind their pictures. Not yet: `niles plugin dev --live`. The interface is [`schemas/niles-integration.wit`](schemas/niles-integration.wit); the guide for people writing one is [`docs/integrations/`](docs/integrations/README.md).

Built-in integrations are crates in this repository, and they stay that way: reviewed, tested and released with Niles. That cannot cover every household. Somebody with an inverter, a pool pump, a local bus timetable or a niche gadget with an HTTP API wants it in Niles without asking for a crate, and without forking. Home Assistant answers this with custom integrations installed from GitHub (HACS). Niles takes the same shape, installing from a GitHub repo, and adds a sandbox around the code.

A third-party integration, a **plugin**, is a **WebAssembly component**: a compiled `.wasm` file that declares in its own metadata which functions it needs from its host and which it provides. Those declarations are written in **WIT**, the component model's interface language, and Niles's is the package `niles:integration@0.2.0`, world `plugin`; plugins built for `niles:integration@0.1.0` still run (see [Versioning the interface](#versioning-the-interface)). Niles runs plugins with **wasmtime**, using the Component Model and WASI 0.2.

### Goals and non-goals

Goals:

- Somebody who has never touched Niles's code can write an integration in an afternoon, publish it as a GitHub release, and install it from the app by pasting the repo's URL.
- An installed plugin is indistinguishable in use from a built-in. Its devices are in the rooms, in rules and on the dashboard, its tools are offered to the voice model and to the Claude and ChatGPT assistants, and it has a card on the Integrations page with a status line and a setup panel.
- A plugin reaches only what its manifest declares and the household approved. A broken or hostile plugin cannot read Niles's database, another integration's secrets, the conversations, the memory, or who lives in the house.

Non-goals:

- **Replacing built-ins.** Anything most households want (lights, music, presence, calendars) is a built-in crate. Plugins are for the long tail.
- **A marketplace.** There is no central index, rating or review. A plugin is a GitHub repo somebody chose to trust. A curated list may come later; it changes nothing in this interface.
- **Arbitrary code in the web app.** A plugin describes its pages and widgets declaratively and the app draws them with its own components. No plugin HTML or script runs in the app (see [Pages and widgets](#pages-and-widgets)).
- **Speed-critical paths.** A plugin never sits between a spoken command and a Zigbee light. Tier 0 and Tier 1 for built-in devices are unchanged by any number of plugins.

**Rejected: native Rust dylibs.** Rust has no stable ABI, so a `.so` built with one compiler version against one version of Niles's types breaks on the next of either; and a native library runs with the full rights of the Niles process, so there is no sandbox to speak of. **Rejected: a subprocess or container per plugin.** A subprocess is only sandboxed by the operating system, differently on every platform, and Niles in Kubernetes cannot start containers. A component is one portable file, its imports are the complete list of what it can touch, and memory and time limits come from the runtime.

### Languages

Anything that produces a WASI 0.2 component targeting the world:

| Language | Toolchain | Notes |
|---|---|---|
| **Rust** (primary) | `wit-bindgen` with the `wasm32-wasip2` target, or `cargo-component` | The template repo and the worked example. Components around 200 KB. |
| JavaScript | `jco componentize` (StarlingMonkey) | Build with `--disable http`, since HTTP goes through Niles's interface. Components around 12 MB. |
| Go | TinyGo with `wit-bindgen-go` | |
| Python | `componentize-py` | Pure Python only: libraries with C extensions do not build. Components around 19 MB, with the interpreter inside. |

### The sandbox

A plugin is one component instance in its own wasmtime `Store`. It has nothing but its imports, and Niles decides what each import does:

- **No WASI filesystem, environment or sockets.** Toolchains import WASI 0.2 whether the code uses it or not, so Niles links it, but empty: no preopened directories, an empty environment, a socket policy that refuses every address and every name lookup, and `wasi:http/types` without `wasi:http/outgoing-handler`. Clocks and randomness work. Stdout and stderr go to the plugin's log.
- **Network only through `niles:integration/http`, and MQTT through `niles:integration/mqtt`** to the brokers the manifest names, with the same address checks ([MQTT](#mqtt)). Each request's host must match the manifest's `http` list (exact, or `*.example.com` for any subdomain), and the address it resolves to is checked again: a private, link-local or unique-local address (RFC 1918, `169.254.0.0/16`, `fc00::/7`, `fe80::/10`) is refused unless the `local_network` permission names it, and loopback (`127.0.0.0/8`, `::1`) and the cloud metadata address `169.254.169.254` are refused whatever it says. Niles connects to the address it checked, so DNS cannot be rebound between the check and the request. Every redirect hop is checked the same way. Internet hosts are `https` only; plain `http` is allowed to local-network addresses, where devices rarely have certificates.
- **No access to Niles's own state.** There is no import for the database, the secret store, conversations, memory, people, voice embeddings or other integrations. A plugin reads its own settings and its own secrets, and its own key-value storage.
- **One instance per plugin, calls serialised.** Niles never makes two calls into the same plugin at once and never calls a plugin while it is inside a host call, so a plugin has no locking to get right. Tool calls and `set-device` go ahead of queued timers and events, since somebody is waiting for them. Plugin calls run on their own threads, never on the async workers that carry the voice loop.

### Runtime limits

| | Limit | Why |
|---|---|---|
| Memory | 64 MiB by default. The manifest may ask for up to 256 MiB (`memory_mb`), shown at approval. | Rust plugins use a few MiB. JavaScript and Python carry an interpreter and want 128. |
| Time per call | 10 s, enforced by epoch interruption. `init` gets 30 s, `render` 3 s, `stop` 2 s. | One instance means a slow call holds up every other. Ten seconds already makes a spoken answer feel broken. |
| Long calls | None. | Work that takes longer is split: start it, then pick it up from a timer. A per-call "long" flag would let one plugin block its own tools for minutes, and nothing in the long tail needs it yet. |
| Component | 64 MiB download; the manifest 64 KiB, the logo 32 KiB | |
| Storage | 1 MiB per plugin; keys up to 256 bytes | Settings, caches and a little history, not a database. |
| HTTP | 4 MiB response body, 5 redirects, timeout 5 s by default, never past the call's deadline | |
| Timers | 32; `every` at least 5 s | |
| Devices | 256 | |
| Tools | 16; description up to 1 KiB, schema up to 8 KiB, result up to 16 KiB (cut off beyond, with a note to the model) | Each tool costs prompt tokens on every request it is sent with. |
| Events | A queue of 256 per plugin; on overflow the oldest are dropped and counted on the card |
| MQTT | 4 connections, 64 subscriptions each; 256 KiB a message each way (past 1 MiB the connection is dropped); 20 publishes a second per connection, bursts of 50; 256 messages and 8 MiB waiting, the oldest dropped and counted in the log | A printer's full report is tens of KiB. The waiting messages are bounded in bytes as well as count, or 256 large ones would be 64 MiB. | |
| UI | 500 nodes, 8 deep, 256 KiB per render; images 64 KiB as `data:` URIs | |
| Notifications | 6 an hour | |
| Logs | 100 lines per 10 s, 2 KiB each; the last 1,000 kept for the card | |

Epoch interruption: a ticker advances wasmtime's epoch every 100 ms and each call is given a deadline in epochs, so a plugin stuck in a loop is stopped with a trap without Niles paying for instruction counting. Time spent waiting in a host call counts against the deadline too; a host call never outlives it.

### The interface

The full WIT is [`schemas/niles-integration.wit`](schemas/niles-integration.wit), commented item by item. In summary:

**What Niles provides (imports):**

| Interface | What it does |
|---|---|
| `log` | Lines at debug, info, warn and error, to Niles's log and the plugin's card. |
| `config` | `get(key)` for the plugin's declared settings, `secret(key)` for its declared secrets. |
| `storage` | `get`, `set`, `delete`, `list-keys(prefix)`: private string values in Postgres, namespaced to the plugin, 1 MiB quota. |
| `http` | `send(request) -> result<response, http-error>`: one request, whole response, hosts checked against the manifest. |
| `devices` | `upsert`, `report-state`, `report-action`, `set-available`, `remove`, `registered`: the plugin's own devices. |
| `timers` | `every(id, seconds)`, `after(id, seconds)`, `cancel(id)`, which call the export `on-timer`. |
| `events` | `subscribe(kind, device-patterns)`, `unsubscribe(kind)` for device-state changes, button actions and the house's presence, delivered to `on-event`. |
| `notify` | `say(message, priority, room)`, through Niles's [notifications](#notifications-niles-speaks-unprompted): chime, routing, quiet hours, recall. |
| `ui` | The node types for pages and widgets, and `invalidate(surface)` to ask open apps to redraw one. |
| `mqtt` (0.2) | `connect(broker, options)`, `subscribe(conn, filter, qos, latest-only)`, `unsubscribe`, `publish(conn, topic, payload, qos, retain)`, `accept-certificate(conn)`, `disconnect`: connections Niles holds to the brokers the manifest names. See [MQTT](#mqtt). |

**What a plugin provides (exports):** `init`, `tools`, `call-tool(name, args-json)`, `set-device(id, desired)`, `on-timer(id)`, `on-event(event)`, `render(surface, context)`, `on-action(surface, control, value, context)`, from 0.2 `on-mqtt-message(conn, topic, payload)` and `on-mqtt-status(conn, status)`, and `stop`. Every export must exist; a plugin with no tools returns an empty list, one with no pages returns an error from `render`, one with no MQTT leaves the two MQTT exports empty.

`stop` is the shutdown hook. It is not called `shutdown` because a world-level export of that name clashes with a symbol in componentize-py's runtime, and the Python build fails.

Device state uses Niles's own fields and units, as `DeviceState` in `niles-core` does: on, brightness 0–100 percent, colour temperature in kelvin, rgb, temperature in °C, humidity in percent, battery in percent, open, motion, illuminance in lux, and position 0 (shut) to 100 (open). Any other number is a named **reading** with the plugin's own unit — `price` in `kr/kWh`, `power` in `W`, `co2` in `ppm` — which rules follow like a temperature. Every field is optional and reports merge field by field, readings name by name, so `{on: false}` says nothing about brightness and a `power` reading says nothing about `energy`.

**Not yet: local TCP and UDP, and host-managed WebSockets.** The shape is settled enough to write down:

```wit
// Draft. Not part of niles:integration@0.2.0.
interface sockets {
    // Niles holds the connection, reconnects with backoff, and delivers each
    // message to the export on-message(id, message).
    open-websocket: func(id: string, url: string, headers: list<tuple<string, string>>) -> result<_, host-error>;
    send-websocket: func(id: string, text: string) -> result<_, host-error>;
    close-websocket: func(id: string);
    // One request and one reply over TCP or UDP, to a local-network address.
    tcp-exchange: func(host: string, port: u16, payload: list<u8>, timeout-ms: u32) -> result<list<u8>, host-error>;
    udp-exchange: func(host: string, port: u16, payload: list<u8>, timeout-ms: u32) -> result<list<u8>, host-error>;
}
```

They are left out because each adds something that should be got right first. Raw TCP and UDP widen the `local_network` permission from "talk HTTP to that address" to "speak any protocol to it", which deserves its own wording at approval. And the long tail reachable with polled HTTP is long: most cloud APIs and most local devices with an API at all. A WebSocket is a long-lived connection pushing messages into a plugin whose calls are serialised, the problem [MQTT](#mqtt) settled in 0.2: Niles holds the connection, reconnects with a backoff, and delivers what arrives to an export through a bounded queue. A WebSocket interface would follow the same rules. The draft above is a starting point, not a commitment.

### MQTT

Interface 0.2 adds MQTT, for devices that speak it rather than HTTP or push their state rather than wait to be asked: 3D printers (a Bambu Lab printer is a broker on port 8883 with its own certificate), Tasmota and Shelly plugs, and the household's own broker. The author guide is [`docs/integrations/mqtt.md`](docs/integrations/mqtt.md).

**The permission** lists the brokers, each named for `connect`:

```toml
[permissions]
mqtt = [
    { name = "printer", host = "setting:address", port = 8883, tls = "trust-device" },
    { name = "cloud", host = "broker.example.com" },          # port 8883, tls = "verify"
]
```

`host` is a host name, an address, or `setting:<key>` (a string setting the household fills in). `port` defaults to 8883, or 1883 without TLS. `tls` is `verify` (the default: the public certificate authorities, for the host's name), `trust-device` (the device's own certificate, below), or `none` (plain text). `trust-device` and `none` are for the home network only: a literal address outside it is refused in the manifest, and every address is checked again when the plugin connects, after DNS, with the HTTP rules ([The sandbox](#the-sandbox)): loopback and the metadata address never; for `trust-device` and `none` only private, link-local and unique-local addresses; for a host name written in the manifest under `verify`, only internet addresses, as for an `http` host, so a name the author controls cannot be pointed into the house; for an address written in the manifest, or one the household typed in, under `verify`, either, since the certificate has to be valid for it. Niles connects to the address it checked. A 0.1 manifest cannot declare `mqtt`. The approval screen says each broker in a line: "Connect to the MQTT broker at the address you enter under Printer address, on your home network, trusting the certificate it shows the first time", "Connect to the MQTT broker at 192.168.1.20, on your home network, unencrypted", "Connect to the MQTT broker at broker.example.com".

**Niles holds the connection.** `connect(broker, options)` checks the broker and the options (client id, username and password, usually from a setting and a secret, keep-alive, clean session), starts a task on the async runtime and returns a connection id at once; nothing in the interface waits on the network. The task resolves and checks the address, connects, does TLS, sends CONNECT, pings at the keep-alive, and when the connection drops reconnects after 1 s, 2 s, 5 s, 10 s, 30 s and then every 60 s (from the start again once a connection lasted a minute), subscribing again to every filter the plugin subscribed to. It speaks MQTT 3.1.1 with rumqttc's packet codec and session state (the crate `niles-mqtt` already uses for Zigbee2MQTT), over a connection it makes itself, because rumqttc's own event loop resolves the host again and so would not connect to the address that was checked. What happens reaches the plugin through two exports: `on-mqtt-status(conn, status)` with `connected`, `disconnected(why)` (Niles is retrying), `failed(why)` (Niles stopped: the broker refused the credentials, the address is not allowed, a `verify` certificate is not valid), or `certificate-changed`; and `on-mqtt-message(conn, topic, payload)`. Each status is also written to the plugin's log. `publish` hands the message to the task and returns; while the connection is down it fails with `unavailable`. Connections belong to the instance: dropping the instance (a trap, a restart, new settings, an update) closes them, and connection ids are never reused, so a message for a closed connection that was already queued is not delivered to its successor.

**Delivery.** Statuses and messages go into a per-plugin queue beside the events, taken in turns with them after jobs and due timers, one call at a time. It holds 256 messages and 8 MiB; past either the oldest messages are dropped (statuses never) and the count is written to the plugin's log, at most every 10 s. A subscription may ask for `latest-only`: while a message on a topic waits, a newer one on the same topic replaces it in place. It is opt-in per subscription because coalescing is right for topics whose messages are each the whole state (a Tasmota `STATE`) and wrong for partial reports that build on each other, as a Bambu Lab printer's are, where dropping one loses that change.

**`trust-device` is trust on first use, pinning the public key.** A device's own certificate cannot be checked against anything, so Niles checks that it stays the same. The first time a connection to a `trust-device` broker gets as far as the broker accepting the username and password, Niles keeps the SHA-256 of the certificate's public key (its SubjectPublicKeyInfo) in the plugin's storage, under `.niles/mqtt-pin/<broker>/<a hash of the address and port>`; storage keys starting with `.niles/` are Niles's own, refused to the plugin's `get`, `set` and `delete` and left out of `list-keys`, and count towards the quota. Every later connection must present the same key, or Niles closes it after the TLS handshake, before CONNECT, so the password is never sent, and tells the plugin `certificate-changed(trusted, presented)`; it then waits, without retrying, until the plugin calls `accept-certificate(conn)`, which pins the key it was shown and connects again. Handshake signatures are verified as usual, so whoever answers holds the key. A new address (the setting changed) is trusted afresh.

The other choice was to accept any certificate, which is what the community integrations for these printers do. It never breaks, but it protects nothing beyond passive eavesdropping: anything that answers at the address (a device that took over the DHCP lease, a spoofed ARP reply) receives the access code, which controls the printer, and can feed the plugin invented reports. Pinning closes that after the first connection, for the price of a question when the key changes. Pinning the key rather than the whole certificate means a device that renews an expiring certificate on the same key, as self-signed devices do, passes silently. Bambu Lab printers can present a new certificate after a firmware update; when that comes with a new key it is a rare, explainable event the household has just caused, which is the right moment to ask, and the plugin asks through its own page (a badge and a "Trust the new certificate" button whose action calls `accept-certificate`) because only it knows how to say what the device is. A plugin that accepted every change itself would defeat the pinning; the guide says not to, and the approval screen says the plugin trusts "the certificate it shows the first time".

**`niles plugin dev`** prints each connect, status, subscription, publish and message, abbreviated to a line (`mqtt 1 < device/01S00A/report {"print":{"nozzle_temper":24.6,…} (31.4 KiB)`), through a watch the runtime calls (`House::with_mqtt_watch`).

### Pages and widgets

A plugin can add to the app: a **page in the menu**, and **widgets** that sit on the front page or on a room card. Both are **declarative**. The plugin describes a surface as a tree of a few Niles components and the app draws them with its own, so a plugin page looks like every other page, follows dark mode and the phone layout, and runs no code in the browser: it cannot read the session, the cookies or anything else on the page.

**Declared in the manifest**, with the `ui` permission:

```toml
[[ui.menu]]
id = "electricity"
label = "Electricity"
icon = "zap"            # a Lucide icon name
refresh_every = 300     # seconds, optional

[[ui.widgets]]
id = "price_now"
title = "Electricity now"
room = "kitchen"        # optional: the room card it starts on; none means the front page
refresh_every = 300
```

A menu entry joins the [menu](#the-web-ui) like the built-in pages, as `<plugin-id>.<entry-id>` in `[menu]`'s `order`, `avatar` and `hidden`, so Settings → Menu can show, hide and move it; an entry not yet arranged goes to the end of the main navigation, as a new built-in page does. `[menu]` cannot know what is installed, so it accepts any entry of that shape, and the app shows only the installed, enabled plugins' entries. Selecting one opens `#/plugins/<plugin-id>/<entry-id>`, titled with its label and drawn with its Lucide icon (fetched in a chunk of its own the first time a plugin page is in the menu; a name Lucide lacks draws a puzzle piece). A widget starts where the manifest says; the plugin's panel on the Integrations page can move it to the front page or another room, or hide it, written as `[menu.widgets.<plugin-id>]` with `<widget-id> = "front"`, `"room:<room>"` or `"hidden"`. On the front page a widget is a card titled with its `title`; on a room it is drawn where the room opens, under its lights, because a room's tile is itself the room's light switch and has no room for more. A room the house does not have puts a widget on the front page, still rendered for that room. A plugin that is turned off takes its pages and widgets out of the app; one that is down keeps them, and each says "Not available right now" with the status line beneath.

**Rendering.** The app asks Niles for a surface, Niles calls the export `render(surface-id, context)`, and the plugin returns a `ui-tree`. The context carries the room (for a widget on a room card) and the signed-in person's slug (`mark`) when the app knows who is looking, so a page can differ per person without the plugin learning anything else about them. `render` has 3 s and should draw from what the plugin already holds, not fetch.

**The vocabulary** is small and versioned with the interface:

| | Nodes |
|---|---|
| Layout | `stack`, `row` (a stack on a phone), `card` with an optional title |
| Content | `heading`, `text` (optionally muted), `reading` (label, value, unit), `badge` (label, tone), `items` (a list of title, detail, trailing) |
| Controls | `button` (id, label, style), `switch` (id, label, value), `slider` (id, min, max, step, value), `choice` (id, options, value), `text-field` (id, placeholder) |
| Charts | `chart`: line or bar, labels and up to 4 series |
| Images | `image`: an `https` URL on a host the `http` permission declares, which Niles fetches and serves itself so the viewer's browser never contacts that host; or a `data:` URI of at most 64 KiB. PNG, JPEG, WebP or GIF; no SVG. |

WIT has no recursive types, so a tree is a flat list of nodes: node 0 is the root, and each node lists its children by index. A child's index is greater than its parent's and no node has two parents, which rules out cycles; Niles rejects a tree that breaks either rule, or that gives children to anything but `stack`, `row` and `card`.

**Actions.** Using a control calls `on-action(surface-id, control-id, value, context)` with what happened: `pressed`, `toggled(bool)`, `number(f64)` when a slider is let go, `chosen(value)`, or `text(value)` when a field is submitted. The plugin answers with an optional toast ("Prices updated") and whether to render again. An `err` is shown as an error toast.

**Refresh.** A surface is rendered when it is opened; after an action that asks for it; every `refresh_every` seconds while it is open (the app asks again); and when the plugin calls `ui.invalidate(surface-id)`, for instance after a timer fetched new data. An invalidation is pushed to the app as a `ui_invalidated` frame on the WebSocket at `GET /events/stream` the app already holds (`{"type":"ui_invalidated","plugin":"spotprice","surface":"electricity"}`; calls within 250 ms are one frame), so nothing polls for it, and the app renders again only if the surface is on screen. Niles keeps the last tree per surface, room and person until it is invalidated or `refresh_every` passes, so ten open phones cost one call; an action answered with `rerender` drops it too.

**In the app.** A renderer (`web/src/components/PluginSurface.tsx`) draws the tree with the app's own components: `stack` and `row` as flex layouts (a row stacks below `sm`), `card` as the app's card, `reading` as the large number with its unit and label, `badge` in the tone's colour, `items` as a divided list, `chart` as a line or bar chart in four series colours checked as a set for colour-blind separation, with a legend from two series, the value under the pointer and the numbers as a table for screen readers, and the controls as the app's button, switch, slider, select and text box. A control shows its new value at once and puts the old one back with the plugin's error if the action fails; a returned toast is shown for a few seconds, and `rerender` asks for the surface again. Each surface is a query keyed by plugin, surface and room: a `ui_invalidated` frame marks every room's copy stale and refetches the ones on screen, and `refresh_every` is the query's refetch interval. A 422 shows the plugin's sentence in place of the surface; anything else shows "Not available right now".

**Pictures.** An `https` image is drawn from `GET /plugins/{id}/image?src=<url>`: Niles checks the URL against the manifest (the `ui` permission, https, no credentials in the URL, a host the `http` permission declares, not `.svg`), fetches it through the plugin HTTP client with a policy of only the `http` hosts (so every hop is checked, and a name resolving to a private, loopback or metadata address is refused, as for the plugin's own requests) within 5 s and 4 MiB, and serves it only if its first bytes are PNG, JPEG, WebP or GIF, with that type, `nosniff`, a content security policy that runs nothing, and `Cache-Control: private, max-age=600`. Fetched pictures are kept 10 minutes, at most 32 MiB across all plugins, oldest out first. A `data:` image is drawn as it is.

**HTTP.** `GET /plugins/{id}/surfaces/{surface}?room=kitchen` answers the tree as JSON, the same flat list with each node's kind as `type` (`{"nodes":[{"type":"stack","children":[1]},{"type":"reading","label":"Now","value":"1.23","unit":"kr/kWh","children":[]}]}`; `niles_plugins::ui::Surface` is the reference). `POST /plugins/{id}/surfaces/{surface}/actions/{control}` takes the value (`{"type":"pressed"}`, `{"type":"toggled","value":true}`, `{"type":"number","value":21.5}`, `{"type":"chosen","value":"DK2"}`, `{"type":"text","value":"…"}`) and answers `{"toast":…,"rerender":…}`. The person is taken from the session, never from the request. The plugin's own error is a 422 with its sentence, a plugin that is down a 503 with its status line; the app shows either in place of the surface.

**Later: a page of its own.** Some plugins will want more than the vocabulary: a floor plan, a camera grid. A possible later escape hatch is a separate permission for a page served from the plugin's own static assets in a sandboxed iframe on a separate origin, without the session. It is not in 0.2, and the vocabulary should grow first.

### The manifest

Every plugin carries `niles-integration.toml`. In outline (the author guide describes every field):

```toml
id = "spotprice"                     # [a-z0-9_], 2-32 characters, starts with a letter
name = "Spot prices"
version = "0.1.0"                    # semver; the release tag is v0.1.0
description = "Hear what power costs hour by hour, and let rules run things when it is cheapest."
authors = ["Jane Doe <jane@example.com>"]
homepage = "https://github.com/jane/niles-spotprice"
min_niles = "0.2.0"                  # the oldest Niles it runs on; 0.2.0 is the first with plugins
interface = "0.2"                    # the niles:integration version it targets: "0.2", or "0.1"
# group = "presence"                 # a catalogue group id; missing or unknown means Custom
logo = "logo.svg"
memory_mb = 64
keywords = ["electricity", "power", "price", "kwh", "cheap", "strøm", "elpris"]

[permissions]
http = ["api.energidataservice.dk"]
local_network = []                   # IPs, host names, a /24 at widest, or "setting:<key>"
devices = true
tools = true
events = []                          # "device_state", "device_action", "presence"
notify = false                       # true, or "urgent" to speak during quiet hours
ui = true
mqtt = []                            # brokers, from interface 0.2; see MQTT

[[settings]]
key = "area"
label = "Price area"
type = "choice"                      # string | number | boolean | secret | choice | room
required = true
default = "DK1"
help = "West of the Great Belt is DK1, east of it DK2."
options = [{ value = "DK1", label = "West Denmark (DK1)" }, { value = "DK2", label = "East Denmark (DK2)" }]
```

- **The description** is one line saying what the household gets, in the voice of the built-in catalogue's blurbs ("Hear what is on your Google calendar."), not what the API is.
- **The id** must not be a built-in catalogue id (`mqtt`, `groq`, `sonos`…) or another installed plugin's. It names the plugin's device source (`x_<id>`), tool prefix (`<id>_`), secret keys (`plugins.<id>.<key>`) and menu entries (`<id>.<entry>`).
- **The logo** is an SVG in the release, drawn on a 24×24 view box like the built-in marks in `BrandMark.tsx`. Niles sanitises it on install (shapes, paths, groups and fills only; no script, style, link, `foreignObject` or external reference; at most 32 KiB): an allowlist refuses anything else with a reason, and the SVG is then written out again from the parse, element by allowed element and attribute by allowed attribute, so the stored copy holds nothing the allowlist does not name. Niles keeps that copy in its database and serves it itself from `GET /plugins/{id}/logo`, with a content security policy that allows nothing to run or load, so the Integrations page never fetches a logo from somewhere else. The app draws it inline, so `currentColor` follows the text colour, as ChatGPT's mark does, in both themes: it fetches the text and rebuilds it as React elements from the same allowlist (`web/src/lib/svg.tsx`), never as markup, so nothing in it can run even if the server's sanitiser ever let something through; anything outside the list draws the generic mark instead. Without a logo the card shows the group's generic icon.
- **Settings** generate the plugin's setup panel. `room` is a `choice` filled with the house's rooms. `secret` values go into Niles's existing encrypted secret store under `plugins.<id>.<key>`, are never shown again, and reach the plugin only through `config.secret`. `GET /plugins` carries each setting as the manifest declares it, with what the household entered (never a secret's value) and whether it is set; the panel draws a text box, a switch, a select or a password box per type, marks the required ones, writes help under each and the default beside it rather than into it, and saves what changed with one `PATCH /config` (an emptied field as `null`, back to its default) and each secret on its own through `PUT /secrets`. Until every required setting has a value it reads "Not set up: needs API key".
- **`setting:<key>`** in `local_network` means "the address the user types into that setting", for devices whose address only the household knows. The approval screen says so in those words.

### Where a plugin appears

On the Integrations page, under **Connected** once installed and set up, and in the group its manifest names, next to the built-ins:

- **Group.** `group` is one of the catalogue's group ids (`essentials`, `music`, `presence`, `calendars`, `shopping`, `assistants`). A missing or unknown id puts the plugin in a **Custom** group, shown after the built-in ones. The ids are part of the interface: Niles may add groups, and a plugin naming a group its Niles does not know yet lands in Custom rather than failing.
- **Marked as custom.** Every plugin's row carries a small **Custom** badge. Its setup panel shows where it came from: the GitHub repo as a link, the authors, the version, the install date. When a newer release exists the panel says **Update available**, and updating shows the change in permissions first (below). A plugin built for an older interface than the newest this Niles runs says so on its panel ("Built for an older Niles"), with a link to its repo.
- **Its log.** The panel shows the plugin's last lines, newest first, with their levels and times, asked for again every 5 s while it is open (`GET /plugins/{id}/log`). The runtime keeps the last 1,000 per plugin in memory, by plugin id rather than by instance, so the lines that explain a crash outlast the restart it caused and an update or rollback: what the plugin logged through `log` and its stdout and stderr, within the rate limit, and what Niles noted about it, each status change ("Working", "Not set up: …", "Not working: trapped in on-timer: …", "Stopped after repeated failures: …") and how many lines the rate limit dropped. Removing the plugin drops its log; a restart of Niles starts it empty.
- **Install entry.** **Add from GitHub** sits at the end of the **Available** tab, under its own heading.
- **Status line.** Like a built-in's: "Working", "Not set up: Price area is required", "Not working: timed out in on-timer after 10 s", "Stopped after repeated failures", or "Off". A working plugin with a newer release reads "Update available: v0.2.0".

### Install

Two steps, so that what is installed is exactly what the household saw.

1. Under Integrations → Available, **Add from GitHub**, and paste the repo's address: `https://github.com/jane/niles-spotprice` (with `.git`, a trailing slash, or `/releases/tag/v0.2.0` for that release), or `jane/niles-spotprice`.
2. **Preview** (`POST /plugins/install/preview` with `{"repo": …, "tag"?: …}`). Niles asks GitHub's REST API, without a token, for the repo's latest release (`GET /repos/{owner}/{repo}/releases/latest`: not drafts, not pre-releases), or for the tag the address or the request names. The release must have `niles-integration.toml` and `integration.wasm` among its assets, and the logo the manifest names (by its file name). Its tag must be the manifest's version, with or without a `v`. No release, no public repo, and GitHub's limit of 60 requests an hour without a token are each said in a sentence.
3. Niles reads the manifest first (at most 64 KiB) and refuses, saying why, if it is invalid, its `interface` is one this Niles does not run, or its `min_niles` is newer than this Niles, before fetching anything else. Then it downloads the component (at most 64 MiB) and the logo (at most 32 KiB), refusing either past its cap whether GitHub's reported size, the `Content-Length` or the bytes as they arrive say so; compares each download with the sha256 GitHub reports for it, where it reports one; and runs everything `niles plugin check` runs, including compiling the component against the world. It refuses what would clash with the house: an id already installed, a tool whose name is one of Niles's own or another plugin's, and devices already in the registry under its source `x_<id>`.
4. It shows the name, description, logo, repo, authors, version and group, and **the permissions in plain words** (see the author guide's [permissions](docs/integrations/permissions.md)): "Connect to api.energidataservice.dk", "Reach the device at the address you enter under Address, on your home network", "Add devices to your home", "Offer 1 tool to the assistant: prices", "Adds a page to the menu: Electricity", "Adds a card to the front page: Electricity now", "See every device in your home change: lights, doors, motion", "Speak through your satellites, even during quiet hours". Followed, always, by what it cannot do: "It cannot see your conversations, the people in your house, other integrations' passwords or Niles's database." Then the settings it will ask for, and warnings: no logo of its own, no checksum from GitHub to compare with, required settings to fill in before it starts, a whole range of home-network addresses.
5. The preview keeps the downloaded files in memory under a random token for 15 minutes (192 MiB of previews at most; past that the oldest goes) and pins the sha256 of the component and of the manifest. **Install** (`POST /plugins/install` with `{"token": …, "sha256": …}`, the sha256 being the one the screen showed) installs exactly those files, after checking them against the pins: nothing is downloaded again, so an asset replaced on GitHub in between changes nothing. A token installs once. The approved manifest, the sanitised logo, the component's bytes and sha256, the manifest's sha256, and the release's tag and page are kept in Postgres: a restart never downloads again.
6. The plugin appears in its group with its panel, and starts; until required settings are filled in it reads "Not set up".

Only public repos for now; a private one needs a GitHub token, which is a later addition. Plugins need [the database](#persistence); without one, the preview says so and nothing is installed.

### Updates

Niles asks each plugin's repo for its releases every 6 hours, starting 2 minutes after it starts (so a Niles restarting in a loop does not spend GitHub's hourly limit), and when **Check for updates** is pressed on its panel (`POST /plugins/{id}/check-update`). The newest release by semver that is newer than the installed version is recorded as **Update available**, with its version, its release notes and the change in permissions, worked out from the two manifests: what is added and what is removed, in the approval screen's words. Pre-releases are offered only to a plugin on a pre-release. Nothing updates by itself.

Updating is the install's two steps from the panel: `POST /plugins/{id}/update/preview` downloads and checks the newer release and shows it with the difference first, and `POST /plugins/{id}/update` with its token installs it. A plugin may also have changed its settings; new required ones put it in "Not set up" until they are filled in.

The version an update replaces is kept. For **10 minutes** after an update the new version is watched, and if it traps, times out, or its component will not load, the previous version is put back by itself and the panel says why. That notice is kept with the plugin's row in the database, so it is still there after a restart, until the next update or rollback. `init` returning an error does not count, since that is the plugin saying what is wrong (usually a setting), and nor does "Not set up". The previous version can also be put back by hand from the panel (`POST /plugins/{id}/rollback`), once: a rollback does not keep the version it replaced.

### Remove and disable

**Turn off** (`POST /plugins/{id}/disable`, and `enable` to turn it on again) stops the plugin, keeps it stopped across restarts, and keeps everything: settings, secrets, storage, devices (shown as unavailable). **Remove** (`DELETE /plugins/{id}`) stops it, deletes its devices from the registry with a `DeviceRemoved` each, its tools, its storage, the stored component and logo, its settings and its secrets. Its pages leave `[menu]`'s `order`, `avatar` and `hidden`, and its `[menu.widgets.<id>]` goes, in the same config revision as its settings, so one undo brings all of it back. Rules that named its devices are kept and shown as broken on the Automations page, so the household decides what to do with them rather than finding them silently gone.

### How a plugin joins Niles

- **Devices** go into the same registry as every other source's, with the source `x_<plugin-id>`: `x_spotprice:system/electricity`. The `x_` prefix keeps plugin sources apart from built-in ones, and both segments follow the `DeviceId` rules in `niles-core` (`[a-z0-9_]+`). People and the model see `system/electricity`, as with any device; where two sources have the same `room/name`, the prefixed form tells them apart, as with WLED.
- From there nothing is plugin-specific. Room cards draw them by class. Lights are in "the lights", the curve and scenes; ambient settings exclude them like any other. **Rules** trigger on them and act on them: a `set_device` on a plugin device is routed to the plugin's `set-device` export. Tier 0 fast paths ("turn off the kitchen lights") include them. A plugin light on the curve gets a `set-device` once a minute, which is why those calls must be quick.
- **Niles remembers a plugin's devices** in its own tables, not in the plugin's quota, so while a plugin is stopped its devices stay in their rooms as unavailable and rules that name them still validate. `devices.registered()` lets a plugin reconcile what Niles holds on `init`.
- **Tools** join the [tool registry](#tools-exposed-to-the-llm) as `<plugin-id>_<name>` (`spotprice_prices`). A name that would clash with a built-in tool or another plugin's is refused at install. They are offered to the voice model (Tier 1 and 2), to the Claude and ChatGPT assistants through Niles's MCP endpoint, and listed by `niles tools list`. Tool results are data, not instructions, to the model: a plugin's text is wrapped as a tool result like any other.
- **Relevance.** Every tool's schema costs prompt tokens on every request it is sent with (`niles-tools/src/relevance.rs` measured one call at 3,497 tokens against a budget of 8,000 a minute). A plugin's tools form one relevance group with the manifest's `keywords`: they are sent when a request mentions one, or when nothing matched and everything is sent. A plugin with tools must declare keywords; the built-in rule that unclassified tools are always sent does not extend to plugins, because a plugin's author cannot see what that costs a household with ten of them.
- **Capability reference.** Niles writes a short reference entry per plugin from its description and tool descriptions, so "what can the spot price thing do?" has an answer.
- **Notifications** from `notify.say` go through `niles-notifications` with the plugin's name as their origin, so recall ("what did I miss?") says where they came from.

### Storage and secrets

Three Postgres tables, owned by `niles-db` and created on first use: `plugins` (the approved manifest and its sha256, the repo, the release's tag and page, the version, the component's bytes and sha256, the sanitised logo, the same for the previous version, enabled, the newer release the last update check found, and the notice about an automatic rollback), `plugin_storage` (plugin id, key, value) and `plugin_devices` (the devices Niles holds for each). The last two reference the first, so removing a plugin's row removes its data with it.

**Settings** are not in those tables but in Niles's [runtime configuration](#runtime-configuration), as `[plugins.<id>]` (`[plugins.spotprice]` with `area = "DK1"`): the same store, history and undo as every other setting, kept in Postgres with the other overrides, and hot — a change to a plugin's table restarts that plugin, as saving its setup panel does. They are typed and checked against the manifest when the plugin reads them; a value that does not fit falls back to the default. **Secrets** live in the existing encrypted secret store as `plugins.<id>.<key>`, the same store and key as the credentials typed into Settings; the Integrations page can say whether one is set and never what it is. `PUT` and `DELETE /secrets/plugins.<id>.<key>` accept a key only when that plugin's manifest declares it as a `secret` setting, `GET /secrets` lists each declared one, and saving or clearing one restarts the plugin, as a settings change does. Removing a plugin deletes its rows from all three tables, its settings and its secrets.

Storage is the plugin's to use as it likes: caches, a cursor, a token it refreshed, a little history. A write that would take the plugin over 1 MiB fails with `limit`.

### Failure handling

- **A trap or a timeout** (a panic, an out-of-bounds access, the memory cap, a call past its deadline) discards the instance, since its memory may be half-updated. Niles logs it, marks the plugin's devices unavailable, and starts a fresh instance with `init` after a backoff: 5 s, 30 s, 2 min, 10 min, then every 30 min. The card reads "Not working: timed out in on-timer after 10 s" or "Not working: trapped in call-tool: out of memory", with the plugin's last log lines beneath.
- **Five restarts within an hour** stop the plugin: "Stopped after repeated failures", with a Restart button. A plugin that cannot start does not get to spend the house's CPU trying.
- **`init` returning `err`** is a plugin telling the household what is wrong ("The address is not set"). The text is the status line; Niles tries again when the settings are saved, and every 10 minutes.
- **While a plugin is down** its tools stay registered, and calling one returns "Spot prices is not working right now" to the model, which can say so; its pages show the status line instead of a tree. Its devices are unavailable, which rules already skip.
- **A failing tool call** (`err` from `call-tool`) is not a plugin failure. The message goes to the model as the tool's failure. Only traps and timeouts restart the instance.
- **Settings saved** restart the plugin: `stop`, then a fresh instance and `init`. One code path, and no plugin has to handle settings changing underneath it.

### Versioning the interface

The package is `niles:integration@0.2.0`, and a component records the exact version it was built against in its imports. wasmtime treats `0.2.x` versions as compatible with each other, so:

- **A patch release** (0.1.1, 0.1.2) may add functions to an imported interface and add new interfaces. Older plugins keep working because they never import what they do not know. It may not change or remove anything that exists, add a case to an existing variant or enum, or add an export.
- **A minor release** (0.2.0) may change anything. Niles runs plugins built for the previous minor version alongside the new one for at least six months, linking both worlds, and the Integrations page marks the old ones "built for an older Niles" with a link to their repo.
- **The manifest's `interface`** lets Niles refuse an unsupported plugin before downloading it; the component's own imports are what Niles actually checks, and the two must agree: a component whose imports say 0.2 (or that exports `on-mqtt-message`) under a manifest that says 0.1, or the reverse, is refused at check and install.

**0.2** is the first minor release: 0.1 plus the `mqtt` interface and the exports `on-mqtt-message` and `on-mqtt-status`. Adding exports changes the world, so a 0.1 component could not be linked as 0.2; 0.2 is therefore a new package version, and Niles links both. The WIT is two files: `schemas/niles-integration.wit` is 0.2, complete, the file authors copy (so bindings stay `niles::integration::*` from version to version); `schemas/niles-integration-0.1.wit` is 0.1, frozen as it shipped. Every interface the two share is identical in both, under its own package version. In the runtime (`crates/niles-plugins/src/bindings.rs`) each world has its own `bindgen!`, and the 0.2 one is told (`with`) to use the 0.1 bindings' modules for every shared interface instead of generating its own: one Rust type per WIT type and one `Host` trait per interface, implemented once, and the same argument types for the exports of both worlds. Only `mqtt` is generated for 0.2. The linker defines each shared interface under both names, `niles:integration/log@0.1.0` and `…@0.2.0`, with the same host functions behind them, through the per-interface `add_to_linker_instance` bindgen generates (wasmtime type-checks structurally at link time, and the types are the same), plus `mqtt@0.2.0`. A component is prepared as one world or the other by its manifest's `interface`, and a small enum dispatches each export call to the right bindings; a 0.1 plugin has no MQTT to deliver. `niles plugin check` reports which world a component targets, and the "built for an older Niles" note now marks 0.1 plugins.
- **The UI vocabulary** is part of the interface: a new node type is an addition in a patch release, and a plugin using it needs a Niles that has it, which `min_niles` says.

### The trust model, honestly

The sandbox limits what a plugin can **reach**. It does not make a plugin **trustworthy**. Once approved, a plugin sees everything its declared hosts and devices give it, and can do with them what their APIs allow: a plugin given an account's password can do anything that account can, and one subscribed to device state learns when the house is busy and when it is empty, and can send that to any host it declared. It can tell the model things in a tool result, and the model may believe them. It can report a device state that is not true.

What the sandbox does guarantee: it cannot reach hosts or local addresses it did not declare, read any other integration's secrets, Niles's database, the conversations, the memory or the people, run code in the app, or take the house down with it (memory and time are capped, and a plugin that keeps failing is stopped).

So the approval screen states plainly what is being granted, the repo is shown for anyone who wants to read the code, the component is pinned so it cannot change after approval, and an update with new permissions has to be approved again. Showing GitHub build-provenance attestations ("built by GitHub Actions from commit abc1234") at install is a natural next step; it is not built.

### Developer tooling

- **A template repo**, `niles-integration-template`, in Rust: a manifest, the WIT, a `lib.rs` with every export stubbed, a logo, and a GitHub Actions workflow that builds the component on a `v*` tag and attaches `niles-integration.toml`, `integration.wasm` and `logo.svg` to the release.
- **`niles plugin dev <path-or-wasm>`** runs one plugin with logs on the terminal, in the same sandbox and limits against a stand-in house: devices it reports and their changes are printed, storage is a file beside it (`.niles-dev/storage.json`), settings come from a local `dev-settings.toml` and `--settings key=value`, HTTP goes out for real under the manifest's permissions, timers fire, and a prompt calls tools (`tool prices {"day":"tomorrow"}`), commands devices, sends events and renders pages as text. `--call <tool> <json>` and `--render <surface>` do the same once after `init`; `--dry-run` does them and exits instead of staying up. **Not built yet: `--live <niles-url>`**, which would install the file on a running Niles as a development plugin, through the API with an API token, reinstall it whenever the file changes, and stream its log back; there it would be marked "Development" on its card.
- **`niles plugin check <path-or-repo-url>`** validates what an install would: the manifest, the tag, the logo, the component's world and version, the tools' names, descriptions and schemas, keywords present when there are tools, and the UI surfaces against the manifest. Given a repo (`jane/niles-spotprice`, or its address) it downloads the latest release, or the tag the address names, and checks that as an install would. The template's workflow can run it before publishing.

### Where this lives in the codebase

A new crate, `niles-plugins`:

- The runtime: wasmtime engine and linker for both interface versions (`bindings`), the host side of each interface, the per-plugin instance with its queue, limits and restarts, and the MQTT connections it holds (`mqtt`)
- Install, update and remove: GitHub releases, manifest validation, hashing, logo sanitising
- Adapters that make a running plugin a device source, a set of tools, a relevance group and a provider of UI surfaces, so the rest of Niles meets plugins through the interfaces it already has
- The `niles plugin dev` and `niles plugin check` subcommands, wired in `niles-bin`

`niles-db` gains the tables above, `niles-api` the `/plugins` routes (the list, pages and actions, install with its preview, update checks, updates and rollback, enable, disable, remove, the logo, the log and the image fetch; settings go through `PATCH /config` and secrets through `PUT /secrets`), and `web/` the Add from GitHub flow, the plugin's row and panel with its generated setup panel and log, the renderer for the UI vocabulary, and the plugins' pages in the menu and widgets on the dashboard. In `niles-plugins`, `log` keeps each plugin's last lines and `images` fetches and keeps a page's pictures. In `niles-plugins`, `github` fetches releases, `permissions` words them, `check` holds the logo sanitiser, and `install` is the two-step flow, the update checks and the rollback watch. The WIT lives at `schemas/niles-integration.wit` (0.2) and `schemas/niles-integration-0.1.wit` (0.1, frozen), the author guide in `docs/integrations/`.

The seams the rest of Niles meets plugins through, chosen so nothing outside `niles-plugins` knows about WebAssembly:

- **Commands.** `niles_mqtt::CommandRouter` turns a desired state into a Zigbee or WLED message by the device's source. Beside it sits a `DeviceCommander`, which the plugin runtime is, owning every `x_*` source. Every path that sends a command — the app's device and room routes and scenes, the `set_device`/`set_room`/`set_blinds` tools, Tier 0, the automation engine's sink, wall switches, the curve and the morning routine — asks `commander_for` first and formats an MQTT message only when nothing else owns the device. A commander rather than a pseudo-topic through the publisher, because a plugin's `set-device` answers ("The pump did not answer.") and a publish does not.
- **Tools.** `niles_tools::ToolRegistry` gains a dynamic section, `DynamicTools`: a shared handle the runtime replaces a plugin's tools through as it starts, restarts and goes, which every registry attached to it — the voice model's, and through it the MCP endpoint the assistants use — sees on the next request. Relevance is checked against each plugin's keywords there; a built-in's name always wins.

### Neighbouring routes

Two other ways to bring things into Niles without a crate, deliberately outside this interface.

#### Home Assistant MQTT discovery

Many devices already announce themselves over MQTT in Home Assistant's discovery format: a retained JSON config under `homeassistant/<component>/<node>/<object>/config`, naming the state and command topics. ESPHome does it by default; Tasmota, Shelly (through scripts) and rtl_433 (through its bridge script) can. Niles could read those configs as a device source on the broker it already uses, mapping `light`, `switch`, `cover` and `binary_sensor`/`sensor` device classes onto its own classes and fields, with the room from the device's `suggested_area` or a `<room>/<device>` name. That is a no-code route for devices, complementary to plugins, and a separate piece of work: it needs no sandbox, because Niles runs no foreign code, only foreign JSON.

#### MCP servers by URL

Somebody who already runs a service with an MCP server (a home lab dashboard, a notes system) could add it under Integrations by URL, with an optional bearer token. Niles would act as an MCP client over Streamable HTTP, list the server's tools, and offer them under the server's name to the Claude and ChatGPT assistants, and to the voice model with keywords the household gives. Nothing runs inside Niles, so no sandbox is needed. The questions are what the server's tools do and what tool arguments Niles sends it, and both are shown when it is added. It provides tools only: no devices, no pages.

### What this section does not specify

- A central index of plugins, or any review of them
- Private repos, and repos hosted anywhere but GitHub
- Per-plugin CPU budgets beyond the per-call deadline
- Reading another source's devices on demand: the interface has events, not a snapshot
- `wasi:http/outgoing-handler`, so that language-native HTTP clients (`fetch` in JavaScript) would work under the same checks

## User recognition

Niles identifies the people in the household by voice. This enables personalization (greeting by name, routing "my X" references to the right person) and access control (rules about who can do what, covered in the next section).

Recognition is opt-in via introduction. There is no enrollment ceremony, no biometric setup wizard. The same person signing in to the web surface is the *same person*: an entry in `[auth] allowed` names the speaker it belongs to, so voice and web share one identity and one memory rather than keeping two. See [Signing in](#signing-in). The first time someone is identified, it's either because they introduced themselves or because Niles asked. Over time, their voice profile gets stronger from repeated successful matches.

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
  calendars (the connections they made, see Calendars)
  github_username (optional)
  linear_user (optional)
  bedroom (which room is this user's bedroom, for "wake me up" routing)
  created_at
```

The LLM uses these when a personal reference appears:

- "What's on my calendar?" → queries the speaker's own [calendar connections](#calendars)
- "Wake me up at 06:30 tomorrow" → looks up `speaker.bedroom`, creates a routine targeting that room
- "Did my task get a PR yet?" → looks up `speaker.linear_user`, queries Linear scoped to that user

If the speaker is unknown and a personal reference is used, Niles prompts for identification before proceeding.

### Privacy posture

Voice fingerprints are biometric data. Niles is deliberate about handling:

- **Stored on your own infrastructure only.** Embeddings live in the cluster Postgres (as `vector` columns via pgvecto.rs). They are never sent to Groq, Anthropic, or any cloud provider. Whisper receives audio for transcription, but Whisper does not do speaker ID.
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
| Unknown speakers cannot file AI-eligible tasks | Triggers real code-gen; user must be known |
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

- Stores rules in Postgres
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

Several places in Niles benefit from "when X happens, do Y" logic: presence-triggered lights, sunset-triggered porch lamps, "when the office door opens to the garden, turn off the office lights." These are *automations* — a first-class concept in Niles, not bolted on later.

### Two shapes

Automations come in two forms:

**Code automations** — complex, foundational behaviors implemented as Rust. Examples: the ambient lighting curve, the morning routine, timer escalation. These are too rich (state, hysteresis, interactions with other subsystems) for a rule and are part of the core.

**Rules** — simple "when X, do Y" automations, made by voice or on the Automations page and stored in Postgres. Examples: "when the office door opens, turn off the office lights"; "when home becomes occupied after sunset, turn on the hallway light." These cover the long tail of household-specific behaviors.

The split keeps complex behavior testable in code while letting the household add its own automations without writing Rust.

### Rule shape

A rule is one or more **triggers**, any number of **conditions** (all must hold when a trigger fires), and one or more **actions**, run in order:

```json
{
  "id": "office-lights-and-the-garden-door",
  "description": "Office lights and the garden door",
  "enabled": true,
  "triggers": [
    { "id": "opened", "type": "contact", "device": "office/door", "open": true },
    { "id": "left-open", "type": "contact", "device": "office/door", "open": true, "for": { "minutes": 10 } }
  ],
  "conditions": [ { "kind": "sun", "after": "sunset", "before": "sunrise" } ],
  "actions": [
    { "do": "if",
      "conditions": [ { "kind": "triggered_by", "triggers": ["opened"] } ],
      "then": [ { "do": "set_room", "room": "office", "on": false } ],
      "else": [ { "do": "notify", "body": "The garden door has been open for ten minutes", "room": "living_room" } ] }
  ]
}
```

The vocabulary is deliberately narrow and grows only as new needs appear:

| | |
|---|---|
| **Triggers** | `contact` (a door or window opens / closes), `device_state` (a device turns on / off), `numeric` (a number — temperature, humidity, battery, brightness, light level, or any named reading the device reports, such as `price`, `power` or `co2` — goes above or below a value), `device_action` (a button or dimmer press), `presence` (the house becomes occupied / unoccupied), `time` (a time of day, on chosen weekdays), `sun` (sunrise or sunset, with an offset), `timer_fired` |
| **Conditions** | `time_of_day` (a window, wrapping past midnight, on chosen weekdays), `sun` (after / before sunrise or sunset, with offsets), `device_is`, `contact_is`, `numeric` (a reading above / below a value), `presence_is` (the house occupied / unoccupied), `triggered_by` (which of the rule's triggers started this run) |
| **Actions** | `set_device`, `set_room` (every light in a room), `notify` (spoken in a room), `delay` (wait, then go on), `if` (conditions, then one list of actions, else another) |

No general scripting language — that would be a security and complexity disaster. Devices are named the way people and the LLM name them (`office/door`); the source prefix (`z2m:`) is filled in by Niles.

**Several triggers.** One rule can start several ways, so a pair of rules — "when the door opens…" and "when it closes…" — is one. Each trigger has an id, given by Niles (`t1`, `t2`…) when the person or the LLM gives none; `triggered_by` names the triggers it means by their ids, and the app shows them by their position and sentence, never asks for an id. A `triggered_by` naming a trigger since removed is refused when the rule is saved.

**How long.** A `contact`, `device_state`, `numeric` or `presence` trigger can carry `for` (hours, minutes, seconds): it fires once its state has held that long — "the door has been open for ten minutes" — and not at all if the state changes before then.

**Branches.** `if` takes the same conditions a rule does, and its `then` and `else` are action lists of their own, which may hold an `if` in turn, to a depth of three. That, with `triggered_by`, is how one rule does different things for different triggers.

**Limits.** At most ten triggers; `for` and `delay` at most 24 hours. Sun triggers and conditions need the house's location (`[home] latitude` / `longitude`); a rule that uses the sun is refused while there is none.

A rule saved before triggers were a list — `"trigger": {…}` — reads as a list of one, with the id `t1`; it is written back as a list the next time it is saved. TOML `[[automation]]` tables may use either.

### When a rule runs

- **One run at a time, the newest wins.** A rule that is still running — waiting out a `delay` — when one of its triggers fires again stops where it is and starts over. That is what "turn the lights off five minutes after the door closes" needs: if the door opens again in the meantime, the lights are left alone.
- **The clock.** `time` and `sun` triggers fire on the minute. Sunrise and sunset are worked out each day for the house's location, in its time zone.
- **What is pending is forgotten on a restart.** A `delay` under way, or a `for` not yet held long enough, does not survive Niles restarting; the next change starts it again.
- **Every run is kept**, the last twenty per rule, for as long as Niles runs: when it started, which trigger, and how it ended — done (and what it did), stopped by a condition (and which one: "it was before 07:00"), replaced by a newer run, or failed. A trigger that matched but whose conditions did not hold is a run too, because "why didn't it turn off?" is exactly the question it answers. The Automations page shows them under each rule, and Niles answers it by voice.

### Seeing and editing rules

The [web UI](#the-web-ui) has an **Automations** page, a menu entry like Groceries or Chat. On a wide screen each rule opens as a flow drawn with React Flow: its triggers at the top, its conditions below them, its actions at the bottom, joined in order — an `if` splitting into its two branches, a `delay` a step of its own. Nodes are added from a palette and edited in a side panel; the layout is automatic (dagre), so positions are not stored — the rule is the record, the flow is a picture of it. On a phone the page lists every rule as its sentence ("When the office door opens, turn off the office lights") with a switch to pause it; the flow can be viewed but is built on a wider screen.

The page is a convenience, never a requirement (principle 8): every rule can also be made, listed, paused and removed by voice.

### Voice-created rules

> User: "Niles, when the office door opens, turn off the office lights."

The LLM extracts the trigger, conditions and actions and **reads the rule back before saving it**:

> Niles: "When the office door opens, I'll turn off the office lights. Shall I?"

Only a yes saves it. Removing a rule by voice is confirmed the same way. The rule then appears on the Automations page like any other, so a misheard one is visible and can be fixed or removed by hand.

### Rule lifecycle

- **List:** "Niles, what automations do I have?" → reads them back as their sentences
- **Pause:** "Niles, pause the garden door automation" → disabled until switched back on (by voice or on the page)
- **Remove:** "Niles, remove the garden door automation" → matches by name or sentence, confirms, deletes

### Conflicts and ordering

A rule is the household's own explicit intent, so it acts even on a light in manual mode: manual mode holds off the *lighting curve*, not a rule someone made on purpose. (The curve then treats a light a rule turned off as off; it does not turn it back on.)

When several rules fire on the same event they run in the order they were created. Most rules don't conflict in practice (different rooms, different devices). If two genuinely conflict, the later one wins for that device — which the household can see and reason about on the Automations page.

### Where this lives in the codebase

`niles-automations`:

- The rule model and the engine: subscribes to the internal event bus, evaluates conditions against the device registry, dispatches actions through the same command router the LLM's device tools use
- Rules are stored in Postgres, in an `automations` table owned by `niles-db` — one row holding the whole ordered set, as scenes and the shopping list are kept: there are a handful, they are read together, and the order is part of the record. The engine reloads when a rule is saved, paused or removed — no restart
- `[[automation]]` tables in the TOML config, which predate the table, are still loaded, after the stored rules; the page shows them read-only, marked as coming from the config
- HTTP: `GET/POST /automations`, `PUT/DELETE /automations/{id}`, `GET /automations/{id}/runs`
- LLM tools for the voice lifecycle: create (after confirmation), list, pause / resume, remove, and a rule's recent runs ("why didn't the office light turn off?")

### Permissions interaction

Anyone in the household may make, pause or remove a rule; the read-back confirmation and the page are the guard against a misheard one. When `niles-permissions` lands, creating and removing rules is the first candidate to become admin-only.

## Runtime configuration

"Code/config over UI" is a statement about *structure*, not about every
value. Two kinds of setting live in the config file, and they behave
differently:

- **Structural config** — the MQTT broker, device topology, listen
  addresses, credentials, which integrations exist. Deployed from git,
  read once at startup, never editable at runtime. Getting one wrong is
  a deploy to fix, which is correct: these describe the house's wiring.
- **Experiential config** — the lighting curve, ambient behaviour,
  automation thresholds. Values tuned by feel, repeatedly, against
  lights you are looking at. Round-tripping those through a git commit
  and a pod restart is friction with no safety benefit.

The second kind is editable while Niles runs, from three surfaces
sharing one store: the config UI, the `update_config` voice tool, and
`PATCH /config`.

### How it works

The ConfigMap is the **base**. A writable **override document** layers
over it, and the result is validated as a whole before it replaces the
running snapshot — so a rejected value leaves the previous config in
force rather than a half-applied one. Overrides are persisted before
the swap, so a write that reaches memory but not storage cannot
silently revert on the next restart. A capped revision history makes
undo possible and shows who changed what, including by voice.

### Hot and boot sections

Each top-level section is classified:

- **Hot** — re-read by the task that owns it, so a change lands on the
  next tick. `lighting` and `ambient_lights` today, and `auth` when it
  lands, for the reason given under [Managing people](#managing-people).
- **Boot** — built once at startup and moved into a task that owns it.
  Everything else.

This is a statement about the *implementation*, not about intent: a
section is hot only when something actually re-reads it. The
classification is exposed rather than hidden — the UI marks boot
sections as needing a restart, and the voice tool **refuses** to edit
one, because storing a change nobody can act on is worse than declining
it out loud.

Credential fields (`*_env`) are never editable and never rendered. They
are env-var *names*; the values live in the secret store and never pass
through this path.

### Storage

Overrides live in Postgres, beside the rest of Niles's state. A
node-local volume was the obvious first choice and the wrong one: it
pins the pod to one node and blocks drains, which on a Talos cluster
means every upgrade. Niles owns its tables and creates them on first
use. Where a deployment has no database, changes still apply — they
just don't outlive the process, and every surface says so.

## The web UI

One page, served by the same binary behind the `ui` cargo feature, with
two halves that answer to different rules.

**Home** is the house: every room Niles knows of, and the lights in it.
Pressing a room switches all of its lights; opening one gives each
light its own power, brightness, and — where the light has them —
colour and colour temperature. Nothing here is configured. The rooms
exist because devices are in them, so a light discovered in Z2M turns
up on the dashboard with nobody naming it twice, which is the same
no-UI rule stated under [device naming](#device-naming-convention-the-no-ui-strategy)
rather than an exception to it.

**Settings** is [Runtime configuration](#runtime-configuration): the
curve, ambient behaviour, the revision history, and an undo.

**Automations** is the household's [rules](#automations), each drawn
as a flow from its trigger to its actions, and made or changed there as
well as by voice.

Who may open either is [Signing in](#signing-in).

### Why a control surface at all

Voice is the primary interface and stays that way. But a voice command
is a poor fit for two things a hand is better at: *surveying* ("which
lights are still on?" is one glance and several sentences), and
*adjusting by eye* (a brightness you find by dragging, not by naming a
percentage). Neither is a setting, so neither belongs in the config
surfaces; both are what a phone in a hallway is for.

### Installing it

The page is a progressive web app, so it can live on a Home Screen and
open without browser furniture — which is what makes "press the hallway
off on your way past" a thing anyone actually does.

The service worker behind that **caches nothing, deliberately**. This
page exists to say which lights are on, and a cached copy is a page
confidently describing a room the reader can see is wrong. Nor is there
anything worth having offline: every control needs Niles at the other
end. So the usual trade of "stale but present" buys nothing here and
costs correctness.

It still has to exist, for two reasons. Chrome declines to offer
installation to a page whose worker has no `fetch` handler — so there
is one, and it is empty, which leaves every request to the network
exactly as if it were absent. And a push notification can only be
received by a service worker, which is where a fired timer will reach a
phone.

There is no install button in the page. Every browser already has one,
Safari's cannot be triggered from script, and a button that guesses at
Chrome's can only reproduce it badly.

### What it talks to

The registry it draws comes from `GET /devices`, which reports each
device's capabilities, so the UI can offer a colour wheel only to
lights that have one. Commands go to `POST /rooms/{room}/{device}` for
one light and `POST /rooms/{room}` for all of them — the room fan-out
happens server-side, next to the broker, rather than as one request per
light from a phone.

State is **pushed, not polled**. The page holds the WebSocket at
`GET /events/stream` open and folds each `device_state_changed` frame
into what it already knows. This matters because the UI is not the only
thing moving these lights: the curve does at sunset, voice does, a wall
switch does. A polled page would spend a few seconds describing a room
the person holding it can see is wrong. Frames carry only what changed,
so they merge rather than replace — a frame saying `{on: false}`
carries nothing about brightness, and treating that as "brightness
cleared" would lose it. Readings merge the same way, name by name.

Presses are shown immediately and reconciled when the light reports.
A command that fails rolls the row back and says why.

## Signing in

### Why there is a sign-in at all

Not because the network is hostile. Niles sits on the house network,
its satellites stream audio to it unauthenticated over Wyoming, and it
talks to the broker over MQTT. None of that changes, and none of it
gains a login.

The reason is that **a house has more than one person in it**. A timer
set on one phone should ring on that phone. What Niles remembers about
one person should not be read back to another. Voice already knows who
is speaking — [speaker recognition](#user-recognition) has known since
it shipped — and the web surface did not, which meant the two halves of
the same system disagreed about whether people exist.

Exposing Niles beyond the house is a **separate decision** that this
makes possible rather than requires. Worth knowing before taking it:
web push does not need Niles to be reachable from outside. A push
subscription lives on Apple's or Google's service and Niles only ever
posts to it outbound, so notifications arrive on a phone anywhere
without a single inbound port.

### Who gets in

Sign-in is GitHub's OAuth app flow. GitHub is the **front door, not the
boundary** — accounts there are free, so "has a GitHub account" tests
nothing.

The boundary is a list in config:

```toml
[auth]
github_client_id_env = "NILES_GITHUB_CLIENT_ID"
github_client_secret_env = "NILES_GITHUB_CLIENT_SECRET"
allowed = [
  { email = "you@example.com",     speaker = "mark" },
  { email = "someone@example.com", speaker = "majse" },
]
```

Four things follow from the list being config rather than a table:

- **Adding a person is an edit, not a message.** No invitation is sent,
  no token is minted, no expiry is tracked, and no mail server has to
  be configured for a household of two. Removing a person is deleting a
  line, and it holds from the next request.
- **Niles never creates an account.** There is nothing to create. A
  verified address that is not in the list is refused, and the refusal
  names the address GitHub vouched for — which is the whole of the fix,
  because the usual cause is a list entry pointing at a different one.
- **The address is the one GitHub verified**, read from `/user/emails`.
  The `email` field on a GitHub profile is whatever the account holder
  typed, and anybody can type yours.
- **`speaker` ties the two identities together**, so the person who
  says "I am Mark" and the person holding the phone are one person and
  share one memory. It is optional; without it the account is
  web-only.

The client secret follows the same `*_env` convention as every other
credential, so it is a variable *name* in config and its value never
passes through the config UI.

### What a session is

A cookie, `HttpOnly; SameSite=Lax; Secure`, and nothing else kept on
the device.

GitHub's OAuth app flow has no discovery document and no ID token, so
there is no signature on the way back to check. What stands in for one
is a pair: a **single-use, expiring `state`**, and a **cookie binding
the attempt to the browser that began it**. Both must match, or the
callback is refused. The destination afterwards is a same-origin path
or nothing — an absolute URL there would turn the sign-in page into a
phishing hop that authenticates against the real Niles and lands
somewhere else.

**The cookie carries the signed address, and the allowlist is read
again on every request.** This is what makes removing a line from
config actually remove somebody: a session signed months ago has no
idea the list changed, so a cookie that carried a bare session id would
outlive the decision to revoke it. Re-reading costs a lookup in a
config snapshot already held in memory, and it means there is no
session table to build, expire, or forget to clear.

Two consequences worth stating rather than discovering:

- The **signing key has to outlive a restart**, or every deploy signs
  everybody out. It is a secret like any other, referenced by env-var
  name.
- `Secure` means **sign-in only works over HTTPS**. A browser will not
  store the cookie over plain HTTP, so reaching Niles at
  `http://<address>:8080` cannot sign in even on the house network.
  Either everything goes through the HTTPS name or that path stays
  anonymous, and choosing is better than finding out.

### Callers that are not browsers

A cookie is for a person with a browser. Niles is also read from a
terminal — `/logs` and `/devices` are the fastest way to answer "why
did it do that", and that is the reason the endpoint exists at all.

So there is also a **bearer token**, one value, referenced by env-var
name, compared in constant time. It is not a lesser session: it is the
operator, and it is not subject to the allowlist because it is not a
person. Cookie for people, token for scripts.

### Managing people

Adding somebody is done in **Settings**, not by editing a file. That is
the reason the allowlist is config in the first place: config is already
runtime-editable from the UI, the voice tool and `PATCH /config`, so
"let Majse in" is a field rather than a deploy.

Three things have to hold for that to be true rather than merely
plausible.

**`auth` is a hot section.** It has to be, or adding somebody would
need a restart — which is the friction the whole design exists to
avoid. It *can* be, for the same reason the allowlist is re-read on
every request: nothing caches it. The list is read from the live
snapshot at the moment a request is authorised, so a change lands on
the next one. This is one mechanism serving two purposes — adding a
person takes effect immediately, and so does removing one.

**The last person cannot be removed.** An empty list would mean nobody
can sign in, and the page offering that button is the page you would
need to be signed in to reach. So a write that would empty the list is
refused, and says why, rather than being accepted and locking the house
out of its own settings. Removing *yourself* is allowed when somebody
else remains — it is a legitimate thing to want, and the person still
listed can undo it.

**A signed-in person can add anyone.** There are no roles here (see
below), so the allowlist is self-amending by design: whoever is in the
house can let another person in. That is the correct shape for two
adults sharing a home and the wrong shape for anything larger, and it
is the assumption to revisit first if Niles ever has a user who is not
a householder.

If it is ever locked out anyway, the bearer token is the way back in —
it is not subject to the allowlist, so `PATCH /config` from a terminal
can always restore it.

### What stays outside it

- `/healthz`, because a liveness probe cannot hold a session.
- The sign-in routes themselves, which exist to get a request *past*
  this.
- `/webhooks/linear`, which authenticates itself with an HMAC over the
  body. A cookie would mean nothing to Linear.
- Wyoming and MQTT, which are not HTTP. **Only the HTTP API gains a
  login**, and if Niles is ever exposed, only that port goes out.

The *page* is also served to everyone, and shows a sign-in screen when
nobody is signed in. Redirecting a navigation instead would mean the UI
never gets to ask who it is talking to — and every request it would
make behind that screen is refused anyway. An API call without a
session gets `401` and a JSON body; a navigation gets the app.

### If Niles is ever exposed

Signing in makes this possible. It does not make it safe on its own,
and two of the following are true whether or not anything is exposed.

- **The WebSocket checks `Origin`.** WebSockets are not subject to the
  same-origin policy, and the browser attaches cookies to the
  handshake — so once a session exists, any page a household member
  visits could otherwise open `/events/stream` and read the live device
  stream, which is to say when the house is empty. `SameSite` helps in
  current browsers; an `Origin` check is the portable answer and does
  not depend on the browser being current. A handshake with no `Origin`
  is allowed: browsers always send one, so its absence means a caller
  with no cookie to borrow.
- **Every state change stays a `POST`, `PATCH` or `DELETE`.** That is
  what makes `SameSite=Lax` sufficient against cross-site requests, and
  it is a property to preserve rather than a defence to add. A
  state-changing `GET` would quietly undo it.
- **`/logs` changes character.** On the house network it serves voice
  transcripts to people already in the house. On the internet it serves
  them to whoever holds a session, which is an argument for keeping the
  allowlist short rather than for hiding the route.
- **Only 8080 leaves.** Wyoming and MQTT stay where they are; the
  satellite streams audio unauthenticated and that is not changing.
- **The OAuth client secret becomes house access**, so it lives in the
  secret store like every other credential and never in the ConfigMap.

None of this is required to get a notification to a phone that is out
of the house. A push subscription lives on Apple's or Google's service
and Niles only posts to it outbound, so a timer reaches a phone
anywhere without a single inbound port. Exposure buys opening the app
from the bus, and should be weighed as that rather than as a
prerequisite.

### What this deliberately is not

- **No roles and no admin screens.** Two people in a house are both
  trusted with the house. A permission model would be machinery
  guarding a distinction nobody in the household is making — with the
  consequence, stated above, that anyone signed in can let anyone else
  in.
- **No passwords**, so no reset flow, no lockout policy, and no second
  factor to build. The account Niles trusts is the GitHub account, and
  keeping that safe is GitHub's job.
- **No sign-up.** Ever. The list is the only way in.

## Deployment

Niles supports two deployment paths as first-class options: Docker Compose and Kubernetes. Most users will choose Compose; Kubernetes is for users running multi-node clusters.

Both paths consume the same canonical Niles config (TOML), the same API keys (env vars), and produce identical runtime behavior. The Niles service has no knowledge of which deployment style launched it.

### Persistence

Niles uses a **single Postgres instance** (with the [pgvecto.rs](https://github.com/tensorchord/pgvecto.rs) extension for vector storage) as the canonical store for *all* runtime state:

- Timers, scenes, voice-created automations, permission rules, notification history, per-room music state — relational tables.
- User voice embeddings — `vector` columns, queried with pgvecto.rs's index-backed approximate nearest neighbor lookup.

This is a deliberate change from the originally-spec'd embedded SQLite. The reasoning:

- **Niles becomes stateless.** Restart the service freely; the database holds the truth. Multi-replica Niles becomes possible later without coordination.
- **One DB instead of two.** pgvector subsumes a separate vector store (Qdrant et al.) for the embeddings use case.
- **Operational lift comes free in a cluster.** If you're already running a Postgres (e.g. CNPG-managed), Niles plugs in. Backups, monitoring, certificate rotation, HA are all the cluster operator's concerns, not Niles's.
- **Latency cost is negligible.** A typical in-cluster Postgres query is sub-millisecond; the voice loop is bounded by STT + LLM + TTS at 400–1300 ms. The DB never sits in a hot path: the registry is in memory, the curve is pure math, timers are tracked in memory and merely *persisted* to the DB on creation/restart.

#### Operational expectations

- **Operator:** [CloudNativePG](https://cloudnative-pg.io/) is the recommended way to run Postgres in a cluster. It handles pod lifecycle, certificates, secret rotation, and (when configured) replication + WAL archiving for HA.
- **Extension:** install [pgvecto.rs](https://github.com/tensorchord/pgvecto.rs) (or any pgvector implementation) at cluster bootstrap so Niles's `CREATE EXTENSION vector;` migration can run.
- **Backups:** CNPG's `Backup`/`ScheduledBackup` CRDs against an S3-compatible target (Cloudflare R2, Minio, etc.). Niles assumes someone has backups; it doesn't implement them itself.
- **Schema migrations:** the Niles binary runs migrations on startup (via `sqlx::migrate!` or `refinery`). A failed migration aborts startup with a clear error; nothing partial.
- **Single-tenant assumption (v1):** one Niles database per home. Schema is not designed for multi-home tenancy.

#### Connection config

A new `[database]` section in `niles.toml`:

```toml
[database]
host = "postgres-rw.database.svc.cluster.local"
port = 5432
database = "niles"
username_env = "NILES_DB_USERNAME"
password_env = "NILES_DB_PASSWORD"
# Optional; defaults shown
# sslmode      = "prefer"   # prefer / require / disable
# pool_max     = 10
```

Same secrets pattern as `[mqtt]`: TOML carries env-var *names*; values come from `.env` in local dev or a k8s `Secret` in deploy.

#### Local dev

Two viable patterns:

1. **Port-forward to the cluster Postgres:** `kubectl port-forward -n database svc/postgres-rw 5432:5432`. Niles connects to `localhost:5432`. Easiest if you already have the cluster.
2. **Local Postgres in a throwaway container:** `docker run` (or a podman-equivalent on Linux) for ad-hoc dev. Niles's migrations bootstrap the schema.

Unit tests do *not* require a running Postgres — pure-logic code is testable against in-memory stubs. Integration tests that exercise the DB path are gated behind a `DATABASE_URL` env var; CI doesn't run them.

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

  postgres:
    image: tensorchord/pgvecto-rs:pg16-v0.4.0  # Postgres 16 + pgvecto.rs
    volumes:
      - ./postgres-data:/var/lib/postgresql/data
    environment:
      POSTGRES_DB: niles
      POSTGRES_USER: ${NILES_DB_USERNAME}
      POSTGRES_PASSWORD: ${NILES_DB_PASSWORD}
    restart: unless-stopped

  niles:
    image: ghcr.io/<org>/niles:latest
    volumes:
      - ./niles-config.toml:/etc/niles/config.toml:ro
    environment:
      GROQ_API_KEY: ${GROQ_API_KEY}
      ANTHROPIC_API_KEY: ${ANTHROPIC_API_KEY}
      TADO_API_TOKEN: ${TADO_API_TOKEN}
      NILES_DB_USERNAME: ${NILES_DB_USERNAME}
      NILES_DB_PASSWORD: ${NILES_DB_PASSWORD}
    ports:
      - "10300:10300"  # Wyoming protocol
      - "8080:8080"    # HTTP API
    depends_on:
      - mosquitto
      - piper-tts
      - postgres
    restart: unless-stopped
```

Plus a `.env` file (gitignored) with the actual secret values:

```
GROQ_API_KEY=gsk_...
ANTHROPIC_API_KEY=sk-ant-...
TADO_API_TOKEN=...
NILES_DB_USERNAME=niles
NILES_DB_PASSWORD=...
```

The Niles container itself has no persistent volume — all state lives in the bundled Postgres. Back up `./postgres-data` (e.g. via `pg_dump` cron or a sidecar) to protect your scenes, embeddings, timers, etc.

Setup is: clone the repo, edit `niles-config.toml` with home coordinates and feature toggles, create `.env` with API keys, `docker compose up -d`. Done.

The repo's `deploy/compose/` directory holds the reference Compose file, example config, and a `.env.example` users copy.

### Kubernetes

For users running a cluster. The repo includes a Kustomize base in `deploy/kubernetes/base/`:

- `mosquitto/` — Deployment + Service + PVC
- `zigbee2mqtt/` — Deployment + Service + PVC
- `piper-tts/` — Deployment + Service
- `niles/` — Deployment + Service, references a `niles-config` ConfigMap and a `niles-secrets` Secret. **No PVC** — Niles is stateless; state lives in cluster Postgres.

Cluster Postgres is *not* in the Kustomize base. The expectation is that a CNPG-managed Postgres already exists in your cluster (or is deployed via a separate Kustomize / Helm release that you own). The Niles `ConfigMap` references it by service DNS, and `niles-secrets` holds the credentials.

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
- Integration endpoints (Linear API token env, Sonos discovery hint, etc.)
- Database connection (host, port, db name; credentials via env-var names)
- Notification preferences (quiet hours, routing)
- Automations defined at deploy time

Secrets (API tokens) live in env vars referenced by the config. Deployment-specific delivery: `.env` for Compose, `Secret` for k8s.

Runtime state (scenes, voice-created automations, timers, user embeddings, permission rules, notification history) lives in the cluster Postgres. Niles itself is stateless — restart it freely; the database holds the truth. See the [Persistence](#persistence) subsection below.

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

### linear_integration
  description: File and query coding tasks in Linear by voice; an ai-harness runs the AI-eligible ones
  requires: a Linear workspace + API token
  api_keys: { linear: required }
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
│   ├── niles-integration-linear/ # Linear task integration (first integration); ai-harness runs AI-eligible tasks
│   ├── niles-recognition/      # speaker identification via voice embeddings
│   ├── niles-permissions/      # rules engine and admin concept; enforces tool access
│   ├── niles-presence/         # presence sources, aggregation, home-state events
│   ├── niles-automations/      # when-X-do-Y rules, voice-creatable and config-defined
│   ├── niles-plugins/          # third-party integrations: wasmtime runtime, GitHub install, sandbox
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
│   ├── device-types.json
│   ├── niles-integration.wit  # the plugin interface (WIT), 0.2
│   └── niles-integration-0.1.wit  # the 0.1 interface, frozen, still run
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
niles plugin dev <path>  # run one plugin locally, dry run or against a live Niles
niles plugin check <path-or-repo>  # validate a plugin before publishing
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
- Postgres persistence so timers survive service restarts
- Verify: common commands stay <800ms; how-to questions answer correctly; timers reliable

### Phase 5: Room speaker integration and music (1–2 weekends)
- SOAP/UPnP client for local Sonos control
- Sonos as an integration found from one speaker's address; Settings → Speakers gives each Sonos room a Niles room
- Sonos as both an LLM tool AND a ducking target during voice responses
- Multi-room awareness ("play music in living room")
- Music intent tools: `play_radio`, `play_music`, `play_podcast`, `resume_in_room`, transport controls, grouping
- Per-room music state in Postgres (last source, last content) including polling for app-initiated playback
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
- `niles-notifications` crate: routing rules, chime + voice formatting, Postgres persistence
- Quiet hours config with priority-aware handling
- "Last active satellite" tracking in `niles-core`
- `list_recent_notifications` LLM tool
- Verify: programmatic test notifications route correctly across rooms and respect quiet hours

### Phase 9: Presence and automations (2 weekends)
- `niles-presence` crate with adapter pattern for sources
- First adapter: Tado (HTTP API client, geofence-derived per-user state)
- Second adapter: manual ("Niles, I'm leaving") as supplement
- Home-state aggregation with hysteresis; presence events published to event bus
- `niles-automations` crate: rule loader, event subscription, condition evaluation, action dispatch
- Rules in Postgres, reloaded live; contact (door / window) triggers and conditions; a room-wide light action
- Automations page: rules drawn and edited as a React Flow chain (trigger → conditions → actions), listed with a pause switch on a phone
- Voice-creatable rules: LLM extracts trigger/conditions/actions, reads the rule back, saves on a yes; list, pause and remove by voice
- Concrete first automation: "when the office door opens, turn off the office lights"
- Verify: the door rule fires on the real sensor; a voice-created rule appears on the page, persists across a restart and fires

### Phase 10: First external integration — Linear (1–2 weekends)
- `niles-integration-linear` crate (talks to Linear's GraphQL API)
- Resolve team + trigger label ("AI Eligible") + initial state ("Todo") by name at startup
- LLM tools: `create_task(title, description)` (files Todo + AI Eligible so the ai-harness runs it), `list_tasks(status?)`, `get_task(id)` (state + linked PR)
- (Later) Linear webhooks → PR-ready / state-change events flow through `niles-notifications`
- Capability reference for Linear, loaded into Tier B
- Default rule: only known users can file AI-eligible tasks
- Verify: voice "add a task: …" files a Linear issue the harness picks up and turns into a PR; "did it get a PR?" reports status
- Note: niles does NOT talk to the harness directly — Linear (label + column) is the whole contract. Archon is dropped.

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
- Conversation memory (short-term in context, long-term in Postgres)

### Phase 13: Additional integrations and extensions
- Additional presence sources (phone-based, BLE)
- Calendar (Microsoft, Google, iCloud)
- Email integration
- GitHub direct integration
- Monitoring/alerting bridge
- RAG layer for documents/notes
- Frontends (Tauri 2 covers desktop + mobile from one codebase)
- Custom Rust firmware on the satellites
- More device sources (Shelly, Matter, Z-Wave)
- GPU node for fully-local STT/LLM
- Satellite-side speaker ID (faster, more private)
- Third-party integrations: WebAssembly plugins installed from a GitHub repo, with devices, tools, pages and widgets ([spec](#third-party-integrations-plugins), interface v0.2, with MQTT, in `schemas/niles-integration.wit`)
- Home Assistant MQTT discovery as a no-code device source (ESPHome, Tasmota, Shelly, rtl_433)
- MCP servers added by URL, their tools offered to the assistants

## What Niles is not

To keep scope honest:

- **Not a Home Assistant replacement for everyone.** If your home has lots of vendor-specific WiFi devices, complex Z-Wave/Matter setups, or you want a polished UI for non-technical family members, stay on HA.
- **Not a UI-driven system in v1.** Configuration is code, config files, and naming conventions in upstream sources. Adding a device means pairing and naming it correctly in Z2M — not clicking through Niles screens. If a UI is added later, it will be optional and used for monitoring/debugging, not required for setup.
- **Not a design or planning surface.** Niles is voice. Voice is a poor medium for design discussions, code review, long-form planning, anything that benefits from reading, scanning, jumping between sections, or seeing code and structured content. That work belongs at a computer (with Claude Code, a real editor, etc.). Niles's role in project work is *filing tasks* (a Linear issue the ai-harness runs) and *being notified* (PR ready) — not thinking through the work itself.
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
- **External integration** — a self-contained crate (`niles-integration-<name>`) connecting Niles to an outside service (Linear, calendar, GitHub, etc.); exposes LLM tools and publishes notification events
- **Plugin (third-party integration)** — a WebAssembly component installed from a GitHub release, targeting the `niles:integration/plugin` world; sandboxed, it adds devices, tools, pages and widgets within the permissions its manifest declares and the household approved
- **WIT** — the WebAssembly Component Model's interface language; `schemas/niles-integration.wit` defines what Niles provides to a plugin and what a plugin provides to Niles
- **Last-active satellite** — the most recently-used satellite by the user; default target for notifications without other routing affinity
- **Speaker recognition** — Niles's identification of who is speaking via voice embeddings; runs in parallel with STT, adds zero perceived latency
- **Voice embedding** — fixed-length numerical representation of a person's voice (~192–512 floats), produced by a small neural model and stored locally for nearest-neighbor identity matching
- **Admin** — a user with elevated privileges; can create rules, manage other users, and modify shared home settings like the lighting curve
- **Rule** — an admin-defined permission constraint (e.g. "only Mark or Majse can change the temperature") evaluated before every tool execution
- **Personal reference** — a phrase like "my calendar" or "wake me up" that the LLM resolves to the speaker's specific external resource (their calendar account, their bedroom, etc.); distinct from per-user preferences, which Niles does not maintain for shared home state
- **Presence source** — an adapter that reports whether a specific user is home (Tado, phone GPS, BLE beacons, manual voice command, etc.); multiple sources can be combined per user
- **Home state** — Niles's aggregated occupancy status (`occupied` / `unoccupied` / `transitioning`), derived from per-user presence with hysteresis
- **Automation** — "when X, do Y". Code automations are foundational behaviors implemented in Rust; rules are one trigger, conditions and actions, made by voice or on the Automations page and stored in Postgres
- **Capability manifest** — `MANIFEST.md`, the LLM-facing structured description of Niles's features, requirements, and setup decisions; generated from the canonical `features.toml`
- **features.toml** — canonical feature catalog at the repo root; single source of truth for the runtime's feature awareness, the generated MANIFEST.md, the README's feature table, and the docs site
- **Music intent** — high-level LLM tool (`play_radio`, `play_music`, etc.) that maps to source-specific Sonos actions internally; insulates the LLM from knowing which music service is being used
- **Per-room music state** — Niles's database memory of what each room was last playing (source, content URI, playing/paused), enabling "play TuneIn" to resume that room's last station

---

*This document is a living architecture spec for Niles, an open-source AI-first home automation system. Contributions and forks welcome.*
