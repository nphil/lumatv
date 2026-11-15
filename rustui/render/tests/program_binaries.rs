use lumatv_render::{gl::Gles, programs::Binaries};
use std::fs;

const VERTEX: &std::ffi::CStr = c"#version 300 es
layout(location = 0) in vec2 position;
void main() { gl_Position = vec4(position, 0.0, 1.0); }";
const FRAGMENT: &std::ffi::CStr = c"#version 300 es
precision mediump float;
out vec4 colour;
void main() { colour = vec4(1.0); }";

#[test]
fn a_kept_program_is_loaded_by_the_next_run_until_it_is_spoilt_or_the_driver_changes() {
    let dir = std::env::temp_dir().join(format!("render-program-binaries-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let run = || {
        let mut gl = Gles::software(c"libEGL.so.1").expect("Mesa software GL is required");
        gl.use_program_binaries(dir.clone()).unwrap();
        gl.program(VERTEX, FRAGMENT).unwrap();
        (gl.programs_linked(), gl.programs_loaded())
    };
    assert_eq!(run(), (1, 0));
    let kept: Vec<_> = fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|e| e == "bin")).collect();
    assert_eq!(kept.len(), 1, "the driver's binary is kept");
    assert_eq!(run(), (0, 1));
    fs::write(&kept[0], b"\x01\x00\x00\x00spoilt").unwrap();
    assert_eq!(run(), (1, 0));
    assert_eq!(run(), (0, 1), "the spoilt binary was replaced");
    Binaries::open(dir.clone(), "another driver build").unwrap();
    assert_eq!(run(), (1, 0), "a binary only loads into the build that made it");
    fs::remove_dir_all(dir).unwrap();
}
