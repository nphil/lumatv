//! LG's DSP graph as `/proc/aaudio/dot_module_connection` prints it, one `a -> b;` edge per line.
//! A port of `lgroute-graph.h` (dts_restore_plus, LGPL-2.1-or-later).

/// A graph node, `<impl>_<8 hex uid>_<16 hex hash>_<port>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Node<'a> {
    pub imp: &'a str,
    pub uid: &'a str,
}

fn hex(text: &str, len: usize) -> bool {
    text.len() == len && text.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The implementation and uid a token names. The implementation name may hold underscores itself, and the shared-buffer
/// tokens (`gluepw_08130001_out`) carry no hash and are not nodes.
pub fn node(token: &str) -> Option<Node<'_>> {
    let (rest, _port) = token.rsplit_once('_')?;
    let (rest, hash) = rest.rsplit_once('_')?;
    let (imp, uid) = rest.rsplit_once('_')?;
    (hex(hash, 16) && hex(uid, 8) && !imp.is_empty() && imp.len() < 32).then_some(Node { imp, uid })
}

fn edge(line: &str) -> Option<(&str, &str)> {
    let mut words = line.split_whitespace();
    let from = words.next()?;
    if words.next()? != "->" {
        return None;
    }
    Some((from, words.next()?.split(';').next()?))
}

/// Whether the renderer whose lipsync stage is `tail` has joined the graph. LG's renderer for `/dev/lg/aren0` names it.
pub fn has_lipsync(graph: &str, tail: &str) -> bool {
    graph.contains(&format!("lipsync_{tail}_"))
}

/// What the graph shows of the PCM bypass converter: the one `srcbin` that an `rts` feeds and that feeds a lipsync stage.
/// LG's own main chain is `srcbin -> lipsync` without an `rts` and is never chosen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Chain {
    Missing,
    One { converter: String, lipsync: String },
    Several(usize),
}

/// The bypass converter feeding `lipsync` (any lipsync when `None`).
pub fn bypass_converter(graph: &str, lipsync: Option<&str>) -> Chain {
    let mut fed: Vec<&str> = Vec::new();
    let mut feeding: Vec<(&str, &str)> = Vec::new();
    for (from, to) in graph.lines().filter_map(edge) {
        let (Some(from), Some(to)) = (node(from), node(to)) else {
            continue;
        };
        if from.imp == "rts" && to.imp == "srcbin" && !fed.contains(&to.uid) {
            fed.push(to.uid);
        }
        if from.imp == "srcbin"
            && to.imp == "lipsync"
            && lipsync.is_none_or(|wanted| wanted == to.uid)
            && !feeding.contains(&(from.uid, to.uid))
        {
            feeding.push((from.uid, to.uid));
        }
    }
    feeding.retain(|(srcbin, _)| fed.contains(srcbin));
    match feeding.as_slice() {
        [] => Chain::Missing,
        [(converter, lipsync)] => Chain::One {
            converter: (*converter).to_owned(),
            lipsync: (*lipsync).to_owned(),
        },
        many => Chain::Several(many.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real C4 dump with a PCM bypass renderer loaded (33.22.52): `rts_0426001b -> srcbin_04270188 -> lipsync_042402f7`
    /// beside LG's own `queue -> srcbin -> sola -> lipsync`.
    const BYPASS: &str = include_str!("../tests/fixtures/c4-bypass-chain.dot");
    /// The same TV with no such renderer.
    const IDLE: &str = include_str!("../tests/fixtures/c4-no-chain.dot");

    #[test]
    fn nodes_parse_with_underscored_implementations_and_skip_hashless_tokens() {
        let n = node("srcbin_04270188_00000000ae6884cf_in0").unwrap();
        assert_eq!((n.imp, n.uid), ("srcbin", "04270188"));
        let n = node("shared_buf_04270188_00000000ae6884cf_out0").unwrap();
        assert_eq!((n.imp, n.uid), ("shared_buf", "04270188"));
        assert_eq!(node("gluepw_08130001_out"), None);
        assert_eq!(node("lipsync_0424_00000000ae6884cf_in0"), None);
        assert_eq!(node("lipsync_042402f7_00000000ae6884cx_in0"), None);
    }

    #[test]
    fn real_dump_yields_the_one_bypass_converter() {
        let chain = Chain::One {
            converter: "04270188".into(),
            lipsync: "042402f7".into(),
        };
        assert_eq!(bypass_converter(BYPASS, None), chain);
        assert_eq!(bypass_converter(BYPASS, Some("042402f7")), chain);
    }

    #[test]
    fn lg_main_chain_and_other_renderers_are_never_the_converter() {
        // 042402f6 is fed by sola, not by a converter.
        assert_eq!(bypass_converter(BYPASS, Some("042402f6")), Chain::Missing);
        assert_eq!(bypass_converter(IDLE, None), Chain::Missing);
        assert!(!IDLE.contains("rts_"));
    }

    #[test]
    fn lipsync_presence_follows_the_tail() {
        assert!(has_lipsync(BYPASS, "042402f7"));
        assert!(!has_lipsync(BYPASS, "042402f8"));
        assert!(!has_lipsync(IDLE, "042402f7"));
    }

    #[test]
    fn two_converters_are_ambiguous_until_the_tail_names_one() {
        let graph = format!(
            "{BYPASS}\n rts_042600ff_0000000000000001_out0 -> srcbin_042700ff_0000000000000002_in0;\n \
             srcbin_042700ff_0000000000000001_out0 -> lipsync_042402ff_0000000000000002_in0;\n"
        );
        assert_eq!(bypass_converter(&graph, None), Chain::Several(2));
        assert_eq!(
            bypass_converter(&graph, Some("042402ff")),
            Chain::One {
                converter: "042700ff".into(),
                lipsync: "042402ff".into()
            }
        );
        assert_eq!(bypass_converter(&graph, Some("04240000")), Chain::Missing);
    }

    #[test]
    fn a_converter_needs_an_rts_in_front() {
        let graph = "queue_04180001_0000000000000001_out0 -> srcbin_04270001_0000000000000002_in0;\n \
                     srcbin_04270001_0000000000000001_out0 -> lipsync_04240001_0000000000000002_in0;\n";
        assert_eq!(bypass_converter(graph, None), Chain::Missing);
        let graph = graph.replace("queue_04180001", "rts_04260001");
        assert!(matches!(bypass_converter(&graph, None), Chain::One { .. }));
    }

    #[test]
    fn repeated_edges_count_once() {
        let line = " rts_04260001_0000000000000001_out0 -> srcbin_04270001_0000000000000002_in0;\n \
                    srcbin_04270001_0000000000000001_out0 -> lipsync_04240001_0000000000000002_in0;\n";
        assert!(matches!(
            bypass_converter(&format!("{line}{line}"), None),
            Chain::One { .. }
        ));
    }
}
