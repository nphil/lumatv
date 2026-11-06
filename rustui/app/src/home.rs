//! LumaHome as a client of the toolkit: the wallpaper on the shared 10-a-second tick, the status bar, the clock, "At home", news and
//! weather cards of glass over the cached blur, and the dock of tiles with the focus rule of `LumaHome.qml` (the highlight at once,
//! the lift and the name after 150 ms of rest, one accent halo under the focused tile).
//!
//! Everything is made once and kept: a key press restyles two layers and moves one halo; the lift is one `Animate` per layer on
//! the compositor's clock. Nothing is drawn per frame by the app except the clock's hands, once a second.

use crate::look::{self, MED, Palette, SEMI};
use crate::raster;
use compositor_protocol::{Ease, Fill, Props, Rect, Rgba, Shape, Wallpaper, WallpaperKind};
use std::io;
use std::path::Path;
use ui::components::{Align, Form, Group, Icon, Item, Label, Mark, Part, Pose, Run, TextLayer, line_height, plate, style};
use ui::geometry::Frame;
use ui::text::Type;
use ui::{GlassLayer, ImageLayer, Ui, WallpaperLayer};

pub const W: f32 = 1920.0;
pub const H: f32 = 1080.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Left,
    Right,
    Up,
    Down,
    Ok,
    Back,
}

// ---- grid geometry (layout.json: safe 64/54, 12 columns, gutter 20, rows 222/238/392, status bar 60 + 20) ----
const SAFE_X: f32 = 64.0;
const SAFE_Y: f32 = 54.0;
const GUTTER: f32 = 20.0;
const COLS: usize = 12;
const ROWS: [f32; 3] = [222.0, 238.0, 392.0];
const STATUS_H: f32 = 60.0;
const STATUS_GAP: f32 = 20.0;

fn cell(col: usize, row: usize, span: usize, rows: usize) -> Frame {
    let area_x = SAFE_X;
    let area_y = SAFE_Y + STATUS_H + STATUS_GAP;
    let area_w = W - 2.0 * SAFE_X;
    let cell_w = (area_w - (COLS as f32 - 1.0) * GUTTER) / COLS as f32;
    let mut y = area_y;
    for r in ROWS.iter().take(row) {
        y += r + GUTTER;
    }
    let mut h = 0.0;
    for (k, r) in ROWS.iter().enumerate().skip(row).take(rows) {
        h += r + if k > row { GUTTER } else { 0.0 };
    }
    Frame::new((area_x + col as f32 * (cell_w + GUTTER)).round(), y.round(), (span as f32 * cell_w + (span as f32 - 1.0) * GUTTER).round(), h.round())
}

// ---- z order ----
const Z_WALL: i32 = 0;
const Z_GLASS: i32 = 10;
const Z_CONTENT: i32 = 12;
const Z_HALO: i32 = 20;
const Z_TILE: i32 = 21;
const Z_TILE_ICON: i32 = 22;
const Z_PILL: i32 = 30;

const PAD_X: f32 = 32.0;
const PAD_Y: f32 = 28.0;

/// What the focus rests on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Status(usize),
    Widget(usize),
    Tile(usize),
}

struct Widget {
    frame: Frame,
    /// Part 0 is the glass: the part that lifts. The content does not move or scale (`Widget.qml`: the chrome grows under the text).
    group: Group,
}

struct Tile {
    frame: Frame,
    plate: Rgba,
    /// Part 0 the plate, part 1 the icon.
    group: Group,
    pill: Group,
}

const TILES: [(&str, Icon, &str); 23] = [
    ("Live TV", Icon::Play, "#2a5bd7"),
    ("Netflix", Icon::Play, "#c0141c"),
    ("YouTube", Icon::Play, "#d62b2b"),
    ("Apple TV", Icon::Play, "#1b1b1f"),
    ("Disney+", Icon::Play, "#14307a"),
    ("Prime Video", Icon::Play, "#0d6fb8"),
    ("Spotify", Icon::Speaker, "#1db954"),
    ("Home Assistant", Icon::HomeAssistant, "#18bcf2"),
    ("Camera", Icon::Camera, "#3d3a4a"),
    ("Calendar", Icon::Calendar, "#e6513f"),
    ("Messages", Icon::Message, "#2fb55c"),
    ("Search", Icon::Search, "#4a4f66"),
    ("Settings", Icon::Sliders, "#5b5f7a"),
    ("Inputs", Icon::Tray, "#3b6f86"),
    ("Apps", Icon::Apps, "#6a4fb8"),
    ("Weather", Icon::PartlyCloudy, "#3a8fd8"),
    ("Clock", Icon::Clock, "#d88c2e"),
    ("Lights", Icon::Bulb, "#e0a63a"),
    ("Climate", Icon::Thermometer, "#37a6a0"),
    ("Door", Icon::Lock, "#8a4a64"),
    ("Intercom", Icon::Mic, "#4b6a3a"),
    ("Downloads", Icon::Download, "#46508f"),
    ("Power", Icon::Power, "#7a2f3a"),
];

