//! HLS playlists as Home Assistant's stream integration serves them: a master playlist naming one variant, and a media playlist of
//! fragmented MP4 segments with an `EXT-X-MAP` initialisation segment, a media sequence number, and (with low-latency HLS on, Home
//! Assistant's default) the parts of the segment being written (`EXT-X-PART`) and the server's leave to block a reload until the next
//! part exists (`EXT-X-SERVER-CONTROL`, `_HLS_msn` and `_HLS_part`).
//!
//! What matters for a live picture: the newest pieces, in order, each once. [`Playlist::live_edge`] says where to begin (the parts of the
//! last segment, or the last whole segment, so the first picture is near now), [`Playlist::after`] what is new since a point, and
//! [`Playlist::next_reload`] how to ask for the next playlist (blocking on the next part when the server allows it, else after a pause).

/// One piece of media to fetch: a whole segment or a part of one.
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    /// The media sequence number of the segment this belongs to.
    pub msn: u64,
    /// The part's index within the segment, `None` for a whole segment.
    pub part: Option<u32>,
    pub uri: String,
    pub duration: f64,
    /// A part that starts with a key picture (a whole segment always does).
    pub independent: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Playlist {
    pub target_duration: f64,
    pub part_target: Option<f64>,
    pub media_sequence: u64,
    pub init_uri: Option<String>,
    /// Whole segments, oldest first.
    pub segments: Vec<Piece>,
    /// The parts of segments not yet complete (and of the last complete ones), oldest first.
    pub parts: Vec<Piece>,
    pub can_block_reload: bool,
    pub ended: bool,
}

/// A point in the stream: everything up to and including it has been fetched.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Position {
    pub msn: u64,
    /// The last part fetched of `msn`, or `None` when the whole segment was.
    pub part: Option<u32>,
}

fn attribute<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let mut rest = line;
    while let Some(at) = rest.find(name) {
        let after = &rest[at + name.len()..];
        let preceded = at == 0
            || !rest.as_bytes()[at - 1].is_ascii_alphanumeric() && rest.as_bytes()[at - 1] != b'-';
        if preceded && let Some(value) = after.strip_prefix('=') {
            return Some(if let Some(quoted) = value.strip_prefix('"') {
                quoted.split('"').next().unwrap_or("")
            } else {
                value.split(',').next().unwrap_or("")
            });
        }
        rest = after;
    }
    None
}

/// The variant of a master playlist: the first media playlist it names (Home Assistant names one).
pub fn parse_master(text: &str) -> Option<String> {
    let mut lines = text.lines().map(str::trim);
    while let Some(line) = lines.next() {
        if line.starts_with("#EXT-X-STREAM-INF") {
            for next in lines.by_ref() {
                if !next.is_empty() && !next.starts_with('#') {
                    return Some(next.to_owned());
                }
            }
        }
    }
    None
}

/// Reads a media playlist. A playlist that is not one (no `#EXTM3U`, no segments and no parts) is `None`.
pub fn parse_media(text: &str) -> Option<Playlist> {
    if !text.trim_start().starts_with("#EXTM3U") || text.contains("#EXT-X-STREAM-INF") {
        return None;
    }
    let mut p = Playlist::default();
    let mut msn = 0u64;
    let mut have_sequence = false;
    let mut next_duration = 0.0f64;
    let mut part_index = 0u32;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(v) = line.strip_prefix("#EXT-X-TARGETDURATION:") {
            p.target_duration = v.trim().parse().unwrap_or(0.0);
        } else if let Some(v) = line.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
            msn = v.trim().parse().unwrap_or(0);
            p.media_sequence = msn;
            have_sequence = true;
        } else if line.starts_with("#EXT-X-PART-INF") {
            p.part_target = attribute(line, "PART-TARGET").and_then(|v| v.parse().ok());
        } else if line.starts_with("#EXT-X-SERVER-CONTROL") {
            p.can_block_reload = attribute(line, "CAN-BLOCK-RELOAD") == Some("YES");
        } else if line.starts_with("#EXT-X-MAP") {
            p.init_uri = attribute(line, "URI").map(str::to_owned);
        } else if let Some(v) = line.strip_prefix("#EXTINF:") {
            next_duration = v
                .split(',')
                .next()
                .unwrap_or("0")
                .trim()
                .parse()
                .unwrap_or(0.0);
        } else if line.starts_with("#EXT-X-PART:") {
            if let Some(uri) = attribute(line, "URI") {
                p.parts.push(Piece {
                    msn,
                    part: Some(part_index),
                    uri: uri.to_owned(),
                    duration: attribute(line, "DURATION")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0.0),
                    independent: attribute(line, "INDEPENDENT") == Some("YES"),
                });
                part_index += 1;
            }
        } else if line == "#EXT-X-ENDLIST" {
            p.ended = true;
        } else if !line.starts_with('#') {
            p.segments.push(Piece {
                msn,
                part: None,
                uri: line.to_owned(),
                duration: next_duration,
                independent: true,
            });
            msn += 1;
            part_index = 0;
            next_duration = 0.0;
        }
    }
    let _ = have_sequence;
    if p.segments.is_empty() && p.parts.is_empty() {
        return None;
    }
    Some(p)
}

