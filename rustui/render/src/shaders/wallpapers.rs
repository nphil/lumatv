//! The GLSL of the wallpapers.
//!
//! The Mali T720 at its 299 MHz fills about 600 million pixels a second with a one-cycle shader, so a wallpaper that
//! covers the screen costs its per-pixel cycles times 0.77 ms and nothing that a pixel does may be more than a texture
//! read and a multiply. Everything that varies slowly across the screen is therefore worked out at the corners of a
//! mesh and interpolated (LumaGlass's lesson, `LumaHome.qml`): the warp of the silk and the water, the colour of the
//! clouds, the sky's glow, the fog. Everything made of separate things (stars, fireflies, raindrops, snowflakes, the
//! sun's rays, cloud puffs) is a small quad or mesh per thing whose position and brightness come from one clock, so a
//! frame's cost follows the area the things cover and not the screen's.
//!
//! **Time.** A wallpaper's clock (`Drawn::phase`) grows for as long as the compositor runs; a float cannot hold weeks
//! of seconds to the precision a step of 33 ms needs. The renderer therefore hands the shaders `u_time` (the clock
//! modulo [`LOOP_S`]) and `u_slow` (modulo [`SLOW_LOOP_S`]); everything that repeats does so a whole number of times in
//! the loop it uses, so that the picture at the end of the loop is the picture at its start.
//!
//! **Placement.** Meshes are made once, in the unit square of the layer or around a centre (`wallpapers`), and placed
//! here by the layer's origin and axes like a glass surface's shadow rings, which is what makes the panel's rotation
//! and the layer's scale the same for every kind.

use crate::wallpapers::{LOOP_S, SLOW_LOOP_S};
use std::ffi::CString;

/// What every vertex shader begins with: the placement, the clocks' periods and a few helpers.
fn prelude() -> String {
    let life = crate::wallpaper::RIPPLE_LIFE_S;
    format!(
        "#version 300 es
precision highp float;
precision highp int;
uniform vec2 u_ndc;
uniform vec2 u_origin;
uniform vec4 u_axes;
uniform vec2 u_size;
const float TAU = 6.2831853;
const float LOOP = {LOOP_S:?};
const float SLOW_LOOP = {SLOW_LOOP_S:?};
const float LIFE = {life:?};
vec4 clip_at(vec2 local) {{
    vec2 panel = u_origin + u_axes.xy * local.x + u_axes.zw * local.y;
    return vec4(panel * u_ndc - 1.0, 0.0, 1.0);
}}
// Numbers in 0..1 from a thing's seed, independent enough to scatter its properties.
float rnd(float seed, float k) {{
    return fract(sin(seed * 78.233 + k * 12.9898) * 43758.5453);
}}
// The corner (-1 or 1 each way) that a sprite's vertex is: 0 upper left, 1 upper right, 2 lower left, 3 lower right.
vec2 corner_of(float c) {{
    return vec2(mod(c, 2.0) * 2.0 - 1.0, floor(c * 0.5) * 2.0 - 1.0);
}}
"
    )
}

fn vertex(body: &str) -> CString {
    CString::new(prelude() + body).expect("GLSL has no NUL")
}

/// A vertex shader with `#define LOOK` set or not, placed where the prelude's `#version` line ends.
fn vertex_with_look(look: bool, body: &str) -> CString {
    let source = prelude().replacen(
        "#version 300 es\n",
        &format!("#version 300 es\n#define LOOK {}\n", u8::from(look)),
        1,
    );
    CString::new(source + body).expect("GLSL has no NUL")
}

/// What every fragment shader of the wallpapers begins with: the shared `dither()` (see `shaders::dither_glsl`).
const DITHER: &str = dither_glsl!();

fn fragment(body: &str) -> CString {
    CString::new(format!(
        "#version 300 es\nprecision mediump float;\n{DITHER}{body}"
    ))
    .expect("GLSL has no NUL")
}

// ---------------------------------------------------------------------------------------------------------------
// The picture kinds: silk and liquid.
// ---------------------------------------------------------------------------------------------------------------

