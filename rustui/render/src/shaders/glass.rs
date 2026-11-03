//! The glass material's GLSL: Lucent LANGUAGE.md section 2 as shaders.
//!
//! A glass surface is drawn in up to four parts, each the cheapest thing that can draw it:
//!
//! * the **shadows** and, for a raised control, its light and shade copies: a mesh of rings around the body whose
//!   vertices carry how much of each shadow covers them (`crate::shadow`), so the fragment shader only tints. Worked
//!   out per pixel this cost the Mali nine cycles a pixel over an area larger than the screen.
//! * the **core**: the interior, where nothing but the fill shows: the backdrop (a blurred picture of what is below,
//!   with saturation), the tint over it and the sheen. The tint and the sheen are worked out at the corners of the
//!   interior, which is cut into cells small enough for the sheen's ramps to be as good as exact between them, so that
//!   what a pixel does is one texture read and one multiply-add: the interior is most of what a screen of glass draws.
//! * the **rim**: the strips along the outline, thick enough for everything that happens near an edge, and the four
//!   corners. Here the distance field is evaluated: the anti-aliased edge, the bend of the backdrop (refraction), the
//!   neumorphic inset of a well, the top highlight and the edge line. It is the expensive part per pixel and covers a
//!   fifth of a card or less.
//!
//! The core and the rim have variants compiled at start (backdrop or none, refraction or none, inset or none): an
//! unused feature then costs no registers and no instructions, and the Mali does not have to skip it per pixel.

use std::ffi::CString;

/// What one variant of the rim program has compiled in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RimFeatures {
    /// Samples the blurred backdrop under the tint. Without it the surface is the tint alone.
    pub backdrop: bool,
    /// Bends the backdrop at the edges. Only with a backdrop.
    pub refraction: bool,
    /// Draws the neumorphic inset of a well.
    pub inset: bool,
}

impl RimFeatures {
    const fn new(backdrop: bool, refraction: bool, inset: bool) -> Self {
        Self {
            backdrop,
            refraction,
            inset,
        }
    }

    /// Every combination that can occur.
    pub const ALL: [RimFeatures; 6] = [
        Self::new(false, false, false),
        Self::new(false, false, true),
        Self::new(true, false, false),
        Self::new(true, false, true),
        Self::new(true, true, false),
        Self::new(true, true, true),
    ];
}

const COMMON: &str = "
float rounded_box(vec2 p, vec2 half_size, float radius) {
    vec2 q = abs(p) - half_size + radius;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - radius;
}
mediump vec4 over(mediump vec4 top, mediump vec4 below) {
    return top + below * (1.0 - top.a);
}
";

fn program(defines: &[(&str, bool)], body: &str, precision: &str) -> CString {
    let mut source = String::from("#version 300 es\n");
    for (name, on) in defines {
        source.push_str(&format!("#define {name} {}\n", u8::from(*on)));
    }
    source.push_str(precision);
    source.push_str(COMMON);
    source.push_str(body);
    CString::new(source).expect("GLSL has no NUL")
}

/// The vertex shader of the core and the rim: a quad placed on the CPU.
const SURFACE_VERTEX: &str = "#version 300 es
layout(location = 0) in vec2 a_panel;
layout(location = 1) in vec2 a_local;
uniform vec2 u_ndc;
uniform vec2 u_size;
uniform vec2 u_inv_panel;
out vec2 v_local;
out mediump float v_t;
out vec2 v_uv;
out highp float v_dither;
void main() {
    v_dither = dot(a_panel, vec2(0.7548776662, 0.5698402910));
    v_local = a_local;
    // The position along the 135 degree light axis, 0 at the upper-left corner and 1 at the lower-right one: a linear
    // function of the layer position, so it is interpolated for free.
    v_t = (a_local.x + a_local.y) / (u_size.x + u_size.y);
    // Where the backdrop picture, which covers the panel, is read.
    v_uv = a_panel * u_inv_panel;
    gl_Position = vec4(a_panel * u_ndc - 1.0, 0.0, 1.0);
}";

pub fn surface_vertex() -> CString {
    CString::new(SURFACE_VERTEX).expect("GLSL has no NUL")
}

