mod common;

use common::*;
use fetch::FetchError;
use lumatv_feeds::FeedError;
use lumatv_feeds::news::*;

const NOW: u64 = 1_792_900_000_000;

fn world() -> FeedSpec {
    default_feeds().remove(0)
}

fn us() -> FeedSpec {
    default_feeds().remove(1)
}

fn plain(tag: &str, title: &str) -> NewsItem {
    NewsItem {
        tag: tag.to_owned(),
        src: "S".to_owned(),
        title: title.to_owned(),
        summary: String::new(),
        link: String::new(),
        img: String::new(),
        pub_date: String::new(),
        pub_ms: None,
    }
}

fn feed(items: &str) -> String {
    format!("<rss><channel><title>Channel</title>{items}</channel></rss>")
}

#[test]
fn defaults_match_the_qml() {
    let cfg = NewsConfig::standard();
    assert_eq!(cfg.per_feed, 4);
    assert_eq!(cfg.feeds, default_feeds());
    assert_eq!(cfg.alt, default_alt_feeds());
    let f = default_feeds();
    assert_eq!(
        (f[0].tag.as_str(), f[0].src.as_str(), f[0].url.as_str()),
        ("WORLD", "BBC News", BBC_WORLD_URL)
    );
    assert_eq!(
        (f[1].tag.as_str(), f[1].src.as_str(), f[1].url.as_str()),
        ("U.S.", "BBC News", BBC_US_URL)
    );
    let a = default_alt_feeds();
    assert_eq!(
        (a[0].tag.as_str(), a[0].src.as_str(), a[0].url.as_str()),
        ("WORLD", "The New York Times", NYT_WORLD_URL)
    );
    assert_eq!(
        (a[1].tag.as_str(), a[1].src.as_str(), a[1].url.as_str()),
        ("U.S.", "The New York Times", NYT_US_URL)
    );
}

#[test]
fn bbc_world_fixture() {
    let items = parse_feed(BBC_WORLD, &world(), 4);
    assert_eq!(items.len(), 4);
    let first = &items[0];
    assert_eq!(first.tag, "WORLD");
    assert_eq!(first.src, "BBC News");
    assert_eq!(
        first.title,
        "Almost 1,000 arrested as clashes follow another day of mass protests across France"
    );
    assert_eq!(
        first.summary,
        "Hundreds of thousands took part in demonstrations in France, with protests also taking place in other European countries."
    );
    assert_eq!(
        first.link,
        "https://www.bbc.co.uk/news/articles/c95ynd587ynzo?at_medium=RSS&at_campaign=rss"
    );
    assert_eq!(
        first.img,
        "https://ichef.bbci.co.uk/ace/standard/240/cpsprodpb/3d00/live/edf93830-cffb-11f1-b444-6fb1f13f8d87.jpg"
    );
    assert_eq!(first.pub_date, "Sun, 25 Oct 2026 04:16:52 GMT");
    // 2026-10-25 is day 20_751 since the epoch: 20_751 * 86_400 + 4:16:52 = 1_792_901_812 s.
    assert_eq!(first.pub_ms, Some(1_792_901_812_000));

    let second = &items[1];
    assert_eq!(
        second.title,
        "Ukraine says Starlink connection 'now stable' after network limits"
    );
    assert_eq!(second.pub_date, "Sat, 24 Oct 2026 12:12:40 GMT");
    assert_eq!(second.pub_ms, Some(1_792_843_960_000));
    assert_eq!(
        items[2].title,
        "Palestinian families displaced as Israeli forces demolish Hebron homes"
    );
    assert_eq!(
        items[3].title,
        "Man arrested after fire at Hitler's mountaintop Eagle's Nest retreat"
    );
    // a trailing space inside the CDATA is trimmed
    assert_eq!(
        items[3].summary,
        "Some 200 emergency personnel were deployed to fight the blaze at the Nazi dictator's Alpine residence in Germany."
    );
}

