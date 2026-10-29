//! `lumatv-ui`: LumaTV's shell logic, a 64-bit process without GL (`docs/research/rust-shell-plan.md`, section 2). It owns the
//! layers of the two stacks the engine inside the compositor draws (LumaShow's `compositor-protocol` over the socket pair the
//! compositor hands it), reads everything else over the UI channel and the helper's API, and gets its keys over the key socket.
//! The screens are built from the TV kit ([`kit`]); the system layer's screens are in [`system`]; [`shell`] is the system layer
//! as one app, with no I/O of its own, so it runs under a test and under the host harness (`src/main.rs --host`) as it runs on the
//! set.

pub mod home;
pub mod host;
pub mod kit;
pub mod shell;
pub mod system;
