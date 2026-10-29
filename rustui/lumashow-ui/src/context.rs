//! What every component is built with: the scene to put layers in, the fonts that rasterise its text, and the look it is
//! drawn in (the resolved theme and the effect tier). One value, made once by the host and lent to whatever builds or updates a
//! component; there is no global theme.

use crate::components::Form;
use crate::lucent::material::{
    ControlState, MaterialContext, MaterialPair, MaterialTier, ResolvedTheme, SmartMaterial,
    ThemeGenerationId,
};
use crate::lucent::palette::{Mode, PaletteRole};
use crate::lucent::resolve::default_theme;
use crate::lucent::smart_material;
use crate::lucent::tokens::{PAGE_SCALE, motion, radius};
use crate::scene::Scene;
use crate::text::{Fonts, Type, Weight};
use compositor_protocol::{Glass, Rgba};

/// The look components are drawn in: a resolved theme, the generation that names it, the effect tier and the reduced-motion
/// preference. Everything a component prepares (material, text colour, icon) is prepared for one generation; when the host
/// installs another look, components restyle or are rebuilt, and no stale one is told from a current one by its colours.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub theme: ResolvedTheme,
    pub generation: ThemeGenerationId,
    pub tier: MaterialTier,
    pub reduced_motion: bool,
    /// The person's text size, 1 to 2 (100 to 200 percent): [`Look::ty`] scales a type role by it.
    pub text_scale: f32,
    /// Whether the thing being made or drawn is a page of the Settings, which is set in the approved mock-ups' type and room
    /// (a button's label in 22 px SemiBold with 24 at each side: `Button`, the confirmation of `Overlay`). The settings view sets it
    /// together with the text size it draws at ([`PAGE_SCALE`] times the person's), and everything it makes is made inside it.
    pub page: bool,
}

impl Look {
    pub fn new(theme: ResolvedTheme, generation: ThemeGenerationId, tier: MaterialTier) -> Self {
        Self {
            theme,
            generation,
            tier,
            reduced_motion: false,
            text_scale: 1.0,
            page: false,
        }
    }

    /// `ty` at the person's text size: the role's pixel size times the scale, rounded up so that a floor is never undercut.
    /// A component measures with this type, sizes what it must contain to it, and never shrinks a label to fit.
    pub fn ty(&self, ty: Type) -> Type {
        Type {
            px: (ty.px as f32 * self.text_scale.clamp(1.0, 2.0)).ceil() as u32,
            ..ty
        }
    }

    /// A type of `px` pixels as the approved Settings mock-ups set it: `px` is the size at the Settings pages' own text size (the
    /// toolkit's roles at [`PAGE_SCALE`]), and it follows the person's text size from there, so a mock's 21 is 21 at the default and
    /// 32 at 150 %. For the sizes of the mock-ups that no role has (a permission's 21 and 18, a dialog's 30). Meant for what is made
    /// inside the settings view's text size; the size is rounded to the nearest pixel (it is a design size, not a floor).
    pub fn page_ty(&self, px: u32, weight: Weight) -> Type {
        let own = self.text_scale.clamp(1.0, 2.0) / PAGE_SCALE;
        Type {
            px: ((px as f32 * own).round() as u32).max(1),
            weight,
        }
    }

    /// The default theme (Rosé Pine) in `mode`, at the baseline tier: what a screen is drawn with until an appearance service
    /// installs the chosen one.
    pub fn standard(mode: Mode) -> Self {
        Self::new(default_theme(mode), ThemeGenerationId(1), MaterialTier::T1)
    }

    pub fn mode(&self) -> Mode {
        self.theme.mode
    }

