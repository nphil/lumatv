//! The format badge's model: what the picture in front is (its size and dynamic range, from videooutputd's MAIN sink, subscribed) and
//! the formats LG announces for it (the toasts of its sound and picture services), shown as marks for a few seconds when a picture starts.
//! LG's announcements show here and never as toasts.

use super::toast::{FEED_TAG, plain_lines};
use crate::kit::screen::{Effect, Input, Model};
use crate::panels::quick::info;
use serde_json::{Value, json};

/// The tag the status subscription's answers come back with.
pub const STATUS_TAG: u64 = 0x7042;
pub const STATUS_URI: &str = "luna://com.webos.service.videooutput/getStatus";
/// How long the badge stays up after its last news.
pub const SHOW_MS: u64 = 5000;
/// How long after a badge appears an announcement joins it: audiod's Dolby toast comes about a second after its picture starts.
pub const JOIN_MS: u64 = 4000;

/// The services whose toasts announce a format.
const ANNOUNCERS: [&str; 2] = ["com.webos.service.audio", "com.webos.service.pqcontroller"];
/// What may follow a format's name on its line (audiod's strings: "Dolby Atmos On Earbuds", "... via sound bar", the small "via Audio Device").
const QUALIFIERS: [&str; 4] = ["on earbuds", "for harmony", "via sound bar", "via audio device"];
/// The names LG announces, as audiod and pqcontroller write them, longest first where one starts another.
const NAMES: [(&str, &str); 9] = [
    ("dolby vision", "dolby-vision"),
    ("dolby atmos", "atmos"),
    ("hlg hdr", "hlg"),
    ("hlg", "hlg"),
    ("hdr10+", "hdr10plus"),
    ("hdr10 plus", "hdr10plus"),
    ("hdr10", "hdr10"),
    ("advanced hdr by technicolor", "hdr"),
    ("hdr", "hdr"),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Badge {
    /// A new showing; a badge whose marks change while it is up keeps its id.
    pub id: u64,
    /// Its marks (`kit::logo::VIDEO_MARKS`) in order: the size, the dynamic range, the sound.
    pub marks: Vec<&'static str>,
    pub since: u64,
    pub expires: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Formats {
    subscribed: bool,
    /// Whether a picture is on the MAIN sink, and the context of the last one shown: a repeat of its status is not news.
    present: bool,
    shown: Option<String>,
    size: Option<&'static str>,
    range: Option<&'static str>,
    /// The range is LG's announcement's, which the status's does not override (videooutputd may still say `none`).
    range_announced: bool,
    sound: Option<&'static str>,
    current: Option<Badge>,
    serial: u64,
    changed: bool,
}

/// The mark of a picture `width` x `height`: 4K from 2160 lines (or 3840 columns), FHD from 1080, HD from 720, none below.
fn size_mark(width: u64, height: u64) -> Option<&'static str> {
    if height >= 2160 || width >= 3840 {
        Some("uhd")
    } else if height >= 1080 || width >= 1920 {
        Some("fhd")
    } else if height >= 720 || width >= 1280 {
        Some("hd")
    } else {
        None
    }
}

/// The mark of videooutputd's `hdrType` (`none`, `HDR10`, `HLG`, `DolbyVision`; a name it may use that is none of them is HDR).
fn range_mark(hdr: &str) -> &'static str {
    let h: String = hdr.to_ascii_lowercase().chars().filter(|c| !matches!(c, ' ' | '_' | '-')).collect();
    if h.contains("dolby") || h.contains("vision") {
        "dolby-vision"
    } else if h.contains("hdr10plus") || h.contains("hdr10+") {
        "hdr10plus"
    } else if h.contains("hdr10") {
        "hdr10"
    } else if h.contains("hlg") {
        "hlg"
    } else if h == "none" || h == "sdr" {
        "sdr"
    } else {
        "hdr"
    }
}

/// LumaTV's own players (the wallpaper `org.nphil.lumatv.vplane`, the camera views) and Home, as the compositor's `vwin` tells them.
fn is_own_app(app: &str) -> bool {
    app.starts_with("org.nphil.lumatv") || app == "com.webos.app.home"
}

