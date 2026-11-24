//! What a client built with [`Request::ClipRadius`] does about a compositor that was built before it.
//!
//! The request is one a compositor that has not got it cannot take: its frame does not decode, and the connection ends. So the
//! client sends it only to a compositor that said it has it, in the last field of `Event::Welcome`. A `Welcome` from one built
//! before it is a frame without that field, which the client's own `Event` cannot read; [`old_welcome`] reads what is there.
//! In the other direction nothing is needed: a client built before the field reads the fields it knows from a frame that has one
//! more, and leaves the rest.

use serde::Deserialize;

/// `Event` as it was before `Welcome::clip_radius`, up to the one variant a client needs of it.
#[derive(Deserialize)]
enum OldEvent {
    Welcome {
        version: u32,
        panel_width: u32,
        panel_height: u32,
        refresh_hz: f32,
        smooth_first_full: bool,
    },
}

/// The `Event::Welcome` of a compositor without `ClipRadius`, read from the frame that the client's `Event` could not: `clip_radius` is
/// false in it. `None` when the frame is not a `Welcome` of that shape either.
pub fn old_welcome(frame: &[u8]) -> Option<crate::Event> {
    match postcard::from_bytes::<OldEvent>(frame).ok()? {
        OldEvent::Welcome { version, panel_width, panel_height, refresh_hz, smooth_first_full } => {
            Some(crate::Event::Welcome { version, panel_width, panel_height, refresh_hz, smooth_first_full, clip_radius: false })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Event;

    #[test]
    fn a_welcome_with_the_flag_is_read_by_a_client_without_it_and_one_without_it_by_a_client_with_it() {
        let new = postcard::to_stdvec(&Event::Welcome { version: 15, panel_width: 1920, panel_height: 1080, refresh_hz: 120.0, smooth_first_full: true, clip_radius: true }).unwrap();
        // A client built before the field has the five and takes a frame that has one more.
        assert!(postcard::from_bytes::<OldEvent>(&new).is_ok());
        // A compositor built before it: the frame is the same without the last byte, which a client with it cannot read as an `Event`.
        let old = &new[..new.len() - 1];
        assert!(postcard::from_bytes::<Event>(old).is_err());
        assert!(matches!(old_welcome(old), Some(Event::Welcome { clip_radius: false, smooth_first_full: true, .. })));
        assert!(matches!(postcard::from_bytes::<Event>(&new), Ok(Event::Welcome { clip_radius: true, .. })));
    }
}
