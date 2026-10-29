//! System's children for seeing, hearing and safety (`settings/PageSafety.qml`, `PageAudioGuide.qml`, `PageAudioOptions.qml`,
//! `PageEmergency.qml`, `PageCaptions.qml`): each row is LG's own key with the TV's own list, in LG's names; a row the TV does not
//! answer or hides is left out, as in LG's screen. The NEXTGEN TV rows wait for the ATSC 3.0 tuner. Safety only reads: LG changes a
//! lock after its PIN prompt, and the setting takes the new value from anyone who writes it, so a page that wrote `lock.parentalControl`
//! would open the lock without the PIN; changing a lock stays in LG's own screen (the last row).

use super::util::{atsc3, atsc3_read};
use crate::settings::ctx::{Ctx, Spec, lg_launch};
use crate::settings::names;
use crate::settings::page::{Page, Read};
use crate::settings::row::Row;

fn child(id: &'static str, title: &'static str, icon: &'static str) -> Page {
    Page::new(id, title, icon).parent("system").back("System").hidden_from_search()
}

// ------------------------------------------------------------------------------------------------------------------ Safety

pub fn safety() -> Page {
    child("safety", "Safety", "shield").reads(safety_reads).rows(safety_rows)
}

const TV_RATINGS: [&str; 7] = ["tvRatingGeneralAge", "tvRatingGeneralDialog", "tvRatingGeneralLanguage", "tvRatingGeneralSex", "tvRatingGeneralViolence", "tvRatingChildrenAge", "tvRatingChildrenFantasy"];

fn safety_reads(_: &Ctx) -> Vec<Read> {
    vec![Read::lg("lock", &["parentalControl", "movieRating", "applockPerApp", "inputBlock"]), Read::lg("lock", &TV_RATINGS)]
}

fn flag(c: &Ctx, ds: &str) -> String {
    if !c.lg.has(ds) {
        String::new()
    } else if c.lg.opt_text(ds).as_deref() == Some("true") {
        "On".into()
    } else {
        "Off".into()
    }
}

/// On when any TV rating is not "off"; nothing when the TV answered none.
fn ratings_on(c: &Ctx) -> String {
    let seen: Vec<String> = TV_RATINGS.iter().filter_map(|k| c.lg.opt_text(&format!("lock.{k}"))).collect();
    if seen.is_empty() {
        String::new()
    } else if seen.iter().any(|v| v != "off") {
        "On".into()
    } else {
        "Off".into()
    }
}

fn state_row(c: &Ctx, icon: &str, label: &str, ds: &str, value: String, help: &str) -> Row {
    let mut r = Row::info(label).icon(icon).label(label).help(help).value(if value.is_empty() { "\u{2013}" } else { &value });
    r.chevron = false;
    r.visible = !c.lg.missing(ds) || ds == "lock.tvRatings";
    r.detail = if ds == "lock.tvRatings" || c.lg.has(ds) { String::new() } else { "TV not answering".into() };
    r
}

fn safety_rows(c: &Ctx) -> Vec<Row> {
    vec![
        state_row(c, "lock", "Lock System", "lock.parentalControl", flag(c, "lock.parentalControl"), "On: the locks below ask for the PIN."),
        state_row(c, "tv", "TV Program Locks", "lock.tvRatings", ratings_on(c), "Blocks programs above the ratings set, until the PIN is typed."),
        state_row(c, "film", "Movie Lock", "lock.movieRating", c.lg.opt_text("lock.movieRating").map(|v| names::name(names::MOVIE_RATING, &v)).unwrap_or_default(), "The highest movie rating that plays without the PIN."),
        state_row(c, "layout-grid", "Application Locks", "lock.applockPerApp", flag(c, "lock.applockPerApp"), "Apps that ask for the PIN to open."),
        state_row(
            c,
            "cable",
            "Input Locks",
            "lock.inputBlock",
            if !c.lg.has("lock.inputBlock") { String::new() } else if c.lg.num("lock.inputBlock", 0.0) > 0.0 { "On".into() } else { "Off".into() },
            "Inputs that ask for the PIN to open.",
        ),
        lg_launch("changelocks", "Change locks", "parentalControls", "").help("A lock changes only with the PIN, in LG's screen."),
    ]
}

