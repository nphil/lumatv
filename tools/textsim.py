#!/usr/bin/env python3
"""Host simulation of LumaTV's text on the TV's path, for `docs/research/text-render-result.md`.

The page is 1920x1080; the display chip's OSD scaler takes it to the panel by 2.01 (3860x2170 of which 3840x2160 shows) with a filter
nobody has measured (bilinear is the assumption, a cubic the bracket), and an optional sharpener (the OSD's `shp`, gain 0x10 on the set)
works on the result. This module draws a line of text the way each candidate would, runs it through a model of that path and measures
the edges. Nothing here has run on the TV: the filter and the sharpener are models (see MODEL below) and the doc says so.

    (cd rustui && cargo build -p lumatv-ui --example textcov)     # the coverage canvases
    python3 tools/textsim.py            # the whole study, writes docs/research/text-render/study.json (needs numpy, Pillow, freetype-py)

Coverage canvases come from `rustui/ui/examples/textcov.rs` (the UI's own rasteriser, fontdue, the same Manrope faces), the hinted ones
from FreeType's autohinter (freetype-py).
"""
import functools
import json
import os
import subprocess
import sys
import tempfile

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, '..'))
TEXTCOV = os.environ.get('TEXTCOV') or os.path.join(os.environ.get('CARGO_TARGET_DIR', os.path.join(ROOT, 'rustui', 'target')), 'debug', 'examples', 'textcov')
FONTS = os.environ.get('MANROPE_DIR', '/data/home/LumaShow/lumashow/assets/fonts')
WEIGHT_FILES = ['Manrope-Regular.ttf', 'Manrope-Medium.ttf', 'Manrope-SemiBold.ttf']
S = 2.01  # the scaler's factor: 3860/1920 = 2.0104 across, 2170/1080 = 2.0093 down
CACHE = os.path.join(tempfile.gettempdir(), 'textsim-cache')

# MODEL: what is assumed, because the TV never showed it (docs/research/ui-sharpness-result.md section 1)
#  scaler  'bilinear': the hardware default filter at select 0 reads as bilinear-ish; 'catmull': the sharper bracket (a cubic select)
#  sharpener: the OSD's SHP is a luminance transient improver ("reg_lti_2d_gain", gain 0x10 on the set), kernel closed. Two stand-ins:
#     'lti'   steepens an edge by moving a pixel toward the local extreme on its side (no overshoot, stair steps on slopes)
#     'peak'  a linear unsharp mask (overshoot, halos)
#  gain g: strength G = g / 0x80, so Strong 0x80 is 1.0, Medium 0x40 is 0.5 and the set's Normal 0x10 is 0.125.


def _pgm(path):
    with open(path, 'rb') as fh:
        assert fh.readline().strip() == b'P5'
        w, h = map(int, fh.readline().split())
        fh.readline()
        return np.frombuffer(fh.read(), dtype=np.uint8).reshape(h, w).astype(np.float64) / 255.0


@functools.lru_cache(maxsize=2048)
def _coverage(text, px, weight, phx, phy):
    """The coverage canvases of one line: dict with cov1 (the UI's, at the page's pixels), cov3 (3 samples across, rows to the page's), ref
    (native at S x, the 4K UI's picture) and meta."""
    os.makedirs(CACHE, exist_ok=True)
    key = '%s_%s_%s_%.3f_%.3f' % (abs(hash(text)) % 10**9, px, weight, phx, phy)
    base = os.path.join(CACHE, key)
    if not os.path.exists(base + '.meta.json'):
        subprocess.run([TEXTCOV, base, text, str(px), str(weight), str(S), str(phx), str(phy)], check=True)
    meta = json.load(open(base + '.meta.json'))
    return {'cov1': _pgm(base + '.cov1.pgm'), 'cov3': _pgm(base + '.cov3.pgm'), 'ref': _pgm(base + '.ref.pgm'), 'meta': meta}


def coverage(text, px, weight, phx=0.0, phy=0.0):
    return _coverage(text, px, weight, float(phx), float(phy))


# ---------------------------------------------------------------------------------------------- the scaler

@functools.lru_cache(maxsize=4096)
def _taps(n_in, n_out, phase, kernel):
    """Weights (n_out x n_in) of one axis: output pixel j covers [(j - phase)/S, (j + 1 - phase)/S) of the page; the sample position in
    pixel-centre indices is t = (j + 0.5 - phase)/S - 0.5."""
    w = np.zeros((n_out, n_in))
    t = (np.arange(n_out) + 0.5 - phase) / S - 0.5
    i0 = np.floor(t).astype(int)
    if kernel == 'bilinear':
        taps = [(i0, 1 - (t - i0)), (i0 + 1, t - i0)]
    elif kernel == 'catmull':
        taps = [(i0 + d, _cr_v(t - (i0 + d))) for d in (-1, 0, 1, 2)]
    elif kernel == 'nearest':
        taps = [(np.floor(t + 0.5).astype(int), np.ones(n_out))]
    else:
        raise ValueError(kernel)
    rows = np.arange(n_out)
    for idx, wt in taps:
        np.add.at(w, (rows, np.clip(idx, 0, n_in - 1)), wt)
    return w


