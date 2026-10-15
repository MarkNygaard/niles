//! Rust bindings for the two worlds Niles runs, generated at build time:
//! `niles:integration/plugin@0.1.0` (`schemas/niles-integration-0.1.wit`)
//! and `@0.2.1` (`schemas/niles-integration.wit`).
//!
//! 0.2 is 0.1 plus `mqtt`, two exports, and (in 0.2.1) one function
//! each in `config` and `devices`. Every other interface the two share
//! is the same WIT under another version. So the 0.2 bindings are told
//! (`with`) to use the 0.1 bindings' modules for those interfaces
//! instead of generating their own: one Rust type per WIT type, one
//! `Host` trait per interface, implemented once in [`crate::host`], and
//! the same argument types for both worlds' exports. `mqtt`, `config`
//! and `devices` are generated for 0.2; their 0.2 `Host` traits hand the
//! functions 0.1 has to the 0.1 implementation.
//!
//! A component names the version it was built against in each import,
//! so [`add_to_linker`] defines each shared interface twice, as
//! `niles:integration/log@0.1.0` and as `niles:integration/log@0.2.1`,
//! with the same host functions behind both names. A component built
//! against 0.2.0 imports `log@0.2.0`, and wasmtime's linker resolves it
//! to `log@0.2.1`: for a `0.x` version, names that differ only in the
//! patch number are semver-compatible. wasmtime then checks the types
//! structurally, function by function, for the functions the component
//! imports, so the functions 0.2.1 added are no obstacle to one that
//! does not know them.

pub mod v0_1 {
    wasmtime::component::bindgen!({
        path: "../../schemas/niles-integration-0.1.wit",
        world: "plugin",
    });
}

pub mod v0_2 {
    wasmtime::component::bindgen!({
        path: "../../schemas/niles-integration.wit",
        world: "plugin",
        with: {
            "niles:integration/types": super::v0_1::niles::integration::types,
            "niles:integration/log": super::v0_1::niles::integration::log,
            "niles:integration/storage": super::v0_1::niles::integration::storage,
            "niles:integration/http": super::v0_1::niles::integration::http,
            "niles:integration/timers": super::v0_1::niles::integration::timers,
            "niles:integration/events": super::v0_1::niles::integration::events,
            "niles:integration/notify": super::v0_1::niles::integration::notify,
            "niles:integration/ui": super::v0_1::niles::integration::ui,
        },
    });
}

pub use v0_1::niles;
pub use v0_2::niles::integration::mqtt;
pub use v0_2::niles::integration::{config as config_0_2, devices as devices_0_2};

use crate::host::HostState;
use wasmtime::component::{HasSelf, Linker};

/// Define every `niles:integration` interface of both worlds.
pub(crate) fn add_to_linker(linker: &mut Linker<HostState>) -> wasmtime::Result<()> {
    v0_1::Plugin::add_to_linker::<_, HasSelf<HostState>>(linker, |state| state)?;
    macro_rules! also_as_0_2 {
        ($($name:ident),*) => {$(
            niles::integration::$name::add_to_linker_instance::<_, HasSelf<HostState>>(
                &mut linker.instance(concat!("niles:integration/", stringify!($name), "@0.2.1"))?,
                |state| state,
            )?;
        )*};
    }
    also_as_0_2!(types, log, storage, http, timers, events, notify, ui);
    config_0_2::add_to_linker::<_, HasSelf<HostState>>(linker, |state| state)?;
    devices_0_2::add_to_linker::<_, HasSelf<HostState>>(linker, |state| state)?;
    mqtt::add_to_linker::<_, HasSelf<HostState>>(linker, |state| state)
}