// ------------------------------------------------------------------------------------------------------------ Audio Guidance

pub fn audio_guide() -> Page {
    child("audioguide", "Audio Guidance", "audio-lines").reads(audio_guide_reads).rows(audio_guide_rows)
}

fn audio_guide_reads(_: &Ctx) -> Vec<Read> {
    vec![Read::lg("option", &["audioGuidance", "audioGuidanceSpeed", "audioGuidanceVolume", "audioGuidancePitch"])]
}

fn audio_guide_rows(c: &Ctx) -> Vec<Row> {
    let on = c.lg.is("option.audioGuidance", "on");
    let voice = |ds: &str, icon: &str, label: &str, table: names::Table| {
        let mut r = c.choice(Spec::new(ds).icon(icon).label(label).names(table));
        r.enabled = on && c.lg.usable(ds);
        r.detail = if on { String::new() } else { "Turn it on first".into() };
        r
    };
    vec![
        c.toggle(Spec::new("option.audioGuidance").icon("audio-lines").label("Audio Guidance")),
        voice("option.audioGuidanceSpeed", "gauge", "Speed", names::SLOW_TO_FAST),
        voice("option.audioGuidanceVolume", "volume-2", "Volume", names::LOW_TO_HIGH),
        voice("option.audioGuidancePitch", "audio-waveform", "Pitch", names::LOW_TO_HIGH),
    ]
}

// ------------------------------------------------------------------------------------------------------------ Audio options

pub fn audio_options() -> Page {
    child("audiooptions", "Audio options", "headphones").reads(audio_options_reads).rows(audio_options_rows)
}

fn audio_options_reads(_: &Ctx) -> Vec<Read> {
    vec![
        Read::lg("sound", &["audioDescription", "audioDescriptionVolume", "audioDescriptionAtsc30", "visualImpaired", "dialogEnhancementAtsc30", "dialogEnhancementVolumeAtsc30", "hearingAid", "tvPowerSound"]),
        Read::lg("option", &["supplementaryAudio", "additionalAudioSelection"]),
        Read::lg("other", &["autoSLZoomWithObjectDetection"]),
        atsc3_read(),
    ]
}

fn audio_options_rows(c: &Ctx) -> Vec<Row> {
    let describing = c.lg.is("sound.audioDescription", "on");
    let amplifying = c.lg.is("sound.dialogEnhancementAtsc30", "on");
    let nextgen = |mut r: Row, ds: &str| {
        r.visible = atsc3(c) && c.lg.shown(ds);
        r
    };
    let mut desc_vol = c.slider(Spec::new("sound.audioDescriptionVolume").icon("volume-2").label("Description Volume").range(-15.0, 15.0, 1.0).lg_shown());
    desc_vol.enabled = describing && c.lg.usable("sound.audioDescriptionVolume");
    desc_vol.detail = if describing { String::new() } else { "Turn Audio Description on".into() };
    let mut gain = nextgen(c.slider(Spec::new("sound.dialogEnhancementVolumeAtsc30").icon("sliders-horizontal").label("Gain Level").range(0.0, 12.0, 1.0).unit(" dB")), "sound.dialogEnhancementVolumeAtsc30");
    gain.enabled = amplifying && c.lg.usable("sound.dialogEnhancementVolumeAtsc30");
    gain.detail = if amplifying { String::new() } else { "Turn Amplify Dialogue on".into() };
    vec![
        c.toggle(Spec::new("sound.visualImpaired").icon("eye").label("Video Description").lg_shown()),
        nextgen(c.toggle(Spec::new("sound.audioDescriptionAtsc30").icon("audio-lines").label("NEXTGEN TV Description")), "sound.audioDescriptionAtsc30"),
        c.choice(Spec::new("option.supplementaryAudio").icon("audio-lines").label("Audio Type").names(names::SUPPLEMENTARY_AUDIO)),
        c.choice(Spec::new("option.additionalAudioSelection").icon("audio-waveform").label("Additional Audio").names(names::ADDITIONAL_AUDIO_SELECTION)),
        desc_vol,
        nextgen(c.toggle(Spec::new("sound.dialogEnhancementAtsc30").icon("mic").label("Amplify Dialogue")), "sound.dialogEnhancementAtsc30"),
        gain,
        c.choice(Spec::new("sound.hearingAid").icon("headphones").label("Hearing Aid").names(names::HEARING_AID)),
        c.toggle(Spec::new("other.autoSLZoomWithObjectDetection").icon("scan-eye").label("Sign Language Zoom").lg_shown()),
        c.toggle(Spec::new("sound.tvPowerSound").icon("bell").label("Device Power Sound").lg_shown()),
    ]
}

