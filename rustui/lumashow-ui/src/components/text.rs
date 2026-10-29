//! Text shortcuts every component shares: runs of text drawn into one image layer, lines broken to a width, text cut to fit,
//! and a timer's reading. Text is rastered here and nowhere else, and only when it changes.

use super::label::Align;
use crate::context::Ui;
use crate::scene::ImageLayer;
use crate::text::{Canvas, Fonts, Type};
use compositor_protocol::Rgba;

/// One piece of text: `x`/`w` is the box it is aligned in and `y` the top of its line box ([`line_height`]).
#[derive(Clone, Copy, Debug)]
pub struct Run<'a> {
    pub text: &'a str,
    pub ty: Type,
    pub colour: Rgba,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub align: Align,
}

impl<'a> Run<'a> {
    pub fn left(text: &'a str, ty: Type, colour: Rgba, x: f32, y: f32, w: f32) -> Self {
        Self {
            text,
            ty,
            colour,
            x,
            y,
            w,
            align: Align::Left,
        }
    }

    pub fn right(text: &'a str, ty: Type, colour: Rgba, x: f32, y: f32, w: f32) -> Self {
        Self {
            align: Align::Right,
            ..Self::left(text, ty, colour, x, y, w)
        }
    }

    pub fn centre(text: &'a str, ty: Type, colour: Rgba, x: f32, y: f32, w: f32) -> Self {
        Self {
            align: Align::Centre,
            ..Self::left(text, ty, colour, x, y, w)
        }
    }
}

/// The height of one line of `ty`: Lucent's 1.4 line.
pub fn line_height(ty: Type) -> f32 {
    (ty.px as f32 * 1.4).ceil()
}

/// Where a run of `width` starts in a box of `box_w` starting at `x`.
pub(crate) fn aligned(x: f32, box_w: f32, width: f32, align: Align) -> f32 {
    x + match align {
        Align::Left => 0.0,
        Align::Centre => ((box_w - width) / 2.0).max(0.0),
        Align::Right => (box_w - width).max(0.0),
    }
}

/// Clears `layer` and draws `runs` into it, then tells the compositor the whole layer changed: one layer, one upload, however
/// many runs, for what changes together (a row's title and detail, every legend of a keyboard page).
pub fn draw_runs(ui: &mut Ui, layer: &mut ImageLayer, runs: &[Run]) {
    paint_runs(ui, layer, runs);
    layer.damage(None);
}

/// Like [`draw_runs`], but leaves the layer undamaged: for a caller that paints more on the layer and then damages it once.
pub fn paint_runs(ui: &mut Ui, layer: &mut ImageLayer, runs: &[Run]) {
    layer.clear();
    let width = layer.width() as usize;
    for run in runs {
        let extent = ui.fonts.measure(run.text, run.ty);
        let x = aligned(run.x, run.w, extent.width, run.align);
        let baseline =
            run.y + ((line_height(run.ty) - extent.height()) / 2.0 + extent.ascent).round();
        ui.fonts.draw(
            &mut Canvas {
                pixels: layer.pixels_mut(),
                width,
            },
            run.text,
            run.ty,
            (x, baseline),
            run.colour,
        );
    }
}

/// `text` broken into lines no wider than `max_w` at `ty`, at spaces; a word wider than a line stands alone on its own line (the
/// layer clips it). Newlines in `text` start a new paragraph and blank lines are kept.
pub fn wrap(fonts: &mut Fonts, text: &str, ty: Type, max_w: f32) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            if line.is_empty() {
                line = word.to_owned();
                continue;
            }
            let joined = format!("{line} {word}");
            if fonts.measure(&joined, ty).width <= max_w {
                line = joined;
            } else {
                lines.push(std::mem::replace(&mut line, word.to_owned()));
            }
        }
        lines.push(line);
    }
    lines
}

/// `wrap`, kept to `max_lines` lines: what does not fit ends the last line in an ellipsis.
pub fn wrap_to(
    fonts: &mut Fonts,
    text: &str,
    ty: Type,
    max_w: f32,
    max_lines: usize,
) -> Vec<String> {
    let mut lines = wrap(fonts, text, ty, max_w);
    if max_lines > 0 && lines.len() > max_lines {
        let rest = lines.split_off(max_lines - 1).join(" ");
        lines.push(ellipsise(fonts, &rest, ty, max_w));
    }
    lines
}

/// `text` cut at its end to fit `max_w` pixels at `ty`, ending in an ellipsis when something had to go (Lucent truncates secondary
/// text, never a label that matters). A space never sits before the ellipsis.
pub fn ellipsise(fonts: &mut Fonts, text: &str, ty: Type, max_w: f32) -> String {
    if fonts.measure(text, ty).width <= max_w {
        return text.to_owned();
    }
    let room = max_w - fonts.measure("…", ty).width;
    let chars: Vec<char> = text.chars().collect();
    let mut used = 0.0;
    let mut keep = 0;
    for (i, c) in chars.iter().enumerate() {
        used += fonts.measure(&c.to_string(), ty).width;
        if used > room {
            break;
        }
        keep = i + 1;
    }
    while keep > 0 && chars[keep - 1] == ' ' {
        keep -= 1;
    }
    let mut cut: String = chars[..keep].iter().collect();
    cut.push('…');
    cut
}

