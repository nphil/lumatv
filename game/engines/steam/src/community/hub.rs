//! The hub's content fragments: what players share (screenshots, artwork, videos) and the guides.
//!
//! Steam answers `homecontent` with a run of cards as HTML, which is the least stable thing the engine reads. A card is an element with
//! the class `apphub_Card` and nothing else on the page is looked at, so a change around the cards costs nothing and a change inside
//! one costs that card. A page with cards of which none reads is `Unreadable`.

use super::html::{Doc, El};
use super::{count, https, listed};
use lumagame_core::community::{Absence, Guide, HubItem, MAX_BLURB, MAX_GUIDES, MAX_HUB_ITEMS, MAX_NAME, MAX_TITLE, Section};

/// Longest id Steam gives a published file, in digits.
const MAX_ID: usize = 20;
/// A YouTube video id.
const YOUTUBE_ID: usize = 11;

pub(super) fn items(page: &str) -> Section<Vec<HubItem>> {
    cards(page, MAX_HUB_ITEMS, item)
}

pub(super) fn guides(page: &str) -> Section<Vec<Guide>> {
    cards(page, MAX_GUIDES, guide)
}

fn cards<T>(page: &str, max: usize, read: fn(&Doc<'_>, El) -> Option<T>) -> Section<Vec<T>> {
    // Steam sends nothing at all for a kind the game has none of.
    if page.trim().is_empty() {
        return Section::absent(Absence::Empty);
    }
    let doc = Doc::parse(page);
    let cards: Vec<El> = doc.with_class(None, "apphub_Card").take(max).collect();
    if cards.is_empty() {
        return Section::absent(Absence::Unreadable);
    }
    listed(cards.len(), cards.into_iter().filter_map(|card| read(&doc, card)).collect())
}

fn item(doc: &Doc<'_>, card: El) -> Option<HubItem> {
    let id = id(doc, card)?;
    let image = https(&doc.attr(doc.first(Some(card), "apphub_CardContentPreviewImage")?, "src")?)?;
    Some(HubItem {
        title: words(doc, card, "apphub_CardContentTitle", MAX_TITLE),
        author: words(doc, card, "apphub_CardContentAuthorName", MAX_NAME),
        youtube: youtube(&image),
        likes: number(doc, card, "apphub_CardRating"),
        comments: number(doc, card, "apphub_CardCommentCount"),
        image,
        id,
    })
}

fn guide(doc: &Doc<'_>, card: El) -> Option<Guide> {
    let id = id(doc, card)?;
    let title = doc.first(Some(card), "apphub_CardContentGuideTitle")?;
    let name = doc.line(title, MAX_TITLE);
    if name.is_empty() {
        return None;
    }
    Some(Guide {
        title: name,
        summary: words(doc, card, "apphub_CardContentGuideDesc", MAX_BLURB),
        author: words(doc, card, "apphub_CardContentAuthorName", MAX_NAME),
        image: doc.first(Some(title), "apphub_CardContentGuideImage").and_then(|image| doc.attr(image, "src")).and_then(|src| https(&src)),
        id,
    })
}

/// The published file's id: its own attribute, or the address of its page, or the card's element id.
fn id(doc: &Doc<'_>, card: El) -> Option<String> {
    let digits = |text: &str| (!text.is_empty() && text.len() <= MAX_ID && text.bytes().all(|byte| byte.is_ascii_digit())).then(|| text.to_owned());
    doc.attr(card, "data-publishedfileid")
        .and_then(|id| digits(&id))
        .or_else(|| {
            doc.attr(card, "data-modal-content-url").and_then(|url| url.rsplit_once("id=").and_then(|(_, id)| digits(id.split('&').next().unwrap_or_default())))
        })
        .or_else(|| doc.attr(card, "id").and_then(|id| id.strip_prefix("apphub_Card_").and_then(digits)))
}

/// The id of a video, from the thumbnail Steam shows for it: the only place the fragment says where a video is from.
fn youtube(image: &str) -> Option<String> {
    let id = image.strip_prefix("https://img.youtube.com/vi/")?.split('/').next()?;
    (id.len() == YOUTUBE_ID && id.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))).then(|| id.to_owned())
}

fn words(doc: &Doc<'_>, card: El, class: &str, max: usize) -> String {
    doc.first(Some(card), class).map(|el| doc.line(el, max)).unwrap_or_default()
}

