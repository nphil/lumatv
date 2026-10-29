//! Test support: a [`Kit`] on the toolkit's recording compositor (`ui::testing`), with the shipped theme and the module's icons, so a
//! surface is built and its layers read in a unit test without GL. The harness must outlive the kit.

use super::Kit;
use lumatv_tokens::Tokens;
use std::path::PathBuf;
use ui::lucent::material::MaterialTier;
use ui::lucent::palette::Mode;

/// The module's directory in the repository (`payload/compositor/lumatv`).
pub fn module_dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../payload/compositor/lumatv"))
}

pub fn kit(text_scale: f32) -> (ui::testing::Harness, Kit) {
    kit_in(Mode::Dark, text_scale)
}

pub fn kit_in(mode: Mode, text_scale: f32) -> (ui::testing::Harness, Kit) {
    let h = ui::testing::harness();
    let mut kit = Kit::new(h.ui.scene.clone(), Tokens::shipped(), mode, MaterialTier::T1, text_scale).expect("a kit on the recording compositor");
    kit.icons = super::icon::Icons::new(&module_dir());
    (h, kit)
}
