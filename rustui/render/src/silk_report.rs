//! Validates the Home silk override carried by a UI state report.
use compositor_protocol::LayerId;
use serde_json::Value;
use crate::{Look, Silk};

/// Missing, null, or malformed overrides clear the previous scoped setting.
pub fn parse(state: &Value) -> Option<(LayerId, Silk)> {
    let silk = state.get("home")?.get("silk")?.as_object()?;
    let id = u32::try_from(silk.get("id")?.as_u64()?).ok()?;
    let number = |key: &str, default: f32, min: f64, max: f64| -> Option<f32> {
        match silk.get(key) {
            None => Some(default),
            Some(value) => {
                let n = value.as_f64()?;
                (n.is_finite() && (min..=max).contains(&n)).then_some(n as f32)
            }
        }
    };
    Some((LayerId(id), Silk {
        amplitude: number("amplitude", 4.0, 0.0, 16.0)?,
        depth: number("depth", 0.25, 0.0, 1.0)?,
        scale: number("scale", 1.0, 0.5, 2.5)?,
        detail: number("detail", 0.35, 0.0, 1.0)?,
        look: Look {
            vignette: number("vignette", 0.0, 0.0, 1.0)?,
            specular: number("specular", 0.0, 0.0, 1.0)?,
            bloom: number("bloom", 0.0, 0.0, 1.0)?,
        },
    }))
}

/// Applies a complete state snapshot, requesting damage only for a change.
pub fn apply(engine: &mut crate::host::Engine, state: &Value) -> bool {
    match parse(state) {
        Some((id, silk)) => engine.set_silk(id, silk),
        None => engine.clear_silk(),
    }
}