// ------------------------------------------------------------------------------------------------------------ Emergency

pub fn emergency() -> Page {
    child("emergency", "Emergency Information", "octagon-alert").reads(emergency_reads).rows(emergency_rows)
}

fn emergency_reads(_: &Ctx) -> Vec<Read> {
    vec![Read::lg("option", &["emergencyInformationAtsc30", "emergencyInformationLanguageAtsc30"])]
}

fn emergency_rows(c: &Ctx) -> Vec<Row> {
    vec![
        c.toggle(Spec::new("option.emergencyInformationAtsc30").tab(0).icon("octagon-alert").label("Enable the function").help("When an emergency occurs, the TV shows the alert and speaks it.").lg_shown()),
        c.choice(Spec::new("option.emergencyInformationLanguageAtsc30").tab(0).icon("languages").label("Audio Language").names(names::DIGITAL_PLUS_CAPTION).help("The language of the spoken alert.")),
    ]
}

// ------------------------------------------------------------------------------------------------------------ Closed Caption

pub fn captions() -> Page {
    child("captions", "Closed Caption", "type").tabs(&["Captions", "Text", "Background", "Subtitles"]).reads(captions_reads).rows(captions_rows)
}

fn captions_reads(_: &Ctx) -> Vec<Read> {
    vec![
        Read::lg(
            "caption",
            &[
                "captionEnable",
                "captionAnalog",
                "captionDigital",
                "digitalPlusCaptionLanguage",
                "captionStyle",
                "captionSize",
                "captionFont",
                "captionTextColor",
                "captionTextOpacity",
                "captionBgColor",
                "captionBgOpacity",
                "captionEdgeType",
                "captionEdgeColor",
                "captionWindowColor",
                "captionWindowOpacity",
                "captionDigitalPosOption",
                "hardOfHearing",
                "aiCaptionEnable",
            ],
        ),
        Read::lg("option", &["subtitleLanguageFirst", "subtitleLanguageSecond"]),
        atsc3_read(),
    ]
}

/// A look row answers only while the style is Custom, as in LG's own screen.
fn look(c: &Ctx, mut r: Row) -> Row {
    let custom = c.lg.is("caption.captionStyle", "manual");
    r.enabled = r.enabled && custom;
    r.detail = if custom { r.detail } else { "Set Style to Custom".into() };
    r
}

