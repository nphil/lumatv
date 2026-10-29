//! Glyph text over a long run of changes: three labels and two text layers say new words again and again, on glyph quads of one
//! atlas, beside the same five as pictures; after every round of changes what the run shows is compared with what a picture of the
//! same words is, pixel for pixel, whatever the atlas has done meanwhile.
//!
//! With an alphabet that is bounded the atlas never grows again after its first pass and nothing falls back. With one that is not
//! it fills up, and each layer that then needs a glyph it cannot have becomes a picture of its new words in the layer it was: the
//! words are never stale and never another's glyph.

use super::label::{Align, Label, style};
use super::text::Run;
use super::text_layer::{Item, TextLayer};
use crate::geometry::Frame;
use crate::testing::{Harness, harness};
use crate::text::{Type, Weight};
use compositor_protocol::{LayerId, Request, Rgba};

const INK: Rgba = Rgba(236, 236, 240, 255);
const INK2: Rgba = Rgba(150, 150, 165, 255);
const LABEL: Frame = Frame::new(4.0, 6.0, 240.0, 30.0);
const LAYER: Frame = Frame::new(8.0, 40.0, 240.0, 64.0);

/// Three sizes: each label says its words in one, each text layer has a line in two of them.
const TYPES: [Type; 3] = [
    Type {
        px: 13,
        weight: Weight::Regular,
    },
    Type {
        px: 16,
        weight: Weight::Medium,
    },
    Type {
        px: 22,
        weight: Weight::SemiBold,
    },
];

/// What the five layers say: three labels (one type each) and two text layers of two lines (two types each, in two colours).
#[derive(Clone, Default)]
struct Words {
    labels: [String; 3],
    lines: [[String; 2]; 2],
}

/// The items of text layer `which`: its two lines, at the two types it uses.
fn items(which: usize, lines: &[String; 2]) -> [Item<'_>; 2] {
    let at = [[TYPES[0], TYPES[1]], [TYPES[1], TYPES[2]]][which];
    [
        Run::left(&lines[0], at[0], INK, 2.0, 2.0, 230.0).into(),
        Run::left(&lines[1], at[1], INK2, 2.0, 34.0, 230.0).into(),
    ]
}

/// Glyph text and, beside it, the same five layers as pictures: a run is compared with the picture of the same words.
struct Rig {
    run: Harness,
    pic: Harness,
    labels: [(Label, Label); 3],
    layers: [(TextLayer, TextLayer); 2],
}

impl Rig {
    fn new(words: &Words) -> Self {
        let mut run = harness();
        run.ui.enable_glyph_text().unwrap();
        let mut pic = harness();
        // What each layer says is made ready and landed before the layer is made, as the shell's start does: made while the page
        // is busy with bands, a layer is a picture, and these must start as runs.
        let warm = |run: &mut Harness, ty: Type, text: &str| {
            let page = run.ui.text_atlas.as_ref().unwrap().view();
            page.prepare(&mut run.ui.fonts, ty, text).unwrap();
            page.pump();
            run.settle_atlas();
        };
        let labels = std::array::from_fn(|i| {
            warm(&mut run, TYPES[i], &words.labels[i]);
            let label = |h: &mut Harness| {
                Label::new(
                    &mut h.ui,
                    LABEL,
                    1,
                    style(TYPES[i], INK, Align::Left),
                    &words.labels[i],
                )
                .unwrap()
            };
            (label(&mut run), label(&mut pic))
        });
        let layers = std::array::from_fn(|i| {
            let items = items(i, &words.lines[i]);
            for item in &items {
                if let Item::Text(text) = item {
                    warm(&mut run, text.ty, text.text);
                }
            }
            let made = TextLayer::new(&mut run.ui, LAYER, 2, &items).unwrap();
            (made, TextLayer::new(&mut pic.ui, LAYER, 2, &items).unwrap())
        });
        let rig = Self {
            run,
            pic,
            labels,
            layers,
        };
        assert_eq!(rig.runs(), [true; 5], "they start as runs");
        rig
    }

