//! The card app's page in the toolkit: the Lucent glass of LumaHome (`look`) on the shipped wallpaper, held still, with one glass card
//! that carries the status or the keys the app has seen, and two buttons. Made once and kept: a key, a state change or a second
//! of uptime restyles a layer or sets the glyphs of a text layer, and nothing is drawn per frame by the app.
//!
//! The words come from [`status_rows`] and [`key_rows`], plain functions of the card's rules (`card`), so the text is tested without a
//! GPU; [`CardUi::show`] only puts them on the layers.

use crate::card::{self, Button, Card, Page, Phase};
use crate::look::{self, MED, Palette, SEMI};
use compositor_protocol::{Props, Wallpaper, WallpaperKind};
use std::io;
use std::path::Path;
use ui::components::{Form, Icon, Item, Mark, Run, TextLayer, line_height};
use ui::geometry::Frame;
use ui::text::Type;
use ui::{GlassLayer, Ui, WallpaperLayer};

pub const W: f32 = 1920.0;
pub const H: f32 = 1080.0;

/// The glass card, centred.
pub const PANEL: Frame = Frame::new(360.0, 150.0, 1200.0, 780.0);
const PAD: f32 = 56.0;
const ROW_TOP: f32 = 204.0;
const ROW_PITCH: f32 = 62.0;
const KEY_PITCH: f32 = 52.0;
/// The hint line, under the buttons.
const HINT_TOP: f32 = 704.0;
/// The keys page's columns: the key, its code, what it did.
const KEY_COLUMNS: [f32; 3] = [0.0, 240.0, 380.0];
const LABEL_W: f32 = 280.0;
const BUTTON_TOP: f32 = 604.0;
const BUTTON_H: f32 = 68.0;
/// Keys, Close.
const BUTTON_W: [f32; 2] = [220.0, 260.0];
const BUTTON_GAP: f32 = 20.0;
const BUTTON_TEXT: [&str; 2] = ["Keys", "Close app"];

const Z_WALL: i32 = 0;
const Z_PANEL: i32 = 10;
const Z_TEXT: i32 = 12;
const Z_BUTTON: i32 = 14;
const Z_BUTTON_TEXT: i32 = 16;

// The card is on the screen and its buttons fit it.
const _: () = {
    assert!(PANEL.x >= 0.0 && PANEL.y >= 0.0 && PANEL.x + PANEL.w <= W && PANEL.y + PANEL.h <= H);
    assert!(PAD + BUTTON_W[0] + BUTTON_GAP + BUTTON_W[1] <= PANEL.w - PAD);
};

const TITLE: Type = look::ty(44, SEMI);
const CAPTION: Type = look::ty(26, MED);
const LABEL: Type = look::ty(28, MED);
const VALUE: Type = look::ty(28, SEMI);
const SMALL: Type = look::ty(22, MED);

/// One line of the status page: the label and what it says now.
pub fn status_rows(card: &Card, uptime_s: u64, graphics: &str) -> Vec<(&'static str, String)> {
    let window = match (card.phase, card.state) {
        (Phase::Hidden, _) => "waiting (preloaded)".to_owned(),
        (_, Some(state)) => format!("{} card", crate::lifecycle::state_name(state)),
        (_, None) => "card, no word from the compositor yet".to_owned(),
    };
    let reason = reason_text(&card.launch_reason);
    let launches = match card.launches {
        1 => format!("once ({reason})"),
        n => format!("{n} times (last: {reason})"),
    };
    let sam = match card.sam {
        None => "no launcher",
        Some(true) => "registered",
        Some(false) => "not registered",
    };
    let key = match card.keys.front() {
        None => "none yet".to_owned(),
        Some(k) => format!("{} {}, code {}", card::key_name(k.code), if k.pressed { "pressed" } else { "released" }, k.code),
    };
    vec![
        ("Window", window),
        ("Graphics", graphics.to_owned()),
        ("Launched", launches),
        ("SAM", sam.to_owned()),
        ("Last key", key),
        ("Up", format!("{}:{:02}", uptime_s / 60, uptime_s % 60)),
    ]
}

