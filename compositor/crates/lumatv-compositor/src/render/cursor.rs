//! The pointer's sprite: a white disc with a dark rim, drawn by the compositor itself at the Magic Remote's
//! position (LG draws its own pointer the same way, on the cursor plane). Composed into the frame for now; the
//! KMS cursor plane, which would move it without a repaint, waits for a measurement on the set.

use crate::Rect;

/// Sprite size in pixels; the hotspot is the centre.
pub const SIZE: i32 = 28;
const RADIUS: f32 = 12.0;
const RIM: f32 = 2.0;

/// The rectangle the sprite occupies for a pointer at `(x, y)`.
pub fn rect_at(x: i32, y: i32) -> Rect {
    Rect::new(x - SIZE / 2, y - SIZE / 2, SIZE, SIZE)
}

/// Premultiplied RGBA bytes, row-major, top-down: the disc's edge and rim are anti-aliased by distance from the
/// centre, so the sprite reads cleanly on any background.
pub fn sprite() -> Vec<u8> {
    let mut out = vec![0u8; (SIZE * SIZE * 4) as usize];
    let c = SIZE as f32 / 2.0;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let d = ((x as f32 + 0.5 - c).powi(2) + (y as f32 + 0.5 - c).powi(2)).sqrt();
            // Coverage of the whole disc (fill plus rim) and of the fill alone, each softened over one pixel.
            let outer = (RADIUS + 0.5 - d).clamp(0.0, 1.0);
            let inner = (RADIUS - RIM + 0.5 - d).clamp(0.0, 1.0);
            let a = outer;
            // Rim colour 0x202020 where the fill does not cover, white where it does; premultiplied by coverage.
            let fill = inner;
            let rim = (outer - inner).max(0.0);
            let r = fill * 1.0 + rim * 0.125;
            let i = ((y * SIZE + x) * 4) as usize;
            out[i] = (r * 255.0).round() as u8;
            out[i + 1] = (r * 255.0).round() as u8;
            out[i + 2] = (r * 255.0).round() as u8;
            out[i + 3] = (a * 255.0).round() as u8;
        }
    }
    out
}

/// The sprite as premultiplied 0xAARRGGBB, for the software renderer.
pub fn sprite_argb() -> Vec<u32> {
    sprite().chunks_exact(4).map(|p| (p[3] as u32) << 24 | (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_disc_is_white_in_the_middle_dark_at_the_rim_and_clear_outside() {
        let s = sprite_argb();
        let at = |x: i32, y: i32| s[(y * SIZE + x) as usize];
        assert_eq!(at(SIZE / 2, SIZE / 2), 0xffff_ffff);
        assert_eq!(at(0, 0), 0, "the corner is outside the disc");
        let rim = at(SIZE / 2, (SIZE as f32 / 2.0 - RADIUS + 1.0) as i32);
        assert_eq!(rim >> 24, 0xff, "the rim is opaque");
        assert!(rim & 0xff < 0x40, "the rim is dark: {rim:#x}");
        assert_eq!(rect_at(100, 200), Rect::new(100 - SIZE / 2, 200 - SIZE / 2, SIZE, SIZE));
    }
}