/// What one line of an announcement names: `Some(None)` for a line that only qualifies the one before ("On Earbuds"), none for a line
/// that is not a format's.
fn line_mark(line: &str) -> Option<Option<&'static str>> {
    let l = line.trim().to_lowercase();
    if QUALIFIERS.contains(&l.as_str()) {
        return Some(None);
    }
    NAMES.iter().find_map(|(name, mark)| {
        let rest = l.strip_prefix(name)?.trim();
        (rest.is_empty() || QUALIFIERS.contains(&rest)).then_some(Some(*mark))
    })
}

/// The marks LG's toast `m` announces: a toast of its sound or picture service (or with their toast icon) whose every line names a format
/// (`Dolby Vision<br>Dolby Atmos<br><font size="1">via Audio Device</font>`, `HLG HDR`). None for any other toast, LG's advice included
/// ("Turn Dolby Atmos on for the best experience with Dolby Atmos content.").
pub fn announced(m: &Value) -> Option<Vec<&'static str>> {
    let source = m.get("sourceId").and_then(Value::as_str).unwrap_or("");
    let icon = m.get("iconPath").and_then(Value::as_str).unwrap_or("");
    if !ANNOUNCERS.contains(&source) && !icon.rsplit('/').next().is_some_and(|f| f.starts_with("light_toast_icon_")) {
        return None;
    }
    let mut marks = Vec::new();
    for line in plain_lines(m.get("message")?.as_str()?) {
        if let Some(mark) = line_mark(&line)? {
            marks.push(mark);
        }
    }
    (!marks.is_empty()).then_some(marks)
}

impl Formats {
    pub fn new() -> Self {
        Self::default()
    }

    /// The subscription to videooutputd's status, asked once at the start.
    pub fn subscribe(&mut self) -> Vec<Effect> {
        if self.subscribed {
            return Vec::new();
        }
        self.subscribed = true;
        vec![Effect::Luna { tag: STATUS_TAG, uri: STATUS_URI.into(), payload: json!({"subscribe": true}), subscribe: true }]
    }

    /// `ctl shell show toast format:MARKS`: a badge of these marks, to see it without a film.
    pub fn show_marks(&mut self, marks: Vec<&'static str>, now: u64) {
        self.serial += 1;
        self.current = Some(Badge { id: self.serial, marks, since: now, expires: now + SHOW_MS });
    }

    /// The state for the compositor's gate (`ctl shell`'s `ui.format`): how many marks are up.
    pub fn state(&self) -> Value {
        json!(self.current.as_ref().map_or(0, |b| b.marks.len()))
    }

    fn marks(&self) -> Vec<&'static str> {
        [self.size, self.range, self.sound].into_iter().flatten().collect()
    }

    fn up(&self, now: u64) -> bool {
        self.current.as_ref().is_some_and(|b| b.expires > now)
    }

    /// Shows the marks: a new badge (`fresh`), or the one that is up with its marks changed and its time renewed.
    fn show(&mut self, now: u64, fresh: bool) {
        let marks = self.marks();
        if marks.is_empty() {
            return;
        }
        match &mut self.current {
            Some(b) if !fresh && b.expires > now => {
                b.marks = marks;
                b.expires = b.expires.max(now + SHOW_MS);
            }
            _ => {
                self.serial += 1;
                self.current = Some(Badge { id: self.serial, marks, since: now, expires: now + SHOW_MS });
            }
        }
        self.changed = true;
    }

    fn status(&mut self, doc: &Value, now: u64) {
        let Some(v) = info::video(doc).filter(|v| !is_own_app(&v.app)) else {
            self.present = false;
            return;
        };
        self.present = true;
        let size = v.width.zip(v.height).and_then(|(w, h)| size_mark(w, h));
        let range = v.hdr.as_deref().map(range_mark);
        if self.shown.as_deref() != Some(v.context.as_str()) {
            // a new picture, shown once its size is known
            if size.is_none() {
                return;
            }
            self.shown = Some(v.context);
            (self.size, self.range, self.range_announced, self.sound) = (size, range, false, None);
            self.show(now, true);
            return;
        }
        // the same picture: a stream's ramp to another size, or its range known late, changes the badge while it is up
        let before = self.marks();
        self.size = size.or(self.size);
        if !self.range_announced {
            self.range = range.or(self.range);
        }
        if self.up(now) && self.marks() != before {
            self.show(now, false);
        }
    }

    fn announce(&mut self, marks: &[&'static str], now: u64) {
        let joins = self.current.as_ref().is_some_and(|b| b.expires > now && now < b.since + JOIN_MS);
        if !joins {
            // news of its own (the range switched mid-film): with the size of the picture that is on
            self.sound = None;
            if !self.present {
                (self.size, self.range) = (None, None);
            }
        }
        for &m in marks {
            if m == "atmos" {
                self.sound = Some(m);
            } else {
                (self.range, self.range_announced) = (Some(m), true);
            }
        }
        self.show(now, !joins);
    }
}