    /// Which of the five are glyph runs now (the three labels, then the two text layers).
    fn runs(&self) -> [bool; 5] {
        let [a, b, c] = &self.labels;
        let [d, e] = &self.layers;
        [
            a.0.is_glyph_run(),
            b.0.is_glyph_run(),
            c.0.is_glyph_run(),
            d.0.is_glyph_run(),
            e.0.is_glyph_run(),
        ]
    }

    /// The five say `words`: one change each, nothing committed yet.
    fn say(&mut self, words: &Words) {
        for (i, (run, picture)) in self.labels.iter_mut().enumerate() {
            run.set(&mut self.run.ui, &words.labels[i]);
            picture.set(&mut self.pic.ui, &words.labels[i]);
        }
        for (i, (run, picture)) in self.layers.iter_mut().enumerate() {
            let items = items(i, &words.lines[i]);
            run.draw(&mut self.run.ui, &items);
            picture.draw(&mut self.pic.ui, &items);
        }
    }

    /// The ids of the five layers (as runs), the labels first.
    fn ids(&self) -> Vec<LayerId> {
        self.labels
            .iter()
            .map(|(run, _)| run.id())
            .chain(self.layers.iter().map(|(run, _)| run.id()))
            .collect()
    }

    /// Lets the atlas settle as the harness does between changes (the compositor answers every upload, the lists that waited for
    /// their glyphs go out), then compares each of the five with its picture. Panics, naming the layer, on the first that differs.
    fn same_as_pictures(&mut self, when: &str) {
        self.run.settle_atlas();
        let pictures: Vec<LayerId> = self
            .labels
            .iter()
            .map(|(_, picture)| picture.id())
            .chain(self.layers.iter().map(|(_, picture)| picture.id()))
            .collect();
        for (i, (run, picture)) in self.ids().into_iter().zip(pictures).enumerate() {
            let (shown, wanted) = (self.run.layer_pixels(run), self.pic.layer_pixels(picture));
            let differ = shown.iter().zip(&wanted).filter(|(a, b)| a != b).count();
            assert!(
                differ == 0 && shown.len() == wanted.len(),
                "layer {i} {when}: {differ} bytes differ from its picture"
            );
        }
    }

    /// Rows of the text page the shelves have used.
    fn page_rows(&self) -> u32 {
        self.run.ui.text_atlas.as_ref().unwrap().used_rows()
    }

    /// What the run harness was told about layer `id`, in order: how it was made, restyled, given quads, deleted.
    fn requests_of(&self, id: LayerId) -> Vec<Request> {
        self.run
            .all()
            .into_iter()
            .filter(|r| match r {
                Request::Create { id: of, .. }
                | Request::Restyle { id: of, .. }
                | Request::SetGlyphs { id: of, .. }
                | Request::Delete { id: of } => *of == id,
                _ => false,
            })
            .collect()
    }
}

/// Numbers that are the same every run.
struct Lcg(u64);

impl Lcg {
    fn below(&mut self, n: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as usize) % n
    }
}

