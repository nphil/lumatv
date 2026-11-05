//! The two things a guide is made of: the broadcast channels the tuner has, and programmes.

use serde::{Deserialize, Serialize};

/// One channel of the tuner's lineup, as the host knows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Channel {
    /// The host's id of the channel (LG's `1_32_2_1_0_0_0`): what the programmes of the guide name.
    pub id: String,
    /// The virtual channel, `2.1`.
    pub major: u16,
    pub minor: u16,
    /// What the broadcast calls the channel (`WSB-HD`): often the station's call sign.
    pub name: String,
    /// The RF channel the station transmits on, when the host knows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rf: Option<u16>,
}

impl Channel {
    /// `2.1`.
    pub fn number(&self) -> String {
        format!("{}.{}", self.major, self.minor)
    }
}

/// One programme of one channel, in epoch milliseconds: the guide data model the TV's guide and channel banner draw.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Programme {
    pub id: String,
    /// The channel's id: the host's for the guide, the source's station while a source holds it.
    pub channel: String,
    pub title: String,
    /// The episode's own title, when there is one.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub subtitle: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    pub start: i64,
    pub end: i64,
    /// `TV-14 DL`: the rating and its descriptors, empty when there is none.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub rating: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub captions: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub described: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl Programme {
    pub fn minutes(&self) -> i64 {
        (self.end - self.start) / 60_000
    }

    /// Whether the programme is on the air at `now`.
    pub fn airing(&self, now_ms: i64) -> bool {
        self.start <= now_ms && now_ms < self.end
    }

    /// How much of the programme has been broadcast at `now`, 0 to 1.
    pub fn progress(&self, now_ms: i64) -> f32 {
        let span = (self.end - self.start).max(1) as f32;
        ((now_ms - self.start) as f32 / span).clamp(0.0, 1.0)
    }

    /// Whether the programme is on the air at any time in `[from, to)`.
    pub fn overlaps(&self, from: i64, to: i64) -> bool {
        self.start < to && from < self.end
    }
}

/// `2.1`, `2-1` or `2_1` as `(2, 1)`; a bare `5` is `(5, 0)`.
pub fn parse_number(text: &str) -> Option<(u16, u16)> {
    let mut parts = text.trim().split(['.', '-', '_']);
    let major = parts.next()?.trim().parse().ok()?;
    let minor = match parts.next() {
        Some(m) => m.trim().parse().ok()?,
        None => 0,
    };
    parts.next().is_none().then_some((major, minor))
}

/// The rating as people read it: `TV14` and `TV-14` are both `TV-14`.
pub fn tidy_rating(code: &str) -> String {
    let code = code.trim();
    match code.strip_prefix("TV") {
        Some(rest) if !rest.is_empty() && !rest.starts_with('-') => format!("TV-{rest}"),
        _ => code.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_read_with_any_separator() {
        assert_eq!(parse_number("2.1"), Some((2, 1)));
        assert_eq!(parse_number("11-11"), Some((11, 11)));
        assert_eq!(parse_number("36"), Some((36, 0)));
        assert_eq!(parse_number("2.1.3"), None);
        assert_eq!(parse_number("ABC"), None);
    }

    #[test]
    fn ratings_are_written_with_their_dash() {
        assert_eq!(tidy_rating("TV14"), "TV-14");
        assert_eq!(tidy_rating("TVPG"), "TV-PG");
        assert_eq!(tidy_rating("TV-MA"), "TV-MA");
        assert_eq!(tidy_rating("PG-13"), "PG-13");
    }

    #[test]
    fn an_empty_field_is_left_out_of_the_json_and_read_back_as_empty() {
        let p = Programme {
            id: "EP1".into(),
            channel: "1_32_2_1_0_0_0".into(),
            title: "News".into(),
            subtitle: String::new(),
            text: String::new(),
            start: 0,
            end: 60_000,
            rating: String::new(),
            captions: true,
            described: false,
        };
        let text = serde_json::to_string(&p).unwrap();
        assert_eq!(
            text,
            r#"{"id":"EP1","channel":"1_32_2_1_0_0_0","title":"News","start":0,"end":60000,"captions":true}"#
        );
        assert_eq!(serde_json::from_str::<Programme>(&text).unwrap(), p);
    }
}