impl Model for Formats {
    type View = Option<Badge>;

    fn update(&mut self, input: &Input, now: u64) -> Vec<Effect> {
        self.changed = false;
        match input {
            Input::Luna { tag, payload } if *tag == STATUS_TAG => self.status(payload, now),
            Input::Luna { tag, payload } if *tag == FEED_TAG => {
                if let Some(marks) = announced(payload) {
                    self.announce(&marks, now);
                }
            }
            Input::Tick if self.current.as_ref().is_some_and(|b| b.expires <= now) => {
                self.current = None;
                self.changed = true;
            }
            _ => {}
        }
        if self.changed { vec![Effect::Redraw] } else { Vec::new() }
    }

    fn view(&self) -> Option<Badge> {
        self.current.clone()
    }

    fn due_at(&self) -> Option<u64> {
        self.current.as_ref().map(|b| b.expires)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(context: &str, app: &str, w: u64, h: u64, hdr: &str) -> Input {
        let main = json!({"sink": "MAIN", "connected": true, "context": context, "appId": app, "width": w, "height": h, "frameRate": 24,
            "videoInfo": {"hdrType": hdr}});
        Input::Luna { tag: STATUS_TAG, payload: json!({"returnValue": true, "video": [main, {"sink": "SUB0", "connected": false}]}) }
    }

    fn toast(source: &str, message: &str) -> Input {
        Input::Luna { tag: FEED_TAG, payload: json!({"sourceId": source, "message": message}) }
    }

    fn marks(f: &Formats) -> Vec<&'static str> {
        f.view().map(|b| b.marks).unwrap_or_default()
    }

    const DOLBY: &str = "Dolby Vision<br>Dolby Atmos<br><font size=\"1\">via Audio Device</font>";

    #[test]
    fn a_picture_shows_its_size_and_range_once_and_lgs_announcement_joins_it() {
        let mut f = Formats::new();
        assert!(matches!(&f.subscribe()[0], Effect::Luna { tag: STATUS_TAG, uri, subscribe: true, .. } if uri == STATUS_URI));
        assert!(f.subscribe().is_empty());
        assert_eq!(f.update(&status("film", "netflix", 3840, 2160, "DolbyVision"), 1000), vec![Effect::Redraw]);
        assert_eq!(marks(&f), ["uhd", "dolby-vision"]);
        let id = f.view().unwrap().id;
        assert!(f.update(&status("film", "netflix", 3840, 2160, "DolbyVision"), 1500).is_empty(), "a repeat is no news");
        f.update(&toast("com.webos.service.audio", DOLBY), 2200);
        assert_eq!(marks(&f), ["uhd", "dolby-vision", "atmos"]);
        assert_eq!(f.view().unwrap().id, id, "it joins the badge that is up");
        assert_eq!(f.due_at(), Some(2200 + SHOW_MS));
        assert!(f.update(&Input::Tick, 2200 + SHOW_MS - 1).is_empty());
        assert_eq!(f.update(&Input::Tick, 2200 + SHOW_MS), vec![Effect::Redraw]);
        assert_eq!(f.view(), None);
        assert!(f.update(&status("film", "netflix", 3840, 2160, "DolbyVision"), 9000).is_empty(), "the same film is not shown again");
        f.update(&status("next", "netflix", 1920, 1080, "none"), 10_000);
        assert_eq!(marks(&f), ["fhd", "sdr"]);
        assert_ne!(f.view().unwrap().id, id);
    }

