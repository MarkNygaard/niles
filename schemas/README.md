# Shared schemas

Language-agnostic JSON Schema definitions used across the Rust backend, firmware, and (future) frontends. Populated incrementally as Phases 2–4 generate them.

Planned files:

- `wyoming-events.json` — Wyoming-protocol event schema
- `tool-definitions.json` — LLM tool catalog schema
- `device-types.json` — device-type and capability schema

Present:

- `niles-integration.wit` — the WIT package `niles:integration@0.1.0`: the interface between Niles and a third-party integration (a WebAssembly component). Run by `crates/niles-plugins` (Niles 0.2.0); see ARCHITECTURE.md, "Third-party integrations (plugins)", and the author guide in `docs/integrations/`. Check it with `wasm-tools component wit schemas/niles-integration.wit`.
