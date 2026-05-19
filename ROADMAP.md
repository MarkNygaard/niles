# Niles — Roadmap

A running view of what's built, what's in flight, and what's queued.
Phases mirror [ARCHITECTURE.md](ARCHITECTURE.md#build-phases). Update
this file in every PR that completes a phase deliverable.

**Legend:** ✅ done · 🚧 in progress · ⏳ not started · 🚫 blocked

## Snapshot

- **Crates with content:** `niles-core`, `niles-intent`, `niles-scheduler`
- **In flight:** Phase 2 (`niles-mqtt` reads + writes; `niles-api` exposes the registry over HTTP), Phase 6 (scenes, morning routine still pending)
- **Blocked on hardware:** Phase 1 (Z2M can't connect to coordinator until the SLZB-06MU arrives), Phase 3 (voice satellite firmware)
- **Last PR merged:** [#13 — niles-api read-only HTTP endpoints](https://github.com/MarkNygaard/niles/pull/13)

---

## Bootstrap ✅

Repo foundation — Cargo workspace, dual-license, CI, all crate skeletons, deploy/firmware/docs placeholders.

- ✅ Cargo workspace with 19 library crates + `niles-bin`
- ✅ Dual-licensed MIT OR Apache-2.0
- ✅ GitHub Actions CI (fmt / clippy / test / build)
- ✅ Dependabot config
- ✅ Architecture, contribution, security docs

PRs: [bootstrap commit](https://github.com/MarkNygaard/niles/commit/cbc318a), [#1](https://github.com/MarkNygaard/niles/pull/1)

---

## Phase 0 — Hardware prep 🚧

Ordering the reSpeaker XVF3800 + XIAO ESP32-S3 satellite and the SMLIGHT SLZB-06MU coordinator.

- 🚧 Hardware ordered (Seeed EU warehouse, awaiting delivery)
- ⏳ First satellite mounted and powered

---

## Phase 1 — Infrastructure 🚫

Blocked on Phase 0 hardware delivery + decision on Kustomize manifest authoring.

- ⏳ Mosquitto deployed to k8s cluster (proper Kustomize, not ad-hoc)
- ⏳ Zigbee2MQTT deployed pointing at the SLZB-06MU
- ⏳ Zigbee devices migrated and renamed to `<room>/<device>`
- ⏳ MQTT round-trip verified end-to-end
- ⏳ Home Assistant decommissioned

---

## Phase 2 — Rust backend skeleton 🚧

- ✅ `niles-core` — event bus, device registry, shared types [#2](https://github.com/MarkNygaard/niles/pull/2)
- 🚧 `niles-mqtt` — connection + Z2M parser ([#9](https://github.com/MarkNygaard/niles/pull/9)) + registry wiring via `Z2mSource` ([#10](https://github.com/MarkNygaard/niles/pull/10)) + auto-reconnect with subscription replay ([#11](https://github.com/MarkNygaard/niles/pull/11)) + Z2M command publishing ([#12](https://github.com/MarkNygaard/niles/pull/12)). Source + sink ownership refactor still pending for a real `niles serve` (currently the source consumes the client; sink uses its own short-lived connection in `niles set`).
- 🚧 `niles-api` — read-only HTTP endpoints (`GET /devices`, `GET /rooms/{room}`, `GET /healthz`) over the registry [#13](https://github.com/MarkNygaard/niles/pull/13). New `niles api` subcommand runs source + API together. Write endpoints + WebSocket event stream still pending.
- ✅ `niles-config` — TOML loading and validation [#8](https://github.com/MarkNygaard/niles/pull/8) (`[home]` + `[lighting]` sections; new sections land alongside their consuming crates)

---

## Phase 3 — Tier 0 + Tier 1 voice loop 🚧

- ✅ `niles-intent` — Tier 0 regex router (lights, timers, stop/cancel) [#3](https://github.com/MarkNygaard/niles/pull/3)
- ⏳ `niles-wyoming` — Wyoming protocol server (blocked on satellite firmware)
- ⏳ Satellite ESPHome firmware flashed
- ⏳ Groq Whisper streaming STT integration
- ⏳ Piper TTS deployed to k8s
- ⏳ End-to-end fast-path command working under ~500ms

---

## Phase 4 — LLM tier + timers + capability reference ⏳

- ⏳ `niles-llm` — Groq client with tool calling
- ⏳ `niles-capabilities` — Tier A/B/C context loader
- ⏳ Topic detection in `niles-intent`
- ⏳ `look_up_capability` and `explain_device_state` tools
- ⏳ Timer subsystem in `niles-scheduler` (set / query / cancel, two-stage alarm)

---

## Phase 5 — Room speaker integration and music ⏳

- ⏳ `niles-speakers` — Sonos SOAP/UPnP client
- ⏳ Ducking during voice responses
- ⏳ Music intent tools (`play_radio`, `play_music`, transport, grouping)
- ⏳ Per-room music state in SQLite
- ⏳ Tier 0 fast-paths for common music commands

---

## Phase 6 — Ambient lighting + scenes 🚧

- ✅ Lighting brightness curve [#5](https://github.com/MarkNygaard/niles/pull/5), spec clarified in [#4](https://github.com/MarkNygaard/niles/pull/4)
- ✅ Color temperature curve [#7](https://github.com/MarkNygaard/niles/pull/7) — anchor-based piecewise-linear, default warm→cool→warm circadian cycle
- ⏳ Morning routine (separate from the curve — its own 0% → 100% ramp)
- ⏳ Manual mode (per-light, escalation on subsequent clicks)
- ⏳ Scenes (save / apply / list / delete / update / exit)
- ⏳ Tier 0 fast-paths for scene phrasings and "back to normal"

---

## Phase 7 — User recognition and permissions ⏳

- ⏳ `niles-recognition` — ECAPA-TDNN via ONNX Runtime
- ⏳ Speaker ID running in parallel with STT
- ⏳ Introduction flow (explicit + prompted)
- ⏳ `niles-permissions` — rules engine + admin concept
- ⏳ Default rules (unknown-speaker restrictions, admin-only ops)

---

## Phase 8 — Notifications subsystem ⏳

- ⏳ `niles-notifications` — routing, chime + voice formatting, SQLite persistence
- ⏳ Quiet hours config with priority-aware handling
- ⏳ Last-active satellite tracking
- ⏳ `list_recent_notifications` LLM tool

---

## Phase 9 — Presence and automations ⏳

- ⏳ `niles-presence` — adapter pattern + Tado HTTP adapter + manual voice override
- ⏳ Home-state aggregation with hysteresis
- ⏳ `niles-automations` — config-defined loader, event subscription, condition + action dispatch
- ⏳ Voice-creatable automations (admin-only)

---

## Phase 10 — First external integration: Archon ⏳

- ⏳ `niles-integration-archon` — HTTP API + webhook events
- ⏳ Project + workflow cache
- ⏳ Workflow run / status / cancel tools
- ⏳ Approval-gate handling via voice
- ⏳ Capability reference for Archon in Tier B context

---

## Phase 11 — LLM-facing docs and deployment polish ⏳

- ⏳ `features.toml` canonical catalog
- ⏳ `MANIFEST.md` generated from `features.toml` (+ CI freshness check)
- ⏳ Reference `docker-compose.yml` and `.env.example`
- ⏳ Kustomize base for k8s deployment
- ⏳ README + mdBook docs site

---

## Phase 12 — Polish (ongoing) ⏳

- ⏳ Order remaining satellites once one room is proven
- ⏳ Tier 2 escalation (Claude Sonnet)
- ⏳ Event log + SQL search tool
- ⏳ Conversation memory (short + long term)

---

## Phase 13 — Additional integrations and extensions ⏳

- ⏳ Calendar (Microsoft 365 / Google)
- ⏳ Email
- ⏳ GitHub direct integration
- ⏳ Monitoring / alerting bridge
- ⏳ Frontends (Tauri 2 for desktop + mobile)
- ⏳ Custom Rust firmware on the satellites
- ⏳ Additional device sources (Shelly, Matter, Z-Wave)
- ⏳ GPU node for fully-local STT/LLM
- ⏳ Satellite-side speaker ID