    #[test]
    fn a_streams_ramp_changes_the_badge_that_is_up_and_lumatvs_own_pictures_are_not_shown() {
        let mut f = Formats::new();
        f.update(&status("own", "org.nphil.lumatv.vplane", 3840, 2160, "none"), 0);
        f.update(&status("cam", "com.webos.app.home", 1920, 1080, "none"), 0);
        assert_eq!(f.view(), None);
        f.update(&status("yt", "youtube.leanback.v4", 0, 0, "none"), 100);
        assert_eq!(f.view(), None, "shown once its size is known");
        f.update(&status("yt", "youtube.leanback.v4", 1280, 720, "none"), 200);
        let id = f.view().unwrap().id;
        f.update(&status("yt", "youtube.leanback.v4", 3840, 2160, "HDR10"), 1200);
        assert_eq!(marks(&f), ["uhd", "hdr10"]);
        assert_eq!(f.view().unwrap().id, id);
        f.update(&Input::Tick, 1200 + SHOW_MS);
        f.update(&status("yt", "youtube.leanback.v4", 1920, 1080, "HDR10"), 7000);
        assert_eq!(f.view(), None, "a ramp after the badge went is not shown");
    }

    #[test]
    fn an_announcement_on_its_own_shows_with_the_size_of_the_picture_that_is_on() {
        let mut f = Formats::new();
        f.update(&status("tv", "com.webos.app.livetv", 3840, 2160, "none"), 0);
        f.update(&Input::Tick, SHOW_MS);
        f.update(&toast("com.webos.service.pqcontroller", "HLG HDR"), 60_000);
        assert_eq!(marks(&f), ["uhd", "hlg"]);
        f.update(&status("tv", "com.webos.app.livetv", 3840, 2160, "none"), 60_500);
        assert_eq!(marks(&f), ["uhd", "hlg"], "videooutputd's late `none` does not undo LG's word");
        f.update(&Input::Luna { tag: STATUS_TAG, payload: json!({"returnValue": true, "video": []}) }, 70_000);
        f.update(&toast("com.webos.service.audio", "Dolby Atmos<br>On Earbuds"), 80_000);
        assert_eq!(marks(&f), ["atmos"], "no picture: the announcement alone");
    }

    #[test]
    fn only_lgs_format_toasts_are_announcements() {
        let lg = |source: &str, message: &str| announced(&json!({"sourceId": source, "message": message}));
        assert_eq!(lg("com.webos.service.audio", DOLBY), Some(vec!["dolby-vision", "atmos"]));
        assert_eq!(lg("com.webos.service.pqcontroller", "Advanced HDR by technicolor"), Some(vec!["hdr"]));
        assert_eq!(lg("com.webos.service.pqcontroller", "HDR10+"), Some(vec!["hdr10plus"]));
        assert_eq!(lg("com.webos.service.audio", "Dolby Atmos via sound bar"), Some(vec!["atmos"]));
        assert_eq!(lg("com.webos.service.audio", "Turn Dolby Atmos on for the best experience with Dolby Atmos content."), None);
        assert_eq!(lg("com.webos.service.audio", "Dolby Vision and Dolby Atmos content detected. Turn Dolby Atmos on for the best sound experience."), None);
        assert_eq!(lg("com.example.app", "Dolby Vision"), None);
        let by_icon = json!({"message": "Dolby Vision", "iconPath": "/usr/share/pqcontroller/images/light_toast_icon_hdr.png"});
        assert_eq!(announced(&by_icon), Some(vec!["dolby-vision"]));
        assert_eq!(range_mark("HDR10_PLUS"), "hdr10plus");
        assert_eq!(range_mark("technicolor"), "hdr");
        assert_eq!((size_mark(1280, 536), size_mark(720, 576)), (Some("hd"), None));
    }
}
