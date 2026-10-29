//! The ring that turns while something loads (the launch screen, the voice bar's "working", the mini frame). A group pose has no rotation, so it
//! is a dozen dots on a circle with a head that steps round every [`SPIN_MS`](crate::system::extras::SPIN_MS), and a tail behind it that fades.

use crate::kit::Kit;
use compositor_protocol::{Props, Rgba};
use ui::components::plate;
use ui::geometry::Frame;
use ui::scene::ShapeLayer;

pub const DOTS: usize = 12;
/// The faintest dot of the tail, as a share of the head's opacity.
const TAIL: f32 = 0.2;

pub struct Spinner {
    dots: Vec<(ShapeLayer, Props)>,
    head: usize,
    shown: bool,
    frame: Frame,
}

impl Spinner {
    /// A ring of `side` by `side` whose top-left is (`x`, `y`), drawn in `ink`; `dot` is the diameter of a dot.
    pub fn new(kit: &Kit, x: f32, y: f32, side: f32, dot: f32, z: i32, ink: Rgba) -> Self {
        let radius = (side - dot) / 2.0;
        let centre = (x + side / 2.0, y + side / 2.0);
        let mut dots = Vec::with_capacity(DOTS);
        for i in 0..DOTS {
            let angle = (i as f32 / DOTS as f32) * std::f32::consts::TAU;
            let at = Props::at((centre.0 + radius * angle.sin() - dot / 2.0).round(), (centre.1 - radius * angle.cos() - dot / 2.0).round());
            let layer = plate::shape(&kit.ui, Frame::new(at.x, at.y, dot, dot), z, plate::solid(ink, dot / 2.0));
            dots.push((layer, at));
        }
        let mut me = Self { dots, head: usize::MAX, shown: true, frame: Frame::new(x, y, side, side) };
        me.step(0);
        me
    }

    pub fn frame(&self) -> Frame {
        self.frame
    }

    /// The head is at dot `n` (any number: it wraps). Returns whether anything moved.
    pub fn step(&mut self, n: usize) -> bool {
        let head = n % DOTS;
        if head == self.head || !self.shown {
            if !self.shown {
                self.head = head;
            }
            return false;
        }
        self.head = head;
        for (i, (layer, at)) in self.dots.iter().enumerate() {
            let age = (head + DOTS - i) % DOTS;
            let opacity = 1.0 - (1.0 - TAIL) * age as f32 / (DOTS - 1) as f32;
            layer.set(Props { opacity, ..*at });
        }
        true
    }

    /// The ring is on the screen or away (its head keeps its place).
    pub fn show(&mut self, on: bool) {
        if on == self.shown {
            return;
        }
        self.shown = on;
        if on {
            let head = std::mem::replace(&mut self.head, usize::MAX);
            self.step(head);
        } else {
            for (layer, at) in &self.dots {
                layer.set(Props { visible: false, opacity: 0.0, ..*at });
            }
        }
    }

    pub fn head(&self) -> usize {
        self.head
    }

    pub fn layers(&self) -> usize {
        self.dots.len()
    }
}