pub struct Home {
    pub ui: Ui,
    p: Palette,
    _wall: WallpaperLayer,
    status: Vec<Widget>,
    widgets: Vec<Widget>,
    dock: Widget,
    tiles: Vec<Tile>,
    halo: GlassLayer,
    halo_frame: Frame,
    /// Pictures and layers that only have to stay alive.
    _keep: Vec<Part>,
    hands: ImageLayer,
    face: u32,
    focus: Target,
    /// The focus has rested at this time: the lift and the name come then.
    lift_at: Option<u64>,
    lifted: bool,
    last_second: i64,
    /// A wall clock to show instead of the real one (hours, minutes, seconds): for runs on a clock of their own.
    clock: Option<(i32, i32, i32)>,
    cols: usize,
    /// Per-widget centre, for the nearest-in-direction move.
    pub keys_handled: u64,
}

fn local_time() -> (i32, i32, i32, i32, i32, i32, i32) {
    // SAFETY: time and localtime_r on a stack struct.
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&now, &mut tm);
        (tm.tm_hour, tm.tm_min, tm.tm_sec, tm.tm_wday, tm.tm_mday, tm.tm_mon, tm.tm_year + 1900)
    }
}

const WEEKDAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

fn decode_png(path: &Path) -> io::Result<(u32, u32, Vec<u8>)> {
    let file = std::fs::File::open(path).map_err(|e| io::Error::other(format!("{}: {e}", path.display())))?;
    let mut decoder = png::Decoder::new(io::BufReader::new(file));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().map_err(io::Error::other)?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).map_err(io::Error::other)?;
    let (w, h) = (info.width, info.height);
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf[..info.buffer_size()].to_vec(),
        png::ColorType::Rgb => buf[..info.buffer_size()].chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        other => return Err(io::Error::other(format!("png color type {other:?} not handled"))),
    };
    Ok((w, h, rgba))
}

/// The wallpaper picture with LumaHome's scrim folded in (theme `wallScrim` 0.10 / 0.18 / 0.46 of the dark scrim colour down the
/// screen, which the QML blends in the wallpaper's own shader at no cost).
pub fn wallpaper_pixels(path: &Path, w: u32, h: u32) -> io::Result<Vec<u8>> {
    let (iw, ih, mut rgba) = decode_png(path)?;
    if (iw, ih) != (w, h) {
        // nearest-neighbour fit; the shipped picture is 1920x1080
        let mut out = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let (sx, sy) = (x * iw / w, y * ih / h);
                let s = ((sy * iw + sx) * 4) as usize;
                let d = ((y * w + x) * 4) as usize;
                out[d..d + 4].copy_from_slice(&rgba[s..s + 4]);
            }
        }
        rgba = out;
    }
    let scrim = [0x12u8, 0x0e, 0x1a];
    for y in 0..h {
        let t = y as f32 / h as f32;
        let a = if t < 0.55 { 0.10 + (0.18 - 0.10) * t / 0.55 } else { 0.18 + (0.46 - 0.18) * (t - 0.55) / 0.45 };
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            for c in 0..3 {
                rgba[i + c] = (f32::from(rgba[i + c]) * (1.0 - a) + f32::from(scrim[c]) * a).round() as u8;
            }
            rgba[i + 3] = 255;
        }
    }
    Ok(rgba)
}

