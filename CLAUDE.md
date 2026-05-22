# CLAUDE.md

Working notes for Claude Code (and any AI coding agent) on this repo.
Auto-loaded by Claude Code when sessions start in this directory.

## What this project is

Niles is an open-source, AI-first home automation system written in Rust.
It replaces Home Assistant for users who want sub-second voice control,
code/config over UI, and an LLM as a first-class citizen.
The full spec is in [ARCHITECTURE.md](ARCHITECTURE.md).
Current build status is in [ROADMAP.md](ROADMAP.md).

## Repository orientation

- **Monorepo, Cargo workspace.** Workspace manifest at `Cargo.toml`, resolver 3, edition 2024.
- **Crates** under `crates/`. Library crates have one module per concern; `niles-bin` is the only binary.
- **Architecture, roadmap, contributing, security, conduct docs** at repo root.
- **Deploy / firmware / docs / schemas / examples / scripts** are placeholders that fill in as their owning phases land.

## Toolchain

- `rust-toolchain.toml` pins `stable` with `rustfmt` + `clippy`. Cloners get the right compiler via `rustup` automatically.
- Edition `2024` is set workspace-wide; crates inherit via `edition.workspace = true`.
- Workspace dependencies live in `[workspace.dependencies]` so every crate shares a single version.

## Common commands

Always run these in PowerShell on the dev machine (Windows MSVC) or on CI (Ubuntu). The verify chain we run before pushing:

```powershell
cargo fmt --all --check
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

To test or run a single crate:

```powershell
cargo test -p niles-core
cargo run -p niles-bin -- --help
```

If formatting fails, run `cargo fmt --all` to auto-fix and re-run the check.

## Conventions

### Crate layout

Each library crate uses this shape:

```
crates/<name>/
├── Cargo.toml      # inherits package metadata + lints from workspace
└── src/
    ├── lib.rs      # module declarations + pub re-exports of the public API
    ├── error.rs    # local Error enum (thiserror) + Result alias
    └── <concern>.rs ...
```

`lib.rs` re-exports the headline types so callers can write `use niles_core::DeviceRegistry;` instead of `use niles_core::registry::DeviceRegistry;`.

### Errors

- `thiserror` for ergonomic `#[derive(Error)]` enums.
- Each crate defines its own `Error` + `pub type Result<T> = std::result::Result<T, Error>;` alias.
- `Error` variants name a *kind* + a *reason* string when the cause is dynamic (e.g. `InvalidName { kind: &'static str, reason: String }`).

### Tests

- Unit tests inline in the same file as the code, under `#[cfg(test)] mod tests`.
- Tests assert *behavior*, not implementation detail. Prefer multiple tightly-scoped tests over one giant test.
- The standard contract: build / fmt / clippy `-D warnings` / tests all pass before commit.

### Public surface

- **No traits without implementations.** `DeviceSource` and `RoomSpeaker` live in their first implementation crate (e.g. `niles-mqtt`), not in `niles-core`, until they're actually used.
- **`#[non_exhaustive]` on public enums** that will gain variants (events, intents) so adding variants isn't a breaking change for downstream matches.
- **Newtypes for validated identifiers** (`RoomName`, `DeviceName`, `MinuteOfDay`) — never raw `String`/`u16` in public APIs where validation rules matter.

### Units and ranges

- Brightness is `u8` in `0..=100` (percent). Upstream-source translation (Z2M's `0..=254`) happens in the source-specific crate, not in core types.
- Color temperature is `u16` Kelvin. Mireds → Kelvin conversion happens in the source-specific crate.

## PR workflow

1. Branch off `main` with a meaningful prefix: `feat/`, `fix/`, `docs/`, `chore/`.
2. Make the change; run the verify chain locally.
3. Commit with a Conventional-Commits-style subject (`feat(niles-core): ...`).
4. Push and open a PR with `gh pr create`. PR body summarizes what's in, what's out, and how it was verified.
5. Wait for CI green. The user reviews, may push fixup commits.
6. Squash-merge with `--delete-branch` once approved.

Commits made by Claude include the trailer:
```
Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
```

## Things to avoid

- **No local Docker workflows.** All containerized services run on the user's k8s cluster. Don't suggest `docker run` or `docker compose up`.
- **Never `git add -A` or `git add .`** Stage files explicitly. Easy to accidentally include `.env` or other secrets.
- **Never skip git hooks** (`--no-verify`, `--no-gpg-sign`). If a hook fails, fix the cause.
- **No backwards-compatibility shims** for code that isn't yet in production. Just change the code.
- **No premature abstractions.** Three similar lines is better than a generic helper used once.
- **No comments that restate the code.** Comments explain *why*, not *what*. Default to none.
- **No half-finished implementations.** If a function can't be completed in this PR, scope it out.

## Spec discipline

- The spec ([ARCHITECTURE.md](ARCHITECTURE.md)) is the source of truth for *what* Niles does.
- When the spec is ambiguous, **fix the spec first** in a doc-only PR, then implement against the clarified version. Don't bake ambiguity into code.
- The roadmap ([ROADMAP.md](ROADMAP.md)) is the source of truth for *what's built*. Update it in any PR that completes a phase deliverable.

## Prior art worth checking before designing new subsystems

Some external projects have interesting patterns for problems Niles will eventually solve. Look at them *before* inventing our own approach — borrow what fits, ignore what doesn't, and be explicit either way.

### Hermes Agent (Nous Research)

Python LLM-agent framework with self-improving skills and a layered memory system.

- **Local code:** `C:\Users\Mark\Github\hermes-agent`
- **Docs:** <https://hermes-agent.nousresearch.com/docs/llms.txt>

Specifically check before building:

- **Phase 4 — `niles-capabilities` (capability reference loader):** Hermes uses the [agentskills.io](https://agentskills.io) format — directory per skill, required `SKILL.md` with YAML frontmatter (`name`, `description`, `version`, `prerequisites`), optional `references/` / `templates/` / `assets/` subdirs. Loaded on-demand via progressive disclosure (metadata in the index, full content fetched when invoked). Worth aligning with this format before defining our own.
- **Phase 4 — `niles-llm` (LLM client, system prompt assembly):** Hermes splits the system prompt into stable / context / volatile tiers to keep Anthropic prompt caching effective (stable bits cached, volatile bits rebuilt per turn). Mirror this layout in our Tier A/B/C context assembly.
- **Phase 7 — `niles-recognition` (per-user identity):** Hermes stores per-user persistent context as a markdown file (`USER.md` per user). Human-readable, hand-editable, diffable. Maps to our "personal references, not preferences" section. Note: this is per-*household-user*. Niles itself has one global persona — not per-user.
- **Phase 12+ — skill curator (autonomous skill creation):** Hermes can mint new skills from successful task patterns and review/archive them over time. Out of scope for v1, but interesting if/when we want Niles to learn its own Tier 0 patterns from real usage.

What *not* to borrow: Hermes's messaging-gateway layer (Telegram/Discord/etc. — not relevant to a voice-first home assistant), its pluggable memory-provider abstraction (we commit to Postgres + pgvector, one canonical backend), and its TUI / Vue dashboards (Niles is headless).

## Pointers

- [ARCHITECTURE.md](ARCHITECTURE.md) — full architectural spec (long, detailed)
- [ROADMAP.md](ROADMAP.md) — current build status across all 13 phases
- [CONTRIBUTING.md](CONTRIBUTING.md) — dev setup and PR conventions for humans
- [README.md](README.md) — human-facing intro
