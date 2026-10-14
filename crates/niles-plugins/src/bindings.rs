//! Rust bindings for `schemas/niles-integration.wit`, the
//! `niles:integration/plugin` world, generated at build time.

wasmtime::component::bindgen!({
    path: "../../schemas/niles-integration.wit",
    world: "plugin",
});
