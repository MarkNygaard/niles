//! Rust bindings for the two worlds Niles runs, generated at build time:
//! `niles:integration/plugin@0.1.0` (`schemas/niles-integration-0.1.wit`)
//! and `@0.2.0` (`schemas/niles-integration.wit`).
//!
//! 0.2 is 0.1 plus `mqtt` and two exports, and every interface the two
//! share is the same WIT under another version. So the 0.2 bindings are
//! told (`with`) to use the 0.1 bindings' modules for those interfaces
//! instead of generating their own: one Rust type per WIT type, one
//! `Host` trait per interface, implemented once in [`crate::host`], and
//! the same argument types for both worlds' exports. Only `mqtt` is
//! generated for 0.2.
//!
//! A component names the version it was built against in each import,
//! so [`add_to_linker`] defines each shared interface twice, as
//! `niles:integration/log@0.1.0` and as `niles:integration/log@0.2.0`,
//! with the same host functions behind both names. wasmtime checks the
//! types structurally when a component is linked, and they are the same.

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
            "niles:integration/config": super::v0_1::niles::integration::config,
            "niles:integration/storage": super::v0_1::niles::integration::storage,
            "niles:integration/http": super::v0_1::niles::integration::http,
            "niles:integration/devices": super::v0_1::niles::integration::devices,
            "niles:integration/timers": super::v0_1::niles::integration::timers,
            "niles:integration/events": super::v0_1::niles::integration::events,
            "niles:integration/notify": super::v0_1::niles::integration::notify,
            "niles:integration/ui": super::v0_1::niles::integration::ui,
        },
    });
}

pub use v0_1::niles;
pub use v0_2::niles::integration::mqtt;

use crate::host::HostState;
use wasmtime::component::{HasSelf, Linker};

/// Define every `niles:integration` interface of both worlds.
pub(crate) fn add_to_linker(linker: &mut Linker<HostState>) -> wasmtime::Result<()> {
    v0_1::Plugin::add_to_linker::<_, HasSelf<HostState>>(linker, |state| state)?;
    macro_rules! also_as_0_2 {
        ($($name:ident),*) => {$(
            niles::integration::$name::add_to_linker_instance::<_, HasSelf<HostState>>(
                &mut linker.instance(concat!("niles:integration/", stringify!($name), "@0.2.0"))?,
                |state| state,
            )?;
        )*};
    }
    also_as_0_2!(
        types, log, config, storage, http, devices, timers, events, notify, ui
    );
    mqtt::add_to_linker::<_, HasSelf<HostState>>(linker, |state| state)
}
