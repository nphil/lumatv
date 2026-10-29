//! A test picture of a size other than the mode's on the primary plane for a while (`ctl pageprobe WxH SECONDS`): what the OSD scaler makes
//! of a smaller buffer, the measurement a smaller composed page depends on (`docs/research/layer-deep-result.md`, "Reduced primary buffer").
//! The driver sets the OSD's input window from the framebuffer's size and the firmware keeps the output at the panel, so a 1280x720 buffer
//! is magnified about x3 where a 1920x1080 one is magnified x2. Never automatic: a probe is started over the control socket, shows for the
//! seconds asked, and the ring's page is back on the plane by itself after them (or at `ctl pageprobe off`). The rules and the picture are
//! here, pure; the buffer, the commit and the clock are the KMS output's (`kms/mod.rs`).

/// The picture's grid pitch in buffer pixels: counting cells across the panel gives the magnification.
pub const GRID: u32 = 32;
pub const MIN_SECONDS: u32 = 1;
pub const MAX_SECONDS: u32 = 120;

/// Whether a probe of `size` for `seconds` may be made on a plane whose mode is `mode`: no larger than the mode, a width the OSD's
/// 16-byte-aligned fetch takes (`osd_hdr_wpl` counts 8-byte words; LG's board options go down to 1280x720), an even height, and a
/// duration the dead-man can be trusted with.
pub fn check(mode: (u32, u32), size: (u32, u32), seconds: u32) -> Result<(), String> {
    let (w, h) = size;
    if w == 0 || h == 0 || w > mode.0 || h > mode.1 {
        return Err(format!("{w}x{h} is not within the mode's {}x{}", mode.0, mode.1));
    }
    if w % 16 != 0 || h % 2 != 0 {
        return Err(format!("{w}x{h}: the width must be a multiple of 16 and the height even"));
    }
    if w < 256 || h < 144 {
        return Err(format!("{w}x{h} is too small to read on the panel (256x144 at least)"));
    }
    if !(MIN_SECONDS..=MAX_SECONDS).contains(&seconds) {
        return Err(format!("{seconds} s: the probe shows for {MIN_SECONDS} to {MAX_SECONDS} s"));
    }
    Ok(())
}

/// Writes the picture into an XR24 buffer of `w` x `h` with rows `pitch` bytes apart (`out` holds at least `pitch * h` bytes): a dark
/// ground with a one-pixel white grid every [`GRID`] pixels and a white border (magnification and the scaler's ringing), a block of
/// one-pixel black and white stripes at the top left (the hardest case for a scaler), a one-pixel diagonal (aliasing), red, green and
/// blue squares in the middle (colour and edge sharpness), and a grey ramp along the bottom (banding).
pub fn fill(w: u32, h: u32, pitch: u32, out: &mut [u8]) {
    let (w, h, pitch) = (w as usize, h as usize, pitch as usize);
    assert!(out.len() >= pitch * h, "the buffer is smaller than its rows");
    let grid = GRID as usize;
    let stripes = (grid * 2).min(w / 4);
    let square = (h / 6).max(8);
    let squares_y = (h - square) / 2;
    let squares_x = [w / 4 - square / 2, w / 2 - square / 2, 3 * w / 4 - square / 2];
    let ramp_top = h - (h / 10).max(8);
    for y in 0..h {
        let row = &mut out[y * pitch..y * pitch + w * 4];
        for x in 0..w {
            let edge = x == 0 || y == 0 || x == w - 1 || y == h - 1;
            let (r, g, b) = if y >= ramp_top {
                let v = (x * 255 / (w - 1).max(1)) as u8;
                (v, v, v)
            } else if x < stripes && y < stripes {
                if x % 2 == 0 { (255, 255, 255) } else { (0, 0, 0) }
            } else if let Some(i) = squares_x.iter().position(|&sx| x >= sx && x < sx + square && y >= squares_y && y < squares_y + square) {
                [(255, 0, 0), (0, 255, 0), (0, 0, 255)][i]
            } else if edge || x % grid == 0 || y % grid == 0 || x == y * w / h {
                (255, 255, 255)
            } else {
                (32, 32, 32)
            };
            row[x * 4..x * 4 + 4].copy_from_slice(&[b, g, r, 0xff]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rules_keep_the_probe_within_the_mode_and_the_fetch_alignment() {
        let mode = (1920, 1080);
        assert_eq!(check(mode, (1280, 720), 10), Ok(()));
        assert_eq!(check(mode, (1440, 810), 10), Ok(()));
        assert_eq!(check(mode, (1920, 1080), 1), Ok(()), "the mode's own size is a control");
        assert!(check(mode, (1920, 1088), 10).unwrap_err().contains("within the mode"));
        assert!(check(mode, (1000, 720), 10).unwrap_err().contains("multiple of 16"));
        assert!(check(mode, (1280, 721), 10).unwrap_err().contains("even"));
        assert!(check(mode, (128, 64), 10).unwrap_err().contains("too small"));
        assert!(check(mode, (1280, 720), 0).unwrap_err().contains("1 to 120"));
        assert!(check(mode, (1280, 720), 121).is_err());
    }

    fn pixel(buf: &[u8], pitch: usize, x: usize, y: usize) -> (u8, u8, u8) {
        let p = &buf[y * pitch + x * 4..y * pitch + x * 4 + 4];
        assert_eq!(p[3], 0xff, "XR24's unused byte is written opaque");
        (p[2], p[1], p[0])
    }

    #[test]
    fn the_picture_has_its_grid_stripes_squares_and_ramp_where_the_photo_is_read() {
        let (w, h, pitch) = (1280u32, 720u32, 1280 * 4 + 64);
        let mut buf = vec![0u8; pitch as usize * h as usize];
        fill(w, h, pitch, &mut buf);
        let px = |x, y| pixel(&buf, pitch as usize, x, y);
        let white = (255, 255, 255);
        assert_eq!(px(0, 300), white, "border");
        assert_eq!(px(1279, 300), white);
        assert_eq!(px(640, 0), white);
        assert_eq!(px(GRID as usize * 10, 200), white, "a grid line every GRID pixels");
        assert_eq!(px(GRID as usize * 10 + 1, 201), (32, 32, 32), "the ground beside it");
        assert_eq!((px(10, 10), px(11, 10)), (white, (0, 0, 0)), "one-pixel stripes at the top left");
        let square = (h as usize / 6).max(8);
        let sy = (h as usize - square) / 2 + square / 2;
        assert_eq!(px(w as usize / 4, sy), (255, 0, 0), "red square at a quarter");
        assert_eq!(px(w as usize / 2, sy), (0, 255, 0));
        assert_eq!(px(3 * w as usize / 4, sy), (0, 0, 255));
        let ramp: Vec<u8> = (0..w as usize).step_by(64).map(|x| px(x, h as usize - 2).0).collect();
        assert!(ramp.windows(2).all(|p| p[0] <= p[1]) && ramp[0] < ramp[ramp.len() - 1], "the ramp rises left to right: {ramp:?}");
        assert!(buf[pitch as usize - 64..pitch as usize].iter().all(|&b| b == 0), "the pitch's padding is untouched");
    }
}