fn number(doc: &Doc<'_>, card: El, class: &str) -> u32 {
    doc.first(Some(card), class).map_or(0, |el| count(&doc.line(el, 16)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREENSHOTS: &str = include_str!("fixtures/hub_screenshots.html");
    const VIDEOS: &str = include_str!("fixtures/hub_videos.html");
    const ARTWORK: &str = include_str!("fixtures/hub_artwork.html");
    const GUIDES: &str = include_str!("fixtures/hub_guides.html");

    fn present<T: std::fmt::Debug>(section: Section<T>) -> T {
        match section {
            Section::Present { value } => value,
            absent @ Section::Absent { .. } => panic!("expected a present section, got {absent:?}"),
        }
    }

    fn absence<T: std::fmt::Debug>(section: Section<T>) -> Absence {
        match section {
            Section::Absent { reason } => reason,
            present @ Section::Present { .. } => panic!("expected an absent section, got {present:?}"),
        }
    }

    #[test]
    fn screenshots_give_id_title_author_picture_page_and_counts() {
        let items = present(items(SCREENSHOTS));
        assert_eq!(items.len(), 2);
        let first = &items[0];
        assert_eq!(
            (first.id.as_str(), first.title.as_str(), first.author.as_str()),
            ("3842423884", "I always dreamed of being an actor… I wanted to see my face on a show’s poster.", "Player One")
        );
        assert_eq!(first.image, "https://images.steamusercontent.com/ugc/1/AA/?imw=1024&&ima=fit&impolicy=Letterbox&imcolor=%23000000&letterbox=false");
        assert_eq!((first.likes, first.comments, first.youtube.as_deref()), (66, 56, None));
        assert_eq!((items[1].likes, items[1].comments, items[1].title.as_str()), (1_234, 1_005, ""), "thousands separators; a card with no title has none");
    }

    #[test]
    fn videos_name_their_youtube_id_and_artwork_reads_like_screenshots() {
        let videos = present(items(VIDEOS));
        assert_eq!(videos[0].youtube.as_deref(), Some("09ErOIROi8c"));
        assert_eq!((videos[0].title.as_str(), videos[0].likes, videos[0].comments), ("Girly Protag Nomad", 0, 0), "a card with no rating has none");
        assert_eq!(videos[0].image, "https://img.youtube.com/vi/09ErOIROi8c/0.jpg");
        assert_eq!(videos[1].youtube, None, "a thumbnail that is not YouTube's names no video");
        let artwork = present(items(ARTWORK));
        assert_eq!((artwork.len(), artwork[0].title.as_str()), (2, "/ / / Λ ＦＴＥＲＬＩＦＥ"));
    }

    #[test]
    fn guides_give_title_summary_author_and_icon() {
        let guides = present(guides(GUIDES));
        assert_eq!(guides.len(), 2);
        let first = &guides[0];
        assert_eq!(first.id, "3838373048");
        assert_eq!(first.title, "All 7 Collectible Locations in Cyberpunk 2077 (UPDATED 2026)");
        assert_eq!(first.summary, "Night City holds seven pieces of an old life, and six of them lead straight to a trophy.");
        assert_eq!(first.author, "Guide Writer");
        assert_eq!(
            first.image.as_deref(),
            Some("https://images.steamusercontent.com/ugc/2/BB/?imw=128&imh=128&ima=fit&impolicy=Letterbox&imcolor=%23000000&letterbox=true")
        );
        assert_eq!((guides[1].id.as_str(), guides[1].image.as_deref()), ("3838000001", None), "the id comes from the page address when the attribute is gone");
    }

    #[test]
    fn nothing_sent_is_empty_and_cards_that_changed_are_unreadable() {
        assert_eq!(absence(items("")), Absence::Empty);
        assert_eq!(absence(items("  \n\t ")), Absence::Empty);
        assert_eq!(absence(guides("")), Absence::Empty);
        assert_eq!(
            absence(items("<html><body><h1>Maintenance</h1></body></html>")),
            Absence::Unreadable,
            "a page with no cards at all is not the empty answer"
        );
        let renamed = SCREENSHOTS.replace("apphub_Card ", "hub_Card ");
        assert_eq!(absence(items(&renamed)), Absence::Unreadable);
        let no_picture = SCREENSHOTS.replace("apphub_CardContentPreviewImage", "preview");
        assert_eq!(absence(items(&no_picture)), Absence::Unreadable, "cards that exist but cannot be read");
        assert_eq!(absence(guides(&GUIDES.replace("apphub_CardContentGuideTitle", "title"))), Absence::Unreadable);
    }

    #[test]
    fn a_card_that_does_not_read_costs_only_itself() {
        let broken = SCREENSHOTS.replacen("https://images.steamusercontent.com/ugc/1/AA/?imw=1024&&", "http://images.invalid/", 1);
        let items = present(items(&broken));
        assert_eq!(items.len(), 1, "the first card's picture is not https");
        assert_eq!(items[0].id, "3842423885");
    }

    #[test]
    fn the_id_comes_from_the_first_place_that_has_one_and_only_digits_count() {
        let card =
            |attrs: &str| format!(r#"<div class="apphub_Card" {attrs}><img class="apphub_CardContentPreviewImage" src="https://a.invalid/p.jpg"></div>"#);
        let id_of = |attrs: &str| items(&card(attrs)).value().map(|items| items[0].id.clone());
        assert_eq!(id_of(r#"data-publishedfileid="12" data-modal-content-url="https://x/?id=34" id="apphub_Card_56""#).as_deref(), Some("12"));
        assert_eq!(id_of(r#"data-modal-content-url="https://x/sharedfiles/filedetails/?id=34&amp;tab=1" id="apphub_Card_56""#).as_deref(), Some("34"));
        assert_eq!(id_of(r#"data-publishedfileid="1x" data-modal-content-url="https://x/?id=oops" id="apphub_Card_56""#).as_deref(), Some("56"));
        assert_eq!(id_of(r#"data-publishedfileid="123456789012345678901""#), None, "more digits than an id has");
        assert_eq!(id_of(""), None);
    }

    #[test]
    fn a_page_holds_no_more_cards_than_the_limit() {
        let card = r#"<div class="apphub_Card" data-publishedfileid="7"><img class="apphub_CardContentPreviewImage" src="https://a.invalid/p.jpg"></div>"#;
        assert_eq!(present(items(&card.repeat(MAX_HUB_ITEMS + 5))).len(), MAX_HUB_ITEMS);
    }

    #[test]
    fn only_a_real_youtube_thumbnail_names_a_video() {
        assert_eq!(youtube("https://img.youtube.com/vi/abc-DEF_123/0.jpg").as_deref(), Some("abc-DEF_123"));
        for image in [
            "https://img.youtube.com/vi/short/0.jpg",
            "https://img.youtube.com/vi/abc-DEF_123?/0.jpg",
            "https://evil.invalid/https://img.youtube.com/vi/abc-DEF_123/0.jpg",
            "https://img.youtube.com/vi//0.jpg",
        ] {
            assert_eq!(youtube(image), None, "{image}");
        }
    }
}
