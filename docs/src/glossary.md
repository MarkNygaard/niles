# Glossary

## Tier 0
Deterministic regex router that answers fixed phrasings (lights, blinds, scenes, music, the TV, timers, heating, weather, the clock) without an LLM. The target is most everyday commands in about half a second end to end; that is a design goal, not a measurement.

## Tier 1
Fast LLM (Cerebras, by default) with the tool registry. Answers whatever Tier 0 misses.

## Tier 2
Optional escalation to a stronger model, off unless `[llm.tier2]` is in the config file. When configured it defaults to OpenAI's `gpt-5.5`. Invoked only through the `escalate_to_tier2` tool.

## Assistant
Claude Code or ChatGPT, on the household's own subscription. When switched on, it answers the chat, works out new rules, writes Saved items, looks at attached files and runs the background review. Chosen under Settings → Voice & AI.

## Capability reference
Structured documentation for a topic (lights, timers, music...) that the LLM reads at runtime to decide how to act. Written as `docs/capabilities/<topic>/SKILL.md` and compiled into the binary by `niles-capabilities`. Only the references for topics detected in a request are put in the prompt.

## Skill
A how-to for a routine: a name, a short "use when" description and a body of instructions. Skills are minted, patched, viewed, retired and deleted through tool calls (`mint_skill`, `patch_skill`, `view_skill`, `list_skills`, `retire_skill`, `delete_skill`), and listed on the app's Skills page.

## Wyoming
The voice protocol Niles speaks with its satellites: `audio-start`, `audio-chunk` and `audio-stop` carrying 16 kHz mono PCM in, and reply audio back. The wake word is not part of it; it runs on the satellite.

## Satellite
A voice appliance (a reSpeaker XVF3800 with an ESP32-S3 running Niles's firmware) that listens for the wake word, streams audio to Niles, and plays back replies. Its room comes from its IP address under Settings → Satellites. Satellites are identical and replaceable; all state lives on the server.

## Intent
A user's goal extracted from a command. Tier 0 produces intents directly; a Tier 0 miss goes to the LLM.

## Tool registry
The structured catalogue of every tool the LLM can call. Tools are offered by topic, from keywords in the request. The registry is queryable structured data, not RAG.

## Add-on library
The Browse tab's list of add-ons other people published, read from one file the registry repo builds. Listed ones were reviewed into the registry; Unreviewed ones are only tagged `niles-addon` on GitHub.

## System prompt
The full context assembled for the LLM on every turn. Ordered stable to volatile: persona, tool guidance, household context, user memory, agent memory, skills, capability references, origin room, speaker, current time.

## Background review
Runs once the house has been quiet for 5 minutes: the assistant reads what was said since the last review and may add to memory, to a person's notes, or a skill. A weekly tidy-up merges and removes; what it removes can be restored in the app. Needs Claude Code or ChatGPT.

## Manifest
`MANIFEST.md`, auto-generated from `features.toml`. The LLM-facing feature catalogue that tracks every user-observable capability.

## Persona
The top layer of the system prompt: who Niles is, how it speaks, and what tone it uses.

## Room / RoomName
A named space in the home (e.g. `kitchen`, `bedroom`), taken from the first half of a device's `room/device` name. Devices and scenes are scoped to rooms.

## Origin room
The room of the satellite that heard the command. Grounds spatial references such as "this room" or "the light in here".

## Manual mode
A light someone set by hand (a spoken level, a scene, a dimmer press) that the daily curve leaves alone until it is switched off and on, or "back to normal" is said.

## Z2M
Zigbee2MQTT. The bridge Niles uses to communicate with Zigbee devices over MQTT.

## Piper
The local text-to-speech (TTS) engine that renders Niles's text responses into audio sent back to the satellite.

## Memory
Persistent notes kept across conversations, in two household-wide documents: `user` (about the household) and `agent` (what Niles has noted). Accessed through one tool, `memory`, with an `action` (add, replace, remove, view) and a `target` (user or agent). Notes about one person live with their voice instead, through `remember_about_me`.

## Tier 2 escalation
The hand-off from Tier 1 to Tier 2 through the `escalate_to_tier2` tool. Available only when `[llm.tier2]` is configured.
