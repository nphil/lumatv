//! OpenNOW's `opennow-streamer-platform`, for the TV. The engine (`opennow-streamer-core`, pinned in gfn/engine/Cargo.toml) is
//! compiled unchanged against this crate in place of upstream's: the same type and function names, a runtime that hands encoded
//! units to lumatv-gfn-feeder instead of decoding them, and controllers from evdev instead of SDL.
//!
//! What the TV path cannot offer and says so: recording and replay (the frames never pass through this process), the microphone,
//! pointer lock, screenshots. What it reports to the core's capability resolver: H.265 8/10-bit 4:2:0 with HDR10, H.264 8-bit; AV1 is
//! listed as unavailable because the set caps it at 4K60 and decodes it slower than HEVC (docs/research/opennow-result.md).
mod feeder;
mod microphone;
mod pads;
mod recording;
mod runtime;
mod types;

pub use feeder::{FeederClient, FeederMessage, Hdr, Load, Status as FeederStatus, decode as decode_feeder_message, msg as feeder_msg};
pub use microphone::{EncodedMicrophoneFrame, MICROPHONE_FRAME_SAMPLES, MICROPHONE_SAMPLE_RATE, MicrophoneReceiver, MicrophoneSession, MicrophoneStatus};
pub use recording::{EncodedRecordingReceiver, ManualRecordingSummary, RecordingSummary, ReplaySnapshot, record_matroska, record_replay_matroska};
pub use runtime::{FeederConfig, MainThreadHost, MediaControl, MediaRuntime, MediaRuntimeControl, MediaSession, MediaSink, controller_rumble, create_runtime};
pub use types::{
    CapturedInput, CapturedInputQueue, CapturedInputSample, DecodeStageTimings, DecodeTimingsReport, EncodedFrame, MediaCodec,
    MediaColorQuality, MediaFeedback, MediaStreamConfig, MediaVideoCodec, PushOutcome, ShortcutChord, StreamShortcutAction,
    StreamShortcutBindings,
};

use opennow_streamer_protocol::{CodecCapability, DecodeDimensions, VideoBackendCapability};

pub const fn supports_audio_decode() -> bool {
    true
}

pub const fn supports_audio_output() -> bool {
    true
}

/// What the set decodes through NDL, as the core's session resolver reads it (opennow-core streamer.rs `codec_entry_decodes`):
/// an entry with colour lists is matched against them, HDR needs `hdrColorQualities`.
pub fn video_backends() -> Vec<VideoBackendCapability> {
    vec![VideoBackendCapability {
        backend: "ndl",
        platform: "webos",
        codecs: vec![
            CodecCapability {
                codec: "h265",
                available: true,
                color_qualities: Some(vec!["8bit_420", "10bit_420"]),
                hdr_color_qualities: Some(vec!["10bit_420"]),
                hdr_supported: Some(true),
                max_decode_dimensions: Some(DecodeDimensions { width: 4096, height: 2304 }),
                reason: None,
            },
            CodecCapability {
                codec: "h264",
                available: true,
                color_qualities: Some(vec!["8bit_420"]),
                hdr_color_qualities: Some(vec![]),
                hdr_supported: Some(false),
                max_decode_dimensions: Some(DecodeDimensions { width: 3840, height: 2160 }),
                reason: None,
            },
            CodecCapability {
                codec: "av1",
                available: false,
                color_qualities: Some(vec![]),
                hdr_color_qualities: Some(vec![]),
                hdr_supported: Some(false),
                max_decode_dimensions: Some(DecodeDimensions { width: 3840, height: 2160 }),
                reason: Some("the set caps AV1 at 4K60 and decodes it slower than HEVC; HEVC carries every mode"),
            },
        ],
        zero_copy_modes: vec!["video-plane"],
        available: true,
        reason: None,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_capability_report_names_hevc_hdr_and_refuses_av1() {
        let b = video_backends();
        assert_eq!(b.len(), 1);
        let v = serde_json::to_value(&b[0]).unwrap();
        assert_eq!(v["backend"], "ndl");
        assert_eq!(v["platform"], "webos");
        let h265 = &v["codecs"][0];
        assert_eq!(h265["codec"], "h265");
        assert_eq!(h265["hdrColorQualities"], serde_json::json!(["10bit_420"]));
        assert_eq!(h265["hdrSupported"], true);
        assert_eq!(h265["maxDecodeDimensions"]["width"], 4096);
        assert_eq!(v["codecs"][2]["available"], false);
    }
}
