//! LG's interface files (which `ssap://` URIs exist, what each needs and where it goes) and permission types (what an unsigned
//! manifest is granted), read from the gateway's own directory and resolved as `interface-manager.js` resolves them.

use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::path::Path;

pub struct Method {
    pub required: Vec<String>,
    pub send_client_info: bool,
    /// Response properties that name files (`resources.media.responseProperties`).
    pub media: Vec<String>,
    /// Response properties that name sockets (`resources.socket.responseProperties`).
    pub socket: Vec<String>,
    provider: Option<(String, String)>,
}

struct Iface {
    is_virtual: bool,
    version: Value,
    methods: HashMap<String, Method>,
}

#[derive(Default)]
pub struct Catalog {
    ifaces: HashMap<String, Iface>,
    order: Vec<String>,
    perms: HashMap<String, String>,
}

pub struct Resolved<'a> {
    pub service: &'a str,
    pub path: &'a str,
    pub method: &'a Method,
}

/// `^[^:]+://([^/]+)(/.*)$`, where `.` does not match a line terminator.
pub fn parse_uri(uri: &str) -> Option<(&str, &str)> {
    let (scheme, rest) = uri.split_once("://")?;
    if scheme.is_empty() || scheme.contains(':') {
        return None;
    }
    let slash = rest.find('/')?;
    let (service, path) = rest.split_at(slash);
    if service.is_empty() || path.contains(['\n', '\r', '\u{2028}', '\u{2029}']) {
        return None;
    }
    Some((service, path))
}

fn strings(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect(),
        _ => Vec::new(),
    }
}

fn parse_iface(json: &Value) -> Result<(String, Iface), String> {
    let (name, is_virtual) = if let Some(v) = json.get("virtualService").filter(|v| crate::util::truthy(Some(v))) {
        (v.as_str().ok_or("virtualService is not a string")?.to_owned(), true)
    } else if let Some(s) = json.get("service").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        (s.to_owned(), false)
    } else {
        return Err("must define 'service' or 'virtualService'".into());
    };
    let mut methods = HashMap::new();
    for m in json.get("methods").and_then(Value::as_array).into_iter().flatten() {
        let path = m.get("path").and_then(Value::as_str).ok_or("method without a path")?;
        let provider = match m.get("provider").and_then(Value::as_str) {
            Some(p) if is_virtual => {
                let slash = p.find('/').ok_or("invalid provider: expect service/path")?;
                if slash == 0 {
                    return Err("invalid provider: expect service/path".into());
                }
                Some((p[..slash].to_owned(), p[slash..].to_owned()))
            }
            None if is_virtual => return Err(format!("virtual method {path} missing provider")),
            Some(_) => return Err("can't specify provider for concrete method".into()),
            None => None,
        };
        let res = m.get("resources");
        let method = Method {
            required: strings(m.get("requiredPermissions")),
            send_client_info: crate::util::truthy(m.get("sendClientInfo")),
            media: strings(res.and_then(|r| r.get("media")).and_then(|r| r.get("responseProperties"))),
            socket: strings(res.and_then(|r| r.get("socket")).and_then(|r| r.get("responseProperties"))),
            provider,
        };
        methods.insert(path.to_owned(), method);
    }
    Ok((name, Iface { is_virtual, version: json.get("version").cloned().unwrap_or(Value::Null), methods }))
}

impl Catalog {
    /// Reads `<gateway>/interfaces/*.interface` and `<gateway>/permission-types/*.permissions` in name order (the order the
    /// set's read-only filesystem lists them in, and so the order LG loads them in). A file that does not parse is skipped, as
    /// LG skips it; the pinned hashes keep the files to the ones this gateway was checked against.
    pub fn load(gateway: &Path) -> std::io::Result<Catalog> {
        let mut c = Catalog::default();
        for (path, text) in read_dir_sorted(&gateway.join("permission-types"), ".permissions")? {
            match serde_json::from_str::<Value>(&text) {
                Ok(v) => c.add_permissions(&v),
                Err(e) => crate::warn!("skipping {}: {e}", path),
            }
        }
        for (path, text) in read_dir_sorted(&gateway.join("interfaces"), ".interface")? {
            let parsed = serde_json::from_str::<Value>(&text).map_err(|e| e.to_string()).and_then(|v| parse_iface(&v));
            match parsed {
                Ok((name, iface)) => c.add(name, iface),
                Err(e) => crate::warn!("skipping {}: {e}", path),
            }
        }
        Ok(c)
    }

    /// A catalog from in-memory interface and permission-type documents.
    #[cfg(test)]
    pub fn from_json(ifaces: &[Value], perms: &[Value]) -> Catalog {
        let mut c = Catalog::default();
        for p in perms {
            c.add_permissions(p);
        }
        for i in ifaces {
            if let Ok((name, iface)) = parse_iface(i) {
                c.add(name, iface);
            }
        }
        c
    }

