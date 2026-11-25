//! Linking (sign-in, QR and PIN) and the question asked before signing out or removing a PC. The Accounts and PCs place itself is a
//! Settings list.
//!
//! Node ids of a sign-in step that waits: 8000 its backdrop, 8001 its sheet, 8880 its words, 8881 the QR, 8882 and 8883 the
//! countdown's ring and what is left of it. Entering a PIN keeps 8000 to 8500 of the text-entry layer; a failure or an empty search
//! is a [`modal`].
use crate::game::model::{Focus, Key, accounts::{KEY_LEN, KeyForm, KeyStep, LinkFlow}};
use crate::game::screens::{
    paint::{Body, Fill, H, Mark, Node, Paint, W, merge_words},
    parts::{SHEET_Z, Side, dialog::*, hints, keyboard::*, motion::Enter},
    symbols::Symbol,
};
use compositor_protocol::Props;
use lumagame_core::ipc::AuthState;
use lumatv_tokens::{Radius, TypeRole as Type};
use ui::{
    components::{Align, Form, Icon},
    geometry::Frame,
    lucent::palette::PaletteRole as Role,
};

/// The QR's white square, its quiet zone included, and the countdown's ring.
const QR: f32 = 360.0;
const RING: f32 = 44.0;
/// The waiting sheet's padding, and the room between its title, its body and its foot.
const PAD: f32 = 48.0;
const GAP: f32 = 40.0;
/// Steps of the countdown's ring: over a 300 s sign-in it is drawn again every 5 s.
const STEPS: u8 = 60;

/// Asks before the open integration signs out or the open PC is removed; Cancel is where focus starts.
pub fn sign_out(p: &mut Paint<'_>) {
    let m = p.model;
    let Some(e) = m.view.selected_engine.as_ref().and_then(|id| m.integration(id)) else {
        return;
    };
    let pc = m.view.selected_target.as_ref().and_then(|t| e.targets.iter().find(|x| x.id == *t));
    let (title, body, act) = match pc {
        Some(pc) => (format!("Remove {}?", pc.name), "Its games leave the library until the PC is paired again.".to_owned(), "Remove"),
        None => (format!("Sign out of {}?", e.manifest.name), "The sign-in is removed from this TV, with what it added to games.".to_owned(), "Sign out"),
    };
    let actions = [
        Action { label: "Cancel", icon: Some(Icon::Back), focus: Focus::SheetAction(0) },
        Action { label: act, icon: Some(Icon::Power), focus: Focus::SheetAction(1) },
    ];
    modal(p, &title, &body, &actions, 0);
    danger(p, 1);
    hints::bar(p, &m.hints(), Side::Right, m.hints_dim);
}

/// The Web API key form: what the key is, then the key on a hex keyboard, then the account on the letter keyboard. The key shows in
/// groups of four, as Steam's page does not, so a typing slip is easy to see; it is not shown again once the account is asked for.
fn key_form(p: &mut Paint<'_>, name: &str, form: &KeyForm) {
    match form.step {
        KeyStep::Intro => {
            let body = "On a phone or PC, sign in at steamcommunity.com and open steamcommunity.com/dev/apikey. Enter any domain name, agree to the terms and copy the 32-character key. The Steam ID is the long number in the profile address.";
            modal(p, &format!("Link {name} with a Web API key"), body, &[Action { label: "Continue", icon: Some(Icon::Enter), focus: Focus::SheetAction(0) }], 0);
            let m = p.model;
            hints::bar(p, &m.hints(), Side::Right, m.hints_dim);
        }
        KeyStep::Key => {
            let grouped: String = form.key.chars().enumerate().flat_map(|(i, c)| (i > 0 && i % 4 == 0).then_some(' ').into_iter().chain([c])).collect();
            let count = format!("{} of {KEY_LEN}", form.key.len());
            let entry = TextEntry {
                title: "Web API key",
                label: "The 32 characters from steamcommunity.com/dev/apikey",
                value: &grouped,
                placeholder: "0 to 9 and A to F",
                secret: false,
                numeric: false,
                hex: true,
                lower: false,
                action: Some(("Next", Icon::Enter)),
                notice: Some(&count),
            };
            p.art = false;
            plan(p, &entry);
        }
        KeyStep::Account => {
            let entry = TextEntry {
                title: "Steam ID",
                label: "The ID number or the address of the profile page",
                value: &form.account,
                placeholder: "Steam ID or profile address",
                secret: false,
                numeric: false,
                hex: false,
                lower: p.model.view.keyboard_lower,
                action: Some(("Link", Icon::Tick)),
                notice: None,
            };
            p.art = false;
            plan(p, &entry);
        }
        KeyStep::Checking => {}
    }
}

