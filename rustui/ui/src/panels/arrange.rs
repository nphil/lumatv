//! Which tiles the quick panel shows and in what order (`panels/QuickArrange.js` of the QML shell, docs/mocks/29-qp-edit and
//! 29-qp-add). The owner's arrangement is the theme's `quickPanel.tiles`: the ids shown, top to bottom. It is written the first time
//! the panel is edited and until then absent, which means the panel's own list (the `dflt` tiles in the catalogue's order). A saved id
//! that no tile answers to now (a plugin that is not connected) is not drawn, and a tile that is in the catalogue and not in the list
//! is one "Add a tile" offers. Nothing here touches the theme: each function takes lists and returns lists, and [`patch`] is what the
//! host writes.

use serde_json::{Value, json};

/// A tile of the catalogue: its id, and whether a panel with no arrangement shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cat {
    pub id: String,
    pub dflt: bool,
}

/// The ids shown: the saved list without ids nothing answers to and without repeats, or, with no list saved, the default tiles.
pub fn shown(saved: Option<&[String]>, all: &[Cat]) -> Vec<String> {
    let Some(saved) = saved else { return all.iter().filter(|c| c.dflt).map(|c| c.id.clone()).collect() };
    let mut out: Vec<String> = Vec::new();
    for id in saved {
        if all.iter().any(|c| c.id == *id) && !out.contains(id) {
            out.push(id.clone());
        }
    }
    out
}

/// What "Add a tile" offers: every tile of the catalogue that is not shown, in the catalogue's order.
pub fn missing(order: &[String], all: &[Cat]) -> Vec<String> {
    all.iter().filter(|c| !order.contains(&c.id)).map(|c| c.id.clone()).collect()
}

pub fn without(order: &[String], id: &str) -> Vec<String> {
    order.iter().filter(|x| x.as_str() != id).cloned().collect()
}

pub fn added(order: &[String], id: &str) -> Vec<String> {
    let mut out = order.to_vec();
    if !out.iter().any(|x| x == id) {
        out.push(id.to_string());
    }
    out
}

/// One place up (`dir` -1) or down (1); at an end, or for an id not in the list, the order is as it was.
pub fn moved(order: &[String], id: &str, dir: i32) -> Vec<String> {
    let mut out = order.to_vec();
    let Some(i) = out.iter().position(|x| x == id) else { return out };
    let j = i as i32 + dir;
    if j < 0 || j as usize >= out.len() {
        return out;
    }
    out.swap(i, j as usize);
    out
}

/// The theme patch that keeps an arrangement: the whole list every time (the host's patch merges one level deep).
pub fn patch(order: &[String]) -> Value {
    json!({"quickPanel": {"tiles": order}})
}

/// The saved arrangement of a theme document, if it has one.
pub fn saved(theme: &Value) -> Option<Vec<String>> {
    theme.pointer("/quickPanel/tiles").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> Vec<Cat> {
        [("picture", true), ("receiver", true), ("sleep", true), ("wifi", true), ("plugin.ambient.ambient", true), ("ai", true), ("screenoff", false), ("energy", false), ("oled", false)]
            .into_iter()
            .map(|(id, dflt)| Cat { id: id.into(), dflt })
            .collect()
    }

    fn v(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn no_arrangement_shows_the_default_tiles_in_the_catalogues_order() {
        assert_eq!(shown(None, &all()), v(&["picture", "receiver", "sleep", "wifi", "plugin.ambient.ambient", "ai"]));
    }

    #[test]
    fn a_saved_arrangement_is_shown_as_saved_without_ids_nothing_answers_to_and_without_repeats() {
        assert_eq!(shown(Some(&v(&["sleep", "picture", "gone", "sleep", "oled"])), &all()), v(&["sleep", "picture", "oled"]));
        assert_eq!(shown(Some(&[]), &all()), Vec::<String>::new());
    }

    #[test]
    fn add_a_tile_offers_what_is_not_shown_in_the_catalogues_order() {
        assert_eq!(missing(&v(&["picture", "ai"]), &all()), v(&["receiver", "sleep", "wifi", "plugin.ambient.ambient", "screenoff", "energy", "oled"]));
        assert_eq!(missing(&shown(None, &all()), &all()), v(&["screenoff", "energy", "oled"]));
    }

    #[test]
    fn remove_add_and_move_leave_the_input_alone_and_return_the_new_order() {
        let o = v(&["picture", "receiver", "sleep"]);
        assert_eq!(without(&o, "receiver"), v(&["picture", "sleep"]));
        assert_eq!(added(&o, "oled"), v(&["picture", "receiver", "sleep", "oled"]));
        assert_eq!(added(&o, "sleep"), o);
        assert_eq!(moved(&o, "sleep", -1), v(&["picture", "sleep", "receiver"]));
        assert_eq!(moved(&o, "picture", 1), v(&["receiver", "picture", "sleep"]));
        assert_eq!(o, v(&["picture", "receiver", "sleep"]));
    }

    #[test]
    fn a_move_past_either_end_changes_nothing() {
        let o = v(&["a", "b", "c"]);
        assert_eq!(moved(&o, "a", -1), o);
        assert_eq!(moved(&o, "c", 1), o);
        assert_eq!(moved(&o, "zz", 1), o);
    }

    #[test]
    fn the_patch_carries_the_whole_list_and_the_theme_gives_it_back() {
        let p = patch(&v(&["a", "b"]));
        assert_eq!(p, json!({"quickPanel": {"tiles": ["a", "b"]}}));
        assert_eq!(saved(&p), Some(v(&["a", "b"])));
        assert_eq!(saved(&json!({})), None);
    }
}
