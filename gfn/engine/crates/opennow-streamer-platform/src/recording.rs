//! Recording and replay as the engine's signatures want them. Both need the encoded units kept in this process; the TV build keeps
//! nothing (the pipeline owns the frames and there is no screenshot path), so a subscription is refused with a message the UI can
//! show, and these functions are never reached with a receiver or a snapshot. They answer the same way if they are.
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use opennow_streamer_protocol::RecordingCompletion;

use crate::types::MediaStreamConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordingSummary {
    pub path: PathBuf,
    pub video_packets: u64,
    pub audio_packets: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualRecordingSummary {
    pub media: RecordingSummary,
    pub completion: RecordingCompletion,
}

#[derive(Debug)]
pub struct EncodedRecordingReceiver;

#[derive(Debug)]
pub struct ReplaySnapshot;

const UNAVAILABLE: &str = "recording is not available on the TV: the decoded stream stays in LG's pipeline";

pub fn record_matroska(_path: &Path, _stream: MediaStreamConfig, _receiver: EncodedRecordingReceiver) -> Result<ManualRecordingSummary, String> {
    Err(UNAVAILABLE.to_owned())
}

pub fn record_replay_matroska(
    _path: &Path,
    _stream: MediaStreamConfig,
    _snapshot: ReplaySnapshot,
    _cancelled: &Arc<AtomicBool>,
) -> Result<RecordingSummary, String> {
    Err(UNAVAILABLE.to_owned())
}