/// The sign-in layer over whatever page began it. A step that waits on a phone or the PC has nothing to focus, and Back cancels it;
/// a failure or an empty search for PCs asks what next.
pub fn auth(p: &mut Paint<'_>) {
    let m = p.model;
    let Some(flow) = &m.view.auth else {
        return;
    };
    let name = m.engine_name(&flow.engine);
    if let (Some(form), None) = (&flow.key_form, &flow.state)
        && form.step != KeyStep::Checking
    {
        key_form(p, name, form);
        return;
    }
    match &flow.state {
        Some(AuthState::Failed { message, .. }) => {
            modal(p, "Connection not completed", message, &[Action { label: "Try again", icon: Some(Icon::Refresh), focus: Focus::SheetAction(0) }], 0);
        }
        None if flow.no_pcs => {
            let manual = m
                .integration(&flow.engine)
                .and_then(|e| e.descriptor.as_ref())
                .is_some_and(|d| d.capabilities.has(lumagame_core::engine::Feature::ManualHost));
            let body = format!("{name} found no PC on this network. Check that the PC is on and on the same network, or enter its address.");
            let mut actions = vec![Action { label: "Try again", icon: Some(Icon::Refresh), focus: Focus::SheetAction(0) }];
            if manual {
                actions.push(Action { label: "Enter address", icon: Some(Icon::Plus), focus: Focus::SheetAction(1) });
            }
            modal(p, "No PCs found", &body, &actions, actions.len() - 1);
        }
        Some(AuthState::EnterPin { digits }) => {
            let entered = format!("{}{}", flow.pin, "_".repeat(usize::from(*digits).saturating_sub(flow.pin.len())));
            enter_pin(p, name, &entered);
        }
        state => {
            waiting(p, name, flow, state.as_ref());
            return;
        }
    }
    hints::bar(p, &m.hints(), Side::Right, m.hints_dim);
}