/// SAM's word for why it launched the app, in the page's.
fn reason_text(reason: &str) -> &str {
    match reason {
        "undefined" => "normal launch",
        other => other,
    }
}

/// The keys page: the newest key first, as three columns.
pub fn key_rows(card: &Card) -> Vec<[String; 3]> {
    card.keys.iter().map(|k| [card::key_name(k.code).to_owned(), k.code.to_string(), (if k.pressed { "pressed" } else { "released" }).to_owned()]).collect()
}

pub fn hint(page: Page) -> &'static str {
    match page {
        Page::Status => "Back goes up a page, then closes the app  ·  Home leaves it running",
        Page::Keys => "Back returns to the status page",
    }
}

/// The two buttons' words, centred on their glass, in the card's own coordinates.
fn button_items(p: &Palette, frames: &[Frame; 2]) -> Vec<Item<'static>> {
    let lh = line_height(VALUE);
    BUTTON_TEXT
        .iter()
        .zip(frames)
        .map(|(&text, f)| Item::Text(Run::centre(text, VALUE, p.ink, f.x - PANEL.x, f.y - PANEL.y + ((BUTTON_H - lh) / 2.0).round(), f.w)))
        .collect()
}

pub struct CardUi {
    pub ui: Ui,
    p: Palette,
    _wall: WallpaperLayer,
    _panel: GlassLayer,
    _head: TextLayer,
    body: TextLayer,
    labels: TextLayer,
    buttons: [GlassLayer; 2],
    button_frames: [Frame; 2],
    /// What the layers show now, so that nothing is sent twice.
    shown: Option<Shown>,
}

#[derive(Clone, PartialEq, Eq)]
struct Shown {
    page: Page,
    focus: Button,
    rows: Vec<(&'static str, String)>,
    keys: Vec<[String; 3]>,
}

impl CardUi {
    pub fn build(mut ui: Ui, assets: &Path, land: &mut dyn FnMut(&mut Ui) -> io::Result<()>) -> io::Result<Self> {
        let p = look::palette();
        ui.warm(&[TITLE, CAPTION, LABEL, VALUE, SMALL]);
        // The glyphs reach the compositor through two staging slots: until they have landed a layer made from them is a picture for
        // good, so the build waits for them (as the Home's does).
        land(&mut ui)?;
        let inner_w = PANEL.w - 2.0 * PAD;
        ui.warm_marks(&[Form::Disc(96), Form::Icon(Icon::Apps, 44), Form::Rule(inner_w as u32)]);
        land(&mut ui)?;

        let wall = {
            // Held still (speed 0, fps 0): the page costs nothing while nothing changes.
            let mut layer = ui.scene.wallpaper(
                W as u32,
                H as u32,
                Z_WALL,
                Wallpaper { kind: WallpaperKind::Silk, speed: 0.0, intensity: 0.0, fps: 0 },
                Props::at(0.0, 0.0),
            )?;
            let pixels = crate::home::wallpaper_pixels(&assets.join("wallpaper/wall_1080.png"), W as u32, H as u32)?;
            layer.pixels_mut().ok_or_else(|| io::Error::other("the wallpaper has no picture"))?.copy_from_slice(&pixels);
            layer.damage(None);
            layer
        };
        let panel = ui.scene.glass(PANEL.w as u32, PANEL.h as u32, Z_PANEL, look::card(&p, look::R_CARD, false, false), Props::at(PANEL.x, PANEL.y));

        // The head: the mark, the name, the line under it and the rule.
        let text_x = PAD + 96.0 + 28.0;
        let head_items = [
            Item::Mark(Mark::disc(PAD as i32, 48, 96, p.accent)),
            Item::Mark(Mark::icon(Icon::Apps, 44, PAD as i32 + 26, 48 + 26, p.ink_on_accent)),
            Item::Text(Run::left("LumaTV Card", TITLE, p.ink, text_x, 50.0, inner_w - 124.0)),
            Item::Text(Run::left("A 64-bit app in its own card window", CAPTION, p.ink2, text_x, 50.0 + line_height(TITLE), inner_w - 124.0)),
            Item::Mark(Mark::rule(PAD as i32, 176, inner_w as u32, look::white(0.10))),
        ];
        let head = TextLayer::new(&mut ui, PANEL, Z_TEXT, &head_items)?;
        let body = TextLayer::new(&mut ui, PANEL, Z_TEXT + 1, &[])?;

        // The buttons: glass over the card; focus is the glass's highlight.
        let mut button_frames = [Frame::new(0.0, 0.0, 0.0, 0.0); 2];
        let mut x = PANEL.x + PAD;
        for (frame, w) in button_frames.iter_mut().zip(BUTTON_W) {
            *frame = Frame::new(x, PANEL.y + BUTTON_TOP, w, BUTTON_H);
            x += w + BUTTON_GAP;
        }
        let button = |f: Frame| ui.scene.glass(f.w as u32, f.h as u32, Z_BUTTON, look::card(&p, BUTTON_H / 2.0, false, false), Props::at(f.x, f.y));
        let buttons = [button(button_frames[0]), button(button_frames[1])];
        let labels = TextLayer::new(&mut ui, PANEL, Z_BUTTON_TEXT, &button_items(&p, &button_frames))?;
        land(&mut ui)?;
        Ok(Self { ui, p, _wall: wall, _panel: panel, _head: head, body, labels, buttons, button_frames, shown: None })
    }