def _cr_v(x, a=-0.5):
    x = np.abs(x)
    return np.where(x < 1, (a + 2) * x ** 3 - (a + 3) * x ** 2 + 1, np.where(x < 2, a * x ** 3 - 5 * a * x ** 2 + 8 * a * x - 4 * a, 0.0))


def scale(img, kernel='bilinear', phx=0.0, phy=0.0, out_shape=None):
    """The OSD scaler on an (H, W) or (H, W, C) image, encoded values (the chip scales what it is given, no gamma)."""
    h, w = img.shape[:2]
    oh, ow = out_shape if out_shape else (int(np.floor(h * S)), int(np.floor(w * S)))
    wy, wx = _taps(h, oh, float(phy), kernel), _taps(w, ow, float(phx), kernel)
    if img.ndim == 2:
        return wy @ img @ wx.T
    return np.stack([wy @ img[..., c] @ wx.T for c in range(img.shape[2])], axis=-1)


# ---------------------------------------------------------------------------------------------- the sharpener

LUMA = np.array([0.2126, 0.7152, 0.0722])


def sharpen(rgb, gain, model='lti'):
    """The OSD sharpener's stand-in on a panel picture (H, W, 3 or H, W), luma only, the change added to every channel."""
    if gain <= 0:
        return rgb
    G = gain / 128.0
    y = rgb @ LUMA if rgb.ndim == 3 else rgb
    if model == 'peak':
        p = np.pad(y, 1, mode='edge')
        lap = 4 * y - p[:-2, 1:-1] - p[2:, 1:-1] - p[1:-1, :-2] - p[1:-1, 2:]
        out = y + G * 0.5 * lap
    elif model == 'lti':
        out = y
        for axis in (1, 0):
            out = _lti_axis(out, G, axis)
    else:
        raise ValueError(model)
    d = out - y
    return rgb + (d[..., None] if rgb.ndim == 3 else d)


def _lti_axis(y, G, axis):
    p = np.pad(y, [(1, 1) if a == axis else (0, 0) for a in (0, 1)], mode='edge')
    sl = lambda o: p[tuple(slice(1 + o, 1 + o + y.shape[a]) if a == axis else slice(None) for a in (0, 1))]
    a, c, b = sl(-1), y, sl(1)
    grad = (b - a) / 2
    lap = a - 2 * c + b
    target = np.where(lap > 0, np.minimum(np.minimum(a, b), c), np.maximum(np.maximum(a, b), c))
    wgt = np.clip(G * np.minimum(1.0, np.abs(grad) / 0.1), 0, 1)
    return c + wgt * (target - c)


# ---------------------------------------------------------------------------------------------- coverage transforms (the candidates)

def curve(a, k=1.0, p=1.0):
    """Contrast about one half, after a power: p < 1 bolder (the stem darkening the glyph shader can do), k > 1 a steeper antialiasing ramp
    that keeps the edge where it was (the half point stays)."""
    a = np.clip(a, 0, 1) ** p
    return np.clip(0.5 + k * (a - 0.5), 0, 1)


def _sobel(a):
    p = np.pad(a, 1, mode='constant')
    gx = (p[:-2, 2:] + 2 * p[1:-1, 2:] + p[2:, 2:] - p[:-2, :-2] - 2 * p[1:-1, :-2] - p[2:, :-2]) / 8
    gy = (p[2:, :-2] + 2 * p[2:, 1:-1] + p[2:, 2:] - p[:-2, :-2] - 2 * p[:-2, 1:-1] - p[:-2, 2:]) / 8
    return gx, gy


def _smooth(x, lo, hi):
    t = np.clip((x - lo) / (hi - lo), 0, 1)
    return t * t * (3 - 2 * t)


def aligned(a, lo=0.80, hi=0.97):
    """How axis-aligned the edge through each pixel is, 0 to 1: the share of the coverage gradient that is across a vertical edge (x) or a
    horizontal one (y), by smoothstep. A stem's edge is 1; a diagonal or the shoulder of a curve less; flat ink or ground (no gradient) 0."""
    gx, gy = _sobel(a)
    m = gx * gx + gy * gy
    c = gx * gx / np.maximum(m, 1e-9)
    w = np.maximum(_smooth(c, lo, hi), _smooth(1 - c, lo, hi))
    return np.where(m > 1e-6, w, 0.0)


def gated(a, k=2.0, p=1.0, lo=0.80, hi=0.97):
    """`curve` only where the edge is axis-aligned: stems and bars are steepened (edges pulled toward whole pixels, which is what hinting does),
    diagonals and curves keep their antialiasing, which is what keeps them from stepping."""
    return a + aligned(a, lo, hi) * (curve(a, k, p) - a)


def usm(a, alpha=0.5):
    """An unsharp mask on the coverage, the text-only pre-sharpening: a five-tap Laplacian across the glyph, clipped to what a coverage can
    be. The glyph shader can read the texels around, never write a coverage beyond 0 or 1."""
    p = np.pad(a, 1, mode='constant')
    lap = 4 * a - p[:-2, 1:-1] - p[2:, 1:-1] - p[1:-1, :-2] - p[1:-1, 2:]
    return np.clip(a + alpha * lap / 2, 0, 1)


