//! Input as the toolkit sees it (design section 1.2): touch contacts in logical units on a monotonic clock, resampled to one
//! position per displayed frame, recognised as gestures, and routed by focus. The host adapter turns its device's events into
//! [`TouchEvent`]s once, dividing coordinates by the display scale; nothing here knows a device.

pub mod focus;
pub mod gesture;
pub mod resample;

pub use focus::{Focus, FocusId};
pub use gesture::{Gesture, Recognizer};
pub use resample::Resampler;

/// What a contact did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchPhase {
    Down,
    Move,
    Up,
    Cancel,
}

/// One change of one finger. Every contact is `Down`, any number of `Move`, then `Up` or `Cancel`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TouchEvent {
    /// Identifies the contact from its `Down` to its `Up` or `Cancel`. Two live contacts never share one.
    pub id: u32,
    pub phase: TouchPhase,
    /// Logical (upright, scaled) units.
    pub x: f32,
    pub y: f32,
    /// A monotonic clock in nanoseconds, the one the compositor timestamps frames with.
    pub at_ns: u64,
}

/// What a shell is told: a raw contact, or what the recognizer made of the contacts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    Touch(TouchEvent),
    Gesture(Gesture),
}