#[test]
fn bbc_world_fixture_all_six() {
    let items = parse_feed(BBC_WORLD, &world(), 10);
    assert_eq!(items.len(), 6);
    assert_eq!(
        items[4].title,
        "Father who killed Oregon school gunman describes anguish over children he couldn't save"
    );
    assert_eq!(
        items[5].title,
        "British woman dies after trying to rescue sons swept away at sea in the Bahamas"
    );
    assert_eq!(
        items[5].summary,
        "Nathan Gregory says the loss of childhood sweetheart Leanne \"is unimaginable for our family\"."
    );
    for it in &items {
        assert!(it.link.starts_with("https://www.bbc.co.uk/news/articles/"));
        assert!(!it.link.contains("&amp;"));
        assert!(
            it.img
                .starts_with("https://ichef.bbci.co.uk/ace/standard/240/")
        );
        assert!(it.pub_ms.is_some());
    }
}

#[test]
fn bbc_us_fixture_heads() {
    let items = parse_feed(BBC_US, &us(), 4);
    assert_eq!(items.len(), 4);
    assert_eq!(items[0].tag, "U.S.");
    assert_eq!(
        items[0].title,
        "Obama attacks Trump as he campaigns for Democrats ahead of midterm elections"
    );
    assert_eq!(
        items[0].summary,
        "\"This election is our last best chance to at least rein in some of the foolishness of this administration,\" Obama said."
    );
    assert_eq!(
        items[0].link,
        "https://www.bbc.co.uk/news/articles/cmj6jwe1j1gjo?at_medium=RSS&at_campaign=rss"
    );
    assert_eq!(
        items[1].title,
        "Are California voters about to impose 'world's first' tax on billionaires?"
    );
    assert_eq!(
        items[2].title,
        "White House lifts tariffs on some Canadian goods as midterm elections loom"
    );
}

#[test]
fn nyt_world_fixture() {
    let items = parse_feed(NYT_WORLD, &default_alt_feeds()[0], 4);
    assert_eq!(items.len(), 4);
    let first = &items[0];
    assert_eq!(first.tag, "WORLD");
    assert_eq!(first.src, "The New York Times");
    assert_eq!(
        first.title,
        "Elon Musk Says Ukraine Violated Starlink\u{2019}s Rules"
    );
    assert_eq!(
        first.summary,
        "The comments were his first since the disruption in the country\u{2019}s satellite service. Earlier, Ukraine said drones using Starshield, the Pentagon-funded service, were back to normal speeds."
    );
    // the <link>, not the <atom:link> that follows the <guid>
    assert_eq!(
        first.link,
        "https://www.nytimes.com/2026/10/24/world/europe/ukraine-starshield-drone-speed.html"
    );
    // the only media is 1800 wide: nothing is <= 1200, so it is the "over" image
    assert_eq!(
        first.img,
        "https://static01.nyt.com/images/2026/10/24/multimedia/24int-ukraine-starshield-jmgk/24int-ukraine-starshield-jmgk-mediumSquareAt3X.jpg"
    );
    assert_eq!(first.pub_date, "Sun, 25 Oct 2026 00:10:07 +0000");
    assert_eq!(first.pub_ms, Some(1_792_887_007_000));
    assert_eq!(
        items[1].title,
        "Iranians Face Anxious Wait as Threat of Attack Looms"
    );
    assert_eq!(
        items[2].title,
        "Italy\u{2019}s Young Protesters Return to the Streets"
    );

    let all = parse_feed(NYT_WORLD, &default_alt_feeds()[0], 10);
    assert_eq!(all.len(), 6);
    assert_eq!(
        all[4].title,
        "\u{2018}Eagle\u{2019}s Nest,\u{2019} Nazi-Era Guesthouse in German Alps, Is Gutted by Fire"
    );
    for it in &all {
        assert!(
            it.img
                .starts_with("https://static01.nyt.com/images/2026/10/24/multimedia/"),
            "{}",
            it.title
        );
        assert!(
            it.link.starts_with("https://www.nytimes.com/"),
            "{}",
            it.link
        );
        assert!(it.pub_ms.is_some());
    }
    // 1491 is the narrowest wide image of its item, but it is the only one there
    assert!(all[4].img.contains("24int-germany-fire-tlcj"));
}

