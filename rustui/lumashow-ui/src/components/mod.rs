//! The public Lucent component toolkit (design section 3.3): the components every screen of every shell is made of. Each owns its
//! retained layers, takes a model and answers with an action, and gets its material from [`crate::lucent::smart_material`], never
//! a glass of its own. Shapes and glass are the compositor's; only text and icons are rastered, and only when they change.
//!
//! Layers move with the toolkit's [`Group`] (one place that turns a pose into layer properties) and a control presses with a
//! [`Surface`] (the prepared rest and pressed glass, crossfaded). Input is the host's [`crate::input::Event`]s; a control decides
//! press, cancel and activation itself with a [`PressTracker`].

pub mod button;
pub mod card;
pub mod choice;
pub mod clock_digits;
pub mod empty_state;
#[cfg(test)]
mod fallback_tests;
pub mod group;
pub mod icon;
pub mod keyboard;
pub mod label;
pub mod list;
pub mod mark;
pub mod overlay;
pub mod pager;
pub mod plate;
pub mod press;
pub mod progress;
pub mod row;
#[cfg(test)]
mod row_tests;
pub mod scroll;
pub mod semantics;
pub mod slider;
pub mod surface;
pub mod swipe;
pub mod text;
pub mod text_field;
pub mod text_layer;
pub mod toast;
pub mod toggle;

pub use button::{Button, ButtonAction, ButtonKind, ButtonModel};
pub use card::{Card, CardAction, CardModel, CardSize};
pub use choice::{
    Choice, ChoiceAction, ChoiceModel, ChoiceOption, Stepper, StepperAction, StepperModel,
};
pub use clock_digits::{ClockDigits, ClockFormat, DigitCells, TimeDigits};
pub use empty_state::{
    EmptyAction, EmptyKind, EmptyModel, EmptyState, StatusBadge, StatusBadgeModel, StatusKind,
};
pub use group::{Group, Kind, Movable, Part, Pose, Rest, pose_props};
pub use icon::{Icon, icon_part, paint, paint_over};
pub use keyboard::{KeyOutput, Keyboard, KeyboardModel};
pub use label::{Align, Block, Label, Style, style};
pub use list::{ListAction, ListModel, VirtualList};
pub use mark::{Form, Mark};
pub use overlay::{
    ActionKind, Dismissal, Options, Overlay, OverlayAction, OverlayKind, OverlayModel,
    OverlayResult,
};
pub use pager::{Pager, PagerAction, PagerLayout, PagerModel};
pub use plate::{CLEAR, backdrop, ring, shape, solid};
pub use press::{Change, PressSet, PressTracker, Step};
pub use progress::{Progress, ProgressModel};
pub use row::{
    Dress, DressStatus, Needs, Row, RowAction, RowId, RowLook, RowModel, Thumbnail, Trailing,
    ValueMark, ValueTone,
};
pub use semantics::{SemanticKind, SemanticNode, SemanticsSink};
pub use slider::{Slider, SliderAction, SliderModel};
pub use surface::Surface;
pub use text::{Run, draw_runs, ellipsise, fit_tail, line_height, minutes_seconds, wrap, wrap_to};
pub use text_field::{TextField, TextFieldAction, TextFieldModel};
pub use text_layer::{Item, TextLayer};
pub use toast::{Toast, ToastEvent, ToastModel};
pub use toggle::{Toggle, ToggleAction, ToggleModel};

use crate::context::Ui;
use crate::geometry::Frame;
use crate::input::Event;

/// What an update changed, so that the host knows whether anything has to be committed or prepared again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Damage {
    /// The model is what the component already shows.
    None,
    /// Only properties moved or restyled: no pixels were drawn.
    Properties,
    /// Content (text, an icon) was drawn again.
    Content,
}

/// The contract of a component (design section 3.3): a model in, an action out. `update` compares the model with what is shown and
/// touches only what differs; `input` takes the host's events and answers with an action when one completes; `frame` is where it
/// takes a touch; `semantics` describes it. Components own retained layers, so none of these rebuilds a tree.
pub trait Component {
    type Model;
    type Action;

    fn update(&mut self, model: &Self::Model, ui: &mut Ui) -> Damage;
    fn input(&mut self, event: &Event, ui: &mut Ui) -> Option<Self::Action>;
    fn frame(&self) -> Frame;
    fn semantics(&self, out: &mut SemanticsSink);
}

/// A touch target of at least `min` in each direction around `visual`: a chip drawn 40 high still takes a touch 64 high, centred.
pub fn target_around(visual: Frame, min: f32) -> Frame {
    let (w, h) = (visual.w.max(min), visual.h.max(min));
    let (cx, cy) = visual.centre();
    Frame::new(cx - w / 2.0, cy - h / 2.0, w, h)
}

#[cfg(test)]
mod movable_tests {
    use super::*;
    use crate::lucent::material::MaterialContext;
    use crate::testing::harness;
    use compositor_protocol::Props;

    fn shown(h: &crate::testing::Harness) -> Vec<Props> {
        h.layers().into_iter().map(|(_, _, p)| p).collect()
    }

    /// The page's pose reaches every layer of the component: the ones that show move by the same amount and the focus ring that
    /// is not wanted stays hidden, so a page cannot light up a ring or leave a layer behind.
    fn moves_with_the_page(h: &mut crate::testing::Harness, c: &mut dyn Movable) {
        h.commit();
        let before = shown(h);
        c.set_in((480.0, 240.0), Pose::at(120.0, -30.0));
        h.commit();
        let after = shown(h);
        assert_eq!(before.len(), after.len());
        for (b, a) in before.iter().zip(&after) {
            if !b.visible {
                assert!(!a.visible, "a hidden layer must stay hidden");
                continue;
            }
            assert_eq!((a.x - b.x, a.y - b.y), (120.0, -30.0), "{b:?} -> {a:?}");
            assert!(a.visible);
        }
        c.set_in((480.0, 240.0), Pose::REST);
        h.commit();
        let back = shown(h);
        for (b, r) in before.iter().zip(&back) {
            // A hidden layer's place is not drawn and may be brought up to date; every layer that shows is where it began.
            if b.visible {
                assert_eq!(b, r, "back at rest, a shown layer is where it began");
            } else {
                assert!(!r.visible);
            }
        }
    }

    #[test]
    fn a_button_a_toggle_and_a_slider_move_with_the_page_and_keep_their_ring_hidden() {
        let mut h = harness();
        let frame = Frame::new(100.0, 100.0, 200.0, 64.0);
        let mut button = Button::new(
            &mut h.ui,
            frame,
            10,
            ButtonKind::Standard,
            MaterialContext::Nested,
            &ButtonModel::labelled("Go"),
        )
        .unwrap();
        moves_with_the_page(&mut h, &mut button);

        let mut h = harness();
        let mut toggle = Toggle::new(
            &mut h.ui,
            100.0,
            100.0,
            10,
            &ToggleModel::new("Wi-Fi", true),
        )
        .unwrap();
        moves_with_the_page(&mut h, &mut toggle);

        let mut h = harness();
        let mut slider = Slider::new(
            &mut h.ui,
            Frame::new(100.0, 100.0, 300.0, 40.0),
            10,
            &SliderModel::new("Volume", 0.0, 100.0, 1.0, 40.0),
        )
        .unwrap();
        moves_with_the_page(&mut h, &mut slider);
    }
}