/// The sheen as a sum of two ramps: `peak` at the corner falling to `floor` at `start`, and on to nothing at `end`.
/// `u_sheen` is (peak, floor, start, end) and `u_sheen_k` the reciprocals of the two ramps' lengths.
const SHEEN: &str = "
mediump float sheen_alpha(mediump float t) {
    return u_sheen.x + (u_sheen.y - u_sheen.x) * clamp(t * u_sheen_k.x, 0.0, 1.0)
        - u_sheen.y * clamp((t - u_sheen.z) * u_sheen_k.y, 0.0, 1.0);
}
";

/// The core's vertex shader. It works out, at each corner, what the fragment shader would otherwise have to at every
/// pixel: the sheen at the corner's place along the light axis, and from it what the interior adds to the backdrop and
/// how much of the backdrop shows:
///
///   colour = sheen + (1 - sheen) * (tint + backdrop * (1 - tint.a)) = added + shown * backdrop.
///
/// Both are already scaled by the surface's opacity. Without a backdrop the colour itself is worked out.
const CORE_VERTEX: &str = "
layout(location = 0) in vec2 a_panel;
layout(location = 1) in vec2 a_local;
uniform vec2 u_ndc;
uniform vec2 u_size;
uniform vec2 u_inv_panel;
uniform mediump vec4 u_tint;
uniform mediump vec4 u_tint_bottom;
uniform mediump vec4 u_sheen;
uniform mediump vec2 u_sheen_k;
uniform mediump float u_opacity;
out mediump vec4 v_colour;
out highp float v_dither;
#if BACKDROP
out mediump vec3 v_seen;
#endif
SHEEN
void main() {
    v_dither = dot(a_panel, vec2(0.7548776662, 0.5698402910));
    // The position along the 135 degree light axis, 0 at the upper-left corner and 1 at the lower-right one.
    mediump float t = clamp((a_local.x + a_local.y) / (u_size.x + u_size.y), 0.0, 1.0);
    mediump float a = 0.0;
    if (u_sheen.x + u_sheen.y > 0.0) {
        a = sheen_alpha(t);
    }
    mediump vec4 tint = mix(u_tint, u_tint_bottom, clamp(a_local.y / u_size.y, 0.0, 1.0));
#if BACKDROP
    v_colour = vec4((vec3(a) + (1.0 - a) * tint.rgb) * u_opacity, u_opacity);
    // Where the backdrop picture, which covers the panel, is read, and how much of it shows.
    v_seen = vec3(a_panel * u_inv_panel, (1.0 - a) * (1.0 - tint.a) * u_opacity);
#else
    v_colour = (vec4(a) + (1.0 - a) * tint) * u_opacity;
#endif
    gl_Position = vec4(a_panel * u_ndc - 1.0, 0.0, 1.0);
}";

/// The vertex shader of the core program; `backdrop` says whether it goes with one that samples a blurred picture.
pub fn core_vertex(backdrop: bool) -> CString {
    let body = CORE_VERTEX.replace("SHEEN\n", SHEEN);
    program(
        &[("BACKDROP", backdrop), ("SATURATION", false)],
        &body,
        "precision highp float;\n",
    )
}

const CORE: &str = concat!(
    dither_glsl!(),
    "
in mediump vec4 v_colour;
in highp float v_dither;
#if BACKDROP
in mediump vec3 v_seen;
uniform sampler2D u_backdrop;
#if SATURATION
uniform mediump float u_saturation;
#endif
#endif
out mediump vec4 o_color;
void main() {
#if BACKDROP
    mediump vec3 seen = texture(u_backdrop, v_seen.xy).rgb;
#if SATURATION
    seen = mix(vec3(dot(seen, vec3(0.299, 0.587, 0.114))), seen, u_saturation);
#endif
    o_color = vec4(v_colour.rgb + v_seen.z * seen, v_colour.a);
#else
    o_color = v_colour;
#endif
    // Noise of under half a level, so that what a blur and a tint make of a ramp is not eight-bit bands (see `dither_glsl!`).
    o_color.rgb += (fract(v_dither) - 0.5) * (1.0 / 255.0);
}"
);

