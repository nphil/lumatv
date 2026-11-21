//! Bounded decoding of receiver-reported AppCommand and Deviceinfo data.
use crate::settings::{Prerequisites, Source, SourceChoice};

pub(crate) const BASIC_REQUEST: &str = "<tx><cmd id=\"1\">GetToneControl</cmd><cmd id=\"1\">GetRenameSource</cmd><cmd id=\"1\">GetDeletedSource</cmd><cmd id=\"1\">GetAllZoneVolume</cmd></tx>";
/// Asks which Audyssey controls, surround parameters and sound-mode buttons the receiver offers for what it plays now.
pub(crate) const PARAMETER_REQUEST: &str = "<tx><cmd id=\"3\"><name>GetAudyssey</name><list><param name=\"dynamiceq\"/><param name=\"reflevoffset\"/><param name=\"dynamicvol\"/><param name=\"multeq\"/></list></cmd><cmd id=\"3\"><name>GetSurroundParameter</name><list><param name=\"cinemaeq\"/><param name=\"loudness\"/><param name=\"dialogcontrol\"/><param name=\"lfe\"/><param name=\"dyncomp\"/></list></cmd><cmd id=\"3\"><name>GetSoundMode</name><list><param name=\"movie\"/><param name=\"music\"/><param name=\"game\"/><param name=\"pure\"/></list></cmd></tx>";

// A deliberately narrow XML reader: reject malformed structure, DTDs and entities rather
// than making unsupported fields look like receiver-reported capabilities.
#[derive(Debug)]
struct Node<'a> {
    name: &'a str,
    attrs: &'a str,
    text: &'a str,
    parent: Option<usize>,
}
fn document(bytes: Option<&[u8]>) -> Option<Vec<Node<'_>>> {
    let xml = std::str::from_utf8(bytes?).ok()?;
    let mut nodes: Vec<Node<'_>> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    let mut rest = xml;
    while !rest.is_empty() {
        if !rest.starts_with('<') {
            let n = rest.find('<').unwrap_or(rest.len());
            let text = &rest[..n];
            if let Some(&i) = stack.last() {
                if !text.trim().is_empty() && !text.contains('&') {
                    nodes[i].text = text.trim();
                }
            } else if !text.trim().is_empty() {
                return None;
            }
            rest = &rest[n..];
            continue;
        }
        if rest.starts_with("<!--") {
            let end = rest.find("-->")?;
            rest = &rest[end + 3..];
            continue;
        }
        if rest.starts_with("<?") {
            let end = rest.find("?>")?;
            rest = &rest[end + 2..];
            continue;
        }
        if rest.starts_with("<!") {
            return None;
        }
        let end = rest.find('>')?;
        let tag = rest[1..end].trim();
        rest = &rest[end + 1..];
        if let Some(name) = tag.strip_prefix('/') {
            let i = stack.pop()?;
            if name.trim() != nodes[i].name {
                return None;
            }
        } else {
            let empty = tag.ends_with('/');
            let tag = tag.trim_end_matches('/').trim();
            let n = tag.find(char::is_whitespace).unwrap_or(tag.len());
            let name = &tag[..n];
            if name.is_empty() || nodes.len() >= 50000 || stack.len() >= 128 {
                return None;
            }
            if stack.is_empty() && !nodes.is_empty() {
                return None;
            }
            nodes.push(Node {
                name,
                attrs: tag[n..].trim(),
                text: "",
                parent: stack.last().copied(),
            });
            if !empty {
                stack.push(nodes.len() - 1);
            }
        }
    }
    if !stack.is_empty() || nodes.is_empty() {
        None
    } else {
        Some(nodes)
    }
}
fn children<'a>(
    nodes: &'a [Node<'a>],
    parent: usize,
    name: &'a str,
) -> impl Iterator<Item = usize> + 'a {
    nodes
        .iter()
        .enumerate()
        .filter_map(move |(i, n)| (n.parent == Some(parent) && n.name == name).then_some(i))
}
fn child<'a>(nodes: &'a [Node<'a>], parent: usize, name: &'a str) -> Option<usize> {
    children(nodes, parent, name).next()
}
fn value<'a>(nodes: &'a [Node<'a>], parent: usize, name: &'a str) -> Option<&'a str> {
    let v = nodes[child(nodes, parent, name)?].text;
    (!v.is_empty()).then_some(v)
}

