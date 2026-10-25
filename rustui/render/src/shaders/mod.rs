//! The GLSL for the things a layer can be. Layers are drawn as quads already placed and rotated on the CPU
//! (attribute 0: panel position, attribute 1: position inside the layer in layer pixels), so the vertex shaders
//! only scale to clip space.
//!
//! Colours are premultiplied throughout and blended with `ONE, ONE_MINUS_SRC_ALPHA`.
//!
//! The Mali T720 at the 299 MHz the Echo Show runs it at fills about 600 million pixels a second with a one-cycle
//! shader, so what a shader does per pixel is what a full-screen layer costs: a full-screen layer with a 10-cycle
//! shader takes 8 ms of a 16.7 ms frame. Shapes therefore come in three programs of very different cost, and
//! opacity is folded into the colours on the CPU wherever the shader has no edge to anti-alias.

/// `dither()`: noise between -1/2 and 1/2 of an 8-bit level for a fragment shader to add (as `dither() * (1.0 / 255.0)`)
/// to the colour it writes, before the frame buffer rounds it. A smooth gradient across the screen is one of the panel's
/// 255 levels every few dozen pixels and the steps line up into bands, the dark ones being the worst to look at; with the
/// noise each band's edge becomes a ramp of the two levels it lies between, whose average is the exact colour. The noise
/// is the fractional part of a plane through the panel's pixels, at the slopes of Roberts' R2 sequence (the reciprocals of
/// the plastic number and of its square), which spreads the neighbours of a pixel evenly in every direction for a dot
/// product and a `fract`. It belongs to the panel's pixels, not to the layer or to the time, so that it does not swim
/// when a layer moves. One definition, shared by every shader that draws a gradient.
macro_rules! dither_glsl {
    () => {
        "
highp float dither() {
    return fract(dot(gl_FragCoord.xy, vec2(0.7548776662, 0.5698402910))) - 0.5;
}
"
    };
}

pub mod blur;
pub mod fade;
pub mod glass;
pub mod wallpapers;

use std::ffi::CStr;