impl Home {
    pub fn build(mut ui: Ui, assets: &Path, land: &mut dyn FnMut(&mut Ui) -> io::Result<()>) -> io::Result<Self> {
        let p = look::palette();
        ui.warm(&[look::ty(24, SEMI), look::ty(26, MED), look::ty(20, MED), look::ty(23, MED), look::ty(22, MED), look::ty(25, SEMI)]);
        ui.warm_chars(&[
            (look::ty(84, SEMI), "0123456789°-"),
            (look::ty(34, SEMI), "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 ,.:'’-“”%?!"),
            (look::ty(32, SEMI), "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ"),
            (look::ty(14, SEMI), "SMTWF"),
            (look::ty(20, SEMI), "0123456789"),
        ]);
        // The glyphs reach the compositor through two staging slots: until they have landed, a label made from them is a picture
        // for good, so the build waits for them (LumaShow's `atlaswarm`).
        land(&mut ui)?;
        // The marks the cards' text layers carry (discs, icons, a rule) are prepared in the atlas the same way.
        let weather_inner = cell(9, 0, 3, 2).w - 2.0 * PAD_X;
        ui.warm_marks(&[
            Form::Disc(36),
            Form::Disc(8),
            Form::Icon(Icon::Bulb, 22),
            Form::Icon(Icon::Thermometer, 22),
            Form::Icon(Icon::Lock, 22),
            Form::Icon(Icon::Speaker, 22),
            Form::Icon(Icon::PartlyCloudy, 96),
            Form::Icon(Icon::Sun, 26),
            Form::Icon(Icon::Cloud, 26),
            Form::Icon(Icon::Rain, 26),
            Form::Icon(Icon::PartlyCloudy, 26),
            Form::Rule(weather_inner as u32),
        ]);
        land(&mut ui)?;

        // ---- wallpaper: the Silk wallpaper at 10 steps a second, the shared tick ----
        let wall = {
            let mut layer = ui.scene.wallpaper(
                W as u32,
                H as u32,
                Z_WALL,
                Wallpaper {
                    kind: WallpaperKind::Silk,
                    // theme.wallpaperMotion: speed 1, amplitude 4 / depth .25 / detail .35 (the QML's three knobs; the toolkit has one)
                    speed: 1.0,
                    intensity: 0.5,
                    fps: 10,
                },
                Props::at(0.0, 0.0),
            )?;
            let pixels = wallpaper_pixels(&assets.join("wallpaper/wall_1080.png"), W as u32, H as u32)?;
            layer.pixels_mut().ok_or_else(|| io::Error::other("the silk wallpaper has no picture"))?.copy_from_slice(&pixels);
            layer.damage(None);
            layer
        };

        let mut keep: Vec<Part> = Vec::new();
        let mut pics = ui.text_pictures;
        let mut check = |ui: &Ui, what: &str| {
            if ui.text_pictures != pics {
                eprintln!("home: {what} is a picture, not glyph quads");
                pics = ui.text_pictures;
            }
        };

        // ---- status bar ----
        let bar_y = SAFE_Y;
        let mut status = Vec::new();
        let profile = Frame::new(SAFE_X, bar_y, 210.0, STATUS_H);
        let buttons = [Icon::Bell, Icon::Sliders, Icon::Search];
        let mut frames = vec![profile];
        for i in 0..buttons.len() {
            let x = W - SAFE_X - (buttons.len() - i) as f32 * STATUS_H - (buttons.len() - 1 - i) as f32 * 16.0;
            frames.push(Frame::new(x, bar_y, STATUS_H, STATUS_H));
        }
        for (i, frame) in frames.iter().enumerate() {
            status.push(Self::card_group(&ui, &p, *frame, STATUS_H / 2.0, Z_GLASS));
            if i == 0 {
                let mut disc = ui.scene.image(40, 40, Z_CONTENT, Props::at(frame.x + 10.0, frame.y + 10.0))?;
                raster::disc(disc.pixels_mut(), 40, (20.0, 20.0), 20.0, p.accent);
                raster::disc(disc.pixels_mut(), 40, (20.0, 15.0), 6.5, p.ink_on_accent);
                raster::ring(disc.pixels_mut(), 40, (20.0, 40.0), 12.0, 6.0, p.ink_on_accent);
                disc.damage(None);
                keep.push(Part::new(disc, Frame::new(frame.x + 10.0, frame.y + 10.0, 40.0, 40.0)));
                let label = Label::new(
                    &mut ui,
                    Frame::new(frame.x + 62.0, frame.y + (STATUS_H - line_height(look::ty(24, SEMI))) / 2.0, 130.0, line_height(look::ty(24, SEMI))),
                    Z_CONTENT,
                    style(look::ty(24, SEMI), p.ink, Align::Left),
                    "Nitin",
                )?;
                keep.push(Part::new(label, *frame));
            } else {
                let frame_icon = Frame::new(frame.x + 14.0, frame.y + 14.0, 32.0, 32.0);
                keep.push(ui::components::icon_part(&mut ui, buttons[i - 1], frame_icon, Z_CONTENT, p.ink)?);
            }
        }

        check(&ui, "the status bar");
        // ---- the four cards ----
        let clock_frame = cell(0, 0, 4, 1);
        let home_frame = cell(0, 1, 4, 1);
        let news_frame = cell(4, 0, 5, 2);
        let weather_frame = cell(9, 0, 3, 2);
        let widgets: Vec<Widget> =
            [clock_frame, home_frame, news_frame, weather_frame].iter().map(|f| Self::card_group(&ui, &p, *f, look::R_CARD, Z_GLASS)).collect();

        // clock: the face once, the hands once a second, the day and the week strip as one text layer
        let dia = (clock_frame.h - 2.0 * PAD_Y).min(166.0);
        let face_at = Frame::new(clock_frame.x + PAD_X, clock_frame.y + PAD_Y, dia, dia);
        let mut face = ui.scene.image(dia as u32, dia as u32, Z_CONTENT, Props::at(face_at.x, face_at.y))?;
        draw_face(face.pixels_mut(), dia as u32, &p);
        face.damage(None);
        keep.push(Part::new(face, face_at));
        let hands = ui.scene.image(dia as u32, dia as u32, Z_CONTENT + 1, Props::at(face_at.x, face_at.y))?;

        let cal_x = PAD_X + dia + 36.0;
        let cal_w = clock_frame.w - cal_x - PAD_X;
        let (_, _, _, wday, mday, mon, year) = local_time();
        let week_y = clock_frame.h - PAD_Y - 70.0;
        let cell_w = (cal_w / 7.0).floor().min(48.0);
        let date_line = format!("{} {}, {}", MONTHS[mon as usize], mday, year);
        let mut items: Vec<Item> = vec![
            Item::Text(Run::left(WEEKDAYS[wday as usize], look::ty(32, SEMI), p.ink, cal_x, PAD_Y - 6.0, cal_w)),
            Item::Text(Run::left(&date_line, look::ty(22, MED), p.ink2, cal_x, PAD_Y + 40.0, cal_w)),
        ];
        let initials = ["S", "M", "T", "W", "T", "F", "S"];
        let day_numbers: Vec<String> = (0..7).map(|i| format!("{}", ((mday + i - wday).max(1) - 1) % 28 + 1)).collect();
        for i in 0..7usize {
            let today = i as i32 == wday;
            let cx = cal_x + i as f32 * cell_w;
            items.push(Item::Text(Run::centre(initials[i], look::ty(14, SEMI), if today { p.ink } else { p.ink3 }, cx, week_y, cell_w)));
            if today {
                items.push(Item::Mark(Mark::disc((cx + ((cell_w - 36.0) / 2.0).round()) as i32, (week_y + 26.0) as i32, 36, p.ink)));
            }
            items.push(Item::Text(Run::centre(
                &day_numbers[i],
                look::ty(20, SEMI),
                if today { p.ink_on_accent } else { p.ink2 },
                cx,
                week_y + 26.0 + 5.0,
                cell_w,
            )));
        }
        let clock_text = TextLayer::new(&mut ui, clock_frame, Z_CONTENT, &items)?;
        check(&ui, "the clock's text");
        keep.push(Part::new(clock_text, clock_frame));

        // At home: four chips
        {
            let f = home_frame;
            let mut chip_items: Vec<Item> = vec![Item::Text(Run::left("At home", look::ty(24, SEMI), p.ink2, PAD_X, PAD_Y - 4.0, f.w - 2.0 * PAD_X))];
            let chips = [
                ("Living room", "3 lights on", Icon::Bulb, "#38ffc45c"),
                ("72° cooling", "Set to 70°", Icon::Thermometer, "#3378beff"),
                ("Front door", "Locked", Icon::Lock, "#2e78f0aa"),
                ("Kitchen", "Playing · Sonos", Icon::Speaker, "#2eff78c8"),
            ];
            let inner_w = f.w - 2.0 * PAD_X;
            let inner_h = f.h - 2.0 * PAD_Y;
            let chip_top = 49.0;
            let gap = 12.0;
            let chip_w = ((inner_w - gap) / 2.0).floor();
            let chip_h = ((inner_h - chip_top - gap) / 2.0).floor();
            for (i, (title, sub, icon, tint)) in chips.iter().enumerate() {
                let x = PAD_X + (i % 2) as f32 * (chip_w + gap);
                let y = PAD_Y + chip_top + (i / 2) as f32 * (chip_h + gap);
                keep.push(Part::new(
                    ui.scene.shape(
                        chip_w as u32,
                        chip_h as u32,
                        Z_CONTENT,
                        Shape { corner_radius: 18.0, fill: Fill::Solid(look::hex("#12ffffff")), border_width: 1.0, border: look::hex("#14ffffff") },
                        Props::at(f.x + x, f.y + y),
                    ),
                    Frame::new(f.x + x, f.y + y, chip_w, chip_h),
                ));
                let disc_y = y + ((chip_h - 36.0) / 2.0).round();
                keep.push(Part::new(
                    ui.scene.shape(36, 36, Z_CONTENT, plate::solid(look::hex(tint), 18.0), Props::at(f.x + x + 16.0, f.y + disc_y)),
                    Frame::new(f.x + x + 16.0, f.y + disc_y, 36.0, 36.0),
                ));
                chip_items.push(Item::Mark(Mark::icon(*icon, 22, (x + 23.0) as i32, (disc_y + 7.0) as i32, p.ink)));
                let ty_a = look::ty(24, SEMI);
                let ty_b = look::ty(20, MED);
                let text_w = chip_w - 66.0 - 12.0;
                let top = y + ((chip_h - 58.0) / 2.0).round();
                chip_items.push(Item::Text(Run::left(title, ty_a, p.ink, x + 66.0, top - 2.0, text_w)));
                chip_items.push(Item::Text(Run::left(sub, ty_b, p.ink2, x + 66.0, top + 29.0, text_w)));
            }
            let layer = TextLayer::new(&mut ui, f, Z_CONTENT + 1, &chip_items)?;
            check(&ui, "the At home text");
            keep.push(Part::new(layer, f));
        }

        // News: hero picture, headline, meta line, page dots
        {
            let f = news_frame;
            let hero_w = (f.w - 2.0 * PAD_X) as u32;
            let hero_h = 262u32;
            let mut hero = ui.scene.image(hero_w, hero_h, Z_CONTENT, Props::at(f.x + PAD_X, f.y + PAD_Y))?;
            draw_hero(hero.pixels_mut(), hero_w, hero_h);
            hero.damage(None);
            keep.push(Part::new(hero, Frame::new(f.x + PAD_X, f.y + PAD_Y, hero_w as f32, hero_h as f32)));
            let headline_ty = look::ty(34, SEMI);
            let inner_w = f.w - 2.0 * PAD_X;
            let lines =
                ui::components::wrap_to(&mut ui.fonts, "Talks resume as regional leaders meet to set out a path for the winter ahead", headline_ty, inner_w, 2);
            let mut items: Vec<Item> = Vec::new();
            let mut y = PAD_Y + hero_h as f32 + 18.0;
            for line in &lines {
                items.push(Item::Text(Run::left(line, headline_ty, p.ink, PAD_X, y, inner_w)));
                y += (headline_ty.px as f32 * 1.22).round();
            }
            let meta_y = f.h - PAD_Y - line_height(look::ty(22, MED));
            items.push(Item::Text(Run::left("BBC News · 12 min ago", look::ty(22, MED), p.ink2, PAD_X, meta_y, inner_w)));
            for i in 0..5 {
                let colour = if i == 0 { p.ink } else { p.ink3 };
                items.push(Item::Mark(Mark::disc((f.w - PAD_X - 5.0 * 16.0 + i as f32 * 16.0) as i32, (meta_y + 12.0) as i32, 8, colour)));
            }
            let layer = TextLayer::new(&mut ui, f, Z_CONTENT + 1, &items)?;
            check(&ui, "the news text");
            keep.push(Part::new(layer, f));
        }

        // Weather: place, big icon, temperature, condition, stats, rule, five rows
        {
            let f = weather_frame;
            let inner_w = f.w - 2.0 * PAD_X;
            let mut items: Vec<Item> = vec![
                Item::Text(Run::left("Atlanta", look::ty(24, SEMI), p.ink2, PAD_X, PAD_Y - 4.0, inner_w)),
                Item::Mark(Mark::icon(Icon::PartlyCloudy, 96, PAD_X as i32, (PAD_Y + 61.0) as i32, p.ink)),
                Item::Text(Run::left("72°", look::ty(84, SEMI), p.ink, PAD_X + 114.0, PAD_Y + 20.0, inner_w - 114.0)),
                Item::Text(Run::left("Partly cloudy", look::ty(26, MED), p.ink2, PAD_X + 114.0, PAD_Y + 128.0, inner_w - 114.0)),
                Item::Text(Run::left("H 78°  L 61°   Wind 6 mph", look::ty(20, MED), p.ink2, PAD_X, PAD_Y + 172.0, inner_w)),
                Item::Mark(Mark::rule(PAD_X as i32, (PAD_Y + 220.0) as i32, inner_w as u32, look::white(0.10))),
            ];
            let days = [
                ("Tue", Icon::Sun, "79°", "62°"),
                ("Wed", Icon::Cloud, "74°", "60°"),
                ("Thu", Icon::Rain, "68°", "57°"),
                ("Fri", Icon::PartlyCloudy, "71°", "55°"),
                ("Sat", Icon::Sun, "76°", "58°"),
            ];
            let row_ty = look::ty(23, MED);
            for (i, (day, icon, hi, lo)) in days.iter().enumerate() {
                let y = PAD_Y + 225.0 + i as f32 * 41.0;
                let text_y = y + ((41.0 - line_height(row_ty)) / 2.0).round();
                items.push(Item::Text(Run::left(day, row_ty, p.ink, PAD_X, text_y, 60.0)));
                items.push(Item::Mark(Mark::icon(*icon, 26, (PAD_X + 70.0) as i32, (y + 8.0) as i32, p.ink2)));
                items.push(Item::Text(Run::right(hi, row_ty, p.ink, PAD_X + inner_w - 112.0, text_y, 56.0)));
                items.push(Item::Text(Run::right(lo, row_ty, p.ink2, PAD_X + inner_w - 56.0, text_y, 56.0)));
            }
            let layer = TextLayer::new(&mut ui, f, Z_CONTENT, &items)?;
            check(&ui, "the weather text");
            keep.push(Part::new(layer, f));
        }

        // ---- the dock ----
        let dock_frame = cell(0, 2, 12, 1);
        let dock = Self::card_group(&ui, &p, dock_frame, look::R_CARD, Z_GLASS);
        let (tile_size, gap, min_pad) = (120.0f32, 16.0f32, 40.0f32);
        let cols = (((dock_frame.w - 2.0 * min_pad + gap) / (tile_size + gap)).floor() as usize).max(1);
        let pad_x = ((dock_frame.w - (cols as f32 * tile_size + (cols as f32 - 1.0) * gap)) / 2.0).round();
        let rows = TILES.len().div_ceil(cols);
        let grid_top = ((dock_frame.h - (rows as f32 * tile_size + (rows as f32 - 1.0) * gap)) / 2.0).round();
        let halo_frame = Frame::new(0.0, 0.0, tile_size, tile_size);
        let halo = ui.scene.glass(tile_size as u32, tile_size as u32, Z_HALO, look::halo(p.accent), Props { opacity: 0.0, ..Props::at(0.0, 0.0) });
        let mut tiles = Vec::new();
        let name_ty = look::ty(25, SEMI);
        for (i, (title, icon, plate_hex)) in TILES.iter().enumerate() {
            let (row, col) = (i / cols, i % cols);
            let frame = Frame::new(
                dock_frame.x + pad_x + col as f32 * (tile_size + gap),
                dock_frame.y + grid_top + row as f32 * (tile_size + gap),
                tile_size,
                tile_size,
            );
            let plate_colour = look::hex(plate_hex);
            let mut group = Group::new(frame.centre());
            let glass = ui.scene.glass(tile_size as u32, tile_size as u32, Z_TILE, look::tile(plate_colour, false, false), Props::at(frame.x, frame.y));
            group.push(Part::new(glass, frame));
            let icon_frame = Frame::new(frame.x + 28.0, frame.y + 28.0, 64.0, 64.0);
            group.push(ui::components::icon_part(&mut ui, *icon, icon_frame, Z_TILE_ICON, look::white(0.96))?);
            // the name pill, 10 px under the plate (the dock's second row would put it below; the last row's would leave the card)
            let text_w = ui.fonts.measure(title, name_ty).width.ceil();
            let pill_w = text_w + 26.0;
            let pill_h = 43.0;
            let pill_frame = Frame::new(frame.x + ((tile_size - pill_w) / 2.0).round(), frame.y + tile_size + 14.0, pill_w, pill_h);
            let mut pill = Group::new(pill_frame.centre());
            pill.push(Part::new(
                ui.scene.glass(
                    pill_w as u32,
                    pill_h as u32,
                    Z_PILL,
                    look::pill(),
                    Props { opacity: 0.0, visible: false, ..Props::at(pill_frame.x, pill_frame.y) },
                ),
                pill_frame,
            ));
            let label = Label::new(
                &mut ui,
                Frame::new(pill_frame.x + 13.0, pill_frame.y + ((pill_h - line_height(name_ty)) / 2.0).round(), text_w + 2.0, line_height(name_ty)),
                Z_PILL + 1,
                style(name_ty, p.ink, Align::Left),
                title,
            )?;
            let mut label_part = Part::new(
                label,
                Frame::new(pill_frame.x + 13.0, pill_frame.y + ((pill_h - line_height(name_ty)) / 2.0).round(), text_w + 2.0, line_height(name_ty)),
            );
            label_part.rest_opacity = 1.0;
            pill.push(label_part);
            pill.set(Pose { opacity: 0.0, visible: false, ..Pose::REST });
            tiles.push(Tile { frame, plate: plate_colour, group, pill });
        }

        let mut home = Self {
            ui,
            p,
            _wall: wall,
            status,
            widgets,
            dock,
            tiles,
            halo,
            halo_frame,
            _keep: keep,
            hands,
            face: dia as u32,
            focus: Target::Tile(0),
            lift_at: None,
            lifted: false,
            last_second: -1,
            clock: None,
            cols,
            keys_handled: 0,
        };
        home.focus_highlight(home.focus, true);
        home.place_halo(home.focus, false);
        home.lifted = false;
        home.tick_clock(true);
        Ok(home)
    }