impl Playlist {
    /// The pieces to begin with, so that the first picture is close to now: with parts, the parts of the newest segment from its last
    /// independent part on (and the whole segments before none); without, the last whole segment.
    pub fn live_edge(&self) -> Vec<Piece> {
        if let Some(last) = self.parts.last() {
            let same: Vec<&Piece> = self.parts.iter().filter(|p| p.msn == last.msn).collect();
            let from = same.iter().rposition(|p| p.independent).unwrap_or(0);
            return same[from..].iter().map(|p| (*p).clone()).collect();
        }
        self.segments.last().cloned().into_iter().collect()
    }

    /// What is new after `at`, in order: parts of a segment that is being written, whole segments once they are complete. A segment
    /// whose parts were all fetched is not fetched again whole.
    pub fn after(&self, at: Position) -> Vec<Piece> {
        let mut out = Vec::new();
        let complete: Vec<&Piece> = self.segments.iter().filter(|s| s.msn > at.msn).collect();
        if self.parts.is_empty() {
            return complete.into_iter().cloned().collect();
        }
        // segments completed since `at` whose parts were never seen (the parts list only holds the newest ones)
        let oldest_part_msn = self.parts.first().map_or(u64::MAX, |p| p.msn);
        for s in complete.iter().filter(|s| s.msn < oldest_part_msn) {
            out.push((*s).clone());
        }
        for p in &self.parts {
            let newer = match (p.msn.cmp(&at.msn), p.part, at.part) {
                (std::cmp::Ordering::Greater, _, _) => true,
                (std::cmp::Ordering::Equal, Some(i), Some(j)) => i > j,
                (std::cmp::Ordering::Equal, Some(_), None) => false,
                _ => false,
            };
            if newer {
                out.push(p.clone());
            }
        }
        out
    }

    /// The next position after fetching `piece`.
    pub fn position_after(piece: &Piece) -> Position {
        Position {
            msn: piece.msn,
            part: piece.part,
        }
    }

    /// How to ask for the next playlist: a blocking reload for the next part (or segment) when the server allows it, else a pause
    /// of half the target duration (a quarter of the part target with parts).
    pub fn next_reload(&self, at: Position) -> Reload {
        if self.can_block_reload {
            let (msn, part) = match (at.part, self.part_target) {
                (Some(i), Some(_)) => (at.msn, Some(i + 1)),
                (None, Some(_)) => (at.msn + 1, Some(0)),
                _ => (at.msn + 1, None),
            };
            return Reload::Block { msn, part };
        }
        let pause = match self.part_target {
            Some(t) if t > 0.0 => t / 4.0,
            _ => (self.target_duration / 2.0).max(0.5),
        };
        Reload::After(std::time::Duration::from_secs_f64(pause.clamp(0.1, 10.0)))
    }