/// A step that waits on someone else: an opaque sheet with the title, what to do and, for a phone, the QR on its white square beside
/// it, with a ring counting down what is left of the sign-in. Its foot says Back cancels. The words are one layer.
fn waiting(p: &mut Paint<'_>, name: &str, flow: &LinkFlow, state: Option<&AuthState>) {
    let width = if p.scale > 1.0 { 1320.0 } else { 1080.0 };
    let left = (W - width) / 2.0;
    let screen = Frame::new(0.0, 0.0, W, H);
    p.arrives(screen, Enter::FADE);
    veil(p, 8000, screen, true);
    let from = p.plan.nodes.len();
    p.arrives(Frame::new(left, 0.0, width, H), Enter::FADE);
    // Laid from y 0 and moved to the middle once its height is known.
    let z = SHEET_Z + 1;
    let title = match state {
        None if flow.key_form.is_some() => "Checking the key".to_owned(),
        Some(AuthState::ShowPin { .. }) => format!("Pair with {name}"),
        Some(AuthState::Approval { .. }) => "Approve this TV".to_owned(),
        Some(_) => format!("Sign in to {name}"),
        None => format!("Connecting to {name}"),
    };
    let top = PAD + p.wrapped(8010, &title, Frame::new(left + PAD, PAD, width - 2.0 * PAD, 0.0), Type::Title, Role::Ink, false, z) + GAP;
    let qr = match state {
        Some(AuthState::Qr { url, .. }) => Some(url.clone()),
        Some(AuthState::DeviceCode { url, qr, .. }) => Some(qr.clone().unwrap_or_else(|| url.clone())),
        _ => None,
    };
    let x = if qr.is_some() { left + PAD + QR + PAD } else { left + PAD };
    let w = left + width - PAD - x;
    let at = |y: f32| Frame::new(x, y, w, 0.0);
    let column = p.plan.nodes.len();
    let mut y = top;
    match state {
        Some(AuthState::Qr { .. }) => {
            y += p.wrapped(8011, &format!("Scan with the {name} app on a phone."), at(y), Type::Body, Role::Ink, false, z) + 20.0;
        }
        Some(AuthState::DeviceCode { url, code, .. }) => {
            y += p.wrapped(8011, "Scan the QR code with a phone, or open", at(y), Type::Body, Role::Ink2, false, z) + 12.0;
            y += p.address(8012, url, at(y), if p.scale > 1.0 { Type::Label } else { Type::Title }, Role::Ink, false, z) + 24.0;
            y += p.wrapped(8013, "Then enter this code", at(y), Type::Body, Role::Ink2, false, z) + 8.0;
            y += p.wrapped(8014, code, at(y), Type::Title, Role::Ink, false, z) + 20.0;
        }
        Some(AuthState::ShowPin { pin }) => {
            y += p.wrapped(8011, "Enter this PIN in the pairing request on the PC.", at(y), Type::Body, Role::Ink2, false, z) + 36.0;
            y += digits(p, 8012, x, y, w, pin, z) + 36.0;
            y += p.wrapped(8013, "Waiting for approval", at(y), Type::Body, Role::Ink2, false, z);
        }
        Some(AuthState::Approval { hint }) => {
            y += p.wrapped(8011, hint.as_deref().unwrap_or("Approve the connection on the PC to finish pairing."), at(y), Type::Body, Role::Ink2, false, z);
        }
        None if flow.key_form.is_some() => y += p.wrapped(8011, &format!("Asking {name} to confirm the key and the account."), at(y), Type::Body, Role::Ink2, false, z),
        _ => y += p.wrapped(8011, "Waiting for the integration", at(y), Type::Body, Role::Ink2, false, z),
    }
    if let (Some(end), Some(remaining)) = (flow.expires_ms, p.model.auth_remaining()) {
        let ring = Frame::new(x, y, RING, RING);
        let full = (end.saturating_sub(flow.since_ms) / 1000).max(1);
        let shown = (remaining * u64::from(STEPS)).div_ceil(full).min(u64::from(STEPS)) as u8;
        p.symbol(8882, Symbol::Ring, ring, p.ink(Role::TrackOff, false), z);
        p.symbol(8883, Symbol::Countdown { left: shown, of: STEPS }, ring, p.ink(Role::Accent, false), z + 1);
        let lead = if matches!(state, Some(AuthState::Qr { .. })) { "Closes in" } else { "Code expires in" };
        let words = format!("{lead} {}:{:02}", remaining / 60, remaining % 60);
        p.text(8015, &words, Frame::new(x + RING + 16.0, y + (RING - p.line(Type::Body)) / 2.0, w - RING - 16.0, p.line(Type::Body)), Type::Body, Role::Ink2, false, z);
        y += RING;
    }
    let mut bottom = y;
    if let Some(value) = qr {
        // The column sits level with the middle of the QR.
        let dy = ((QR - (y - top)) / 2.0).max(0.0);
        shift(&mut p.plan.nodes[column..], dy);
        p.plan.nodes.push(Node { id: 8881, frame: Frame::new(left + PAD, top, QR, QR), props: Props::at(left + PAD, top), z, body: Body::Qr(value) });
        bottom = (y + dy).max(top + QR);
    }
    let foot = bottom + GAP;
    let lead = p.lead(Key::Back, Icon::Back);
    p.hint(8884, "Cancel", lead, left + PAD, foot, false, z);
    let height = foot + (37.0 * p.scale).max(p.line(Type::Label)) + PAD;
    p.surface(8001, Frame::new(left, 0.0, width, height), Fill::Sheet, Radius::Sheet, false, false, false, SHEET_Z);
    merge_words(&mut p.plan.nodes, from, 8880, |_| false);
    shift(&mut p.plan.nodes[from..], (H - height) / 2.0);
}

