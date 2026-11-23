//! A forgiving reader for the few pages the engine takes from Steam's community site.
//!
//! It lists a page's elements in page order, each with where its attributes and contents lie, and answers the few questions the
//! parsers ask: which elements carry a class, what an attribute says, what the text inside is. A page that is not what a parser expects
//! has no such elements, so a changed page ends in an empty answer and never in a panic.

use super::text;
use std::borrow::Cow;
use std::ops::Range;

/// A page is read up to this many elements. A forum page of 15 posts has about 1,200.
const MAX_ELEMENTS: usize = 20_000;

/// Elements that never have contents.
const VOID: &[&str] = &["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr"];

#[derive(Clone, Copy)]
struct Span {
    start: u32,
    end: u32,
}

/// Offsets are 32 bits: `Doc::parse` refuses a page that is longer, so the clamp only guards the arithmetic.
fn offset(at: usize) -> u32 {
    u32::try_from(at).unwrap_or(u32::MAX)
}

impl Span {
    fn new(start: usize, end: usize) -> Self {
        Self { start: offset(start), end: offset(end) }
    }

    fn range(self) -> Range<usize> {
        self.start as usize..self.end as usize
    }
}

struct Node {
    name: Span,
    /// Between the name and the `>`.
    attrs: Span,
    inner: Span,
    /// The number of the first element that is not inside this one.
    after: u32,
}

/// One element of a [`Doc`].
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct El(u32);

pub(super) struct Doc<'a> {
    src: &'a str,
    nodes: Vec<Node>,
}

impl<'a> Doc<'a> {
    pub(super) fn parse(src: &'a str) -> Self {
        let src = if u32::try_from(src.len()).is_ok() { src } else { "" };
        let mut nodes: Vec<Node> = Vec::new();
        // The elements opened and not yet closed, innermost last.
        let mut open: Vec<usize> = Vec::new();
        let mut at = 0;
        while nodes.len() < MAX_ELEMENTS {
            let Some(lt) = src.get(at..).and_then(|tail| tail.find('<')).map(|found| at + found) else { break };
            at = lt + 1;
            if src[lt..].starts_with("<!--") {
                at = src[lt + 4..].find("-->").map_or(src.len(), |end| lt + 4 + end + 3);
                continue;
            }
            let closing = src.as_bytes().get(lt + 1) == Some(&b'/');
            let name_start = lt + 1 + usize::from(closing);
            let Some(name) = tag_name(src.get(name_start..).unwrap_or_default()) else { continue };
            let name_end = name_start + name.len();
            // A tag that does not end is skipped, not the rest of the page: one odd attribute costs its own element only.
            let Some(gt) = text::tag_end(src.get(name_end..).unwrap_or_default()).map(|found| name_end + found) else {
                at = src.ceil_char_boundary(name_end + text::MAX_TAG);
                continue;
            };
            at = gt + 1;
            if closing {
                // Closes the nearest open element of that name and what was left open inside it; a close with no open is ignored.
                let named = |candidate: &usize| src.get(nodes[*candidate].name.range()).is_some_and(|candidate_name| candidate_name.eq_ignore_ascii_case(name));
                if let Some(depth) = open.iter().rposition(named) {
                    for node in open.drain(depth..) {
                        nodes[node].inner.end = offset(lt);
                        nodes[node].after = offset(nodes.len());
                    }
                }
                continue;
            }
            let number = nodes.len();
            let attrs = Span::new(name_end, gt);
            let node = Node { name: Span::new(name_start, name_end), attrs, inner: Span::new(gt + 1, src.len()), after: 0 };
            let raw = name.eq_ignore_ascii_case("script") || name.eq_ignore_ascii_case("style");
            if raw || src[..gt].ends_with('/') || VOID.iter().any(|void| name.eq_ignore_ascii_case(void)) {
                // No contents to read: a script's text is code, and a void element has none.
                if raw {
                    at = text::element_end(src, gt + 1, name);
                }
                nodes.push(Node { inner: Span::new(gt + 1, gt + 1), after: offset(number + 1), ..node });
            } else {
                nodes.push(node);
                open.push(number);
            }
        }
        for node in open {
            nodes[node].after = offset(nodes.len());
        }
        Self { src, nodes }
    }

    fn node(&self, el: El) -> Option<&Node> {
        self.nodes.get(el.0 as usize)
    }

