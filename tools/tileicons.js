#!/usr/bin/env node
// Dock tile colours, derived from the icons themselves. Runs on the set.
//
// Home paints each tile as the launch point's iconColor with the icon on top, so
// a tile is only as filled as its iconColor matches the artwork. LG's own apps
// ship both to match; third-party and homebrew apps mostly do not, which shows
// as a frame of the wrong colour around the artwork. This reads every launch
// point's icon and sets iconColor to fit:
//
//   artwork fills its canvas with one edge colour  -> that colour
//   artwork floats on transparency, colour unset   -> a theme colour from the wallpaper
//   anything else (developer chose a colour, or the edge is not one colour) -> untouched
//
// Pure-black pixels in any icon are lifted to #060606: the compositor keys pure
// black as Home's scaffold, so an icon's own black would show the wallpaper.
//
// Every file changed is backed up once under $STATE and restored by `revert`.
// appinfo.json edits only reach Home after sam re-reads them, so `apply` restarts
// sam and homelaunchpoints when, and only when, something changed.
//
// Usage: tileicons.js apply <state-dir> <wallpaper.png> | revert <state-dir>
'use strict';
const fs = require('fs');
const path = require('path');
const zlib = require('zlib');
const { execFileSync } = require('child_process');

const BLACK_LIFT = 6;
const PLACEHOLDER = '#26233a';   // set on homebrew apps by earlier Luma releases

// ---------------------------------------------------------------- png
function readPng(file) {
  const buf = fs.readFileSync(file);
  if (buf.readUInt32BE(0) !== 0x89504e47) throw new Error('not png');
  let pos = 8, w, h, depth, ctype, interlace, plte, trns;
  const idat = [];
  while (pos < buf.length) {
    const len = buf.readUInt32BE(pos), type = buf.toString('ascii', pos + 4, pos + 8);
    const data = buf.subarray(pos + 8, pos + 8 + len);
    if (type === 'IHDR') { w = data.readUInt32BE(0); h = data.readUInt32BE(4); depth = data[8]; ctype = data[9]; interlace = data[12]; }
    else if (type === 'PLTE') plte = data;
    else if (type === 'tRNS') trns = data;
    else if (type === 'IDAT') idat.push(data);
    else if (type === 'IEND') break;
    pos += 12 + len;
  }
  if (depth !== 8 || interlace !== 0) throw new Error('unsupported png');
  const ch = { 0: 1, 2: 3, 3: 1, 4: 2, 6: 4 }[ctype];
  if (!ch) throw new Error('unsupported colour type');
  const raw = zlib.inflateSync(Buffer.concat(idat));
  const stride = w * ch, out = Buffer.alloc(h * stride);
  let prev = Buffer.alloc(stride);
  for (let y = 0; y < h; y++) {
    const f = raw[y * (stride + 1)], line = raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1));
    const cur = out.subarray(y * stride, (y + 1) * stride);
    for (let i = 0; i < stride; i++) {
      const a = i >= ch ? cur[i - ch] : 0, b = prev[i], c = i >= ch ? prev[i - ch] : 0;
      let v = line[i];
      if (f === 1) v += a; else if (f === 2) v += b; else if (f === 3) v += (a + b) >> 1;
      else if (f === 4) { const p = a + b - c, pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c); v += (pa <= pb && pa <= pc) ? a : (pb <= pc ? b : c); }
      cur[i] = v & 255;
    }
    prev = cur;
  }
  // expand to RGBA
  const rgba = Buffer.alloc(w * h * 4);
  for (let i = 0, o = 0; i < w * h; i++, o += 4) {
    const s = i * ch;
    if (ctype === 6) { rgba[o] = out[s]; rgba[o + 1] = out[s + 1]; rgba[o + 2] = out[s + 2]; rgba[o + 3] = out[s + 3]; }
    else if (ctype === 2) { rgba[o] = out[s]; rgba[o + 1] = out[s + 1]; rgba[o + 2] = out[s + 2]; rgba[o + 3] = 255; }
    else if (ctype === 0) { rgba[o] = rgba[o + 1] = rgba[o + 2] = out[s]; rgba[o + 3] = 255; }
    else if (ctype === 4) { rgba[o] = rgba[o + 1] = rgba[o + 2] = out[s]; rgba[o + 3] = out[s + 1]; }
    else { const p = out[s] * 3; rgba[o] = plte[p]; rgba[o + 1] = plte[p + 1]; rgba[o + 2] = plte[p + 2]; rgba[o + 3] = trns && out[s] < trns.length ? trns[out[s]] : 255; }
  }
  return { w, h, rgba };
}