/// A GLSL source written as a `&str` literal with its NUL, as the `&CStr` GL takes.
const fn glsl(source: &'static str) -> &'static CStr {
    match CStr::from_bytes_with_nul(source.as_bytes()) {
        Ok(source) => source,
        Err(_) => panic!("GLSL source must end in its only NUL"),
    }
}

/// Image layers: the layer's pixels, scaled by opacity.
pub const IMAGE_VERTEX: &CStr = c"#version 300 es
layout(location = 0) in vec2 a_panel;
layout(location = 1) in vec2 a_local;
uniform vec2 u_ndc;
out vec2 v_local;
void main() {
    v_local = a_local;
    gl_Position = vec4(a_panel * u_ndc - 1.0, 0.0, 1.0);
}";

pub const IMAGE_FRAGMENT: &CStr = c"#version 300 es
precision highp float;
in vec2 v_local;
uniform sampler2D u_image;
uniform vec2 u_texel;
uniform float u_opacity;
out mediump vec4 o_color;
void main() {
    o_color = texture(u_image, v_local * u_texel) * u_opacity;
}";

/// Glyph runs, drawn with `IMAGE_VERTEX`: the atlas's coverage where the quad's corners say it is (`v_local` is a place in
/// the atlas, in texels, like an image layer's is a place in its picture, so a quad on whole pixels at scale 1 reads
/// every texel at its centre and the filter takes nothing from the neighbours), times the colour of the ink being drawn (one
/// draw per ink of the run's palette), which the CPU has already multiplied by the layer's opacity (premultiplied, so it is a
/// colour and an alpha in one).
pub const GLYPH_FRAGMENT: &CStr = c"#version 300 es
precision highp float;
in vec2 v_local;
uniform sampler2D u_atlas;
uniform vec2 u_texel;
uniform vec4 u_colour;
out mediump vec4 o_color;
void main() {
    o_color = u_colour * texture(u_atlas, v_local * u_texel).r;
}";

/// Shape layers, all three programs. `u_gradient` is the gradient's direction and the reciprocal of its length
/// across the shape, so the position along the gradient is a linear function of the layer position and is
/// interpolated for free between the vertices; a solid fill is a gradient between equal colours.
pub const SHAPE_VERTEX: &CStr = c"#version 300 es
layout(location = 0) in vec2 a_panel;
layout(location = 1) in vec2 a_local;
uniform vec2 u_ndc;
uniform vec2 u_size;
uniform vec3 u_gradient;
out vec2 v_local;
out float v_along;
void main() {
    v_local = a_local;
    v_along = dot(a_local - 0.5 * u_size, u_gradient.xy) * u_gradient.z + 0.5;
    gl_Position = vec4(a_panel * u_ndc - 1.0, 0.0, 1.0);
}";

/// A rectangle with square corners, no border and a solid colour, on whole pixels: the colour, nothing else.
pub const FLAT_SOLID_FRAGMENT: &CStr = c"#version 300 es
precision mediump float;
uniform vec4 u_from;
out vec4 o_color;
void main() {
    o_color = u_from;
}";

/// The same with a gradient: one multiply-add, and the dither. `u_from` and `u_delta` (the change from one end to the
/// other) are already scaled by the layer's opacity.
pub const FLAT_GRADIENT_FRAGMENT: &CStr = glsl(concat!(
    "#version 300 es
precision mediump float;
",
    dither_glsl!(),
    "in highp float v_along;
uniform vec4 u_from;
uniform vec4 u_delta;
out vec4 o_color;
void main() {
    o_color = u_from + u_delta * v_along;
    o_color.rgb += dither() * (1.0 / 255.0);
}\0"
));

/// Everything else: rounded corners, a border, or an edge that is not on whole pixels and so needs anti-aliasing.
/// Away from the edge and the corners, which is nearly every pixel of a large shape, it costs a compare and a mix.
/// A gradient is dithered; a solid colour (`u_from == u_to`) is not, so it stays exactly its colour.
pub const ROUNDED_FRAGMENT: &CStr = glsl(concat!(
    "#version 300 es
precision highp float;
",
    dither_glsl!(),
    "in vec2 v_local;
in float v_along;
uniform vec2 u_size;
uniform float u_radius;
uniform float u_border_width;
uniform float u_scale;
uniform float u_pixel;
uniform float u_opacity;
uniform mediump vec4 u_from;
uniform mediump vec4 u_to;
uniform mediump vec4 u_border;
out mediump vec4 o_color;

// Distance from p to the edge of a rounded rectangle centred on the origin; negative inside.
float rounded_box(vec2 p, vec2 half_size, float radius) {
    vec2 q = abs(p) - half_size + radius;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - radius;
}

void main() {
    vec2 half_size = 0.5 * u_size;
    vec2 p = v_local - half_size;
    mediump vec4 fill = mix(u_from, u_to, clamp(v_along, 0.0, 1.0));
    if (u_from != u_to) {
        fill.rgb += dither() * (1.0 / 255.0);
    }
    float coverage = 1.0;
    vec2 room = half_size - abs(p);
    // u_pixel is one panel pixel in layer pixels: the width of the anti-aliased edge.
    if (min(room.x, room.y) <= u_radius + u_border_width + u_pixel) {
        float d = rounded_box(p, half_size, u_radius);
        coverage = clamp(0.5 - d * u_scale, 0.0, 1.0);
        if (u_border_width > 0.0) {
            // The border is the strip just inside the edge, drawn over the fill.
            float ring = clamp(0.5 + (d + u_border_width) * u_scale, 0.0, 1.0);
            fill = u_border * ring + fill * (1.0 - u_border.a * ring);
        }
    }
    o_color = fill * (coverage * u_opacity);
}\0"
));