def lcd_channels(cov3, kernel=(1 / 3, 1 / 3, 1 / 3), off=(-1.0, 0.0, 1.0)):
    """Per-channel coverage at the page's pixels from three samples a pixel (`cov3`, the pixel's own samples at 3x, 3x+1, 3x+2): the
    samples filtered along the row by `kernel` (FreeType's default LCD filter is FIR5 [8, 77, 86, 77, 8]/256, its light one a three-sample
    box), the R, G and B channels read at the pixel's centre sample plus `off[c]` samples (a third of a page pixel each; fractions
    interpolate). (-1, 0, 1) is a stripe laid on the page's pixels; (-1/2, 0, 1/2) the same stripe as the panel's pixels are half the page's."""
    h, w3 = cov3.shape
    w = w3 // 3
    k = np.asarray(kernel, dtype=float)
    half = len(k) // 2
    pad = np.pad(cov3, ((0, 0), (half, half)))
    f = sum(k[i] * pad[:, i:i + w3] for i in range(len(k)))
    out = []
    for c in range(3):
        pos = np.arange(w) * 3 + 1 + off[c]
        i0 = np.floor(pos).astype(int)
        fr = pos - i0
        i0c, i1c = np.clip(i0, 0, w3 - 1), np.clip(i0 + 1, 0, w3 - 1)
        out.append(f[:, i0c] * (1 - fr) + f[:, i1c] * fr)
    return np.stack(out, axis=-1)


FIR5 = np.array([0x08, 0x4D, 0x56, 0x4D, 0x08]) / 256.0


# ---------------------------------------------------------------------------------------------- compositing

def hexrgb(s):
    s = s.lstrip('#')
    return np.array([int(s[i:i + 2], 16) for i in (0, 2, 4)]) / 255.0


DARK = (hexrgb('#F2EFF6'), hexrgb('#1F1D2E'))   # the dark theme's ink on its sheet (payload/theme/theme.json)
LIGHT = (hexrgb('#17131F'), hexrgb('#FAF4ED'))  # the light theme's


def page(cov, fg, bg):
    """Source over in encoded values, which is what the GL blend does (premultiplied, no gamma): cov is (H, W) or (H, W, 3)."""
    a = cov if cov.ndim == 3 else cov[..., None]
    return bg * (1 - a) + fg * a


def luma(rgb):
    return rgb @ LUMA


# ---------------------------------------------------------------------------------------------- measures

def rise(profile, lo=0.1, hi=0.9, normal=None):
    """10 to 90 percent rise of the first rising edge of a 1-D profile (0 to 1 plateaus), in samples, by linear interpolation; None when the
    profile never reaches the high mark."""
    p = np.asarray(profile, dtype=float)
    if normal is not None:
        p = (p - normal[0]) / (normal[1] - normal[0])
    if p.max() < hi or p.min() > lo:
        return None
    i_hi = int(np.argmax(p >= hi))
    # last sample below lo before i_hi
    i_lo = i_hi
    while i_lo > 0 and p[i_lo - 1] > lo:
        i_lo -= 1
    i_lo = max(i_lo - 1, 0)
    x_lo = i_lo + (lo - p[i_lo]) / (p[i_lo + 1] - p[i_lo]) if p[i_lo + 1] != p[i_lo] else i_lo
    x_hi = (i_hi - 1) + (hi - p[i_hi - 1]) / (p[i_hi] - p[i_hi - 1]) if i_hi > 0 and p[i_hi] != p[i_hi - 1] else i_hi
    return float(x_hi - x_lo)


def edge_row(f):
    """A 1-row page edge: the coverage of an ideal vertical edge at x = 20 + f, ink to the right, 41 pixels, exact area."""
    x = np.arange(41)
    return np.clip(x + 1 - (20 + f), 0, 1)


def step_study(transform, kernel='bilinear', gains=(0,), model='lti', fracs=None, phases=None):
    """The edge-spread study on an ideal straight vertical edge: over the edge's place in a page pixel (f) and the scaler's phase,
    the 10-90 rise in panel pixels, the overshoot (beyond 0 and 1, percent of the step), and where the half point lands against where the
    edge is (panel pixels). `transform` maps a coverage row (2-D, one row) to the coverage drawn."""
    fracs = np.arange(0, 1, 0.125) if fracs is None else fracs
    phases = np.arange(0, 1, 0.25) if phases is None else phases
    rows = []
    for gain in gains:
        rs, os_, ps = [], [], []
        for f in fracs:
            for ph in phases:
                cov = transform(edge_row(f)[None, :].repeat(5, 0))
                img = scale(cov, kernel, ph, 0.0, out_shape=(10, 82))
                img = sharpen(img, gain, model)
                prof = img[5]
                r = rise(prof[20:70])
                rs.append(r if r is not None else np.nan)
                os_.append(100 * max(prof.max() - 1, -prof.min(), 0))
                x50 = int(np.argmax(prof >= 0.5))
                x50 = (x50 - 1) + (0.5 - prof[x50 - 1]) / (prof[x50] - prof[x50 - 1])
                ideal = (20 + f) * S + ph  # where the edge is, on the panel's pixel-edge axis (the content sits `ph` right of the grid)
                ps.append(x50 + 0.5 - ideal)
        rows.append({'gain': gain, 'rise_mean': float(np.nanmean(rs)), 'rise_max': float(np.nanmax(rs)), 'rise_min': float(np.nanmin(rs)),
                     'overshoot': float(np.mean(os_)), 'overshoot_max': float(np.max(os_)), 'shift_mean': float(np.mean(ps)), 'shift_abs_max': float(np.max(np.abs(ps)))})
    return rows