    /// A glass card with the group the lift moves.
    fn card_group(ui: &Ui, p: &Palette, frame: Frame, radius: f32, z: i32) -> Widget {
        let mut group = Group::new(frame.centre());
        let glass = ui.scene.glass(frame.w.round() as u32, frame.h.round() as u32, z, look::card(p, radius, false, false), Props::at(frame.x, frame.y));
        group.push(Part::new(glass, frame));
        Widget { frame, group }
    }

    fn glass_of(group: &Group, index: usize) -> &GlassLayer {
        group.parts[index].glass().expect("a glass part")
    }

    // ------------------------------------------------------------------------------------------------------------------------
    // focus

    fn widget_for(&mut self, t: Target) -> Option<&mut Widget> {
        match t {
            Target::Status(i) => self.status.get_mut(i),
            Target::Widget(i) => self.widgets.get_mut(i),
            Target::Tile(_) => None,
        }
    }

    fn radius_of(t: Target) -> f32 {
        match t {
            Target::Status(_) => STATUS_H / 2.0,
            _ => look::R_CARD,
        }
    }

    /// The highlight half of focus (or its removal): at once, on every key. A lift in progress is dropped.
    fn focus_highlight(&mut self, t: Target, on: bool) {
        let p_radius = Self::radius_of(t);
        match t {
            Target::Tile(i) => {
                let tile = &mut self.tiles[i];
                Self::glass_of(&tile.group, 0).restyle(look::tile(tile.plate, on, false));
                if !on {
                    tile.group.set(Pose::REST);
                    tile.pill.set(Pose { opacity: 0.0, visible: false, ..Pose::REST });
                }
            }
            _ => {
                let palette = &self.p;
                let glass = look::card(palette, p_radius, on, false);
                if let Some(w) = self.widget_for(t) {
                    Self::glass_of(&w.group, 0).restyle(glass);
                    if !on {
                        w.group.set(Pose::REST);
                    }
                }
            }
        }
    }