    fn add(&mut self, name: String, iface: Iface) {
        if self.ifaces.insert(name.clone(), iface).is_none() {
            self.order.push(name);
        }
    }

    fn add_permissions(&mut self, doc: &Value) {
        for t in doc.get("permissionTypes").and_then(Value::as_array).into_iter().flatten() {
            if let Some(id) = t.get("id").and_then(Value::as_str) {
                let access = t.get("accessType").and_then(Value::as_str).unwrap_or_default();
                self.perms.insert(id.to_owned(), access.to_owned());
            }
        }
    }

    /// The concrete method a service and path name, following one virtual redirect at most.
    pub fn find(&self, service: &str, path: &str) -> Option<Resolved<'_>> {
        let (mut service, mut path) = (service, path);
        let mut redirects = 0;
        loop {
            let (name, iface) = self.ifaces.get_key_value(service)?;
            let (key, method) = iface.methods.get_key_value(path)?;
            if !iface.is_virtual {
                return Some(Resolved { service: name, path: key, method });
            }
            redirects += 1;
            if redirects >= 2 {
                return None;
            }
            let (s, p) = method.provider.as_ref()?;
            service = s;
            path = p;
        }
    }

    /// The permissions an unsigned manifest's list is granted: each known, non-SECURE type, once, in the order asked.
    pub fn grant(&self, requested: Option<&Value>) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for id in requested.and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
            if let Some(access) = self.perms.get(id)
                && access != "SECURE"
                && !out.iter().any(|g| g == id)
            {
                out.push(id.to_owned());
            }
        }
        out
    }

    /// `getServiceList`: the virtual services in load order.
    pub fn service_list(&self) -> Value {
        let services: Vec<Value> = self
            .order
            .iter()
            .filter_map(|name| {
                let iface = &self.ifaces[name];
                iface.is_virtual.then(|| {
                    let version = if crate::util::truthy(Some(&iface.version)) { iface.version.clone() } else { json!(1) };
                    let mut m = Map::new();
                    m.insert("name".into(), Value::String(name.clone()));
                    m.insert("version".into(), version);
                    Value::Object(m)
                })
            })
            .collect();
        json!({ "services": services })
    }
}

fn read_dir_sorted(dir: &Path, suffix: &str) -> std::io::Result<Vec<(String, String)>> {
    let mut names: Vec<String> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.ends_with(suffix))
        .collect();
    names.sort();
    let mut out = Vec::new();
    for n in names {
        let path = dir.join(&n);
        out.push((path.display().to_string(), std::fs::read_to_string(&path)?));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Catalog {
        Catalog::from_json(
            &[
                json!({"virtualService": "audio", "version": 1, "methods": [
                    {"path": "/getVolume", "provider": "com.webos.service.audio/master/getVolume"},
                    {"path": "/loop", "provider": "audio/getVolume"}]}),
                json!({"service": "com.webos.service.audio", "methods": [
                    {"path": "/master/getVolume", "requiredPermissions": "READ_AUDIO"}]}),
                json!({"virtualService": "broken", "methods": [{"path": "/x"}]}),
            ],
            &[json!({"permissionTypes": [
                {"id": "READ_AUDIO", "accessType": "PROTECTED"},
                {"id": "SECRET", "accessType": "SECURE"},
                {"id": "LAUNCH", "accessType": "OPEN"}]})],
        )
    }

    #[test]
    fn uris_resolve_through_one_virtual_hop() {
        let c = catalog();
        let r = c.find("audio", "/getVolume").unwrap();
        assert_eq!((r.service, r.path, r.method.required.as_slice()), ("com.webos.service.audio", "/master/getVolume", &["READ_AUDIO".to_owned()][..]));
        assert!(c.find("audio", "/loop").is_none(), "a second virtual hop is not followed");
        assert!(c.find("broken", "/x").is_none(), "an interface with a bad method is skipped whole");
        assert_eq!(parse_uri("ssap://audio/getVolume"), Some(("audio", "/getVolume")));
        assert_eq!(parse_uri("ssap://com.webos.service.audio/master/getVolume"), Some(("com.webos.service.audio", "/master/getVolume")));
        assert_eq!(parse_uri("ssap://audio"), None);
        assert_eq!(parse_uri("audio/getVolume"), None);
    }

    #[test]
    fn grant_keeps_known_non_secure_types_in_request_order() {
        let c = catalog();
        let asked = json!(["LAUNCH", "SECRET", "UNKNOWN", "READ_AUDIO", "LAUNCH"]);
        assert_eq!(c.grant(Some(&asked)), ["LAUNCH", "READ_AUDIO"]);
        assert!(c.grant(Some(&json!("LAUNCH"))).is_empty());
        assert_eq!(c.service_list(), json!({"services": [{"name": "audio", "version": 1}]}));
    }
}
