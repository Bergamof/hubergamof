//! Makes Cargo rebuild the manager when the embedded UI build changes.
//!
//! `rust-embed` reads `ui/build/` from disk at run time in builds with debug
//! assertions, and embeds it otherwise. Only the embedding builds need to
//! track the directory: Cargo then rebuilds when files are added, changed or
//! removed (and on every build while the directory is missing, which only
//! happens for a release built without `make ui-build`).

/// UI build output, relative to this crate.
const UI_BUILD_DIR: &str = "../../ui/build";

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    if std::env::var_os("CARGO_CFG_DEBUG_ASSERTIONS").is_none() {
        println!("cargo::rerun-if-changed={UI_BUILD_DIR}");
    }
}
