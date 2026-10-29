//! The display lab: a guided, on-screen look at what the display chip can do beyond LG's own settings, driven with the Magic Remote
//! (`docs/research/display-lab-result.md`). One short step at a time: a test card with the zone to look at marked, a line on what the
//! difference looks like, the current option labelled big (A or B, and its name), and an answer (which is better, or whether a look is
//! acceptable, or that the two cannot be told apart). The registers are the helper's (`helper/app/src/displaylab/`), behind a hard list,
//! a journal and a dead-man; this module only asks it, and leaves by asking it to put everything back.

pub mod card;
pub mod host;
pub mod model;
pub mod plan;
pub mod surface;
