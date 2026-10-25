//! The fade of a feathered clip, as GLSL: a twin of any fragment shader of the compositor that has it compiled in.
//!
//! A layer whose clip is feathered (`Props::feather`) is drawn within the clip's scissor like any other, and its fragment
//! shader multiplies what it writes by a ramp that is 0 on each feathered edge of the clip and 1 the edge's width further in
//! (`crate::feather` says what `u_feather` holds). Every program that draws a layer has a twin made from its own source by
//! [`feathered`]: the program is left exactly as it is, and a layer that has no feather, or lies wholly where the ramp is 1, is
//! drawn by it, so a feather of 0 costs not one instruction. The twin is the same program with one uniform array of two vectors,
//! a handful of operations on the pixel's place, and one multiply of the colour at the end; nothing is drawn twice and no pass
//! is added.

use std::ffi::{CStr, CString};

/// What a twin declares before its `main`: the fade's uniforms and the ramp. `t` holds the four edges' ramps (low x, high x,
/// low y, high y): each is `x * scale + offset`, 0 at its edge and 1 a width in, and an edge that is not feathered has scale 0
/// and offset 1. The least of an axis's two is the axis's ramp (the two overlap only in a clip narrower than both widths), and
/// the corner is the product of the axes'. Everything is `highp`: a ramp over twelve pixels at the far end of the panel needs
/// more than half precision.
const DECLARATIONS: &str = "
uniform highp vec4 u_feather[2];
highp float feather_alpha() {
    highp vec4 t = gl_FragCoord.xxyy * u_feather[0] + u_feather[1];
    highp vec2 m = clamp(min(t.xz, t.yw), 0.0, 1.0);
    return m.x * m.y;
}

";

/// What a twin does at the end of its `main`: the colour (premultiplied, so the multiply fades colour and alpha together) times
/// the fade. A path that leaves `main` early writes nothing that a multiply would change.
const FADE: &str = "    o_color *= feather_alpha();\n}";

/// The twin of a fragment shader that writes `o_color` in a `main` that is its last function: the ramp is declared before
/// `main`, and `main` ends by multiplying `o_color` by it. The program is otherwise the plain one's text, so what it computes
/// before the multiply is the same to the last bit, and where the ramp is 1 so is what it writes.
///
/// # Panics
/// On a source that does not have that shape: it is a mistake in a shader of this crate, and every one is made at start.
pub fn feathered(source: &CStr) -> CString {
    let source = source.to_str().expect("GLSL is ASCII");
    assert!(
        source.contains("o_color"),
        "a shader with a feathered twin writes o_color"
    );
    assert_eq!(
        source.matches("void main()").count(),
        1,
        "a shader with a feathered twin has one main"
    );
    let main = source.find("void main()").expect("one main");
    let end = source.trim_end().len() - 1;
    assert_eq!(
        &source[end..=end],
        "}",
        "a shader with a feathered twin ends with its main"
    );
    // The brace that closes the last thing in the source is the one that closes `main`.
    let mut depth = 0i32;
    for (at, c) in source[main..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                assert!(
                    depth > 0 || main + at == end,
                    "a shader with a feathered twin has nothing after its main"
                );
            }
            _ => {}
        }
    }
    assert_eq!(depth, 0, "a shader's braces are balanced");
    let mut twin = String::with_capacity(source.len() + DECLARATIONS.len() + FADE.len());
    twin.push_str(&source[..main]);
    twin.push_str(DECLARATIONS);
    twin.push_str(&source[main..end]);
    twin.push_str(FADE);
    CString::new(twin).expect("GLSL has no NUL")
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAIN: &CStr = c"#version 300 es
precision mediump float;
uniform vec4 u_from;
out vec4 o_color;
void main() {
    o_color = u_from;
    if (u_from.a == 0.0) {
        return;
    }
}";

    #[test]
    fn a_twin_is_the_program_with_the_ramp_declared_before_main_and_applied_at_its_end() {
        let twin = feathered(PLAIN);
        let twin = twin.to_str().unwrap();
        let plain = PLAIN.to_str().unwrap();
        // Everything before main and everything in it is the program's own, byte for byte.
        let main = plain.find("void main()").unwrap();
        assert!(twin.starts_with(&plain[..main]));
        assert!(twin.contains(&plain[main..plain.len() - 1]));
        // The ramp comes in between, and the multiply last.
        assert!(
            twin.find("uniform highp vec4 u_feather[2];").unwrap()
                < twin.find("void main()").unwrap()
        );
        assert!(twin.ends_with("    o_color *= feather_alpha();\n}"));
        assert_eq!(twin.matches("void main()").count(), 1);
        assert_eq!(twin.matches("u_feather").count(), 3);
    }

    #[test]
    #[should_panic(expected = "one main")]
    fn a_shader_without_one_main_has_no_twin() {
        feathered(c"out vec4 o_color;");
    }

    #[test]
    #[should_panic(expected = "nothing after its main")]
    fn a_shader_with_a_function_after_its_main_has_no_twin() {
        feathered(c"out vec4 o_color;\nvoid main() {\n}\nvoid later() {\n}");
    }

    #[test]
    #[should_panic(expected = "writes o_color")]
    fn a_shader_that_writes_no_colour_has_no_twin() {
        feathered(c"void main() {\n}");
    }
}
