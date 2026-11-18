//! The format badge's model: what the picture in front is (its size, refresh and dynamic range, from videooutputd's MAIN sink, subscribed),
//! the sound an HDMI, DisplayPort or USB-C input carries (utp.extinputs, as LG's INFO card reads it), the sound an app that decodes for
//! itself says it delivers (the helper's `media.audio` topic) and the formats LG announces for it (the toasts of its sound and picture
//! services), shown as marks for a few seconds when a picture starts or its format changes. LG's announcements show here and never as toasts.

use super::banner::is_input_app;
use super::toast::{FEED_TAG, plain_lines};
use crate::host::channels::LIVE_TV_APP;
use crate::kit::screen::{Effect, Input, Model};
use crate::panels::quick::info;
use serde_json::{Value, json};

/// The tag the status subscription's answers come back with.
pub const STATUS_TAG: u64 = 0x7042;
pub const STATUS_URI: &str = "luna://com.webos.service.videooutput/getStatus";
/// The input's sound: one subscription per picture, each with its own tag from this one, so a late answer of the one before is not taken.
const AUDIO_TAG_BASE: u64 = 0x7043_0000;
const AUDIO_URI: &str = "luna://com.webos.service.utp.extinputs/getCurrentAudioCodecType";
/// The helper's topic of what apps that decode their own sound deliver (`docs/media-audio.md`).
const APP_AUDIO_TOPIC: &str = "media.audio";
/// How long the badge stays up after its last news.
pub const SHOW_MS: u64 = 3000;
/// How long after a badge appears an announcement joins it: audiod's Dolby toast comes about a second after its picture starts.
pub const JOIN_MS: u64 = 3000;

/// The services whose toasts announce a format.
const ANNOUNCERS: [&str; 2] = ["com.webos.service.audio", "com.webos.service.pqcontroller"];
/// What may follow a format's name on its line (audiod's strings: "Dolby Atmos On Earbuds", "... via sound bar", the small "via Audio Device").
const QUALIFIERS: [&str; 4] = ["on earbuds", "for harmony", "via sound bar", "via audio device"];
/// The speaker layouts that may follow a sound's name ("Dolby Digital 5.1") or stand on their own line; each is its own mark.
const CHANNELS: [&str; 2] = ["5.1", "7.1"];
/// The names LG announces, as audiod and pqcontroller write them, and the marks they are.
const NAMES: [(&str, &str); 22] = [
    ("dolby vision", "dolby-vision"),
    ("dolby atmos", "atmos"),
    ("dolby digital plus", "dolby-digital-plus"),
    ("dolby digital+", "dolby-digital-plus"),
    ("dolby digital", "dolby-digital"),
    ("dolby truehd", "dolby-truehd"),
    ("dolby audio", "dolby-audio"),
    ("dts:x", "dtsx"),
    ("dts-x", "dtsx"),
    ("dts-hd", "dts-hd"),
    ("dts hd", "dts-hd"),
    ("dts", "dts"),
    ("pcm", "pcm"),
    ("hlg hdr", "hlg"),
    ("hlg", "hlg"),
    ("hdr10+", "hdr10plus"),
    ("hdr10 plus", "hdr10plus"),
    ("hdr10", "hdr10"),
    ("advanced hdr by technicolor", "hdr"),
    ("hdr", "hdr"),
    ("5.1", "5.1"),
    ("7.1", "7.1"),
];

/// What an app says it delivers (one entry of the `media.audio` topic): only what the badge reads.
#[derive(Clone, Debug, PartialEq, Eq)]
struct AppAudio {
    app: String,
    /// videooutputd's context of the picture it belongs to; none: whichever picture the app has in front.
    context: Option<String>,
    codec: String,
    profile: String,
    atmos: bool,
    channels: Option<u64>,
}

/// The sound mark and the speakers mark an app's entry makes: none for a format that has no mark (AAC, Opus, FLAC, MP3).
type AppSound = (Option<&'static str>, Option<&'static str>);

fn app_entries(topic: &Value) -> Vec<AppAudio> {
    let text = |e: &Value, k: &str| e.get(k).and_then(Value::as_str).map(str::to_owned);
    topic
        .get("entries")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|e| {
            Some(AppAudio {
                app: text(e, "app")?,
                context: text(e, "context"),
                codec: text(e, "codec")?,
                profile: text(e, "profile").unwrap_or_default().to_ascii_lowercase(),
                atmos: e.get("atmos").and_then(Value::as_bool).unwrap_or(false),
                channels: e.get("channels").and_then(Value::as_u64),
            })
        })
        .collect()
}