#[test]
fn nyt_us_fixture() {
    let items = parse_feed(NYT_US, &default_alt_feeds()[1], 10);
    assert_eq!(items.len(), 6);
    assert!(
        items
            .iter()
            .all(|i| i.tag == "U.S." && !i.title.is_empty() && !i.title.contains('<'))
    );
}

#[test]
fn per_feed_limit_and_empty_titles() {
    assert_eq!(parse_feed(BBC_WORLD, &world(), 2).len(), 2);
    assert_eq!(parse_feed(BBC_WORLD, &world(), 6).len(), 6);
    assert_eq!(parse_feed(BBC_WORLD, &world(), 100).len(), 6);
    assert!(parse_feed(BBC_WORLD, &world(), 0).is_empty());

    let xml = feed(
        "<item><title>  </title></item><item><title><![CDATA[]]></title></item><item><description>d</description></item><item><title>Real</title></item><item><title>Second</title></item>",
    );
    let items = parse_feed(&xml, &world(), 1);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].title, "Real");
    // channel-level title is not an item
    assert!(parse_feed(&feed(""), &world(), 4).is_empty());
    // an unterminated item is ignored
    assert!(parse_feed("<item><title>Cut", &world(), 4).is_empty());
}

#[test]
fn element_names_match_whole_names_only() {
    let xml = feed(
        "<item><titles>Wrong</titles><title lang=\"en\">Right &amp; <b>bold</b></title>\
         <atom:link href=\"https://wrong/\" rel=\"self\"/><link>https://right/a?x=1&amp;y=2</link>\
         <descriptions>no</descriptions><description>Yes</description><pubDate>Sun, 25 Oct 2026 00:00:00 GMT</pubDate></item>",
    );
    let items = parse_feed(&xml, &world(), 4);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].title, "Right & bold");
    assert_eq!(items[0].link, "https://right/a?x=1&y=2");
    assert_eq!(items[0].summary, "Yes");
    assert_eq!(items[0].pub_ms, Some(1_792_886_400_000));

    let only_atom = feed(
        "<item><title>T</title><atom:link href=\"https://wrong/\"/><link rel=\"x\">https://also-wrong/</link></item>",
    );
    let items = parse_feed(&only_atom, &world(), 4);
    assert_eq!(items[0].link, "");
    assert_eq!(items[0].summary, "");
    assert_eq!(items[0].img, "");
    assert_eq!(items[0].pub_date, "");
    assert_eq!(items[0].pub_ms, None);
}

fn image_of(tags: &str) -> String {
    let xml = feed(&format!("<item><title>T</title>{tags}</item>"));
    parse_feed(&xml, &world(), 1).remove(0).img
}

#[test]
fn image_selection_rules() {
    // widest at most 1200 wins
    assert_eq!(
        image_of(
            "<media:thumbnail width=\"240\" url=\"a\"/><media:thumbnail width=\"480\" url=\"b\"/><media:content url=\"c\" width=\"1300\"/>"
        ),
        "b"
    );
    // 1200 still qualifies; a later, narrower one does not replace it
    assert_eq!(
        image_of(
            "<media:content width=\"1200\" url=\"x\"/><media:thumbnail url=\"y\" width=\"100\"/>"
        ),
        "x"
    );
    // equal width: the first stays
    assert_eq!(
        image_of(
            "<media:thumbnail width=\"480\" url=\"first\"/><media:thumbnail width=\"480\" url=\"second\"/>"
        ),
        "first"
    );
    // no width counts as 0 and still qualifies; a wider one replaces it
    assert_eq!(image_of("<media:content url=\"nowidth\"/>"), "nowidth");
    assert_eq!(
        image_of("<media:content url=\"nowidth\"/><media:content url=\"w\" width=\"10\"/>"),
        "w"
    );
    // only wide images: the narrowest
    assert_eq!(
        image_of(
            "<media:content width=\"1800\" url=\"big\"/><media:content width=\"1300\" url=\"mid\"/><media:content width=\"1500\" url=\"upper\"/>"
        ),
        "mid"
    );
    // single quotes, other attributes around, `&amp;` decoded, nothing else touched
    assert_eq!(
        image_of(
            "<media:content medium='image' url='https://i/x.jpg?a=1&amp;b=2' width='300'></media:content>"
        ),
        "https://i/x.jpg?a=1&b=2"
    );
    assert_eq!(
        image_of("<media:thumbnail url=\"https://i/standard/240/p.jpg\" width=\"240\"/>"),
        "https://i/standard/240/p.jpg"
    );
    // a tag without url is skipped; other media elements are not images
    assert_eq!(
        image_of(
            "<media:content width=\"100\"/><media:credit url=\"no\"/><media:description>d</media:description>"
        ),
        ""
    );
    assert_eq!(
        image_of("<media:content data-url=\"no\" width=\"100\"/>"),
        ""
    );
    assert_eq!(image_of(""), "");
}

