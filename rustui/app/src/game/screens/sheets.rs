use super::{
    pages::{keyboard, logo, stage, way},
    paint::{Body, Fill, H, Node, PAGE_EXTENT, Paint, SHEET_GROUP, ScrollGroup, W},
};
use compositor_protocol::{Edges, Rect};
use crate::game::{helper::LeadjoyState, model::{Focus, Key, Page, Section, Sheet, Sort}};
use lumagame_core::ipc::AuthState;
use lumatv_tokens::{Radius, TypeRole as Type};
use ui::{
    components::{Align, Icon},
    geometry::Frame,
    lucent::palette::PaletteRole as Role,
};

fn curtain(p: &mut Paint<'_>) {
    let colour = if p.model.view.sheet == Some(Sheet::EndSession) {
        let mut colour = p.straight(Role::Canvas, false);
        colour.a = 247;
        colour.premultiply(0.0)
    } else {
        p.ink(Role::Scrim, false)
    };
    p.solid(8000, Frame::new(0.0, 0.0, W, H), colour, 0.0, 25);
}
fn panel(p: &mut Paint<'_>, width: f32, height: f32) -> Frame {
    let frame = Frame::new((W - width) / 2.0, (H - height) / 2.0, width, height);
    p.surface(8001, frame, Fill::Sheet, Radius::Sheet, false, false, true, 30);
    frame
}
pub fn sheet(p: &mut Paint<'_>) {
    let Some(sheet) = p.model.view.sheet else {
        return;
    };
    curtain(p);
    match sheet {
        Sheet::Chooser => chooser(p),
        Sheet::Sort => list(p, "Sort by", Sort::ALL.iter().map(|s| s.name().into()).collect()),
        Sheet::Genre => {
            let mut names = vec!["All genres".into()];
            names.extend(p.model.genres());
            list(p, "Genre", names);
        }
        Sheet::Theme => themes(p),
        Sheet::Auth => auth(p),
        Sheet::EndSession => end_session(p),
        Sheet::AddHost | Sheet::EditText => edit_text(p),
        Sheet::Delete => delete(p),
    }
}
fn chooser(p: &mut Paint<'_>) {
    let Some(game) = p.model.selected_game() else {
        return;
    };
    let row_h = if p.scale > 1.0 { p.line(Type::Body) + p.line(Type::Caption) * 2.0 + 44.0 } else { 80.0 };
    let header = p.line(Type::Title) + 26.0;
    let footer = p.tokens.target.max(p.line(Type::Label) + 24.0) + 26.0;
    let remember_h = 80.0f32.max(p.line(Type::Body) + 24.0);
    let height = (74.0 + header + game.sources.len() as f32 * (row_h + 6.0) + remember_h + footer).min(H - 108.0);
    let f = panel(p, 1320.0, height);
    let x = f.x + 36.0;
    let w = f.w - 72.0;
    let mut y = f.y + 40.0;
    p.text(8010, &format!("Where to play {}", game.title), Frame::new(x, y, w, p.line(Type::Title)), Type::Title, Role::Ink, false, 32);
    y += header;
    let focus = if let Focus::SheetRow(i) = p.model.view.focus {
        i
    } else {
        p.model.view.chooser_selected.as_ref().and_then(|key| game.sources.iter().position(|s| s.key() == *key)).unwrap_or(0)
    };
    let room = height - 74.0 - header - footer - remember_h;
    let visible = (room / (row_h + 6.0)).floor().max(1.0) as usize;
    let first = focus.saturating_sub(visible - 1);
    let view = Rect { x, y, w, h: visible as f32 * (row_h + 6.0) - 6.0 };
    scrolling(p, view, row_h + 6.0, game.sources.len(), first, 4.0, |p, i, y| {
        way(p, 8100 + i as u64 * 20, i, game.sources[i].key(), Frame::new(x, y, w, 0.0), true);
    });
    let remember_y = f.y + height - 34.0 - footer - remember_h;
    let rf = Frame::new(x, remember_y, w, remember_h);
    let from = p.plan.nodes.len();
    if p.focused(Focus::Remember) {
        p.surface(8700, rf, Fill::Tint(0.09), Radius::Row, false, true, false, 32);
    }
    p.text(
        8701,
        "Remember for this game",
        Frame::new(x + 26.0, remember_y + (remember_h - p.line(Type::Body)) / 2.0, w - 160.0, p.line(Type::Body)),
        Type::Body,
        Role::Ink,
        false,
        33,
    );
    p.toggle(8702, x + w - 108.0, remember_y + (remember_h - 48.0) / 2.0, p.model.view.remember, false, 33);
    if p.focused(Focus::Remember) {
        p.zoom(from, rf, p.tokens.focus.card_scale);
    }
    p.hit(Focus::Remember, rf);
    let y = f.y + height - 34.0 - p.tokens.target.max(p.line(Type::Label) + 24.0);
    let back = p.lead(Key::Back, Icon::Back);
    let choose = p.lead(Key::Ok, Icon::Circle);
    let h = p.hint(8710, "Back", back, x, y + 12.0, false, 33);
    p.hint(8714, "Choose", choose, x + h + 24.0, y + 12.0, false, 33);
    let label = "Play here";
    let button_w = p.width(label, Type::Label) + 110.0;
    p.button(8720, Focus::SheetAction(0), label, Some(Icon::Play), x + w - button_w, y, true, false, 34);
}
fn list(p: &mut Paint<'_>, title: &str, values: Vec<String>) {
    let h = 80.0f32.max(p.line(Type::Body) + 24.0);
    let height = (p.line(Type::Title) + 106.0 + values.len() as f32 * (h + 6.0)).min(H - 108.0);
    let f = panel(p, 960.0, height);
    p.text(8010, title, Frame::new(f.x + 36.0, f.y + 40.0, f.w - 72.0, p.line(Type::Title)), Type::Title, Role::Ink, false, 32);
    let y = f.y + 66.0 + p.line(Type::Title);
    let visible = ((height - p.line(Type::Title) - 106.0) / (h + 6.0)).floor().max(1.0) as usize;
    let first = if let Focus::SheetRow(i) = p.model.view.focus { i.saturating_sub(visible - 1) } else { 0 };
    let view = Rect { x: f.x + 36.0, y, w: f.w - 72.0, h: visible as f32 * (h + 6.0) - 6.0 };
    scrolling(p, view, h + 6.0, values.len(), first, 4.0, |p, i, y| {
        let row = Frame::new(f.x + 36.0, y, f.w - 72.0, h);
        let from = p.plan.nodes.len();
        p.surface(
            8100 + i as u64 * 3,
            row,
            Fill::Tint(if p.focused(Focus::SheetRow(i)) { 0.09 } else { 0.0 }),
            Radius::Row,
            false,
            p.focused(Focus::SheetRow(i)),
            false,
            32,
        );
        p.text(
            8101 + i as u64 * 3,
            &values[i],
            Frame::new(row.x + 26.0, y + (h - p.line(Type::Body)) / 2.0, row.w - 52.0, p.line(Type::Body)),
            Type::Body,
            Role::Ink,
            false,
            33,
        );
        if p.focused(Focus::SheetRow(i)) {
            p.zoom(from, row, p.tokens.focus.card_scale);
        }
        p.hit(Focus::SheetRow(i), row);
    });
}
fn themes(p: &mut Paint<'_>) {
    let f = panel(p, 1728.0, 972.0);
    p.text(8010, "Choose a theme", Frame::new(f.x + 36.0, f.y + 36.0, f.w - 72.0, p.line(Type::Title)), Type::Title, Role::Ink, false, 32);
    let cols = if p.scale > 1.0 { 3 } else { 4 };
    let card_h = if p.scale > 1.0 { 270.0 } else { 212.0 };
    let w = (f.w - 72.0 - 24.0 * (cols - 1) as f32) / cols as f32;
    let rows = ((f.h - p.line(Type::Title) - 108.0) / (card_h + 24.0)).floor() as usize;
    let focus = if let Focus::Theme(i) = p.model.view.focus { i } else { 0 };
    let first_row = (focus / cols).saturating_sub(rows.saturating_sub(1));
    let top = f.y + 72.0 + p.line(Type::Title);
    let count = lumatv_tokens::themes::game_options().len();
    let view = Rect { x: f.x + 36.0, y: top, w: f.w - 72.0, h: rows as f32 * (card_h + 24.0) - 24.0 };
    scrolling(p, view, card_h + 24.0, count.div_ceil(cols), first_row, 8.0, |p, row, y| {
        for i in row * cols..((row + 1) * cols).min(count) {
            super::settings::theme_card(p, 8100 + i as u64 * 10, Focus::Theme(i), i, Frame::new(f.x + 36.0 + (i % cols) as f32 * (w + 24.0), y, w, card_h));
        }
    });
}
fn auth(p: &mut Paint<'_>) {
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
fn end_session(p: &mut Paint<'_>) {
    let (title, body, confirm) = p.model.end_copy();
    let f = panel(p, 1080.0, if p.scale > 1.0 { 700.0 } else { 400.0 });
    let x = f.x + 40.0;
    p.text(8010, title, Frame::new(x, f.y + 40.0, f.w - 80.0, p.line(Type::Title) * 2.0), Type::Title, Role::Ink, false, 32);
    p.wrapped(8020, &body, Frame::new(x, f.y + 78.0 + p.line(Type::Title), f.w - 80.0, 0.0), Type::Body, Role::Ink2, false, 32);
    let y = f.y + f.h - 104.0;
    let w = p.button(8030, Focus::SheetAction(0), "Keep playing", Some(Icon::Play), x, y, true, false, 34);
    p.button(8040, Focus::SheetAction(1), confirm, Some(Icon::Stop), x + w + 16.0, y, false, false, 34);
}
fn delete(p: &mut Paint<'_>) {
    let field = match &p.model.leadjoy.state {
        LeadjoyState::Ready(profile) => p.model.view.edit_field.and_then(|i| profile.schema.fields().nth(i)),
        _ => None,
    };
    let (title, body) = field.map_or_else(|| ("Delete".into(), String::new()), |field| (field.label.clone(), field.help.clone().unwrap_or_default()));
    let f = panel(p, 1080.0, if p.scale > 1.0 { 700.0 } else { 400.0 });
    let x = f.x + 40.0;
    p.text(8010, &title, Frame::new(x, f.y + 40.0, f.w - 80.0, p.line(Type::Title) * 2.0), Type::Title, Role::Ink, false, 32);
    p.wrapped(8020, &body, Frame::new(x, f.y + 78.0 + p.line(Type::Title), f.w - 80.0, 0.0), Type::Body, Role::Ink2, false, 32);
    let y = f.y + f.h - 104.0;
    let keep = p.lead(Key::Back, Icon::Back);
    let w = p.button_lead(8030, Focus::SheetAction(0), "Keep", Some(keep), x, y, true, false, 34);
    p.button(8040, Focus::SheetAction(1), "Delete", Some(Icon::Close), x + w + 16.0, y, false, false, 34);
}
fn edit_text(p: &mut Paint<'_>) {
    let editing = p.model.view.sheet == Some(Sheet::EditText);
    let diagnostics = editing && p.model.view.page == Page::Settings && p.model.view.section == Section::Diagnostics;
    let secret = diagnostics && p.model.view.edit_field == Some(4);
    let label = if diagnostics {
        match p.model.view.edit_field { Some(2) => "Server URL", Some(3) => "Topic", Some(4) => "Write token", _ => "Edit setting" }
    } else if editing && p.model.view.page == Page::Leadjoy {
        match &p.model.leadjoy.state {
            LeadjoyState::Ready(profile) => p.model.view.edit_field.and_then(|i| profile.schema.fields().nth(i)).map_or("Edit setting", |field| field.label.as_str()),
            _ => "Edit setting",
        }
    } else if editing {
        p.model
            .view
            .selected_engine
            .as_ref()
            .and_then(|id| p.model.integration(id))
            .and_then(|e| e.schema.fields().nth(p.model.view.edit_field?))
            .map_or("Edit setting", |f| f.label.as_str())
    } else {
        "Add a PC"
    };
    let f = panel(p, if p.scale > 1.0 { 1728.0 } else { 1500.0 }, if p.scale > 1.0 { 972.0 } else { 850.0 });
    let x = f.x + 40.0;
    let w = f.w - 80.0;
    p.text(8010, label, Frame::new(x, f.y + 32.0, w, p.line(Type::Title)), Type::Title, Role::Ink, false, 32);
    let y = f.y + 48.0 + p.line(Type::Title);
    let input_h = p.line(Type::Body) + 24.0;
    p.surface(8020, Frame::new(x, y, w, input_h), Fill::Tint(0.09), Radius::Control, false, false, false, 32);
    let placeholder = if editing { "Enter a value" } else { "IP address or hostname" };
    let masked = if secret { "•".repeat(p.model.view.address.chars().count()) } else { String::new() };
    p.text(
        8021,
        if p.model.view.address.is_empty() { placeholder } else if secret { &masked } else { &p.model.view.address },
        Frame::new(x + 24.0, y + 12.0, w - 48.0, p.line(Type::Body)),
        Type::Body,
        Role::Ink,
        false,
        33,
    );
    keyboard(p, 8100, x, y + input_h + 16.0, w, false, false);
    let button_h = p.tokens.target.max(p.line(Type::Label) + 24.0);
    p.button(
        8500,
        Focus::SheetAction(0),
        if editing { "Save" } else { "Add PC" },
        Some(if editing { Icon::Tick } else { Icon::Plus }),
        x,
        f.y + f.h - 34.0 - button_h,
        true,
        false,
        34,
    );
    if let Some(notice) = &p.model.view.notice {
        p.text(8510, notice, Frame::new(x + 360.0, f.y + f.h - 32.0 - button_h, w - 360.0, button_h), Type::Caption, Role::Warning, false, 34);
    }
}
pub fn overlay(p: &mut Paint<'_>) {
    let Some(session) = p.model.session() else {
        return;
    };
    stage(p, session.game, 1080.0, 0.0, 1080.0, false, true);
    let button_h = p.tokens.target.max(p.line(Type::Label) + 24.0);
    let actions_y = H - 118.0 - button_h * if p.scale >= 2.0 { 2.0 } else { 1.0 } - if p.scale >= 2.0 { 16.0 } else { 0.0 };
    let mut stats = Vec::new();
    let s = &p.model.view.stats;
    if let Some(v) = s.rtt_ms {
        stats.push(format!("{v:.0} ms network"));
    }
    if let Some(v) = s.loss_pct {
        stats.push(format!("{v:.1}% packet loss"));
    }
    if let Some(v) = s.bitrate_kbps {
        stats.push(format!("{:.0} Mb/s", v as f32 / 1000.0));
    }
    if let Some(v) = s.host_ms {
        stats.push(format!("{v:.1} ms host"));
    }
    if let Some(v) = s.client_ms {
        stats.push(format!("{v:.1} ms client"));
    }
    let stats_text = stats.join("  ·  ");
    let stats_h = if stats.is_empty() { 0.0 } else { ui::components::wrap(p.fonts, &stats_text, p.ty(Type::Label), W - p.x * 2.0).len() as f32 * p.line(Type::Label) + 10.0 };
    let mut session_line = vec![p.model.engine_name(&session.source.engine).to_string()];
    if let Some(stream) = &p.model.view.stream {
        session_line.push(format!(
            "{}p · {} fps · {} · {}",
            stream.height,
            stream.fps,
            stream.codec.as_str().to_uppercase(),
            if stream.hdr { "HDR" } else { "SDR" }
        ));
    }
    let elapsed = p.model.elapsed();
    session_line.push(format!("{}:{:02} elapsed", elapsed / 60, elapsed % 60));
    let line = session_line.join("  ·  ");
    let line_h = ui::components::wrap(p.fonts, &line, p.ty(Type::Label), W - p.x * 2.0).len() as f32 * p.line(Type::Label);
    let logo_y = actions_y - 40.0 - stats_h - line_h - 30.0 - 240.0;
    logo(p, 100, session.game, Frame::new(p.x, logo_y, 640.0, 240.0));
    p.wrapped(101, &line, Frame::new(p.x, logo_y + 270.0, W - p.x * 2.0, line_h), Type::Label, Role::Ink, true, 10);
    if !stats.is_empty() {
        p.wrapped(102, &stats_text, Frame::new(p.x, logo_y + 270.0 + line_h + 10.0, W - p.x * 2.0, stats_h), Type::Label, Role::Ink2, true, 10);
    }
    if let Some(reason) = p.model.view.stream.as_ref().and_then(|s| s.sdr_reason.as_deref()) {
        p.text(103, reason, Frame::new(p.x, logo_y - 60.0, W - p.x * 2.0, p.line(Type::Caption)), Type::Caption, Role::Warning, true, 10);
    }
    let mut x = p.x;
    let stats_label = format!("Stats · {}", p.model.prefs.stats.mode.name());
    let mut y = actions_y;
    let stats = p.lead(Key::Menu, Icon::Gauge);
    let buttons = [("Resume", Icon::Play.into()), ("Stream settings", Icon::Sliders.into()), (stats_label.as_str(), stats), ("End session", Icon::Stop.into())];
    for (i, (label, lead)) in buttons.into_iter().enumerate() {
        if p.scale >= 2.0 && i == 2 { x = p.x; y += button_h + 16.0; }
        x += p.button_lead(110 + i as u64 * 5, Focus::Overlay(i), label, Some(lead), x, y, i == 0, true, 12) + 16.0;
    }
}
pub fn media(p: &mut Paint<'_>) {
    if let Some(item) = p.model.view.selected.and_then(|id| p.model.details.get(&id)).and_then(|d| d.media.get(p.model.view.media)) {
        if let Some(path) = p.model.art_path(&item.image) {
            p.image(100, path, Frame::new(p.x, 140.0, W - p.x * 2.0, H - 220.0), 0.0, true, None, 10);
        }
        p.text(101, &item.title, Frame::new(p.x + 88.0, p.y + 12.0, 1500.0, p.line(Type::Label)), Type::Label, Role::Ink, false, 12);
    }
    p.icon_button(110, Focus::HubTop(0), Icon::Back, p.x, p.y, false, 12);
}

/// Lays out rows `0..rows`, each `pitch` tall, of a list that shows from row `first` inside `view`, as a scroll group: the rows are built
/// at their place for that offset and the group's spring carries the list there, so a list that moves on goes on from the speed it has.
/// Only the rows the travel from the list's present offset to that one passes through are built. `pad` is how far the clip reaches past
/// `view` above and below (and 32 px at the sides) for a focused row's lift; it is less than the gap between rows.
fn scrolling(p: &mut Paint<'_>, view: Rect, pitch: f32, rows: usize, first: usize, pad: f32, mut row: impl FnMut(&mut Paint<'_>, usize, f32)) {
    let offset = first as f32 * pitch;
    let from = p.scroll_positions.get(&SHEET_GROUP).copied().unwrap_or(offset);
    let (lo, hi) = (from.min(offset), from.max(offset));
    let start = p.plan.nodes.len();
    for i in 0..rows {
        let top = i as f32 * pitch;
        if top + pitch <= lo - pad || top >= hi + view.h + pad {
            continue;
        }
        let hits = p.plan.hits.len();
        row(p, i, view.y + top - offset);
        for k in hits..p.plan.hits.len() {
            let focus = p.plan.hits[k].0;
            p.plan.hit_groups.insert(focus, SHEET_GROUP);
        }
    }
    let nodes = p.plan.nodes[start..].iter().map(|node| node.id).collect();
    p.plan.groups.push(ScrollGroup {
        id: SHEET_GROUP,
        parent: None,
        clip: Rect { x: view.x - 32.0, y: view.y - pad, w: view.w + 64.0, h: view.h + 2.0 * pad },
        feather: Edges::NONE,
        horizontal: false,
        offset,
        extent: PAGE_EXTENT,
        nodes,
    });
}
#[cfg(test)]
mod tests {
    use super::*;

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
