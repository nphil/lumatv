//! Steam's words as plain text.
//!
//! Steam sends the same kind of text as BBCode (reviews, guides, announcements), as HTML (forum posts, hover texts, some feeds) or with
//! the markup already removed, and a field can change from one to another without notice. One pass reads all three, so a field that
//! changes its format still comes out as words.

/// A tag that does not end within this many bytes is text. Steam's own pages carry attributes of a few kilobytes (the page's
/// configuration alone is 2.6 KB), which is why it is not smaller.
pub(super) const MAX_TAG: usize = 16_384;
const MAX_BBCODE_TAG: usize = 300;
/// The longest `&name;` looked for. Longer is text.
const MAX_ENTITY: usize = 10;
/// A cut gives up the half-written word at its end when the word started this close to it, in bytes.
const WORD_SLACK: usize = 24;

/// The BBCode tags Steam writes. Any other word in brackets is the author's own and stays.
const BBCODE: &[&str] = &[
    "b",
    "i",
    "u",
    "s",
    "strike",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "p",
    "url",
    "list",
    "olist",
    "*",
    "quote",
    "code",
    "hr",
    "noparse",
    "table",
    "tr",
    "th",
    "td",
    "center",
    "sup",
    "sub",
    "carousel",
    "expand",
    "spoiler",
    "img",
    "previewyoutube",
    "video",
    "dynamiclink",
];

/// `raw` as plain text of at most `max` characters: markup removed, entities decoded, white space collapsed, and every character that
/// draws nothing or reorders its neighbours (controls, zero-width marks, bidirectional overrides) dropped. Paragraphs and list items
/// stay as line breaks. A text that does not fit ends in "…". A spoiler becomes "…" too, and pictures, link addresses and the
/// clan-image placeholders of announcements are dropped.
pub(super) fn plain(raw: &str, max: usize) -> String {
    let mut out = Out::new(max);
    let mut rest = raw;
    while !out.cut {
        let Some(first) = rest.chars().next() else { break };
        let used = match first {
            '<' => html(rest, &mut out),
            '&' => entity(rest).map(|(c, used)| {
                out.push(c);
                used
            }),
            '[' => bbcode(rest, &mut out),
            '{' => placeholder(rest),
            _ => None,
        };
        let used = used.unwrap_or_else(|| {
            out.push(first);
            first.len_utf8()
        });
        rest = rest.get(used..).unwrap_or_default();
    }
    out.finish()
}

