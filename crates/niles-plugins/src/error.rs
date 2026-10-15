//! Error types for niles-plugins.

use thiserror::Error;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    /// `niles-integration.toml` could not be read, or says something
    /// Niles cannot accept. The reason is written for the plugin's author.
    #[error("the manifest {0}")]
    Manifest(String),

    /// The `.wasm` is not a component this Niles can run.
    #[error("the component {0}")]
    Component(String),

    /// The WebAssembly engine could not be set up.
    #[error("the plugin runtime could not start: {0}")]
    Engine(String),

    /// No plugin by that id is loaded.
    #[error("there is no plugin called {0}")]
    UnknownPlugin(String),

    /// A rollback was asked for, and there is no earlier version.
    #[error("there is no earlier version of {0} to go back to")]
    NoPrevious(String),

    /// A plugin with this id is already running.
    #[error("a plugin called {0} is already running")]
    AlreadyRunning(String),

    /// Where installed plugins and their data are kept could not be read
    /// or written.
    #[error("plugin storage: {0}")]
    Store(String),
}

pub type Result<T> = std::result::Result<T, Error>;
