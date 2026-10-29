//! The one list of what the display lab may touch, and nothing else exists: six targets (two knobs under `/proc/lg/fbdev/chip`, four
//! registers of the OSD block at `0xCC48xxxx`), the values each may be given, and the three controls the shell's lab drives them by.
//!
//! The list is the 2026-10-29 session's (`docs/research/layer-deep-result.md` §8.2, registers read and written on the set by HwTest, with
//! the dead-man of `tools/layerdeep-tv.sh`). There is no way to name an address or a value from outside: a request names a control and
//! one of its options, [`plan`] turns that into writes, and the writes pass [`allowed`] again at the bottom (`hw.rs`), so a wrong table
//! here still cannot write a value that is not listed. A write is one of the **fixed** values below, or the **original** the lab read from
//! the target itself before it wrote anything (never one a client supplied), and an original has to be plausible first: a register that
//! already holds a lab value (a crashed run's) or something that no register of this block holds is not taken for the set's own.

/// What the lab can address. A closed set: there is no constructor from a number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Target {
    /// `/proc/lg/fbdev/chip/osd_sr_gain`: the OSD sharpener's gain (`sr_ctrl[0].ti_gain`, 0..255; the set's own is `0x10`).
    Gain,
    /// `/proc/lg/fbdev/chip/osd_sr`: the OSD sharpener on (1) or off (0) (`osd_sr.sw_status`; the set's own is on).
    Sharpener,
    /// `0xCC480360`, OSD0's video signal processor: `vspyc_en` bit 0, `graycolor_en` bit 1, `r_center` bits 8:15, `r_contrast` bits 16:25.
    Vsp,
    /// `0xCC4802C8`, B.OSD1 hole 0: width bits 0:12, x bits 13:25, enable bit 28.
    HoleX,
    /// `0xCC4802CC`, B.OSD1 hole 0: height bits 0:11, y bits 12:23, alpha bits 24:31.
    HoleY,
    /// `0xCC48029C`, B.OSD1: `osd0_en` bit 0, `osd0_hole_sel` bits 1:3.
    HoleSel,
}

impl Target {
    pub const ALL: [Target; 6] = [
        Target::Gain,
        Target::Sharpener,
        Target::Vsp,
        Target::HoleX,
        Target::HoleY,
        Target::HoleSel,
    ];

    /// The name the journal and the log use.
    pub fn name(self) -> &'static str {
        match self {
            Target::Gain => "osd_sr_gain",
            Target::Sharpener => "osd_sr",
            Target::Vsp => "osd0_vsp",
            Target::HoleX => "hole0_wx",
            Target::HoleY => "hole0_hy",
            Target::HoleSel => "hole_sel",
        }
    }

    pub fn from_name(name: &str) -> Option<Target> {
        Target::ALL.into_iter().find(|t| t.name() == name)
    }

    /// The physical address of a register target; `None` for a `/proc` knob.
    pub fn address(self) -> Option<u32> {
        match self {
            Target::Vsp => Some(0xCC48_0360),
            Target::HoleX => Some(0xCC48_02C8),
            Target::HoleY => Some(0xCC48_02CC),
            Target::HoleSel => Some(0xCC48_029C),
            Target::Gain | Target::Sharpener => None,
        }
    }

    /// The file of a `/proc` knob, under `/proc/lg/fbdev/chip/`; `None` for a register.
    pub fn knob(self) -> Option<&'static str> {
        match self {
            Target::Gain => Some("osd_sr_gain"),
            Target::Sharpener => Some("osd_sr"),
            _ => None,
        }
    }
}

/// The page every register target lies in: one 4 KiB page of the OSD block, so the one mapping the real access makes covers them all.
pub const REGISTER_PAGE: u32 = 0xCC48_0000;

/// The fields of `0xCC480360` that exist: anything else set in a read means that is not the register the lab thinks it is.
const VSP_FIELDS: u32 = 0x0000_0003 | 0x0000_FF00 | 0x03FF_0000;

/// The values a target may be given besides its original.
pub fn fixed_values(t: Target) -> &'static [u32] {
    match t {
        // 0x40 was tried on the set too; the lab's options use the other two
        Target::Gain => &[0x00, 0x40, 0x80],
        Target::Sharpener => &[0, 1],
        // half contrast (`r_contrast` 0x100 of the neutral 0x200), and graycolor on with the contrast neutral
        Target::Vsp => &[0x0100_0001, 0x0200_0003],
        // x 720 in the UI's 1920 units, width 960 panel pixels, enabled: a window in the middle of the panel (measured on the set, round 19W,
        // `layer-deep-result.md` § 8.4; the session's first guess, x 1440, opened it at the right edge)
        Target::HoleX => &[0x105A_03C0],
        // y 810, height 540, alpha 0
        Target::HoleY => &[0x0032_A21C],
        // OSD0's input takes hole 0 with `osd0_en` and the select field at 1; with the field at 0 (`0x1`) nothing opens (measured, § 8.4)
        Target::HoleSel => &[0x3],
    }
}