# ---------------------------------------------------------------------------------------------- hinted coverage (FreeType)

def hinted(text, px, weight, mode, base):
    """The line's coverage at the page's pixels from FreeType, glyph origins and canvas as `base` (the cov1 canvas): mode 'none' (no
    hinting, a check against fontdue), 'light' (vertical snapping only), 'full' (the autohinter at normal target: edges aligned, stem widths smooth), 'grid' (the autohinter at the mono target, then drawn antialiased: stems and bars snapped to whole pixels both ways)."""
    import freetype as ft
    face = ft.Face(os.path.join(FONTS, WEIGHT_FILES[weight]))
    face.set_pixel_sizes(0, px)
    flags = {'none': ft.FT_LOAD_NO_HINTING, 'light': ft.FT_LOAD_TARGET_LIGHT | ft.FT_LOAD_FORCE_AUTOHINT,
             'full': ft.FT_LOAD_TARGET_NORMAL | ft.FT_LOAD_FORCE_AUTOHINT,
             'grid': ft.FT_LOAD_TARGET_MONO | ft.FT_LOAD_FORCE_AUTOHINT}[mode]
    if mode != 'grid':
        flags |= ft.FT_LOAD_RENDER
    meta = base['meta']
    h, w = base['cov1'].shape
    out = np.zeros((h, w))
    for c, origin in zip(text, meta['origins']):
        face.load_char(c, flags)
        g = face.glyph
        if mode == 'grid':
            # the mono hinter snaps stems and bars to whole pixels, then the outline is drawn antialiased: curves keep their grey edges
            g.render(ft.FT_RENDER_MODE_NORMAL)
        bm = g.bitmap
        if bm.width == 0 or bm.rows == 0:
            continue
        a = np.array(bm.buffer, dtype=np.uint8).reshape(bm.rows, bm.pitch)[:, :bm.width] / 255.0
        x0, y0 = origin + g.bitmap_left, meta['baseline'] - g.bitmap_top
        ys, xs = max(y0, 0), max(x0, 0)
        ye, xe = min(y0 + bm.rows, h), min(x0 + bm.width, w)
        out[ys:ye, xs:xe] = np.minimum(out[ys:ye, xs:xe] + a[ys - y0:ye - y0, xs - x0:xe - x0], 1)
    return out


# ---------------------------------------------------------------------------------------------- variants and the pipeline

LINES = [
    ('Hamburger fonts illuminate Settings', 26, 1),
    ('Living room lights 21\u00b0 \u00b7 Playing: The quick brown fox 0123456789', 24, 1),
    ('Sharper text up close, without jagged edges', 28, 1),
    ('Kitchen ceiling light  Bedroom lamp', 21, 0),
    ('Small print for the caption row, 19 px', 19, 0),
    ('Settings and Picture', 40, 2),
]
PHASES = [(0.0, 0.0), (0.5, 0.25), (0.25, 0.75), (0.75, 0.5)]


def variant(name, **kw):
    kw['name'] = name
    return kw


# src: 'fd' fontdue (the UI's), 'ft-light', 'ft-full', 'lcd'; weight_up: the next heavier face; curve (k, p); usm alpha; lcd (kernel, offsets)
VARIANTS = [
    variant('a  current (fontdue grey AA)', src='fd'),
    variant('b1 subpixel, FreeType FIR5 stripe', src='lcd', lcd=(FIR5, (-1, 0, 1))),
    variant('b2 subpixel, box3 stripe', src='lcd', lcd=((1 / 3,) * 3, (-1, 0, 1))),
    variant('b3 subpixel, box3 at panel scale', src='lcd', lcd=((1 / 3,) * 3, (-0.5, 0, 0.5))),
    variant('c1 contrast k1.25', src='fd', curve=(1.25, 1.0)),
    variant('c2 contrast k1.5', src='fd', curve=(1.5, 1.0)),
    variant('c3 contrast k2', src='fd', curve=(2.0, 1.0)),
    variant('c4 contrast k3', src='fd', curve=(3.0, 1.0)),
    variant('c5 stem darkening p0.8', src='fd', curve=(1.0, 0.8)),
    variant('c6 contrast k1.5, axis-aligned edges only', src='fd', gated=(1.5, 1.0)),
    variant('d1 unsharp on coverage .25', src='fd', usm=0.25),
    variant('d2 unsharp on coverage .5', src='fd', usm=0.5),
    variant('d3 unsharp on coverage 1.0', src='fd', usm=1.0),
    variant('d4 stem snap in the shader', src='fd', snap=True),
    variant('e1 SemiBold for Medium', src='fd', weight_up=True),
    variant('e2 FreeType, no hinting (check)', src='ft-none'),
    variant('e3 autohint, light (vertical only)', src='ft-light'),
    variant('e4 autohint, normal target', src='ft-full'),
    variant('e5 autohint, grid fit (mono target, grey AA)', src='ft-grid'),
    variant('f1 grid fit + contrast k1.25', src='ft-grid', curve=(1.25, 1.0)),
    variant('f2 grid fit + stem darkening p0.9', src='ft-grid', curve=(1.0, 0.9)),
]