#[test]
fn decode_text_table() {
    let cases = [
        ("", ""),
        ("plain", "plain"),
        ("<![CDATA[Hello]]>", "Hello"),
        (
            "<![CDATA[<b>markup inside CDATA</b>]]>",
            "markup inside CDATA",
        ),
        ("a <b>bold <i>nested</i></b> word", "a bold nested word"),
        ("&amp;lt;", "&lt;"),
        ("&lt;p&gt;", "<p>"),
        ("Tom &amp; Jerry", "Tom & Jerry"),
        ("&quot;x&quot;", "\"x\""),
        ("&#39;a&#039;b&apos;c", "'a'b'c"),
        ("it&#8217;s", "it\u{2019}s"),
        ("it&#x2019;s", "it\u{2019}s"),
        ("it&#X2019;s", "it\u{2019}s"),
        ("a&nbsp;b", "a b"),
        ("a&#160;&#160;b", "a b"),
        ("  a \n\t b\r\n  c  ", "a b c"),
        ("a<>b", "a<>b"),
        ("5 < 6", "5 < 6"),
        ("&#xD800;", "&#xD800;"),
        ("&#0;", "&#0;"),
        ("&#99999999;", "&#99999999;"),
        ("&#;", "&#;"),
        ("&#+39;", "&#+39;"),
        ("&#12", "&#12"),
        ("&amp;#8217;", "&#8217;"),
        ("<br/>", ""),
        ("   ", ""),
    ];
    for (input, want) in cases {
        assert_eq!(decode_text(input), want, "decode_text({input:?})");
    }
}

#[test]
fn merge_interleaves_and_dedupes() {
    let a = vec![plain("A", "one"), plain("A", "Two"), plain("A", "three")];
    let b = vec![plain("B", "ONE"), plain("B", "four")];
    let c = vec![];
    let d = vec![plain("D", "TWO"), plain("D", "five")];
    let out = merge(vec![a, b, c, d]);
    let got: Vec<(&str, &str)> = out
        .iter()
        .map(|i| (i.tag.as_str(), i.title.as_str()))
        .collect();
    // round 0: A:one, B:ONE (duplicate), D:TWO; round 1: A:Two (duplicate: D took "two" in round 0), B:four, D:five; round 2: A:three
    assert_eq!(
        got,
        [
            ("A", "one"),
            ("D", "TWO"),
            ("B", "four"),
            ("D", "five"),
            ("A", "three")
        ]
    );
}

#[test]
fn merge_round_robin_order() {
    let out = merge(vec![
        vec![plain("A", "a0"), plain("A", "a1"), plain("A", "a2")],
        vec![plain("B", "b0"), plain("B", "b1")],
    ]);
    let titles: Vec<&str> = out.iter().map(|i| i.title.as_str()).collect();
    assert_eq!(titles, ["a0", "b0", "a1", "b1", "a2"]);
    assert!(merge(vec![]).is_empty());
    assert!(merge(vec![vec![], vec![]]).is_empty());
}

