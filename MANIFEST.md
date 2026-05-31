<!-- DO NOT EDIT BY HAND. Generated from features.toml — run `cargo run -p niles-bin -- generate-manifest`. -->

# niles — feature manifest

Niles is an AI-first home-automation system for private homes. It listens to natural-language voice commands, dispatches them through a tiered intent router, and can escalate to an LLM when a request exceeds the deterministic tier. The feature catalog below tracks every user-observable capability shipped so far. For the full architecture spec see ARCHITECTURE.md.

## ambient

| Feature | Summary | Since |
| --- | --- | --- |
| `curve-brightness` | Per-minute adaptive brightness curve based on time of day | [#5](https://github.com/MarkNygaard/niles/pull/5) |
| `curve-color-temp` | Per-minute adaptive color-temperature curve based on time of day | [#7](https://github.com/MarkNygaard/niles/pull/7) |
| `curve-morning-routine` | Gradual morning-routine ramp that respects per-room claims | [#24](https://github.com/MarkNygaard/niles/pull/24) |
| `manual-mode-override` | Clear manual-mode overrides after a configurable timeout | [#32](https://github.com/MarkNygaard/niles/pull/32) |

## api

| Feature | Summary | Since |
| --- | --- | --- |
| `api-events-stream` | WebSocket /events/stream pushes real-time device and system events | [#86](https://github.com/MarkNygaard/niles/pull/86) |
| `api-get-devices` | GET /devices returns the full device registry snapshot | [#13](https://github.com/MarkNygaard/niles/pull/13) |
| `api-get-room` | GET /rooms/{room} returns the devices in a single room | [#13](https://github.com/MarkNygaard/niles/pull/13) |
| `api-healthz` | GET /healthz returns a liveness probe for load balancers | [#13](https://github.com/MarkNygaard/niles/pull/13) |
| `api-post-device` | POST /rooms/{room}/{device} sends a state update to a device | [#59](https://github.com/MarkNygaard/niles/pull/59) |

## llm

| Feature | Summary | Since |
| --- | --- | --- |
| `tool-command-history` | Query the command-history log for past user requests | [#70](https://github.com/MarkNygaard/niles/pull/70) |
| `tool-datetime` | Return the current date and time in the configured timezone | [#84](https://github.com/MarkNygaard/niles/pull/84) |
| `tool-device-history` | Query the device-state history log for past state changes | [#71](https://github.com/MarkNygaard/niles/pull/71) |
| `tool-explain-device-state` | Provide a natural-language explanation of a device's current state | [#70](https://github.com/MarkNygaard/niles/pull/70) |
| `tool-light-control` | Turn lights on or off and set brightness or color temperature | [#3](https://github.com/MarkNygaard/niles/pull/3) |
| `tool-look-up-capability` | Retrieve a capability reference document by name for the LLM context | [#84](https://github.com/MarkNygaard/niles/pull/84) |
| `tool-scene-control` | Apply or delete saved scenes via structured tool calls | [#43](https://github.com/MarkNygaard/niles/pull/43) |
| `tool-sonos-transport` | Control Sonos speaker transport state via the speakers subsystem | [#65](https://github.com/MarkNygaard/niles/pull/65) |
| `tool-timer` | Set, cancel, and list timers via structured tool calls | [#3](https://github.com/MarkNygaard/niles/pull/3) |
| `tool-weather` | Fetch current weather and forecast via Open-Meteo | [#81](https://github.com/MarkNygaard/niles/pull/81) |
| `tool-web-search` | Search the web via SearXNG and return summarized results | [#83](https://github.com/MarkNygaard/niles/pull/83) |

## runtime

| Feature | Summary | Since |
| --- | --- | --- |
| `observability-command-log` | Structured command-history log for every voice and API request | [#70](https://github.com/MarkNygaard/niles/pull/70) |
| `observability-state-log` | Structured device-state history log for every state change | [#71](https://github.com/MarkNygaard/niles/pull/71) |
| `persistence-json-store` | JSON file persistence for timers, scenes, and morning-routine claims | [#64](https://github.com/MarkNygaard/niles/pull/64) |

## voice

| Feature | Summary | Since |
| --- | --- | --- |
| `tier-two-escalation` | Escalate a voice request to the Tier 2 LLM when Tier 0/1 cannot answer | [#87](https://github.com/MarkNygaard/niles/pull/87) |
| `light-all-brightness` | Set brightness for every light in a room simultaneously | [#25](https://github.com/MarkNygaard/niles/pull/25) |
| `light-all-on-off` | Turn all lights in a room on or off at once | [#3](https://github.com/MarkNygaard/niles/pull/3) |
| `light-all-temp` | Set color temperature for every light in a room simultaneously | [#72](https://github.com/MarkNygaard/niles/pull/72) |
| `light-brightness` | Set a light's brightness to a specific percent | [#25](https://github.com/MarkNygaard/niles/pull/25) |
| `light-named-white` | Set a light to a named white such as warm or cool | [#74](https://github.com/MarkNygaard/niles/pull/74) |
| `light-on-off` | Turn a single light on or off by room and name | [#3](https://github.com/MarkNygaard/niles/pull/3) |
| `light-step` | Step a light brighter or dimmer by one increment | [#27](https://github.com/MarkNygaard/niles/pull/27) |
| `light-temp-step` | Step a light warmer or cooler by one color-temperature increment | [#72](https://github.com/MarkNygaard/niles/pull/72) |
| `media-duck` | Temporarily lower volume for voice responses | [#73](https://github.com/MarkNygaard/niles/pull/73) |
| `media-next-previous` | Skip to the next or previous track | [#75](https://github.com/MarkNygaard/niles/pull/75) |
| `media-play-pause` | Play or pause media playback on the active speaker | [#65](https://github.com/MarkNygaard/niles/pull/65) |
| `media-volume-set` | Set the speaker volume to a specific level | [#65](https://github.com/MarkNygaard/niles/pull/65) |
| `media-volume-step` | Raise or lower the speaker volume by one step | [#65](https://github.com/MarkNygaard/niles/pull/65) |
| `memory-add` | Append a note to the user's persistent memory | [#77](https://github.com/MarkNygaard/niles/pull/77) |
| `memory-remove` | Remove a note from the user's persistent memory | [#77](https://github.com/MarkNygaard/niles/pull/77) |
| `memory-replace` | Replace an existing memory entry | [#77](https://github.com/MarkNygaard/niles/pull/77) |
| `memory-view` | Show the contents of the user's persistent memory | [#77](https://github.com/MarkNygaard/niles/pull/77) |
| `scene-apply` | Restore a previously saved scene by name | [#43](https://github.com/MarkNygaard/niles/pull/43) |
| `scene-delete` | Delete a saved scene by name | [#45](https://github.com/MarkNygaard/niles/pull/45) |
| `scene-list` | List all saved scenes | [#45](https://github.com/MarkNygaard/niles/pull/45) |
| `scene-save` | Save the current device states as a named scene | [#43](https://github.com/MarkNygaard/niles/pull/43) |
| `skill-background-review` | Review skill suggestions generated in the background | [#85](https://github.com/MarkNygaard/niles/pull/85) |
| `skill-curator` | Automatic background transitions for stale and archived skills | [#80](https://github.com/MarkNygaard/niles/pull/80) |
| `skill-delete` | Delete a saved skill by name | [#78](https://github.com/MarkNygaard/niles/pull/78) |
| `skill-mint` | Create a new skill from a natural-language description | [#78](https://github.com/MarkNygaard/niles/pull/78) |
| `skill-patch` | Update an existing skill's body while keeping its name | [#78](https://github.com/MarkNygaard/niles/pull/78) |
| `skill-view` | Read the full body of a saved skill | [#78](https://github.com/MarkNygaard/niles/pull/78) |
| `timer-cancel` | Cancel an active timer by name or duration | [#3](https://github.com/MarkNygaard/niles/pull/3) |
| `timer-list` | List all currently active timers | [#3](https://github.com/MarkNygaard/niles/pull/3) |
| `timer-set` | Set a timer for a given duration | [#3](https://github.com/MarkNygaard/niles/pull/3) |

### `tier-two-escalation` — example phrasings

- ask the smart assistant
- escalate this to tier two

### `light-all-brightness` — example phrasings

- set all lights to 75 percent

### `light-all-on-off` — example phrasings

- turn on all the lights
- switch everything off

### `light-all-temp` — example phrasings

- make all the lights warmer

### `light-brightness` — example phrasings

- set the living room light to 50 percent
- dim the hallway to twenty percent

### `light-named-white` — example phrasings

- set the light to warm white
- switch to cool white

### `light-on-off` — example phrasings

- turn on the kitchen light
- switch off the bedroom lamp

### `light-step` — example phrasings

- make it brighter
- dim the light a little

### `light-temp-step` — example phrasings

- make it warmer
- cool down the light

### `media-duck` — example phrasings

- duck the music
- lower the music while you speak

### `media-next-previous` — example phrasings

- next song
- previous track

### `media-play-pause` — example phrasings

- play music
- pause the music

### `media-volume-set` — example phrasings

- set the volume to thirty
- turn the music down to ten

### `media-volume-step` — example phrasings

- turn it up
- make it quieter

### `memory-add` — example phrasings

- remember that I like the lights dim
- add to my memory

### `memory-remove` — example phrasings

- forget that I like dim lights
- remove from my memory

### `memory-replace` — example phrasings

- update my memory to say I prefer warm white

### `memory-view` — example phrasings

- what do you know about me
- read my memory

### `scene-apply` — example phrasings

- activate movie mode
- set the scene to dinner

### `scene-delete` — example phrasings

- delete the dinner scene
- remove movie mode

### `scene-list` — example phrasings

- what scenes do we have
- list my scenes

### `scene-save` — example phrasings

- save this as movie mode
- remember this scene as dinner

### `skill-background-review` — example phrasings

- review my pending skills
- show background skill suggestions

### `skill-curator` — example phrasings

- run the skill curator
- clean up old skills

### `skill-delete` — example phrasings

- delete the bedtime skill
- remove movie night

### `skill-mint` — example phrasings

- save this as a skill called bedtime
- mint a skill named movie night

### `skill-patch` — example phrasings

- update the bedtime skill
- patch movie night

### `skill-view` — example phrasings

- show me the bedtime skill
- what is in movie night

### `timer-cancel` — example phrasings

- cancel the five minute timer
- stop the timer

### `timer-list` — example phrasings

- what timers are running
- list my timers

### `timer-set` — example phrasings

- set a timer for five minutes
- timer ten minutes
