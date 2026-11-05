//! The guide as XMLTV, the file media servers and players read (Plex, Jellyfin, Emby, Channels DVR, TiviMate, Kodi).

use crate::model::Programme;
use crate::time;

/// One channel of the document and its programmes.
pub struct Listing<'a> {
    /// The channel's XMLTV id: what a playlist's `tvg-id` names (the TV's guide number, `2.1`).
    pub id: &'a str,
    pub name: &'a str,
    pub programmes: &'a [Programme],
}

/// Text for an element or an attribute: the five XML characters escaped, control characters dropped.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if c.is_control() && c != '\n' && c != '\t' => {}
            c => out.push(c),
        }
    }
    out
}

/// The document: every channel, then every programme of each in start order (UTC times).
pub fn document(listings: &[Listing<'_>]) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<tv generator-info-name=\"lumaguide\">\n",
    );
    for l in listings {
        out.push_str(&format!(
            "<channel id=\"{}\"><display-name>{}</display-name></channel>\n",
            escape(l.id),
            escape(l.name)
        ));
    }
    for l in listings {
        let mut list: Vec<&Programme> = l
            .programmes
            .iter()
            .filter(|p| p.end > p.start && !p.title.is_empty())
            .collect();
        list.sort_by_key(|p| p.start);
        list.dedup_by_key(|p| p.start);
        for p in list {
            out.push_str(&format!(
                "<programme start=\"{}\" stop=\"{}\" channel=\"{}\"><title>{}</title>",
                time::xmltv(p.start),
                time::xmltv(p.end),
                escape(l.id),
                escape(&p.title)
            ));
            if !p.subtitle.is_empty() {
                out.push_str(&format!("<sub-title>{}</sub-title>", escape(&p.subtitle)));
            }
            if !p.text.is_empty() {
                out.push_str(&format!("<desc>{}</desc>", escape(&p.text)));
            }
            if p.captions {
                out.push_str("<subtitles type=\"teletext\" />");
            }
            if !p.rating.is_empty() {
                out.push_str(&format!(
                    "<rating system=\"VCHIP\"><value>{}</value></rating>",
                    escape(&p.rating)
                ));
            }
            out.push_str("</programme>\n");
        }
    }
    out.push_str("</tv>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_document_is_utc_escaped_and_in_start_order() {
        let p = |title: &str, start: i64| Programme {
            id: title.into(),
            channel: "1_39_2_1_0_0_0".into(),
            title: title.into(),
            subtitle: if title == "Q&A" {
                "Part \"1\"".into()
            } else {
                String::new()
            },
            text: String::new(),
            start,
            end: start + 1_800_000,
            rating: if title == "Q&A" {
                "TV-14".into()
            } else {
                String::new()
            },
            captions: title == "Q&A",
            described: false,
        };
        let t = 1_793_901_600_000;
        let programmes = [p("Later", t + 1_800_000), p("Q&A", t)];
        let got = document(&[Listing {
            id: "2.1",
            name: "WSB <HD>",
            programmes: &programmes,
        }]);
        assert_eq!(
            got,
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<tv generator-info-name=\"lumaguide\">\n\
             <channel id=\"2.1\"><display-name>WSB &lt;HD&gt;</display-name></channel>\n\
             <programme start=\"20261105180000 +0000\" stop=\"20261105183000 +0000\" channel=\"2.1\"><title>Q&amp;A</title>\
             <sub-title>Part &quot;1&quot;</sub-title><subtitles type=\"teletext\" /><rating system=\"VCHIP\"><value>TV-14</value></rating></programme>\n\
             <programme start=\"20261105183000 +0000\" stop=\"20261105190000 +0000\" channel=\"2.1\"><title>Later</title></programme>\n\
             </tv>\n"
        );
    }
}