    /// The material of `role` in `state` for this look: the pair a component prepares once and crossfades.
    pub fn material(
        &self,
        role: SmartMaterial,
        state: &ControlState,
        context: MaterialContext,
    ) -> MaterialPair {
        // In the Settings' page look a chip that is in (a day of the week) is the accent with its corners as they were, as the
        // approved mock-up's `.days span.on`: without the check the toolkit's chip shows, the wash it has is all that tells it.
        if self.page && role == SmartMaterial::StatusButton && state.selected {
            let accent = smart_material(
                SmartMaterial::PrimaryButton,
                state,
                context,
                &self.theme,
                self.tier,
            );
            return MaterialPair {
                rest: Glass {
                    corner_radius: radius::PILL,
                    ..accent.rest
                },
                pressed: Glass {
                    corner_radius: radius::PILL,
                    ..accent.pressed
                },
                press: accent.press,
            };
        }
        smart_material(role, state, context, &self.theme, self.tier)
    }

    /// A palette role as the compositor takes a colour.
    pub fn colour(&self, role: PaletteRole) -> Rgba {
        self.theme.colour(role)
    }

    pub fn canvas(&self) -> Rgba {
        self.colour(PaletteRole::Canvas)
    }

    /// The text of a surface: primary.
    pub fn ink(&self) -> Rgba {
        self.colour(PaletteRole::Ink)
    }

    /// Secondary text.
    pub fn ink2(&self) -> Rgba {
        self.colour(PaletteRole::Ink2)
    }

    /// Tertiary text: hints and disabled labels.
    pub fn ink3(&self) -> Rgba {
        self.colour(PaletteRole::Ink3)
    }

    pub fn accent(&self) -> Rgba {
        self.colour(PaletteRole::Accent)
    }

    /// Text and marks on an accent fill.
    pub fn accent_ink(&self) -> Rgba {
        self.colour(PaletteRole::AccentInk)
    }

    pub fn danger(&self) -> Rgba {
        self.colour(PaletteRole::Danger)
    }

    pub fn positive(&self) -> Rgba {
        self.colour(PaletteRole::Positive)
    }

    /// The focus ring: a 3:1 line against the sheet in either look.
    pub fn focus_cue(&self) -> Rgba {
        self.theme.materials.focus_cue
    }

    pub fn warning(&self) -> Rgba {
        self.colour(PaletteRole::Warning)
    }

    pub fn scrim(&self) -> Rgba {
        self.colour(PaletteRole::Scrim)
    }

    /// The ink of an icon that is disabled: .55 of its alpha.
    pub fn disabled_ink(&self) -> Rgba {
        self.theme.colour_scaled(
            PaletteRole::Ink,
            crate::lucent::tokens::state::DISABLED_ICON,
        )
    }

    /// A duration with reduced motion applied: a change is still shown, in no more than the reduced-motion ceiling.
    pub fn duration_ms(&self, ms: u32) -> u32 {
        if self.reduced_motion {
            ms.min(motion::REDUCED_MAX_MS)
        } else {
            ms
        }
    }

    /// Whether scale and travel are used at all (reduced motion crossfades instead).
    pub fn moves(&self) -> bool {
        !self.reduced_motion
    }
}

/// Every character the toolkit's screens draw, so a glyph is rastered once, when a type is warmed, and never while a finger is down.
/// In the order a warm that stops at its bound ([`WARM_ROWS`]) should keep them: the letters and digits of every line first, the
/// capitals and the marks of a sentence next, the arrows and the symbols nobody types last.
const WARM: &str = " abcdefghijklmnopqrstuvwxyz0123456789.,:-'ABCDEFGHIJKLMNOPQRSTUVWXYZ!?()/&%+;°·…—–’“”•←→↑↓↵‹›×\"#$*<=>@[\\]^_`{|}~";

/// The width of a page of the glyph atlas the labels draw from, and its height in rows. A client may hold 4 MiB of atlases in all
/// (`compositor_protocol::resources::MAX_ATLAS_BYTES`) and a look change holds two pages at once (the shell on the old one, the next
/// look's glyphs landing on the new), beside the page of a label of the host that outlives the look (a toast): three pages, so the
/// tallest page is the one three of which fit: 1344 rows of 1024 (1.31 MiB).
///
/// The page was 1024 rows (1 MiB, one page to spare), and the shell the app makes stood in most of it the moment it was made: 838
/// rows (82 %) at 100 % text, 918 and 904 (88 to 90 %) at 125 % and 150 %, and 987 (96 %) at 200 %, over the 90 % that crowds a page,
/// so that the app made its shell again at its first quiet moment. What the home says of the day (its date, the hours and days of its
/// line) moved the shell by a shelf: 905 or 942 rows at 200 % after a look change, depending on the date and the hour, and at 942 the page
/// was crowded again at once, and the app made its shell again every ten minutes. On 1344 rows the start stands at 62 to 73 % (838 to
/// 987 rows) and a look change at 60 to 70 % (812 to 942 rows; measured at every half hour of two days, on fourteen days in a row and
/// on fourteen days of the other months), and the page is crowded ([`CROWDED_PERCENT`]) at 1210 rows.
const TEXT_PAGE_WIDTH: u32 = 1024;
const TEXT_PAGE_ROWS: u32 = 1344;

