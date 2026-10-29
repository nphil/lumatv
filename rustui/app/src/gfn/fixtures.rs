//! The states the layout checks and the renders go through: every screen the app has, in the state that fills it most.
use crate::gfn::app::{App, Game, Screen, Stats};
use serde_json::json;

pub const TITLES: [&str; 6] = ["Portal 2", "The Witcher 3: Wild Hunt - Complete Edition", "Counter-Strike 2", "Cyberpunk 2077", "Supercalifragilisticexpialidocious Adventures", "Hades"];

/// A QR-shaped matrix of `n` rows ("1" is a dark module): the three finder squares and a pseudo-random fill, the shape the core's
/// `qrRows` has, for the sign-in screen's render.
pub fn fake_qr(n: usize) -> Vec<String> {
    let mut seed = 0x2545_F491u32;
    (0..n)
        .map(|y| {
            (0..n)
                .map(|x| {
                    let finder = |cx: usize, cy: usize| {
                        let (dx, dy) = (x.abs_diff(cx), y.abs_diff(cy));
                        let d = dx.max(dy);
                        (d <= 3).then_some(d != 2)
                    };
                    let corner = [finder(3, 3), finder(n - 4, 3), finder(3, n - 4)].into_iter().flatten().next();
                    seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    if corner.unwrap_or(seed >> 31 == 1) { '1' } else { '0' }
                })
                .collect()
        })
        .collect()
}

pub fn games(n: usize) -> Vec<Game> {
    (0..n)
        .map(|i| Game {
            id: format!("g{i}"),
            variant_id: format!("v{i}"),
            title: TITLES[i % 6].into(),
            image_url: format!("https://x/{i}.jpg"),
            store: "STEAM".into(),
            in_library: true,
        })
        .collect()
}

/// Every screen the app has, in a state that fills it: the longest words the rules produce.
pub fn every_screen() -> Vec<(&'static str, App)> {
    let mut out = Vec::new();
    let base = || {
        let mut a = App::new();
        a.signed_in = true;
        a.user_name = "Nitin Phil".into();
        a.tier = "ULTIMATE".into();
        a.games = games(40);
        a.runtime_capabilities = json!({"nativeHdrSupported": true});
        a
    };
    let mut a = App::new();
    a.screen = Screen::Starting;
    out.push(("starting", a));
    let mut a = base();
    a.screen = Screen::SignIn;
    a.sign_in.user_code = "ABCD-EFGH".into();
    a.sign_in.url = "https://www.nvidia.com/en-us/geforce-now/activate".into();
    a.sign_in.status = "pending".into();
    a.sign_in.qr_rows = fake_qr(33);
    out.push(("signin", a));
    let mut a = base();
    a.screen = Screen::Library;
    out.push(("library", a));
    let mut a = base();
    a.screen = Screen::Library;
    a.focus = 17;
    a.first_row = 2;
    out.push(("library-scrolled", a));
    let mut a = base();
    a.screen = Screen::Library;
    a.header_focus = true;
    out.push(("library-header", a));
    let mut a = base();
    a.screen = Screen::Library;
    a.games.clear();
    out.push(("library-empty", a));
    for (i, name) in ["settings", "settings-hdr", "settings-bitrate", "settings-deadzone", "settings-stats", "settings-signout"].into_iter().enumerate() {
        let mut a = base();
        a.screen = Screen::Settings;
        a.settings_row = i;
        out.push((name, a));
    }
    for i in [0usize, 2, 4, 5] {
        let mut a = base();
        a.screen = Screen::Settings;
        a.settings_row = i;
        a.help_open = true;
        out.push((["settings-help-stream", "", "settings-help-bitrate", "", "settings-help-stats", "settings-help-signout"][i], a));
    }
    for row in [0usize, 1] {
        let mut a = base();
        a.screen = Screen::SignOut;
        a.confirm_row = row;
        out.push((["signout-keep", "signout-confirm"][row], a));
    }
    let mut a = base();
    a.screen = Screen::Detail;
    a.detail = Some(games(2)[1].clone());
    a.decision = "ownership_required".into();
    out.push(("detail-blocked", a));
    let mut a = base();
    a.screen = Screen::Detail;
    a.detail = Some(games(2)[1].clone());
    a.decision = "ready".into();
    out.push(("detail-ready", a));
    let mut a = base();
    a.screen = Screen::Launching;
    a.detail = Some(games(2)[1].clone());
    out.push(("launching", a));
    let mut a = base();
    a.screen = Screen::Stream;
    a.now_ms = 1000;
    a.chrome_until_ms = 5000;
    a.stats = Stats { status: "streaming".into(), fps: 119.6, bitrate_mbps: 78.4, input_ready: false, queue_drops: 12345, ..Default::default() };
    out.push(("stream", a));
    let mut a = base();
    a.screen = Screen::Overlay;
    a.prefs.show_stats = true;
    out.push(("overlay", a));
    let mut a = base();
    a.screen = Screen::Error;
    a.error = "The session could not be started because the service says there is no seat for this game right now (upstream)".into();
    out.push(("error", a));
    out
}