    /// The halo moves under the focused tile at once and fades in; it is off for anything else.
    fn place_halo(&mut self, t: Target, visible: bool) {
        match t {
            Target::Tile(i) if visible => {
                let f = self.tiles[i].frame;
                self.halo.set(Props { opacity: 0.0, ..Props::at(f.x, f.y) });
                self.halo.animate(Props { opacity: look::AMBIENT, ..Props::at(f.x, f.y) }, look::CARD_MS, Ease::Out);
            }
            Target::Tile(i) => {
                let f = self.tiles[i].frame;
                self.halo.set(Props { opacity: 0.0, ..Props::at(f.x, f.y) });
            }
            _ => {
                let f = self.halo_frame;
                self.halo.set(Props { opacity: 0.0, ..Props::at(f.x, f.y) });
            }
        }
    }

    /// The lift half: after the focus has rested.
    fn lift(&mut self) {
        self.lifted = true;
        let t = self.focus;
        match t {
            Target::Tile(i) => {
                let tile = &mut self.tiles[i];
                Self::glass_of(&tile.group, 0).restyle(look::tile(tile.plate, true, true));
                tile.group.animate(Pose { dy: -6.0 * 1.0, scale: look::TILE_SCALE, ..Pose::REST }, look::FOCUS_MS, Ease::Out);
                tile.pill.set(Pose { opacity: 0.0, ..Pose::REST });
                tile.pill.animate(Pose::REST, look::FOCUS_MS, Ease::Out);
            }
            Target::Status(_) | Target::Widget(_) => {
                let (scale, lift) = match t {
                    Target::Status(_) => (look::BUTTON_SCALE, 0.0),
                    _ => (look::CARD_SCALE, look::CARD_LIFT),
                };
                let radius = Self::radius_of(t);
                let glass = look::card(&self.p, radius, true, true);
                if let Some(w) = self.widget_for(t) {
                    Self::glass_of(&w.group, 0).restyle(glass);
                    w.group.animate(Pose { dy: -lift, scale, ..Pose::REST }, look::CARD_MS, Ease::Out);
                }
            }
        }
    }