/// How many rows of a page a warm may use. What it prepares is the toolkit's types in full, every character of each; the rest of the
/// page is for what labels ask for as they are made (the words of the day at sizes nothing warms, icons and rules, a glyph the toolkit
/// does not list) and for the screens the person opens later. It is 70 % of the 1024 rows the page had before it was made taller,
/// and stays what it was: the added rows are room, not more warm. Measured on the real shell: the types it warms take 506 rows at
/// 100 % text, 711 at 125 %, 872 at 150 %, 930 at 175 % and 568 at 200 %.
const WARM_ROWS: u32 = 1024 * 70 / 100;

/// A page is crowded at this percent of its rows, or once a label has found it full ([`Ui::text_atlas_full`]): a new shell's labels
/// would meet an atlas that has no room for the next size or screen, and the host makes a fresh page when it makes the shell again.
const CROWDED_PERCENT: u32 = 90;

/// Whether [`CROWDED_PERCENT`] of the rows of `page` are used.
fn crowded(page: &crate::atlas::AtlasView) -> bool {
    page.used_rows() * 100 >= page.size().1 * CROWDED_PERCENT
}

/// What components are built with.
pub struct Ui {
    pub scene: Scene,
    pub fonts: Fonts,
    pub look: Look,
    /// Labels and text layers made as a picture while glyph text was on (`enable_glyph_text`): their words could not be shown from
    /// the atlas when they were made (not handed over to the compositor yet, the page full, too many quads or colours). Counted at
    /// creation only; a run that becomes a picture later is [`Ui::glyph_text_fallbacks`].
    pub text_pictures: u32,
    /// The glyph atlas that new labels and text layers draw from when text is the compositor's quads (`enable_glyph_text`), not a
    /// picture each. A run keeps drawing from the atlas it was made on, whatever this is by then.
    pub(crate) text_atlas: Option<crate::atlas::Atlas>,
    /// The page new text is made on (`text_atlas`) said it had no room (`AtlasFull`) when something was prepared in it, since this
    /// was last cleared; whoever replaces that page at a settled boundary reads and clears it. Set by labels, text layers and
    /// [`Ui::warm`]; not by a run of an older page: that page being full says nothing about the one new text goes on now.
    pub text_atlas_full: bool,
    /// Runs that had to become pictures after they were runs, counted: a label or text layer whose new words could not be shown from
    /// its atlas (full, more than 256 quads, more than four colours, a hairline wider than the atlas) is replaced, in the same
    /// compositor layer, by a picture of them, and stays one. Its words are right, but it is a texture and pixels written, which
    /// is what glyph text saves.
    pub glyph_text_fallbacks: u32,
    /// The picture the compositor keeps from one owner to the next ([`Ui::keep_image`]: the shell's wallpaper).
    pub(crate) kept_image: Option<std::rc::Rc<crate::kept::Kept>>,
}

impl Ui {
    /// A UI on `scene` in the standard dark look.
    pub fn new(scene: Scene) -> Self {
        Self::with_look(scene, Look::standard(Mode::Dark))
    }

    pub fn with_look(scene: Scene, look: Look) -> Self {
        Self {
            scene,
            fonts: Fonts::new(),
            look,
            text_pictures: 0,
            text_atlas: None,
            text_atlas_full: false,
            glyph_text_fallbacks: 0,
            kept_image: None,
        }
    }

