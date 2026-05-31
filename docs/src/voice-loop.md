# Voice loop

Niles processes every voice command through a tiered pipeline that trades latency against capability. The loop is deterministic at the bottom, intelligent at the top, and always streaming.

## Tier 0 — regex router

Tier 0 is a deterministic regex router that catches roughly 80 % of common household commands in sub-100 milliseconds. It matches literal patterns such as "turn on the kitchen light" or "set a timer for five minutes" without touching an LLM. Because it is stateless and runs locally, it is the default path for speed-critical interactions.

## Tier 1 — Groq LLM with tool registry

When Tier 0 misses, the request falls through to Tier 1: a fast LLM (Groq) armed with the tool registry. This tier handles roughly 19 % of commands that need light reasoning — parsing intent, selecting tools, and filling parameters — while staying within a ~1-second budget. The tool registry exposes every device command, scene, timer, and skill as a structured call, so the LLM does not guess; it picks.

## Tier 2 — OpenAI escalation

The remaining ~1 % of requests exceed Tier 1's reasoning depth or tool set. Tier 2 escalates to OpenAI via the `escalate_to_tier2` tool, carrying full conversation context and the capability references the model needs to answer correctly. This tier is slower but unconstrained in reasoning complexity.

## System prompt layout

The system prompt is assembled in a stable-to-volatile order so that common commands never pay the cost of carrying full documentation:

1. **Persona** — who Niles is and how it speaks.
2. **Household context** — rooms, devices, and their current states.
3. **User / agent memory** — persistent notes about preferences and habits.
4. **Available skills** — user-defined shortcuts loaded dynamically.
5. **Capability references** — full documentation for tools the LLM can call.
6. **Origin room** — which satellite heard the command, grounding spatial references.

## Background-review fork

After every Tier 1 turn, a background-review fork evaluates the conversation for lessons learned. If a command was mis-routed, a skill could have handled it, or the user corrected Niles, the fork extracts a suggestion and queues it for review. The user can accept, edit, or reject these suggestions via the `skill-background-review` tool.

For the deep dive on every subsystem, see the canonical [ARCHITECTURE.md](../../ARCHITECTURE.md) at the repo root.
