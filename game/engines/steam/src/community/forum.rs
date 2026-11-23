//! The hub's discussion list and one discussion, from their HTML pages.
//!
//! Both pages are behind an age check for a game marked mature. The check is not an error: the page says so with its own marker, and
//! that is `AgeGated` rather than a page that changed.

use super::html::{Doc, El};
use super::{count, listed};
use lumagame_core::community::{Absence, MAX_BLURB, MAX_NAME, MAX_POST_TEXT, MAX_POSTS, MAX_TITLE, MAX_TOPICS, Post, Section, Thread, Topic};

/// The longest a number in a topic id may be, in digits.
const MAX_ID: usize = 20;

/// A topic's id as the app holds it, `forum/topic`: its two numbers, each of digits only. `None` for anything else, so that nothing
/// the app sends can name another page of the site.
pub(super) fn split_id(id: &str) -> Option<(&str, &str)> {
    let (forum, topic) = id.split_once('/')?;
    (digits(forum) && digits(topic)).then_some((forum, topic))
}

fn digits(text: &str) -> bool {
    !text.is_empty() && text.len() <= MAX_ID && text.bytes().all(|byte| byte.is_ascii_digit())
}

/// Whether the page is Steam's age check and not the content behind it.
fn age_gated(doc: &Doc<'_>) -> bool {
    doc.first(None, "contentcheck_desc_ctn").is_some()
}

/// The topics of the hub's discussion page: the pinned ones first, then the most recently answered.
pub(super) fn topics(page: &str) -> Section<Vec<Topic>> {
    let doc = Doc::parse(page);
    let rows: Vec<El> = doc.with_class(None, "forum_topic").take(MAX_TOPICS).collect();
    if rows.is_empty() {
        return Section::absent(if age_gated(&doc) {
            Absence::AgeGated
        } else if doc.first(None, "forum_topics_container").is_some() {
            // The list is there and holds no topic: a hub nobody has written in.
            Absence::Empty
        } else {
            Absence::Unreadable
        });
    }
    listed(rows.len(), rows.into_iter().filter_map(|row| topic(&doc, row)).collect())
}

fn topic(doc: &Doc<'_>, row: El) -> Option<Topic> {
    // The address ends `/discussions/<forum>/<topic>/`.
    let href = doc.attr(doc.first(Some(row), "forum_topic_overlay")?, "href")?;
    let mut numbers = href.split_once("/discussions/")?.1.split('/');
    let id = format!("{}/{}", numbers.next()?, numbers.next()?);
    split_id(&id)?;

    let name = doc.first(Some(row), "forum_topic_name")?;
    // A pinned topic's name starts with a label in the reader's language. It is cut off by its own text, whatever the word is.
    let label = doc.first(Some(name), "forum_topic_label");
    let label_text = label.map(|label| doc.line(label, MAX_NAME)).unwrap_or_default();
    let full = doc.line(name, MAX_TITLE + label_text.chars().count() + 1);
    let title = full.strip_prefix(label_text.as_str()).unwrap_or(&full).trim_start().to_owned();
    if title.is_empty() {
        return None;
    }
    Some(Topic {
        id,
        title,
        author: doc.first(Some(row), "forum_topic_op").map(|el| doc.line(el, MAX_NAME)).unwrap_or_default(),
        replies: doc.first(Some(row), "forum_topic_reply_count").map_or(0, |el| count(&doc.line(el, 16))),
        last_post: doc.first(Some(row), "forum_topic_lastpost").and_then(|el| timestamp(doc, el)),
        pinned: doc.has_class(row, "sticky") || label.is_some_and(|label| doc.has_class(label, "sticky_label")),
        locked: doc.has_class(row, "locked"),
        preview: preview(doc, row),
    })
}

/// The first words of the opening post, which Steam keeps as escaped HTML in the row's hover text.
fn preview(doc: &Doc<'_>, row: El) -> String {
    let Some(hover) = doc.attr(row, "data-tooltip-forum") else { return String::new() };
    let inner = Doc::parse(&hover);
    inner.first(None, "topic_hover_text").map(|text| inner.line(text, MAX_BLURB)).unwrap_or_default()
}

fn timestamp(doc: &Doc<'_>, el: El) -> Option<u64> {
    doc.attr(el, "data-timestamp")?.trim().parse().ok()
}