    /// The elements with `class` among their classes, in page order: all of them, or those inside `within`.
    pub(super) fn with_class<'d>(&'d self, within: Option<El>, class: &'d str) -> impl Iterator<Item = El> + 'd {
        let (from, to) = match within.and_then(|el| self.node(el).map(|node| (el.0 as usize + 1, node.after as usize))) {
            Some(range) => range,
            None if within.is_some() => (0, 0),
            None => (0, self.nodes.len()),
        };
        (from..to).map(|number| El(offset(number))).filter(move |el| self.has_class(*el, class))
    }

    pub(super) fn first(&self, within: Option<El>, class: &str) -> Option<El> {
        self.with_class(within, class).next()
    }

    pub(super) fn has_class(&self, el: El, class: &str) -> bool {
        self.attr(el, "class").is_some_and(|classes| classes.split_ascii_whitespace().any(|token| token == class))
    }

    /// An attribute's value with its entities decoded. A flag with no value reads as the empty text.
    pub(super) fn attr(&self, el: El, name: &str) -> Option<Cow<'a, str>> {
        let attrs = self.src.get(self.node(el)?.attrs.range())?;
        let value = raw_attr(attrs, name)?;
        Some(if value.contains('&') { Cow::Owned(text::decode_entities(value)) } else { Cow::Borrowed(value) })
    }

    /// The text inside the element, as [`text::plain`] reads it.
    pub(super) fn text(&self, el: El, max: usize) -> String {
        text::plain(self.inner(el), max)
    }

    /// The text inside the element on one line, as [`text::line`] reads it.
    pub(super) fn line(&self, el: El, max: usize) -> String {
        text::line(self.inner(el), max)
    }

    fn inner(&self, el: El) -> &'a str {
        self.node(el).and_then(|node| self.src.get(node.inner.range())).unwrap_or_default()
    }
}

/// The tag name at the start of `text`: letters and digits, starting with a letter.
fn tag_name(text: &str) -> Option<&str> {
    let len = text.bytes().take_while(u8::is_ascii_alphanumeric).count();
    text.get(..len).filter(|name| name.starts_with(|c: char| c.is_ascii_alphabetic()))
}