/// LumaGlass's slow domain warp (`LumaHome.qml`): two sine fields and a finer third, evaluated at `uv` (0..1 across the
/// layer). `d` is where the picture is read from, less `uv`, in units of `u_params.x`; `light` is the brightness the
/// folds are given, in units of `u_params.y`; `w` are the three fields.
const WARP: &str = "
uniform vec4 u_phase;   // 0.70 t, 0.50 t, 0.40 t, 0.90 t, each in radians and wrapped
uniform float u_phase5; // 0.56 t
uniform vec4 u_params;  // amplitude (of the layer's width), depth, wave scale, detail
void warp(vec2 uv, out vec2 d, out float light, out vec3 w) {
    float k = u_params.z;
    float w1 = sin(uv.y * 6.0 * k + u_phase.x) * cos(uv.x * 4.0 * k - u_phase.y);
    float w2 = sin((uv.x + uv.y) * 5.0 * k - u_phase.z);
    float w3 = sin(uv.x * 11.0 * k + u_phase.w) * sin(uv.y * 9.0 * k - u_phase5);
    d = vec2(w1 + 0.5 * w2, 0.7 * w2 - 0.5 * w1) + u_params.w * 0.5 * vec2(w3, -w3);
    light = 0.3 * (w1 - 0.5 * w2) + u_params.w * 0.25 * w3;
    w = vec3(w1, w2, w3);
}
";

/// Silk: the picture through the warp, its folds brightened and darkened, the three-stop scrim over it. The vignette,
/// the glossy ridges and the bloom LumaGlass has are terms of `u_look`, off unless asked for. What the fragment shader
/// is left with is one texture read, a multiply and an add.
const SILK_VERTEX: &str = "
layout(location = 0) in vec2 a_unit;
layout(location = 1) in vec2 a_aux;
uniform vec3 u_scrim;   // the scrim's alpha at 0, 55 and 100 % of the height
uniform mediump float u_opacity;
#if LOOK
uniform vec3 u_look;    // vignette, specular, bloom
uniform sampler2D u_picture;
out mediump vec3 v_add;
#endif
out highp vec2 v_uv;
out mediump float v_gain;
WARP
#if LOOK
// The picture's brightness around uv, taken from a ring of taps as far out as LumaGlass's 30 px blur reaches: read
// here, at the corners, and interpolated, it costs no texture read per pixel.
vec3 glow_at(vec2 uv) {
    vec2 o = vec2(0.03, 0.03 * u_size.x / u_size.y);
    vec3 sum = texture(u_picture, uv).rgb;
    sum += texture(u_picture, uv + vec2(o.x, 0.0)).rgb + texture(u_picture, uv - vec2(o.x, 0.0)).rgb;
    sum += texture(u_picture, uv + vec2(0.0, o.y)).rgb + texture(u_picture, uv - vec2(0.0, o.y)).rgb;
    sum += texture(u_picture, uv + o).rgb + texture(u_picture, uv - o).rgb;
    sum += texture(u_picture, uv + vec2(o.x, -o.y)).rgb + texture(u_picture, uv + vec2(-o.x, o.y)).rgb;
    return sum / 9.0;
}
#endif
void main() {
    vec2 uv = a_unit;
    vec2 d;
    float light;
    vec3 w;
    warp(uv, d, light, w);
    v_uv = uv + u_params.x * d;
    light = 1.0 + u_params.y * light;
#if LOOK
    // A narrow lobe on the crest of the main field.
    float crest = clamp(0.5 + 0.5 * (w.x + 0.4 * w.z), 0.0, 1.0);
    float crest2 = crest * crest;
    float spec = u_look.y * 0.35 * crest2 * crest2 * crest2;
    // The vignette is folded into the light: a quadratic falloff, corrected for the layer's shape.
    vec2 q = (uv - 0.5) * vec2(1.0, u_size.y / u_size.x);
    light *= 1.0 - u_look.x * 0.75 * smoothstep(0.12, 0.42, dot(q, q));
    vec3 add = vec3(spec);
    if (u_look.z > 0.001) {
        add += max(glow_at(v_uv) - 0.3, 0.0) * u_look.z;
    }
#endif
    float y = clamp(v_uv.y, 0.0, 1.0);
    float scrim = y < 0.55 ? mix(u_scrim.x, u_scrim.y, y / 0.55) : mix(u_scrim.y, u_scrim.z, (y - 0.55) / 0.45);
    // The layer's opacity is folded in, so that a pixel is a read and a multiply.
    v_gain = light * (1.0 - scrim) * u_opacity;
#if LOOK
    v_add = add * (1.0 - scrim) * u_opacity;
#endif
    gl_Position = clip_at(uv * u_size);
}
";