/// One discussion's page: the opening post and the first page of replies.
pub(super) fn thread(page: &str) -> Section<Thread> {
    let doc = Doc::parse(page);
    let Some(op) = doc.first(None, "forum_op") else {
        return Section::absent(if age_gated(&doc) { Absence::AgeGated } else { Absence::Unreadable });
    };
    let title = doc.first(Some(op), "topic").map(|el| doc.line(el, MAX_TITLE)).unwrap_or_default();
    let opening = Post {
        author: doc.first(Some(op), "forum_op_author").map(|el| doc.line(el, MAX_NAME)).unwrap_or_default(),
        at: posted(&doc, op),
        text: doc.first(Some(op), "content").map(|el| doc.text(el, MAX_POST_TEXT)).unwrap_or_default(),
    };
    if title.is_empty() || opening.text.is_empty() {
        return Section::absent(Absence::Unreadable);
    }
    let mut posts = vec![opening];
    posts.extend(doc.with_class(None, "commentthread_comment").take(MAX_POSTS - 1).filter_map(|comment| reply(&doc, comment)));
    let replies = total_replies(page).unwrap_or_else(|| u32::try_from(posts.len() - 1).unwrap_or(u32::MAX));
    Section::Present { value: Thread { title, posts, replies } }
}

fn reply(doc: &Doc<'_>, comment: El) -> Option<Post> {
    let text = doc.text(doc.first(Some(comment), "commentthread_comment_text")?, MAX_POST_TEXT);
    if text.is_empty() {
        return None;
    }
    Some(Post {
        author: doc.first(Some(comment), "commentthread_author_link").map(|el| doc.line(el, MAX_NAME)).unwrap_or_default(),
        at: posted(doc, comment),
        text,
    })
}

/// When a post was written. A reply has two timestamp elements and only the second carries the time.
fn posted(doc: &Doc<'_>, post: El) -> Option<u64> {
    doc.with_class(Some(post), "commentthread_comment_timestamp").find_map(|el| timestamp(doc, el))
}

