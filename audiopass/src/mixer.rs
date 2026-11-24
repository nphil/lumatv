//! LG's own mixer route for a player's stream. A player's PCM sink books a PCM mix channel with uMS (`PCMMC<n>`), and LG's audio
//! daemon connects that channel's mixer input (`AMIXER<n>`) to the main output for the stream, with the eARC words mixed: on the
//! C4 at the Load, before the player can arm. The route takes that input over only when LG's own records show it carries the arming
//! player's stream and nothing else plays sound: audiooutputd lists that input alone, the kernel wires no ADEC and no mixer input
//! beyond [`LG_MIXERS`](crate::sndout::LG_MIXERS) and it, SAM names the player's process as an app, uMS has that app's pipeline
//! holding the channel, and no other app holds a sound resource.

use crate::health;
use crate::host::Host;
use crate::sndout::Sndout;
use serde_json::Value;

/// SAM's running apps: `{running: [{id, processid, ...}]}`, the numbers as strings.
pub const RUNNING: &str = "luna://com.webos.service.applicationmanager/running";
/// uMS's pipelines: a list of `{id, type, currentAppId, resource: [{index, resource}]}`, or an object saying there are none.
pub const PIPELINES: &str = "luna://com.webos.media/getActivePipelines";
/// The PCM mix channels uMS hands out (`PCMMC`, qty 5 in `umediaserver_resource_config.txt`).
pub const PCM_CHANNELS: u8 = 5;
/// The resources of `umediaserver_resource_config.txt` that decode, carry or mix sound. Another app holding one plays sound.
const SOUND: [&str; 9] = [
    "ADEC",
    "ADEC_BANDWIDTH",
    "CLONED_ADEC",
    "PCMMC",
    "MAIN_SOUND",
    "MIXING_SOUND",
    "AHDMIRX",
    "AADC",
    "SNDSCART",
];

/// LG's mixer input for a player's stream, as the session took it over: `AMIXER<port>`, for uMS pipeline `pipeline`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Taken {
    pub port: u8,
    pub pipeline: String,
}

impl Taken {
    /// `<port> <pipeline>`, as the journal keeps it.
    pub fn parse(text: &str) -> Option<Self> {
        let (port, pipeline) = text.split_once(' ')?;
        let port = port.parse().ok().filter(|port| *port < PCM_CHANNELS)?;
        (!pipeline.is_empty() && !pipeline.contains(char::is_whitespace)).then(|| Self {
            port,
            pipeline: pipeline.to_owned(),
        })
    }

    /// The name audiooutputd knows the input by.
    pub fn source(&self) -> String {
        format!("AMIXER{}", self.port)
    }
}

struct Pipeline {
    id: String,
    kind: String,
    app: Option<String>,
    resources: Vec<(String, Option<u64>)>,
}

impl Pipeline {
    fn holds_channel(&self, port: u8) -> bool {
        self.resources
            .iter()
            .any(|(name, index)| name == "PCMMC" && *index == Some(u64::from(port)))
    }

    fn sound(&self) -> Option<String> {
        self.resources
            .iter()
            .find(|(name, _)| SOUND.contains(&name.as_str()))
            .map(|(name, index)| match index {
                Some(index) => format!("{name}{index}"),
                None => name.clone(),
            })
    }
}

/// `None` when the reply is not uMS's list.
fn pipelines(reply: &str) -> Option<Vec<Pipeline>> {
    let value: Value = serde_json::from_str(reply).ok()?;
    let list = match &value {
        Value::Array(list) => list,
        // `{"returnValue":true,"data":"pipeline empty",...}` when uMS has none.
        Value::Object(fields) if fields.get("returnValue") == Some(&Value::Bool(true)) => {
            return Some(Vec::new());
        }
        _ => return None,
    };
    let text = |entry: &Value, key: &str| entry.get(key).and_then(Value::as_str).map(str::to_owned);
    list.iter()
        .map(|entry| {
            let resources = entry
                .get("resource")
                .and_then(Value::as_array)
                .map(|held| {
                    held.iter()
                        .filter_map(|unit| {
                            Some((
                                text(unit, "resource")?,
                                unit.get("index").and_then(Value::as_u64),
                            ))
                        })
                        .collect()
                })
                .unwrap_or_default();
            Some(Pipeline {
                id: text(entry, "id")?,
                kind: text(entry, "type").unwrap_or_default(),
                app: text(entry, "currentAppId").filter(|app| !app.is_empty()),
                resources,
            })
        })
        .collect()
}