#[test]
fn rfc2822_dates() {
    let cases = [
        ("Sun, 25 Oct 2026 00:10:07 +0000", Some(1_792_887_007_000)),
        ("Sun, 25 Oct 2026 04:16:52 GMT", Some(1_792_901_812_000)),
        ("Sun, 25 Oct 2026 00:10:07 UT", Some(1_792_887_007_000)),
        ("Sun, 25 Oct 2026 00:10:07 UTC", Some(1_792_887_007_000)),
        ("Sun, 25 Oct 2026 00:10:07 Z", Some(1_792_887_007_000)),
        ("Sun, 25 Oct 2026 00:10:07 -0500", Some(1_792_905_007_000)),
        ("Sun, 25 Oct 2026 00:10:07 EST", Some(1_792_905_007_000)),
        ("Sun, 25 Oct 2026 00:10:07 EDT", Some(1_792_901_407_000)),
        ("Sun, 25 Oct 2026 00:10:07 CST", Some(1_792_908_607_000)),
        ("Sun, 25 Oct 2026 00:10:07 CDT", Some(1_792_905_007_000)),
        ("Sun, 25 Oct 2026 00:10:07 MST", Some(1_792_912_207_000)),
        ("Sun, 25 Oct 2026 00:10:07 MDT", Some(1_792_908_607_000)),
        ("Sun, 25 Oct 2026 00:10:07 PST", Some(1_792_915_807_000)),
        ("Sun, 25 Oct 2026 00:10:07 PDT", Some(1_792_912_207_000)),
        ("Sun, 25 Oct 2026 00:10:07 +0530", Some(1_792_867_207_000)),
        ("Sun, 25 Oct 2026 00:10 GMT", Some(1_792_887_000_000)),
        ("25 Oct 2026 00:00:00 GMT", Some(1_792_886_400_000)),
        (
            "5 Oct 2026 00:00:00 GMT",
            Some(1_792_886_400_000 - 20 * 86_400_000),
        ),
        ("25 oct 2026 00:00:00 gmt", Some(1_792_886_400_000)),
        ("  Sun, 25 Oct 2026 00:00:00 GMT  ", Some(1_792_886_400_000)),
        ("Thu, 01 Jan 1970 00:00:00 GMT", Some(0)),
        ("Thu, 29 Feb 2024 12:00:00 GMT", Some(1_709_208_000_000)),
        ("Sun, 25 Foo 2026 00:00:00 GMT", None),
        ("Sun, 25 Oct 26 00:00:00 GMT", None),
        ("Sun, 32 Oct 2026 00:00:00 GMT", None),
        ("Sun, 00 Oct 2026 00:00:00 GMT", None),
        ("Sun, 31 Nov 2026 00:00:00 GMT", None),
        ("Fri, 29 Feb 2026 00:00:00 GMT", None),
        ("Sun, 25 Oct 2026 24:00:00 GMT", None),
        ("Sun, 25 Oct 2026 00:60:00 GMT", None),
        ("Sun, 25 Oct 2026 00:00:60 GMT", None),
        ("Sun, 25 Oct 2026 00:00:00", None),
        ("Sun, 25 Oct 2026 00:00:00 XYZ", None),
        ("Sun, 25 Oct 2026 00:00:00 +00", None),
        ("Sun, 25 Oct 2026 00:00:00 +0060", None),
        ("Sun, 25 Oct 2026 00:00:00 GMT extra", None),
        ("Sunday, 25 Oct 2026 00:00:00 GMT", None),
        ("Wed, 31 Dec 1969 23:59:59 GMT", None),
        ("Thu, 01 Jan 1970 00:00:00 +0100", None),
        ("yesterday", None),
        ("2026-10-25T00:00:00Z", None),
        ("", None),
    ];
    for (input, want) in cases {
        assert_eq!(parse_rfc2822_ms(input), want, "{input:?}");
    }
}

fn bbc_script() -> Script {
    Script::new(vec![
        (BBC_WORLD_URL, ok(BBC_WORLD)),
        (BBC_US_URL, ok(BBC_US)),
    ])
}

