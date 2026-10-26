//! The microphone API the engine compiles against. The TV has no microphone the stream could capture (the remote's voice key is
//! LG's voice service), so a session is never created; the types exist for the engine's signatures.
pub const MICROPHONE_SAMPLE_RATE: u32 = 48_000;
pub const MICROPHONE_FRAME_SAMPLES: usize = 480;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedMicrophoneFrame {
    pub opus: Vec<u8>,
    pub timestamp: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MicrophoneStatus {
    pub enabled: bool,
    pub error: Option<String>,
    pub dropped_pcm_frames: u64,
    pub dropped_encoded_frames: u64,
}

#[derive(Debug, Clone)]
pub struct MicrophoneReceiver;

impl MicrophoneReceiver {
    pub fn try_recv(&self) -> Result<Option<EncodedMicrophoneFrame>, String> {
        Err("no microphone on the TV".to_owned())
    }

    pub fn status(&self) -> MicrophoneStatus {
        MicrophoneStatus { enabled: false, error: Some("no microphone on the TV".to_owned()), ..Default::default() }
    }

    pub fn stop(&self) {}
}

#[derive(Debug)]
pub struct MicrophoneSession;

impl MicrophoneSession {
    pub fn receiver(&self) -> MicrophoneReceiver {
        MicrophoneReceiver
    }

    pub fn try_recv(&mut self) -> Result<Option<EncodedMicrophoneFrame>, String> {
        MicrophoneReceiver.try_recv()
    }

    pub fn status(&mut self) -> MicrophoneStatus {
        MicrophoneReceiver.status()
    }

    pub fn stop(&mut self) {}
}
