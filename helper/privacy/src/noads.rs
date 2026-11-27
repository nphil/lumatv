//! Structural player transform derived from samoz83/AmazOff (MIT; payload/privacy/NOTICE); the patches follow its build-patch.js.
//! Every ad patch must match before a byte is served: a player they do not all fit stays unmodified and never receives an old one.
//! The report patches (telemetry and beacons) are best effort: Prime moves that code more often, and a report that is still sent is a
//! smaller harm than ads that come back.
use fancy_regex::Regex;
use std::sync::LazyLock;

/// How many places a patch has to be found in.
enum Need {
    /// An ad patch: exactly this many.
    Exactly(usize),
    /// An ad patch for a name the player uses in several places (live pre-rolls: five in one build, six in the next): at least one, and
    /// every use of the name is one of its places, so a use in a form not seen before (a write, a read without its guard, a string) is
    /// not passed over. A place does not start inside an identifier, a property name or a string.
    Every(&'static str),
    /// A report patch: wherever it is found. A player without it is noted in the result and the rest is applied.
    Anywhere,
}
struct Patch {
    name: &'static str,
    /// What the expression starts with: only these places are tried.
    anchor: &'static str,
    expression: Regex,
    replacement: &'static str,
    need: Need,
}
fn patches() -> &'static [Patch] {
    static PATCHES: LazyLock<Vec<Patch>> = LazyLock::new(|| {
        use Need::{Anywhere, Every, Exactly};
        [
            // First: every other patch reads the flag this one defines, so none is applied without it.
            ("no-ads flag", "\"use strict\";var ", r#"("use strict";var [A-Za-z_$][\w$]*="undefined"!=typeof globalThis\?globalThis:"undefined"!=typeof window\?window:"undefined"!=typeof global\?global:"undefined"!=typeof self\?self:\{\};)(function e\(e\)\{)"#, "${1}var __HOMEBREW_NOADS=!0;$2", Exactly(1)),
            ("playlist converter", "intraTitlePlaylist:", r#"intraTitlePlaylist:([A-Za-z_$][\w$]*)\.playbackUrls\.intraTitlePlaylist,pauseAdsResolution:\1\.playbackUrls\.pauseAdsResolution,"#, "intraTitlePlaylist:__HOMEBREW_NOADS?$1.playbackUrls.intraTitlePlaylist.filter(function(x){return\"Remote\"!==x.type}).map(function(x){if(null==x.nonLinearAds)return x;var y=Object.assign({},x);delete y.nonLinearAds;return y}):$1.playbackUrls.intraTitlePlaylist,pauseAdsResolution:__HOMEBREW_NOADS?void 0:$1.playbackUrls.pauseAdsResolution,", Exactly(1)),
            ("pause behavior", "pauseBehavior:", r#"pauseBehavior:([A-Za-z_$][\w$]*)\(([A-Za-z_$][\w$]*)\),playbackResourceExpirationTimeMs:"#, "pauseBehavior:__HOMEBREW_NOADS?void 0:$1($2),playbackResourceExpirationTimeMs:", Exactly(1)),
            ("new-stack pause behavior", "pauseBehavior:function(", r#"(pauseBehavior:function\([A-Za-z_$][\w$]*\)\{)(?=var [A-Za-z_$][\w$]*=[A-Za-z_$][\w$]*\.PlaybackResourceUtils\.valueOrUndefined\([A-Za-z_$][\w$]*\.playbackUrls\);if\(null==[A-Za-z_$][\w$]*\)\{var [A-Za-z_$][\w$]*=[A-Za-z_$][\w$]*\.PlaybackResourceUtils\.valueOrUndefined\([A-Za-z_$][\w$]*\.playlistedPlaybackUrls\))"#, "${1}if(__HOMEBREW_NOADS)return;", Exactly(1)),
            ("ad-break resolver", ".prototype.resolveWithAdBreaks=", r#"(\.prototype\.resolveWithAdBreaks=freeOnUnref\(function\([A-Za-z_$][\w$]*,[A-Za-z_$][\w$]*\)\{var [A-Za-z_$][\w$]*=this;if\()(0===[A-Za-z_$][\w$]*\.length\))"#, "${1}__HOMEBREW_NOADS||$2", Exactly(1)),
            ("unresolved content source", "case\"Remote\":", r#"case"Remote":(?=var [^;]{0,200}?this\.adBreakConfigBuilder\.build\()"#, "case\"Remote\":if(__HOMEBREW_NOADS)break;", Exactly(1)),
            // The player tests the preference behind a null check on the same receiver, in either polarity. Only the preference read is
            // answered locally (false), so the null check, the comparison and its short-circuit stay the player's own.
            ("live pre-rolls", "null", r#"(null!=([A-Za-z_$][\w$]*(?:\.[A-Za-z_$][\w$]*)*)\.showLivePreRolls&&(?:!0|true)===)(\2\.showLivePreRolls\.enabled)(?=&&(?!=)|\|\|(?!=)|[),;}\]:]|\?(?![.?]))|(null==([A-Za-z_$][\w$]*(?:\.[A-Za-z_$][\w$]*)*)\.showLivePreRolls\|\|(?:!0|true)!==)(\5\.showLivePreRolls\.enabled)(?=&&(?!=)|\|\|(?!=)|[),;}\]:]|\?(?![.?]))"#, "${1}${4}(__HOMEBREW_NOADS?!1:${3}${6})", Every(".showLivePreRolls.enabled")),
            // By name only: Prime changed what the method does first (it no longer calls applyDeferredEventTimestamp), not that it is one.
            ("media events", ".prototype.enqueueMediaEvents=", r#"(\.prototype\.enqueueMediaEvents=freeOnUnref\(function\(e\)\{)"#, "${1}return;", Anywhere),
            ("telemetry queue", ".prototype.reportEvent=", r#"(\.prototype\.reportEvent=freeOnUnref\(function\(e\)\{var t=this;)(?=this\.queue\.add\(e\),this\.isQueueAtMaxDepth\(\))"#, "${1}return;", Anywhere),
            ("minerva metrics", ".prototype.recordMetric=", r#"(\.prototype\.recordMetric=freeOnUnref\(function\(e\)\{)(?=if\(null!=this\.minervaClient\)try\{this\.minervaClient\.record\(e\))"#, "${1}return;", Anywhere),
            ("minerva kpi upload", ".prototype.uploadMinervaKpiMetrics=", r#"(\.prototype\.uploadMinervaKpiMetrics=freeOnUnref\(function\(\)\{)"#, "${1}return;", Anywhere),
            ("client log upload", ".prototype.write=", r#"(\.prototype\.write=freeOnUnref\(function\(e,t\)\{)(?=if\(!\(e<this\.logLevel\)\)\{var n=this\.toLogLine\()"#, "${1}return;", Anywhere),
            ("playback diagnostics", ".prototype.batchMessage=", r#"(\.prototype\.batchMessage=freeOnUnref\(function\(e,t\)\{)(?=1e3<=this\.batchedMessages\.length)"#, "${1}return;", Anywhere),
            ("ad beacons", ".prototype.send=", r#"(\.prototype\.send=freeOnUnref\(function\(e,t,n\)\{if\(void 0===t&&\(t=\{\}\),)this\.enableEventReporting(?=\)\{for\(var r=0)"#, "$1!1", Anywhere),
            ("ad audit pings", ".prototype.make", r#"(\.prototype\.make(?:Direct)?AuditPingRequest=freeOnUnref\(function\([\w$]+,[\w$]+\)\{)"#, "${1}return;", Anywhere),
            ("ad beacon client", "{key:\"dispatch\",value:function(", r#"(\{key:"dispatch",value:function\([A-Za-z_$][\w$]*,[A-Za-z_$][\w$]*\)\{)(?=var [A-Za-z_$][\w$]*,[A-Za-z_$][\w$]*=this,[A-Za-z_$][\w$]*=[A-Za-z_$][\w$]*\.url,[A-Za-z_$][\w$]*="UnknownError")"#, "${1}if(__HOMEBREW_NOADS)return;", Anywhere),
        ].into_iter().map(|(name, anchor, expression, replacement, need)| Patch {
            name, anchor, expression: Regex::new(&format!("^(?:{expression})")).expect("constant transform expression"), replacement, need
        }).collect()
    });
    &PATCHES
}

#[derive(Debug)]
pub struct Transformed {
    pub body: String,
    /// The report patches this player had no place for: it still sends those reports.
    pub missed: Vec<&'static str>,
}

/// The player with the patches applied, or why it cannot be trusted with them: an ad patch did not match, and the caller serves the
/// original.
pub fn transform(source: &str) -> Result<Transformed, String> {
    let mut edits = Vec::with_capacity(32);
    let mut missed = Vec::new();
    for patch in patches() {
        let first = edits.len();
        // A minified player has thousands of `null` tests but only a handful of preference reads.
        // Look back locally from each required read; a nearby earlier match must end at this read.
        let every = match patch.need {
            Need::Every(name) => Some(name),
            _ => None,
        };
        for (anchor, _) in source.match_indices(every.unwrap_or(patch.anchor)) {
            let start = if every.is_some() {
                let mut window = anchor.saturating_sub(2048);
                while !source.is_char_boundary(window) {
                    window += 1;
                }
                let Some(n) = source[window..anchor].rfind(patch.anchor) else {
                    continue;
                };
                window + n
            } else {
                anchor
            };
            if every.is_some()
                && start > 0
                && matches!(source.as_bytes()[start - 1], b'.' | b'"' | b'\'' | b'`' | b'_' | b'$' | b'0'..=b'9' | b'a'..=b'z' | b'A'..=b'Z' | 0x80..=0xFF)
            {
                continue;
            }
            let mut end = (start + 2048).min(source.len());
            while !source.is_char_boundary(end) {
                end -= 1;
            }
            let Some(caps) = patch
                .expression
                .captures(&source[start..end])
                .map_err(|e| format!("{}: {e}", patch.name))?
            else {
                continue;
            };
            let m = caps.get(0).unwrap();
            if every.is_some_and(|name| start + m.end() != anchor + name.len()) {
                continue;
            }
            let mut replacement = String::new();
            caps.expand(patch.replacement, &mut replacement);
            edits.push((start + m.start(), start + m.end(), replacement));
        }
        let found = edits.len() - first;
        match patch.need {
            Need::Exactly(n) if found != n => {
                return Err(format!("{}: expected {n} anchors, got {found}", patch.name));
            }
            Need::Every(name) => {
                let uses = source.matches(name).count();
                if found == 0 || found != uses {
                    return Err(format!(
                        "{}: {found} of {uses} uses of {name} are in a known form",
                        patch.name
                    ));
                }
            }
            Need::Anywhere if found == 0 => missed.push(patch.name),
            _ => {}
        }
    }
    edits.sort_unstable_by_key(|e| e.0);
    let capacity = edits
        .iter()
        .fold(source.len(), |n, (a, b, s)| n + s.len() - (b - a));
    let mut out = String::with_capacity(capacity);
    let mut at = 0;
    for (start, end, replacement) in edits {
        if start < at {
            return Err("overlapping player transform anchors".into());
        }
        out.push_str(&source[at..start]);
        out.push_str(&replacement);
        at = end;
    }
    out.push_str(&source[at..]);
    Ok(Transformed { body: out, missed })
}

pub fn player_route(path: &str) -> Option<&str> {
    let path = path.split('?').next()?;
    let route = path
        .strip_prefix("/family/lg/ATVUnfPlayerBundle-1.0/")?
        .strip_suffix("/js/ATVUnfPlayerBundle.js")?;
    (!route.is_empty()
        && route.len() <= 32
        && route
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-'))
    .then_some(route)
}

#[cfg(test)]
mod tests {
    use super::*;
    const PLAYER: &str = include_str!("../tests/player.js");
    #[test]
    fn routes_are_exact_not_substrings() {
        assert_eq!(
            player_route("/family/lg/ATVUnfPlayerBundle-1.0/wave3/js/ATVUnfPlayerBundle.js?v=1"),
            Some("wave3")
        );
        for p in [
            "/packages/SyeCorePlayerNativeModule-1.0/prod/ignite/module.so",
            "/licence",
            "/family/lg/ATVUnfPlayerBundle-1.0/../../js/ATVUnfPlayerBundle.js",
        ] {
            assert_eq!(player_route(p), None);
        }
    }
    #[test]
    fn unknown_and_already_patched_bundles_fail_closed() {
        assert!(
            transform("var x=1;")
                .unwrap_err()
                .starts_with("no-ads flag:")
        );
    }
    #[test]
    fn deterministic_player_has_all_transforms_and_keeps_session_calls() {
        let out = transform(PLAYER).unwrap();
        assert!(out.missed.is_empty(), "{:?}", out.missed);
        let body = out.body;
        assert!(body.contains("var __HOMEBREW_NOADS=!0;function e"));
        assert!(body.contains("__HOMEBREW_NOADS||0===e.length"));
        assert!(body.contains("pauseBehavior:function(e){if(__HOMEBREW_NOADS)return;var t="));
        assert_eq!(
            body.matches("(__HOMEBREW_NOADS?!1:").count(),
            3,
            "every live pre-roll place"
        );
        assert!(
            body.contains(r#"{key:"dispatch",value:function(o,t){if(__HOMEBREW_NOADS)return;"#)
        );
        // found by its name, whatever Prime made the method do first: the three shapes it has had
        assert_eq!(
            body.matches(".enqueueMediaEvents=freeOnUnref(function(e){return;")
                .count(),
            3
        );
        assert!(body.contains("untouched licence and UpdateStream PES sessions bookmarks"));
        assert!(
            transform(&body).is_err(),
            "an already patched player is not patched again"
        );
    }
    #[test]
    fn a_player_the_ad_patches_do_not_fit_is_served_as_it_is() {
        let cases = [
            ("no-ads flag", "function e(e)", "function z(e)"),
            (
                "playlist converter",
                "pauseAdsResolution:t.",
                "pauseAdsResolution:x.",
            ),
            (
                "new-stack pause behavior",
                "e.playlistedPlaybackUrls",
                "e.other",
            ),
            ("ad-break resolver", "if(0===e.length)", "if(!e.length)"),
            (
                "live pre-rolls",
                ".showLivePreRolls.enabled",
                ".other.enabled",
            ),
        ];
        let mut broken = Vec::new();
        for (patch, from, to) in cases {
            broken.push((patch, PLAYER.replace(from, to)));
        }
        // a use of the name in a form not seen before, beside the three that are known
        let unseen = format!("{PLAYER}var z=true===e.showLivePreRolls.enabled;");
        broken.push(("live pre-rolls", unseen));
        for (patch, source) in broken {
            let reason = transform(&source).unwrap_err();
            assert!(
                reason.starts_with(&format!("{patch}:")),
                "{patch}: {reason}"
            );
        }
    }
    #[test]
    fn a_report_with_no_place_is_noted_and_the_ads_are_still_removed() {
        let out = transform(&PLAYER.replace("uploadMinervaKpiMetrics", "uploadOther")).unwrap();
        assert_eq!(out.missed, ["minerva kpi upload"]);
        assert!(out.body.contains("__HOMEBREW_NOADS||0===e.length"));
    }
    const WRAP: &str = "(__HOMEBREW_NOADS?!1:";
    /// The patched source for a filter setting: the wrapper is `(__HOMEBREW_NOADS?!1:read)`, so this is the program the player runs.
    fn resolved(js: &str, filter: bool) -> String {
        let mut out = String::new();
        let mut rest = js;
        while let Some(at) = rest.find(WRAP) {
            let tail = &rest[at + WRAP.len()..];
            let close = tail.find(')').unwrap();
            out.push_str(&rest[..at]);
            out.push_str(if filter { "!1" } else { &tail[..close] });
            rest = &tail[close + 1..];
        }
        out + rest
    }
    /// What `transform` makes of extra statements placed after the player.
    fn after_player(extra: &str) -> Result<String, String> {
        let body = transform(&format!("{PLAYER}\n;/*tail*/;{extra}"))?.body;
        Ok(body.split_once("/*tail*/;").unwrap().1.to_owned())
    }
    #[test]
    fn a_guarded_preference_read_is_disabled_while_filtering_and_unchanged_otherwise() {
        // the guard, the comparison and the short-circuit are the player's own in both polarities
        for (read, disabled) in [
            (
                "var a=null!=e.showLivePreRolls&&!0===e.showLivePreRolls.enabled&&y;",
                "var a=null!=e.showLivePreRolls&&!0===!1&&y;",
            ),
            (
                "var b=null==Ng.store.cfg.showLivePreRolls||!0!==Ng.store.cfg.showLivePreRolls.enabled;",
                "var b=null==Ng.store.cfg.showLivePreRolls||!0!==!1;",
            ),
            (
                "if(null!=a.b.showLivePreRolls&&true===a.b.showLivePreRolls.enabled)x();",
                "if(null!=a.b.showLivePreRolls&&true===!1)x();",
            ),
            (
                "function f(){return null==$.q.showLivePreRolls||true!==$.q.showLivePreRolls.enabled}",
                "function f(){return null==$.q.showLivePreRolls||true!==!1}",
            ),
        ] {
            let patched = after_player(read).unwrap();
            assert_eq!(patched.matches(WRAP).count(), 1, "{read}");
            assert_eq!(resolved(&patched, false), read, "filter off keeps the read");
            assert_eq!(resolved(&patched, true), disabled, "filter on reads it as disabled");
        }
    }
    #[test]
    fn a_use_that_is_not_the_guarded_read_rejects_the_whole_player() {
        let guarded = "null!=e.showLivePreRolls&&!0===e.showLivePreRolls.enabled";
        for source in [
            "e.showLivePreRolls.enabled=!0;".to_owned(),
            format!("var z={guarded}=1;"),
            format!("var z={guarded}++;"),
            format!("var z={guarded}Soon;"),
            format!("var z={guarded}.x;"),
            format!("var z={guarded}(1);"),
            format!("var z={guarded}&&null!=e.showLivePreRolls&&e.showLivePreRolls.enabled;"),
            "var z=null!=a.b.showLivePreRolls&&!0===a.c.showLivePreRolls.enabled;".to_owned(),
            "var z=null!=b.showLivePreRolls&&!0===a.b.showLivePreRolls.enabled;".to_owned(),
            "var z=null!=f().showLivePreRolls&&!0===f().showLivePreRolls.enabled;".to_owned(),
            format!("var z=x{guarded};"),
            format!("var z=\"{guarded}\";"),
        ] {
            let reason = after_player(&source).unwrap_err();
            assert!(reason.starts_with("live pre-rolls:"), "{source}: {reason}");
        }
    }
}
