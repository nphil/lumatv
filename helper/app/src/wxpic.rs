//! What the weather view's pictures are on the wire. The shell says where its view is (the world point at its middle, the zoom, its size in
//! pixels) and what it wants of it (`GET /v1/wx/pic?k=map|backdrop|radar&cx=&cy=&zoom=&w=&h=&dark=&epoch=[&t=]`); the helper picks the level
//! and the cells that cover it ([`ViewQuery::plan`]) and answers one mosaic of whole cells as premultiplied RGBA in the compositor's own
//! format, row after row from the top left, `width * height * 4` bytes, followed by a [`TRAILER`] that says which cells they were.
//!
//! Raw and not a PNG: the shell makes a layer of the bytes it was given (no decode, no premultiply, on the shell's loop) and the helper does
//! no encode; over the loopback a 1920 by 1080 picture is a copy of 8 MB, where its PNG took 17 to 47 ms to make and 18 to 26 ms to read on
//! the host. The trailer is after the pixels so that the shell can cut it off without moving them.
//!
//! A **cell** is 256 by 256 pixels. The map's cells are cut at a grid zoom in half levels, `g`, and cover `2^-(g / 2)` of the world on a side
//! (the map crate's raster grid, which holds a picture to 0.84 to 1.19 times its size at rest); the radar's at a whole tile zoom `z` and
//! cover `2^-z`. Column `x` and row `y` count from the world's top left, and the mosaic is the cells `x0..=x1` by `y0..=y1`.
//!
//! **Which level.** The map's is the grid nearest the view's zoom, held to `10..=22` (zoom 5 to 11); its *backdrop* is the same view four grid
//! steps (two zoom levels) coarser, a handful of cells to keep under the map while the finer ones are drawn or the view has gone past
//! their edge, and there is none under zoom 7. The radar's tile zoom is the view's zoom less 3 (held to 2..=7: the radar's own sources have
//! a kilometre to a pixel at 7): one tile covers about a screen, so a frame is one to four of them and its pixels are about eight of the
//! screen's. `shell/home/radar_pics.rs` has the same three rules for what it keeps.

use crate::http::Request;

/// A cell's side in pixels.
pub const CELL_PX: usize = 256;
/// The most cells a mosaic may have: the map's view of the screen is up to 10 by 6 and a radar frame's up to 3 by 2.
pub const MAX_MAP_CELLS: usize = 80;
pub const MAX_RADAR_CELLS: usize = 16;
/// The trailer's length and its first four bytes.
pub const TRAILER: usize = 64;
pub const MAGIC: [u8; 4] = *b"WXP2";
/// The map's grid zooms (in half levels: 10 is zoom 5) and how many steps coarser its backdrop is.
pub const MAP_GRID: std::ops::RangeInclusive<u8> = 10..=22;
pub const BACKDROP_STEP: u8 = 4;
/// The radar's tile zooms, and how far under the view's zoom they are.
pub const RADAR_ZOOM: std::ops::RangeInclusive<u8> = 2..=7;
pub const RADAR_BELOW: f64 = 3.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Map,
    /// The map, coarser (a [`ViewQuery`]'s; its plan is a `Map` picture of the coarser grid).
    Backdrop,
    Radar,
}

/// The grid zoom of the map for a view at `zoom`.
pub fn map_grid(zoom: f64) -> u8 {
    (zoom * 2.0).round().clamp(f64::from(*MAP_GRID.start()), f64::from(*MAP_GRID.end())) as u8
}

/// The radar's tile zoom for a view at `zoom`.
pub fn radar_zoom(zoom: f64) -> u8 {
    (zoom - RADAR_BELOW).round().clamp(f64::from(*RADAR_ZOOM.start()), f64::from(*RADAR_ZOOM.end())) as u8
}

/// What the shell asks: a view, and what of it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewQuery {
    pub kind: Kind,
    /// The world point at the view's middle (0 to 1 each way, y south).
    pub cx: f64,
    pub cy: f64,
    pub zoom: f64,
    /// The view's size in pixels.
    pub w: f64,
    pub h: f64,
    /// The radar frame's time.
    pub t: i64,
    pub dark: bool,
    /// The view the shell asked in: a request of an older one is refused, and work for an older one is dropped.
    pub epoch: u64,
}

