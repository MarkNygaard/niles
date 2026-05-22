# Niles

> Status: **pre-alpha** — under active development, not yet usable.

**Niles** (NYE-uls) — *Neural Intelligence, Lightweight Edge System* — is an open-source, AI-first home automation system designed to replace Home Assistant for users who want sub-second voice interactions, code/config over UI, and a voice assistant with real LLM intelligence as a first-class citizen.

It runs locally on small hardware (a Linux host or Kubernetes cluster), bridges Zigbee devices via Zigbee2MQTT, and exposes a voice interface through ESPHome-based satellites with a three-tier intent pipeline: regex → fast LLM → smart LLM.

## Documentation

- **[ARCHITECTURE.md](ARCHITECTURE.md)** — full architectural spec: hardware, voice pipeline, lighting model, scenes, timers, integrations, permissions, deployment.
- **[CLAUDE.md](CLAUDE.md)** — project orientation for Claude Code and other AI coding agents.
- **[CONTRIBUTING.md](CONTRIBUTING.md)** — dev setup and PR conventions.
- `MANIFEST.md` — LLM-facing capability manifest (generated; not yet present, see Phase 11).

## License

Dual-licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this project by you, as defined in the Apache-2.0 license, shall be dual-licensed as above, without any additional terms or conditions.