#[test]
fn two_thousand_changes_of_words_in_a_bounded_alphabet_never_fall_back_and_never_grow_the_page_after_the_first_pass()
 {
    let alphabet: Vec<char> = (' '..='~').collect();
    assert_eq!(alphabet.len(), 95);
    let text = |chars: &[char]| chars.iter().collect::<String>();
    let words_of = |chunk: &[char]| {
        let (a, b) = chunk.split_at(chunk.len() / 2);
        Words {
            labels: [text(chunk), text(chunk), text(chunk)],
            lines: [[text(a), text(b)], [text(b), text(a)]],
        }
    };
    let mut rig = Rig::new(&words_of(&alphabet[..12]));
    // The first pass: the whole alphabet goes through every layer, so that every glyph of every type is prepared once.
    for (pass, chunk) in alphabet.chunks(12).enumerate() {
        rig.say(&words_of(chunk));
        rig.same_as_pictures(&format!("in the first pass, chunk {pass}"));
    }
    let rows = rig.page_rows();
    assert!(rows > 0 && rows < 1024, "{rows}");
    // Then 400 rounds of one change in each of the five: words of 2 to 14 characters of the same alphabet, at random.
    let mut dice = Lcg(0x2545_f491_4f6c_dd1d);
    let say = |dice: &mut Lcg| -> String {
        let len = 2 + dice.below(13);
        (0..len)
            .map(|_| alphabet[dice.below(alphabet.len())])
            .collect()
    };
    let mut changes = 0;
    for round in 0..400 {
        let words = Words {
            labels: std::array::from_fn(|_| say(&mut dice)),
            lines: std::array::from_fn(|_| std::array::from_fn(|_| say(&mut dice))),
        };
        rig.say(&words);
        changes += 5;
        rig.same_as_pictures(&format!("in round {round}"));
        assert_eq!(rig.page_rows(), rows, "the page grew in round {round}");
    }
    assert_eq!(changes, 2000);
    assert_eq!(rig.run.ui.glyph_text_fallbacks, 0);
    assert!(!rig.run.ui.text_atlas_full);
    assert_eq!(
        rig.runs(),
        [true; 5],
        "none of them ever stopped being a run"
    );
    // No layer was made again: each is the layer it was made as.
    for id in rig.ids() {
        let creates = rig
            .requests_of(id)
            .iter()
            .filter(|r| matches!(r, Request::Create { .. }))
            .count();
        assert_eq!(creates, 1, "{id:?}");
    }
}

#[test]
fn thousands_of_new_characters_fill_the_page_and_each_layer_then_becomes_a_picture_of_its_latest_words_and_never_shows_a_stale_or_wrong_glyph()
 {
    // Characters nobody has used yet, from U+0100 on: the Latin, Greek and Cyrillic the font has, then the box it draws for the
    // ones it has not. Each is a new glyph at the type it is said in, with a place of its own on the page.
    let mut fresh = (0x100u32..).filter_map(char::from_u32);
    let used = std::cell::Cell::new(0);
    let mut say = |n: usize| -> String {
        used.set(used.get() + n);
        fresh.by_ref().take(n).collect()
    };
    let words = |say: &mut dyn FnMut(usize) -> String| Words {
        labels: std::array::from_fn(|_| say(7)),
        lines: std::array::from_fn(|_| std::array::from_fn(|_| say(4))),
    };
    let mut rig = Rig::new(&words(&mut say));
    let (mut fell, mut rounds_after) = (0, 0);
    for round in 0..600 {
        let before = rig.runs();
        rig.say(&words(&mut say));
        rig.same_as_pictures(&format!("in round {round}"));
        let after = rig.runs();
        for (was, is) in before.into_iter().zip(after) {
            assert!(!is || was, "a picture went back to a run in round {round}");
            fell += usize::from(was && !is);
        }
        assert_eq!(rig.run.ui.glyph_text_fallbacks as usize, fell);
        if after == [false; 5] {
            // A few rounds more, as pictures: the words go on being right.
            rounds_after += 1;
            if rounds_after == 5 {
                break;
            }
        }
    }
    // The page filled up, and every one of the five became a picture, once: they are the same five layers.
    assert_eq!(rig.runs(), [false; 5]);
    assert_eq!((fell, rig.run.ui.glyph_text_fallbacks), (5, 5));
    assert!(rig.run.ui.text_atlas_full);
    assert!(used.get() > 2000, "{} characters", used.get());
    assert_eq!(rig.run.ui.scene.live_layers(), 5);
    for id in rig.ids() {
        let of = rig.requests_of(id);
        let made = of
            .iter()
            .rposition(|r| matches!(r, Request::Create { .. }))
            .unwrap();
        assert_eq!(
            of.iter()
                .filter(|r| matches!(r, Request::Create { .. }))
                .count(),
            2,
            "made as a run and made again as a picture: {id:?}"
        );
        assert!(
            of[made..]
                .iter()
                .all(|r| !matches!(r, Request::SetGlyphs { .. } | Request::Restyle { .. })),
            "quads or a palette were sent to the picture {id:?}"
        );
        assert!(!of.iter().any(|r| matches!(r, Request::Delete { .. })));
    }
}