/// The fragment shader of the core program; `backdrop` says whether it samples a blurred picture, and `saturation`
/// whether it changes the picture's saturation (most surfaces leave it as it is).
pub fn core(backdrop: bool, saturation: bool) -> CString {
    program(
        &[("BACKDROP", backdrop), ("SATURATION", saturation)],
        CORE,
        "precision mediump float;\n",
    )
}

const RIM: &str = concat!(
    dither_glsl!(),
    "
in vec2 v_local;
in mediump float v_t;
in vec2 v_uv;
in highp float v_dither;
uniform vec2 u_size;
uniform float u_radius;
uniform float u_scale;
uniform float u_pixel;
uniform mediump float u_opacity;
uniform mediump vec4 u_tint;
uniform mediump vec4 u_tint_bottom;
uniform mediump vec4 u_edge;
uniform float u_edge_width;
uniform mediump vec4 u_highlight;
uniform mediump vec4 u_sheen;
uniform mediump vec2 u_sheen_k;
uniform vec3 u_edge_lens;
#if BACKDROP
uniform sampler2D u_backdrop;
uniform mediump float u_saturation;
#endif
#if REFRACTION
uniform vec4 u_axes;
uniform vec2 u_inv_panel;
uniform vec2 u_bend;
#endif
#if INSET
// The inset's offset, the width of its soft edge, and the corner radius of the blurred shape.
uniform vec4 u_inset;
uniform mediump vec4 u_inset_shade;
uniform mediump vec4 u_inset_light;
#endif
out mediump vec4 o_color;
SHEEN
void main() {
    vec2 half_size = 0.5 * u_size;
    vec2 p = v_local - half_size;
    vec2 room = half_size - abs(p);
    // The distance to the outline: along a straight edge it is the distance to that edge, and only in a corner
    // does it take the square root.
    bool corner = room.x < u_radius && room.y < u_radius;
    float d = corner ? rounded_box(p, half_size, u_radius) : -min(room.x, room.y);
    float cover = clamp(0.5 - d * u_scale, 0.0, 1.0);
    if (cover <= 0.0) {
        o_color = vec4(0.0);
        return;
    }

    mediump vec4 tint = mix(u_tint, u_tint_bottom, clamp(v_local.y / u_size.y, 0.0, 1.0));
#if BACKDROP
    vec2 uv = v_uv;
#if REFRACTION
    if (d > -max(u_bend.y, u_edge_lens.x)) {
        vec2 q = abs(p) - half_size + u_radius;
        vec2 n = max(q.x, q.y) > 0.0 ? normalize(max(q, 0.0)) : (q.x > q.y ? vec2(1.0, 0.0) : vec2(0.0, 1.0));
        n *= sign(p);
        vec2 axis = u_axes.xy * n.x + u_axes.zw * n.y;
        if (u_edge_lens.x > 0.0) {
            float t = clamp(1.0 + d / u_edge_lens.x, 0.0, 1.0);
            uv += axis * (t * t * u_edge_lens.y) * u_inv_panel;
        } else {
            float bend = u_bend.x * (1.0 - smoothstep(0.0, u_bend.y, -d));
            uv -= axis * bend * u_inv_panel;
        }
    }
#endif
    mediump vec3 seen = texture(u_backdrop, uv).rgb;
    if (u_saturation != 1.0) {
        seen = mix(vec3(dot(seen, vec3(0.299, 0.587, 0.114))), seen, u_saturation);
    }
    float clear_rim = u_edge_lens.x > 0.0 ? clamp(1.0 + d / u_edge_lens.x, 0.0, 1.0) * u_edge_lens.z : 0.0;
    tint *= 1.0 - clear_rim;
    mediump vec4 c = vec4(tint.rgb + seen * (1.0 - tint.a), 1.0);
#else
    mediump vec4 c = tint;
#endif

    if (u_sheen.x + u_sheen.y > 0.0) {
        mediump float a = sheen_alpha(clamp(v_t, 0.0, 1.0));
        c = vec4(a) + c * (1.0 - a);
    }

    // What follows is measured from the padding box: inside the edge line.
    mediump float inside = clamp(0.5 - (d + u_edge_width) * u_scale, 0.0, 1.0);
#if INSET
    {
        vec2 pad_half = half_size - u_edge_width;
        float w = max(u_inset.y, u_pixel);
        mediump float shade = smoothstep(-w, w, rounded_box(p - u_inset.x, pad_half, u_inset.z));
        mediump float light = smoothstep(-w, w, rounded_box(p + u_inset.x, pad_half, u_inset.z));
        c = over(u_inset_shade * (shade * inside), c);
        c = over(u_inset_light * (light * inside), c);
    }
#endif
    // The top lip: the padding box minus itself moved one pixel down. It only exists near the top.
    if (u_highlight.a > 0.0 && p.y < u_edge_width + 3.0 - half_size.y) {
        mediump float lower = clamp(0.5 - (rounded_box(p - vec2(0.0, 1.0), half_size, u_radius) + u_edge_width) * u_scale, 0.0, 1.0);
        c = over(u_highlight * (inside * (1.0 - lower)), c);
    }
    // The edge line, the strip just inside the outline.
    mediump float ring = clamp(0.5 + (d + u_edge_width) * u_scale, 0.0, 1.0);
    c = over(u_edge * ring, c);
    o_color = c * (cover * u_opacity);
    // The same noise as the interior's, so that the rim is no cleaner than what it borders.
    o_color.rgb += (fract(v_dither) - 0.5) * (1.0 / 255.0) * o_color.a;
}"
);

