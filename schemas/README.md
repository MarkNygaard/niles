# Shared schemas

The interface between Niles and its third-party plugins, in WIT.

- `niles-integration.wit` is the WIT package `niles:integration@0.3.0`, the file plugin authors copy. 0.3.0 is 0.2.1 plus the `secrets` and `inbound` interfaces and the exports `authorize-url`, `on-callback` and `on-webhook`: connecting an account through its own sign-in page, keeping the tokens it hands back, and a secret address the internet may call. Nothing 0.2.1 had changed. `crates/niles-plugins` runs it; the author guide is in `site/content/docs/writing-add-ons/`. Check it with `wasm-tools component wit schemas/niles-integration.wit`.
- `niles-integration-0.2.wit` is `niles:integration@0.2.1`, frozen. 0.2 is 0.1 plus `mqtt` and two exports, and 0.2.1 adds `config.time-zone` and `devices.set-room-light`. Components built against 0.2.0 run unchanged.
- `niles-integration-0.1.wit` is `niles:integration@0.1.0`, frozen as it shipped.

Niles still runs plugins built against every one of them. They are separate files rather than one with nested packages, so that bindings generated from the current file keep their plain module names (`niles::integration::devices`, not `integration0_3_0`). How the runtime links all three without implementing anything twice is in `crates/niles-plugins/src/bindings.rs`.
