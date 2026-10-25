//! Area rectangles of the four histogram presets, copied from LG's published kernel header `v4l2-ext-vt.h`
//! (kdriver/include/webosext/v4l2-ext-renderer-header/linux/, "Set the histogram preset mode value"). Fractions of the
//! picture, (0, 0) top left. Preset 0 is plain video, 1 skips top and bottom bars, 2 skips left and right bars, 3 is
//! games (14 areas). Areas 14 and 15 are the same in presets 0 to 2: a strip along the top (14) and one on the left
//! (15) that stay put so bars can be seen going away.

use crate::zones::Rect;

pub(crate) const PRESET0: [Rect; 16] = [
    Rect::new(0.25, 0.0, 0.75, 1.0),
    Rect::new(0.2, 0.8, 0.8, 1.0),
    Rect::new(0.3, 0.3, 0.7, 0.7),
    Rect::new(0.1, 0.0, 0.4, 0.2),
    Rect::new(0.0, 0.0, 0.5, 1.0),
    Rect::new(0.0, 0.67, 0.29, 1.0),
    Rect::new(0.0, 0.24, 0.2, 0.76),
    Rect::new(0.0, 0.0, 0.3, 0.375),
    Rect::new(0.5, 0.0, 1.0, 1.0),
    Rect::new(0.7, 0.67, 1.0, 1.0),
    Rect::new(0.8, 0.24, 1.0, 0.76),
    Rect::new(0.35, 0.0, 0.65, 0.2),
    Rect::new(0.6, 0.0, 0.9, 0.2),
    Rect::new(0.7, 0.0, 1.0, 0.375),
    Rect::new(0.3, 0.0, 0.7, 0.1265),
    Rect::new(0.0, 0.3, 0.13, 0.7),
];
pub(crate) const PRESET1: [Rect; 16] = [
    Rect::new(0.25, 0.1265, 0.75, 0.8735),
    Rect::new(0.2, 0.7241, 0.8, 0.8735),
    Rect::new(0.3, 0.3506, 0.7, 0.6494),
    Rect::new(0.1, 0.1265, 0.4, 0.2759),
    Rect::new(0.0, 0.1265, 0.5, 0.8735),
    Rect::new(0.0, 0.62699, 0.29, 0.8735),
    Rect::new(0.0, 0.30578, 0.2, 0.69422),
    Rect::new(0.0, 0.1265, 0.3, 0.406625),
    Rect::new(0.5, 0.1265, 1.0, 0.8735),
    Rect::new(0.7, 0.62699, 1.0, 0.8735),
    Rect::new(0.8, 0.30578, 1.0, 0.69422),
    Rect::new(0.35, 0.1265, 0.65, 0.2759),
    Rect::new(0.6, 0.1265, 0.9, 0.2759),
    Rect::new(0.7, 0.1265, 1.0, 0.406625),
    Rect::new(0.3, 0.0, 0.7, 0.1265),
    Rect::new(0.0, 0.3, 0.13, 0.7),
];
pub(crate) const PRESET2: [Rect; 16] = [
    Rect::new(0.315, 0.0, 0.685, 1.0),
    Rect::new(0.278, 0.8, 0.722, 1.0),
    Rect::new(0.352, 0.3, 0.648, 0.7),
    Rect::new(0.204, 0.0, 0.426, 0.2),
    Rect::new(0.13, 0.0, 0.5, 1.0),
    Rect::new(0.13, 0.67, 0.3446, 1.0),
    Rect::new(0.13, 0.24, 0.278, 0.76),
    Rect::new(0.13, 0.0, 0.352, 0.375),
    Rect::new(0.5, 0.0, 0.87, 1.0),
    Rect::new(0.648, 0.67, 0.87, 1.0),
    Rect::new(0.722, 0.24, 0.87, 0.76),
    Rect::new(0.389, 0.0, 0.611, 0.2),
    Rect::new(0.574, 0.0, 0.796, 0.2),
    Rect::new(0.648, 0.0, 0.87, 0.375),
    Rect::new(0.3, 0.0, 0.7, 0.1265),
    Rect::new(0.0, 0.3, 0.13, 0.7),
];
pub(crate) const PRESET3: [Rect; 14] = [
    Rect::new(0.5, 0.1, 1.0, 0.9),
    Rect::new(0.3, 0.0, 0.7, 1.0),
    Rect::new(0.0, 0.1, 0.5, 0.9),
    Rect::new(0.33, 0.3, 0.67, 0.67),
    Rect::new(0.23, 0.74, 0.77, 1.0),
    Rect::new(0.66, 0.62, 1.0, 1.0),
    Rect::new(0.74, 0.27, 1.0, 0.73),
    Rect::new(0.7, 0.0, 1.0, 0.375),
    Rect::new(0.6, 0.0, 0.9, 0.2),
    Rect::new(0.35, 0.0, 0.65, 0.2),
    Rect::new(0.1, 0.0, 0.4, 0.2),
    Rect::new(0.0, 0.0, 0.3, 0.375),
    Rect::new(0.0, 0.27, 0.26, 0.73),
    Rect::new(0.0, 0.62, 0.34, 1.0),
];