/// Whether `v`, read from `t` before the lab wrote anything, may be written back as its original. A value that is one of the lab's own
/// is a crashed run's leftover and is refused (the real original is lost); so is anything outside the register's fields, and any hole
/// register that is not clear (nothing else of this set programs them: another writer is there).
pub fn plausible_original(t: Target, v: u32) -> bool {
    match t {
        Target::Gain => v <= 0xFF && !fixed_values(t).contains(&v),
        Target::Sharpener => v <= 1,
        Target::Vsp => v & !VSP_FIELDS == 0 && !fixed_values(t).contains(&v),
        Target::HoleX | Target::HoleY | Target::HoleSel => v == 0,
    }
}

/// Whether a write of `v` to `t` could be on the list at all: one of its fixed values, or a value that could be its original. The check at
/// the bottom, in front of the bytes (`hw.rs`), which does not know the original; the lab's own check is [`permitted`].
pub fn allowed(t: Target, v: u32) -> bool {
    fixed_values(t).contains(&v) || plausible_original(t, v)
}

/// Whether the lab may write `v` to `t` now: one of its fixed values, or exactly the original it read from `t` (which has to be plausible).
pub fn permitted(t: Target, v: u32, original: Option<u32>) -> bool {
    fixed_values(t).contains(&v) || (original == Some(v) && plausible_original(t, v))
}

/// The three things the lab changes, each with a few options; the first option of each is the set's own state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Control {
    Sharpness,
    Dimming,
    Window,
}

impl Control {
    pub const ALL: [Control; 3] = [Control::Sharpness, Control::Dimming, Control::Window];

    pub fn id(self) -> &'static str {
        match self {
            Control::Sharpness => "sharpness",
            Control::Dimming => "dimming",
            Control::Window => "window",
        }
    }

    pub fn from_id(id: &str) -> Option<Control> {
        Control::ALL.into_iter().find(|c| c.id() == id)
    }

    /// The options of the control, in the order the lab offers them.
    pub fn options(self) -> &'static [Opt] {
        match self {
            Control::Sharpness => SHARPNESS,
            Control::Dimming => DIMMING,
            Control::Window => WINDOW,
        }
    }

    /// The targets the control writes, in the order they are put back.
    pub fn targets(self) -> &'static [Target] {
        match self {
            Control::Sharpness => &[Target::Sharpener, Target::Gain],
            Control::Dimming => &[Target::Vsp],
            Control::Window => &[Target::HoleSel, Target::HoleX, Target::HoleY],
        }
    }

    pub fn option(self, id: &str) -> Option<&'static Opt> {
        self.options().iter().find(|o| o.id == id)
    }
}

/// What an option writes to a target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Val {
    /// What the lab read from the target before it wrote anything.
    Original,
    Fixed(u32),
}

#[derive(Clone, Copy, Debug)]
pub struct Opt {
    pub id: &'static str,
    pub writes: &'static [(Target, Val)],
}

const SHARPNESS: &[Opt] = &[
    Opt {
        id: "normal",
        writes: &[
            (Target::Gain, Val::Original),
            (Target::Sharpener, Val::Original),
        ],
    },
    // between the set's own 0x10 and strong: Strong was crisp but jagged up close (the TV check of 2026-10-29)
    Opt {
        id: "medium",
        writes: &[
            (Target::Gain, Val::Fixed(0x40)),
            (Target::Sharpener, Val::Fixed(1)),
        ],
    },
    Opt {
        id: "strong",
        writes: &[
            (Target::Gain, Val::Fixed(0x80)),
            (Target::Sharpener, Val::Fixed(1)),
        ],
    },
    Opt {
        id: "soft",
        writes: &[
            (Target::Gain, Val::Fixed(0x00)),
            (Target::Sharpener, Val::Fixed(1)),
        ],
    },
    Opt {
        id: "off",
        writes: &[
            (Target::Gain, Val::Original),
            (Target::Sharpener, Val::Fixed(0)),
        ],
    },
];

