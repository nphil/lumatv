//! Structural player transform derived from samoz83/AmazOff (MIT; payload/privacy/NOTICE).
//! Every anchor must match before a byte is served. Unknown builds stay unmodified, never receive an old player.
use fancy_regex::Regex;
use std::sync::LazyLock;

struct Patch { name: &'static str, anchor: &'static str, expression: Regex, replacement: &'static str, count: usize }
fn patches() -> &'static [Patch] {
    static PATCHES: LazyLock<Vec<Patch>> = LazyLock::new(|| {
        [
            ("no-ads flag", r#"("use strict";var [A-Za-z_$][\w$]*="undefined"!=typeof globalThis\?globalThis:"undefined"!=typeof window\?window:"undefined"!=typeof global\?global:"undefined"!=typeof self\?self:\{\};)(function e\(e\)\{)"#, "${1}var __HOMEBREW_NOADS=!0;$2", 1),
            ("playlist converter", r#"intraTitlePlaylist:([A-Za-z_$][\w$]*)\.playbackUrls\.intraTitlePlaylist,pauseAdsResolution:\1\.playbackUrls\.pauseAdsResolution,"#, "intraTitlePlaylist:__HOMEBREW_NOADS?$1.playbackUrls.intraTitlePlaylist.filter(function(x){return\"Remote\"!==x.type}).map(function(x){if(null==x.nonLinearAds)return x;var y=Object.assign({},x);delete y.nonLinearAds;return y}):$1.playbackUrls.intraTitlePlaylist,pauseAdsResolution:__HOMEBREW_NOADS?void 0:$1.playbackUrls.pauseAdsResolution,", 1),
            ("pause behavior", r#"pauseBehavior:([A-Za-z_$][\w$]*)\(([A-Za-z_$][\w$]*)\),playbackResourceExpirationTimeMs:"#, "pauseBehavior:__HOMEBREW_NOADS?void 0:$1($2),playbackResourceExpirationTimeMs:", 1),
            ("ad-break resolver", r#"(\.prototype\.resolveWithAdBreaks=freeOnUnref\(function\([A-Za-z_$][\w$]*,[A-Za-z_$][\w$]*\)\{var [A-Za-z_$][\w$]*=this;if\()(0===[A-Za-z_$][\w$]*\.length\))"#, "${1}__HOMEBREW_NOADS||$2", 1),
            ("unresolved content source", r#"case"Remote":(?=var [^;]{0,200}?this\.adBreakConfigBuilder\.build\()"#, "case\"Remote\":if(__HOMEBREW_NOADS)break;", 1),
            ("media events", r#"(\.prototype\.enqueueMediaEvents=freeOnUnref\(function\(e\)\{)(?=this\.applyDeferredEventTimestamp\(e\))"#, "${1}return;", 2),
            ("telemetry queue", r#"(\.prototype\.reportEvent=freeOnUnref\(function\(e\)\{var t=this;)(?=this\.queue\.add\(e\),this\.isQueueAtMaxDepth\(\))"#, "${1}return;", 1),
            ("minerva metrics", r#"(\.prototype\.recordMetric=freeOnUnref\(function\(e\)\{)(?=if\(null!=this\.minervaClient\)try\{this\.minervaClient\.record\(e\))"#, "${1}return;", 1),
            ("minerva kpi upload", r#"(\.prototype\.uploadMinervaKpiMetrics=freeOnUnref\(function\(\)\{)"#, "${1}return;", 1),
            ("client log upload", r#"(\.prototype\.write=freeOnUnref\(function\(e,t\)\{)(?=if\(!\(e<this\.logLevel\)\)\{var n=this\.toLogLine\()"#, "${1}return;", 1),
            ("playback diagnostics", r#"(\.prototype\.batchMessage=freeOnUnref\(function\(e,t\)\{)(?=1e3<=this\.batchedMessages\.length)"#, "${1}return;", 1),
            ("ad beacons", r#"(\.prototype\.send=freeOnUnref\(function\(e,t,n\)\{if\(void 0===t&&\(t=\{\}\),)this\.enableEventReporting(?=\)\{for\(var r=0)"#, "$1!1", 1),
            ("ad audit pings", r#"(\.prototype\.make(?:Direct)?AuditPingRequest=freeOnUnref\(function\([\w$]+,[\w$]+\)\{)"#, "${1}return;", 2),
        ].into_iter().zip(["\"use strict\";var ", "intraTitlePlaylist:", "pauseBehavior:", ".prototype.resolveWithAdBreaks=", "case\"Remote\":", ".prototype.enqueueMediaEvents=", ".prototype.reportEvent=", ".prototype.recordMetric=", ".prototype.uploadMinervaKpiMetrics=", ".prototype.write=", ".prototype.batchMessage=", ".prototype.send=", ".prototype.make"]).map(|((name, expression, replacement, count),anchor)| Patch {
            name, anchor, expression: Regex::new(&format!("^(?:{expression})")).expect("constant transform expression"), replacement, count
        }).collect()
    });
    &PATCHES
}

pub fn transform(source: &str) -> Result<String, String> {
    let mut edits = Vec::with_capacity(16);
    for patch in patches() {
        let mut count = 0;
        for (start, _) in source.match_indices(patch.anchor) {
            let mut end = (start + 2048).min(source.len());
            while !source.is_char_boundary(end) { end -= 1; }
            let Some(caps) = patch.expression.captures(&source[start..end]).map_err(|e| format!("{}: {e}", patch.name))? else { continue; };
            let m = caps.get(0).unwrap();
            let mut replacement = String::new();
            caps.expand(patch.replacement, &mut replacement);
            edits.push((start + m.start(), start + m.end(), replacement));
            count += 1;
        }
        if count != patch.count { return Err(format!("{}: expected {} anchors, got {count}", patch.name, patch.count)); }
    }
    edits.sort_unstable_by_key(|e| e.0);
    let capacity = edits.iter().fold(source.len(), |n, (a,b,s)| n + s.len() - (b-a));
    let mut out = String::with_capacity(capacity);
    let mut at = 0;
    for (start, end, replacement) in edits {
        if start < at { return Err("overlapping player transform anchors".into()); }
        out.push_str(&source[at..start]); out.push_str(&replacement); at = end;
    }
    out.push_str(&source[at..]);
    Ok(out)
}

pub fn player_route(path: &str) -> Option<&str> {
    let path = path.split('?').next()?;
    let route = path.strip_prefix("/family/lg/ATVUnfPlayerBundle-1.0/")?
        .strip_suffix("/js/ATVUnfPlayerBundle.js")?;
    (!route.is_empty() && route.len() <= 32 && route.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')).then_some(route)
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn routes_are_exact_not_substrings() {
        assert_eq!(player_route("/family/lg/ATVUnfPlayerBundle-1.0/wave3/js/ATVUnfPlayerBundle.js?v=1"),Some("wave3"));
        for p in ["/packages/SyeCorePlayerNativeModule-1.0/prod/ignite/module.so", "/licence", "/family/lg/ATVUnfPlayerBundle-1.0/../../js/ATVUnfPlayerBundle.js"] { assert_eq!(player_route(p), None); }
    }
    #[test] fn unknown_and_already_patched_bundles_fail_closed() {
        assert!(transform("var x=1;").unwrap_err().starts_with("no-ads flag:"));
    }
    #[test] fn deterministic_player_has_all_transforms_and_keeps_session_calls() {
        let source=include_str!("../tests/player.js");
        let out=transform(source).unwrap();
        assert!(out.contains("var __HOMEBREW_NOADS=!0;function e"));
        assert!(out.contains("__HOMEBREW_NOADS||0===e.length"));
        assert!(out.contains("untouched licence and UpdateStream PES sessions bookmarks"));
        assert!(out.contains("return;this.applyDeferredEventTimestamp"));
        assert!(transform(&out).is_err());
        assert!(transform(&source.replace("pauseAdsResolution:t.playbackUrls", "pauseAdsResolution:x.playbackUrls")).is_err());
    }
    #[test] fn research_bundles_match_reference_when_available() {
        let Ok(dir) = std::env::var("PRIVACY_REFERENCE_DIR") else { return; };
        for route in ["wave1", "wave3"] {
            let source = std::fs::read_to_string(format!("{dir}/live/{route}.js")).unwrap();
            let expected = std::fs::read_to_string(format!("{dir}/out/{route}/ATVUnfPlayerBundle.js")).unwrap();
            let actual = transform(&source).unwrap();
            assert_eq!(actual, expected);
            assert!(transform(&actual).is_err());
        }
        assert!(transform(&std::fs::read_to_string(format!("{dir}/live/onebox2.js")).unwrap()).is_err());
    }
}