/// The app SAM started as process `pid`.
fn app_of(host: &dyn Host, pid: u32) -> Result<String, String> {
    let reply = host.luna(RUNNING, "{}").ok_or("SAM does not answer")?;
    let value: Value =
        serde_json::from_str(&reply).map_err(|_| "SAM's running list cannot be read")?;
    let pid = pid.to_string();
    value
        .get("running")
        .and_then(Value::as_array)
        .ok_or("SAM's running list cannot be read")?
        .iter()
        .find(|app| app.get("processid").and_then(Value::as_str) == Some(pid.as_str()))
        .and_then(|app| app.get("id")?.as_str())
        .map(str::to_owned)
        .ok_or_else(|| format!("SAM lists no app as process {pid}"))
}

/// Proves that the one stream mixer input the kernel lists in `snd` is LG's route for `pid`'s own stream while nothing else plays
/// sound, and names the uMS pipeline it is for. `ours` is the session's ADEC while the session has it connected: the only other
/// input allowed. Says why not.
pub fn prove(host: &dyn Host, pid: u32, snd: &Sndout, ours: Option<u8>) -> Result<Taken, String> {
    let port = match snd.stream_mixers().as_slice() {
        [one] => one
            .parse()
            .ok()
            .filter(|port| *port < PCM_CHANNELS)
            .ok_or_else(|| format!("AMIXER{one} is not a PCM mix channel's input"))?,
        [] => return Err("the kernel lists no stream mixer input".into()),
        many => return Err(format!("the kernel lists AMIXER{}", many.join(", AMIXER"))),
    };
    let ours = ours.map(|adec| adec.to_string());
    if let Some(adec) = snd
        .connected_adecs()
        .into_iter()
        .find(|adec| Some(*adec) != ours.as_deref())
    {
        return Err(format!("the kernel lists ADEC{adec}"));
    }
    let source = format!("AMIXER{port}");
    let ours = ours.map(|adec| format!("ADEC{adec}"));
    let list = health::connections(host).ok_or("the audio service's connections cannot be read")?;
    if !list.contains(&source)
        || list
            .iter()
            .any(|s| *s != source && Some(s) != ours.as_ref())
    {
        return Err(format!(
            "audiooutputd lists {}, not {source} alone",
            list.join(", ")
        ));
    }
    let app = app_of(host, pid)?;
    let pipelines = host
        .luna(PIPELINES, "{}")
        .as_deref()
        .and_then(pipelines)
        .ok_or("uMS's pipelines cannot be read")?;
    for pipeline in pipelines
        .iter()
        .filter(|p| p.app.as_deref() != Some(app.as_str()))
    {
        if let Some(sound) = pipeline.sound() {
            let holder = pipeline.app.as_deref().unwrap_or(&pipeline.kind);
            return Err(format!("{holder} holds {sound}"));
        }
    }
    let holders: Vec<&Pipeline> = pipelines.iter().filter(|p| p.holds_channel(port)).collect();
    match holders.as_slice() {
        [pipeline] => Ok(Taken {
            port,
            pipeline: pipeline.id.clone(),
        }),
        [] => Err(format!("{app} holds no PCMMC{port}")),
        _ => Err(format!(
            "{app} holds PCMMC{port} in {} pipelines",
            holders.len()
        )),
    }
}

/// Whether uMS still has the pipeline holding the channel the input was taken for: the stream plays on. `None` when uMS does not
/// answer.
pub fn held(host: &dyn Host, taken: &Taken) -> Option<bool> {
    let pipelines = pipelines(&host.luna(PIPELINES, "{}")?)?;
    Some(
        pipelines
            .iter()
            .any(|p| p.id == taken.pipeline && p.holds_channel(taken.port)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ums_lists_parse_in_both_shapes() {
        let list = pipelines(
            r#"[{"id":"_L0UqiY5wlJr66R","type":"vtclient","resource":[]},
                {"id":"_f4r","type":"media","currentAppId":"com.beb.plxnative",
                 "resource":[{"index":0,"resource":"VDEC"},{"index":4,"resource":"PCMMC"}]}]"#,
        )
        .unwrap();
        assert_eq!(list.len(), 2);
        assert!(list[1].holds_channel(4) && !list[1].holds_channel(3));
        assert_eq!(list[1].sound().as_deref(), Some("PCMMC4"));
        assert_eq!((list[0].app.as_deref(), list[0].sound()), (None, None));
        let none =
            r#"{"errorCode":0,"returnValue":true,"errorText":"No Error","data":"pipeline empty"}"#;
        assert_eq!(pipelines(none).map(|list| list.len()), Some(0));
        assert!(pipelines(r#"{"returnValue":false}"#).is_none());
        assert!(pipelines("not json").is_none());
    }

    #[test]
    fn a_journalled_input_parses_only_as_a_channel_and_one_pipeline() {
        assert_eq!(
            Taken::parse("4 _f4rXIG4rerDUwJ"),
            Some(Taken {
                port: 4,
                pipeline: "_f4rXIG4rerDUwJ".into()
            })
        );
        for bad in ["4", "5 _f4r", "x _f4r", "4 ", "4 _f4r more"] {
            assert_eq!(Taken::parse(bad), None, "{bad}");
        }
    }
}