/// [`plain`] on one line, for a title or a name: every line break becomes a space.
pub(super) fn line(raw: &str, max: usize) -> String {
    plain(raw, max).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `text` with its entities decoded and nothing else changed: for an attribute value.
pub(super) fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        match entity(rest) {
            Some((c, used)) => {
                out.push(c);
                rest = rest.get(used..).unwrap_or_default();
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The white space put before the next visible character; the widest one asked for wins.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Gap {
    None,
    Space,
    Line,
    Blank,
}

struct Out {
    text: String,
    chars: usize,
    max: usize,
    gap: Gap,
    /// Line breaks of the source since the last visible character: two or more make a blank line.
    breaks: u8,
    /// More visible text came than fits.
    cut: bool,
}

impl Out {
    fn new(max: usize) -> Self {
        Self { text: String::new(), chars: 0, max, gap: Gap::None, breaks: 0, cut: false }
    }

    fn gap(&mut self, gap: Gap) {
        self.gap = self.gap.max(gap);
    }

    /// A line break of the source: one is a new line, two in a row (whatever mix of `\n` and `<br>`) a blank one.
    fn newline(&mut self) {
        self.breaks = self.breaks.saturating_add(1);
        self.gap(if self.breaks > 1 { Gap::Blank } else { Gap::Line });
    }

    fn push(&mut self, c: char) {
        if c.is_whitespace() {
            if c == '\n' {
                self.newline();
            } else {
                self.gap(Gap::Space);
            }
            return;
        }
        if hidden(c) {
            return;
        }
        let lead = match self.gap {
            _ if self.text.is_empty() => "",
            Gap::None => "",
            Gap::Space => " ",
            Gap::Line => "\n",
            Gap::Blank => "\n\n",
        };
        if self.chars + lead.len() + 1 > self.max {
            self.cut = true;
            return;
        }
        self.text.push_str(lead);
        self.text.push(c);
        self.chars += lead.len() + 1;
        (self.gap, self.breaks) = (Gap::None, 0);
    }

    fn bullet(&mut self) {
        self.gap(Gap::Line);
        self.push('•');
        self.gap(Gap::Space);
    }

    /// Stands for what was left out on purpose.
    fn left_out(&mut self) {
        self.gap(Gap::Space);
        self.push('…');
    }

    fn finish(mut self) -> String {
        if self.cut {
            // Room for the ellipsis; a cut inside a word gives that word up, one between words keeps them all.
            let room = self.max.saturating_sub(1);
            let mut between_words = self.gap != Gap::None;
            if self.chars > room {
                let end = self.text.char_indices().nth(room).map_or(self.text.len(), |(index, _)| index);
                self.text.truncate(end);
                between_words = false;
            }
            if !between_words && let Some(space) = self.text.rfind(char::is_whitespace).filter(|space| self.text.len() - space <= WORD_SLACK) {
                self.text.truncate(space);
            }
            let kept = self.text.trim_end_matches(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | ':' | '-' | '(')).len();
            self.text.truncate(kept);
            self.text.push('…');
        }
        self.text
    }
}

/// Characters that draw nothing or change the order of what is around them.
fn hidden(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{ad}' | '\u{200b}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{2069}' | '\u{feff}' | '\u{e0000}'..='\u{e007f}')
}

/// The tag or comment at the start of `rest`, acted on; the bytes it took. `None` when `<` is just a character.
fn html(rest: &str, out: &mut Out) -> Option<usize> {
    if let Some(comment) = rest.strip_prefix("<!--") {
        return Some(4 + comment.find("-->").map_or(comment.len(), |end| end + 3));
    }
    let closing = rest.as_bytes().get(1) == Some(&b'/');
    let start = 1 + usize::from(closing);
    let name = word(rest.get(start..)?, false)?.to_ascii_lowercase();
    let after_name = start + name.len();
    let used = after_name + tag_end(rest.get(after_name..)?)? + 1;
    if !closing {
        let raw = matches!(name.as_str(), "script" | "style");
        if raw || rest.get(..used)?.contains("bb_spoiler") {
            if !raw {
                out.left_out();
            }
            return Some(element_end(rest, used, &name));
        }
    }
    match name.as_str() {
        "br" => out.newline(),
        "div" | "tr" | "hr" | "dt" | "dd" | "section" | "article" | "header" | "footer" => out.gap(Gap::Line),
        "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "ul" | "ol" | "table" | "blockquote" | "pre" => out.gap(Gap::Blank),
        "li" if closing => out.gap(Gap::Line),
        "li" => out.bullet(),
        "td" | "th" if closing => out.gap(Gap::Space),
        _ => {}
    }
    Some(used)
}

/// The bytes before the `>` that ends the tag whose name has just been read, quotes respected. `None` when the tag does not end.
pub(super) fn tag_end(after_name: &str) -> Option<usize> {
    let mut quote = None;
    for (at, byte) in after_name.bytes().enumerate().take(MAX_TAG) {
        match (quote, byte) {
            (None, b'>') => return Some(at),
            (None, b'"' | b'\'') => quote = Some(byte),
            (Some(open), _) if open == byte => quote = None,
            _ => {}
        }
    }
    None
}

/// The tag name at the start of `text`: letters and digits, starting with a letter. BBCode's bullet, `*`, is a name too.
fn word(text: &str, bbcode: bool) -> Option<&str> {
    let len = text.bytes().take_while(|byte| byte.is_ascii_alphanumeric() || (bbcode && *byte == b'*')).count();
    text.get(..len).filter(|name| name.starts_with(|c: char| c.is_ascii_alphabetic() || (bbcode && c == '*')))
}

/// Where the element opened just before `from` ends: just past its closing tag, nested ones of its own name counted. The end of
/// `html` when it is never closed.
pub(super) fn element_end(html: &str, from: usize, name: &str) -> usize {
    let mut depth = 1_usize;
    let mut at = from;
    while let Some(found) = html.get(at..).and_then(|tail| tail.find('<')) {
        let lt = at + found;
        at = lt + 1;
        let tail = html.get(lt + 1..).unwrap_or_default();
        let (closing, tail) = tail.strip_prefix('/').map_or((false, tail), |tail| (true, tail));
        let named = tail.as_bytes().get(..name.len()).is_some_and(|head| head.eq_ignore_ascii_case(name.as_bytes()))
            && !tail.as_bytes().get(name.len()).is_some_and(u8::is_ascii_alphanumeric);
        if !named {
            continue;
        }
        if !closing {
            depth += 1;
        } else if depth > 1 {
            depth -= 1;
        } else {
            return html.get(lt..).and_then(|from_tag| from_tag.find('>')).map_or(html.len(), |gt| lt + gt + 1);
        }
    }
    html.len()
}

/// The character an entity at the start of `rest` stands for, and the bytes it took. Only `&name;` and `&#n;` forms count: Steam
/// escapes with the semicolon, and a bare `&copy=1` in an address is not a sign.
fn entity(rest: &str) -> Option<(char, usize)> {
    let end = rest.get(1..)?.bytes().take(MAX_ENTITY).position(|byte| byte == b';')? + 1;
    let name = rest.get(1..end)?;
    let c = match name.strip_prefix('#') {
        Some(number) => match number.strip_prefix(['x', 'X']) {
            Some(hex) if hex.bytes().all(|byte| byte.is_ascii_hexdigit()) => char::from_u32(u32::from_str_radix(hex, 16).ok()?)?,
            None if number.bytes().all(|byte| byte.is_ascii_digit()) => char::from_u32(number.parse().ok()?)?,
            _ => return None,
        },
        None => match name {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            "apos" => '\'',
            "nbsp" => '\u{a0}',
            "hellip" => '…',
            "mdash" => '—',
            "ndash" => '–',
            "lsquo" => '‘',
            "rsquo" => '’',
            "ldquo" => '“',
            "rdquo" => '”',
            "bull" => '•',
            "middot" => '·',
            "copy" => '©',
            "reg" => '®',
            "trade" => '™',
            "deg" => '°',
            "times" => '×',
            "euro" => '€',
            "pound" => '£',
            _ => return None,
        },
    };
    Some((c, end + 1))
}

/// The BBCode tag at the start of `rest`, acted on; the bytes it took. `None` when the bracket is the author's own.
fn bbcode(rest: &str, out: &mut Out) -> Option<usize> {
    let closing = rest.as_bytes().get(1) == Some(&b'/');
    let start = 1 + usize::from(closing);
    let name = word(rest.get(start..)?, true)?.to_ascii_lowercase();
    if !BBCODE.contains(&name.as_str()) {
        return None;
    }
    let after_name = rest.get(start + name.len()..)?;
    let close = after_name.bytes().take(MAX_BBCODE_TAG).position(|byte| matches!(byte, b']' | b'[' | b'\n'))?;
    if after_name.as_bytes()[close] != b']' || !(close == 0 || after_name.starts_with(['=', ' '])) {
        return None;
    }
    let used = start + name.len() + close + 1;
    match (closing, name.as_str()) {
        // What these hold is shown elsewhere or not at all; a text cut before the closing tag loses the rest with it.
        (false, "img" | "previewyoutube" | "video" | "dynamiclink") => return Some(bbcode_end(rest, used, &name)),
        (false, "spoiler") => {
            out.left_out();
            return Some(bbcode_end(rest, used, &name));
        }
        (_, "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "p") => out.gap(Gap::Blank),
        (_, "list" | "olist" | "quote" | "table" | "tr" | "hr") => out.gap(Gap::Line),
        (false, "*") => out.bullet(),
        (true, "td" | "th") => out.gap(Gap::Space),
        _ => {}
    }
    Some(used)
}

/// Where `[/name]` ends, searched from `from`. The end of `text` when it is not there.
fn bbcode_end(text: &str, from: usize, name: &str) -> usize {
    let mut at = from;
    while let Some(found) = text.get(at..).and_then(|tail| tail.find("[/")) {
        let tag = at + found + 2;
        let tail = text.get(tag..).unwrap_or_default().as_bytes();
        if tail.get(..name.len()).is_some_and(|head| head.eq_ignore_ascii_case(name.as_bytes())) && tail.get(name.len()) == Some(&b']') {
            return tag + name.len() + 1;
        }
        at = tag;
    }
    text.len()
}

/// A clan-image placeholder of an announcement, `{STEAM_CLAN_IMAGE}`, and the path that follows it; the bytes both took.
fn placeholder(rest: &str) -> Option<usize> {
    let inner = rest.strip_prefix("{STEAM_")?;
    let end = inner.bytes().take(32).position(|byte| byte == b'}')?;
    if !inner.as_bytes()[..end].iter().all(|byte| byte.is_ascii_uppercase() || *byte == b'_') {
        return None;
    }
    let used = "{STEAM_".len() + end + 1;
    let path = rest.get(used..)?;
    Some(used + path.find(char::is_whitespace).unwrap_or(path.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(raw: &str) -> String {
        plain(raw, 1_000)
    }

    #[test]
    fn bbcode_is_removed_and_its_structure_kept() {
        assert_eq!(p("[h1]Patch 2.5[/h1]\n[b]Fixed[/b] a [i]bug[/i] in [url=https://example.invalid/x]the map[/url]."), "Patch 2.5\n\nFixed a bug in the map.");
        assert_eq!(p("[list][*]One[*]Two[/list]"), "• One\n• Two");
        assert_eq!(p("[quote=Someone;123]said this[/quote] and I agree"), "said this\nand I agree");
        assert_eq!(p("[B]shouting[/B] [Url]x[/Url]"), "shouting x");
        assert_eq!(p("[table][tr][th]A[/th][th]B[/th][/tr][tr][td]1[/td][td]2[/td][/tr][/table]"), "A B\n1 2");
        assert_eq!(p("a[hr]b"), "a\nb");
    }

    #[test]
    fn pictures_videos_and_spoilers_are_left_out() {
        assert_eq!(p("Look [img]{STEAM_CLAN_IMAGE}/1/a.png[/img] here"), "Look here");
        assert_eq!(p("[previewyoutube=abc;full][/previewyoutube]Watch"), "Watch");
        assert_eq!(p("Fun. [spoiler]The butler did it[/spoiler] Really."), "Fun. … Really.");
        assert_eq!(p("Text [img]{STEAM_CLAN_IMAGE}/1/a"), "Text", "a post cut inside a picture loses only the picture");
        assert_eq!(p(r#"Fun. <span class="bb_spoiler"><span>secret</span></span> Next."#), "Fun. … Next.");
    }

    #[test]
    fn a_bracket_that_is_not_steams_stays() {
        assert_eq!(p("[Update] fixed [1] and [b"), "[Update] fixed [1] and [b");
        assert_eq!(p("[url=broken text"), "[url=broken text");
        assert_eq!(p("2 [/nothing] 3"), "2 [/nothing] 3");
    }

    #[test]
    fn html_is_removed_and_its_blocks_kept() {
        assert_eq!(p("<p>One</p><p>Two<br>three &amp; <b>four</b></p>"), "One\n\nTwo\nthree & four");
        assert_eq!(p("<ul><li>a</li><li>b</li></ul>"), "• a\n• b");
        assert_eq!(p("<script>var x = '<b>';</script>y<style>p{}</style>z"), "yz");
        assert_eq!(p("a<!-- <b>hidden</b> -->b"), "ab");
        assert_eq!(p(r#"<a href="x>y" title='<'>link</a>"#), "link");
        assert_eq!(p("1 < 2 and 3<4 and <3"), "1 < 2 and 3<4 and <3");
        assert_eq!(p("<b unfinished"), "<b unfinished");
    }

    #[test]
    fn entities_are_decoded_once_and_only_when_whole() {
        assert_eq!(p("Tom &amp; Jerry&#39;s &#x1F600; &nbsp;ok &bogus; &amp"), "Tom & Jerry's 😀 ok &bogus; &amp");
        assert_eq!(p("&lt;b&gt;not a tag&lt;/b&gt; &amp;amp;"), "<b>not a tag</b> &amp;");
        assert_eq!(p("a?x=1&copy=2"), "a?x=1&copy=2");
        assert_eq!(p("&#0; &#xD800; &#99999999999; &#xZZ; &#; &#x;"), "&#xD800; &#99999999999; &#xZZ; &#; &#x;");
        assert_eq!(decode_entities("a&amp;b &quot;c&quot; &x"), "a&b \"c\" &x");
    }

    #[test]
    fn announcement_placeholders_and_their_paths_go() {
        assert_eq!(p("{STEAM_CLAN_LOC_IMAGE}/3703047/ab12.png Patch 7.42 is out"), "Patch 7.42 is out");
        assert_eq!(p("{STEAM_CLAN_IMAGE}/1/a.png{STEAM_CLAN_IMAGE}/1/b.png"), "");
        assert_eq!(p("{braces} and {STEAM_lower} stay"), "{braces} and {STEAM_lower} stay");
    }

    #[test]
    fn characters_that_hide_or_reorder_text_are_dropped() {
        assert_eq!(p("a\u{202e}b\u{0}c\u{200b}d\u{feff}e\u{7f}f\u{9f}g\u{e0041}h"), "abcdefgh");
    }

    #[test]
    fn white_space_collapses_and_a_blank_line_survives() {
        assert_eq!(p("  a \t b\r\n\r\n\r\n\r\nc\u{a0}\u{a0}d \n "), "a b\n\nc d");
        assert_eq!(p("one\ntwo\n\nthree"), "one\ntwo\n\nthree");
        assert_eq!(line("  one\n\ntwo \t three\n", 100), "one two three");
    }

    #[test]
    fn a_long_text_is_cut_at_a_word_with_an_ellipsis_within_the_limit() {
        assert_eq!(plain(&"word ".repeat(100), 20), "word word word word…");
        assert_eq!(plain("exactly ten", 11), "exactly ten", "a text that fits is not marked");
        assert_eq!(plain("exactly ten!", 11), "exactly…", "a word cut in half is given up");
        assert_eq!(plain(&"😀".repeat(50), 10), format!("{}…", "😀".repeat(9)), "the limit counts characters, not bytes");
        assert_eq!(plain("anything", 0), "…");
    }

    #[test]
    fn a_huge_text_is_cut_without_reading_all_of_it() {
        let huge = format!("{}end", "<b>x</b> ".repeat(100_000));
        let cut = plain(&huge, 100);
        assert!(cut.chars().count() <= 100 && cut.ends_with('…'), "{cut}");
    }

    #[test]
    fn nothing_in_a_truncated_or_odd_text_can_panic() {
        let sample = "<p class=\"a\">Ünï &#x1F600; &amp [url=https://e.x]é[/url] [spoiler]é[/spoiler] {STEAM_CLAN_IMAGE}/é <!-- é --> <script>é</script></p>";
        for end in (0..=sample.len()).filter(|end| sample.is_char_boundary(*end)) {
            let _ = plain(&sample[..end], 40);
            let _ = decode_entities(&sample[..end]);
        }
    }
}
