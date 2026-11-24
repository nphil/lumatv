//! Builds the programs the helper starts as children into the helper.
//!
//! Nothing else on the set has them, so the helper carries them and they always come from one commit with it:
//!
//! - `lumatv-venc` (built for the set by `venc/build.sh`) is what the TV's decoder and encoder are driven from, for the transcoder;
//! - `lumatv-lsbridge` (`lsbridge/build.sh`, 32-bit like LG's luna-service2) is the one bus client that holds the luna subscriptions.
//!
//! A build for any other machine, or one where a program was not built (a test run), carries nothing for it: the transcoder then says it
//! was not built in, the subscriptions each keep a `luna-send` of their own, and nothing else changes.

use std::env;
use std::fs;
use std::path::PathBuf;

/// Writes the image of `name` into `OUT_DIR`: the file `var` names, else `built` (relative to this crate), in a build for the set.
fn embed(name: &str, var: &str, built: &str) {
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR")).join(name);
    println!("cargo:rerun-if-env-changed={var}");
    let image = if env::var("CARGO_CFG_TARGET_ARCH").is_ok_and(|a| a == "aarch64") {
        let built = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR")).join(built);
        let from = env::var_os(var).map_or(built, PathBuf::from);
        // only a build for the set watches the file: cargo runs a script again on every build while a file it watches is missing
        println!("cargo:rerun-if-changed={}", from.display());
        fs::read(&from).unwrap_or_default()
    } else {
        Vec::new()
    };
    fs::write(&out, image).unwrap_or_else(|e| panic!("write the image of {name} into OUT_DIR: {e}"));
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    embed("lumatv-venc", "LUMATV_VENC", "../../venc/build/arm64/lumatv-venc");
    embed("lumatv-lsbridge", "LUMATV_LSBRIDGE", "../lsbridge/build/arm/lumatv-lsbridge");
}
