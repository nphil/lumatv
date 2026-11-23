//! What a game's page shows below its actions, per section. Owned by the game-page slice; F0 fixes the section names only.

/// A section of the game's page, which `Model::open_game` can open the page at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Activity,
    Community,
    Media,
    About,
}