/// The fragment shader of the rim program for `features`.
pub fn rim(features: RimFeatures) -> CString {
    let body = RIM.replace("SHEEN\n", SHEEN);
    program(
        &[
            ("BACKDROP", features.backdrop),
            ("REFRACTION", features.refraction),
            ("INSET", features.inset),
        ],
        &body,
        "precision highp float;\n",
    )
}

/// The vertex shader of the plate blit (`ideas::STILL_GLASS`): the surface's own pieces, each pixel read back from the plate at
/// the panel pixel it is drawn on.
const PLATE_VERTEX: &str = "#version 300 es
layout(location = 0) in vec2 a_panel;
layout(location = 1) in vec2 a_local;
uniform vec2 u_ndc;
uniform vec2 u_inv_panel;
out highp vec2 v_uv;
void main() {
    v_uv = a_panel * u_inv_panel;
    gl_Position = vec4(a_panel * u_ndc - 1.0, 0.0, 1.0);
}";

pub fn plate_vertex() -> CString {
    CString::new(PLATE_VERTEX).expect("GLSL has no NUL")
}

/// The plate's pixel, as it is: premultiplied, and drawn over the frame like the surface itself would be.
const PLATE_FRAGMENT: &str = "#version 300 es
precision highp float;
in highp vec2 v_uv;
uniform sampler2D u_plate;
out mediump vec4 o_color;
void main() {
    o_color = texture(u_plate, v_uv);
}";

pub fn plate_fragment() -> CString {
    CString::new(PLATE_FRAGMENT).expect("GLSL has no NUL")
}

/// The vertex shader of the shadow rings: vertices are in layer pixels, placed by the layer's origin and axes.
const RING_VERTEX: &str = "#version 300 es
layout(location = 0) in vec2 a_local;
layout(location = 1) in vec2 a_cover;
uniform vec2 u_ndc;
uniform vec2 u_origin;
uniform vec4 u_axes;
out mediump vec2 v_cover;
void main() {
    v_cover = a_cover;
    vec2 panel = u_origin + u_axes.xy * a_local.x + u_axes.zw * a_local.y;
    gl_Position = vec4(panel * u_ndc - 1.0, 0.0, 1.0);
}";

pub fn ring_vertex() -> CString {
    CString::new(RING_VERTEX).expect("GLSL has no NUL")
}

/// Two fields tinted: the first over the second. The colours are premultiplied and already scaled by the opacity.
const RING_FRAGMENT: &str = "#version 300 es
precision mediump float;
in mediump vec2 v_cover;
uniform mediump vec4 u_colour_a;
uniform mediump vec4 u_colour_b;
out mediump vec4 o_color;
void main() {
    mediump vec4 a = u_colour_a * v_cover.x;
    o_color = a + u_colour_b * v_cover.y * (1.0 - a.a);
}";

pub fn ring_fragment() -> CString {
    CString::new(RING_FRAGMENT).expect("GLSL has no NUL")
}