const CRC = (() => { const t = new Int32Array(256); for (let n = 0; n < 256; n++) { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; t[n] = c; } return t; })();
function crc32(buf) { let c = -1; for (let i = 0; i < buf.length; i++) c = CRC[(c ^ buf[i]) & 255] ^ (c >>> 8); return (c ^ -1) >>> 0; }
function chunk(type, data) {
  const out = Buffer.alloc(12 + data.length);
  out.writeUInt32BE(data.length, 0); out.write(type, 4, 'ascii'); data.copy(out, 8);
  out.writeUInt32BE(crc32(out.subarray(4, 8 + data.length)), 8 + data.length);
  return out;
}
function writePng(file, img) {
  const stride = img.w * 4, raw = Buffer.alloc(img.h * (stride + 1));
  for (let y = 0; y < img.h; y++) { raw[y * (stride + 1)] = 0; img.rgba.copy(raw, y * (stride + 1) + 1, y * stride, (y + 1) * stride); }
  const ihdr = Buffer.alloc(13); ihdr.writeUInt32BE(img.w, 0); ihdr.writeUInt32BE(img.h, 4); ihdr[8] = 8; ihdr[9] = 6;
  fs.writeFileSync(file, Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), chunk('IHDR', ihdr), chunk('IDAT', zlib.deflateSync(raw, { level: 9 })), chunk('IEND', Buffer.alloc(0))]));
}

// ---------------------------------------------------------------- analysis
function liftBlack(img) {
  let n = 0;
  for (let o = 0; o < img.rgba.length; o += 4)
    if (img.rgba[o + 3] > 0 && img.rgba[o] < BLACK_LIFT && img.rgba[o + 1] < BLACK_LIFT && img.rgba[o + 2] < BLACK_LIFT) { img.rgba[o] = img.rgba[o + 1] = img.rgba[o + 2] = BLACK_LIFT; n++; }
  return n;
}
function px(img, x, y) { const o = (y * img.w + x) * 4; return [img.rgba[o], img.rgba[o + 1], img.rgba[o + 2], img.rgba[o + 3]]; }
// First opaque pixel walking inward along a line; null when the line is transparent throughout.
function firstOpaque(img, pts) { for (const [x, y] of pts) { const p = px(img, x, y); if (p[3] > 200) return { x, y, c: p }; } return null; }
function classify(img) {
  const { w, h } = img, cx = w >> 1, cy = h >> 1;
  const range = (n, rev) => { const a = []; for (let i = 0; i < n; i++) a.push(rev ? n - 1 - i : i); return a; };
  const L = firstOpaque(img, range(w).map(x => [x, cy])), R = firstOpaque(img, range(w, 1).map(x => [x, cy]));
  const T = firstOpaque(img, range(h).map(y => [cx, y])), B = firstOpaque(img, range(h, 1).map(y => [cx, y]));
  const margins = [L ? L.x : w, R ? w - 1 - R.x : w, T ? T.y : h, B ? h - 1 - B.y : h];
  const samples = [];
  for (let t = 10; t < 90; t += 4) {
    const fx = Math.floor(w * t / 100), fy = Math.floor(h * t / 100);
    for (const line of [range(w).map(x => [x, fy]), range(w, 1).map(x => [x, fy]), range(h).map(y => [fx, y]), range(h, 1).map(y => [fx, y])]) {
      const p = firstOpaque(img, line); if (p) samples.push(p.c);
    }
  }
  if (!samples.length) return { kind: 'empty', margins };
  const med = [0, 1, 2].map(i => { const v = samples.map(c => c[i]).sort((a, b) => a - b); return v[v.length >> 1]; });
  const agree = samples.filter(c => Math.max(Math.abs(c[0] - med[0]), Math.abs(c[1] - med[1]), Math.abs(c[2] - med[2])) <= 12).length / samples.length;
  const fills = Math.max.apply(null, margins) <= 1;
  const kind = agree >= 0.9 ? (fills ? 'full-uniform-edge' : 'padded') : (fills ? 'full-varied-edge' : 'floating');
  let lum = 0, n = 0;
  for (let o = 0; o < img.rgba.length; o += 4) if (img.rgba[o + 3] > 200) { lum += 0.299 * img.rgba[o] + 0.587 * img.rgba[o + 1] + 0.114 * img.rgba[o + 2]; n++; }
  return { kind, edge: med, margins, lum: n ? lum / n / 255 : 0 };
}
const hex = c => '#' + c.map(v => Math.max(BLACK_LIFT, Math.round(v)).toString(16).padStart(2, '0')).join('');