/// Liquid: the same warp, slower and wider, for the swell, and the taps' ripples on top: rings that spread from a tap,
/// thin as they go and are gone after `LIFE` seconds. Each ring is a wave packet in the distance from the tap; the
/// picture is read at where the surface's slope bends the light from, and the slopes facing the light are brighter.
const LIQUID_VERTEX: &str = "
layout(location = 0) in vec2 a_unit;
layout(location = 1) in vec2 a_aux;
uniform int u_count;
uniform vec4 u_ripple[6];   // the tap's x and y in layer pixels, its age in seconds, its strength
uniform vec3 u_wave;        // the ripples' height in pixels, the refraction in pixels of shift per unit of slope, the light
uniform mediump float u_opacity;
out highp vec4 v_lit;       // where the picture is read from, the light on it and the glint added to it
WARP
const float LAMBDA = 76.0;  // the distance between crests, in pixels
const float SPEED = 190.0;  // how fast a ring spreads, in pixels a second
const float WIDTH = 70.0;   // how far from the crest the packet reaches
const vec2 TOWARDS_LIGHT = vec2(-0.6, -0.8);
vec2 ripple_slope(vec2 local, vec4 r) {
    vec2 d = local - r.xy;
    float rho = max(length(d), 1.0e-3);
    float s = rho - SPEED * r.z;
    float env = exp(-(s * s) / (WIDTH * WIDTH));
    float amp = r.w * exp(-1.1 * r.z) * smoothstep(0.0, 0.12, r.z) * (1.0 - smoothstep(0.7 * LIFE, LIFE, r.z))
        / sqrt(1.0 + rho / 80.0);
    float k = TAU / LAMBDA;
    float slope = amp * env * (-k * sin(k * s) - 2.0 * s / (WIDTH * WIDTH) * cos(k * s));
    return slope * d / rho;
}
void main() {
    vec2 uv = a_unit;
    vec2 local = uv * u_size;
    vec2 d;
    float light;
    vec3 w;
    warp(uv, d, light, w);
    vec2 slope = vec2(0.0);
    for (int i = 0; i < 6; i++) {
        if (i >= u_count) {
            break;
        }
        slope += ripple_slope(local, u_ripple[i]);
    }
    slope *= u_wave.x;
    float lit = dot(slope, TOWARDS_LIGHT);
    float gain = 1.0 + u_params.y * light + u_wave.z * lit;
    float glint = 0.22 * smoothstep(0.12, 0.45, lit);
    // The layer's opacity is folded in, so that a pixel is a read, a multiply and an add.
    v_lit = vec4(uv + u_params.x * d - slope * u_wave.y / u_size, gain * u_opacity, glint * u_opacity);
    gl_Position = clip_at(local);
}
";

/// The fragment shader of silk: the picture where the vertices read it from, lit; the light already has the layer's
/// opacity in it. With `LOOK` there is light to add as well.
const SILK_FRAGMENT: &str = "
in highp vec2 v_uv;
in mediump float v_gain;
#if LOOK
in mediump vec3 v_add;
#endif
uniform sampler2D u_picture;
uniform mediump float u_opacity;
out mediump vec4 o_color;
void main() {
#if LOOK
    o_color = vec4(texture(u_picture, v_uv).rgb * v_gain + v_add + dither() * (1.0 / 255.0), u_opacity);
#else
    o_color = vec4(texture(u_picture, v_uv).rgb * v_gain + dither() * (1.0 / 255.0), u_opacity);
#endif
}
";

/// The fragment shader of liquid: the picture where the vertices read it from, lit, with the glints added; the light and
/// the glints already have the layer's opacity in them.
const LIQUID_FRAGMENT: &str = "
in highp vec4 v_lit;
uniform sampler2D u_picture;
uniform mediump float u_opacity;
out mediump vec4 o_color;
void main() {
    o_color = vec4(texture(u_picture, v_lit.xy).rgb * v_lit.z + v_lit.w + dither() * (1.0 / 255.0), u_opacity);
}
";

