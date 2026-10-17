# Niles

> Status: **pre-alpha** — under active development, not yet usable.

**Niles** (NYE-uls) — *Neural Intelligence, Lightweight Edge System* — is an open-source, AI-first home automation system designed to replace Home Assistant for users who want sub-second voice interactions, code/config over UI, and a voice assistant with real LLM intelligence as a first-class citizen.

It runs locally on small hardware (a Linux host or Kubernetes cluster), bridges Zigbee devices via Zigbee2MQTT, and exposes a voice interface through ESP32-S3 satellites (a reSpeaker XVF3800 with Niles's own firmware and an on-device "Niles" wake word) with a three-tier intent pipeline: regex → fast LLM → smart LLM.

It is set up from its own app. Settings keeps the base system — the broker your lights are reached through, and who hears and who thinks (ElevenLabs and Cerebras by default) — apart from the add-ons: optional extras such as Sonos, tado°, calendars and the Claude or ChatGPT assistants, and third-party add-ons installed from GitHub.

## Documentation

A navigable docs site lives at [docs/](docs/). Build locally with `mdbook serve docs` and browse at <http://localhost:3000>.

- **[ARCHITECTURE.md](ARCHITECTURE.md)** — the map and the rules: the crates, how a turn flows through them, the house rules and the big decisions. How to use Niles is in the docs site.
- **[AGENTS.md](AGENTS.md)** — how to work in this repo, for AI coding agents and the people who run them.
- **[CONTRIBUTING.md](CONTRIBUTING.md)** — dev setup and PR conventions.
- **[Naming your devices](docs/src/naming.md)** — how to name things in Zigbee2MQTT so Niles finds them, and what to call them.
- **[Set up a satellite](firmware/xiao-wyoming/satellite-idf/README.md#set-up-a-satellite)** — flash a reSpeaker XVF3800 with the Niles firmware and add it to a room.
- **[MANIFEST.md](MANIFEST.md)** — what Niles can do, generated from `features.toml`.

## License

Dual-licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this project by you, as defined in the Apache-2.0 license, shall be dual-licensed as above, without any additional terms or conditions.
