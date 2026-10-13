//! Embeds `docs/capabilities/<topic>/SKILL.md` into the crate.
//!
//! The references describe Niles itself, so they ship inside the
//! binary: no directory to mount, nothing in the config file to point at
//! one, and no way for the running build and its references to drift.
//! This writes a list of `(topic, include_str!(path))` pairs that
//! `CapabilityLoader::builtin` parses.

use std::fmt::Write as _;
use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("set by cargo"));
    let root = manifest.join("../../docs/capabilities");
    // A directory is scanned for any change, so adding a topic or
    // editing one rebuilds the crate.
    println!("cargo:rerun-if-changed={}", root.display());

    // Missing is a build error rather than an empty set: an image built
    // without the docs would otherwise ship a Niles that cannot explain
    // itself, and nothing would say so.
    let entries =
        std::fs::read_dir(&root).unwrap_or_else(|e| panic!("reading {}: {e}", root.display()));
    let mut topics: Vec<(String, PathBuf)> = Vec::new();
    for entry in entries {
        let entry = entry.expect("reading docs/capabilities");
        if !entry.file_type().expect("file type").is_dir() {
            continue;
        }
        let topic = entry.file_name().to_string_lossy().into_owned();
        let skill = entry.path().join("SKILL.md");
        assert!(
            skill.is_file(),
            "docs/capabilities/{topic} has no SKILL.md; every directory there is a capability"
        );
        topics.push((topic, skill));
    }
    topics.sort();

    let mut out = String::from("&[\n");
    for (topic, path) in &topics {
        let path = path.to_str().expect("UTF-8 path");
        writeln!(out, "    ({topic:?}, include_str!({path:?})),").expect("write to String");
    }
    out.push_str("]\n");

    let dest = PathBuf::from(std::env::var_os("OUT_DIR").expect("set by cargo"))
        .join("builtin_capabilities.rs");
    std::fs::write(dest, out).expect("writing builtin_capabilities.rs");
}