    /// Makes labels draw their text as glyph quads of one atlas (a run layer, no texture, no pixels written) instead of a picture
    /// each. A label whose glyphs cannot be shown at once (not handed over to the compositor yet, atlas full) is a picture, as it
    /// always was; one that is a run and then cannot show its new words becomes a picture of them, in place
    /// ([`Ui::glyph_text_fallbacks`]). The atlas is 1024 x 1344 coverage bytes.
    pub fn enable_glyph_text(&mut self) -> Result<(), crate::error::UiError> {
        if self.text_atlas.is_none() {
            self.text_atlas = Some(self.scene.atlas(TEXT_PAGE_WIDTH, TEXT_PAGE_ROWS)?);
        }
        Ok(())
    }

    /// A fresh, empty page becomes the one labels draw from: the look changes (a new text size, a shell made again) and the page of
    /// the old look has no room for the new one's glyphs. Returns whether it did; `false` when glyph text is off or when the
    /// compositor's limits on atlases leave no room for a second page beside those held (4 pages, 4 MiB): nothing changes then.
    ///
    /// The old page stays on the compositor as long as a run of it does and goes with the last (`Atlas` is a handle: dropped
    /// here). A run draws only from the page it was made on, so labels already made are unharmed; the labels made from now on draw
    /// from the new page, which starts with none of the old glyphs: [`Ui::warm`] it before the shell that needs it is made, and
    /// land it ([`Ui::glyph_text_settled`]) before that shell is shown.
    pub fn renew_text_atlas(&mut self) -> bool {
        if self.text_atlas.is_none() {
            return false;
        }
        match self.scene.atlas(TEXT_PAGE_WIDTH, TEXT_PAGE_ROWS) {
            Ok(page) => {
                self.text_atlas = Some(page);
                // The page that said it was full is not the one text is made on now.
                self.text_atlas_full = false;
                true
            }
            Err(_) => false,
        }
    }

    /// Whether the page labels draw from is nearly used up: [`CROWDED_PERCENT`] of its rows are, or something that was made on it
    /// found it full ([`Ui::text_atlas_full`]). False without glyph text.
    pub fn text_atlas_crowded(&self) -> bool {
        self.text_atlas
            .as_ref()
            .is_some_and(|page| self.text_atlas_full || crowded(page))
    }

    /// Whether the glyph atlas has nothing left to hand over to the compositor and nothing the compositor has not said it landed
    /// (true without glyph text, which has none): every glyph prepared is on the texture, so a label made now is a run.
    pub fn glyph_text_settled(&self) -> bool {
        self.text_atlas
            .as_ref()
            .is_none_or(|atlas| atlas.is_settled())
    }

    /// The page labels draw their glyphs from now, with glyph text on.
    pub fn text_atlas(&self) -> Option<&crate::atlas::Atlas> {
        self.text_atlas.as_ref()
    }

    /// Commits what is staged, never waiting: its tag, or `None` when the compositor has not caught up (the transaction stays
    /// staged and the tag unused; the main loop commits again once the socket is writable, `Scene::writer_ready`). An error is
    /// the connection's, or a transaction that can never be taken.
    pub fn try_commit(&self) -> std::io::Result<Option<u64>> {
        match self.scene.try_commit() {
            Ok(tag) => Ok(Some(tag.get())),
            Err(crate::SubmitError::WouldBlock) => Ok(None),
            Err(e) => Err(e.into_io()),
        }
    }

    /// Rasters the glyphs of `types` now: an uncached glyph costs 0.1 to 10 ms, which must never land on a frame. With glyph text
    /// they are also prepared in the atlas, in the order given and as far as [`WARM_ROWS`] rows reach: the types that come
    /// first are in the atlas in full, the rest are left to the labels. Types the page already holds cost nothing. A warm that finds
    /// a crowded page without room for what it was asked says so in [`Ui::text_atlas_full`].
    pub fn warm(&mut self, types: &[Type]) {
        for &ty in types {
            self.warm_one(ty, WARM);
        }
    }