/// Moves `nodes` down by `dy`.
fn shift(nodes: &mut [Node], dy: f32) {
    for n in nodes {
        n.frame.y += dy;
        n.props.y += dy;
    }
}

/// The PIN's digits, each centred on a tinted square, as one layer at `id`. Returns their height.
fn digits(p: &mut Paint<'_>, id: u64, x: f32, y: f32, w: f32, code: &str, z: i32) -> f32 {
    let chars = code.chars().collect::<Vec<_>>();
    let (gap, h) = (20.0, 168.0);
    let side = ((w - gap * chars.len().saturating_sub(1) as f32) / chars.len().max(1) as f32).min(136.0);
    let radius = p.tokens.radius(Radius::Control) as u32;
    let ty = p.tokens.ty(Type::Display);
    let ink = p.ink(Role::Ink, false);
    let mut words = Vec::with_capacity(chars.len());
    let mut marks = Vec::with_capacity(chars.len());
    for (i, c) in chars.iter().enumerate() {
        let cx = i as f32 * (side + gap);
        marks.push(Mark { form: Form::Veil { w: side as u32, h: h as u32, radius, alpha: 15 }, colour: ink, x: cx, y: 0.0 });
        let mut word = p.word(c.to_string(), Type::Display, ink, cx, (h - p.line(Type::Display).min(130.0)) / 2.0, side);
        word.ty.px = ty.px;
        word.align = Align::Centre;
        words.push(word);
    }
    p.words(id, Frame::new(x, y, w, h), words, marks, z);
    h
}

