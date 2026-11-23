//! An integration's page, the first-run list of integrations, and linking: sign-in, QR and PIN.
use crate::game::screens::paint::{Body, Fill, Node, Paint, W};
use crate::game::model::{Focus, Key};
use lumagame_core::{engine::{EngineKind, Feature}, ipc::{AuthState, Link, Reach}, settings::{FieldKind, Value}};
use lumatv_tokens::{Radius, TypeRole as Type};
use ui::{components::{Align, Icon}, geometry::Frame, lucent::palette::PaletteRole as Role};
use crate::game::screens::parts::{dialog::*, keyboard::*, row::*, topbar::{self, Here}};

pub fn first_run(p: &mut Paint<'_>) {
    let attention = topbar::needs_attention(p);
    topbar::bar(p, Here::Elsewhere, attention);
    let title = "Add a way to play";
    let y = if p.scale > 1.0 { 220.0 } else { 300.0 };
    let title_h = p.wrapped(100, title, Frame::new(p.x, y, W - p.x * 2.0, 0.0), Type::Display, Role::Ink, false, 10);
    let sub = "Connect a service or a PC. Games appear here after a library is linked.";
    let sub_h = p.wrapped(101, sub, Frame::new(p.x, y + title_h + 12.0, W - p.x * 2.0, 0.0), Type::Body, Role::Ink2, false, 10);
    let top = y + title_h + sub_h + 68.0;
    let cols = if p.scale > 1.0 { 2 } else { 4 };
    let w = (W - p.x * 2.0 - p.gap * (cols - 1) as f32) / cols as f32;
    let focus = if let Focus::FirstRun(i) = p.model.view.focus { i } else { 0 };
    let start = focus / cols * cols;
    for (i, e) in p.model.integrations.iter().enumerate().skip(start).take(cols) {
        let f = Frame::new(p.x + (i - start) as f32 * (w + p.gap), top, w, 320.0);
        let from = p.plan.nodes.len();
        p.surface(200 + i as u64 * 10, f, Fill::Glass, Radius::Card, false, p.focused(Focus::FirstRun(i)), false, 10);
        let icon = match e.manifest.kind {
            EngineKind::Cloud => Icon::CloudOutline,
            EngineKind::Host => Icon::Tiles,
            EngineKind::Account => Icon::Person,
            _ => Icon::Package,
        };
        p.words(201 + i as u64 * 10, f, vec![], vec![p.mark(icon, 64.0, 28.0, 28.0, Role::Ink, false)], 11);
        let used = p.wrapped(202 + i as u64 * 10, &e.manifest.name, Frame::new(f.x + 28.0, f.y + 116.0, w - 56.0, 0.0), Type::Body, Role::Ink, false, 11);
        p.text(
            203 + i as u64 * 10,
            if e.manifest.kind == EngineKind::Host {
                "Pair a PC"
            } else if e.manifest.kind == EngineKind::Native {
                "Choose apps"
            } else {
                "Sign in"
            },
            Frame::new(f.x + 28.0, f.y + 132.0 + used, w - 56.0, p.line(Type::Caption)),
            Type::Caption,
            Role::Ink2,
            false,
            11,
        );
        if p.focused(Focus::FirstRun(i)) {
            p.zoom(from, f, p.tokens.focus.card_scale);
        }
        p.hit(Focus::FirstRun(i), f);
    }
    if p.model.integrations.is_empty() {
        p.text(200, "No integrations are installed", Frame::new(p.x, top, 1400.0, p.line(Type::Body)), Type::Body, Role::Ink2, false, 10);
    }
}
pub fn integration(p: &mut Paint<'_>) {
    let attention = topbar::needs_attention(p);
    topbar::bar(p, Here::Elsewhere, attention);
    let m = p.model;
    let Some(e) = m.view.selected_engine.as_ref().and_then(|id| m.integration(id)) else {
        return;
    };
    let x = p.x;
    let w = W - p.x * 2.0;
    let mut y = 170.0;
    let enabled = m.enabled(&e.manifest.id);
    p.text(100, &e.manifest.name, Frame::new(x + 26.0, y, w - 52.0, p.line(Type::Title)), Type::Title, Role::Ink, false, 10);
    y += p.line(Type::Title) + 24.0;
    y += row(p, 200, Focus::Setting(0), "Enable integration", Some("Runs only while enabled"), Control::Toggle(enabled), Frame::new(x, y, w, 0.0));
    if !enabled {
        return;
    }
    if let Some((_, error)) = &e.error {
        y += p.wrapped(225, error, Frame::new(x + 26.0, y + 16.0, w - 52.0, 0.0), Type::Body, Role::Warning, false, 10) + 32.0;
    }
    if let Some(d) = &e.descriptor {
        if !d.capabilities.auth.is_empty() {
            y += row(
                p,
                230,
                Focus::Setting(1),
                if e.manifest.kind == EngineKind::Host { "Pair a PC" } else if e.can_sign_out() { "Switch account" } else { "Sign in" },
                None,
                Control::Value("Connect"),
                Frame::new(x, y, w, 0.0),
            );
        }
        if d.capabilities.has(Feature::ManualHost) {
            y += row(p, 260, Focus::Setting(2), "Add PC by address", None, Control::Value("Add"), Frame::new(x, y, w, 0.0));
        }
    }
    for (i, target) in e.targets.iter().enumerate() {
        let state = match (target.link, target.reach) {
            (Link::Expired, _) => "Sign in again",
            (Link::Linked, Reach::Online) => "Connected",
            (_, Reach::Asleep) => "Asleep",
            (_, Reach::Offline) => "Offline",
            _ => "Not linked",
        };
        y += row(p, 10_000 + i as u64 * 100, Focus::Target(i), &target.name, None, Control::Value(state), Frame::new(x, y, w, 0.0));
    }
    if m.can_refresh(e) {
        let note = match m.refresh_line() {
            Some(crate::game::model::RefreshLine::Working { text, seconds }) => format!("{text}…{}", seconds.map_or(String::new(), |s| format!("  {s} s"))),
            Some(crate::game::model::RefreshLine::Done { text, .. }) => text,
            None => "Checks the account for games added or removed".to_owned(),
        };
        y += row(p, 9_500, Focus::Setting(4), "Refresh libraries", Some(&note), Control::Value(if m.refreshing() { "Working" } else { "Refresh" }), Frame::new(x, y, w, 0.0));
    }
    if e.can_sign_out() {
        y += row(p, 9_000, Focus::Setting(3), "Sign out", Some("Removes the saved sign-in from this TV"), Control::Value("Sign out"), Frame::new(x, y, w, 0.0));
    }
    for (i, field) in e.schema.fields().enumerate() {
        let saved = m.state().engines.get(&e.manifest.id).and_then(|e| e.settings.get(&field.key)).cloned().or_else(|| field.kind.default());
        let label = &field.label;
        let description = field.help.as_deref();
        let value = match &saved {
            Some(Value::Text(v)) => v.clone(),
            Some(Value::Int(v)) => v.to_string(),
            Some(Value::Bool(v)) => v.to_string(),
            None => String::new(),
        };
        let choices;
        let control = match &field.kind {
            FieldKind::Toggle { .. } => Control::Toggle(saved == Some(Value::Bool(true))),
            FieldKind::Choice { options, .. } => {
                choices = options.iter().map(|c| c.label.as_str()).collect::<Vec<_>>();
                Control::Segments(&choices, options.iter().position(|o| o.id == value).unwrap_or(0))
            }
            FieldKind::Action => Control::Value("Open"),
            FieldKind::Note => Control::None,
            _ => Control::Value(&value),
        };
        y += row(
            p,
            100_000 + i as u64 * 100,
            if field.kind == FieldKind::Note { Focus::None } else { Focus::EngineField(i) },
            label,
            description,
            control,
            Frame::new(x, y, w, 0.0),
        );
    }
}
pub fn auth(p: &mut Paint<'_>) {
    let Some(flow) = &p.model.view.auth else {
        return;
    };
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
        keyboard(p, 8100, x, y, width - 80.0, true, false);
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
