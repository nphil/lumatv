//! What Steam's community says about one game, as data the app lays out.
//!
//! An engine for a store with a community (Steam) answers [`Request::Community`](crate::ipc::Request::Community) with one
//! [`Community`]: the reviews, the news, how many play now, how rare each achievement is, the store's and the hub's pictures and
//! videos, the hub's discussions and guides. The pages behind them change without notice, so every section stands alone:
//! [`Section::Present`] with something to show, or [`Section::Absent`] with the reason. A page that failed never costs the rest, and a
//! present section is never empty.
//!
//! All text is plain: the engine strips markup, decodes entities and cuts every item to the limits below, so the app only lays it out.
//! Every address is `https`, and none is the owner's: the data is public and needs no sign-in.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

fn is_false(value: &bool) -> bool {
    !*value
}

/// How many items a section holds at most. An engine asks Steam for no more than it keeps.
pub const MAX_REVIEWS: usize = 6;
pub const MAX_NEWS: usize = 6;
pub const MAX_SHOTS: usize = 12;
pub const MAX_TRAILERS: usize = 6;
/// Per kind: screenshots, artwork and videos each hold this many.
pub const MAX_HUB_ITEMS: usize = 6;
pub const MAX_GUIDES: usize = 6;
pub const MAX_TOPICS: usize = 15;
/// The opening post and the first page of replies.
pub const MAX_POSTS: usize = 16;
pub const MAX_RARITY: usize = 1_000;

/// How long text is kept, in characters: a title, a name, a short description, a review, a news post and a post in a discussion.
pub const MAX_TITLE: usize = 160;
pub const MAX_NAME: usize = 64;
pub const MAX_BLURB: usize = 320;
pub const MAX_REVIEW_TEXT: usize = 800;
pub const MAX_NEWS_TEXT: usize = 1_200;
pub const MAX_POST_TEXT: usize = 1_600;
/// The longest address kept, and the longest achievement id, both in bytes. An item with a longer one is left out.
pub const MAX_URL: usize = 512;
pub const MAX_API_NAME: usize = 128;

word_enum! {
    /// Why a section has nothing to show.
    pub enum Absence {
        /// Steam answered, and has nothing of this kind for the game.
        Empty => "empty",
        /// Steam could not be reached, was too slow, or asked for fewer requests. Asking again later can work.
        Unreachable => "unreachable",
        /// Steam answered in a form the engine does not understand: a page it changed. Asking again does not help.
        Unreadable => "unreadable",
        /// Steam asks for an age check first, and the request did not say the owner agreed to one.
        AgeGated => "age-gated",
    }
    unknown => Other
}

/// One part of a [`Community`], on its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum Section<T> {
    Present { value: T },
    Absent { reason: Absence },
}

impl<T> Section<T> {
    pub fn absent(reason: Absence) -> Self {
        Self::Absent { reason }
    }

    pub fn value(&self) -> Option<&T> {
        match self {
            Self::Present { value } => Some(value),
            Self::Absent { .. } => None,
        }
    }
}

impl<T> Section<Vec<T>> {
    /// The items, or [`Absence::Empty`] when there are none.
    pub fn list(items: Vec<T>) -> Self {
        if items.is_empty() { Self::absent(Absence::Empty) } else { Self::Present { value: items } }
    }
}

