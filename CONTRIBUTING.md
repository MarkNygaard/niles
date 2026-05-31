# Contributing to Niles

Thanks for your interest. Niles is in early development — expect frequent breaking changes.

## Development setup

Niles is a Rust monorepo using a Cargo workspace. The pinned toolchain is in `rust-toolchain.toml`; `rustup` will fetch it automatically on first build.

```bash
# Build everything
cargo build --workspace

# Run tests
cargo test --workspace

# Format check
cargo fmt --all --check

# Lint
cargo clippy --workspace --all-targets -- -D warnings
```

CI runs all four on every PR.

## Conventions

- **Branching:** feature branches off `main`, PRs into `main`.
- **Commits:** short imperative summary line; explain *why* in the body if it isn't obvious from the diff.
- **Scope:** keep PRs focused. A single PR can atomically update Rust types, schemas, firmware config, and deployment manifests (that's the point of the monorepo) — but each PR should still be reviewable as one change.

## Larger changes

For any of the following, open an issue or draft RFC first to discuss the approach before writing code:

- Adding a new tier to the voice pipeline
- Adding a new transport (Wyoming alternative, MQTT topic structure changes)
- Adding a new core trait (DeviceSource, RoomSpeaker, etc.) or breaking changes to existing ones
- New top-level crates

Smaller PRs (bug fixes, new tools, additional integrations) are welcome directly.

## Code of conduct

Project participation is governed by [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).