/// All the replies the topic has, which the page's own script states: it shows only the first page of them.
fn total_replies(page: &str) -> Option<u32> {
    const KEY: &str = "\"total_count\":";
    let script = page.get(page.find("InitializeCommentThread(")?..)?;
    let rest = script.get(script.find(KEY)? + KEY.len()..)?;
    let end = rest.bytes().position(|byte| !byte.is_ascii_digit()).unwrap_or(rest.len());
    rest.get(..end)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DISCUSSIONS: &str = include_str!("fixtures/discussions.html");
    const THREAD: &str = include_str!("fixtures/thread.html");
    const GATE: &str = include_str!("fixtures/age_gate.html");

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
    fn topics_give_title_author_counts_time_flags_and_the_first_words() {
        let topics = present(topics(DISCUSSIONS));
        assert_eq!(topics.len(), 3);
        let pinned = &topics[0];
        assert_eq!(pinned.id, "0/591785860830267587");
        assert_eq!(pinned.title, "Posting in long-inactive threads is NOT forbidden", "the label is not part of the title");
        assert_eq!(
            (pinned.author.as_str(), pinned.replies, pinned.last_post, pinned.pinned, pinned.locked),
            ("Moderator One", 0, Some(1_761_227_875), true, true)
        );
        assert_eq!(
            pinned.preview,
            "There are a lot of reports about posts that revive old threads. These are almost always invalid; posting in long-inactive threads is not against this hub's rules . So, please, only report posts when they clearly break one ..."
        );
        let recent = &topics[1];
        assert_eq!(
            (recent.id.as_str(), recent.title.as_str(), recent.replies, recent.last_post, recent.pinned, recent.locked),
            ("0/565925967576627123", "Crashing after the latest update", 1_204, Some(1_795_386_899), false, false)
        );
        assert_eq!(recent.preview, "I get a crash at the loading screen & the game closes <b>on its own</b>.");
        assert_eq!(topics[2].preview, "", "a row with no hover text has no preview");
    }

    #[test]
    fn a_pinned_topic_is_known_by_its_label_in_any_language() {
        let german = DISCUSSIONS.replace("PINNED:", "ANGEHEFTET:").replace("forum_topic  unread locked sticky", "forum_topic  unread locked");
        let topics = present(topics(&german));
        assert!(topics[0].pinned && topics[0].locked);
        assert_eq!(topics[0].title, "Posting in long-inactive threads is NOT forbidden");
    }

    #[test]
    fn a_topic_needs_an_address_a_title_and_digits_for_its_id() {
        let no_address = DISCUSSIONS.replacen("forum_topic_overlay", "overlay", 1);
        assert_eq!(present(topics(&no_address)).len(), 2, "the row without an address is left out");
        let odd_id = DISCUSSIONS.replacen("/discussions/0/591785860830267587/", "/discussions/0/59178x/", 1);
        assert_eq!(present(topics(&odd_id)).len(), 2);
    }

    #[test]
    fn the_age_check_is_age_gated_and_not_a_changed_page() {
        assert_eq!(absence(topics(GATE)), Absence::AgeGated);
        assert_eq!(absence(thread(GATE)), Absence::AgeGated);
    }

    #[test]
    fn the_configuration_every_live_page_opens_with_does_not_hide_what_follows() {
        // A div whose one attribute is the page's configuration: 2.6 KB of escaped JSON on the real pages.
        let config = format!(r#"<div id="application_config" style="display: none;" data-config="{}"></div>"#, "&quot;a&quot;:1,".repeat(200));
        assert_eq!(present(topics(&format!("{config}{DISCUSSIONS}"))).len(), 3);
        assert_eq!(present(thread(&format!("{config}{THREAD}"))).posts.len(), 3);
        assert_eq!(absence(topics(&format!("{config}{GATE}"))), Absence::AgeGated);
    }

    #[test]
    fn a_list_with_no_rows_is_empty_and_anything_else_is_unreadable() {
        assert_eq!(
            absence(topics(r#"<div id="forum_General_1_topiccontainer" class="forum_topics_container"><div class="forum_topics"></div></div>"#)),
            Absence::Empty
        );
        assert_eq!(absence(topics("<html><body>Something went wrong</body></html>")), Absence::Unreadable);
        assert_eq!(absence(topics("")), Absence::Unreadable);
        let unreadable_rows = DISCUSSIONS.replace("forum_topic_name", "name");
        assert_eq!(absence(topics(&unreadable_rows)), Absence::Unreadable, "rows with nothing readable in them");
    }

    #[test]
    fn a_thread_gives_the_opening_post_and_the_replies_in_order() {
        let thread = present(thread(THREAD));
        assert_eq!(thread.title, "Can Anyone Explain Ricochets to Me?");
        assert_eq!(thread.replies, 13, "the count is the page's own, which is more than the page shows");
        assert_eq!(thread.posts.len(), 3);
        let opening = &thread.posts[0];
        assert_eq!((opening.author.as_str(), opening.at), ("Poster One", Some(1_795_390_824)));
        assert_eq!(opening.text, "I understand the basics of HOW they work, but I'm struggling to see their purpose.\n\nDoes anyone have concrete numbers?");
        let reply = &thread.posts[1];
        assert_eq!(
            (reply.author.as_str(), reply.at, reply.text.as_str()),
            ("Replier One", Some(1_795_394_153), "makes it easier to hit people behind cover with trick shots.")
        );
        assert_eq!(thread.posts[2].text, "Originally posted by Replier One:\nsaid this\n\nI agree & so does my friend.");
    }

    #[test]
    fn a_thread_without_its_count_falls_back_to_the_replies_it_holds() {
        let thread = present(thread(&THREAD.replace("\"total_count\":13", "\"count\":13")));
        assert_eq!(thread.replies, 2);
    }

    #[test]
    fn a_thread_holds_no_more_posts_than_the_limit() {
        let comment = r#"<div class="commentthread_comment responsive_body_text"><a class="commentthread_author_link"><bdi>A</bdi></a><div class="commentthread_comment_text">x</div></div>"#;
        let page = THREAD.replacen("</body>", &format!("{}</body>", comment.repeat(30)), 1);
        let thread = present(thread(&page));
        assert_eq!(thread.posts.len(), MAX_POSTS);
        assert_eq!(thread.replies, 13);
    }

    #[test]
    fn a_page_that_is_not_a_thread_is_unreadable() {
        assert_eq!(absence(thread("<html><body>Not found</body></html>")), Absence::Unreadable);
        assert_eq!(absence(thread("")), Absence::Unreadable);
        assert_eq!(absence(thread(&THREAD.replace("class=\"topic\"", "class=\"t\""))), Absence::Unreadable, "no title");
        assert_eq!(absence(thread(&THREAD.replace("class=\"content\"", "class=\"c\""))), Absence::Unreadable, "no opening text");
        assert_eq!(absence(thread(&THREAD.replace("forum_op ", "op "))), Absence::Unreadable);
    }

    #[test]
    fn only_two_numbers_name_a_topic() {
        assert_eq!(split_id("0/565925967576627123"), Some(("0", "565925967576627123")));
        for id in ["", "0", "0/", "/5", "a/5", "0/5/", "0/5/6", "../5", "0/5?x=1", "0/ 5", "0/123456789012345678901", "-1/5", "0/５"] {
            assert_eq!(split_id(id), None, "{id:?}");
        }
    }
}
