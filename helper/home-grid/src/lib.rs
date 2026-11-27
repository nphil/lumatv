// The TV UI owns this model and runs its tests; the helper uses the same geometry without linking the UI.
#[path = "../../../rustui/ui/src/home/grid.rs"]
pub mod grid;

pub fn shipped_dock_home() -> serde_json::Value {
    grid::dock_only(&grid::shipped())
}