    /// Puts the card's rules on the layers, sending only what changed since the last call.
    pub fn show(&mut self, card: &Card, uptime_s: u64, graphics: &str) {
        let next = Shown {
            page: card.page,
            focus: card.focus,
            rows: if card.page == Page::Status { status_rows(card, uptime_s, graphics) } else { Vec::new() },
            keys: if card.page == Page::Keys { key_rows(card) } else { Vec::new() },
        };
        if self.shown.as_ref() == Some(&next) {
            return;
        }
        let prev = self.shown.take();
        if prev.as_ref().map(|s| (&s.rows, &s.keys, s.page)) != Some((&next.rows, &next.keys, next.page)) {
            self.draw_body(&next);
        }
        if prev.as_ref().map(|s| (s.page, s.focus)) != Some((next.page, next.focus)) {
            let on_status = next.page == Page::Status;
            for (i, which) in [Button::Keys, Button::Close].into_iter().enumerate() {
                let f = self.button_frames[i];
                self.buttons[i].set(Props { visible: on_status, ..Props::at(f.x, f.y) });
                self.buttons[i].restyle(look::card(&self.p, BUTTON_H / 2.0, on_status && next.focus == which, false));
            }
            let items = if on_status { button_items(&self.p, &self.button_frames) } else { Vec::new() };
            self.labels.draw(&mut self.ui, &items);
        }
        self.shown = Some(next);
    }