/// What a section reads as when the engine that sent the rest did not know it: nothing, for a reason this build cannot name.
impl<T> Default for Section<T> {
    fn default() -> Self {
        Self::absent(Absence::Other)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Community {
    pub reviews: Section<Reviews>,
    /// The developer's own posts first; the newest first.
    pub news: Section<Vec<NewsItem>>,
    /// How many are playing now. Absent when Steam says none, which it also does for what it does not count.
    pub players: Section<u32>,
    /// The share of players that has each achievement, by the game's own id for it, in tenths of a percent (`885` is 88.5 %).
    pub rarity: Section<BTreeMap<String, u16>>,
    /// The store page's pictures.
    pub store_screenshots: Section<Vec<Shot>>,
    pub trailers: Section<Vec<Trailer>>,
    /// What players share in the game's community hub, the most talked about first.
    pub hub_screenshots: Section<Vec<HubItem>>,
    pub hub_artwork: Section<Vec<HubItem>>,
    pub hub_videos: Section<Vec<HubItem>>,
    pub guides: Section<Vec<Guide>>,
    /// The hub's discussion list: pinned topics first, then the most recent.
    pub discussions: Section<Vec<Topic>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reviews {
    /// Steam's word for the score (`Very Positive`). None while Steam gives none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// The share of all reviews that recommend the game, 0 to 100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percent: Option<u8>,
    pub positive: u32,
    pub negative: u32,
    /// The most helpful ones. Names and accounts are left out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub top: Vec<Review>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Review {
    pub text: String,
    pub recommended: bool,
    /// How many found it helpful, and how many found it funny.
    pub helpful: u32,
    pub funny: u32,
    /// What the reviewer has played the game for in all, in seconds.
    pub playtime_s: u64,
    /// When it was written, in Unix seconds.
    pub at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewsItem {
    pub title: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Where it was posted (`Community Announcements`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// In Unix seconds.
    pub at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// A store screenshot: a small picture to list and the full-size one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shot {
    pub thumb: String,
    pub full: String,
}

/// A store trailer. H.264 only, which every TV plays; an item holds at least one of the two streams.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trailer {
    pub title: String,
    pub thumb: String,
    /// An HLS playlist.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hls: Option<String>,
    /// A DASH manifest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dash: Option<String>,
}

/// One thing a player shared in the hub: a screenshot, a piece of artwork or a video.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HubItem {
    /// Steam's id for it, digits only.
    pub id: String,
    /// Empty when the player gave none.
    pub title: String,
    pub author: String,
    /// A preview picture.
    pub image: String,
    /// Its page on steamcommunity.com.
    pub url: String,
    /// The YouTube id, for a video.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub youtube: Option<String>,
    pub likes: u32,
    pub comments: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Guide {
    /// Steam's id for it, digits only.
    pub id: String,
    pub title: String,
    pub summary: String,
    pub author: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    pub url: String,
}

/// One line of the hub's discussion list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Topic {
    /// Names the topic in [`Request::CommunityThread`](crate::ipc::Request::CommunityThread). The app never reads inside it.
    pub id: String,
    pub title: String,
    pub author: String,
    pub replies: u32,
    /// When the last reply came, in Unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_post: Option<u64>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub pinned: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub locked: bool,
    /// The first words of the opening post.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub preview: String,
}

/// One topic with its first posts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thread {
    pub title: String,
    /// The opening post first, then the replies, oldest first.
    pub posts: Vec<Post>,
    /// All the replies the topic has, which is more than `posts` holds when it is long.
    pub replies: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Post {
    pub author: String,
    /// In Unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<u64>,
    pub text: String,
}

/// One answer with something in most sections, for the tests that send it through a message.
#[cfg(test)]
pub(crate) fn sample() -> Community {
    Community {
        reviews: Section::Present {
            value: Reviews {
                label: Some("Very Positive".into()),
                percent: Some(89),
                positive: 361_150,
                negative: 42_549,
                top: vec![Review { text: "Great.".into(), recommended: true, helpful: 41, funny: 2, playtime_s: 3_600, at: 1_700_000_000 }],
            },
        },
        news: Section::list(vec![NewsItem {
            title: "Patch 2.5".into(),
            text: "Fixes.".into(),
            author: Some("Dev".into()),
            source: None,
            at: 1_700_000_000,
            url: Some("https://example.invalid/n".into()),
        }]),
        players: Section::Present { value: 47_861 },
        rarity: Section::Present { value: BTreeMap::from([("ACH_WIN".to_owned(), 885)]) },
        store_screenshots: Section::list(vec![Shot { thumb: "https://example.invalid/t.jpg".into(), full: "https://example.invalid/f.jpg".into() }]),
        trailers: Section::absent(Absence::Empty),
        hub_screenshots: Section::list(vec![HubItem {
            id: "123".into(),
            title: String::new(),
            author: "Someone".into(),
            image: "https://example.invalid/i.jpg".into(),
            url: "https://steamcommunity.com/sharedfiles/filedetails/?id=123".into(),
            youtube: None,
            likes: 5,
            comments: 1,
        }]),
        hub_artwork: Section::absent(Absence::Unreachable),
        hub_videos: Section::absent(Absence::Unreadable),
        guides: Section::absent(Absence::Empty),
        discussions: Section::absent(Absence::AgeGated),
    }
}

#[cfg(test)]
pub(crate) fn sample_thread() -> Thread {
    Thread {
        title: "Tips".into(),
        posts: vec![Post { author: "A".into(), at: Some(1_700_000_000), text: "First.".into() }, Post { author: "B".into(), at: None, text: "Second.".into() }],
        replies: 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ProviderGameId;
    use crate::ipc::{Event, FromEngine, MAX_LINE_BYTES, decode, encode};

    #[test]
    fn a_section_is_present_with_a_value_or_absent_with_a_reason() {
        assert_eq!(serde_json::to_string(&Section::Present { value: 7_u32 }).unwrap(), r#"{"state":"present","value":7}"#);
        assert_eq!(serde_json::to_string(&Section::<u32>::absent(Absence::AgeGated)).unwrap(), r#"{"state":"absent","reason":"age-gated"}"#);
        let read: Section<Vec<Shot>> = serde_json::from_str(r#"{"state":"absent","reason":"slow-down"}"#).unwrap();
        assert_eq!(read, Section::absent(Absence::Other), "a reason from a newer engine is still an absence");
        assert!(serde_json::from_str::<Section<u32>>(r#"{"state":"loading"}"#).is_err(), "a state the app cannot act on is refused, not guessed");
    }

    #[test]
    fn a_list_is_present_only_with_something_in_it() {
        let shot = || Shot { thumb: "https://example.invalid/t.jpg".into(), full: "https://example.invalid/f.jpg".into() };
        assert_eq!(Section::<Vec<Shot>>::list(Vec::new()), Section::absent(Absence::Empty));
        assert_eq!(Section::list(vec![shot()]).value().map(Vec::len), Some(1));
        assert_eq!(Section::<u32>::absent(Absence::Empty).value(), None);
    }

    #[test]
    fn a_section_an_older_engine_does_not_send_reads_as_absent() {
        let older: Community = decode(r#"{"players":{"state":"present","value":3}}"#).unwrap();
        assert_eq!(older.players, Section::Present { value: 3 });
        assert_eq!((older.reviews, older.discussions), (Section::absent(Absence::Other), Section::absent(Absence::Other)));
    }

    /// Four bytes a character is the widest text there is, so this is the most a full answer can take.
    #[test]
    fn the_most_an_engine_may_send_leaves_the_line_cap_half_free() {
        let text = |chars: usize| "😀".repeat(chars);
        let url = || format!("https://{}", "a".repeat(MAX_URL - "https://".len()));
        let hub = || {
            Section::list(
                (0..MAX_HUB_ITEMS)
                    .map(|_| HubItem {
                        id: "9".repeat(20),
                        title: text(MAX_TITLE),
                        author: text(MAX_NAME),
                        image: url(),
                        url: url(),
                        youtube: Some("x".repeat(11)),
                        likes: u32::MAX,
                        comments: u32::MAX,
                    })
                    .collect(),
            )
        };
        let full = Community {
            reviews: Section::Present {
                value: Reviews {
                    label: Some(text(MAX_NAME)),
                    percent: Some(100),
                    positive: u32::MAX,
                    negative: u32::MAX,
                    top: (0..MAX_REVIEWS)
                        .map(|_| Review {
                            text: text(MAX_REVIEW_TEXT),
                            recommended: true,
                            helpful: u32::MAX,
                            funny: u32::MAX,
                            playtime_s: u64::MAX,
                            at: u64::MAX,
                        })
                        .collect(),
                },
            },
            news: Section::list(
                (0..MAX_NEWS)
                    .map(|_| NewsItem {
                        title: text(MAX_TITLE),
                        text: text(MAX_NEWS_TEXT),
                        author: Some(text(MAX_NAME)),
                        source: Some(text(MAX_NAME)),
                        at: u64::MAX,
                        url: Some(url()),
                    })
                    .collect(),
            ),
            players: Section::Present { value: u32::MAX },
            rarity: Section::Present { value: (0..MAX_RARITY).map(|n| (format!("{n:0>width$}", width = MAX_API_NAME), u16::MAX)).collect() },
            store_screenshots: Section::list((0..MAX_SHOTS).map(|_| Shot { thumb: url(), full: url() }).collect()),
            trailers: Section::list((0..MAX_TRAILERS).map(|_| Trailer { title: text(MAX_TITLE), thumb: url(), hls: Some(url()), dash: Some(url()) }).collect()),
            hub_screenshots: hub(),
            hub_artwork: hub(),
            hub_videos: hub(),
            guides: Section::list(
                (0..MAX_GUIDES)
                    .map(|_| Guide {
                        id: "9".repeat(20),
                        title: text(MAX_TITLE),
                        summary: text(MAX_BLURB),
                        author: text(MAX_NAME),
                        image: Some(url()),
                        url: url(),
                    })
                    .collect(),
            ),
            discussions: Section::list(
                (0..MAX_TOPICS)
                    .map(|_| Topic {
                        id: "9".repeat(41),
                        title: text(MAX_TITLE),
                        author: text(MAX_NAME),
                        replies: u32::MAX,
                        last_post: Some(u64::MAX),
                        pinned: true,
                        locked: true,
                        preview: text(MAX_BLURB),
                    })
                    .collect(),
            ),
        };
        let game = ProviderGameId::new("1091500").unwrap();
        let line = encode(&FromEngine::answer(u64::MAX, Event::Community { game: game.clone(), community: full }));
        assert!(line.len() < MAX_LINE_BYTES / 2, "{} bytes", line.len());

        let thread = Thread {
            title: text(MAX_TITLE),
            posts: (0..MAX_POSTS).map(|_| Post { author: text(MAX_NAME), at: Some(u64::MAX), text: text(MAX_POST_TEXT) }).collect(),
            replies: u32::MAX,
        };
        let line = encode(&FromEngine::answer(u64::MAX, Event::CommunityThread { game, topic: "9".repeat(41), thread: Section::Present { value: thread } }));
        assert!(line.len() < MAX_LINE_BYTES / 2, "{} bytes", line.len());
    }
}