/// The marks of what an app delivers. Atmos is the mark of any Dolby carrier that has it and DTS-HD that of every DTS-HD profile
/// (Master Audio, High Resolution, Express): the badge never spells the longer name.
fn app_sound(a: &AppAudio) -> AppSound {
    let sound = match a.codec.as_str() {
        "truehd" | "eac3" if a.atmos => Some("atmos"),
        "truehd" => Some("dolby-truehd"),
        "eac3" => Some("dolby-digital-plus"),
        "ac3" => Some("dolby-digital"),
        "dts" if a.profile.contains("dts:x") => Some("dtsx"),
        "dts" if a.profile.starts_with("dts-hd") => Some("dts-hd"),
        "dts" => Some("dts"),
        "pcm" => Some("pcm"),
        _ => None,
    };
    let channels = match a.channels {
        Some(6) => Some("5.1"),
        Some(8) => Some("7.1"),
        _ => None,
    };
    (sound, channels)
}

/// The marks that fit a row, in the order the badge shows them: when the whole row is too wide the speakers go first, then the sound
/// takes its shortest mark (any Dolby Digital family format is "Dolby Audio", the brand's own mark for them). `fits` says whether a row fits.
pub fn fit(marks: &[&'static str], fits: impl Fn(&[&'static str]) -> bool) -> Vec<&'static str> {
    if fits(marks) {
        return marks.to_vec();
    }
    let mut row: Vec<&'static str> = marks.iter().copied().filter(|m| kind_of(m) != Kind::Channels).collect();
    if fits(&row) {
        return row;
    }
    for m in &mut row {
        if matches!(*m, "dolby-digital-plus" | "dolby-digital" | "dolby-truehd") {
            *m = "dolby-audio";
        }
    }
    row
}

/// What a mark says, in the order a badge shows them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Size,
    Refresh,
    Range,
    Sound,
    Channels,
}

