//! TrueHD passthrough to an eARC receiver on LG webOS: readiness, arming, monitoring and restoration of LG's audio route.
//!
//! A player that holds an LG renderer (`/dev/lg/aren0`) asks the route to arm for it. The route configures LG's PCM bypass
//! converter, un-mixes the eARC output words and, once the stream has been flowing, connects ADEC1 to the main output so the TV's
//! eARC transmitter sends the encoded stream on. Every change is journalled first and fully put back when the session ends. See
//! [`route`] for the sequence and [`host`] for the rules about touching the sound card.
//!
//! Ported from dts_restore_plus (LGPL-2.1-or-later), see NOTICE. No dependency on the rest of LumaTV, so the route can ship as a
//! standalone tool.

pub mod firmware;
pub mod graph;
pub mod health;
pub mod host;
pub mod journal;
pub mod route;
pub mod sndout;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