/// A PIN to type in: the digits so far, the number keys and Pair.
fn enter_pin(p: &mut Paint<'_>, name: &str, code: &str) {
    veil(p, 8000, Frame::new(0.0, 0.0, W, H), true);
    let width = if p.scale > 1.0 { 1160.0 } else { 840.0 };
    let f = panel(p, width, 900.0);
    let x = f.x + 40.0;
    let mut y = f.y + 40.0;
    y += p.wrapped(8010, &format!("Pair with {name}"), Frame::new(x, y, width - 80.0, 0.0), Type::Title, Role::Ink, false, 32) + 20.0;
    y += p.wrapped(8011, "Enter the PIN shown on the PC.", Frame::new(x, y, width - 80.0, 0.0), Type::Body, Role::Ink2, false, 32) + 36.0;
    y += digits(p, 8020, x, y, width - 80.0, code, 33) + 36.0;
    keyboard(p, 8100, x, y, width - 80.0, true);
    p.button_lead(8500, Focus::SheetAction(0), "Pair", Some(Icon::Tick.into()), x, f.y + f.h - 104.0, true, false, 34);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::model::{Layer, Model, tests::{SteamShot, with_steam}};
    use crate::game::screens::parts::layer::sheet;
    use compositor_protocol::Backdrop;

    fn steam(shot: SteamShot) -> Model {
        with_steam(Model::new(Default::default(), Default::default(), vec![]), shot)
    }

    #[test]
    fn the_qr_sheet_is_its_words_the_qr_and_the_countdown_on_an_opaque_backdrop_with_nothing_to_press() {
        let m = steam(SteamShot::Qr);
        let tokens = lumatv_tokens::Tokens::shipped();
        let mut fonts = ui::Fonts::new();
        let mut p = Paint::new(&tokens, &m, &mut fonts);
        sheet(&mut p);
        let mut ids = p.plan.nodes.iter().map(|n| n.id).collect::<Vec<_>>();
        ids.sort_unstable();
        assert_eq!(ids, [8000, 8001, 8880, 8881, 8882, 8883]);
        assert!(p.plan.hits.is_empty());
        assert!(p.plan.nodes.iter().any(|n| matches!(&n.body, Body::Qr(url) if url.starts_with("https://s.team/"))));
        assert!(p.plan.nodes.iter().all(|n| !matches!(&n.body, Body::Surface(g) if g.backdrop != Backdrop::None)), "nothing captures what is under the sheet");
    }

    #[test]
    fn the_sign_out_question_marks_only_its_own_button_as_dangerous() {
        let mut m = steam(SteamShot::SignedOut);
        m.view.push(Layer::SignOut, m.view.focus);
        m.view.focus = Focus::SheetAction(0);
        let tokens = lumatv_tokens::Tokens::shipped();
        let mut fonts = ui::Fonts::new();
        let mut p = Paint::new(&tokens, &m, &mut fonts);
        sheet(&mut p);
        let danger = p.ink(Role::Danger, false);
        let icon = |id: u64| match &p.plan.nodes.iter().find(|n| n.id == id).expect("a button").body {
            Body::Text { marks, .. } => marks[0].colour,
            _ => panic!("{id} is not text"),
        };
        assert_eq!((icon(8821) == danger, icon(8825) == danger), (false, true));
    }

    #[test]
    fn no_pc_found_keeps_both_buttons_whole() {
        // A button takes its id and the next two (label, focus ring): "Enter address" at 8825 replaced "Try again"'s label.
        let m = crate::game::model::tests::no_pc_found();
        let tokens = lumatv_tokens::Tokens::shipped();
        let mut fonts = ui::Fonts::new();
        let mut p = Paint::new(&tokens, &m, &mut fonts);
        sheet(&mut p);
        let ids = p.plan.nodes.iter().map(|n| n.id).collect::<std::collections::BTreeSet<_>>();
        assert_eq!(ids.len(), p.plan.nodes.len());
        assert!(ids.contains(&8821) && ids.contains(&8825), "both labels are drawn");
    }

    #[test]
    fn the_key_form_draws_the_sixteen_hex_keys_for_the_key_and_the_letter_keys_for_the_account() {
        use crate::game::model::{Input, tests::key};
        let drawn = |m: &Model| {
            let tokens = lumatv_tokens::Tokens::shipped();
            let mut fonts = ui::Fonts::new();
            let mut p = Paint::new(&tokens, m, &mut fonts);
            sheet(&mut p);
            let keys = p.plan.hits.iter().filter(|(focus, _)| matches!(focus, Focus::Keyboard(_))).count();
            let value = match &p.plan.nodes.iter().find(|n| n.id == 8021).expect("the input").body {
                Body::Text { words, .. } => words[0].text.clone(),
                _ => panic!("the input is text"),
            };
            (keys, value)
        };
        let mut m = steam(SteamShot::SignedOut);
        m.begin_key_link(lumagame_core::ids::EngineId::new("steam").unwrap(), &mut Vec::new());
        key(&mut m, Key::Ok);
        m.step(Input::Text("0123456789ab".into()));
        assert_eq!(drawn(&m), (18, "0123 4567 89AB".to_owned()), "the key shows in groups of four");
        m.step(Input::Text("cdef0123456789abcdef".into()));
        m.view.focus = Focus::SheetAction(0);
        key(&mut m, Key::Ok);
        assert_eq!(drawn(&m), (45, "Steam ID or profile address".to_owned()), "the key is not shown again");
    }
}
