//! Which build of Niles runs, and whether a newer image is published.
//!
//! The image build bakes the commit and build time into the binary
//! (`NILES_GIT_SHA`, `NILES_BUILD_TIME`, read with `option_env!` by
//! [`Build`]); any other build is a dev build, says so, and never
//! checks for a newer one.
//!
//! [`Updates`] asks GitHub without signing in whether `latest` on GHCR
//! is a newer build: an anonymous pull token, then a `HEAD` of the
//! manifest, so a quiet check is two requests and pulls nothing. For a
//! new digest it reads the linux/amd64 image's config for the commit,
//! date and version labels.
//!
//! What changed is listed from GitHub's compare API, at most once per
//! pair of commits, to stay within the 60-requests-an-hour anonymous
//! limit.
//!
//! Nothing here signs in, and nothing names the house: the requests
//! carry Niles's public image and this build's public commit, which is
//! why house rule 26 allows them.
//!
//! A dev build never checks: `cargo run` and CI's own image are dev
//! builds, and a thousand of them polling `latest` would be a
//! self-inflicted denial of service against GitHub's anonymous limit.
//!
//! Phase 2 of NIL-13, the Update button, lands here too: reading and
//! patching Niles's own Deployment through an optional Role.

mod build_info;
mod changes;
mod check;
mod error;
mod http;
mod registry;

pub use build_info::{Build, Commit};
pub use changes::{Change, Changes, GitHub, REPO_URL};
pub use check::{Failed, State, Status, Updates, WhatsNew};
pub use error::{Error, Result};
pub use registry::{IMAGE, ImageDigest, Newest, Registry};
