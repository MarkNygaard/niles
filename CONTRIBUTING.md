# Contributing to Niles

Thanks for your interest. Niles is in early development, so expect breaking changes.

## Development setup

Niles is a Rust monorepo in one Cargo workspace. The toolchain is pinned in `rust-toolchain.toml`, and `rustup` fetches it on the first build. The web app in `web/` uses Bun for its dependencies.

Run the whole chain before every push. CI runs it on every PR:

```bash
cargo fmt --all --check
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

If you changed `web/`, also run `npm run build` and `npm test` there. If you changed `site/`, the website and the docs, run `bun install`, `bun run typecheck` and `bun run build` there.

A change people will notice updates its page under `site/content/docs/` in the same PR.

## Pull requests

- Branch off `development` and open the PR into `development`. `main` is what is released.
- The PR title is a Conventional Commit (`feat: ...`, `fix(niles-mqtt): ...`), because release-please builds the changelog from the titles. PRs are squash-merged.
- Keep a PR to one reviewable change. It may still touch Rust, schemas, firmware and deploy manifests together; that is what the monorepo is for.
- A new feature, a new crate, a new transport or a change to a core trait starts as an issue, so the approach is agreed before the code.

The code conventions (crate layout, errors, tests, units, file and function size) are in `AGENTS.md` at the repo root. It is written for coding agents and reads fine for people.

## Code of conduct

Project participation is governed by [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).
