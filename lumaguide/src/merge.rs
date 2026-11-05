//! One channel's guide from several lists in order of trust: the online sources in the owner's order, then the broadcast's own.
//!
//! A list wins wherever it has a programme; a later list only fills the gaps of the ones before it. A programme that overlaps one
//! already kept is left out whole rather than cut: a gap of a few minutes where two lists disagree about a boundary reads better than a
//! programme with a start nobody broadcast.

use crate::model::Programme;

/// The programmes of `layers` (each sorted by start, the first trusted most), as one list sorted by start in which no two overlap.
pub fn merge(layers: &[&[Programme]]) -> Vec<Programme> {
    let mut kept: Vec<Programme> = Vec::with_capacity(layers.first().map_or(0, |l| l.len()));
    // the spans kept, sorted and apart: what a later programme must not touch
    let mut spans: Vec<(i64, i64)> = Vec::new();
    for layer in layers {
        for p in layer.iter().filter(|p| p.end > p.start) {
            let i = spans.partition_point(|&(_, end)| end <= p.start);
            if spans.get(i).is_some_and(|&(start, _)| start < p.end) {
                continue;
            }
            spans.insert(i, (p.start, p.end));
            kept.push(p.clone());
        }
    }
    kept.sort_by_key(|p| p.start);
    kept
}

/// The online guide over the broadcast one.
pub fn over(online: &[Programme], broadcast: &[Programme]) -> Vec<Programme> {
    if online.is_empty() {
        return broadcast.to_vec();
    }
    merge(&[online, broadcast])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(title: &str, start_min: i64, end_min: i64) -> Programme {
        Programme {
            id: title.into(),
            channel: "c".into(),
            title: title.into(),
            subtitle: String::new(),
            text: String::new(),
            start: start_min * 60_000,
            end: end_min * 60_000,
            rating: String::new(),
            captions: false,
            described: false,
        }
    }

    fn titles(list: &[Programme]) -> Vec<&str> {
        list.iter().map(|p| p.title.as_str()).collect()
    }

    #[test]
    fn online_wins_where_it_has_something_and_the_broadcast_fills_the_gaps() {
        let online = [p("News", 0, 60), p("Movie", 120, 240)];
        // the broadcast's news runs a minute late; its 60-120 show fills the hole; its movie is covered
        let broadcast = [
            p("news", 1, 61),
            p("Quiz", 61, 120),
            p("movie", 120, 240),
            p("Late", 240, 270),
        ];
        assert_eq!(
            titles(&over(&online, &broadcast)),
            ["News", "Quiz", "Movie", "Late"]
        );
    }

    #[test]
    fn a_programme_that_straddles_a_kept_one_is_left_out_whole() {
        let online = [p("A", 0, 30), p("B", 60, 90)];
        let broadcast = [p("x", 20, 70), p("y", 30, 60)];
        assert_eq!(titles(&over(&online, &broadcast)), ["A", "y", "B"]);
    }

    #[test]
    fn sources_are_trusted_in_order_and_overlaps_inside_a_list_are_dropped() {
        let first = [p("P1", 0, 60), p("P1 again", 30, 90)];
        let second = [p("S1", 60, 120), p("S2", 120, 180)];
        let broadcast = [p("B", 150, 200), p("B2", 180, 240)];
        assert_eq!(
            titles(&merge(&[&first, &second, &broadcast])),
            ["P1", "S1", "S2", "B2"]
        );
        assert_eq!(titles(&over(&[], &broadcast)), ["B", "B2"]);
    }
}