pub fn silk_fragment(look: bool) -> CString {
    CString::new(format!(
        "#version 300 es\n#define LOOK {}\nprecision mediump float;\n{DITHER}{SILK_FRAGMENT}",
        u8::from(look)
    ))
    .expect("GLSL has no NUL")
}

pub fn silk_vertex(look: bool) -> CString {
    vertex_with_look(look, &SILK_VERTEX.replace("WARP", WARP))
}

pub fn liquid_vertex() -> CString {
    vertex(&LIQUID_VERTEX.replace("WARP", WARP))
}

pub fn liquid_fragment() -> CString {
    fragment(LIQUID_FRAGMENT)
}

// ---------------------------------------------------------------------------------------------------------------
// The colour fields: aurora, the sky, the fog. One colour a vertex, interpolated.
// ---------------------------------------------------------------------------------------------------------------

/// The four colours of the aurora as soft clouds that wander over a dark ground, their outlines drifting with a warp of
/// the space they are in. Each cloud's path is a whole number of turns in the loop.
const AURORA_VERTEX: &str = "
layout(location = 0) in vec2 a_unit;
layout(location = 1) in vec2 a_aux;
uniform float u_time;
uniform vec3 u_colour[4];
uniform float u_strength;
out mediump vec4 v_colour;
float cloud(vec2 p, vec2 centre, vec2 radius) {
    vec2 d = (p - centre) / radius;
    return exp(-dot(d, d));
}
void main() {
    float t = u_time * TAU / LOOP;
    float aspect = u_size.x / u_size.y;
    vec2 p = a_unit * vec2(aspect, 1.0);
    p += 0.10 * vec2(sin(p.y * 2.7 + 4.0 * t), sin(p.x * 2.1 - 3.0 * t + 1.0));
    vec3 c = vec3(0.040, 0.036, 0.068);
    c += u_colour[0] * cloud(p, vec2(aspect * (0.50 + 0.36 * sin(3.0 * t + 0.4)), 0.50 + 0.30 * sin(4.0 * t + 1.1)), vec2(0.50, 0.30)) * u_strength;
    c += u_colour[1] * cloud(p, vec2(aspect * (0.50 + 0.38 * sin(4.0 * t + 2.6)), 0.50 + 0.32 * sin(2.0 * t + 0.2)), vec2(0.46, 0.32)) * u_strength;
    c += u_colour[2] * cloud(p, vec2(aspect * (0.50 + 0.34 * sin(2.0 * t + 4.1)), 0.50 + 0.30 * sin(5.0 * t + 2.3)), vec2(0.52, 0.28)) * u_strength;
    c += u_colour[3] * cloud(p, vec2(aspect * (0.50 + 0.40 * sin(5.0 * t + 5.5)), 0.50 + 0.28 * sin(3.0 * t + 3.9)), vec2(0.44, 0.30)) * u_strength;
    v_colour = vec4(1.0 - exp(-c * 1.15), 1.0);
    gl_Position = clip_at(a_unit * u_size);
}
";

/// The night sky's ground: a gradient from the zenith at the top to a lighter horizon, and a faint milky way, a broad
/// band across the sky that turns with the stars.
const SKY_VERTEX: &str = "
layout(location = 0) in vec2 a_unit;
layout(location = 1) in vec2 a_aux;
uniform float u_slow;
out mediump vec4 v_colour;
void main() {
    vec2 centred = (a_unit - 0.5) * u_size;
    float angle = u_slow * TAU / SLOW_LOOP;
    vec2 rot = vec2(cos(angle), sin(angle));
    // The point of the sky this corner shows: the screen turned back by the sky's angle.
    vec2 sky = vec2(centred.x * rot.x + centred.y * rot.y, -centred.x * rot.y + centred.y * rot.x);
    float across = sky.y * 0.94 - sky.x * 0.34;
    float along = sky.x * 0.94 + sky.y * 0.34;
    float band = exp(-(across * across) / (150.0 * 150.0)) * (0.65 + 0.35 * sin(along / 95.0 + 2.0));
    vec3 c = mix(vec3(0.018, 0.016, 0.036), vec3(0.070, 0.058, 0.115), a_unit.y * a_unit.y);
    c += vec3(0.034, 0.028, 0.060) * band;
    v_colour = vec4(c, 1.0);
    gl_Position = clip_at(a_unit * u_size);
}
";