fn captions_rows(c: &Ctx) -> Vec<Row> {
    let pick = |ds: &str, tab: usize, icon: &str, label: &str, table: names::Table| look(c, c.choice(Spec::new(ds).tab(tab).icon(icon).label(label).names(table)));
    let seg = |ds: &str, tab: usize, icon: &str, label: &str, options: &[&str], values: &[&str]| look(c, c.seg(Spec::new(ds).tab(tab).icon(icon).label(label).options(options).values(values).lg_shown()));
    let opacity = ["Flash", "Translucent", "Transparent", "Solid"];
    let opacity_values = ["flash", "translucent", "transparent", "solid"];
    let mut nextgen = c.choice(Spec::new("caption.digitalPlusCaptionLanguage").tab(0).icon("languages").label("NEXTGEN TV Mode").names(names::DIGITAL_PLUS_CAPTION));
    nextgen.visible = atsc3(c) && c.lg.shown("caption.digitalPlusCaptionLanguage");
    vec![
        c.toggle(Spec::new("caption.captionEnable").tab(0).icon("type").label("Closed Caption")),
        c.choice(Spec::new("caption.captionAnalog").tab(0).icon("tv").label("Analog Mode").names(names::CAPTION_ANALOG)),
        c.choice(Spec::new("caption.captionDigital").tab(0).icon("monitor").label("Digital Mode").names(names::CAPTION_DIGITAL)),
        nextgen,
        c.choice(Spec::new("caption.captionDigitalPosOption").tab(0).icon("layout-grid").label("Position Settings").names(names::CAPTION_POS_OPTION).values(&["auto", "autoAi", "splitDown16_9", "splitUp16_9"])),
        c.seg(Spec::new("caption.captionStyle").tab(0).icon("paintbrush").label("Style").options(&["Set by program", "Custom"]).values(&["auto", "manual"]).lg_shown()),
        c.toggle(Spec::new("caption.hardOfHearing").tab(0).icon("volume-1").label("Hard of Hearing").lg_shown()),
        c.toggle(Spec::new("caption.aiCaptionEnable").tab(0).icon("sparkles").label("AI Caption").lg_shown()),
        seg("caption.captionSize", 1, "type", "Size", &["Small", "Standard", "Large"], &["small", "standard", "large"]),
        pick("caption.captionFont", 1, "type", "Font", names::CAPTION_FONT),
        pick("caption.captionTextColor", 1, "palette", "Text Color", names::CAPTION_COLOR),
        seg("caption.captionTextOpacity", 1, "blend", "Text Opacity", &opacity, &opacity_values),
        pick("caption.captionEdgeType", 1, "square", "Edge Type", names::CAPTION_EDGE_TYPE),
        pick("caption.captionEdgeColor", 1, "palette", "Edge Color", names::CAPTION_COLOR),
        pick("caption.captionBgColor", 2, "palette", "BG Color", names::CAPTION_COLOR),
        seg("caption.captionBgOpacity", 2, "blend", "BG Opacity", &opacity, &opacity_values),
        pick("caption.captionWindowColor", 2, "palette", "Window Color", names::CAPTION_COLOR),
        seg("caption.captionWindowOpacity", 2, "blend", "Window Opacity", &["Translucent", "Transparent", "Solid"], &["translucent", "transparent", "solid"]),
        c.choice(Spec::new("option.subtitleLanguageFirst").tab(3).icon("languages").label("Subtitle First Choice").names(names::DIGITAL_PLUS_CAPTION)),
        c.choice(Spec::new("option.subtitleLanguageSecond").tab(3).icon("languages").label("Subtitle Second Choice").names(names::DIGITAL_PLUS_CAPTION)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ctx::fixtures::{World, lg_with};
    use crate::settings::row::{Arg, Op};
    use serde_json::json;

    fn by_id<'a>(rows: &'a [Row], id: &str) -> &'a Row {
        rows.iter().find(|r| r.id == id).unwrap_or_else(|| panic!("no row {id} in {:?}", rows.iter().map(|r| &r.id).collect::<Vec<_>>()))
    }

    #[test]
    fn safety_only_shows_the_locks_and_the_last_row_leaves_for_lgs_screen() {
        let w = World::new(lg_with(
            &[("lock", json!({"settings": {"parentalControl": "true", "movieRating": "pg13", "applockPerApp": "false", "inputBlock": 3, "tvRatingGeneralAge": "off", "tvRatingGeneralDialog": "tv-14"}}))],
            &[],
        ));
        let r = safety_rows(&w.ctx());
        assert_eq!(by_id(&r, "Lock System").value, "On");
        assert_eq!(by_id(&r, "TV Program Locks").value, "On", "one rating that is not off");
        assert_eq!(by_id(&r, "Movie Lock").value, "PG-13");
        assert_eq!(by_id(&r, "Application Locks").value, "Off");
        assert_eq!(by_id(&r, "Input Locks").value, "On");
        assert!(r.iter().filter(|x| x.id != "changelocks").all(|x| x.act.is_none() || x.info), "no row here changes a lock");
        let ops = (by_id(&r, "changelocks").act.as_ref().unwrap())(Arg::Press);
        assert!(matches!(&ops[..], [Op::Launch { params, .. }] if params["target"] == "parentalControls"));
        let empty = World::new(lg_with(&[], &[]));
        let r = safety_rows(&empty.ctx());
        assert_eq!(by_id(&r, "TV Program Locks").value, "\u{2013}");
        assert!(by_id(&r, "Lock System").visible && by_id(&r, "Lock System").detail == "TV not answering");
    }

    #[test]
    fn the_voice_rows_wait_for_the_guidance() {
        let mut w = World::new(lg_with(&[("option", json!({"settings": {"audioGuidance": "off", "audioGuidanceSpeed": "medium", "audioGuidanceVolume": "high", "audioGuidancePitch": "low"}}))], &[]));
        let r = audio_guide_rows(&w.ctx());
        assert!(!by_id(&r, "option.audioGuidanceSpeed").enabled && by_id(&r, "option.audioGuidanceSpeed").detail == "Turn it on first");
        assert_eq!(by_id(&r, "option.audioGuidanceSpeed").value, "Medium");
        w.lg.learn("option", &json!({"settings": {"audioGuidance": "on"}}));
        assert!(by_id(&audio_guide_rows(&w.ctx()), "option.audioGuidanceVolume").enabled);
    }

    #[test]
    fn captions_look_rows_answer_only_while_the_style_is_custom_and_nextgen_waits_for_the_tuner() {
        let mut w = World::new(lg_with(&[("caption", json!({"settings": {"captionEnable": "on", "captionStyle": "auto", "captionSize": "standard", "captionFont": "default", "digitalPlusCaptionLanguage": "eng"}}))], &[]));
        let r = captions_rows(&w.ctx());
        assert_eq!(r.iter().map(|x| x.tab).max(), Some(3));
        assert!(!by_id(&r, "caption.captionSize").enabled && by_id(&r, "caption.captionSize").detail == "Set Style to Custom");
        assert!(by_id(&r, "caption.captionEnable").enabled, "the switches of the first tab always answer");
        assert!(!by_id(&r, "caption.digitalPlusCaptionLanguage").visible);
        w.lg.learn("caption", &json!({"settings": {"captionStyle": "manual"}}));
        assert!(by_id(&captions_rows(&w.ctx()), "caption.captionSize").enabled);
        assert_eq!(by_id(&r, "caption.captionAnalog").label, "Analog Mode");
    }

    #[test]
    fn the_audio_options_leave_out_what_the_tv_hides_and_the_tuner_rows_wait_for_the_tuner() {
        let w = World::new(lg_with(&[("sound", json!({"settings": {"audioDescription": "on", "audioDescriptionVolume": 3, "visualImpaired": "off", "tvPowerSound": "on", "hearingAid": "off"}}))], &[]));
        let r = audio_options_rows(&w.ctx());
        assert!(by_id(&r, "sound.audioDescriptionVolume").enabled, "description is on");
        assert!(!by_id(&r, "sound.dialogEnhancementVolumeAtsc30").visible && !by_id(&r, "sound.audioDescriptionAtsc30").visible);
        assert_eq!(by_id(&r, "sound.hearingAid").value, "Off");
    }
}
