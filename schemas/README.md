# Shared schemas

Language-agnostic JSON Schema definitions used across the Rust backend, firmware, and (future) frontends. Populated incrementally as Phases 2–4 generate them.

Planned files:

- `wyoming-events.json` — Wyoming-protocol event schema
- `tool-definitions.json` — LLM tool catalog schema
- `device-types.json` — device-type and capability schema

Present:

- `niles-integration.wit` — the WIT package `niles:integration@0.2.0`: the interface between Niles and a third-party integration (a WebAssembly component), 0.1 plus MQTT. The file plugin authors copy. Run by `crates/niles-plugins` (Niles 0.2.0); see ARCHITECTURE.md, "Third-party integrations (plugins)", and the author guide in `docs/integrations/`. Check it with `wasm-tools component wit schemas/niles-integration.wit`.
- `niles-integration-0.1.wit` — `niles:integration@0.1.0`, frozen as it shipped. Niles still runs plugins built against it; every interface it has is the same in 0.2, which only adds `mqtt` and two exports. Two files rather than one with nested packages, so that bindings generated from the current file keep their plain module names (`niles::integration::devices`, not `integration0_2_0`). How the runtime links both without implementing anything twice is in ARCHITECTURE.md, "Versioning the interface".