/// The value of attribute `name` in the text between a tag's name and its `>`, quoted or not, as written.
fn raw_attr<'s>(attrs: &'s str, name: &str) -> Option<&'s str> {
    let bytes = attrs.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        while at < bytes.len() && (bytes[at].is_ascii_whitespace() || bytes[at] == b'/') {
            at += 1;
        }
        let start = at;
        while at < bytes.len() && !bytes[at].is_ascii_whitespace() && !matches!(bytes[at], b'=' | b'/' | b'>') {
            at += 1;
        }
        let key = attrs.get(start..at)?;
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        let mut value = "";
        if bytes.get(at) == Some(&b'=') {
            at += 1;
            while at < bytes.len() && bytes[at].is_ascii_whitespace() {
                at += 1;
            }
            let (from, to) = match bytes.get(at) {
                Some(&quote @ (b'"' | b'\'')) => {
                    let from = at + 1;
                    let len = bytes[from..].iter().position(|byte| *byte == quote).unwrap_or(bytes.len() - from);
                    at = from + len + 1;
                    (from, from + len)
                }
                _ => {
                    let from = at;
                    while at < bytes.len() && !bytes[at].is_ascii_whitespace() {
                        at += 1;
                    }
                    (from, at)
                }
            };
            value = attrs.get(from..to)?;
        } else if at == start {
            // A byte that starts no attribute: step over it.
            at += 1;
        }
        if !key.is_empty() && key.eq_ignore_ascii_case(name) {
            return Some(value);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elements_are_found_by_class_token_in_page_order_and_inside_one() {
        let doc = Doc::parse(
            r#"<div class="card big"><b class="t">One</b></div><div class="cardinal"><b class="t">No</b></div><div class="x card"><b class="t">Two</b><i class="t">Three</i></div>"#,
        );
        let cards: Vec<El> = doc.with_class(None, "card").collect();
        assert_eq!(cards.len(), 2, "a class is a whole word, not the start of one");
        assert_eq!(doc.with_class(None, "t").count(), 4);
        let inside: Vec<String> = doc.with_class(Some(cards[1]), "t").map(|el| doc.line(el, 20)).collect();
        assert_eq!(inside, ["Two", "Three"]);
        assert_eq!(doc.first(Some(cards[0]), "t").map(|el| doc.line(el, 20)).as_deref(), Some("One"));
        assert!(doc.first(Some(cards[0]), "nothing").is_none());
    }

    #[test]
    fn attributes_read_whatever_their_quoting_and_decode_their_entities() {
        let doc = Doc::parse(r#"<a HREF = "https://e.x/?a=1&amp;b=2" data-x='1 "2"' data-y=plain data-flag class=k title="é &lt;b&gt; &bogus;">t</a>"#);
        let a = doc.first(None, "k").unwrap();
        assert_eq!(doc.attr(a, "href").as_deref(), Some("https://e.x/?a=1&b=2"));
        assert_eq!(doc.attr(a, "DATA-X").as_deref(), Some(r#"1 "2""#));
        assert_eq!(doc.attr(a, "data-y").as_deref(), Some("plain"));
        assert_eq!(doc.attr(a, "data-flag").as_deref(), Some(""));
        assert_eq!(doc.attr(a, "title").as_deref(), Some("é <b> &bogus;"));
        assert_eq!(doc.attr(a, "missing"), None);
        assert!(matches!(doc.attr(a, "data-y"), Some(Cow::Borrowed(_))), "text without an entity is not copied");
    }

    #[test]
    fn broken_markup_still_nests_where_it_can() {
        // `<p>` is never closed, `</span>` has no opening, `<BR>` and `<img>` are void, tags are not case sensitive.
        let doc = Doc::parse(r#"<DIV class="a"><p class="b">one<BR>two<img src=x></span><span class="c">three</SPAN></div><div class="d">four</div>"#);
        let a = doc.first(None, "a").unwrap();
        assert_eq!(doc.with_class(Some(a), "c").count(), 1);
        assert_eq!(doc.with_class(Some(a), "d").count(), 0, "an unclosed element does not swallow the next sibling of its parent");
        assert_eq!(doc.line(doc.first(None, "c").unwrap(), 20), "three");
        assert_eq!(doc.line(doc.first(None, "d").unwrap(), 20), "four");
    }

    #[test]
    fn scripts_comments_and_stray_angle_brackets_make_no_elements() {
        let doc = Doc::parse(r#"<script>var s = '<div class="x">';</script><!-- <div class="x"> --><style>.x{}</style> 1 < 2 <3 <div class="x">real</div>"#);
        assert_eq!(doc.with_class(None, "x").count(), 1);
        assert_eq!(doc.line(doc.first(None, "x").unwrap(), 20), "real");
    }

    #[test]
    fn an_unfinished_tag_ends_the_markup_and_the_rest_is_still_readable() {
        let doc = Doc::parse(r#"<div class="a">kept</div><div class="b" title="never ends"#);
        assert_eq!(doc.with_class(None, "a").count(), 1);
        assert_eq!(doc.with_class(None, "b").count(), 0);
    }

    #[test]
    fn an_attribute_of_a_few_kilobytes_is_read_and_one_too_long_costs_only_its_own_element() {
        let found = |config: &str| {
            let page = format!(r#"<div class="a" data-config="{config}"><i class="c"></i></div><b class="b">kept</b>"#);
            let doc = Doc::parse(&page);
            (doc.with_class(None, "a").count(), doc.with_class(None, "b").count())
        };
        assert_eq!(found(&"x".repeat(4_000)), (1, 1), "a page's own configuration is this big");
        assert_eq!(found(&"x".repeat(text::MAX_TAG + 10)), (0, 1));
    }

    #[test]
    fn a_page_is_read_up_to_a_number_of_elements() {
        let page = "<i class=\"x\"></i>".repeat(MAX_ELEMENTS + 500);
        assert_eq!(Doc::parse(&page).with_class(None, "x").count(), MAX_ELEMENTS);
    }

    #[test]
    fn a_page_cut_anywhere_never_panics() {
        let page = r#"<div class="c é" data-a="é&amp;" id=1><p>Ünï<br/>x</p><!-- é --><script>é</script><img src="é"></div><a href='é'>l"#;
        for end in (0..=page.len()).filter(|end| page.is_char_boundary(*end)) {
            let doc = Doc::parse(&page[..end]);
            for el in doc.with_class(None, "c").chain(doc.with_class(None, "none")) {
                let _ = (doc.attr(el, "data-a"), doc.text(el, 10), doc.line(el, 10));
            }
        }
    }
}