def page_coverage(v, text, px, weight, phx=0.0, phy=0.0):
    """Coverage the variant's text path draws (H, W) or (H, W, 3) and the native reference of its face, at the panel's pixels."""
    weight = min(weight + 1, 2) if v.get('weight_up') else weight
    base = coverage(text, px, weight, phx, phy)
    src = v['src']
    if src == 'fd':
        cov = base['cov1']
    elif src.startswith('ft-'):
        cov = hinted(text, px, weight, src[3:], base)
    elif src == 'lcd':
        kernel, off = v['lcd']
        cov = lcd_channels(base['cov3'], kernel, off)
    else:
        raise ValueError(src)
    if v.get('curve'):
        cov = curve(cov, *v['curve'])
    if v.get('gated'):
        cov = gated(cov, *v['gated'])
    if v.get('snap'):
        cov = snapped(cov, **(v['snap'] if isinstance(v['snap'], dict) else {}))
    if v.get('usm'):
        cov = usm(cov, v['usm']) if cov.ndim == 2 else np.stack([usm(cov[..., c], v['usm']) for c in range(3)], -1)
    return cov, base['ref']


def panel_picture(cov, theme, kernel='bilinear', phx=0.0, phy=0.0, gain=0, model='lti', out_shape=None):
    fg, bg = theme
    img = page(cov, fg, bg)
    out = scale(img, kernel, phx, phy, out_shape)
    return np.clip(sharpen(out, gain, model), 0, 1)


def norm_luma(rgb, theme):
    """Luma scaled so the background is 0 and the ink 1."""
    fg, bg = theme
    lf, lb = luma(fg), luma(bg)
    return (luma(rgb) - lb) / (lf - lb)


def grad_energy(n):
    gy, gx = np.gradient(n)
    return float((gx ** 2 + gy ** 2).sum())


def text_metrics(v, theme, kernel='bilinear', gain=0, model='lti', lines=None):
    """Means over lines and the four phases: RMS error against the native-4K reference of the same face (normalised luma), the gradient
    energy against the reference's (1 is as sharp as a native 4K UI; below soft; above over-sharp), the overshoot beyond the ink and the
    ground (percent of the contrast), and the ink against the reference's (a weight change)."""
    rm, ge, ov, ink = [], [], [], []
    for text, px, weight in (lines or LINES):
        for phx, phy in PHASES:
            cov, ref = page_coverage(v, text, px, weight, phx, phy)
            ref_h, ref_w = ref.shape
            out = panel_picture(cov, theme, kernel, phx, phy, gain, model, out_shape=(ref_h, ref_w))
            n = norm_luma(out, theme)
            r = norm_luma(page(ref, *theme), theme)
            m = 12  # the pad
            n, r = n[m:-m, m:-m], r[m:-m, m:-m]
            rm.append(float(np.sqrt(((n - r) ** 2).mean())))
            ge.append(grad_energy(n) / grad_energy(r))
            ov.append(100 * max(n.max() - 1, -n.min(), 0))
            ink.append(float(n.sum() / r.sum()))
    return {'rmse': float(np.mean(rm)), 'sharp': float(np.mean(ge)), 'over': float(np.mean(ov)), 'over_max': float(np.max(ov)), 'ink': float(np.mean(ink))}


def _peaks(p, thr=0.5):
    """Spans of a 1-D profile above `thr`: [(start, end)]."""
    above = p >= thr
    d = np.diff(above.astype(int))
    starts, ends = list(np.where(d == 1)[0] + 1), list(np.where(d == -1)[0] + 1)
    if above[0]:
        starts.insert(0, 0)
    if above[-1]:
        ends.append(len(p))
    return list(zip(starts, ends))


def stem_study(v, theme, kernel='bilinear', gain=0, model='lti', sizes=range(20, 31), axis='x'):
    """Rise on real stems: the 10-90 percent rise (panel pixels) of every flank of the stems of 'IlIlIl' (axis 'x') or of the bars of
    'ETETET' (axis 'y'), over sizes and the scaler's phases, with the stem's peak (the highest the profile gets, 1 is the ink) and its width at
    half height. Flanks of a stem that never reaches 90 percent are counted apart."""
    rises, peaks, widths, short = [], [], [], 0
    text = 'IlIlIl' if axis == 'x' else 'ETETET'
    for px in sizes:
        for phx, phy in PHASES:
            cov, ref = page_coverage(v, text, px, 1, phx, phy)
            h, w = ref.shape
            out = panel_picture(cov, theme, kernel, phx, phy, gain, model, out_shape=(h, w))
            n = norm_luma(out, theme)
            base = coverage(text, px, 1, phx, phy)['meta']['baseline']
            if axis == 'x':
                r0, r1 = int((base - 0.62 * px) * S), int((base - 0.25 * px) * S)
                prof = n[r0:r1].mean(axis=0)
            else:
                # the bar of an E (its top arm) lies in the first glyph; read down its stem-free right part
                glyph_w = coverage(text, px, 1, phx, phy)['meta']['origins']
                c0 = int((glyph_w[0] + 0.30 * px) * S)
                c1 = int((glyph_w[0] + 0.42 * px) * S)
                top = base - 0.72 * px
                prof = n[int((top - 0.1 * px) * S):int((top + 0.2 * px) * S), c0:c1].mean(axis=1)
            for a, b in _peaks(prof):
                if a < 3 or b > len(prof) - 3 or b - a > 14:
                    continue
                pk = float(prof[max(a - 2, 0):b + 2].max())
                lo, hi = max(a - 6, 0), min(b + 6, len(prof))
                seg = prof[lo:hi]
                i_pk = int(np.argmax(seg))
                left, right = seg[:i_pk + 1], seg[i_pk:][::-1]
                for side in (left, right):
                    rr = rise(side, normal=(0.0, 1.0))
                    if rr is None:
                        short += 1
                    else:
                        rises.append(rr)
                peaks.append(pk)
                widths.append(float(b - a))
    return {'rise': float(np.mean(rises)) if rises else float('nan'), 'rise_max': float(np.max(rises)) if rises else float('nan'),
            'peak': float(np.mean(peaks)), 'width': float(np.mean(widths)), 'short': short, 'n': len(rises)}


