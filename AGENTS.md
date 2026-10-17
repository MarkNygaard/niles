# AGENTS.md

How to work in this repo, for any coding agent (Claude Code, Codex, Hermes, Cursor) and the people who run one. `CLAUDE.md` imports this file and adds only what is particular to Claude Code.

## What Niles is

Niles is an open-source, AI-first home automation system in Rust. It replaces Home Assistant for a household that wants voice answers in well under a second, config over setup screens, and a language model built in. Niles is used in English only; English voices and phrasings are enough.

## Where things are

- `ARCHITECTURE.md` is the map: the crates, how a turn flows, the 27 house rules and the big decisions. Every change keeps the house rules.
- `features.toml`, and the `MANIFEST.md` generated from it, say what exists.
- A crate's doc comments say how it works and why.
- `docs/src/` is the user guide (an mdBook); `docs/integrations/` is the plugin author guide.
- Planned work and feature specs are Linear issues in the *Niles software* project.

A new or ambiguous feature gets a Linear issue with the decision written down before any code. A change to a house rule, the crate map or a big decision updates `ARCHITECTURE.md` in the same PR.

The repo is one Cargo workspace (`crates/`, with `niles-bin` the only binary) beside `web/` (the React app), `firmware/xiao-wyoming/satellite-idf/`, `hardware/satellite-v2/`, `schemas/` (the plugin WIT), `deploy/` (Kustomize), `examples/` and `scripts/`.

## Commands

`rust-toolchain.toml` pins stable with rustfmt and clippy. Edition 2024 is set for the whole workspace, and shared dependencies live in `[workspace.dependencies]`. The dev machine is Windows (MSVC, PowerShell); CI is Ubuntu.

The verify chain. Run all of it before every push, including the push of a review fix, and run `cargo fmt --all` before every commit:

