//! The blur of a glass backdrop: dual Kawase (Marius Bjørge, "Bandwidth-efficient rendering", SIGGRAPH 2015). A picture
//! of what is below the glass is halved again and again with a five-tap filter, then doubled back up with an
//! eight-tap one; each halving doubles how far the blur reaches, so a wide blur costs a few small passes rather than
//! one with hundreds of taps. All the passes draw one quad over the region that matters, in coordinates that are the
//! same fraction of the panel at every size, so the textures line up whatever their resolution.
//!
//! The taps are placed in units of the source texture's texels, scaled by `u_offset`; `crate::backdrop` chooses the
//! number of levels and the offset that make the whole chain blur as wide as the glass asks. All four channels are
//! blurred, premultiplied as they are drawn: the picture of a glass surface's backdrop is opaque and keeps its alpha
//! of one, and the plate of a prepared backdrop, which is transparent where its group draws nothing, keeps the
//! soft edge the blur makes of that.

use std::ffi::CString;

const VERTEX: &str = "#version 300 es
layout(location = 0) in vec2 a_position;
out vec2 v_uv;
void main() {
    v_uv = a_position * 0.5 + 0.5;
    gl_Position = vec4(a_position, 0.0, 1.0);
}";

pub fn vertex() -> CString {
    CString::new(VERTEX).expect("GLSL has no NUL")
}

/// Halving: the destination texel sits between four source texels; its own tap and four diagonal ones, `u_offset`
/// source texels away, are averaged.
const DOWN: &str = "#version 300 es
precision mediump float;
in highp vec2 v_uv;
uniform sampler2D u_source;
uniform highp vec2 u_texel;
uniform highp float u_offset;
out mediump vec4 o_color;
void main() {
    highp vec2 o = u_texel * u_offset;
    mediump vec4 sum = texture(u_source, v_uv) * 4.0;
    sum += texture(u_source, v_uv + vec2(-o.x, -o.y));
    sum += texture(u_source, v_uv + vec2(o.x, -o.y));
    sum += texture(u_source, v_uv + vec2(-o.x, o.y));
    sum += texture(u_source, v_uv + vec2(o.x, o.y));
    o_color = sum * 0.125;
}";

pub fn down() -> CString {
    CString::new(DOWN).expect("GLSL has no NUL")
}

/// Doubling: four taps along the axes, `u_offset` source texels away, and four diagonal ones half that, each pair
/// twice as heavy.
const UP: &str = "#version 300 es
precision mediump float;
in highp vec2 v_uv;
uniform sampler2D u_source;
uniform highp vec2 u_texel;
uniform highp float u_offset;
out mediump vec4 o_color;
void main() {
    highp vec2 a = u_texel * u_offset;
    highp vec2 h = a * 0.5;
    mediump vec4 sum = texture(u_source, v_uv + vec2(-a.x, 0.0));
    sum += texture(u_source, v_uv + vec2(0.0, a.y));
    sum += texture(u_source, v_uv + vec2(a.x, 0.0));
    sum += texture(u_source, v_uv + vec2(0.0, -a.y));
    sum += (texture(u_source, v_uv + vec2(-h.x, h.y))
        + texture(u_source, v_uv + vec2(h.x, h.y))
        + texture(u_source, v_uv + vec2(h.x, -h.y))
        + texture(u_source, v_uv + vec2(-h.x, -h.y))) * 2.0;
    o_color = sum * (1.0 / 12.0);
}";

pub fn up() -> CString {
    CString::new(UP).expect("GLSL has no NUL")
}