const DIMMING: &[Opt] = &[
    Opt {
        id: "normal",
        writes: &[(Target::Vsp, Val::Original)],
    },
    Opt {
        id: "dimmed",
        writes: &[(Target::Vsp, Val::Fixed(0x0100_0001))],
    },
    Opt {
        id: "grey",
        writes: &[(Target::Vsp, Val::Fixed(0x0200_0003))],
    },
];

// The rectangle first and the select last when a window opens, the select first when it closes: nothing selects a rectangle that is not there.
const WINDOW: &[Opt] = &[
    Opt {
        id: "off",
        writes: &[
            (Target::HoleSel, Val::Original),
            (Target::HoleX, Val::Original),
            (Target::HoleY, Val::Original),
        ],
    },
    Opt {
        id: "on",
        writes: &[
            (Target::HoleX, Val::Fixed(0x105A_03C0)),
            (Target::HoleY, Val::Fixed(0x0032_A21C)),
            (Target::HoleSel, Val::Fixed(0x3)),
        ],
    },
];

/// The writes an option makes, in order, as `(target, value)`: an original is looked up in `original`. `None` when the control has no
/// such option, an original is missing (the lab has not read it) or any resulting value is not on the list.
pub fn plan(
    control: Control,
    option: &str,
    original: impl Fn(Target) -> Option<u32>,
) -> Option<Vec<(Target, u32)>> {
    let opt = control.option(option)?;
    let mut out = Vec::with_capacity(opt.writes.len());
    for (target, val) in opt.writes {
        let v = match val {
            Val::Fixed(v) => *v,
            Val::Original => {
                let v = original(*target)?;
                if !plausible_original(*target, v) {
                    return None;
                }
                v
            }
        };
        if !allowed(*target, v) {
            return None;
        }
        out.push((*target, v));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 2026-10-29 session's numbers, as `layer-deep-result.md` §8.2 and `tools/layerdeep-tv.sh` give them.
    #[test]
    fn the_list_is_what_the_session_recorded() {
        assert_eq!(Target::Vsp.address(), Some(0xCC48_0360));
        assert_eq!(Target::HoleX.address(), Some(0xCC48_02C8));
        assert_eq!(Target::HoleY.address(), Some(0xCC48_02CC));
        assert_eq!(Target::HoleSel.address(), Some(0xCC48_029C));
        assert_eq!(
            (Target::Gain.knob(), Target::Sharpener.knob()),
            (Some("osd_sr_gain"), Some("osd_sr"))
        );
        assert_eq!(fixed_values(Target::Gain), &[0x00, 0x40, 0x80]);
        assert_eq!(fixed_values(Target::Sharpener), &[0, 1]);
        assert_eq!(
            fixed_values(Target::Vsp),
            &[0x0100_0001, 0x0200_0003],
            "half contrast, then grey"
        );
        assert_eq!(
            (fixed_values(Target::HoleX), fixed_values(Target::HoleY)),
            (&[0x105A_03C0][..], &[0x0032_A21C][..])
        );
        assert_eq!(
            fixed_values(Target::HoleSel),
            &[0x3],
            "hole 0: the select field at 0 opened nothing on the set"
        );
        // every register lies in the one page the real access maps, and no target has two ways to be reached
        for t in Target::ALL {
            if let Some(a) = t.address() {
                assert_eq!(a & !0xFFF, REGISTER_PAGE, "{t:?}");
                assert_eq!(a % 4, 0, "{t:?}: 32-bit registers");
                assert!(t.knob().is_none());
            } else {
                assert!(t.knob().is_some());
            }
        }
        // the hole's fields decode to the centred window: width 960 and height 540 in panel pixels, x 720 of the UI's 1920 (the panel's 1440) and y 810
        // of the panel's 2160, alpha 0
        let wx = fixed_values(Target::HoleX)[0];
        let hy = fixed_values(Target::HoleY)[0];
        assert_eq!(
            (wx & 0x1FFF, (wx >> 13) & 0x1FFF, (wx >> 28) & 1),
            (960, 720, 1)
        );
        assert_eq!((hy & 0xFFF, (hy >> 12) & 0xFFF, hy >> 24), (540, 810, 0));
        // the VSP values: half the neutral contrast 0x200 with the unit on, then neutral with the grey bit
        for (v, contrast, gray) in [(0x0100_0001u32, 0x100, 0), (0x0200_0003, 0x200, 1)] {
            assert_eq!(
                ((v >> 16) & 0x3FF, (v >> 1) & 1, v & 1),
                (contrast, gray, 1)
            );
        }
    }

    #[test]
    fn every_option_writes_only_what_the_list_allows_and_its_control_owns() {
        for control in Control::ALL {
            assert_eq!(
                control.options()[0].id,
                if control == Control::Window {
                    "off"
                } else {
                    "normal"
                },
                "the first option is the set's own"
            );
            for opt in control.options() {
                assert!(!opt.writes.is_empty(), "{} {}", control.id(), opt.id);
                for (target, val) in opt.writes {
                    assert!(
                        control.targets().contains(target),
                        "{} {} writes {target:?}, which the control does not own",
                        control.id(),
                        opt.id
                    );
                    if let Val::Fixed(v) = val {
                        assert!(
                            fixed_values(*target).contains(v),
                            "{} {}: {v:#x} is not a fixed value of {target:?}",
                            control.id(),
                            opt.id
                        );
                    }
                }
                // an option writes each target once
                let mut seen: Vec<Target> = opt.writes.iter().map(|w| w.0).collect();
                seen.sort();
                seen.dedup();
                assert_eq!(seen.len(), opt.writes.len(), "{} {}", control.id(), opt.id);
            }
            let mut ids: Vec<&str> = control.options().iter().map(|o| o.id).collect();
            ids.sort();
            ids.dedup();
            assert_eq!(ids.len(), control.options().len());
        }
        // the controls' targets are distinct: no register is moved by two controls
        let mut all: Vec<Target> = Control::ALL
            .iter()
            .flat_map(|c| c.targets().iter().copied())
            .collect();
        all.sort();
        all.dedup();
        assert_eq!(all, Target::ALL.to_vec());
        assert_eq!(
            all.len(),
            Control::ALL
                .iter()
                .map(|c| c.targets().len())
                .sum::<usize>()
        );
    }

    #[test]
    fn the_options_of_the_shell_are_the_ones_listed() {
        let ids = |c: Control| c.options().iter().map(|o| o.id).collect::<Vec<_>>();
        assert_eq!(
            ids(Control::Sharpness),
            ["normal", "medium", "strong", "soft", "off"]
        );
        assert_eq!(ids(Control::Dimming), ["normal", "dimmed", "grey"]);
        assert_eq!(ids(Control::Window), ["off", "on"]);
        assert_eq!(Control::from_id("sharpness"), Some(Control::Sharpness));
        for bad in [
            "",
            "Sharpness",
            "sharpness ",
            "../sharpness",
            "osd_sr",
            "vsp",
            "hole",
            "all",
        ] {
            assert_eq!(Control::from_id(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_plan_is_the_options_writes_with_the_originals_filled_in_and_refuses_the_rest() {
        let originals = |t: Target| {
            Some(match t {
                Target::Gain => 0x10,
                Target::Sharpener => 1,
                _ => 0,
            })
        };
        assert_eq!(
            plan(Control::Sharpness, "medium", originals),
            Some(vec![(Target::Gain, 0x40), (Target::Sharpener, 1)]),
            "between the set's 0x10 and strong's 0x80, on the list's own 0x40"
        );
        assert_eq!(
            plan(Control::Sharpness, "strong", originals),
            Some(vec![(Target::Gain, 0x80), (Target::Sharpener, 1)])
        );
        assert_eq!(
            plan(Control::Sharpness, "off", originals),
            Some(vec![(Target::Gain, 0x10), (Target::Sharpener, 0)])
        );
        assert_eq!(
            plan(Control::Sharpness, "normal", originals),
            Some(vec![(Target::Gain, 0x10), (Target::Sharpener, 1)])
        );
        assert_eq!(
            plan(Control::Dimming, "grey", originals),
            Some(vec![(Target::Vsp, 0x0200_0003)])
        );
        assert_eq!(
            plan(Control::Window, "on", originals),
            Some(vec![
                (Target::HoleX, 0x105A_03C0),
                (Target::HoleY, 0x0032_A21C),
                (Target::HoleSel, 0x3)
            ])
        );
        assert_eq!(
            plan(Control::Window, "off", originals),
            Some(vec![
                (Target::HoleSel, 0),
                (Target::HoleX, 0),
                (Target::HoleY, 0)
            ])
        );
        // an option that does not exist, and one of another control
        assert_eq!(plan(Control::Sharpness, "dimmed", originals), None);
        assert_eq!(plan(Control::Dimming, "", originals), None);
        // an original that has not been read
        assert_eq!(plan(Control::Sharpness, "normal", |_| None), None);
        // an original that could not be one (a leftover of a crashed run, a hole register in use): no plan to put it back
        assert_eq!(
            plan(Control::Dimming, "normal", |_| Some(0x0100_0001)),
            None
        );
        assert_eq!(plan(Control::Window, "off", |_| Some(5)), None);
        // a fixed value never needs an original
        assert!(plan(Control::Dimming, "dimmed", |_| None).is_some());
    }

    #[test]
    fn an_original_has_to_be_one_the_set_could_hold() {
        // what the set read on 2026-10-29: the gain 0x10, the sharpener on, the VSP 0x02000001 on one boot and 0 on the next, the holes clear
        assert!(plausible_original(Target::Gain, 0x10));
        assert!(
            plausible_original(Target::Sharpener, 1) && plausible_original(Target::Sharpener, 0)
        );
        assert!(plausible_original(Target::Vsp, 0x0200_0001) && plausible_original(Target::Vsp, 0));
        for t in [Target::HoleX, Target::HoleY, Target::HoleSel] {
            assert!(plausible_original(t, 0), "{t:?}");
            assert!(
                !plausible_original(t, 1) && !plausible_original(t, 0xFFFF_FFFF),
                "{t:?}"
            );
        }
        // a lab value is a leftover, not an original
        assert!(
            !plausible_original(Target::Gain, 0x80)
                && !plausible_original(Target::Gain, 0x00)
                && !plausible_original(Target::Gain, 0x40)
        );
        assert!(
            !plausible_original(Target::Vsp, 0x0100_0001)
                && !plausible_original(Target::Vsp, 0x0200_0003)
        );
        // a read that is all ones or outside the fields is a bus that did not answer, not a register
        assert!(
            !plausible_original(Target::Vsp, 0xFFFF_FFFF)
                && !plausible_original(Target::Vsp, 0xDEAD_BEEF)
                && !plausible_original(Target::Vsp, 0x8000_0000)
        );
        assert!(
            !plausible_original(Target::Gain, 0x100) && !plausible_original(Target::Sharpener, 2)
        );
    }

    #[test]
    fn nothing_but_the_list_passes_the_check_in_front_of_the_bytes() {
        for t in Target::ALL {
            for v in fixed_values(t) {
                assert!(allowed(t, *v), "{t:?} {v:#x}");
            }
        }
        // a value that is on another target's list is not on this one's
        assert!(!allowed(Target::Vsp, 0x80));
        assert!(!allowed(Target::Gain, 0x0200_0003));
        assert!(!allowed(Target::HoleSel, 0x105A_03C0));
        assert!(!allowed(Target::HoleX, 0x0032_A21C));
        // nor a whole range: every hole value but the listed ones and zero is refused, and so is any VSP with bits outside its fields
        for v in [
            1u32,
            2,
            0x7,
            0xFF,
            0x1000_0000,
            0x105A_03C1,
            0x105A_0000,
            0x10B4_03C0,
            0xFFFF_FFFF,
        ] {
            assert!(
                !allowed(Target::HoleX, v),
                "{v:#x} (0x10B403C0 opened the window at the right edge and is no longer on the list)"
            );
        }
        for v in [0x0400_0001u32, 0x0200_0007, 0x8000_0001, 0xFFFF_FFFF] {
            assert!(!allowed(Target::Vsp, v), "{v:#x}");
        }
        assert!(!allowed(Target::Gain, 0x1FF) && !allowed(Target::Sharpener, 3));
    }

    #[test]
    fn the_lab_writes_a_fixed_value_or_exactly_the_original_it_read_and_nothing_between() {
        // the original of the VSP on one boot: putting it back is allowed, writing a neighbour of it is not
        assert!(permitted(Target::Vsp, 0x0200_0001, Some(0x0200_0001)));
        assert!(
            !permitted(Target::Vsp, 0x0300_0001, Some(0x0200_0001)),
            "in the register's fields, but not what was read"
        );
        assert!(
            !permitted(Target::Vsp, 0x0200_0001, None),
            "no original was read"
        );
        assert!(
            permitted(Target::Vsp, 0x0100_0001, None),
            "a fixed value needs none"
        );
        // an original that is not plausible is never written back
        assert!(!permitted(Target::Vsp, 0x0400_0001, Some(0x0400_0001)));
        assert!(!permitted(Target::HoleSel, 5, Some(5)));
        assert!(permitted(Target::HoleSel, 0, Some(0)));
        assert!(
            permitted(Target::Gain, 0x10, Some(0x10)) && !permitted(Target::Gain, 0x11, Some(0x10))
        );
    }

    #[test]
    fn a_target_is_known_by_its_name_and_no_other_word() {
        for t in Target::ALL {
            assert_eq!(Target::from_name(t.name()), Some(t));
        }
        for bad in ["", "OSD_SR", "0xCC480360", "/dev/mem", "vsp"] {
            assert_eq!(Target::from_name(bad), None, "{bad:?}");
        }
    }
}