/// Fog: a veil whose density is a slow sum of sines across the screen, denser towards the bottom, drifting sideways.
const FOG_VERTEX: &str = "
layout(location = 0) in vec2 a_unit;
layout(location = 1) in vec2 a_aux;
uniform float u_time;
uniform vec2 u_fog;   // the veil's most opaque alpha, unused
out mediump vec4 v_colour;
void main() {
    float t = u_time * TAU / LOOP;
    vec2 p = a_unit * vec2(u_size.x / u_size.y, 1.0);
    float n = 0.5 + 0.5 * sin(p.x * 2.4 + 2.0 * sin(p.y * 2.9 + 3.0 * t) + 2.0 * t);
    n *= 0.65 + 0.35 * sin(p.x * 5.3 - 4.0 * t + 4.0 * sin(p.y * 3.7 + t));
    float a = u_fog.x * mix(0.45, 1.0, a_unit.y) * (0.20 + 0.80 * n);
    v_colour = vec4(vec3(0.80, 0.79, 0.86) * a, a);
    gl_Position = clip_at(a_unit * u_size);
}
";

/// The fragment shader of the colour fields: the colour the corners interpolate, dithered.
const FIELD_FRAGMENT: &str = "
in mediump vec4 v_colour;
uniform mediump float u_opacity;
out mediump vec4 o_color;
void main() {
    o_color = vec4(v_colour.rgb * u_opacity + dither() * (1.0 / 255.0), v_colour.a * u_opacity);
}
";

pub fn aurora_vertex() -> CString {
    vertex(AURORA_VERTEX)
}

pub fn sky_vertex() -> CString {
    vertex(SKY_VERTEX)
}

pub fn fog_vertex() -> CString {
    vertex(FOG_VERTEX)
}

pub fn field_fragment() -> CString {
    fragment(FIELD_FRAGMENT)
}

// ---------------------------------------------------------------------------------------------------------------
// The things: stars, fireflies, rain, snow. A sprite is a quad of six vertices that all carry the thing's seed.
// ---------------------------------------------------------------------------------------------------------------

/// The stars: fixed in a sky that turns about the middle of the screen, once in `SLOW_LOOP`, each twinkling with a
/// period of its own between 2 and 7 seconds. Homes are in half-diagonals of the layer from its centre, so that the
/// sky covers the corners as it turns.
const STARS_VERTEX: &str = "
layout(location = 0) in vec2 a_home;
layout(location = 1) in vec2 a_aux;   // seed, corner
uniform float u_time;
uniform float u_slow;
out mediump vec2 v_corner;
out mediump vec4 v_tint;
void main() {
    float seed = a_aux.x;
    vec2 corner = corner_of(a_aux.y);
    float r1 = rnd(seed, 1.0);
    float r2 = rnd(seed, 2.0);
    float r3 = rnd(seed, 3.0);
    float r4 = rnd(seed, 4.0);
    float angle = u_slow * TAU / SLOW_LOOP;
    vec2 rot = vec2(cos(angle), sin(angle));
    vec2 home = a_home * (0.5 * length(u_size));
    vec2 p = 0.5 * u_size + vec2(home.x * rot.x - home.y * rot.y, home.x * rot.y + home.y * rot.x);
    // Most stars are dim, a few are bright.
    float base = 0.30 + 0.70 * r1 * r1 * r1;
    float twinkle = 0.5 + 0.5 * sin(u_time * TAU / LOOP * (43.0 + floor(r2 * 78.0)) + TAU * r3);
    float bright = base * (0.45 + 0.55 * twinkle);
    float radius = 0.9 + 2.3 * base;
    float extent = radius * 1.7 + 1.0;
    vec3 tint = r4 < 0.33 ? vec3(0.77, 0.65, 0.91) : (r4 < 0.66 ? vec3(0.61, 0.81, 0.85) : vec3(0.96, 0.76, 0.47));
    tint = mix(vec3(1.0), tint, 0.35);
    v_tint = vec4(tint * bright, bright * 0.9);
    v_corner = corner;
    gl_Position = clip_at(p + corner * extent);
}
";