fn kind_of(mark: &str) -> Kind {
    match mark {
        "uhd" | "fhd" | "hd" => Kind::Size,
        "120hz" | "144hz" => Kind::Refresh,
        "dolby-vision" | "hdr10plus" | "hdr10" | "hlg" | "hdr" | "sdr" => Kind::Range,
        "5.1" | "7.1" => Kind::Channels,
        _ => Kind::Sound,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Badge {
    /// A new showing; a badge whose marks change while it is up keeps its id.
    pub id: u64,
    /// Its marks (`kit::logo::VIDEO_MARKS`) in order: the size, the refresh, the dynamic range, the sound, the speakers.
    pub marks: Vec<&'static str>,
    pub since: u64,
    pub expires: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Formats {
    subscribed: bool,
    /// The app in front (the `foreground` topic), none on Home or while it is not known.
    foreground: Option<String>,
    /// What apps say they deliver, and the sound of the entry the badge follows: none while no entry is for the picture shown.
    app_audio: Vec<AppAudio>,
    app_sound: Option<AppSound>,
    /// Whether a picture is on the MAIN sink, and the context of the last one shown: a repeat of its status is not news.
    present: bool,
    shown: Option<String>,
    size: Option<&'static str>,
    range: Option<&'static str>,
    /// The range is LG's announcement's, which the status's does not override (videooutputd may still say `none`).
    range_announced: bool,
    sound: Option<&'static str>,
    channels: Option<&'static str>,
    refresh: Option<&'static str>,
    /// The subscription to the input's sound while an input's picture is shown, and the sound it last said.
    audio_tag: Option<u64>,
    input_sound: Option<&'static str>,
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

/// The mark of a high refresh: 120 Hz from 100 frames a second (119.88 included), 144 Hz from 132; none for the usual rates.
fn refresh_mark(fps: f64) -> Option<&'static str> {
    if fps >= 132.0 {
        Some("144hz")
    } else if fps >= 100.0 {
        Some("120hz")
    } else {
        None
    }
}

/// The mark of utp.extinputs' `audioCodecType`, as LG's INFO card names it (`InputDetailBannerController.qml`): Atmos over any Dolby
/// carrier, AC-4 and MAT as Dolby Audio; none for a codec the card has no mark for (AAC, MPEG).
fn codec_mark(codec: &str) -> Option<&'static str> {
    Some(match codec.strip_prefix("AUDIO_CODEC_")? {
        "AC4_ATMOS" | "MAT_ATMOS" | "TRUEHD_ATMOS" | "DOLBY_PLUS_ATMOS" => "atmos",
        "DOLBY_PLUS" => "dolby-digital-plus",
        "AC3_DOLBY" => "dolby-digital",
        "TRUEHD" => "dolby-truehd",
        "AC4" | "MAT" => "dolby-audio",
        "DTS" => "dts",
        "DTS_HD" | "DTS_HD_MA" | "DTS_EXPRESS" => "dts-hd",
        "DTS_X_P1" | "DTS_X_P2" => "dtsx",
        "PCM" => "pcm",
        _ => return None,
    })
}

/// An HDMI, DisplayPort or USB-C input: utp.extinputs knows its sound. Live TV's tuner is not one.
pub(crate) fn is_external_input(app: &str) -> bool {
    is_input_app(app) && app != LIVE_TV_APP
}

/// The mark of videooutputd's `hdrType` (`none`, `HDR10`, `HLG`, `DolbyVision`; a name it may use that is none of them is HDR).
pub(crate) fn range_mark(hdr: &str) -> &'static str {
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

/// What one line of an announcement names: its format and the speakers after it ("Dolby Digital 5.1"), nothing for a line that only
/// qualifies the one before ("On Earbuds"), none for a line that is not a format's.
fn line_marks(line: &str) -> Option<Vec<&'static str>> {
    let l = line.trim().to_lowercase();
    if QUALIFIERS.contains(&l.as_str()) {
        return Some(Vec::new());
    }
    let ends = |rest: &str| rest.is_empty() || QUALIFIERS.contains(&rest);
    NAMES.iter().find_map(|(name, mark)| {
        let rest = l.strip_prefix(name)?.trim();
        if ends(rest) {
            return Some(vec![*mark]);
        }
        CHANNELS.iter().find_map(|c| ends(rest.strip_prefix(c)?.trim()).then(|| vec![*mark, *c]))
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
        marks.extend(line_marks(&line)?);
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
        [self.size, self.refresh, self.range, self.sound, self.channels].into_iter().flatten().collect()
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

    /// The sound the badge has when no app's entry gives it: the input's, if the picture is an input's.
    fn base_sound(&self) -> AppSound {
        self.app_sound.unwrap_or((self.input_sound, None))
    }

    /// What the app in front says it delivers for the picture shown: an input's picture has its own sound, and an entry that names a
    /// context is for that picture alone.
    fn wanted_app_sound(&self) -> Option<AppSound> {
        if !self.present || self.audio_tag.is_some() {
            return None;
        }
        let (shown, front) = (self.shown.as_deref()?, self.foreground.as_deref()?);
        self.app_audio.iter().find(|e| e.app == front && e.context.as_deref().is_none_or(|c| c == shown)).map(app_sound)
    }

    /// Follows the entries, the app in front and the picture: the app's sound joins the badge that is up, and a change of it later is news
    /// of its own, as an input's is.
    fn follow_app_audio(&mut self, now: u64) {
        let wanted = self.wanted_app_sound();
        if wanted == self.app_sound {
            return;
        }
        self.app_sound = wanted;
        let before = self.marks();
        (self.sound, self.channels) = self.base_sound();
        let marked = wanted.is_some_and(|(sound, channels)| sound.is_some() || channels.is_some());
        if self.marks() != before && (marked || self.up(now)) {
            self.show(now, !self.up(now));
        }
    }

    /// Follows the sound of the input whose picture is `pipeline` (videooutputd's context is the input's pipeline), or of none.
    fn follow_audio(&mut self, pipeline: Option<&str>) -> Vec<Effect> {
        let mut out = Vec::new();
        if let Some(tag) = self.audio_tag.take() {
            out.push(Effect::Cancel { tag });
        }
        self.input_sound = None;
        if let Some(pipeline) = pipeline {
            self.serial += 1;
            let tag = AUDIO_TAG_BASE + self.serial;
            self.audio_tag = Some(tag);
            out.push(Effect::Luna { tag, uri: AUDIO_URI.into(), payload: json!({"pipelineId": pipeline, "subscribe": true}), subscribe: true });
        }
        out
    }

    fn status(&mut self, doc: &Value, now: u64) -> Vec<Effect> {
        let Some(v) = info::video(doc).filter(|v| !is_own_app(&v.app)) else {
            (self.present, self.app_sound) = (false, None);
            return if self.audio_tag.is_some() { self.follow_audio(None) } else { Vec::new() };
        };
        self.present = true;
        // videooutputd's first status of a new video is a placeholder 1920x1080 with no frame rate and no video info (Live TV's 704x480 69-6,
        // 2026-11-06): a size counts once the rate or the video info comes with it, and then replaces the one before, none below 720 lines included
        let settled = v.fps.is_some() || v.hdr.is_some();
        let size = v.width.zip(v.height).filter(|_| settled).map(|(w, h)| size_mark(w, h));
        let refresh = v.fps.map(refresh_mark);
        let range = v.hdr.as_deref().map(range_mark);
        let input = is_external_input(&v.app);
        if self.shown.as_deref() != Some(v.context.as_str()) {
            // a new picture, shown once its size has settled
            let Some(size) = size else {
                return Vec::new();
            };
            let effects = self.follow_audio(input.then_some(v.context.as_str()));
            self.shown = Some(v.context);
            (self.size, self.refresh, self.range, self.range_announced, self.sound, self.channels) = (size, refresh.flatten(), range, false, None, None);
            // the app in front says what it delivers for this picture: that sound is the badge's from its first showing
            self.app_sound = self.wanted_app_sound();
            (self.sound, self.channels) = self.base_sound();
            self.show(now, true);
            return effects;
        }
        // the same picture: a stream's ramp to another size, or its range known late, changes the badge while it is up; an input's source
        // switching its signal (a console to 120 Hz, a player to HDR for a film) is news of its own
        let before = self.marks();
        if let Some(size) = size {
            self.size = size;
        }
        self.refresh = refresh.unwrap_or(self.refresh);
        if !self.range_announced {
            self.range = range.or(self.range);
        }
        if self.marks() != before && (input || self.up(now)) {
            self.show(now, !self.up(now));
        }
        // the picture may have gone and come back as the same one: the entry is followed again
        self.follow_app_audio(now);
        Vec::new()
    }

    /// The input's sound, as utp.extinputs says it: it joins the badge that is up, and a change of it later (the source starting a film's
    /// Atmos after its menu's PCM) is news of its own. A repeat is not news.
    fn input_audio(&mut self, doc: &Value, now: u64) {
        let sound = doc.get("audioCodecType").and_then(Value::as_str).and_then(codec_mark);
        if sound == self.input_sound {
            return;
        }
        self.input_sound = sound;
        let before = self.marks();
        self.sound = sound;
        if self.marks() != before && (sound.is_some() || self.up(now)) {
            self.show(now, !self.up(now));
        }
    }

    fn announce(&mut self, marks: &[&'static str], now: u64) {
        let joins = self.current.as_ref().is_some_and(|b| b.expires > now && now < b.since + JOIN_MS);
        if !joins {
            // news of its own (the range switched mid-film): with the size of the picture that is on, and its input's sound
            (self.sound, self.channels) = self.base_sound();
            if !self.present {
                (self.size, self.refresh, self.range) = (None, None, None);
            }
        }
        for &m in marks {
            match kind_of(m) {
                Kind::Size => self.size = Some(m),
                Kind::Refresh => self.refresh = Some(m),
                Kind::Range => (self.range, self.range_announced) = (Some(m), true),
                // what an app says it delivers is what reaches the speakers: LG's own word about the sound does not replace it
                Kind::Sound | Kind::Channels if self.app_sound.is_some() => {}
                Kind::Sound => self.sound = Some(m),
                Kind::Channels => self.channels = Some(m),
            }
        }
        self.show(now, !joins);
    }
}

impl Model for Formats {
    type View = Option<Badge>;

    fn update(&mut self, input: &Input, now: u64) -> Vec<Effect> {
        self.changed = false;
        let mut out = Vec::new();
        match input {
            Input::Luna { tag, payload } if *tag == STATUS_TAG => out = self.status(payload, now),
            Input::Luna { tag, payload } if Some(*tag) == self.audio_tag => self.input_audio(payload, now),
            Input::Luna { tag, payload } if *tag == FEED_TAG => {
                if let Some(marks) = announced(payload) {
                    self.announce(&marks, now);
                }
            }
            Input::Helper { topic, data } if topic == APP_AUDIO_TOPIC => {
                self.app_audio = app_entries(data);
                self.follow_app_audio(now);
            }
            Input::Topic { topic, data } if topic == "foreground" => {
                let app = data.get("app").and_then(Value::as_str).filter(|a| !a.is_empty() && data.get("home").and_then(Value::as_bool) != Some(true));
                self.foreground = app.map(str::to_owned);
                self.follow_app_audio(now);
            }
            Input::Tick if self.current.as_ref().is_some_and(|b| b.expires <= now) => {
                self.current = None;
                self.changed = true;
            }
            _ => {}
        }
        if self.changed {
            out.push(Effect::Redraw);
        }
        out
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
        status_at(context, app, w, h, 24.0, hdr)
    }

    fn status_at(context: &str, app: &str, w: u64, h: u64, fps: f64, hdr: &str) -> Input {
        let main = json!({"sink": "MAIN", "connected": true, "context": context, "appId": app, "width": w, "height": h, "frameRate": fps,
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
    fn live_tvs_placeholder_size_is_not_shown_and_a_later_size_replaces_the_one_before() {
        // videooutputd's MAIN sink tuning Live TV to 69-6 (2026-11-06, `livetv-videooutput.txt`)
        let ctx = "5f43b00E5b861F74Ea8BebADAB60Bab";
        let main = |m: Value| Input::Luna { tag: STATUS_TAG, payload: json!({"returnValue": true, "video": [m, {"sink": "SUB0", "connected": false}]}) };
        let tv = |w: u64, h: u64, fps: u64, info: Value| {
            main(json!({"sink": "MAIN", "connected": true, "context": ctx, "appId": LIVE_TV_APP, "width": w, "height": h, "frameRate": fps, "videoInfo": info}))
        };
        let mut f = Formats::new();
        f.update(&main(json!({"sink": "MAIN", "connected": false, "context": "unknown", "appId": "unknown", "width": 0, "height": 0})), 0);
        f.update(&tv(1920, 1080, 0, Value::Null), 100);
        f.update(&tv(0, 0, 0, json!({"hdrType": "none"})), 200);
        assert_eq!(f.view(), None, "the placeholder 1920x1080 is not shown");
        f.update(&tv(704, 480, 29, json!({"hdrType": "none"})), 300);
        assert_eq!(marks(&f), ["sdr"]);
        f.update(&tv(1280, 720, 29, json!({"hdrType": "none"})), 400);
        f.update(&tv(704, 480, 29, json!({"hdrType": "none"})), 500);
        assert_eq!(marks(&f), ["sdr"], "a size below 720 lines replaces HD");
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
        for (message, marks) in [
            ("Dolby Digital", vec!["dolby-digital"]),
            ("Dolby Digital Plus", vec!["dolby-digital-plus"]),
            ("Dolby Digital+ 5.1", vec!["dolby-digital-plus", "5.1"]),
            ("Dolby TrueHD 7.1 via sound bar", vec!["dolby-truehd", "7.1"]),
            ("Dolby Audio", vec!["dolby-audio"]),
            ("Dolby Atmos<br>7.1", vec!["atmos", "7.1"]),
            ("DTS", vec!["dts"]),
            ("DTS-HD", vec!["dts-hd"]),
            ("DTS:X", vec!["dtsx"]),
            ("PCM", vec!["pcm"]),
            ("HLG HDR", vec!["hlg"]),
            ("HDR10", vec!["hdr10"]),
            ("HDR", vec!["hdr"]),
        ] {
            assert_eq!(lg("com.webos.service.audio", message), Some(marks), "{message}");
        }
        assert_eq!(lg("com.webos.service.audio", "DTS 9.1"), None, "a layout without a mark is not a format's line");
        assert_eq!(lg("com.webos.service.audio", "Turn Dolby Atmos on for the best experience with Dolby Atmos content."), None);
        assert_eq!(lg("com.webos.service.audio", "Dolby Vision and Dolby Atmos content detected. Turn Dolby Atmos on for the best sound experience."), None);
        assert_eq!(lg("com.example.app", "Dolby Vision"), None);
        let by_icon = json!({"message": "Dolby Vision", "iconPath": "/usr/share/pqcontroller/images/light_toast_icon_hdr.png"});
        assert_eq!(announced(&by_icon), Some(vec!["dolby-vision"]));
    }

    #[test]
    fn every_signal_lg_reports_has_its_mark() {
        for (hdr, mark) in [("none", "sdr"), ("HDR10", "hdr10"), ("HDR10_PLUS", "hdr10plus"), ("HLG", "hlg"), ("DolbyVision", "dolby-vision"),
            ("DOLBY", "dolby-vision"), ("Prime", "hdr")]
        {
            assert_eq!(range_mark(hdr), mark, "{hdr}");
        }
        assert_eq!([(3840, 2160), (1920, 1080), (1280, 536), (720, 576)].map(|(w, h)| size_mark(w, h)), [Some("uhd"), Some("fhd"), Some("hd"), None]);
        assert_eq!([24.0, 59.94, 119.88, 144.0].map(refresh_mark), [None, None, Some("120hz"), Some("144hz")]);
        for (codec, mark) in [
            ("PCM", Some("pcm")),
            ("AC3_DOLBY", Some("dolby-digital")),
            ("DOLBY_PLUS", Some("dolby-digital-plus")),
            ("DOLBY_PLUS_ATMOS", Some("atmos")),
            ("TRUEHD_ATMOS", Some("atmos")),
            ("TRUEHD", Some("dolby-truehd")),
            ("MAT", Some("dolby-audio")),
            ("AC4", Some("dolby-audio")),
            ("DTS", Some("dts")),
            ("DTS_HD_MA", Some("dts-hd")),
            ("DTS_X_P2", Some("dtsx")),
            ("AAC", None),
        ] {
            assert_eq!(codec_mark(&format!("AUDIO_CODEC_{codec}")), mark, "{codec}");
        }
    }

    fn audio_sub(fx: &[Effect]) -> u64 {
        let Some(Effect::Luna { tag, uri, payload, subscribe: true }) = fx.iter().find(|e| matches!(e, Effect::Luna { .. })) else { panic!("{fx:?}") };
        assert_eq!((uri.as_str(), &payload["pipelineId"]), (AUDIO_URI, &json!("_pipe")));
        *tag
    }

    fn codec(tag: u64, codec: &str) -> Input {
        Input::Luna { tag, payload: json!({"returnValue": true, "audioCodecType": codec}) }
    }

    #[test]
    fn an_hdmi_inputs_signal_shows_with_its_sound_and_a_change_of_either_is_news() {
        let mut f = Formats::new();
        let tag = audio_sub(&f.update(&status_at("_pipe", "com.webos.app.hdmi2", 3840, 2160, 60.0, "none"), 0));
        f.update(&codec(tag, "AUDIO_CODEC_PCM"), 100);
        assert_eq!(marks(&f), ["uhd", "sdr", "pcm"], "the sound joins the badge that is up");
        assert!(f.update(&codec(tag, "AUDIO_CODEC_PCM"), 200).is_empty(), "a repeat is no news");
        f.update(&Input::Tick, 100 + SHOW_MS);
        f.update(&codec(tag, "AUDIO_CODEC_DOLBY_PLUS_ATMOS"), 10_000);
        assert_eq!(marks(&f), ["uhd", "sdr", "atmos"], "the film's sound after the menu's");
        f.update(&Input::Tick, 10_000 + SHOW_MS);
        f.update(&status_at("_pipe", "com.webos.app.hdmi2", 3840, 2160, 120.0, "HDR10"), 20_000);
        assert_eq!(marks(&f), ["uhd", "120hz", "hdr10", "atmos"], "the source switching its signal");
        f.update(&toast("com.webos.service.pqcontroller", "HLG HDR"), 30_000);
        assert_eq!(marks(&f), ["uhd", "120hz", "hlg", "atmos"], "an announcement keeps the input's sound");
        assert!(f.update(&codec(tag + 1, "AUDIO_CODEC_DTS"), 30_100).iter().all(|e| *e != Effect::Redraw), "another subscription's answer");
        let gone = f.update(&Input::Luna { tag: STATUS_TAG, payload: json!({"returnValue": true, "video": []}) }, 40_000);
        assert_eq!(gone, [Effect::Cancel { tag }]);
        let app = f.update(&status("film", "netflix", 1920, 1080, "none"), 50_000);
        assert!(!app.iter().any(|e| matches!(e, Effect::Luna { .. })), "an app's sound is not an input's");
    }

    fn front(app: &str) -> Input {
        Input::Topic { topic: "foreground".into(), data: json!({"app": app, "home": app == "com.webos.app.home"}) }
    }

    fn entry(app: &str, context: Option<&str>, codec: &str, profile: Option<&str>, atmos: bool, channels: Option<u64>) -> Value {
        json!({"app": app, "context": context, "codec": codec, "profile": profile, "atmos": atmos, "channels": channels, "layout": null, "delivered": "source"})
    }

    fn said(entries: Vec<Value>) -> Input {
        Input::Helper { topic: APP_AUDIO_TOPIC.into(), data: json!({"entries": entries}) }
    }

    #[test]
    fn what_an_app_delivers_is_the_sound_of_its_picture_and_a_change_of_it_is_news() {
        let mut f = Formats::new();
        f.update(&front("marquee"), 0);
        f.update(&status("film", "marquee", 3840, 2160, "DolbyVision"), 1000);
        assert_eq!(marks(&f), ["uhd", "dolby-vision"]);
        let id = f.view().unwrap().id;
        // it joins the badge that is up
        assert_eq!(f.update(&said(vec![entry("marquee", Some("film"), "truehd", None, true, Some(8))]), 1500), vec![Effect::Redraw]);
        assert_eq!(marks(&f), ["uhd", "dolby-vision", "atmos", "7.1"]);
        assert_eq!(f.view().unwrap().id, id);
        // a change of it renews the badge; LG's own word about the sound does not replace it
        f.update(&said(vec![entry("marquee", Some("film"), "eac3", None, false, Some(6))]), 2000);
        assert_eq!(marks(&f), ["uhd", "dolby-vision", "dolby-digital-plus", "5.1"]);
        f.update(&toast("com.webos.service.audio", "Dolby Atmos"), 2100);
        assert_eq!(marks(&f), ["uhd", "dolby-vision", "dolby-digital-plus", "5.1"]);
        assert!(f.update(&said(vec![entry("marquee", Some("film"), "eac3", None, false, Some(6))]), 2200).is_empty(), "a repeat is no news");
        // the app stops saying it: the sound goes
        f.update(&said(Vec::new()), 2500);
        assert_eq!(marks(&f), ["uhd", "dolby-vision"]);
        // after the badge went, a new entry for the picture that is playing shows a badge of its own
        f.update(&Input::Tick, 2500 + SHOW_MS);
        assert_eq!(f.view(), None);
        f.update(&said(vec![entry("marquee", None, "dts", Some("DTS-HD MA"), false, None)]), 20_000);
        assert_eq!(marks(&f), ["uhd", "dolby-vision", "dts-hd"]);
        assert_ne!(f.view().unwrap().id, id);
        // AAC has no mark and takes away the one before
        f.update(&said(vec![entry("marquee", None, "aac", None, false, Some(2))]), 20_500);
        assert_eq!(marks(&f), ["uhd", "dolby-vision"]);
    }

    #[test]
    fn an_entry_is_waited_for_until_its_picture_is_the_one_shown_and_shows_from_the_first_showing() {
        let mut f = Formats::new();
        f.update(&front("marquee"), 0);
        f.update(&said(vec![entry("marquee", Some("next"), "truehd", None, true, None)]), 100);
        f.update(&status("prev", "marquee", 1920, 1080, "none"), 200);
        assert_eq!(marks(&f), ["fhd", "sdr"], "the entry is for another picture");
        f.update(&Input::Tick, 200 + SHOW_MS);
        f.update(&status("next", "marquee", 3840, 2160, "HDR10"), 9000);
        assert_eq!(marks(&f), ["uhd", "hdr10", "atmos"]);
    }

    #[test]
    fn an_entry_of_an_app_that_is_not_in_front_or_of_another_picture_is_ignored() {
        let sound = |foreground: &str, picture: &str, entry_app: &str, entry_context: Option<&str>| {
            let mut f = Formats::new();
            f.update(&said(vec![entry(entry_app, entry_context, "truehd", None, true, Some(8))]), 0);
            f.update(&front(foreground), 1);
            f.update(&status(picture, "marquee", 3840, 2160, "none"), 1000);
            marks(&f)
        };
        assert_eq!(sound("marquee", "film", "marquee", Some("film")), ["uhd", "sdr", "atmos", "7.1"]);
        assert_eq!(sound("marquee", "film", "marquee", None), ["uhd", "sdr", "atmos", "7.1"], "no context: whichever picture is in front");
        assert_eq!(sound("com.webos.app.hdmi1", "film", "marquee", Some("film")), ["uhd", "sdr"], "another app is in front");
        assert_eq!(sound("com.webos.app.home", "film", "marquee", None), ["uhd", "sdr"], "Home is in front");
        assert_eq!(sound("marquee", "film", "marquee", Some("trailer")), ["uhd", "sdr"], "another picture");
        assert_eq!(sound("marquee", "film", "plex", Some("film")), ["uhd", "sdr"], "another app's entry");
        // the app leaving the front takes its sound off the badge that is up
        let mut f = Formats::new();
        f.update(&front("marquee"), 0);
        f.update(&status("film", "marquee", 3840, 2160, "none"), 1000);
        f.update(&said(vec![entry("marquee", Some("film"), "pcm", None, false, Some(6))]), 1100);
        assert_eq!(marks(&f), ["uhd", "sdr", "pcm", "5.1"]);
        f.update(&front("com.webos.app.home"), 1200);
        assert_eq!(marks(&f), ["uhd", "sdr"]);
    }

    #[test]
    fn an_inputs_picture_keeps_its_own_sound_whatever_an_app_says() {
        let mut f = Formats::new();
        f.update(&front("com.webos.app.hdmi2"), 0);
        f.update(&said(vec![entry("com.webos.app.hdmi2", None, "truehd", None, true, Some(8))]), 0);
        let tag = audio_sub(&f.update(&status_at("_pipe", "com.webos.app.hdmi2", 3840, 2160, 60.0, "none"), 100));
        f.update(&codec(tag, "AUDIO_CODEC_PCM"), 200);
        assert_eq!(marks(&f), ["uhd", "sdr", "pcm"]);
    }

    #[test]
    fn every_format_an_app_can_deliver_has_its_mark_or_none() {
        let marks = |codec: &str, profile: Option<&str>, atmos: bool, channels: Option<u64>| {
            let e = app_entries(&json!({"entries": [entry("a", None, codec, profile, atmos, channels)]}));
            app_sound(&e[0])
        };
        for (codec, profile, atmos, channels, sound, speakers) in [
            ("truehd", None, true, Some(8), Some("atmos"), Some("7.1")),
            ("eac3", None, true, Some(6), Some("atmos"), Some("5.1")),
            ("truehd", None, false, Some(8), Some("dolby-truehd"), Some("7.1")),
            ("eac3", None, false, Some(6), Some("dolby-digital-plus"), Some("5.1")),
            ("ac3", None, false, Some(6), Some("dolby-digital"), Some("5.1")),
            ("ac3", None, true, Some(2), Some("dolby-digital"), None),
            ("dts", Some("DTS:X"), false, Some(8), Some("dtsx"), Some("7.1")),
            ("dts", Some("dts-hd ma"), false, Some(8), Some("dts-hd"), Some("7.1")),
            ("dts", Some("DTS-HD High Resolution"), false, Some(8), Some("dts-hd"), Some("7.1")),
            ("dts", None, false, Some(6), Some("dts"), Some("5.1")),
            ("pcm", None, false, Some(2), Some("pcm"), None),
            ("aac", None, false, Some(6), None, Some("5.1")),
            ("opus", None, false, None, None, None),
            ("flac", None, false, Some(2), None, None),
            ("mp3", None, false, None, None, None),
            ("other", None, false, Some(4), None, None),
        ] {
            assert_eq!(marks(codec, profile, atmos, channels), (sound, speakers), "{codec} {profile:?} atmos {atmos} {channels:?}");
        }
    }

    #[test]
    fn a_row_too_wide_drops_the_speakers_first_and_then_shortens_the_sound() {
        let row = ["uhd", "dolby-vision", "dolby-truehd", "7.1"];
        let width = |row: &[&str]| row.iter().map(|m| match *m { "dolby-truehd" => 4, "dolby-audio" => 3, "7.1" => 2, _ => 1 }).sum::<usize>();
        assert_eq!(fit(&row, |r| width(r) <= 8), row, "it fits");
        assert_eq!(fit(&row, |r| width(r) <= 6), ["uhd", "dolby-vision", "dolby-truehd"], "the speakers go first");
        assert_eq!(fit(&row, |r| width(r) <= 5), ["uhd", "dolby-vision", "dolby-audio"], "then the sound is shortened");
        assert_eq!(fit(&["uhd", "atmos", "7.1"], |r| width(r) <= 1), ["uhd", "atmos"], "Atmos has no shorter mark");
    }
}