pub(crate) fn model_name(xml: &str) -> Option<&str> {
    let nodes = document(Some(xml.as_bytes()))?;
    let node = match nodes[0].name {
        "Device_Info" => child(&nodes, 0, "ModelName")?,
        "device" => child(&nodes, 0, "modelName")?,
        _ => return None,
    };
    let text = nodes[node].text;
    (!text.is_empty() && !text.chars().any(char::is_control)).then_some(text)
}
fn flag(s: &str) -> Option<bool> {
    match s {
        "0" => Some(false),
        "1" => Some(true),
        _ => None,
    }
}
/// An AppCommand0300 `control` code. Measured on the S760H playing Dolby Digital in Dolby Surround: `2` on every control its
/// own setup page shows active (`/ajax/audio/get_config`, display 3, not grey) and `0` on Dialog Control, which that page
/// hides. `1` was never seen; it is not taken as controllable.
fn control(s: &str) -> Option<bool> {
    match s {
        "2" => Some(true),
        "0" | "1" => Some(false),
        _ => None,
    }
}
fn attr<'a>(attrs: &'a str, name: &str) -> Option<&'a str> {
    let mut rest = attrs.trim();
    while !rest.is_empty() {
        let end = rest.find(|c: char| c.is_whitespace() || c == '=')?;
        let key = &rest[..end];
        rest = rest[end..].trim_start();
        rest = rest.strip_prefix('=')?.trim_start();
        let quote = rest.chars().next()?;
        if quote != '\'' && quote != '"' {
            return None;
        }
        rest = &rest[1..];
        let end = rest.find(quote)?;
        let v = &rest[..end];
        rest = rest[end + 1..].trim_start();
        if key == name {
            return Some(v);
        }
    }
    None
}
fn source(name: &str) -> Option<Source> {
    match name {
        "PHONO" => Some(Source::Phono),
        "CD" => Some(Source::Cd),
        "DVD" => Some(Source::Dvd),
        "Blu-ray" => Some(Source::Bluray),
        "TV AUDIO" => Some(Source::Tv),
        "CBL/SAT" => Some(Source::SatCbl),
        "Media Player" => Some(Source::MediaPlayer),
        "GAME" => Some(Source::Game),
        "AUX1" => Some(Source::Aux1),
        "AUX2" => Some(Source::Aux2),
        "NETWORK" => Some(Source::Network),
        "Bluetooth" => Some(Source::Bluetooth),
        "TUNER" | "HD Radio" | "HDRADIO" => Some(Source::Tuner),
        _ => None,
    }
}
fn source_choices(
    nodes: &[Node<'_>],
    zone: usize,
    basic: Option<&[Node<'_>]>,
) -> Option<Vec<SourceChoice>> {
    let input = child(nodes, zone, "InputSource")?;
    if value(nodes, input, "Control").and_then(flag) != Some(true) {
        return None;
    }
    let list = child(nodes, input, "List")?;
    let basic = basic?;
    let commands: Vec<_> = children(basic, 0, "cmd").collect();
    if commands.len() != 4 {
        return None;
    }
    let renames = child(basic, commands[1], "functionrename");
    let deletes = child(basic, commands[2], "functiondelete")?;
    let mut out = Vec::new();
    for item in children(nodes, list, "Source") {
        let Some(raw) = value(nodes, item, "FuncName") else {
            continue;
        };
        let Some(id) = source(raw) else { continue };
        let deleted = children(basic, deletes, "list")
            .find(|&i| value(basic, i, "FuncName") == Some(raw))
            .and_then(|i| value(basic, i, "use"))
            .and_then(flag)?;
        if !deleted {
            continue;
        }
        let rename = renames.and_then(|r| {
            children(basic, r, "list")
                .find(|&i| value(basic, i, "name") == Some(raw))
                .and_then(|i| value(basic, i, "rename"))
        });
        let name = rename
            .or_else(|| value(nodes, item, "DefaultName"))
            .unwrap_or(raw);
        if !out.iter().any(|v: &SourceChoice| v.id == id) {
            out.push(SourceChoice {
                id,
                name: name.to_owned(),
            });
        }
    }
    Some(out)
}

pub(crate) fn parse(
    info: Option<&[u8]>,
    basic: Option<&[u8]>,
    parameters: Option<&[u8]>,
) -> Prerequisites {
    let mut result = Prerequisites::default();
    let info = document(info).filter(|n| n[0].name == "Device_Info");
    let basic = document(basic).filter(|n| n[0].name == "rx");
    let parameters = document(parameters).filter(|n| n[0].name == "rx");
    if let Some(nodes) = &basic {
        let cmds: Vec<_> = children(nodes, 0, "cmd").collect();
        if cmds.len() == 4 {
            result.tone_available = value(nodes, cmds[0], "status").and_then(flag);
            result.tone_enabled = value(nodes, cmds[0], "adjust").and_then(flag);
            if let Some(zone) = child(nodes, cmds[3], "zone2") {
                result.zone2_volume_variable = value(nodes, zone, "state").and_then(|v| match v {
                    "variable" => Some(true),
                    "fixed" => Some(false),
                    _ => None,
                });
                result.zone2_volume_limit = value(nodes, zone, "limit")
                    .and_then(|v| v.parse::<f64>().ok())
                    .map(|v| v + 80.0)
                    .filter(|v| v.is_finite() && (0.0..=98.0).contains(v));
            }
        }
    }
    if let Some(nodes) = &parameters {
        for cmd in children(nodes, 0, "cmd") {
            let name = value(nodes, cmd, "name");
            let Some(list) = child(nodes, cmd, "list") else {
                continue;
            };
            for p in children(nodes, list, "param") {
                let available = attr(nodes[p].attrs, "control").and_then(control);
                let slot = match (name, attr(nodes[p].attrs, "name")) {
                    (Some("GetAudyssey"), Some("dynamiceq")) => &mut result.dynamic_eq_available,
                    (Some("GetAudyssey"), Some("reflevoffset")) => &mut result.reference_level_available,
                    (Some("GetAudyssey"), Some("dynamicvol")) => &mut result.dynamic_volume_available,
                    (Some("GetAudyssey"), Some("multeq")) => &mut result.mult_eq_available,
                    (Some("GetSurroundParameter"), Some("cinemaeq")) => &mut result.cinema_eq_available,
                    (Some("GetSurroundParameter"), Some("loudness")) => &mut result.loudness_available,
                    (Some("GetSurroundParameter"), Some("dialogcontrol")) => {
                        &mut result.dialog_control_available
                    }
                    (Some("GetSurroundParameter"), Some("lfe")) => &mut result.lfe_available,
                    (Some("GetSurroundParameter"), Some("dyncomp")) => &mut result.drc_available,
                    // Any one button the receiver takes means the mode can be changed now.
                    (Some("GetSoundMode"), Some("movie" | "music" | "game" | "pure")) => {
                        &mut result.sound_mode_available
                    }
                    _ => continue,
                };
                *slot = if *slot == Some(true) { Some(true) } else { available.or(*slot) };
            }
        }
    }
    if let Some(nodes) = &info {
        for (zone, node) in nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.name == "DeviceZoneCapabilities")
        {
            let _ = node;
            match child(nodes, zone, "Zone").and_then(|z| value(nodes, z, "No")) {
                Some("0") => result.main_sources = source_choices(nodes, zone, basic.as_deref()),
                Some("1") => {
                    result.zone2_present = Some(true);
                    result.zone2_sources = source_choices(nodes, zone, basic.as_deref());
                }
                _ => {}
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    const INFO: &str = r#"<Device_Info><ModelName>AVR-S760H</ModelName>
      <DeviceZoneCapabilities><Zone><No>0</No></Zone><InputSource><Control>1</Control><List>
        <Source><FuncName>TV AUDIO</FuncName><DefaultName>TV Audio</DefaultName></Source>
        <Source><FuncName>CD</FuncName><DefaultName>CD</DefaultName></Source>
      </List></InputSource></DeviceZoneCapabilities>
      <DeviceZoneCapabilities><Zone><No>1</No></Zone><InputSource><Control>1</Control><List>
        <Source><FuncName>Blu-ray</FuncName><DefaultName>Blu-ray</DefaultName></Source>
      </List></InputSource></DeviceZoneCapabilities></Device_Info>"#;
    const BASIC: &str = r#"<rx><cmd><status>1</status><adjust>0</adjust></cmd>
      <cmd><functionrename><list><name>TV AUDIO</name><rename>Television</rename></list></functionrename></cmd>
      <cmd><functiondelete>
        <list><FuncName>TV AUDIO</FuncName><use>1</use></list>
        <list><FuncName>CD</FuncName><use>0</use></list>
        <list><FuncName>Blu-ray</FuncName><use>1</use></list>
      </functiondelete></cmd>
      <cmd><zone2><volume>-4</volume><state>variable</state><limit>0.0</limit></zone2></cmd></rx>"#;
    const PARAMETERS: &str = r#"<rx><cmd><name>GetAudyssey</name><list>
      <param name="dynamiceq" control="2">1</param><param control="0" name="reflevoffset">0</param>
      <param name="dynamicvol" control="2">2</param><param name="multeq" control="1">3</param>
      </list></cmd><cmd><name>GetSurroundParameter</name><list>
      <param name="cinemaeq" control="2">0</param><param name="dialogcontrol" control="0"></param>
      <param name="lfe" control="2">0</param><param name="dyncomp" control="9">0</param>
      </list></cmd><cmd><name>GetSoundMode</name><list>
      <param name="movie" control="0">0</param><param name="pure" control="2">1</param>
      </list></cmd></rx>"#;

    #[test]
    fn tuner_source_requires_an_explicit_enabled_report() {
        let info = INFO.replace("CD", "TUNER");
        let basic = BASIC.replace("<FuncName>CD</FuncName>", "<FuncName>TUNER</FuncName>");
        let disabled = parse(Some(info.as_bytes()), Some(basic.as_bytes()), None);
        assert!(!disabled.main_sources.unwrap().iter().any(|s| s.id == Source::Tuner));
        let basic = basic.replace("<FuncName>TUNER</FuncName><use>0</use>", "<FuncName>TUNER</FuncName><use>1</use>");
        let enabled = parse(Some(info.as_bytes()), Some(basic.as_bytes()), None);
        assert!(enabled.main_sources.unwrap().iter().any(|s| s.id == Source::Tuner));
    }

    #[test]
    fn reads_real_shaped_controls_and_distinct_zones() {
        let result = parse(
            Some(INFO.as_bytes()),
            Some(BASIC.as_bytes()),
            Some(PARAMETERS.as_bytes()),
        );
        assert_eq!(result.tone_available, Some(true));
        assert_eq!(result.tone_enabled, Some(false));
        assert_eq!(result.dynamic_eq_available, Some(true));
        assert_eq!(result.reference_level_available, Some(false));
        assert_eq!(result.dynamic_volume_available, Some(true));
        assert_eq!(result.mult_eq_available, Some(false));
        assert_eq!(result.cinema_eq_available, Some(true));
        assert_eq!(result.dialog_control_available, Some(false));
        assert_eq!(result.lfe_available, Some(true));
        assert_eq!(result.drc_available, None, "an unknown code is not proof either way");
        assert_eq!(result.loudness_available, None);
        assert_eq!(result.sound_mode_available, Some(true));
        assert_eq!(result.zone2_present, Some(true));
        assert_eq!(result.zone2_volume_variable, Some(true));
        assert_eq!(result.zone2_volume_limit, Some(80.0));
        assert_eq!(
            result.main_sources,
            Some(vec![SourceChoice {
                id: Source::Tv,
                name: "Television".into()
            }])
        );
        assert_eq!(
            result.zone2_sources,
            Some(vec![SourceChoice {
                id: Source::Bluray,
                name: "Blu-ray".into()
            }])
        );
    }

    #[test]
    fn partial_and_malformed_responses_do_not_invent_capabilities() {
        let partial = parse(Some(INFO.as_bytes()), None, Some(PARAMETERS.as_bytes()));
        assert_eq!(partial.zone2_present, Some(true));
        assert_eq!(partial.main_sources, None);
        assert_eq!(partial.zone2_sources, None);
        assert_eq!(partial.tone_available, None);
        assert_eq!(partial.dynamic_eq_available, Some(true));
        let broken = parse(
            Some(b"<Device_Info>"),
            Some(BASIC.as_bytes()),
            Some(b"<rx><cmd>"),
        );
        assert_eq!(broken.zone2_present, None);
        assert_eq!(broken.tone_available, Some(true));
        assert_eq!(broken.dynamic_eq_available, None);
        let no_hide = BASIC.replace("<use>1</use>", "<use></use>");
        assert_eq!(
            parse(Some(INFO.as_bytes()), Some(no_hide.as_bytes()), None).main_sources,
            None
        );
    }

    #[test]
    fn volume_state_is_explicit_and_limits_are_relative_db() {
        let fixed = BASIC
            .replace("<state>variable</state>", "<state>fixed</state>")
            .replace("<limit>0.0</limit>", "<limit>-10.5</limit>");
        let result = parse(None, Some(fixed.as_bytes()), None);
        assert_eq!(result.zone2_volume_variable, Some(false));
        assert_eq!(result.zone2_volume_limit, Some(69.5));
        let missing = BASIC
            .replace("<state>variable</state>", "")
            .replace("<limit>0.0</limit>", "<limit>OFF</limit>");
        let result = parse(None, Some(missing.as_bytes()), None);
        assert_eq!(result.zone2_volume_variable, None);
        assert_eq!(result.zone2_volume_limit, None);
    }

    #[test]
    fn model_identity_requires_the_right_element_and_structure() {
        assert_eq!(model_name(INFO), Some("AVR-S760H"));
        assert_eq!(
            model_name("<device><modelName>AVR-S760H</modelName></device>"),
            Some("AVR-S760H")
        );
        assert_eq!(
            model_name("<Device_Info><modelName>AVR-S760H</modelName></Device_Info>"),
            None
        );
        assert_eq!(
            model_name("<Device_Info><!-- <ModelName>AVR-S760H</ModelName> --></Device_Info>"),
            None
        );
        assert_eq!(
            model_name("<Device_Info><ModelName>AVR-S760H</Device_Info>"),
            None
        );
    }
}