/// A mosaic: the cells of a level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PicQuery {
    /// `Map` or `Radar`.
    pub kind: Kind,
    /// The map's grid zoom (half levels) or the radar's tile zoom.
    pub zoom: u8,
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
    pub t: i64,
    pub dark: bool,
    pub epoch: u64,
}

impl ViewQuery {
    /// The query a request makes, or what is wrong with it.
    pub fn parse(request: &Request) -> Result<Self, &'static str> {
        let kind = match request.param("k") {
            Some("map") => Kind::Map,
            Some("backdrop") => Kind::Backdrop,
            Some("radar") => Kind::Radar,
            _ => return Err("k is map, backdrop or radar"),
        };
        let num = |k: &str| request.param(k).and_then(|v| v.parse::<f64>().ok()).filter(|v| v.is_finite());
        let (Some(cx), Some(cy), Some(zoom)) = (num("cx"), num("cy"), num("zoom")) else {
            return Err("cx, cy and zoom are needed");
        };
        if !(0.0..=1.0).contains(&cx) || !(0.0..=1.0).contains(&cy) || !(0.0..=24.0).contains(&zoom) {
            return Err("the view is not on the map");
        }
        let (w, h) = (num("w").unwrap_or(1920.0), num("h").unwrap_or(1080.0));
        if !(64.0..=4096.0).contains(&w) || !(64.0..=4096.0).contains(&h) {
            return Err("w and h are 64 to 4096");
        }
        let t = request.param("t").and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
        if kind == Kind::Radar && t == 0 {
            return Err("t is needed");
        }
        Ok(Self { kind, cx, cy, zoom, w, h, t, dark: request.param("dark") != Some("0"), epoch: request.param("epoch").and_then(|v| v.parse().ok()).unwrap_or(0) })
    }

    /// The cells that cover the view at the level asked for, or `None` when there is no such picture (no backdrop under zoom 7; a view off
    /// the map).
    pub fn plan(&self) -> Option<PicQuery> {
        let (kind, zoom) = match self.kind {
            Kind::Map => (Kind::Map, map_grid(self.zoom)),
            Kind::Backdrop => {
                let g = map_grid(self.zoom);
                (Kind::Map, g.checked_sub(BACKDROP_STEP).filter(|b| MAP_GRID.contains(b))?)
            }
            Kind::Radar => (Kind::Radar, radar_zoom(self.zoom)),
        };
        let mut q = PicQuery { kind, zoom, x0: 0, y0: 0, x1: 0, y1: 0, t: self.t, dark: self.dark, epoch: self.epoch };
        let cell = q.cell();
        let n = q.columns();
        let across = self.w / 2.0 / (CELL_PX as f64 * 2f64.powf(self.zoom));
        let down = self.h / 2.0 / (CELL_PX as f64 * 2f64.powf(self.zoom));
        let first = |c: f64, half: f64| ((c - half) / cell).floor().max(0.0) as i32;
        let last = |c: f64, half: f64| (((c + half) / cell - 1e-9).floor() as i32).min(n - 1);
        q.x0 = first(self.cx, across);
        q.x1 = last(self.cx, across);
        q.y0 = first(self.cy, down);
        q.y1 = last(self.cy, down);
        if q.x1 < q.x0 || q.y1 < q.y0 {
            return None;
        }
        let most = if kind == Kind::Map { MAX_MAP_CELLS } else { MAX_RADAR_CELLS };
        (q.cells() <= most).then_some(q)
    }
}

impl PicQuery {
    pub fn cells(&self) -> usize {
        (self.x1 - self.x0 + 1) as usize * (self.y1 - self.y0 + 1) as usize
    }

    pub fn width(&self) -> usize {
        (self.x1 - self.x0 + 1) as usize * CELL_PX
    }

    pub fn height(&self) -> usize {
        (self.y1 - self.y0 + 1) as usize * CELL_PX
    }

    /// A cell's side in world units.
    pub fn cell(&self) -> f64 {
        match self.kind {
            Kind::Radar => 2f64.powi(-i32::from(self.zoom)),
            _ => 2f64.powf(-f64::from(self.zoom) / 2.0),
        }
    }

    /// How many cells the world is across at this level (the map's grid at an odd half level has a last cell that is partly outside).
    pub fn columns(&self) -> i32 {
        (1.0 / self.cell() - 1e-9).ceil() as i32
    }

    /// The part of the world the mosaic covers: left, top, right, bottom.
    pub fn world(&self) -> [f64; 4] {
        let c = self.cell();
        [f64::from(self.x0) * c, f64::from(self.y0) * c, f64::from(self.x1 + 1) * c, f64::from(self.y1 + 1) * c]
    }

