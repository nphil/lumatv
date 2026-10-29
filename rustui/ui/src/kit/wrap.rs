//! Words broken into lines: the kit's text is one line in a box, so a paragraph is a few lines, each its own label. [`lines`] breaks at
//! spaces to a width in the style's type (measured, never guessed), keeps explicit breaks, cuts a word that is wider than the line, and
//! ends the last line of a text that does not fit with an ellipsis.

use super::Kit;
use ui::components::Style;

const ELLIPSIS: char = '\u{2026}';

fn width(kit: &mut Kit, style: Style, text: &str) -> f32 {
    kit.ui.fonts.measure(text, style.ty).width
}

/// `text` cut to fit `max_w`, with an ellipsis where it was cut; unchanged when it fits.
pub fn elide(kit: &mut Kit, style: Style, text: &str, max_w: f32) -> String {
    if width(kit, style, text) <= max_w {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    // the longest prefix that fits with the ellipsis, by bisection (widths only grow with the prefix)
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let candidate: String = chars[..mid].iter().collect::<String>().trim_end().to_string() + &ELLIPSIS.to_string();
        if width(kit, style, &candidate) <= max_w {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let head: String = chars[..lo].iter().collect();
    head.trim_end().to_string() + &ELLIPSIS.to_string()
}

/// The lines of `text` in `style` no wider than `max_w`, at most `max_lines` (the last elided when there is more).
pub fn lines(kit: &mut Kit, style: Style, text: &str, max_w: f32, max_lines: usize) -> Vec<String> {
    let max_lines = max_lines.max(1);
    let mut out: Vec<String> = Vec::new();
    let mut more = false;
    'paragraphs: for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            let trial = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if width(kit, style, &trial) <= max_w {
                line = trial;
                continue;
            }
            if !line.is_empty() {
                out.push(std::mem::take(&mut line));
                if out.len() == max_lines {
                    more = true;
                    break 'paragraphs;
                }
            }
            // a word wider than the line begins its own, cut where it must be
            let mut rest = word.to_string();
            while width(kit, style, &rest) > max_w {
                let chars: Vec<char> = rest.chars().collect();
                let (mut lo, mut hi) = (1usize, chars.len());
                while lo < hi {
                    let mid = (lo + hi).div_ceil(2);
                    if width(kit, style, &chars[..mid].iter().collect::<String>()) <= max_w {
                        lo = mid;
                    } else {
                        hi = mid - 1;
                    }
                }
                out.push(chars[..lo].iter().collect());
                rest = chars[lo..].iter().collect();
                if out.len() == max_lines {
                    more = true;
                    break 'paragraphs;
                }
            }
            line = rest;
        }
        out.push(line);
        if out.len() == max_lines && paragraph != text.split('\n').next_back().unwrap_or("") {
            more = true;
            break;
        }
    }
    out.truncate(max_lines);
    if more && let Some(last) = out.last_mut() {
        let joined = format!("{last}{ELLIPSIS}");
        *last = if width(kit, style, &joined) <= max_w { joined } else { elide(kit, style, &format!("{last} {ELLIPSIS}{ELLIPSIS}"), max_w) };
    }
    while out.len() > 1 && out.last().is_some_and(String::is_empty) {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::testkit;
    use crate::kit::text::role_style;
    use lumatv_tokens::TypeRole;
    use ui::components::Align;

    fn style(kit: &Kit) -> Style {
        role_style(kit, TypeRole::Body, None, kit.ui.look.ink(), Align::Left)
    }

    #[test]
    fn lines_break_at_spaces_and_each_fits_the_width() {
        let (_h, mut kit) = testkit::kit(1.0);
        let s = style(&kit);
        let text = "The app and its sign-in on this TV are removed. Downloads in the app are lost. It can be installed again from the LG Content Store.";
        let out = lines(&mut kit, s, text, 600.0, 8);
        assert!(out.len() >= 3, "{out:?}");
        for l in &out {
            assert!(width(&mut kit, s, l) <= 600.0, "{l:?}");
        }
        assert_eq!(out.join(" "), text, "nothing is lost or added");
    }

    #[test]
    fn an_explicit_break_is_kept_and_the_last_line_is_elided_when_text_is_left() {
        let (_h, mut kit) = testkit::kit(1.0);
        let s = style(&kit);
        assert_eq!(lines(&mut kit, s, "One\nTwo", 600.0, 4), vec!["One", "Two"]);
        let long = "word ".repeat(80);
        let out = lines(&mut kit, s, &long, 400.0, 3);
        assert_eq!(out.len(), 3);
        assert!(out[2].ends_with('\u{2026}'), "{out:?}");
        assert!(width(&mut kit, s, &out[2]) <= 400.0);
    }

    #[test]
    fn a_word_wider_than_the_line_is_cut_and_elide_keeps_what_fits() {
        let (_h, mut kit) = testkit::kit(1.0);
        let s = style(&kit);
        let out = lines(&mut kit, s, "Supercalifragilisticexpialidocious_and_more_and_more_and_more", 200.0, 4);
        assert!(out.len() >= 2);
        for l in &out {
            assert!(width(&mut kit, s, l) <= 200.0, "{l:?}");
        }
        let e = elide(&mut kit, s, "A camera named for the room it watches at the back of the house", 250.0);
        assert!(e.ends_with('\u{2026}') && width(&mut kit, s, &e) <= 250.0, "{e:?}");
        assert_eq!(elide(&mut kit, s, "Short", 250.0), "Short");
    }
}