    fn set_focus(&mut self, to: Target, now_ns: u64) {
        if to == self.focus {
            return;
        }
        let from = self.focus;
        self.focus_highlight(from, false);
        if matches!(from, Target::Tile(_)) && !matches!(to, Target::Tile(_)) {
            self.place_halo(from, false);
        }
        self.focus = to;
        self.focus_highlight(to, true);
        self.place_halo(to, true);
        self.lifted = false;
        self.lift_at = Some(now_ns + look::REST_MS * 1_000_000);
    }

    fn center_of(&self, t: Target) -> (f32, f32) {
        let f = match t {
            Target::Status(i) => self.status[i].frame,
            Target::Widget(i) => self.widgets[i].frame,
            Target::Tile(i) => self.tiles[i].frame,
        };
        f.centre()
    }

    fn nearest_widget(&self, from: usize, dx: f32, dy: f32) -> usize {
        let (fx, fy) = self.center_of(Target::Widget(from));
        let mut best = (from, f32::INFINITY);
        for i in 0..self.widgets.len() {
            if i == from {
                continue;
            }
            let (cx, cy) = self.center_of(Target::Widget(i));
            let (ddx, ddy) = (cx - fx, cy - fy);
            if dx != 0.0 && ddx * dx <= 0.0 {
                continue;
            }
            if dy != 0.0 && ddy * dy <= 0.0 {
                continue;
            }
            let (primary, secondary) = if dx != 0.0 { (ddx.abs(), ddy.abs()) } else { (ddy.abs(), ddx.abs()) };
            let score = primary + secondary * 2.0;
            if score < best.1 {
                best = (i, score);
            }
        }
        best.0
    }