#[test]
fn refresh_interleaves_both_feeds() {
    let script = bbc_script();
    let doc = refresh(&script, &NewsConfig::standard(), NOW).unwrap();
    assert_eq!(script.requests(), [BBC_WORLD_URL, BBC_US_URL]);
    assert_eq!(doc.schema, 1);
    assert_eq!(doc.fetched_at, NOW);
    assert_eq!(doc.stale_at, NOW + 900_000);
    assert_eq!(doc.expires_at, NOW + 21_600_000);
    assert_eq!(doc.feeds, [BBC_WORLD_URL, BBC_US_URL]);
    assert_eq!(doc.items.len(), 8);
    let tags: Vec<&str> = doc.items.iter().map(|i| i.tag.as_str()).collect();
    assert_eq!(
        tags,
        [
            "WORLD", "U.S.", "WORLD", "U.S.", "WORLD", "U.S.", "WORLD", "U.S."
        ]
    );
    assert_eq!(
        doc.items[0].title,
        "Almost 1,000 arrested as clashes follow another day of mass protests across France"
    );
    assert_eq!(
        doc.items[1].title,
        "Obama attacks Trump as he campaigns for Democrats ahead of midterm elections"
    );
    assert_eq!(
        doc.items[2].title,
        "Ukraine says Starlink connection 'now stable' after network limits"
    );
    assert_eq!(
        doc.items[3].title,
        "Are California voters about to impose 'world's first' tax on billionaires?"
    );
}

#[test]
fn refresh_honours_per_feed() {
    let cfg = NewsConfig {
        per_feed: 1,
        ..NewsConfig::standard()
    };
    let doc = refresh(&bbc_script(), &cfg, NOW).unwrap();
    assert_eq!(doc.items.len(), 2);
}

#[test]
fn refresh_survives_one_failing_feed() {
    let script = Script::new(vec![
        (BBC_WORLD_URL, Err(FetchError::Http(503))),
        (BBC_US_URL, ok(BBC_US)),
    ]);
    let doc = refresh(&script, &NewsConfig::standard(), NOW).unwrap();
    assert_eq!(doc.items.len(), 4);
    assert!(doc.items.iter().all(|i| i.tag == "U.S."));
    assert_eq!(doc.feeds, [BBC_WORLD_URL, BBC_US_URL]);
    // the alternative feeds are not asked when the primary ones gave something
    assert_eq!(script.requests(), [BBC_WORLD_URL, BBC_US_URL]);
}

#[test]
fn refresh_falls_back_to_the_alternative_feeds() {
    let script = Script::new(vec![
        (BBC_WORLD_URL, Err(FetchError::Offline)),
        (BBC_US_URL, Err(FetchError::Timeout)),
        (NYT_WORLD_URL, ok(NYT_WORLD)),
        (NYT_US_URL, ok(NYT_US)),
    ]);
    let doc = refresh(&script, &NewsConfig::standard(), NOW).unwrap();
    assert_eq!(
        script.requests(),
        [BBC_WORLD_URL, BBC_US_URL, NYT_WORLD_URL, NYT_US_URL]
    );
    // the key is the configuration, not the source that answered
    assert_eq!(doc.feeds, [BBC_WORLD_URL, BBC_US_URL]);
    assert_eq!(doc.items.len(), 8);
    assert!(doc.items.iter().all(|i| i.src == "The New York Times"));
    assert_eq!(
        doc.items[0].title,
        "Elon Musk Says Ukraine Violated Starlink\u{2019}s Rules"
    );
    assert_eq!(doc.items[0].tag, "WORLD");
    assert_eq!(doc.items[1].tag, "U.S.");
}

#[test]
fn refresh_falls_back_when_primary_feeds_have_no_items() {
    let script = Script::new(vec![
        (BBC_WORLD_URL, ok("<rss></rss>")),
        (BBC_US_URL, ok("<html>maintenance</html>")),
        (NYT_WORLD_URL, ok(NYT_WORLD)),
        (NYT_US_URL, Err(FetchError::Offline)),
    ]);
    let doc = refresh(&script, &NewsConfig::standard(), NOW).unwrap();
    assert_eq!(doc.items.len(), 4);
    assert!(doc.items.iter().all(|i| i.tag == "WORLD"));
}

