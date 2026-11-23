//! Builds the transcoder's child program into the helper.
//!
//! `lumatv-venc` (built for the set by `venc/build.sh`) is what the TV's decoder and encoder are driven from; the helper starts it as a
//! child and nothing else on the set has it, so the helper carries it and the two always come from one commit. A build for any other
//! machine, or one where `venc/build/arm64/lumatv-venc` was not built (a test run), carries nothing: the transcoder then says it was not
//! built in, and nothing else changes.

use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR")).join("lumatv-venc");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=LUMATV_VENC");
    let image = if env::var("CARGO_CFG_TARGET_ARCH").is_ok_and(|a| a == "aarch64") {
        let built = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR")).join("../../venc/build/arm64/lumatv-venc");
        let from = env::var_os("LUMATV_VENC").map_or(built, PathBuf::from);
        // only a build for the set watches the file: cargo runs a script again on every build while a file it watches is missing
        println!("cargo:rerun-if-changed={}", from.display());
        fs::read(&from).unwrap_or_default()
    } else {
        Vec::new()
    };
    fs::write(&out, image).expect("write the child's image into OUT_DIR");
}