    fn nearest_by_x(&self, x: f32, among: impl Iterator<Item = Target>) -> Option<Target> {
        among.map(|t| ((self.center_of(t).0 - x).abs(), t)).min_by(|a, b| a.0.total_cmp(&b.0)).map(|(_, t)| t)
    }

    /// A key of the remote. `LumaHome.qml`'s focus layers: the status bar over the cards over the dock.
    pub fn key(&mut self, key: Key, now_ns: u64) {
        self.keys_handled += 1;
        let cols = self.cols;
        let rows = TILES.len().div_ceil(cols);
        let next = match (self.focus, key) {
            (Target::Tile(i), Key::Left) => Target::Tile(if i % cols > 0 { i - 1 } else { i }),
            (Target::Tile(i), Key::Right) => Target::Tile(if i % cols + 1 < cols && i + 1 < TILES.len() { i + 1 } else { i }),
            (Target::Tile(i), Key::Down) => Target::Tile(if i / cols + 1 < rows && i + cols < TILES.len() { i + cols } else { i }),
            (Target::Tile(i), Key::Up) => {
                if i / cols > 0 {
                    Target::Tile(i - cols)
                } else {
                    // from the dock into the card whose bottom edge is nearest (lowestWidgetByX)
                    let x = self.tiles[i].frame.centre().0;
                    let dock_top = self.dock.frame.y;
                    let best = (0..self.widgets.len())
                        .map(|w| {
                            let f = self.widgets[w].frame;
                            let score = ((f.x + f.w / 2.0) - x).abs() + (dock_top - GUTTER - (f.y + f.h)).abs() * 2.0;
                            (score, w)
                        })
                        .min_by(|a, b| a.0.total_cmp(&b.0))
                        .map(|(_, w)| w)
                        .unwrap_or(0);
                    Target::Widget(best)
                }
            }
            (Target::Widget(i), Key::Left) => Target::Widget(self.nearest_widget(i, -1.0, 0.0)),
            (Target::Widget(i), Key::Right) => Target::Widget(self.nearest_widget(i, 1.0, 0.0)),
            (Target::Widget(i), Key::Up) => {
                if self.widgets[i].frame.y <= SAFE_Y + STATUS_H + STATUS_GAP + 1.0 {
                    let x = self.widgets[i].frame.centre().0;
                    self.nearest_by_x(x, (0..self.status.len()).map(Target::Status)).unwrap_or(self.focus)
                } else {
                    Target::Widget(self.nearest_widget(i, 0.0, -1.0))
                }
            }
            (Target::Widget(i), Key::Down) => {
                let f = self.widgets[i].frame;
                if (f.y + f.h - (self.dock.frame.y - GUTTER)).abs() < 2.0 {
                    let x = f.centre().0;
                    self.nearest_by_x(x, (0..cols).map(Target::Tile)).unwrap_or(self.focus)
                } else {
                    Target::Widget(self.nearest_widget(i, 0.0, 1.0))
                }
            }
            (Target::Status(i), Key::Left) => Target::Status(i.saturating_sub(1)),
            (Target::Status(i), Key::Right) => Target::Status((i + 1).min(self.status.len() - 1)),
            (Target::Status(i), Key::Down) => {
                let x = self.status[i].frame.centre().0;
                self.nearest_by_x(x, (0..self.widgets.len()).map(Target::Widget)).unwrap_or(self.focus)
            }
            (t, _) => t,
        };
        self.set_focus(next, now_ns);
    }