# ---------------------------------------------------------------------------------------------- stair steps

def slanted(theta_deg, n=56, ss=8, scale_=1.0):
    """Coverage of a half plane whose edge is `theta_deg` off the vertical through the middle, ink on the right, at `scale_` pixels per page
    pixel (exact to 1/ss)."""
    m = int(round(n * scale_))
    th = np.radians(theta_deg)
    ys, xs = np.mgrid[0:m * ss, 0:m * ss]
    x = (xs + 0.5) / (ss * scale_) - n / 2
    y = (ys + 0.5) / (ss * scale_) - n / 2
    ink = (x * np.cos(th) - y * np.sin(th)) > 0
    return ink.reshape(m, ss, m, ss).mean(axis=(1, 3))


def edge_positions(n_img, rows):
    """Sub-pixel place of the half-height crossing in each row of a normalised picture, rising to the right."""
    out = []
    for r in rows:
        p = n_img[r]
        i = int(np.argmax(p >= 0.5))
        out.append((i - 1) + (0.5 - p[i - 1]) / (p[i] - p[i - 1]))
    return np.array(out)


def jag_study(transform, theme=DARK, kernel='bilinear', gain=0, model='lti', angles=(4, 8, 15, 30), phases=((0.0, 0.0), (0.5, 0.5), (0.25, 0.75))):
    """Stair steps: the RMS (panel pixels) of the edge's place about its best line, for slanted ideal edges. `transform` maps the page coverage
    drawn to the coverage that is kept; the reference is the same edge drawn natively at the panel's pixels. Returns {angle: (rms, rms_ref)}
    means over the phases."""
    out = {}
    for a in angles:
        page_cov = transform(slanted(a))
        r = []
        for phx, phy in phases:
            img = panel_picture(page_cov, theme, kernel, phx, phy, gain, model, out_shape=(int(56 * S) - 1, int(56 * S) - 1))
            n = norm_luma(img, theme)
            rows = np.arange(24, n.shape[0] - 24)
            pos = edge_positions(n, rows)
            fit = np.polyval(np.polyfit(rows, pos, 1), rows)
            r.append(float(np.sqrt(((pos - fit) ** 2).mean())))
        ref = norm_luma(page(slanted(a, scale_=S), *theme), theme)
        rows = np.arange(24, ref.shape[0] - 24)
        pos = edge_positions(ref, rows)
        fit = np.polyval(np.polyfit(rows, pos, 1), rows)
        out[a] = (float(np.mean(r)), float(np.sqrt(((pos - fit) ** 2).mean())))
    return out


# ---------------------------------------------------------------------------------------------- pictures

def label_font(size=15):
    from PIL import ImageFont
    for path in ('/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf', os.path.join(FONTS, 'Manrope-Medium.ttf')):
        if os.path.exists(path):
            return ImageFont.truetype(path, size)
    return ImageFont.load_default()


def crop_picture(v, theme, text, px, weight, kernel='bilinear', gain=0, model='lti', phase=(0.0, 0.0), box=None):
    cov, ref = page_coverage(v, text, px, weight, *phase)
    h, w = ref.shape
    out = panel_picture(cov, theme, kernel, phase[0], phase[1], gain, model, out_shape=(h, w))
    if box:
        x0, y0, x1, y1 = box
        out = out[y0:y1, x0:x1]
    return out


def sheet(rows, path, zoom=3, box=None, label_w=300, gap=6, header=None):
    """rows: [(label, picture (H, W, 3))] stacked, each shown at `zoom` times with nearest pixels, label on the left."""
    from PIL import Image, ImageDraw
    pics = [Image.fromarray((np.clip(p, 0, 1) * 255 + 0.5).astype(np.uint8)).resize((p.shape[1] * zoom, p.shape[0] * zoom), Image.NEAREST) for _, p in rows]
    w = max(p.width for p in pics) + label_w
    hdr = 28 if header else 0
    h = sum(p.height + gap for p in pics) + hdr
    im = Image.new('RGB', (w, h), (128, 128, 128))
    d = ImageDraw.Draw(im)
    f = label_font(15)
    if header:
        d.text((6, 6), header, fill=(255, 255, 255), font=f)
    y = hdr
    for (label, _), p in zip(rows, pics):
        im.paste(p, (label_w, y))
        d.text((6, y + 4), label, fill=(255, 255, 255), font=f)
        y += p.height + gap
    im.save(path)
    return path