#[test]
fn refresh_everything_offline_reports_the_first_error() {
    let script = Script::new(vec![
        (BBC_WORLD_URL, Err(FetchError::Offline)),
        (BBC_US_URL, Err(FetchError::Offline)),
        (NYT_WORLD_URL, Err(FetchError::Offline)),
        (NYT_US_URL, Err(FetchError::Offline)),
    ]);
    let err = refresh(&script, &NewsConfig::standard(), NOW).unwrap_err();
    assert_eq!(err, FeedError::Fetch(FetchError::Offline));
    assert_eq!(err.code(), "offline");
    assert_eq!(script.requests().len(), 4);

    let script = Script::new(vec![
        (BBC_WORLD_URL, Err(FetchError::Timeout)),
        (BBC_US_URL, Err(FetchError::Offline)),
        (NYT_WORLD_URL, Err(FetchError::Offline)),
        (NYT_US_URL, Err(FetchError::Offline)),
    ]);
    assert_eq!(
        refresh(&script, &NewsConfig::standard(), NOW).unwrap_err(),
        FeedError::Fetch(FetchError::Timeout)
    );
}

#[test]
fn refresh_pages_without_items_are_empty_not_an_error_of_transport() {
    let page = "<html><body>no stories</body></html>";
    let script = Script::new(vec![
        (BBC_WORLD_URL, ok(page)),
        (BBC_US_URL, ok(page)),
        (NYT_WORLD_URL, ok(page)),
        (NYT_US_URL, ok(page)),
    ]);
    let err = refresh(&script, &NewsConfig::standard(), NOW).unwrap_err();
    assert_eq!(err, FeedError::Empty);
    assert_eq!(err.code(), "empty");
}

#[test]
fn refresh_with_no_feeds_is_empty() {
    let cfg = NewsConfig {
        feeds: vec![],
        alt: vec![],
        per_feed: 4,
    };
    assert_eq!(
        refresh(&Script::new(vec![]), &cfg, NOW).unwrap_err(),
        FeedError::Empty
    );
}

#[test]
fn refresh_reads_non_utf8_feeds_lossily() {
    let mut bytes = b"<rss><item><title>Caf".to_vec();
    bytes.push(0xE9);
    bytes.extend_from_slice(b"</title></item></rss>");
    let script = Script::new(vec![
        (BBC_WORLD_URL, Ok(bytes)),
        (BBC_US_URL, Err(FetchError::Offline)),
    ]);
    let doc = refresh(&script, &NewsConfig::standard(), NOW).unwrap();
    assert_eq!(doc.items.len(), 1);
    assert_eq!(doc.items[0].title, "Caf\u{fffd}");
}

#[test]
fn json_shape() {
    let doc = refresh(&bbc_script(), &NewsConfig::standard(), NOW).unwrap();
    let v = serde_json::to_value(&doc).unwrap();
    let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "expiresAt",
            "feeds",
            "fetchedAt",
            "items",
            "schema",
            "staleAt"
        ]
    );
    let mut item: Vec<&str> = v["items"][0]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    item.sort_unstable();
    assert_eq!(
        item,
        [
            "img", "link", "pub", "pubMs", "src", "summary", "tag", "title"
        ]
    );
    assert_eq!(v["items"][0]["pub"], "Sun, 25 Oct 2026 04:16:52 GMT");
    assert_eq!(v["items"][0]["pubMs"], 1_792_901_812_000_u64);

    let mut unknown = plain("T", "no date");
    unknown.pub_date = "soon".to_owned();
    let v = serde_json::to_value(&unknown).unwrap();
    assert!(v["pubMs"].is_null());
    assert_eq!(v["pub"], "soon");
    let back: NewsItem = serde_json::from_value(v).unwrap();
    assert_eq!(back, unknown);

    let back: NewsDoc = serde_json::from_value(serde_json::to_value(&doc).unwrap()).unwrap();
    assert_eq!(back, doc);
}