/// `text` cut at its start to fit `max_w` pixels at `ty`, with an ellipsis in front of what is left: for a field whose end is where
/// the caret is, so what was just typed is always in view.
pub fn fit_tail(fonts: &mut Fonts, text: &str, ty: Type, max_w: f32) -> String {
    if fonts.measure(text, ty).width <= max_w {
        return text.to_owned();
    }
    let room = max_w - fonts.measure("…", ty).width;
    let chars: Vec<char> = text.chars().collect();
    let mut used = 0.0;
    let mut start = chars.len();
    for (i, c) in chars.iter().enumerate().rev() {
        used += fonts.measure(&c.to_string(), ty).width;
        if used > room {
            break;
        }
        start = i;
    }
    let mut cut = String::from("…");
    cut.extend(&chars[start..]);
    cut
}

/// `m:ss`, the way a timer or a track position reads (minutes are not padded).
pub fn minutes_seconds(total_seconds: u32) -> String {
    format!("{}:{:02}", total_seconds / 60, total_seconds % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::Weight;

    const BODY: Type = Type {
        px: 20,
        weight: Weight::Regular,
    };

    #[test]
    fn text_that_fits_is_left_alone_and_text_that_does_not_is_cut_at_its_end_with_an_ellipsis_that_fits()
     {
        let mut fonts = Fonts::new();
        assert_eq!(ellipsise(&mut fonts, "Home", BODY, 300.0), "Home");
        let name = "A network with a very long name indeed, as neighbours' routers have";
        let cut = ellipsise(&mut fonts, name, BODY, 240.0);
        assert!(
            cut.ends_with('…') && name.starts_with(cut.trim_end_matches('…')),
            "{cut}"
        );
        assert!(fonts.measure(&cut, BODY).width <= 240.0, "{cut}");
        assert!(
            fonts.measure(&cut, BODY).width > 200.0,
            "as much as fits is kept: {cut}"
        );
        let spaced = ellipsise(&mut fonts, "Guest  network  name  here", BODY, 150.0);
        assert!(!spaced.trim_end_matches('…').ends_with(' '), "{spaced:?}");
        assert_eq!(ellipsise(&mut fonts, "Anything", BODY, 2.0), "…");
    }

    #[test]
    fn text_cut_at_its_start_keeps_the_end_and_fits() {
        let mut fonts = Fonts::new();
        assert_eq!(fit_tail(&mut fonts, "abc", BODY, 300.0), "abc");
        let cut = fit_tail(
            &mut fonts,
            "0123456789 abcdefghijklmnopqrstuvwxyz",
            BODY,
            200.0,
        );
        assert!(cut.starts_with('…') && cut.ends_with("xyz"), "{cut}");
        assert!(fonts.measure(&cut, BODY).width <= 200.0, "{cut}");
    }

    #[test]
    fn wrapped_lines_fit_the_width_and_keep_every_word_in_order() {
        let mut fonts = Fonts::new();
        let text =
            "LumaShow keeps every screen at sixty frames by drawing text only when it changes";
        let lines = wrap(&mut fonts, text, BODY, 300.0);
        assert!(lines.len() > 1);
        for l in &lines {
            assert!(
                fonts.measure(l, BODY).width <= 300.0,
                "{l:?} is wider than the line"
            );
        }
        assert_eq!(lines.join(" "), text);
    }

    #[test]
    fn a_word_wider_than_the_line_stands_alone_and_paragraphs_survive() {
        let mut fonts = Fonts::new();
        let lines = wrap(
            &mut fonts,
            "a\n\nsupercalifragilisticexpialidocious b",
            BODY,
            60.0,
        );
        assert_eq!(lines, ["a", "", "supercalifragilisticexpialidocious", "b"]);
    }

    #[test]
    fn a_text_kept_to_some_lines_ends_the_last_in_an_ellipsis_that_fits() {
        let mut fonts = Fonts::new();
        let text = "one two three four five six seven eight nine ten eleven twelve thirteen";
        let lines = wrap_to(&mut fonts, text, BODY, 150.0, 2);
        assert_eq!(lines.len(), 2);
        assert!(lines[1].ends_with('…'), "{lines:?}");
        assert!(lines.iter().all(|l| fonts.measure(l, BODY).width <= 150.0));
        // Text that fits is not touched, and a limit of zero is no limit.
        assert_eq!(wrap_to(&mut fonts, "short", BODY, 150.0, 2), ["short"]);
        assert!(wrap_to(&mut fonts, text, BODY, 150.0, 0).len() > 2);
    }

    #[test]
    fn a_line_box_is_one_and_four_tenths_of_the_size() {
        assert_eq!(line_height(BODY), 28.0);
    }

    #[test]
    fn a_timer_reads_minutes_unpadded_and_seconds_padded() {
        assert_eq!(minutes_seconds(0), "0:00");
        assert_eq!(minutes_seconds(59), "0:59");
        assert_eq!(minutes_seconds(60), "1:00");
        assert_eq!(minutes_seconds(272), "4:32");
        assert_eq!(minutes_seconds(3600 + 5), "60:05");
    }
}