/// Fireflies: each lives between 7 and 15 seconds, rising a little from its home while it sways sideways, fading in and
/// out and blinking more slowly than it moves.
const FIREFLIES_VERTEX: &str = "
layout(location = 0) in vec2 a_home;   // in the layer's width and height
layout(location = 1) in vec2 a_aux;    // seed, corner
uniform float u_time;
uniform vec3 u_colour;
out mediump vec2 v_corner;
out mediump vec4 v_tint;
void main() {
    float seed = a_aux.x;
    vec2 corner = corner_of(a_aux.y);
    float r1 = rnd(seed, 1.0);
    float r2 = rnd(seed, 2.0);
    float r3 = rnd(seed, 3.0);
    float r4 = rnd(seed, 4.0);
    float r5 = rnd(seed, 5.0);
    float r6 = rnd(seed, 6.0);
    float turns = floor(LOOP / (7.0 + 8.0 * r1));
    float t = fract(u_time * turns / LOOP + r2);
    float rise = 36.0 + 90.0 * r3;
    float sway = (12.0 + 22.0 * r4) * sin(u_time * TAU / LOOP * (25.0 + floor(r5 * 46.0)) + TAU * r6);
    vec2 p = a_home * u_size + vec2(sway, rise * (0.5 - t));
    float fade = smoothstep(0.0, 0.22, t) * (1.0 - smoothstep(0.65, 1.0, t));
    float blink = 0.72 + 0.28 * sin(u_time * TAU / LOOP * (60.0 + floor(r3 * 80.0)) + TAU * r1);
    float alpha = fade * blink * (0.75 + 0.25 * r4);
    v_tint = vec4(u_colour * alpha, alpha * 0.45);
    v_corner = corner;
    gl_Position = clip_at(p + corner * 13.0);
}
";

/// Snow: flakes of 1.7 to 5 px radius falling at speeds that follow their size (the near ones are bigger and quicker),
/// each swaying about its path with a period of its own.
const SNOW_VERTEX: &str = "
layout(location = 0) in vec2 a_home;   // x in the layer's width
layout(location = 1) in vec2 a_aux;    // seed, corner
uniform float u_time;
uniform vec3 u_colour;
out mediump vec2 v_corner;
out mediump vec4 v_tint;
void main() {
    float seed = a_aux.x;
    vec2 corner = corner_of(a_aux.y);
    float r1 = rnd(seed, 1.0);
    float r2 = rnd(seed, 2.0);
    float r3 = rnd(seed, 3.0);
    float r4 = rnd(seed, 4.0);
    float r5 = rnd(seed, 5.0);
    float radius = 1.7 + 3.3 * r1 * r1;
    float margin = radius + 24.0;
    float span = u_size.y + 2.0 * margin;
    float speed = 22.0 + 16.0 * radius;
    float turns = max(floor(LOOP * speed / span + 0.5), 1.0);
    float t = fract(u_time * turns / LOOP + r2);
    float sway = (10.0 + 24.0 * r3) * sin(u_time * TAU / LOOP * (20.0 + floor(r4 * 40.0)) + TAU * r5);
    vec2 p = vec2(a_home.x * u_size.x + sway, -margin + t * span);
    float alpha = 0.50 + 0.45 * r5;
    v_tint = vec4(u_colour * alpha, alpha);
    v_corner = corner;
    gl_Position = clip_at(p + corner * (radius + 1.5));
}
";

/// The disc of a sprite: a bright core and a soft halo, `u_disc` being the core's radius as a fraction of the quad and
/// how much of the picture is the halo.
const DISC_FRAGMENT: &str = "
in mediump vec2 v_corner;
in mediump vec4 v_tint;
uniform mediump vec2 u_disc;
uniform mediump float u_opacity;
out mediump vec4 o_color;
void main() {
    mediump float d = length(v_corner);
    mediump float core = 1.0 - smoothstep(0.0, u_disc.x, d);
    mediump float halo = max(1.0 - d * d, 0.0);
    halo *= halo;
    o_color = v_tint * (mix(core, halo, u_disc.y) * u_opacity);
}
";

