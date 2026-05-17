# Niles — Initial Setup Tasks

Scope: **repo bootstrap only.** Everything needed before Phase 1 (Infrastructure) work begins. The goal is an empty-but-buildable monorepo with the structure described in [ARCHITECTURE.md](ARCHITECTURE.md), plus CI and licensing.

Estimated effort: ~1 evening of focused work.

---

## 1. Decisions to make first

These block other tasks. Resolve before writing anything.

- [x] ~~Pick a license~~ → **Dual-licensed `MIT OR Apache-2.0`** (Rust ecosystem convention; gives downstream users the choice).
- [x] ~~Decide what to do with the existing architecture doc~~ → renamed `niles-architecture.md` → `ARCHITECTURE.md` (matches the layout in the spec).
- [x] ~~Pick a Rust toolchain pin~~ → **`stable`** with `rustfmt` + `clippy` components (`rust-toolchain.toml` written).
- [x] ~~Pick an edition for all crates~~ → **`2024`** (configured in workspace `Cargo.toml` during section 3).

---

## 2. Project metadata at repo root

- [x] Add `LICENSE-MIT` and `LICENSE-APACHE` at repo root (standard Rust convention for dual-licensed crates). Reference both in the workspace `Cargo.toml` via `license = "MIT OR Apache-2.0"`.
- [x] Add `.gitignore` covering: Rust (`target/`), IDE noise, env files, OS junk, and Niles runtime data (`niles-data/`, `mosquitto-data/`, `z2m-data/`). `Cargo.lock` is committed (workspace produces a binary).
- [x] Add `.editorconfig` for consistent whitespace across the polyglot repo.
- [x] Add `README.md` stub.
- [x] Add `CONTRIBUTING.md` stub.
- [x] Add `CODE_OF_CONDUCT.md` (adopts Contributor Covenant 2.1 by reference).
- [x] Add `SECURITY.md`.

---

## 3. Cargo workspace

- [x] `Cargo.toml` at repo root — workspace manifest with `resolver = "3"`, `members = ["crates/*"]`, `[workspace.package]` (version, edition `2024`, dual-license, authors), and `[workspace.lints.rust]`.
- [x] `rust-toolchain.toml` pins `stable` with `rustfmt` + `clippy`.
- [ ] `.rustfmt.toml` (optional — using rustfmt defaults).
- [ ] `clippy.toml` (optional — no tuning needed yet).
- [x] `crates/` directory created.

### Library crates

All 19 created with workspace-inherited `Cargo.toml` and a module-doc `src/lib.rs`.

- [x] `crates/niles-core` — event bus, registry, types
- [x] `crates/niles-wyoming` — satellite protocol server
- [x] `crates/niles-mqtt` — MQTT + Z2M device source
- [x] `crates/niles-stt` — STT trait + providers
- [x] `crates/niles-llm` — LLM trait + providers
- [x] `crates/niles-tts` — TTS trait + providers
- [x] `crates/niles-speakers` — room speaker trait + Sonos impl
- [x] `crates/niles-intent` — Tier 0 regex router
- [x] `crates/niles-tools` — tool definitions for LLMs
- [x] `crates/niles-capabilities` — capability reference loader
- [x] `crates/niles-scheduler` — time-driven behaviors
- [x] `crates/niles-notifications` — unprompted speech routing
- [x] `crates/niles-integration-archon` — Archon integration
- [x] `crates/niles-recognition` — speaker identification
- [x] `crates/niles-permissions` — rules engine + admin concept
- [x] `crates/niles-presence` — presence sources + aggregation
- [x] `crates/niles-automations` — when-X-do-Y rules
- [x] `crates/niles-api` — HTTP/WebSocket API
- [x] `crates/niles-config` — config loading + validation

### Binary crate

- [x] `crates/niles-bin` — produces the `niles` binary. `clap`-derived CLI with `serve`, `migrate-from-ha`, `flash-satellite`, `config validate`, `tools list`. Each subcommand is `todo!()` for now.

### Verify workspace builds

- [x] `cargo build --workspace` — green (5.03s, all 20 crates + clap deps compiled).
- [x] `cargo fmt --all --check` — passes.
- [x] `cargo clippy --workspace --all-targets -- -D warnings` — passes.
- [x] `cargo run -p niles-bin -- --help` prints the subcommand skeleton.

---

## 4. Top-level directories (placeholders)

Create the dirs from the architecture's repo layout so future phases drop files into known locations. Each gets a one-line `README.md` explaining its purpose so the empty dirs aren't lost in git.

- [x] `firmware/esphome/README.md`
- [x] `firmware/esp-rs/README.md`
- [x] `deploy/compose/README.md`
- [x] `deploy/kubernetes/base/README.md`
- [x] `deploy/kubernetes/overlays/README.md`
- [x] `deploy/docker/README.md`
- [x] `docs/src/README.md`
- [x] `schemas/README.md`
- [x] `examples/README.md`
- [x] `scripts/README.md`

---

## 5. CI (GitHub Actions)

Minimum viable CI matching the architecture's "at minimum format/clippy/test and a build-all-crates job."

- [x] `.github/workflows/ci.yml` — single job with sequential steps (fmt → clippy → test → build --release), `Swatinem/rust-cache@v2` for caching, `dtolnay/rust-toolchain@stable` for setup, `concurrency` group to cancel superseded runs. Single-job approach beats parallel jobs here because the cache works better and there's no compile redundancy. Refactor later if the test matrix grows.
- [x] `.github/dependabot.yml` — weekly updates for `cargo` and `github-actions`.
- [ ] `.github/PULL_REQUEST_TEMPLATE.md` (deferred — wait until contribution patterns emerge).
- [ ] `.github/ISSUE_TEMPLATE/` (deferred — wait until issue patterns emerge).

---

## 6. Sanity check

- [x] `cargo build --workspace` works without warnings (5.03s on this machine).
- [ ] CI is green on the first push — pending first push to GitHub.
- [x] `niles --help` shows the subcommand skeleton.
- [x] Top-level layout matches [ARCHITECTURE.md#repository-layout-monorepo](ARCHITECTURE.md#repository-layout-monorepo). Only `MANIFEST.md` and `features.toml` missing — deferred to Phase 11 per the spec.

---

## Explicitly deferred (not part of bootstrap)

These appear in the architecture but belong to later phases — don't tackle them now:

- `features.toml` and the `MANIFEST.md` build script → **Phase 11**.
- mdBook content and theming → **Phase 11**.
- Actual `docker-compose.yml` and Kustomize base → **Phase 11**.
- JSON schema contents in `schemas/` → fill in as Phases 2–4 generate them.
- `niles migrate-from-ha` implementation → **Phase 1** / as needed.
- Any actual Rust logic inside the empty crates → starts in **Phase 2** (`niles-core`, `niles-mqtt`, `niles-api`, `niles-config`).

---

*Once everything above is checked, you have a buildable skeleton and can move into Phase 1 (Infrastructure) from the architecture doc.*
