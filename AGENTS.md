# AGENTS.md

How to work in this repo, for any coding agent (Claude Code, Codex, Hermes, Cursor) and the people who run one. `CLAUDE.md` imports this file and adds only what is particular to Claude Code.

## What Niles is

Niles is an open-source, AI-first home automation system in Rust. It replaces Home Assistant for a household that wants voice answers in well under a second, config over setup screens, and a language model built in. Niles is used in English only; English voices and phrasings are enough.

## Where things are

- `ARCHITECTURE.md` is the map: the crates, how a turn flows, the 27 house rules and the big decisions. Every change keeps the house rules.
- `features.toml`, and the `MANIFEST.md` generated from it, say what exists.
- A crate's doc comments say how it works and why.
- `site/` is getniles.app, a Next.js and Fumadocs site: the landing page, the add-on library, and the docs in `site/content/docs/`. Those are the user guide and, in `site/content/docs/writing-add-ons/`, the plugin author guide.
- Planned work and feature specs are Linear issues in the *Niles software* project.

A new or ambiguous feature gets a Linear issue with the decision written down before any code. A change to a house rule, the crate map or a big decision updates `ARCHITECTURE.md` in the same PR.

The repo is one Cargo workspace (`crates/`, with `niles-bin` the only binary) beside `web/` (the React app), `site/` (the website and the docs), `firmware/xiao-wyoming/satellite-idf/`, `hardware/satellite-v2/`, `schemas/` (the plugin WIT), `deploy/` (Kustomize), `examples/` and `scripts/`.

## Commands

`rust-toolchain.toml` pins stable with rustfmt and clippy. Edition 2024 is set for the whole workspace, and shared dependencies live in `[workspace.dependencies]`. The dev machine is Windows (MSVC, PowerShell); CI is Ubuntu.

The verify chain. Run all of it before every push, including the push of a review fix, and run `cargo fmt --all` before every commit:

```powershell
cargo fmt --all --check
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

When `web/` changed, also run, in `web/`: `bun install` (only if dependencies changed), `npm run build` (tsc and vite) and `npm test` (vitest). When `site/` changed (a docs page included), run `cd site; bun install; bun run typecheck; bun run build`, and `bun run dev` to look at it on http://localhost:4000. When a skill changed, run `python scripts/check-skills.py`.

One crate at a time: `cargo test -p niles-core`, `cargo run -p niles-bin -- --help`.

## Conventions

- **Crate layout.** `lib.rs` declares the modules and re-exports the headline types, so callers write `use niles_core::DeviceRegistry;`. `error.rs` holds the crate's own `Error` (a `thiserror` enum) and `pub type Result<T> = std::result::Result<T, Error>;`. A variant with a dynamic cause names a kind and a reason: `InvalidName { kind: &'static str, reason: String }`. One module per concern. Every `Cargo.toml` has `[lints] workspace = true`.
- **Tests** sit inline under `#[cfg(test)] mod tests` and assert behaviour, not implementation. Several small tests beat one big one.
- **Public surface.** No traits without implementations: a trait lives in the crate of its first implementation, as `DeviceCommander` does in `niles-mqtt`. `#[non_exhaustive]` on public enums that will grow (events, intents). Newtypes for validated identifiers (`RoomName`, `DeviceName`, `MinuteOfDay`), never a raw `String` or `u16`.
- **Units.** Brightness is a `u8` in `0..=100`, colour temperature a `u16` in Kelvin. The source crate translates (Z2M's 0-254, WLED's 0-255, mireds).
- **Comments** explain why. Never restate the code; the default is no comment.
- **features.toml.** A user-visible feature adds or updates its `[[feature]]` entry (`id`, `category`, `summary`, `phrasings`, `since_pr` set to the PR's number). Then `cargo run -p niles-bin -- generate-manifest` rewrites `MANIFEST.md`; the test `committed_manifest_matches_features_toml` fails when the two disagree.
- **The docs.** A user-visible change updates its page under `site/content/docs/` in the same PR: the user guide for what a household sees, `writing-add-ons/` for what a plugin author sees. Pages are MDX, so a bare `{` or `<` in prose is escaped (`\{`, `\<`); links between pages are `/docs/...` routes, and links to repository files are GitHub URLs on `main`.

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

Rulesets protect both branches: no direct pushes, no force-pushes, no deletion, both CI jobs must pass, and Mark (the code owner for everything, in `.github/CODEOWNERS`) must approve. A push after the approval needs a new one. The admin may bypass them only through a pull request.

## Linear

Issues go in the `niles` team, in one of three projects:

- ***Niles software*:** everything outside `firmware/` and `hardware/`. Open to anyone working on Niles.
- ***Satellite firmware*:** `firmware/`. The current XIAO firmware (`firmware/xiao-wyoming/satellite-idf/`) is open to anyone working on Niles. The port to the satellite v2 mainboard is Mark's until his boards work. A firmware change is tested by flashing a real satellite before it merges, because CI doesn't build the firmware; say in the PR what was tried on which satellite.
- ***Satellite v2 PCB*:** `hardware/`, the case and the ordering. Mark's alone; leave it alone.

An agent reaches Linear through Linear's MCP server (`https://mcp.linear.app/mcp`), signed in as the person running it, so what it files is theirs.

- **Filing.** Anyone but Mark files a new issue into Triage. Only Mark moves an issue to Todo, and only he hands one to the ai-harness. An agent never triggers the harness. A good issue says what exists today (with file paths), the decision already made, and when it is done.
- **Before filing, search.** Look through the issues in Triage, Backlog, Todo and the started states (In Progress through Ready for merge) for the same thing. If one already covers it, add what you know as a comment there instead of filing a new one. If one is close but doesn't cover it, file the new issue and mark it *related to* that one, saying in the description how the two differ.
- **Picking one up.** Work only on an issue in Todo that nobody else is assigned to and that isn't delegated to the AI Harness. Assign it to yourself and move it to In Progress before you start.
- **The PR.** Name the branch after the issue (`fix/nil-34-app-timers`), and put `Fixes NIL-34` in the PR body, so Linear's GitHub integration links the two. Move the issue to In Review when the PR is open.
- **After that, it's Mark's.** Changes requested, Functional testing and Ready for merge are his to set. The issue goes to Done when the PR merges.

## Things to avoid

- Local Docker workflows. Everything containerised runs on the household's Kubernetes cluster; don't suggest `docker run` or `docker compose up`.
- `git add -A` or `git add .`. Stage files by name, so an `.env` never slips in.
- Skipping hooks (`--no-verify`, `--no-gpg-sign`). If a hook fails, fix the cause.
- Compatibility shims for code that isn't in production yet. Change the code.
- Premature abstractions. Three similar lines beat a helper used once.
- Half-finished implementations and `todo!()` paths. Scope out what this PR can't finish.
- ConfigMap toggles. A feature works by default: on when the database is there, with its state in Postgres. Never require a `niles.toml` section to switch it on.
- Printing secrets. A credential is never echoed, logged or pasted into an issue, a PR or a test.