def transform_of(v):
    """The variant's coverage transform alone, for the ideal-edge studies; None for a variant that is not one (a rasteriser change)."""
    if v['src'] != 'fd' or v.get('weight_up'):
        return None

    def f(a):
        if v.get('curve'):
            a = curve(a, *v['curve'])
        if v.get('gated'):
            a = gated(a, *v['gated'])
        if v.get('snap'):
            a = snapped(a, **(v['snap'] if isinstance(v['snap'], dict) else {}))
        if v.get('usm'):
            a = usm(a, v['usm'])
        return a
    return f


def score(v, theme=DARK, kernel='bilinear', gain=0, model='lti', sizes=(22, 24, 26, 28)):
    """One row of the table: stems (x) and bars (y) rise, edge rise on the ideal edge over all phases, error and sharpness against the native
    4K, overshoot, ink, and the stair steps of ideal slanted edges (None where the variant is a rasteriser change)."""
    sx = stem_study(v, theme, kernel, gain, model, sizes, 'x')
    sy = stem_study(v, theme, kernel, gain, model, sizes, 'y')
    tm = text_metrics(v, theme, kernel, gain, model)
    t = transform_of(v)
    jag = None
    if t is not None:
        j = jag_study(t, theme, kernel, gain, model)
        jag = float(np.mean([x[0] for x in j.values()]))
    return {'name': v['name'], 'stem_rise': sx['rise'], 'stem_rise_max': sx['rise_max'], 'bar_rise': sy['rise'], 'bar_rise_max': sy['rise_max'],
            'short': sx['short'], 'rmse': tm['rmse'], 'sharp': tm['sharp'], 'over': tm['over'], 'ink': tm['ink'], 'jag': jag}


def snap_runs(a, axis=1, reach=3, wmax=3.6, thr=0.02, strength=1.0):
    """The shader-side stem snap along one axis: a run of nonzero coverage not wider than `wmax` pixels that ends inside the window of
    `reach` pixels either side is redrawn with its width rounded to a whole number of pixels, centred where it was, so both edges land on
    pixel boundaries. Runs that are longer, or end beyond the window, are left alone (a bowl, a diagonal, a wide shape). `strength` mixes."""
    a = np.asarray(a, dtype=float)
    if axis == 0:
        return snap_runs(a.T, 1, reach, wmax, thr, strength).T
    h, w = a.shape
    pad = np.pad(a, ((0, 0), (reach + 1, reach + 1)))
    out = a.copy()
    nz = pad > thr
    for y in range(h):
        row, nzr = pad[y], nz[y]
        x = 0
        n = len(row)
        while x < n:
            if not nzr[x]:
                x += 1
                continue
            s = x
            while x < n and nzr[x]:
                x += 1
            e = x  # run is [s, e)
            if e - s > int(wmax) + 2:
                continue
            tot = float(row[s:e].sum())
            if tot > wmax or tot < 0.5:
                continue
            xl = s + 1 - row[s]
            xr = (e - 1) + row[e - 1]
            wt = max(1.0, np.floor(tot + 0.5))
            xc = (xl + xr) / 2
            nl = np.floor(xc - wt / 2 + 0.5)
            nr = nl + wt
            for p in range(max(s - 1, 0), min(e + 1, n)):
                cov = np.clip(min(p + 1, nr) - max(p, nl), 0, 1)
                px = p - (reach + 1)
                if 0 <= px < w:
                    out[y, px] = row[p] + strength * (cov - row[p])
    return out


def straight(a, axis, span=2, tol=0.04):
    """1 where the coverage is the same `span` pixels either way along `axis` (the edge there is a straight line along it), else 0."""
    p = np.pad(a, ((span, span), (0, 0)) if axis == 0 else ((0, 0), (span, span)))
    ok = np.ones(a.shape, dtype=bool)
    for d in range(-span, span + 1):
        if d:
            sl = p[span + d:span + d + a.shape[0], :] if axis == 0 else p[:, span + d:span + d + a.shape[1]]
            ok &= np.abs(sl - a) < tol
    return ok.astype(float)


def snapped(a, straight_only=True, **kw):
    """Both axes: x runs where the edge is vertical, y runs where it is horizontal; the rest keeps its antialiasing. With `straight_only` a run is
    snapped only along the length of a straight stem (the same five pixels along it), so the joins of a stem and a bowl keep theirs."""
    sx, sy = snap_runs(a, 1, **kw), snap_runs(a, 0, **kw)
    gx, gy = _sobel(a)
    m = gx * gx + gy * gy
    c = gx * gx / np.maximum(m, 1e-9)
    wx = _smooth(c, 0.6, 0.9)
    wy = _smooth(1 - c, 0.6, 0.9)
    if straight_only:
        wx = wx * straight(a, 0)  # a vertical stem is the same up and down
        wy = wy * straight(a, 1)
    return a + wx * (sx - a) + wy * (sy - a) * (1 - wx)


# ---------------------------------------------------------------------------------------------- rings (curves) and fringes

