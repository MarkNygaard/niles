# Glossary

## Tier 0
Deterministic regex router that catches ~80 % of common commands in sub-100 ms. No LLM involved.

## Tier 1
Fast LLM (Groq) with the tool registry. Handles ~19 % of commands needing light reasoning within ~1 s.

## Tier 2
Smart LLM (OpenAI) escalation for the ~1 % of requests that exceed Tier 1's reasoning depth. Invoked via the `escalate_to_tier2` tool.

## Capability reference
Structured documentation for a tool or feature that the LLM reads at runtime to decide how to act. Lives in `crates/niles-capabilities/`.

## Skill
A user-defined shortcut: a natural-language trigger plus a body of instructions that Niles executes. Skills are minted, patched, and deleted via tool calls.

## Wyoming
The open voice-protocol that Niles speaks with its satellites. Handles wake-word detection, streaming audio in, and TTS audio out.

## Satellite
An ESPHome-based voice appliance that listens for a wake word, streams audio to Niles, and plays back TTS responses. Satellites are identical and replaceable; all state lives on the server.

## Intent
A user's goal extracted from a voice command. Niles routes intents through Tier 0 → Tier 1 → Tier 2 until one tier can satisfy the request.

## Tool registry
The structured catalogue of every device command, scene, timer, and skill that the LLM can invoke. The registry is queryable structured data, not RAG.

## System prompt
The full context assembled for the LLM on every turn. Ordered stable → volatile: persona → household → memory → skills → capabilities → origin room.

## Background review
A forked review process that runs after Tier 1 turns, extracting lessons-learned suggestions for new or updated skills. Results are queued for user approval via `skill-background-review`.

## Manifest
`MANIFEST.md`, auto-generated from `features.toml`. The LLM-facing feature catalogue that tracks every user-observable capability.

## Persona
The top layer of the system prompt: who Niles is, how it speaks, and what tone it uses.

## Room / RoomName
A named space in the home (e.g. `kitchen`, `bedroom`). Devices and scenes are scoped to rooms.

## Origin room
The room of the satellite that heard the command. Grounds spatial references such as "this room" or "the light in here".

## Z2M
Zigbee2MQTT. The bridge Niles uses to communicate with Zigbee devices over MQTT.

## Piper
The local text-to-speech (TTS) engine that renders Niles's text responses into audio sent back to the satellite.

## Memory
Persistent user-specific or agent-specific notes stored across sessions. Accessed via `memory-add`, `memory-remove`, `memory-replace`, and `memory-view`.

## Tier-2 escalation
The hand-off from Tier 1 to Tier 2 via the `escalate_to_tier2` tool, carrying full conversation context and capability references.
