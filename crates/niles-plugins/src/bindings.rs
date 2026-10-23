//! Rust bindings for the three worlds Niles runs, generated at build
//! time: `niles:integration/plugin@0.1.0`
//! (`schemas/niles-integration-0.1.wit`), `@0.2.1`
//! (`schemas/niles-integration-0.2.wit`) and `@0.3.0`
//! (`schemas/niles-integration.wit`).
//!
//! 0.2 is 0.1 plus `mqtt`, two exports, and (in 0.2.1) one function
//! each in `config` and `devices`. 0.3 is 0.2.1 plus `secrets`,
//! `inbound` and three exports. Every other interface they share is the
//! same WIT under another version. So each newer world is told (`with`)
//! to use the older bindings' modules for those interfaces instead of
//! generating its own: one Rust type per WIT type, one `Host` trait per
//! interface, implemented once in [`crate::host`], and the same argument
//! types for every world's exports. `mqtt`, `config` and `devices` are
//! generated for 0.2, and their 0.2 `Host` traits hand the functions 0.1
//! has to the 0.1 implementation; 0.3 uses 0.2's for those three and
//! generates only `secrets` and `inbound`.
//!
//! A component names the version it was built against in each import,
//! so [`add_to_linker`] defines each shared interface under each
//! version, `niles:integration/log@0.1.0`, `…@0.2.1` and `…@0.3.0`, with
//! the same host functions behind every name. A component built against
//! 0.2.0 imports `log@0.2.0`, and wasmtime's linker resolves it to
//! `log@0.2.1`: for a `0.x` version, names that differ only in the patch
//! number are semver-compatible. wasmtime then checks the types
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
        path: "../../schemas/niles-integration-0.2.wit",
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

pub mod v0_3 {
    wasmtime::component::bindgen!({
        path: "../../schemas/niles-integration-0.3.wit",
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
            "niles:integration/config": super::v0_2::niles::integration::config,
            "niles:integration/devices": super::v0_2::niles::integration::devices,
            "niles:integration/mqtt": super::v0_2::niles::integration::mqtt,
        },
    });
}

pub mod v0_4 {
    wasmtime::component::bindgen!({
        path: "../../schemas/niles-integration-0.4.wit",
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
            "niles:integration/config": super::v0_2::niles::integration::config,
            "niles:integration/devices": super::v0_2::niles::integration::devices,
            "niles:integration/mqtt": super::v0_2::niles::integration::mqtt,
            "niles:integration/secrets": super::v0_3::niles::integration::secrets,
            "niles:integration/inbound": super::v0_3::niles::integration::inbound,
        },
    });
}

pub mod v0_5 {
    wasmtime::component::bindgen!({
        path: "../../schemas/niles-integration-0.5.wit",
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
            "niles:integration/config": super::v0_2::niles::integration::config,
            "niles:integration/devices": super::v0_2::niles::integration::devices,
            "niles:integration/mqtt": super::v0_2::niles::integration::mqtt,
            "niles:integration/secrets": super::v0_3::niles::integration::secrets,
            "niles:integration/inbound": super::v0_3::niles::integration::inbound,
            "niles:integration/websocket": super::v0_4::niles::integration::websocket,
        },
    });
}

pub mod v0_6 {
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
            "niles:integration/ui": super::v0_1::niles::integration::ui,
            "niles:integration/config": super::v0_2::niles::integration::config,
            "niles:integration/devices": super::v0_2::niles::integration::devices,
            "niles:integration/mqtt": super::v0_2::niles::integration::mqtt,
            "niles:integration/secrets": super::v0_3::niles::integration::secrets,
            "niles:integration/inbound": super::v0_3::niles::integration::inbound,
            "niles:integration/websocket": super::v0_4::niles::integration::websocket,
            "niles:integration/speech-stream": super::v0_5::niles::integration::speech_stream,
        },
    });
}

pub use v0_1::niles;
pub use v0_2::niles::integration::mqtt;
pub use v0_2::niles::integration::{config as config_0_2, devices as devices_0_2};
pub use v0_3::niles::integration::{inbound, secrets};
pub use v0_4::niles::integration::websocket;
pub use v0_5::niles::integration::speech_stream;
pub use v0_6::niles::integration::notify as notify_0_6;

