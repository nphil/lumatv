//! Linking (sign-in, QR and PIN) and the question asked before signing out or removing a PC. The Accounts and PCs place itself is a
//! Settings list.
use crate::game::screens::paint::{Body, Fill, H, Node, Paint, W};
use crate::game::model::{Focus, Key};
use lumagame_core::ipc::AuthState;
use lumatv_tokens::{Radius, TypeRole as Type};
use ui::{components::{Align, Icon}, geometry::Frame, lucent::palette::PaletteRole as Role};
use crate::game::screens::parts::{Side, dialog::*, hints, keyboard::*};

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
    hints::bar(p, &m.hints(), Side::Right, m.hints_dim);
}
pub fn auth(p: &mut Paint<'_>) {
    let Some(flow) = &p.model.view.auth else {
        return;
    };
    veil(p, 8000, Frame::new(0.0, 0.0, W, H), true);
    let name = p.model.engine_name(&flow.engine);
    match &flow.state {
        Some(AuthState::DeviceCode { url, code, qr, .. }) => {
            let f = panel(p, if p.scale > 1.0 { 1728.0 } else { 1320.0 }, if p.scale > 1.0 { 972.0 } else { 660.0 });
            p.text(
                8010,
                &format!("Sign in to {name}"),
                Frame::new(f.x + 40.0, f.y + 40.0, f.w - 80.0, p.line(Type::Title) * 2.0),
                Type::Title,
                Role::Ink,
                false,
                32,
            );
            let top = f.y + 66.0 + p.line(Type::Title);
            let q = Frame::new(f.x + 40.0, top, 360.0, 360.0);
            p.plan.nodes.push(Node {
                id: 8020,
                frame: q,
                props: compositor_protocol::Props::at(q.x, q.y),
                z: 33,
                body: Body::Qr(qr.as_ref().unwrap_or(url).clone()),
            });
            let x = q.x + 416.0;
            let w = f.x + f.w - 40.0 - x;
            let mut y = top;
            y += p.wrapped(8021, "Scan the QR code with a phone, or open", Frame::new(x, y, w, 0.0), Type::Body, Role::Ink2, false, 33) + 12.0;
            y += p.address(8022, url, Frame::new(x, y, w, 0.0), if p.scale > 1.0 { Type::Label } else { Type::Title }, Role::Ink, false, 33) + 24.0;
            y += p.wrapped(8023, "Then enter this code", Frame::new(x, y, w, 0.0), Type::Body, Role::Ink2, false, 33) + 8.0;
            p.text(8024, code, Frame::new(x, y, w, p.line(Type::Title)), Type::Title, Role::Ink, false, 33);
            y += p.line(Type::Title) + 24.0;
            let remaining = p.model.auth_remaining().unwrap_or(0);
            p.text(
                8025,
                &format!("Code expires in {}:{:02}", remaining / 60, remaining % 60),
                Frame::new(x, y, w, p.line(Type::Caption)),
                Type::Caption,
                Role::Ink2,
                false,
                33,
            );
            if p.scale > 1.0 {
                p.wrapped(8030, "Waiting for sign-in", Frame::new(q.x, top + 384.0, q.w, 0.0), Type::Caption, Role::Ink2, false, 33);
            } else {
                p.hint(8030, "Waiting for sign-in", Icon::Hourglass, q.x, top + 384.0, false, 33);
            }
            let lead = p.lead(Key::Back, Icon::Back);
            p.button_lead(8040, Focus::SheetAction(0), "Cancel", Some(lead), f.x + 40.0, f.y + f.h - 104.0, false, false, 34);
        }
        Some(AuthState::Qr { url, .. }) => {
            // No address or code is shown: the phone's app scans the QR and asks for the approval itself.
            let f = panel(p, if p.scale > 1.0 { 1728.0 } else { 1320.0 }, if p.scale > 1.0 { 972.0 } else { 660.0 });
            p.text(8010, &format!("Sign in to {name}"), Frame::new(f.x + 40.0, f.y + 40.0, f.w - 80.0, p.line(Type::Title) * 2.0), Type::Title, Role::Ink, false, 32);
            let top = f.y + 66.0 + p.line(Type::Title);
            let q = Frame::new(f.x + 40.0, top, 360.0, 360.0);
            p.plan.nodes.push(Node { id: 8020, frame: q, props: compositor_protocol::Props::at(q.x, q.y), z: 33, body: Body::Qr(url.clone()) });
            let x = q.x + 416.0;
            let w = f.x + f.w - 40.0 - x;
            let mut y = top;
            y += p.wrapped(8021, &format!("Open the {name} app on a phone and scan this code."), Frame::new(x, y, w, 0.0), Type::Title, Role::Ink, false, 33) + 16.0;
            y += p.wrapped(8022, "The app asks to confirm the sign-in. No password is entered on the TV.", Frame::new(x, y, w, 0.0), Type::Body, Role::Ink2, false, 33) + 24.0;
            let remaining = p.model.auth_remaining().unwrap_or(0);
            p.text(8025, &format!("Closes in {}:{:02}", remaining / 60, remaining % 60), Frame::new(x, y, w, p.line(Type::Caption)), Type::Caption, Role::Ink2, false, 33);
            if p.scale > 1.0 {
                p.wrapped(8030, "Waiting for sign-in", Frame::new(q.x, top + 384.0, q.w, 0.0), Type::Caption, Role::Ink2, false, 33);
            } else {
                p.hint(8030, "Waiting for sign-in", Icon::Hourglass, q.x, top + 384.0, false, 33);
            }
            let lead = p.lead(Key::Back, Icon::Back);
            p.button_lead(8040, Focus::SheetAction(0), "Cancel", Some(lead), f.x + 40.0, f.y + f.h - 104.0, false, false, 34);
        }
        Some(AuthState::ShowPin { pin: code }) => pin(p, name, code, false),
        Some(AuthState::EnterPin { digits }) => {
            let entered = format!("{}{}", flow.pin, "_".repeat(usize::from(*digits).saturating_sub(flow.pin.len())));
            pin(p, name, &entered, true);
        }
        Some(AuthState::Approval { hint }) => {
            let f = panel(p, 1080.0, 580.0f32.max(p.line(Type::Title) * 2.0 + p.line(Type::Body) * 3.0 + 240.0));
            p.text(8010, "Approve this TV", Frame::new(f.x + 40.0, f.y + 40.0, f.w - 80.0, p.line(Type::Title)), Type::Title, Role::Ink, false, 32);
            p.wrapped(
                8020,
                hint.as_deref().unwrap_or("Approve the connection on the PC to finish pairing."),
                Frame::new(f.x + 40.0, f.y + 80.0 + p.line(Type::Title), f.w - 80.0, 0.0),
                Type::Body,
                Role::Ink2,
                false,
                33,
            );
            p.hint(8030, "Waiting for approval", Icon::Hourglass, f.x + 40.0, f.y + f.h - 210.0, false, 33);
            let lead = p.lead(Key::Back, Icon::Back);
            p.button_lead(8040, Focus::SheetAction(0), "Cancel", Some(lead), f.x + 40.0, f.y + f.h - 104.0, false, false, 34);
        }
        Some(AuthState::Failed { message, .. }) => {
            let f = panel(p, 1080.0, 500.0f32.max(p.line(Type::Title) + p.line(Type::Body) * 4.0 + 200.0));
            p.text(
                8010,
                "Connection not completed",
                Frame::new(f.x + 40.0, f.y + 40.0, f.w - 80.0, p.line(Type::Title) * 2.0),
                Type::Title,
                Role::Ink,
                false,
                32,
            );
            p.wrapped(8020, message, Frame::new(f.x + 40.0, f.y + 100.0 + p.line(Type::Title), f.w - 80.0, 0.0), Type::Body, Role::Ink2, false, 33);
            p.button(8040, Focus::SheetAction(0), "Try again", Some(Icon::Refresh), f.x + 40.0, f.y + f.h - 104.0, true, false, 34);
        }
        None if flow.no_pcs => {
            let manual = p
                .model
                .integration(&flow.engine)
                .and_then(|e| e.descriptor.as_ref())
                .is_some_and(|d| d.capabilities.has(lumagame_core::engine::Feature::ManualHost));
            let f = panel(p, 1080.0, 500.0f32.max(p.line(Type::Title) + p.line(Type::Body) * 4.0 + 200.0));
            p.text(8010, "No PCs found", Frame::new(f.x + 40.0, f.y + 40.0, f.w - 80.0, p.line(Type::Title) * 2.0), Type::Title, Role::Ink, false, 32);
            p.wrapped(
                8020,
                &format!("{name} found no PC on this network. Check that the PC is on and on the same network, or enter its address."),
                Frame::new(f.x + 40.0, f.y + 100.0 + p.line(Type::Title), f.w - 80.0, 0.0),
                Type::Body,
                Role::Ink2,
                false,
                33,
            );
            let y = f.y + f.h - 104.0;
            let w = p.button(8040, Focus::SheetAction(0), "Try again", Some(Icon::Refresh), f.x + 40.0, y, !manual, false, 34);
            if manual {
                p.button(8044, Focus::SheetAction(1), "Enter address", Some(Icon::Plus), f.x + 56.0 + w, y, true, false, 34);
            }
        }
        _ => {
            let f = panel(p, 960.0, 440.0);
            p.text(
                8010,
                &format!("Connecting to {name}"),
                Frame::new(f.x + 40.0, f.y + 40.0, f.w - 80.0, p.line(Type::Title) * 2.0),
                Type::Title,
                Role::Ink,
                false,
                32,
            );
            p.hint(8020, "Waiting for the integration", Icon::Hourglass, f.x + 40.0, f.y + 180.0, false, 33);
            let lead = p.lead(Key::Back, Icon::Back);
            p.button_lead(8040, Focus::SheetAction(0), "Cancel", Some(lead), f.x + 40.0, f.y + f.h - 104.0, false, false, 34);
        }
    }
}
fn pin(p: &mut Paint<'_>, name: &str, code: &str, enter: bool) {
    let width = if p.scale > 1.0 { 1160.0 } else { 840.0 };
    let f = panel(
        p,
        width,
        if enter {
            900.0
        } else if p.scale > 1.0 {
            800.0
        } else {
            650.0
        },
    );
    let x = f.x + 40.0;
    let mut y = f.y + 40.0;
    y += p.wrapped(8010, &format!("Pair with {name}"), Frame::new(x, y, width - 80.0, 0.0), Type::Title, Role::Ink, false, 32) + 20.0;
    y += p.wrapped(
        8011,
        if enter { "Enter the PIN shown on the PC." } else { "Enter this PIN in the pairing request on the PC." },
        Frame::new(x, y, width - 80.0, 0.0),
        Type::Body,
        Role::Ink2,
        false,
        32,
    ) + 36.0;
    let digits = code.chars().collect::<Vec<_>>();
    let gap = 20.0;
    let w = ((width - 80.0 - gap * (digits.len().saturating_sub(1)) as f32) / digits.len().max(1) as f32).min(136.0);
    for (i, c) in digits.iter().enumerate() {
        let f = Frame::new(x + i as f32 * (w + gap), y, w, 168.0);
        p.surface(8020 + i as u64 * 2, f, Fill::Tint(0.06), Radius::Control, false, false, false, 32);
        let mut word = p.word(c.to_string(), Type::Display, p.ink(Role::Ink, false), 0.0, (168.0 - p.line(Type::Display).min(130.0)) / 2.0, w);
        word.ty.px = p.tokens.ty(Type::Display).px;
        word.align = Align::Centre;
        p.words(8021 + i as u64 * 2, f, vec![word], vec![], 33);
    }
    y += 204.0;
    if enter {
        keyboard(p, 8100, x, y, width - 80.0, true, false, 6);
    } else {
        p.hint(8100, "Waiting for approval", Icon::Hourglass, x, y, false, 33);
    }
    let lead = if enter { Icon::Tick.into() } else { p.lead(Key::Back, Icon::Back) };
    p.button_lead(8500, Focus::SheetAction(0), if enter { "Pair" } else { "Cancel" }, Some(lead), x, f.y + f.h - 104.0, enter, false, 34);
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::screens::parts::layer::sheet;

    #[test]
    fn no_pc_found_keeps_both_buttons_whole() {
        // A button takes its id and the next two (label, focus ring): "Enter address" at 8041 replaced "Try again"'s label.
        let m = crate::game::model::tests::no_pc_found();
        let tokens = lumatv_tokens::Tokens::shipped();
        let mut fonts = ui::Fonts::new();
        let mut p = Paint::new(&tokens, &m, &mut fonts);
        sheet(&mut p);
        let ids = p.plan.nodes.iter().map(|n| n.id).collect::<std::collections::BTreeSet<_>>();
        assert_eq!(ids.len(), p.plan.nodes.len());
        assert!(ids.contains(&8041) && ids.contains(&8045), "both labels are drawn");
    }
}
