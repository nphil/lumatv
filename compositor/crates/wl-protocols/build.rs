// The protocol code is generated from ../../protocol/*.xml by proc macros, which cargo does not track: without this a
// changed XML leaves the old tables compiled in.
fn main() {
    println!("cargo:rerun-if-changed=../../protocol");
}
