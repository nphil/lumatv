//! In-app video: a trailer or a community video plays in LumaTV's frame app (`org.nphil.lumatv.frame`, an overlay web app) on the
//! TV's main hardware video window, never as a texture of LumaGame's. The frame loads the player page (`frame/player.html`) in mode
//! `video` over the still's card; Up there grows it to full screen and Down returns it, Back closes it and LumaGame has the keys again.
//! While the player fills the screen the frame keeps the file `flag` (a launch parameter, [`FULL_FLAG`] in LumaGame's data directory),
//! and LumaGame draws nothing: its window sits over the video plane, so any page left up would show through as a ghost over the video.
use crate::game::model::game::media::{VideoKind, VideoRect};
use serde_json::{Value, json};
use std::path::Path;

pub const FRAME_APP: &str = "org.nphil.lumatv.frame";
/// The player page (game/player), served by the LumaTV server. YouTube's embed needs a real http(s) page for its Referer: from
/// file:// inside the frame package it answered error 153 on the C4.
pub const PLAYER: &str = "https://lumatv.nateshome.net/player/";
/// YouTube's least embedded player size (16:9).
const MIN_W: f32 = 480.0;
const MIN_H: f32 = 270.0;
const SCREEN: VideoRect = VideoRect::FULL;
/// The frame's full-screen mark, in LumaGame's data directory.
pub const FULL_FLAG: &str = "video-full";

/// The frame's launch parameters for `source` of `kind` at `rect`; `flag` is the file it keeps while the player fills the screen.
pub fn launch(kind: VideoKind, source: &str, rect: VideoRect, flag: &Path) -> Value {
    let query = match kind {
        VideoKind::YouTube => format!("yt={}", encode(source)),
        VideoKind::Trailer => format!("hls={}", encode(source)),
    };
    json!({"url": format!("{PLAYER}?{query}"), "mode": "video", "rect": {"x": rect.x.round(), "y": rect.y.round(), "w": rect.w.round(), "h": rect.h.round()}, "flag": flag})
}

/// What closes the frame when LumaGame leaves with a video still up.
pub fn close() -> Value {
    json!({"close": true})
}

/// The still's card as a player: grown about its centre to at least 480x270 (YouTube's least) and kept on the screen.
pub fn card(x: f32, y: f32, w: f32, h: f32) -> VideoRect {
    let scale = (MIN_W / w).max(MIN_H / h).max(1.0);
    let (w, h) = ((w * scale).min(SCREEN.w), (h * scale).min(SCREEN.h));
    let (cx, cy) = (x + w / scale / 2.0, y + h / scale / 2.0);
    VideoRect { x: (cx - w / 2.0).clamp(0.0, SCREEN.w - w), y: (cy - h / 2.0).clamp(0.0, SCREEN.h - h), w, h }
}

const HEX: &[u8; 16] = b"0123456789ABCDEF";
/// Percent-encodes a query value (RFC 3986's unreserved characters stay).
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.extend(['%', HEX[(b >> 4) as usize] as char, HEX[(b & 15) as usize] as char]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_frame_gets_the_player_in_video_mode_at_the_card() {
        let p = launch(VideoKind::Trailer, "https://v.steam/a b.m3u8?t=1", VideoRect { x: 10.4, y: 20.0, w: 480.0, h: 270.0 }, Path::new("/d/video-full"));
        assert_eq!(p["url"], format!("{PLAYER}?hls=https%3A%2F%2Fv.steam%2Fa%20b.m3u8%3Ft%3D1"));
        assert_eq!((p["mode"].as_str(), p["rect"]["x"].as_f64(), p["flag"].as_str()), (Some("video"), Some(10.0), Some("/d/video-full")));
        assert_eq!(launch(VideoKind::YouTube, "dQw4w9WgXcQ", VideoRect::FULL, Path::new("/d/f"))["url"], format!("{PLAYER}?yt=dQw4w9WgXcQ"));
    }

    #[test]
    fn a_small_card_grows_to_youtubes_least_size_about_its_centre_and_stays_on_the_screen() {
        assert_eq!(card(100.0, 200.0, 360.0, 202.5), VideoRect { x: 40.0, y: 166.25, w: 480.0, h: 270.0 });
        assert_eq!(card(1700.0, 900.0, 240.0, 135.0), VideoRect { x: 1440.0, y: 810.0, w: 480.0, h: 270.0 });
        assert_eq!(card(600.0, 300.0, 640.0, 360.0), VideoRect { x: 600.0, y: 300.0, w: 640.0, h: 360.0 }, "large enough already");
    }
}