// Theme colours from the wallpaper: its mean hue, at a dark and a light tone.
function themeColours(wallFile) {
  const img = readPng(wallFile);
  let sx = 0, sy = 0;
  for (let y = 0; y < img.h; y += 8) for (let x = 0; x < img.w; x += 8) {
    const [r, g, b] = px(img, x, y).map(v => v / 255);
    const mx = Math.max(r, g, b), mn = Math.min(r, g, b), s = mx ? (mx - mn) / mx : 0;
    if (s < 0.15 || mx < 0.25 || mx > 0.85) continue;
    let hh; const d = mx - mn;
    if (mx === r) hh = ((g - b) / d) % 6; else if (mx === g) hh = (b - r) / d + 2; else hh = (r - g) / d + 4;
    const a = hh / 6 * 2 * Math.PI; sx += Math.cos(a); sy += Math.sin(a);
  }
  const hue = ((Math.atan2(sy, sx) / (2 * Math.PI)) + 1) % 1;
  const hsv = (h, s, v) => { const i = Math.floor(h * 6), f = h * 6 - i, p = v * (1 - s), q = v * (1 - f * s), t = v * (1 - (1 - f) * s);
    const c = [[v, t, p], [q, v, p], [p, v, t], [p, q, v], [t, p, v], [v, p, q]][i % 6]; return c.map(x => x * 255); };
  return { dark: hex(hsv(hue, 0.40, 0.46)), light: hex(hsv(hue, 0.22, 0.88)) };
}

// ---------------------------------------------------------------- launch points
function luna(uri, params) {
  const out = execFileSync('luna-send', ['-n', '1', '-f', uri, JSON.stringify(params)], { encoding: 'utf8', timeout: 20000 });
  return JSON.parse(out.slice(out.indexOf('{'), out.lastIndexOf('}') + 1));
}
function writable(p) { return p.startsWith('/media/developer/') || p.startsWith('/media/cryptofs/'); }
function appRoot(iconPath) { let d = path.dirname(iconPath); while (d !== '/' && !fs.existsSync(path.join(d, 'appinfo.json'))) d = path.dirname(d); return d === '/' ? null : d; }

function backupOnce(state, key, file) {
  const dir = path.join(state, 'tile-backup'), bk = path.join(dir, key);
  fs.mkdirSync(dir, { recursive: true });
  if (!fs.existsSync(bk)) { fs.copyFileSync(file, bk); fs.writeFileSync(bk + '.path', file); }
}

function apply(state, wallFile) {
  const theme = themeColours(wallFile);
  const lps = luna('luna://com.webos.applicationManager/listLaunchPoints', {}).launchPoints || [];
  const seen = new Set(); let changed = 0; const log = [];
  for (const lp of lps) {
    if (seen.has(lp.id)) continue; seen.add(lp.id);
    // 1. lift pure black in every icon Home might draw
    for (const key of ['icon', 'largeIcon', 'mediumLargeIcon', 'extraLargeIcon']) {
      const f = lp[key];
      if (!f || !f.startsWith('/') || !writable(f) || !fs.existsSync(f)) continue;
      let img; try { img = readPng(f); } catch (e) { continue; }
      if (liftBlack(img) === 0) continue;
      backupOnce(state, lp.id + '.' + key + '.png', f);
      writePng(f, img); changed++; log.push(lp.id + ': lifted black in ' + key);
    }
    // 2. iconColor from the large icon
    const big = lp.largeIcon || lp.icon;
    if (!big || !writable(big) || !fs.existsSync(big)) continue;
    const root = appRoot(big); if (!root) continue;
    let img; try { img = readPng(big); } catch (e) { continue; }
    const cls = classify(img), cur = (lp.iconColor || '').toLowerCase();
    let want = null;
    if (cls.kind === 'full-uniform-edge') { const c = hex(cls.edge); if (c !== cur && (cur === '' || cur === PLACEHOLDER || cur === '#000000')) want = c; }
    else if (cls.kind === 'floating' && (cur === '' || cur === PLACEHOLDER)) want = cls.lum < 0.35 ? theme.light : theme.dark;
    if (!want) continue;
    const ai = path.join(root, 'appinfo.json'), js = JSON.parse(fs.readFileSync(ai, 'utf8'));
    if ((js.iconColor || '').toLowerCase() === want) continue;
    backupOnce(state, lp.id + '.appinfo.json', ai);
    js.iconColor = want; fs.writeFileSync(ai, JSON.stringify(js, null, 2) + '\n'); changed++;
    log.push(lp.id + ': iconColor ' + (cur || '(none)') + ' -> ' + want + ' (' + cls.kind + ')');
  }
  console.log(JSON.stringify({ ok: true, changed, theme, log }));
}

function revert(state) {
  const dir = path.join(state, 'tile-backup'); let restored = 0;
  if (fs.existsSync(dir)) for (const f of fs.readdirSync(dir)) {
    if (f.endsWith('.path')) continue;
    const dest = fs.readFileSync(path.join(dir, f + '.path'), 'utf8');
    try { fs.copyFileSync(path.join(dir, f), dest); restored++; } catch (e) { }
    fs.unlinkSync(path.join(dir, f)); fs.unlinkSync(path.join(dir, f + '.path'));
  }
  console.log(JSON.stringify({ ok: true, restored }));
}

const [verb, state, wall] = process.argv.slice(2);
if (verb === 'apply' && state && wall) apply(state, wall);
else if (verb === 'revert' && state) revert(state);
else { console.error('usage: tileicons.js apply <state> <wallpaper.png> | revert <state>'); process.exit(2); }