    /// [`Ui::warm`] for the characters a type is drawn in, which are not always all of them: a type that only ever draws a
    /// reading (the digits of a countdown, a temperature) costs the page a shelf for its digits and a few marks, where every
    /// character of the toolkit costs it two or three. Same order, same bound, same account of a page that is full.
    pub fn warm_chars(&mut self, sets: &[(Type, &str)]) {
        for &(ty, chars) in sets {
            self.warm_one(ty, chars);
        }
    }

    fn warm_one(&mut self, ty: Type, chars: &str) {
        self.fonts.warm(chars, ty);
        if let Some(atlas) = &self.text_atlas {
            // Its glyphs travel to the compositor now, in the time a screen is being made, and not in the commit that shows it.
            let rows = WARM_ROWS;
            let prepared = atlas.prepare_within(&mut self.fonts, ty, chars, rows);
            // Stopping at the bound says nothing of the page; a page that is crowded and lacks glyphs the warm wanted is full.
            if matches!(prepared, Err(crate::error::UiError::AtlasFull))
                || (crowded(atlas) && chars.chars().any(|c| atlas.glyph(ty, c).is_none()))
            {
                self.text_atlas_full = true;
            }
            atlas.pump();
        }
    }

    /// Prepares the marks `forms` (icons, discs, rounded bars) in the atlas, as [`Ui::warm`] does glyphs and within the same bound
    /// ([`WARM_ROWS`] rows), so that the text layer that carries one finds it handed over to the compositor when it is
    /// made: a layer whose mark is still waiting for one of the atlas's two slots is a picture of its whole frame instead of a run.
    /// A mark that did not fit says so in [`Ui::text_atlas_full`] only if the page is crowded. Without glyph text there is nothing to
    /// prepare.
    pub fn warm_marks(&mut self, forms: &[Form]) {
        let Some(atlas) = &self.text_atlas else {
            return;
        };
        let rows = WARM_ROWS;
        for form in forms {
            let sprite = form.sprite();
            let _ = atlas.prepare_sprite_within(
                sprite,
                |_| {
                    let (w, h) = form.size();
                    (w, h, form.coverage())
                },
                rows,
            );
            if crowded(atlas) && atlas.sprite(sprite).is_none() {
                self.text_atlas_full = true;
            }
        }
        atlas.pump();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{Align, Label, style};
    use crate::testing::{count_requests, harness};
    use crate::text::Weight;
    use compositor_protocol::{Content, Request};

    const INK: Rgba = Rgba(236, 236, 240, 255);
    const BODY: Type = Type {
        px: 20,
        weight: Weight::Regular,
    };

    fn text(h: &mut crate::testing::Harness, y: f32, words: &str) -> Label {
        Label::line(
            &mut h.ui,
            0.0,
            y,
            300.0,
            1,
            style(BODY, INK, Align::Left),
            words,
        )
        .expect("a label")
    }

    /// The atlases the runs made in `sent` draw from.
    fn pages_of(sent: &[Request]) -> Vec<compositor_protocol::AtlasId> {
        sent.iter()
            .filter_map(|r| match r {
                Request::Create {
                    content: Content::Glyphs(style),
                    ..
                } => Some(style.atlas),
                _ => None,
            })
            .collect()
    }

    fn deleted(sent: &[Request]) -> Vec<compositor_protocol::AtlasId> {
        sent.iter()
            .filter_map(|r| match r {
                Request::AtlasDelete { id } => Some(*id),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_warm_of_characters_prepares_those_and_costs_a_shelf_where_the_whole_type_costs_several() {
        let digits = Type {
            px: 52,
            weight: Weight::SemiBold,
        };
        let mut few = harness();
        few.ui.enable_glyph_text().unwrap();
        few.ui.warm_chars(&[(digits, "0123456789:")]);
        few.settle_atlas();
        let page = few.ui.text_atlas().unwrap();
        assert!(
            "0123456789:"
                .chars()
                .all(|c| page.glyph(digits, c).is_some())
        );
        assert!(
            page.glyph(digits, 'a').is_none(),
            "only the characters asked for are prepared"
        );
        let mut all = harness();
        all.ui.enable_glyph_text().unwrap();
        all.ui.warm(&[digits]);
        all.settle_atlas();
        assert!(
            page.used_rows() * 2 < all.ui.text_atlas().unwrap().used_rows(),
            "{} rows for eleven digits, {} for the type in full",
            page.used_rows(),
            all.ui.text_atlas().unwrap().used_rows()
        );
    }

    #[test]
    fn marks_are_warmed_within_the_bound_a_warm_of_types_keeps() {
        use crate::components::Icon;
        let marks = [
            Form::Icon(Icon::Plus, 29),
            Form::Icon(Icon::Rain, 52),
            Form::Disc(10),
        ];
        let mut empty = harness();
        empty.ui.enable_glyph_text().unwrap();
        empty.ui.warm_marks(&marks);
        empty.settle_atlas();
        let page = empty.ui.text_atlas().unwrap();
        assert!(marks.iter().all(|m| page.sprite(m.sprite()).is_some()));

        // A page the types have filled to its bound takes no new shelf for a mark.
        let mut full = harness();
        full.ui.enable_glyph_text().unwrap();
        let types: Vec<Type> = [56, 52, 48, 44, 40, 36, 32]
            .map(|px| Type {
                px,
                weight: Weight::SemiBold,
            })
            .to_vec();
        full.ui.warm(&types);
        full.settle_atlas();
        let before = full.ui.text_atlas().unwrap().used_rows();
        full.ui
            .warm_marks(&[Form::Icon(Icon::Snow, 61), Form::Icon(Icon::Fog, 63)]);
        full.settle_atlas();
        let page = full.ui.text_atlas().unwrap();
        assert_eq!(
            page.used_rows(),
            before,
            "no shelf was opened past the bound"
        );
        assert!(page.used_rows() <= WARM_ROWS);
    }

    #[test]
    fn a_warm_that_cannot_hold_every_type_prepares_the_first_ones_in_full_and_leaves_the_rest_of_the_page_to_the_labels()
     {
        let mut h = harness();
        h.ui.enable_glyph_text().unwrap();
        // Seven big types: each is 130 to 245 rows of the page's 1024 in full, together far more than it has.
        let types: Vec<Type> = [
            (56, Weight::SemiBold),
            (56, Weight::Medium),
            (48, Weight::SemiBold),
            (48, Weight::Regular),
            (40, Weight::SemiBold),
            (40, Weight::Medium),
            (36, Weight::Medium),
        ]
        .map(|(px, weight)| Type { px, weight })
        .to_vec();
        h.ui.warm(&types);
        h.settle_atlas();

        let page = h.ui.text_atlas().unwrap();
        assert!(
            page.used_rows() <= WARM_ROWS,
            "a warm stays within its bound: {} rows",
            page.used_rows()
        );
        assert!(
            WARM.chars().all(|c| page.glyph(types[0], c).is_some()),
            "the first type is in the atlas in full"
        );
        let last = *types.last().unwrap();
        assert!(
            WARM.chars().any(|c| page.glyph(last, c).is_none()),
            "the last is left to the labels"
        );

        // What the labels ask for after it still finds room: a word of a type that was not warmed is a run, not a picture.
        let cut = Type {
            px: 30,
            weight: Weight::Regular,
        };
        let label = Label::line(
            &mut h.ui,
            0.0,
            0.0,
            300.0,
            1,
            style(cut, INK, Align::Left),
            "Sunrise",
        )
        .unwrap();
        assert!(label.is_glyph_run());
    }

    #[test]
    fn the_letters_and_digits_of_a_type_come_before_its_symbols_in_a_warm_that_stops_early() {
        let mut h = harness();
        h.ui.enable_glyph_text().unwrap();
        // A type that does not fit in a page that has 20 rows left: only what is in front of the string is prepared.
        let big = Type {
            px: 56,
            weight: Weight::SemiBold,
        };
        let page = h.ui.text_atlas().unwrap();
        page.prepare_within(&mut Fonts::new(), big, WARM, 70)
            .unwrap();
        assert!(page.used_rows() <= 70);
        assert!(
            ('a'..='z').all(|c| page.glyph(big, c).is_some()),
            "the lowercase letters are first"
        );
        assert!(
            page.glyph(big, '~').is_none() && page.glyph(big, '{').is_none(),
            "the symbols nobody types are last"
        );
    }

    #[test]
    fn a_fresh_page_takes_over_while_the_old_one_stays_for_its_runs_and_goes_with_the_last_of_them()
    {
        let mut h = harness();
        h.ui.enable_glyph_text().unwrap();
        let old_label = text(&mut h, 0.0, "Volume");
        assert!(old_label.is_glyph_run());
        let first = h.commit();
        let old_page = pages_of(&first)[0];

        assert!(h.ui.renew_text_atlas());
        // The labels made now draw from the new page, which has none of the glyphs the old one has.
        let new_label = text(&mut h, 30.0, "Volume");
        assert!(new_label.is_glyph_run());
        let second = h.commit();
        let new_page = pages_of(&second)[0];
        assert_ne!(old_page, new_page);
        assert_eq!(
            count_requests(&second, |r| matches!(
                r,
                Request::AtlasCreate { id, width: TEXT_PAGE_WIDTH, height: TEXT_PAGE_ROWS } if *id == new_page
            )),
            1
        );
        assert!(
            count_requests(
                &second,
                |r| matches!(r, Request::AtlasUpload { id, .. } if *id == new_page)
            ) > 0,
            "the new page was filled, not borrowed from"
        );
        assert!(
            deleted(&second).is_empty(),
            "the old page stays while a run draws from it"
        );

        drop(old_label);
        let third = h.commit();
        assert_eq!(
            deleted(&third),
            [old_page],
            "and goes with the last of its runs"
        );
        drop(new_label);
        assert_eq!(
            deleted(&h.commit()),
            [],
            "the page in use stays: it is the handle's"
        );
    }

    #[test]
    fn a_label_made_before_a_renewal_shows_its_new_words_rightly_from_its_own_page_after_it() {
        // The host's own labels (a toast, a counter) outlive a look and the page it was made on.
        let words = "Muted: Kitchen Display";
        let mut h = harness();
        h.ui.enable_glyph_text().unwrap();
        let mut toast = text(&mut h, 0.0, "Saved");
        h.settle_atlas();
        let old_page = h.ui.text_atlas().unwrap().wire_id();
        assert!(h.ui.renew_text_atlas());
        let new_page = h.ui.text_atlas().unwrap().wire_id();
        h.commit();

        // Letters the old page has never held: they are prepared on the old page, which the run draws from, and not on the new.
        let before = h.all().len();
        toast.set(&mut h.ui, words);
        h.settle_atlas();
        let sent = h.all()[before..].to_vec();
        let to = |page| {
            count_requests(
                &sent,
                |r| matches!(r, Request::AtlasUpload { id, .. } if *id == page),
            )
        };
        assert!(to(old_page) > 0, "the new letters went to the old page");
        assert_eq!(to(new_page), 0, "and none to the new one");
        assert!(toast.is_glyph_run());
        assert_eq!(h.ui.glyph_text_fallbacks, 0);

        // The words shown are the words: the pixels of a picture of them.
        let mut plain = harness();
        let picture = text(&mut plain, 0.0, words);
        plain.commit();
        assert_eq!(h.layer_pixels(toast.id()), plain.pixels(picture.id()));
    }

    #[test]
    fn a_renewal_that_would_pass_the_compositors_limit_on_atlases_changes_nothing() {
        let mut h = harness();
        h.ui.enable_glyph_text().unwrap();
        // Two pages pinned by a run each, and the one labels draw from: three, which is all a client may hold of pages this tall
        // (3 x 1.31 MiB; a fourth would pass the 4 MiB the compositor lets it have).
        let mut pinned = Vec::new();
        for n in 0..2 {
            pinned.push(text(&mut h, n as f32 * 30.0, "Volume"));
            assert!(h.ui.renew_text_atlas(), "page {n}");
        }
        let current = h.ui.text_atlas().unwrap().wire_id();
        assert!(
            !h.ui.renew_text_atlas(),
            "a fourth page does not fit beside three"
        );
        assert_eq!(h.ui.text_atlas().unwrap().wire_id(), current);
        assert!(
            text(&mut h, 100.0, "Volume").is_glyph_run(),
            "the page still serves"
        );
        // Without glyph text there is no page to renew.
        let mut plain = harness();
        assert!(!plain.ui.renew_text_atlas());
        assert!(plain.ui.text_atlas().is_none());
    }

    /// The page is the tallest of which the host's three at once (the shell, the next look's landing, a toast) fit in what the
    /// compositor lets a client hold: a page of the text that is taller would make the third of them refused.
    #[test]
    fn the_page_is_the_tallest_of_which_three_fit_in_the_compositors_limit_on_atlases() {
        let limit = compositor_protocol::resources::MAX_ATLAS_BYTES;
        let page = |rows: u32| u64::from(TEXT_PAGE_WIDTH) * u64::from(rows);
        assert!(3 * page(TEXT_PAGE_ROWS) <= limit, "three pages fit");
        assert!(
            3 * page(TEXT_PAGE_ROWS + 32) > limit,
            "a taller one would not"
        );
        // A page is not wider or taller than the protocol allows a side to be (checked as the test is built).
        const {
            assert!(TEXT_PAGE_WIDTH <= compositor_protocol::resources::MAX_ATLAS_SIDE);
            assert!(TEXT_PAGE_ROWS <= compositor_protocol::resources::MAX_ATLAS_SIDE);
        }
    }

    #[test]
    fn a_page_is_crowded_from_nine_tenths_of_its_rows_and_a_fresh_one_is_not() {
        let mut h = harness();
        assert!(!h.ui.text_atlas_crowded(), "no page, nothing is crowded");
        h.ui.enable_glyph_text().unwrap();
        assert!(!h.ui.text_atlas_crowded());
        let page = h.ui.text_atlas().unwrap();
        let mut fonts = Fonts::new();
        for (px, weight) in [
            (56, Weight::SemiBold),
            (56, Weight::Medium),
            (56, Weight::Regular),
            (48, Weight::SemiBold),
            (48, Weight::Medium),
            (48, Weight::Regular),
            (44, Weight::SemiBold),
            (44, Weight::Medium),
        ] {
            let _ = page.prepare(&mut fonts, Type { px, weight }, WARM);
            // Crowded exactly when the rows used are 90 % of the page's.
            assert_eq!(
                h.ui.text_atlas_crowded(),
                page.used_rows() * 100 >= TEXT_PAGE_ROWS * CROWDED_PERCENT
            );
        }
        assert!(h.ui.text_atlas_crowded());
        assert!(h.ui.renew_text_atlas());
        assert!(!h.ui.text_atlas_crowded());
    }
}

#[cfg(test)]
mod page_type_tests {
    use super::*;

    #[test]
    fn the_mocks_own_sizes_are_what_the_settings_draw_at_their_text_size_and_grow_with_the_persons()
    {
        let mut look = Look::standard(Mode::Dark);
        look.text_scale = PAGE_SCALE;
        // The approved mock-ups' pixels, exactly: a 30 is 30 (25 times 1.2 rounded up would be 31).
        for px in [18, 20, 21, 22, 23, 26, 28, 30, 32] {
            let ty = look.page_ty(px, Weight::SemiBold);
            assert_eq!((ty.px, ty.weight), (px, Weight::SemiBold));
        }
        // The person's own text size goes on top of it: at 150 % a mock's 21 is 31.5, drawn at 32.
        look.text_scale = 1.5 * PAGE_SCALE;
        assert_eq!(look.page_ty(21, Weight::Regular).px, 32);
        assert_eq!(look.page_ty(20, Weight::Regular).px, 30);
        // And the roles of the toolkit agree with the mock where they have its size.
        look.text_scale = PAGE_SCALE;
        assert_eq!(look.ty(crate::lucent::tokens::ty::BODY).px, 24);
        assert_eq!(look.ty(crate::lucent::tokens::ty::LABEL).px, 22);
        assert_eq!(look.ty(crate::lucent::tokens::ty::CAPTION).px, 20);
    }
}