/// Rain: streaks that each fall the height of the layer plus their length in between 0.4 and 1 second, a whole number
/// of times in the loop, along a slant `u_rain.x` (pixels across per pixel down). The quad is the streak: `corner.x`
/// across it, `corner.y` along it, the head being the lower end.
const RAIN_VERTEX: &str = "
layout(location = 0) in vec2 a_home;   // x across the layer, 0..1
layout(location = 1) in vec2 a_aux;    // seed, corner
uniform float u_time;
uniform vec4 u_rain;   // the slant, the length scale, the brightness
out mediump vec2 v_corner;
out mediump vec4 v_tint;
void main() {
    float seed = a_aux.x;
    vec2 corner = corner_of(a_aux.y);
    float r1 = rnd(seed, 1.0);
    float r2 = rnd(seed, 2.0);
    float r3 = rnd(seed, 3.0);
    float r4 = rnd(seed, 4.0);
    float len = (22.0 + 26.0 * r1) * u_rain.y;
    float span = u_size.y + 2.0 * len;
    float turns = max(floor(LOOP * (620.0 + 520.0 * r2) / span + 0.5), 1.0);
    float t = fract(u_time * turns / LOOP + r3);
    float y = -len + t * span;
    vec2 dir = normalize(vec2(u_rain.x, 1.0));
    float slide = u_rain.x * span;
    float x = a_home.x * (u_size.x + slide) - slide + u_rain.x * (y + len);
    vec2 across = vec2(dir.y, -dir.x);
    vec2 p = vec2(x, y) + dir * (corner.y * 0.5 * len) + across * (corner.x * (0.8 + 0.5 * r4));
    float bright = u_rain.z * (0.22 + 0.30 * r4);
    v_tint = vec4(vec3(0.80, 0.86, 0.98) * bright, bright * 0.8);
    v_corner = corner;
    gl_Position = clip_at(p);
}
";

const STREAK_FRAGMENT: &str = "
in mediump vec2 v_corner;
in mediump vec4 v_tint;
uniform mediump float u_opacity;
out mediump vec4 o_color;
void main() {
    mediump float across = 1.0 - abs(v_corner.x);
    mediump float along = v_corner.y * 0.5 + 0.5;
    mediump float a = across * (0.12 + 0.88 * along * along);
    o_color = v_tint * (a * u_opacity);
}
";

pub fn stars_vertex() -> CString {
    vertex(STARS_VERTEX)
}

pub fn fireflies_vertex() -> CString {
    vertex(FIREFLIES_VERTEX)
}

pub fn snow_vertex() -> CString {
    vertex(SNOW_VERTEX)
}

pub fn rain_vertex() -> CString {
    vertex(RAIN_VERTEX)
}

pub fn disc_fragment() -> CString {
    fragment(DISC_FRAGMENT)
}

pub fn streak_fragment() -> CString {
    fragment(STREAK_FRAGMENT)
}

// ---------------------------------------------------------------------------------------------------------------
// The lights and the clouds: meshes whose corners carry the alpha.
// ---------------------------------------------------------------------------------------------------------------

/// The sun's rays: thin wedges from the sun, each sweeping a little and pulsing on its own, brightest near the sun and
/// gone by the far end. A wedge is the apex and two rows of five corners (across -1 to 1; along 0.5 and 1) with the alpha
/// a bell across it, highest along its middle.
const RAYS_VERTEX: &str = "
layout(location = 0) in vec2 a_wedge;   // across (-1 to 1), along (0, 0.5 or 1)
layout(location = 1) in vec2 a_aux;     // seed
uniform float u_time;
uniform vec4 u_sun;    // the sun's place in the layer's width and height, its brightness
out mediump vec4 v_colour;
void main() {
    float seed = a_aux.x;
    float r1 = rnd(seed, 1.0);
    float r2 = rnd(seed, 2.0);
    float r3 = rnd(seed, 3.0);
    float r4 = rnd(seed, 4.0);
    float r5 = rnd(seed, 5.0);
    float r6 = rnd(seed, 6.0);
    float across = a_wedge.x;
    float along = a_wedge.y;
    vec2 sun = u_sun.xy * u_size;
    float sway = 0.045 * sin(u_time * TAU / LOOP * (4.0 + floor(r2 * 6.0)) + TAU * r3);
    float half_width = 0.030 + 0.032 * r4;
    float angle = mix(1.55, 2.75, r1) + sway + across * half_width;
    vec2 p = sun + vec2(cos(angle), sin(angle)) * (along * 1.15 * length(u_size));
    float pulse = 0.65 + 0.35 * sin(u_time * TAU / LOOP * (3.0 + floor(r5 * 5.0)) + TAU * r6);
    float reach = along < 0.25 ? 1.0 : (along < 0.75 ? 0.62 : 0.0);
    float bell = 1.0 - across * across;
    float a = u_sun.z * pulse * bell * bell * reach;
    v_colour = vec4(vec3(1.0, 0.86, 0.56) * a, a * 0.55);
    gl_Position = clip_at(p);
}
";

