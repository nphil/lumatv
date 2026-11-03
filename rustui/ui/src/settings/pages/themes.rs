//! Theme families, shown in small rows so the Settings list can virtualize them.

use crate::settings::ctx::Ctx;
use crate::settings::page::Page;
use crate::settings::row::{Arg, Card, Op, Row};
use ui::lucent::palette::Mode;

pub fn page() -> Page {
    Page::new("themes", "Themes", "palette").parent("look").back("Home & look").tabs(&["Light", "Dark"]).rows(rows)
}

fn rows(c: &Ctx) -> Vec<Row> {
    let mut rows = Vec::new();
    let scale = c.local.get("themes.text_scale").as_f64().unwrap_or_else(|| c.theme_num("type.scale", 1.0));
    let columns = if scale >= 1.5 { 2 } else { 3 };
    for (tab, (name, mode)) in [("light", Mode::Light), ("dark", Mode::Dark)].into_iter().enumerate() {
        if c.local.get("themes.error").as_bool() == Some(true) {
            rows.push(Row::note("Could not save theme. Try again.", tab));
        }
        let selected = c.theme.pointer(&format!("/palette/{name}")).and_then(|v| v.as_str());
        for (line, families) in lumatv_tokens::themes::catalog().chunks(columns).enumerate() {
            let cards: Vec<Card> = families.iter().map(|family| Card {
                title: family.name.into(),
                icon: format!("theme:{name}:{}", family.id),
                ..Card::default()
            }).collect();
            let mut row = Row::cards(&format!("themes:{name}:{line}"), cards).tab(tab).admin();
            row.cols = columns;
            row.index = families.iter().position(|family| selected == Some(family.id));
            row = row.act(move |arg| match arg {
                Arg::Index(i) => families.get(i).map(|family| Op::ThemeChoice(family.patch(mode))).into_iter().collect(),
                _ => Vec::new(),
            });
            rows.push(row);
        }
    }
    rows
}