use crate::host::HostState;
use wasmtime::component::{HasSelf, Linker};

/// Define every `niles:integration` interface of all three worlds.
pub(crate) fn add_to_linker(linker: &mut Linker<HostState>) -> wasmtime::Result<()> {
    v0_1::Plugin::add_to_linker::<_, HasSelf<HostState>>(linker, |state| state)?;
    macro_rules! also_as {
        ($version:literal, $module:path, $($name:ident),*) => {$(
            {
                use $module as m;
                m::$name::add_to_linker_instance::<_, HasSelf<HostState>>(
                    &mut linker.instance(concat!(
                        "niles:integration/",
                        stringify!($name),
                        "@",
                        $version
                    ))?,
                    |state| state,
                )?;
            }
        )*};
    }
    also_as!(
        "0.2.1",
        niles::integration,
        types,
        log,
        storage,
        http,
        timers,
        events,
        notify,
        ui
    );
    config_0_2::add_to_linker::<_, HasSelf<HostState>>(linker, |state| state)?;
    devices_0_2::add_to_linker::<_, HasSelf<HostState>>(linker, |state| state)?;
    mqtt::add_to_linker::<_, HasSelf<HostState>>(linker, |state| state)?;
    also_as!(
        "0.3.0",
        niles::integration,
        types,
        log,
        storage,
        http,
        timers,
        events,
        notify,
        ui
    );
    also_as!("0.3.0", v0_2::niles::integration, config, devices, mqtt);
    secrets::add_to_linker::<_, HasSelf<HostState>>(linker, |state| state)?;
    inbound::add_to_linker::<_, HasSelf<HostState>>(linker, |state| state)?;
    also_as_v4(linker)?;
    add_v5(linker)?;
    add_v6(linker)
}

fn also_as_v4(linker: &mut Linker<HostState>) -> wasmtime::Result<()> {
    macro_rules! add { ($module:path, $($name:ident),*) => { $( { use $module as m; m::$name::add_to_linker_instance::<_, HasSelf<HostState>>(&mut linker.instance(concat!("niles:integration/", stringify!($name), "@0.4.0"))?, |s| s)?; } )* }; }
    add!(
        niles::integration,
        types,
        log,
        storage,
        http,
        timers,
        events,
        notify,
        ui
    );
    add!(v0_2::niles::integration, config, devices, mqtt);
    add!(v0_3::niles::integration, secrets, inbound);
    websocket::add_to_linker::<_, HasSelf<HostState>>(linker, |state| state)
}

fn add_v5(linker: &mut Linker<HostState>) -> wasmtime::Result<()> {
    macro_rules! add { ($module:path, $($name:ident),*) => { $( { use $module as i; i::$name::add_to_linker_instance::<_, HasSelf<HostState>>(&mut linker.instance(concat!("niles:integration/", stringify!($name), "@0.5.0"))?, |s| s)?; } )* }; }
    add!(
        niles::integration,
        types,
        log,
        storage,
        http,
        timers,
        events,
        notify,
        ui
    );
    add!(v0_2::niles::integration, config, devices, mqtt);
    add!(v0_3::niles::integration, secrets, inbound);
    add!(v0_4::niles::integration, websocket);
    speech_stream::add_to_linker::<_, HasSelf<HostState>>(linker, |state| state)
}

fn add_v6(linker: &mut Linker<HostState>) -> wasmtime::Result<()> {
    macro_rules! add { ($module:path, $($name:ident),*) => { $( { use $module as i; i::$name::add_to_linker_instance::<_, HasSelf<HostState>>(&mut linker.instance(concat!("niles:integration/", stringify!($name), "@0.6.0"))?, |s| s)?; } )* }; }
    add!(
        niles::integration,
        types,
        log,
        storage,
        http,
        timers,
        events,
        ui
    );
    add!(v0_2::niles::integration, config, devices, mqtt);
    add!(v0_3::niles::integration, secrets, inbound);
    add!(v0_4::niles::integration, websocket);
    add!(v0_5::niles::integration, speech_stream);
    notify_0_6::add_to_linker::<_, HasSelf<HostState>>(linker, |state| state)
}