    fn draw_body(&mut self, s: &Shown) {
        let p = &self.p;
        let lh = line_height(VALUE);
        let inner_w = PANEL.w - 2.0 * PAD;
        let mut items: Vec<Item> = Vec::new();
        match s.page {
            Page::Status => {
                for (i, (name, text)) in s.rows.iter().enumerate() {
                    let y = ROW_TOP + i as f32 * ROW_PITCH + ((ROW_PITCH - lh) / 2.0).round();
                    items.push(Item::Text(Run::left(name, LABEL, p.ink2, PAD, y, LABEL_W)));
                    items.push(Item::Text(Run::left(text, VALUE, p.ink, PAD + LABEL_W, y, inner_w - LABEL_W)));
                }
            }
            Page::Keys => {
                if s.keys.is_empty() {
                    items.push(Item::Text(Run::left("No key yet. Press a key on the remote.", VALUE, p.ink2, PAD, ROW_TOP + 10.0, inner_w)));
                }
                for (i, line) in s.keys.iter().enumerate() {
                    let y = ROW_TOP + i as f32 * KEY_PITCH;
                    let (ty, colour) = if i == 0 { (VALUE, p.ink) } else { (LABEL, p.ink2) };
                    for (text, x) in line.iter().zip(KEY_COLUMNS) {
                        items.push(Item::Text(Run::left(text, ty, colour, PAD + x, y, 220.0)));
                    }
                }
            }
        }
        items.push(Item::Text(Run::left(hint(s.page), SMALL, p.ink3, PAD, HINT_TOP, inner_w)));
        self.body.draw(&mut self.ui, &items);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::Input;

    #[test]
    fn the_status_page_says_what_the_card_knows() {
        let mut card = Card::new(false, "undefined", true);
        let rows = status_rows(&card, 75, "64-bit Mali");
        assert_eq!(rows[0], ("Window", "card, no word from the compositor yet".to_owned()));
        assert_eq!(rows[2].1, "once (normal launch)");
        assert_eq!(rows[3].1, "not registered");
        assert_eq!(rows[4].1, "none yet");
        assert_eq!(rows[5].1, "1:15");
        card.handle(Input::State(card::FULLSCREEN));
        card.handle(Input::Sam { registered: true });
        card.handle(Input::Key { code: 412, pressed: true });
        card.handle(Input::Relaunch { reason: "undefined".into(), parameters: "{}".into() });
        let rows = status_rows(&card, 3, "Mesa software");
        assert_eq!(rows[0].1, "fullscreen card");
        assert_eq!(rows[1].1, "Mesa software");
        assert_eq!(rows[2].1, "2 times (last: normal launch)");
        assert_eq!(rows[3].1, "registered");
        assert_eq!(rows[4].1, "Back pressed, code 412");
        card.handle(Input::State(card::MINIMIZED));
        assert_eq!(status_rows(&card, 3, "x")[0].1, "minimized card");
        assert_eq!(status_rows(&Card::new(true, "preload", true), 0, "x")[0].1, "waiting (preloaded)");
        assert_eq!(status_rows(&Card::new(false, "undefined", false), 0, "x")[3].1, "no launcher");
    }

    #[test]
    fn the_keys_page_lists_the_newest_first() {
        let mut card = Card::new(false, "undefined", false);
        card.handle(Input::Key { code: 105, pressed: true });
        card.handle(Input::Key { code: 105, pressed: false });
        let columns = |a: &str, b: &str, c: &str| [a.to_owned(), b.to_owned(), c.to_owned()];
        assert_eq!(key_rows(&card), vec![columns("Left", "105", "released"), columns("Left", "105", "pressed")]);
    }

    /// Lucent: no text over text. Every block sits clear of the next, inside the card and the screen, at the sizes set above.
    #[test]
    fn nothing_on_the_page_overlaps() {
        let rows_end = ROW_TOP + 6.0 * ROW_PITCH;
        assert!(rows_end + 16.0 <= BUTTON_TOP, "the rows end at {rows_end}, the buttons start at {BUTTON_TOP}");
        let buttons_end = BUTTON_TOP + BUTTON_H;
        assert!(buttons_end + 16.0 <= HINT_TOP, "the buttons end at {buttons_end}, the hint starts at {HINT_TOP}");
        assert!(HINT_TOP + line_height(SMALL) + 32.0 <= PANEL.h, "the hint ends inside the card with room under it");
        let keys_end = ROW_TOP + card::KEY_LINES as f32 * KEY_PITCH;
        assert!(keys_end + 16.0 <= HINT_TOP, "eight key lines end at {keys_end}");
        assert!(line_height(VALUE) <= ROW_PITCH && line_height(VALUE) <= KEY_PITCH, "a line fits its row");
        assert!(KEY_COLUMNS.windows(2).all(|c| c[1] - c[0] >= 140.0), "the key columns are apart");
    }
}