```powershell
cargo fmt --all --check
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

When `web/` changed, also run, in `web/`: `bun install` (only if dependencies changed), `npm run build` (tsc and vite) and `npm test` (vitest). When a skill changed, run `python scripts/check-skills.py`.

One crate at a time: `cargo test -p niles-core`, `cargo run -p niles-bin -- --help`.

## Conventions

- **Crate layout.** `lib.rs` declares the modules and re-exports the headline types, so callers write `use niles_core::DeviceRegistry;`. `error.rs` holds the crate's own `Error` (a `thiserror` enum) and `pub type Result<T> = std::result::Result<T, Error>;`. A variant with a dynamic cause names a kind and a reason: `InvalidName { kind: &'static str, reason: String }`. One module per concern. Every `Cargo.toml` has `[lints] workspace = true`.
- **Tests** sit inline under `#[cfg(test)] mod tests` and assert behaviour, not implementation. Several small tests beat one big one.
- **Public surface.** No traits without implementations: a trait lives in the crate of its first implementation, as `DeviceCommander` does in `niles-mqtt`. `#[non_exhaustive]` on public enums that will grow (events, intents). Newtypes for validated identifiers (`RoomName`, `DeviceName`, `MinuteOfDay`), never a raw `String` or `u16`.
- **Units.** Brightness is a `u8` in `0..=100`, colour temperature a `u16` in Kelvin. The source crate translates (Z2M's 0-254, WLED's 0-255, mireds).
- **Comments** explain why. Never restate the code; the default is no comment.
- **features.toml.** A user-visible feature adds or updates its `[[feature]]` entry (`id`, `category`, `summary`, `phrasings`, `since_pr` set to the PR's number). Then `cargo run -p niles-bin -- generate-manifest` rewrites `MANIFEST.md`; the test `committed_manifest_matches_features_toml` fails when the two disagree.

### File and function size

Keep a source file under roughly 1,000 to 1,500 lines, not counting its tests. Split it by concern before it grows past that, the way #446 split `crates/niles-bin/src/main.rs` into `cli.rs`, `commands.rs`, `dispatch/` and `wiring/`.

A function is at most 100 lines. Clippy's `too_many_lines` enforces it (the threshold is in `clippy.toml`). A longer function needs `#[expect(clippy::too_many_lines, reason = "...")]` with a real reason, such as "one match over every intent; splitting it scatters the dispatch table".

## The web UI

`web/` is a Vite and React app, served by `niles-api` behind the `ui` feature. Bun installs the dependencies and `bun.lock` is the lockfile. Components come from shadcn/ui on Base UI (`components.json`, `style: "base-nova"`). Two traps that each cost an hour:

- **`npx shadcn@latest add <x>` rewrites an import and installs a package for it.** The generated file says `import { cn } from "cn"` and a `cn` dependency appears in `package.json`. Both are wrong here. Change the import to `@/lib/utils`, run `npm uninstall cn`, then `bun install`, because npm does not update `bun.lock`.
- **Base UI's `Select` popup hangs jsdom.** The run never finishes. Anything worth asserting about what a `Select` offers belongs in a function the test calls directly, like `usableFor` in `RoleCard.tsx` and `roomChoices` in `SatellitesCard.tsx`. `Dialog`, `Popover` and `Switch` are fine, though `Switch` needs the `PointerEvent` shim in `src/test-setup.ts`.

## Skills

Skills live in `.agents/skills/`, where Codex, Hermes and the skills CLI look. `.claude/skills/` is a copy for Claude Code, because the repo has no symlinks (Windows checkouts run with `core.symlinks` off). Edit `.agents/skills/`, then copy the change over `.claude/skills/`. `scripts/check-skills.py` fails CI when the two differ.

Hermes users run `hermes skills trust` once in the repo to load them.

`shadcn` is pinned by hash in `skills-lock.json`, which the skills CLI (`npx skills`) writes. Updating it means updating both copies and the lock. `unslop` lists the AI tells to cut from docs, PR bodies and user-facing text. Niles's own assistant (Claude Code or ChatGPT, in the chat and in artifact jobs) is told the same rules: `niles-bin` compiles `.agents/skills/unslop/SKILL.md` into its system prompt, so an edit there changes how Niles writes.

## Branches, PRs and releases

`development` is the default branch, where work lands. `main` is what is released, and images are built from it.

1. Branch off `development` as `feat/`, `fix/`, `docs/` or `chore/`.
2. Make the change and run the verify chain.
3. Open a PR into `development`. Its title is a Conventional Commit (`feat(niles-core): ...`, `fix: ...`), because release-please reads the titles. The body says what is in, what is out, and how it was verified.
4. Wait for CI. The PR is squash-merged with `--delete-branch`, the only method `development` takes.

release-please (`.github/workflows/release-please.yml`, `release-please-config.json`) keeps a release PR open against `development` with the next version and its `CHANGELOG.md` entry. Merging it tags `vX.Y.Z`. The version is in `Cargo.toml`'s `[workspace.package]`. Then one PR from `development` into `main`, merged as a merge commit (the only method `main` takes), builds the image (`image.yml`).

Rulesets protect both branches: no direct pushes, no force-pushes, no deletion, and both CI jobs must pass. The admin may bypass them only through a pull request.

## Linear

Issues go in the `niles` team, project *Niles software*. Anyone but Mark files a new issue into Triage. Only Mark moves an issue to Todo, and only he hands one to the ai-harness. An agent never triggers the harness.

## Things to avoid

- Local Docker workflows. Everything containerised runs on the household's Kubernetes cluster; don't suggest `docker run` or `docker compose up`.
- `git add -A` or `git add .`. Stage files by name, so an `.env` never slips in.
- Skipping hooks (`--no-verify`, `--no-gpg-sign`). If a hook fails, fix the cause.
- Compatibility shims for code that isn't in production yet. Change the code.
- Premature abstractions. Three similar lines beat a helper used once.
- Half-finished implementations and `todo!()` paths. Scope out what this PR can't finish.
- ConfigMap toggles. A feature works by default: on when the database is there, with its state in Postgres. Never require a `niles.toml` section to switch it on.
- Printing secrets. A credential is never echoed, logged or pasted into an issue, a PR or a test.