    /// Puts the trailer after the pixels: what the mosaic is, so that the shell needs none of this arithmetic.
    pub fn seal(&self, mut rgba: Vec<u8>, complete: bool, have: usize, want: usize) -> Vec<u8> {
        let (w, h) = (self.width(), self.height());
        debug_assert_eq!(rgba.len(), w * h * 4);
        rgba.reserve_exact(TRAILER);
        rgba.extend_from_slice(&MAGIC);
        rgba.extend_from_slice(&(w as u32).to_le_bytes());
        rgba.extend_from_slice(&(h as u32).to_le_bytes());
        rgba.extend_from_slice(&u32::from(complete).to_le_bytes());
        rgba.extend_from_slice(&(have.min(65_535) as u16).to_le_bytes());
        rgba.extend_from_slice(&(want.min(65_535) as u16).to_le_bytes());
        rgba.push(self.zoom);
        rgba.extend_from_slice(&[0; 3]);
        for v in self.world() {
            rgba.extend_from_slice(&v.to_le_bytes());
        }
        rgba.extend_from_slice(&[0; 8]);
        rgba
    }
}

/// What asking for a mosaic gets.
#[derive(Debug, PartialEq, Eq)]
pub enum PicAnswer {
    /// The pixels and their trailer.
    Ready(Vec<u8>),
    /// Some of the tiles are still to come (`have` of `want`; `failing` when one could not be had and is waited out): ask again soon.
    Coming { have: usize, want: usize, failing: bool },
    /// The radar has no frame at that time (the shell's list is older than the helper's).
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(query: &[(&str, &str)]) -> Request {
        Request { method: "GET".into(), path: "/v1/wx/pic".into(), query: query.iter().map(|(k, v)| ((*k).into(), (*v).into())).collect(), headers: Vec::new(), body: Vec::new(), peer: std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST) }
    }

    fn view(kind: Kind, zoom: f64) -> ViewQuery {
        // Atlanta
        ViewQuery { kind, cx: 0.2670, cy: 0.4116, zoom, w: 1920.0, h: 1080.0, t: 1_792_548_000, dark: true, epoch: 3 }
    }

    #[test]
    fn a_view_is_asked_for_by_its_middle_zoom_and_size_and_the_limits_hold() {
        let q = ViewQuery::parse(&request(&[("k", "map"), ("cx", "0.267"), ("cy", "0.4116"), ("zoom", "8.25"), ("epoch", "7"), ("dark", "0")])).unwrap();
        assert_eq!((q.kind, q.zoom, q.w, q.h, q.epoch, q.dark), (Kind::Map, 8.25, 1920.0, 1080.0, 7, false));
        let r = ViewQuery::parse(&request(&[("k", "radar"), ("cx", "0.2"), ("cy", "0.3"), ("zoom", "9"), ("t", "1792548000")])).unwrap();
        assert_eq!((r.kind, r.t, r.dark), (Kind::Radar, 1_792_548_000, true));
        for (bad, why) in [
            (vec![("k", "map"), ("cx", "0.2"), ("cy", "0.3")], "cx, cy and zoom are needed"),
            (vec![("k", "map"), ("cx", "1.5"), ("cy", "0.3"), ("zoom", "8")], "the view is not on the map"),
            (vec![("k", "map"), ("cx", "0.5"), ("cy", "0.3"), ("zoom", "NaN")], "cx, cy and zoom are needed"),
            (vec![("k", "map"), ("cx", "0.5"), ("cy", "0.3"), ("zoom", "8"), ("w", "9000")], "w and h are 64 to 4096"),
            (vec![("k", "radar"), ("cx", "0.5"), ("cy", "0.3"), ("zoom", "8")], "t is needed"),
            (vec![("k", "tile"), ("cx", "0.5"), ("cy", "0.3"), ("zoom", "8")], "k is map, backdrop or radar"),
        ] {
            assert_eq!(ViewQuery::parse(&request(&bad)), Err(why), "{bad:?}");
        }
    }

    #[test]
    fn the_levels_follow_the_zoom() {
        assert_eq!([4.0, 5.0, 7.5, 8.2, 9.0, 10.0, 11.0, 12.0].map(map_grid), [10, 10, 15, 16, 18, 20, 22, 22]);
        assert_eq!([4.0, 5.0, 6.0, 8.0, 9.4, 10.0, 12.0].map(radar_zoom), [2, 2, 3, 5, 6, 7, 7]);
        assert_eq!(view(Kind::Map, 9.0).plan().unwrap().zoom, 18);
        assert_eq!(view(Kind::Backdrop, 9.0).plan().unwrap().zoom, 14, "two zoom levels coarser");
        assert!(view(Kind::Backdrop, 6.0).plan().is_none(), "no backdrop under zoom 7");
        assert_eq!(view(Kind::Radar, 9.0).plan().unwrap().zoom, 6);
    }

    #[test]
    fn the_cells_cover_the_screen_and_a_screen_is_a_few_dozen_of_them() {
        for zoom in [5.0, 6.3, 7.0, 8.25, 9.0, 9.75, 11.0] {
            let q = view(Kind::Map, zoom).plan().unwrap();
            let [l, t, r, b] = q.world();
            let half = (960.0 / (256.0 * 2f64.powf(zoom)), 540.0 / (256.0 * 2f64.powf(zoom)));
            assert!(l <= 0.2670 - half.0 + 1e-9 && r >= 0.2670 + half.0 - 1e-9 && t <= 0.4116 - half.1 + 1e-9 && b >= 0.4116 + half.1 - 1e-9, "{zoom}: {:?} does not cover the screen", q.world());
            assert!((20..=MAX_MAP_CELLS).contains(&q.cells()), "{zoom}: {} cells", q.cells());
            // no more than a cell's width of what is not on the screen on each side
            assert!(l > 0.2670 - half.0 - q.cell() && r < 0.2670 + half.0 + q.cell());
        }
        for zoom in [5.0, 7.0, 8.0, 9.0, 10.0] {
            let q = view(Kind::Radar, zoom).plan().unwrap();
            assert!((1..=6).contains(&q.cells()), "radar at {zoom}: {} tiles", q.cells());
        }
        let b = view(Kind::Backdrop, 9.0).plan().unwrap();
        assert!(b.cells() <= 9, "{}", b.cells());
    }

    #[test]
    fn a_view_at_the_edge_of_the_world_is_held_inside_it() {
        // the north-west corner: no column or row below 0
        let q = ViewQuery { cx: 0.001, cy: 0.001, ..view(Kind::Map, 9.0) }.plan().unwrap();
        assert_eq!((q.x0, q.y0), (0, 0));
        // the south-east corner: no cell at or past the last of the grid
        let q = ViewQuery { cx: 0.999, cy: 0.999, ..view(Kind::Map, 9.0) }.plan().unwrap();
        assert_eq!((q.x1, q.y1), (q.columns() - 1, q.columns() - 1));
        // a half-level grid has a last cell that is partly outside the world
        assert_eq!(PicQuery { kind: Kind::Map, zoom: 17, x0: 0, y0: 0, x1: 0, y1: 0, t: 0, dark: true, epoch: 0 }.columns(), 363);
    }

    #[test]
    fn the_trailer_follows_the_pixels_and_comes_off_without_moving_them() {
        let q = PicQuery { kind: Kind::Radar, zoom: 6, x0: 17, y0: 26, x1: 18, y1: 26, t: 5, dark: true, epoch: 1 };
        let pixels = vec![9u8; 512 * 256 * 4];
        let sealed = q.seal(pixels.clone(), true, 5, 6);
        assert_eq!(sealed.len(), pixels.len() + TRAILER);
        assert_eq!(&sealed[..pixels.len()], &pixels[..]);
        let t = &sealed[pixels.len()..];
        let f64_at = |at: usize| f64::from_le_bytes(t[at..at + 8].try_into().unwrap());
        assert_eq!(&t[..4], b"WXP2");
        assert_eq!((u32::from_le_bytes(t[4..8].try_into().unwrap()), u32::from_le_bytes(t[8..12].try_into().unwrap())), (512, 256));
        assert_eq!(u32::from_le_bytes(t[12..16].try_into().unwrap()), 1);
        assert_eq!((u16::from_le_bytes(t[16..18].try_into().unwrap()), u16::from_le_bytes(t[18..20].try_into().unwrap()), t[20]), (5, 6, 6));
        let c = 1.0 / 64.0;
        assert_eq!([f64_at(24), f64_at(32), f64_at(40), f64_at(48)], [17.0 * c, 26.0 * c, 19.0 * c, 27.0 * c]);
    }
}