    /// The segment a reload should say it is expecting to make the asked part exist (Home Assistant's last-part rule: a part asked past
    /// the segment's end rolls over to the next segment's first).
    pub fn last_msn(&self) -> u64 {
        self.segments
            .last()
            .map(|s| s.msn)
            .max(self.parts.last().map(|p| p.msn))
            .unwrap_or(self.media_sequence)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reload {
    /// `?_HLS_msn=<msn>&_HLS_part=<part>`: the server answers when that part (or segment) exists.
    Block {
        msn: u64,
        part: Option<u32>,
    },
    After(std::time::Duration),
}

impl Reload {
    pub fn query(&self) -> String {
        match self {
            Self::Block { msn, part: Some(p) } => format!("_HLS_msn={msn}&_HLS_part={p}"),
            Self::Block { msn, part: None } => format!("_HLS_msn={msn}"),
            Self::After(_) => String::new(),
        }
    }
}

/// Joins a playlist's relative URI to the playlist's own URL.
pub fn resolve(base: &str, uri: &str) -> String {
    if uri.contains("://") {
        return uri.to_owned();
    }
    if let Some(rest) = uri.strip_prefix('/') {
        let after_scheme = base.find("://").map_or(0, |i| i + 3);
        let origin_end = base[after_scheme..]
            .find('/')
            .map_or(base.len(), |i| after_scheme + i);
        return format!("{}/{rest}", &base[..origin_end]);
    }
    let path_end = base.find('?').unwrap_or(base.len());
    let dir_end = base[..path_end].rfind('/').map_or(path_end, |i| i + 1);
    format!("{}{uri}", &base[..dir_end])
}

#[cfg(test)]
mod tests {
    use super::*;

    const VOD: &str = "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:1\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:1.000000,\nseg0.m4s\n#EXTINF:1.000000,\nseg1.m4s\n#EXTINF:1.000000,\nseg2.m4s\n#EXT-X-ENDLIST\n";
    // as Home Assistant writes it with low-latency HLS: whole segments, then the parts of the one being written
    const LL: &str = "#EXTM3U\n#EXT-X-VERSION:9\n#EXT-X-TARGETDURATION:6\n#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,PART-HOLD-BACK=3.0\n#EXT-X-PART-INF:PART-TARGET=1.0\n#EXT-X-MEDIA-SEQUENCE:40\n#EXT-X-MAP:URI=\"./init.mp4\"\n#EXTINF:6.0,\n./segment/40.m4s\n#EXTINF:6.0,\n./segment/41.m4s\n#EXT-X-PART:DURATION=1.0,URI=\"./segment/42.0.m4s\",INDEPENDENT=YES\n#EXT-X-PART:DURATION=1.0,URI=\"./segment/42.1.m4s\"\n#EXT-X-PART:DURATION=1.0,URI=\"./segment/42.2.m4s\"\n";

    #[test]
    fn a_plain_playlist_begins_at_its_last_segment_and_reloads_after_a_pause() {
        let p = parse_media(VOD).unwrap();
        assert_eq!(
            (
                p.target_duration,
                p.media_sequence,
                p.init_uri.as_deref(),
                p.ended
            ),
            (1.0, 0, Some("init.mp4"), true)
        );
        assert_eq!(
            p.segments
                .iter()
                .map(|s| (s.msn, s.uri.as_str()))
                .collect::<Vec<_>>(),
            [(0, "seg0.m4s"), (1, "seg1.m4s"), (2, "seg2.m4s")]
        );
        let edge = p.live_edge();
        assert_eq!(edge.len(), 1);
        assert_eq!(edge[0].uri, "seg2.m4s");
        assert_eq!(
            p.after(Position { msn: 0, part: None })
                .iter()
                .map(|s| s.msn)
                .collect::<Vec<_>>(),
            [1, 2]
        );
        assert_eq!(
            p.next_reload(Position { msn: 2, part: None }),
            Reload::After(std::time::Duration::from_millis(500))
        );
        assert!(parse_media("not a playlist").is_none());
        assert!(
            parse_media("#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1\nplaylist.m3u8\n").is_none(),
            "a master is not a media playlist"
        );
        assert_eq!(
            parse_master("#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1\nplaylist.m3u8\n").as_deref(),
            Some("playlist.m3u8")
        );
    }

    #[test]
    fn a_low_latency_playlist_begins_at_the_last_independent_part_and_blocks_on_the_next() {
        let p = parse_media(LL).unwrap();
        assert_eq!(
            (p.part_target, p.can_block_reload, p.media_sequence),
            (Some(1.0), true, 40)
        );
        let edge = p.live_edge();
        assert_eq!(
            edge.iter()
                .map(|x| (x.msn, x.part, x.independent))
                .collect::<Vec<_>>(),
            [
                (42, Some(0), true),
                (42, Some(1), false),
                (42, Some(2), false)
            ]
        );
        let at = Playlist::position_after(edge.last().unwrap());
        assert_eq!(
            at,
            Position {
                msn: 42,
                part: Some(2)
            }
        );
        assert!(p.after(at).is_empty());
        let reload = p.next_reload(at);
        assert_eq!(
            reload,
            Reload::Block {
                msn: 42,
                part: Some(3)
            }
        );
        assert_eq!(reload.query(), "_HLS_msn=42&_HLS_part=3");
        // the next playlist: segment 42 complete, parts of 43
        let next = LL.replace(
            "#EXT-X-PART:DURATION=1.0,URI=\"./segment/42.0.m4s\",INDEPENDENT=YES\n#EXT-X-PART:DURATION=1.0,URI=\"./segment/42.1.m4s\"\n#EXT-X-PART:DURATION=1.0,URI=\"./segment/42.2.m4s\"\n",
            "#EXTINF:6.0,\n./segment/42.m4s\n#EXT-X-PART:DURATION=1.0,URI=\"./segment/43.0.m4s\",INDEPENDENT=YES\n",
        );
        let p2 = parse_media(&next).unwrap();
        let new = p2.after(at);
        assert_eq!(
            new.iter().map(|x| (x.msn, x.part)).collect::<Vec<_>>(),
            [(43, Some(0))],
            "segment 42 was had as parts: not fetched again"
        );
        assert_eq!(
            resolve(
                "http://ha:8123/api/hls/tok/playlist.m3u8",
                "./segment/43.0.m4s"
            ),
            "http://ha:8123/api/hls/tok/./segment/43.0.m4s"
        );
        assert_eq!(
            resolve("http://ha:8123/api/hls/tok/playlist.m3u8?x=1", "init.mp4"),
            "http://ha:8123/api/hls/tok/init.mp4"
        );
        assert_eq!(
            resolve("http://ha:8123/api/hls/tok/playlist.m3u8", "/api/other"),
            "http://ha:8123/api/other"
        );
    }
}