def ring_study(v, theme=DARK, kernel='bilinear', gain=0, model='lti', sizes=(26, 28, 40), weight=1):
    """Stair steps on real curves: casts rays from the middle of an 'o' and finds where the half-height crosses on the outside, in the panel
    picture and in the native reference; the roughness is the RMS (panel pixels) of the difference between them with the slow part (the first
    four harmonics: a hint that moves the whole shape) taken away."""
    rough, bias = [], []
    for px in sizes:
        for phx, phy in PHASES:
            cov, ref = page_coverage(v, 'oo', px, weight, phx, phy)
            h, w = ref.shape
            out = norm_luma(panel_picture(cov, theme, kernel, phx, phy, gain, model, out_shape=(h, w)), theme)
            r = norm_luma(page(ref, *theme), theme)
            ys, xs = np.nonzero(r > 0.5)
            m = xs < (xs.min() + xs.max()) / 2 + 0.0
            # the first 'o': left half of the ink's extent
            cx, cy = xs[m].mean(), ys[m].mean()
            th = np.linspace(0, 2 * np.pi, 180, endpoint=False)

            def radii(img):
                res = []
                for t in th:
                    rr = np.arange(2, 0.75 * px * S, 0.1)
                    xx, yy = cx + rr * np.cos(t), cy + rr * np.sin(t)
                    ok = (xx >= 0) & (xx < img.shape[1] - 1) & (yy >= 0) & (yy < img.shape[0] - 1)
                    xx, yy, rr = xx[ok], yy[ok], rr[ok]
                    x0, y0 = xx.astype(int), yy.astype(int)
                    fx, fy = xx - x0, yy - y0
                    val = (img[y0, x0] * (1 - fx) * (1 - fy) + img[y0, x0 + 1] * fx * (1 - fy) + img[y0 + 1, x0] * (1 - fx) * fy + img[y0 + 1, x0 + 1] * fx * fy)
                    # the outer edge of the ring: the last crossing from ink down through 0.5, within the first o
                    below = np.where(val < 0.5)[0]
                    inside = np.where(val >= 0.5)[0]
                    if len(inside) == 0:
                        res.append(np.nan)
                        continue
                    last = inside.max()
                    if last + 1 >= len(val):
                        res.append(np.nan)
                        continue
                    res.append(rr[last] + (val[last] - 0.5) / (val[last] - val[last + 1]) * 0.1)
                return np.array(res)
            ro, rr_ = radii(out), radii(r)
            d = ro - rr_
            good = ~np.isnan(d)
            if good.sum() < 120:
                continue
            d = np.where(good, d, np.nanmean(d))
            spec = np.fft.rfft(d)
            spec[:5] = 0
            hp = np.fft.irfft(spec, n=len(d))
            rough.append(float(np.sqrt((hp ** 2).mean())))
            bias.append(float(np.mean(d)))
    return {'rough': float(np.mean(rough)), 'bias': float(np.mean(bias))}


def fringe(v, theme=(np.array([1.0, 1.0, 1.0]), np.array([0.07, 0.07, 0.07])), kernel='bilinear'):
    """Colour fringes of a variant on neutral ink and ground: the largest spread between a pixel's channels (percent of the contrast) and the
    mean spread over the pixels that carry any ink."""
    mx, mean = [], []
    for text, px, weight in LINES[:3]:
        for phx, phy in PHASES[:2]:
            cov, ref = page_coverage(v, text, px, weight, phx, phy)
            h, w = ref.shape
            out = panel_picture(cov, theme, kernel, phx, phy, 0, 'lti', out_shape=(h, w))
            spread = (out.max(axis=-1) - out.min(axis=-1)) / (theme[0][0] - theme[1][0])
            ink = norm_luma(out, theme) > 0.05
            mx.append(float(spread.max()))
            mean.append(float(spread[ink].mean()))
    return {'max': 100 * float(np.mean(mx)), 'mean': 100 * float(np.mean(mean))}


def study(path=None):
    """Runs the whole table and writes it as JSON."""
    out = {'rows': [], 'robust': []}
    for v in VARIANTS:
        r = score(v, gain=16)
        r['ring'] = ring_study(v, gain=16)['rough']
        fr = fringe(v)
        r['fringe_max'], r['fringe_mean'] = fr['max'], fr['mean']
        out['rows'].append(r)
        print(r['name'], {k: (round(x, 3) if isinstance(x, float) else x) for k, x in r.items() if k != 'name'}, flush=True)
    keep = ['a  current (fontdue grey AA)', 'c2 contrast k1.5', 'd2 unsharp on coverage .5', 'e1 SemiBold for Medium', 'e5 autohint, grid fit (mono target, grey AA)']
    for v in VARIANTS:
        if v['name'] not in keep:
            continue
        for kernel, gain, model in [('bilinear', 64, 'lti'), ('bilinear', 128, 'lti'), ('bilinear', 64, 'peak'), ('bilinear', 128, 'peak'), ('catmull', 16, 'lti'), ('catmull', 64, 'lti')]:
            r = score(v, kernel=kernel, gain=gain, model=model)
            r.update(kernel=kernel, gain=gain, model=model, ring=ring_study(v, kernel=kernel, gain=gain, model=model)['rough'])
            out['robust'].append(r)
            print(v['name'], kernel, gain, model, {k: (round(x, 3) if isinstance(x, float) else x) for k, x in r.items() if k not in ('name', 'kernel', 'gain', 'model')}, flush=True)
    if path:
        json.dump(out, open(path, 'w'), indent=1)
    return out


if __name__ == '__main__':
    study(sys.argv[1] if len(sys.argv) > 1 else os.path.join(ROOT, 'docs', 'research', 'text-render', 'study.json'))
