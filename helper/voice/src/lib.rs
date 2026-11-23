//! A far-field microphone as a voice satellite's ears and mouth.
//!
//! The engine: ALSA capture (`arecord`) brought to 16 kHz mono, the microWakeWord detector, a turn that goes up only after a wake
//! word and the client's say-so, a player for the reply, and a server for all of it on the voice wire (`voice-wire`, LumaShow's
//! language for a sound service, so LumaShow's voice host drives it as it is). Nothing here knows of a TV's shell, a settings
//! page or Home Assistant; a standalone voice-satellite app puts a host on the other end of the socket.
//!
//! While the TV plays, its sound is read back from the TV's mixer as a reference and taken out of the microphone by an echo canceller
//! ([`echo`]), after the lag between the two streams has been read from the sound itself ([`delay`]).
//!
//! The microphone is opened when the client asks to listen and released when it mutes or goes. No sample leaves it before a wake
//! word, and the preroll that goes with a turn lives in memory only.

pub mod delay;
pub mod echo;
pub mod engine;
pub mod input;
pub mod level;
pub mod models;
pub mod pcm;
pub mod server;
pub mod speech;

pub use delay::Search;
pub use echo::{Canceller, EchoConfig, EchoState, EchoStatus};
pub use engine::{Engine, EngineConfig, Out};
pub use input::{Alsa, Card, Input};
pub use pcm::{Format, Frames, Pick, Sample};
pub use server::{Config, Handle, Status, Tap, start};
pub use speech::{Aplay, Speaker};