/// The sun's glow: a disc around the sun, brightest at its centre.
const GLOW_VERTEX: &str = "
layout(location = 0) in vec2 a_disc;   // in the disc's radii from its centre
layout(location = 1) in vec2 a_aux;    // radius of the ring, seed
uniform vec4 u_sun;
out mediump vec4 v_colour;
void main() {
    vec2 sun = u_sun.xy * u_size;
    float bell = 1.0 - a_aux.x * a_aux.x;
    float a = u_sun.w * bell * bell;
    v_colour = vec4(vec3(1.0, 0.84, 0.52) * a, a * 0.6);
    gl_Position = clip_at(sun + a_disc * (0.34 * length(u_size)));
}
";

/// Cloud puffs: soft ellipses of 170 to 300 px, each drifting across the layer once in some multiple of the slow loop
/// and rising and sinking a little; alpha falls smoothly from the middle to nothing at the rim.
const PUFFS_VERTEX: &str = "
layout(location = 0) in vec2 a_disc;   // in the puff's radii from its centre
layout(location = 1) in vec2 a_aux;    // radius of the ring, seed
uniform float u_time;
uniform float u_slow;
uniform vec2 u_cloud;   // the most opaque a puff is, unused
out mediump vec4 v_colour;
void main() {
    float seed = a_aux.y;
    float r1 = rnd(seed, 1.0);
    float r2 = rnd(seed, 2.0);
    float r3 = rnd(seed, 3.0);
    float r4 = rnd(seed, 4.0);
    float r5 = rnd(seed, 5.0);
    float r6 = rnd(seed, 6.0);
    float rx = mix(170.0, 300.0, r1);
    float ry = rx * mix(0.48, 0.68, r2);
    float span = u_size.x + 2.0 * rx;
    float turns = 6.0 + floor(r3 * 9.0);
    float x = -rx + fract(r4 + u_slow * turns / SLOW_LOOP) * span;
    float y = u_size.y * mix(0.08, 0.60, r5) + 9.0 * sin(u_time * TAU / LOOP * (5.0 + floor(r6 * 5.0)) + TAU * r1);
    float bell = 1.0 - a_aux.x * a_aux.x;
    float a = u_cloud.x * (0.6 + 0.4 * r6) * bell * bell;
    v_colour = vec4(vec3(0.83, 0.81, 0.90) * a, a);
    gl_Position = clip_at(vec2(x, y) + a_disc * vec2(rx, ry));
}
";

/// A flat colour over the whole layer: the storm's darkness and its flashes.
const FLAT_VERTEX: &str = "
layout(location = 0) in vec2 a_unit;
layout(location = 1) in vec2 a_aux;
void main() {
    gl_Position = clip_at(a_unit * u_size);
}
";

const FLAT_FRAGMENT: &str = "
uniform mediump vec4 u_colour;
uniform mediump float u_opacity;
out mediump vec4 o_color;
void main() {
    o_color = u_colour * u_opacity;
}
";

pub fn rays_vertex() -> CString {
    vertex(RAYS_VERTEX)
}

pub fn glow_vertex() -> CString {
    vertex(GLOW_VERTEX)
}

pub fn puffs_vertex() -> CString {
    vertex(PUFFS_VERTEX)
}

pub fn flat_vertex() -> CString {
    vertex(FLAT_VERTEX)
}

pub fn flat_fragment() -> CString {
    fragment(FLAT_FRAGMENT)
}