    // ------------------------------------------------------------------------------------------------------------------------
    // the clock

    /// Shows this time instead of the wall clock's (`None`: the wall clock's again) from the next `tick_clock`.
    pub fn set_clock(&mut self, hms: Option<(i32, i32, i32)>) {
        self.clock = hms;
    }

    /// Draws the hands for the wall clock's second now (`force`) or when it has changed since the last time.
    pub fn tick_clock(&mut self, force: bool) {
        let (h, m, s) = self.clock.unwrap_or_else(|| {
            let now = local_time();
            (now.0, now.1, now.2)
        });
        let stamp = i64::from(h) * 3600 + i64::from(m) * 60 + i64::from(s);
        if !force && stamp == self.last_second {
            return;
        }
        self.last_second = stamp;
        let size = self.face;
        let c = size as f32 / 2.0;
        let pixels = self.hands.pixels_mut();
        pixels.fill(0);
        let point = |angle_deg: f32, length: f32| {
            let a = angle_deg.to_radians();
            (c + a.sin() * length, c - a.cos() * length)
        };
        let hour_angle = ((h % 12) as f32 + m as f32 / 60.0) * 30.0;
        let minute_angle = (m as f32 + s as f32 / 60.0) * 6.0;
        let second_angle = s as f32 * 6.0;
        raster::stroke(pixels, size, point(hour_angle + 180.0, 12.0), point(hour_angle, 44.0), 4.0, self.p.ink);
        raster::stroke(pixels, size, point(minute_angle + 180.0, 12.0), point(minute_angle, 62.0), 3.0, self.p.ink);
        raster::stroke(pixels, size, point(second_angle + 180.0, 18.0), point(second_angle, 66.0), 1.0, self.p.second);
        raster::disc(pixels, size, (c, c), 6.0, self.p.second);
        raster::disc(pixels, size, (c, c), 2.0, self.p.ink_on_accent);
        self.hands.damage(None);
    }

    /// What the app does on its own clock: the second hand, and the lift once focus has rested. Returns when it next wants a
    /// call (monotonic ns), if it does.
    pub fn tick(&mut self, now_ns: u64) -> Option<u64> {
        self.tick_clock(false);
        if let Some(at) = self.lift_at {
            if now_ns >= at {
                self.lift_at = None;
                self.lift();
            }
        }
        // the next second boundary of the wall clock
        let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
        // SAFETY: a plain clock read into a stack struct.
        unsafe { libc::clock_gettime(libc::CLOCK_REALTIME, &mut ts) };
        let to_second = 1_000_000_000u64 - ts.tv_nsec as u64;
        let second = now_ns + to_second + 1_000_000;
        Some(match self.lift_at {
            Some(at) => at.min(second),
            None => second,
        })
    }

    pub fn focus_label(&self) -> String {
        match self.focus {
            Target::Status(i) => format!("status {i}"),
            Target::Widget(i) => format!("widget {i}"),
            Target::Tile(i) => format!("tile {} ({})", i, TILES[i].0),
        }
    }
}

/// The clock's face: a glass disc, four bold markers, eight dots (`ClockWidget.qml`'s canvas).
fn draw_face(pixels: &mut [u8], size: u32, p: &Palette) {
    let r = size as f32 / 2.0;
    let c = (r, r);
    raster::disc(pixels, size, c, r - 1.0, look::white(0.07));
    raster::ring(pixels, size, c, r - 0.5, 1.0, look::white(0.16));
    for i in 0..12 {
        let a = i as f32 * std::f32::consts::PI / 6.0;
        let major = i % 3 == 0;
        if major {
            let a_pt = (r + a.cos() * (r - 12.0), r + a.sin() * (r - 12.0));
            let b_pt = (r + a.cos() * (r - 24.0), r + a.sin() * (r - 24.0));
            raster::stroke(pixels, size, a_pt, b_pt, 2.0, p.ink);
        } else {
            raster::disc(pixels, size, (r + a.cos() * (r - 18.0), r + a.sin() * (r - 18.0)), 2.5, p.ink3);
        }
    }
}

/// A stand-in for the story's photograph: a diagonal gradient with a few soft blobs, opaque.
fn draw_hero(pixels: &mut [u8], w: u32, h: u32) {
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
            let t = (fx * 0.6 + fy * 0.4).clamp(0.0, 1.0);
            let blob = |cx: f32, cy: f32, r: f32| {
                let d = ((fx - cx) * 1.8).powi(2) + (fy - cy).powi(2);
                (1.0 - d / (r * r)).clamp(0.0, 1.0)
            };
            let b = blob(0.3, 0.45, 0.45) * 0.5 + blob(0.78, 0.3, 0.3) * 0.4;
            let (r, g, bl) = (40.0 + 120.0 * t + 90.0 * b, 60.0 + 70.0 * (1.0 - t) + 60.0 * b, 110.0 + 90.0 * (1.0 - t) + 40.0 * b);
            let at = ((y * w + x) * 4) as usize;
            pixels[at] = r.min(255.0) as u8;
            pixels[at + 1] = g.min(255.0) as u8;
            pixels[at + 2] = bl.min(255.0) as u8;
            pixels[at + 3] = 255;
        }
    }
}

#[allow(dead_code)]
fn unused(_: Rect, _: Type) {}
