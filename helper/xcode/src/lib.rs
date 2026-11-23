//! The scheduler of an LG webOS TV's one video encoder, with no host in it.
//!
//! The TV has one hardware H.264 encoder and a handful of decoders, shared with the TV's own apps. `Engine` runs one supervised `lumatv-venc xcode`
//! child at a time and decides which job has it: a viewer watching now beats a recording, a recording beats the replay ring, and a job that loses the
//! hardware is paused with the reason, not failed. A gate (`Engine::hold`) lets the host close the hardware to jobs while an app needs the decoder; the
//! child is gone within a second of it closing, and the pause is reported once the hardware is free.
//!
//! The pieces:
//! - [`proto`] is the wire to the child (`venc/xcode_proto.h`), and [`Spawn`] / [`Proc`] the process behind it, so the engine runs against scripted
//!   children in tests;
//! - `sched` is the scheduler as a pure state machine on an injected clock; `engine` is the thread that runs it;
//! - `buffer` holds a job's output (a bounded queue, or a ring of whole groups of pictures for clips) and `io` the pipe threads.
//!
//! A job is fed with `Job::push` and read with `Job::pull`, and neither ever waits.

mod buffer;
mod engine;
mod io;
pub mod proc;
pub mod proto;
mod sched;
mod types;

pub use engine::{Engine, Job};
pub use proc::{ChildCfg, Exit, OsSpawn, Proc, Sig, Spawn};
pub use types::{
    Au, Clip, ClipError, Config, EngineState, Event, Fault, H264Profile, HoldInfo, JobId, JobInfo, JobKind, JobSpec, JobState, Limits, MAX_RING_BYTES, Out, Output,
    Priority, PushError, Reason, RecvError, ScreenPoint, Sink, Snapshot, Source, SourceCodec, StartError, Wait,
};
