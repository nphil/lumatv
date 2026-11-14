//! The hub's requests and the agent's replies (`internal/common/common-ws.go` upstream), and the line
//! between a hub this agent understands and one that has moved past it.

use crate::auth::Keys;
use crate::cbor::{self, Dec, Enc};

pub const GET_DATA: u8 = 0;
pub const CHECK_FINGERPRINT: u8 = 1;
pub const GET_CONTAINER_LOGS: u8 = 2;
pub const GET_CONTAINER_INFO: u8 = 3;
pub const GET_SMART_DATA: u8 = 4;
pub const GET_ZFS_DATA: u8 = 6;
pub const SYNC_NETWORK_MONITORS: u8 = 7;
pub const GET_PACKAGE_UPDATES: u8 = 8;
pub const GET_SYSTEMD_LOGS: u8 = 9;

// AgentResponse fields.
const SYSTEM_DATA: u64 = 1;
const FINGERPRINT: u64 = 2;
const ERROR: u64 = 3;
const STRING: u64 = 4;
const SMART_DATA: u64 = 5;
const DATA: u64 = 7;

/// `HubRequest[cbor.RawMessage]`: the payload stays encoded until the action is known.
pub struct Request<'a> {
    pub action: u8,
    pub data: Option<&'a [u8]>,
    pub id: Option<u32>,
}

impl<'a> Request<'a> {
    pub fn decode(msg: &'a [u8]) -> cbor::Result<Request<'a>> {
        let mut dec = Dec::new(msg);
        let mut req = Request { action: 0, data: None, id: None };
        dec.map(|d, key| {
            match key {
                Some(0) if !d.null()? => req.action = narrow(d.uint()?)?,
                Some(1) => req.data = Some(d.item()?),
                Some(2) if !d.null()? => req.id = Some(narrow(d.uint()?)?),
                // A null action or id: the guard above consumed it.
                Some(0 | 2) => {}
                _ => d.skip()?,
            }
            Ok(())
        })?;
        if !dec.finished() {
            return Err(cbor::Error("data after the request"));
        }
        Ok(req)
    }
}

fn narrow<T: TryFrom<u64>>(value: u64) -> cbor::Result<T> {
    T::try_from(value).map_err(|_| cbor::Error("integer out of range"))
}

/// Reads a payload map; an absent or null payload leaves every field at its zero value, as in Go.
fn decode_fields(data: Option<&[u8]>, field: impl FnMut(&mut Dec, Option<u64>) -> cbor::Result<()>) -> cbor::Result<()> {
    let Some(data) = data else { return Ok(()) };
    let mut dec = Dec::new(data);
    if dec.null()? {
        return Ok(());
    }
    dec.map(field)
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct DataOptions {
    pub cache_time_ms: u16,
    pub include_details: bool,
}

impl DataOptions {
    pub fn decode(data: Option<&[u8]>) -> cbor::Result<DataOptions> {
        let mut out = DataOptions::default();
        decode_fields(data, |d, key| {
            match key {
                Some(0) if !d.null()? => out.cache_time_ms = narrow(d.uint()?)?,
                Some(1) if !d.null()? => out.include_details = d.bool()?,
                Some(0 | 1) => {}
                _ => d.skip()?,
            }
            Ok(())
        })?;
        Ok(out)
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct FingerprintRequest {
    pub signature: Vec<u8>,
    pub need_sys_info: bool,
}

impl FingerprintRequest {
    pub fn decode(data: Option<&[u8]>) -> cbor::Result<FingerprintRequest> {
        let mut out = FingerprintRequest::default();
        decode_fields(data, |d, key| {
            match key {
                Some(0) if !d.null()? => out.signature = d.bytes()?,
                Some(1) if !d.null()? => out.need_sys_info = d.bool()?,
                Some(0 | 1) => {}
                _ => d.skip()?,
            }
            Ok(())
        })?;
        Ok(out)
    }
}

/// The parts of a network monitor sync this agent acts on. A replace carries a zero `Config` beside its list (the hub's Go struct is
/// always encoded), which the Go agent ignores, so only the action and the list's length matter.
#[derive(Debug, Default, PartialEq)]
struct SyncRequest {
    action: u8,
    configs: u64,
}

const SYNC_REPLACE: u8 = 0;
const SYNC_DELETE: u8 = 2;

impl SyncRequest {
    fn decode(data: Option<&[u8]>) -> cbor::Result<SyncRequest> {
        let mut out = SyncRequest::default();
        decode_fields(data, |d, key| {
            match key {
                Some(0) if !d.null()? => out.action = narrow(d.uint()?)?,
                Some(1) => d.skip()?,
                Some(2) if !d.null()? => out.configs = d.array_len()?,
                Some(0..=2) => {}
                _ => d.skip()?,
            }
            Ok(())
        })?;
        Ok(out)
    }
}

/// What the agent tells the hub about itself once the hub has proved its key.
pub struct Identity {
    pub keys: Keys,
    pub token: String,
    pub fingerprint: String,
    pub hostname: String,
    /// The port the Go agent would listen on, reported when the hub creates the system itself.
    pub port: String,
    pub name: String,
}

pub enum Step {
    Reply(Vec<u8>),
    /// The request failed; Go logs it as `Error handling message`. A verified hub that gave an id is
    /// also told, so it does not wait for its timeout.
    Failed { err: String, reply: Option<Vec<u8>> },
    /// The hub speaks a protocol newer than this agent.
    NewerHub(String),
}

/// One connection's state: whether the hub has signed the token yet.
pub struct Session<'a> {
    pub identity: &'a Identity,
    pub verified: bool,
}

impl Session<'_> {
    pub fn handle(&mut self, msg: &[u8], gather: &mut dyn FnMut(DataOptions) -> Vec<u8>) -> Step {
        let req = match Request::decode(msg) {
            Ok(req) => req,
            Err(err) => return Step::NewerHub(format!("hub request does not decode: {err}")),
        };
        let fail = |err: String| {
            let reply = req.id.filter(|_| self.verified).map(|id| error_reply(id, &err));
            Step::Failed { err, reply }
        };
        if req.action > GET_SYSTEMD_LOGS {
            return fail(format!("unknown action: {}", req.action));
        }
        if req.action != CHECK_FINGERPRINT && !self.verified {
            return fail("hub not verified".into());
        }
        let (key, value) = match req.action {
            GET_DATA => match DataOptions::decode(req.data) {
                Ok(options) => (SYSTEM_DATA, gather(options)),
                Err(err) => return Step::NewerHub(format!("data request does not decode: {err}")),
            },
            CHECK_FINGERPRINT => {
                let challenge = match FingerprintRequest::decode(req.data) {
                    Ok(challenge) => challenge,
                    Err(err) => return Step::NewerHub(format!("fingerprint request does not decode: {err}")),
                };
                if !self.identity.keys.verify(&self.identity.token, &challenge.signature) {
                    return fail("invalid signature - check KEY value".into());
                }
                self.verified = true;
                (FINGERPRINT, self.fingerprint(challenge.need_sys_info))
            }
            GET_CONTAINER_LOGS | GET_CONTAINER_INFO => (STRING, vec![0x60]),
            // No SMART devices: an empty map. No storage pools: null. No package manager: an empty list.
            GET_SMART_DATA => (SMART_DATA, vec![0xa0]),
            GET_ZFS_DATA => (DATA, vec![0xf6]),
            GET_PACKAGE_UPDATES => (DATA, vec![0xa1, 0x03, 0xf6]),
            SYNC_NETWORK_MONITORS => match SyncRequest::decode(req.data) {
                Ok(sync) if sync.configs == 0 && matches!(sync.action, SYNC_REPLACE | SYNC_DELETE) => (DATA, vec![0xa0]),
                Ok(_) => return fail("network monitors are not supported by this agent".into()),
                Err(err) => return fail(err.to_string()),
            },
            // Systemd services and logs (5 and 9): the TV has no systemd.
            _ => return fail("unsupported operation".into()),
        };
        Step::Reply(reply(req.id, key, &value))
    }

    fn fingerprint(&self, need_sys_info: bool) -> Vec<u8> {
        let id = self.identity;
        let mut e = Enc::new();
        let mut m = e.begin_map();
        e.key(&mut m, 0);
        e.text(&id.fingerprint);
        if need_sys_info {
            for (key, value) in [(1, &id.hostname), (2, &id.port), (3, &id.name)] {
                if !value.is_empty() {
                    e.key(&mut m, key);
                    e.text(value);
                }
            }
        }
        e.end_map(m);
        e.buf
    }
}

/// Go's `newAgentResponse`: `{0: id, field: value}`, or the bare value for a hub that sent no id.
fn reply(id: Option<u32>, field: u64, value: &[u8]) -> Vec<u8> {
    let Some(id) = id else { return value.to_vec() };
    let mut e = Enc::new();
    e.map(2);
    e.uint(0);
    e.uint(id.into());
    e.uint(field);
    e.raw(value);
    e.buf
}

fn error_reply(id: u32, err: &str) -> Vec<u8> {
    let mut e = Enc::new();
    e.map(2);
    e.uint(0);
    e.uint(id.into());
    e.uint(ERROR);
    e.text(err);
    e.buf
}

/// Whether a refused upgrade turned the agent away for its version: the hub answers a version it
/// will not take with a 4xx and a short plain-text reason that says so ("Invalid agent version").
/// A wrong token or a proxy's error page is not that.
pub fn refused_for_version(status: u16, body: &str) -> bool {
    (400..500).contains(&status)
        && body.len() <= 512
        && !body.contains('<')
        && body.to_ascii_lowercase().contains("version")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::tests::hub_key;

    fn request(action: u8, data: Option<&[u8]>, id: Option<u32>) -> Vec<u8> {
        let mut e = Enc::new();
        let mut m = e.begin_map();
        e.key(&mut m, 0);
        e.uint(action.into());
        if let Some(data) = data {
            e.key(&mut m, 1);
            e.raw(data);
        }
        if let Some(id) = id {
            e.key(&mut m, 2);
            e.uint(id.into());
        }
        e.end_map(m);
        e.buf
    }

    fn data_options(cache_time_ms: u16, include_details: bool) -> Vec<u8> {
        let mut e = Enc::new();
        e.map(2);
        e.uint(0);
        e.uint(cache_time_ms.into());
        e.uint(1);
        e.bool(include_details);
        e.buf
    }

    fn challenge(signature: &[u8], need_sys_info: bool) -> Vec<u8> {
        let mut e = Enc::new();
        e.map(2);
        e.uint(0);
        e.bytes(signature);
        e.uint(1);
        e.bool(need_sys_info);
        e.buf
    }

    struct Fixture {
        identity: Identity,
        pair: ring::signature::Ed25519KeyPair,
    }

    fn fixture() -> Fixture {
        let (pair, line) = hub_key();
        let identity = Identity {
            keys: Keys::parse(&line).unwrap(),
            token: "tok".into(),
            fingerprint: "ab12".into(),
            hostname: "lgtv".into(),
            port: "45876".into(),
            name: String::new(),
        };
        Fixture { identity, pair }
    }

    fn reply_of(step: Step) -> Vec<u8> {
        match step {
            Step::Reply(reply) => reply,
            Step::Failed { err, .. } => panic!("failed: {err}"),
            Step::NewerHub(err) => panic!("newer hub: {err}"),
        }
    }

    /// Decodes `{0: id, field: value}` into the id, the field and the value's bytes.
    fn envelope(reply: &[u8]) -> (u32, u64, Vec<u8>) {
        let mut d = Dec::new(reply);
        let (mut id, mut field, mut value) = (0, 0, Vec::new());
        d.map(|d, key| {
            match key.unwrap() {
                0 => id = d.uint()? as u32,
                key => {
                    field = key;
                    value = d.item()?.to_vec();
                }
            }
            Ok(())
        })
        .unwrap();
        (id, field, value)
    }

    #[test]
    fn requests_round_trip() {
        let opts = data_options(1000, true);
        let msg = request(GET_DATA, Some(&opts), Some(7));
        let req = Request::decode(&msg).unwrap();
        assert_eq!((req.action, req.id), (GET_DATA, Some(7)));
        assert_eq!(DataOptions::decode(req.data).unwrap(), DataOptions { cache_time_ms: 1000, include_details: true });
        assert_eq!(DataOptions::decode(None).unwrap(), DataOptions::default());

        let msg = request(CHECK_FINGERPRINT, Some(&challenge(b"sig", true)), None);
        let req = Request::decode(&msg).unwrap();
        let expected = FingerprintRequest { signature: b"sig".to_vec(), need_sys_info: true };
        assert_eq!(FingerprintRequest::decode(req.data).unwrap(), expected);
    }

    #[test]
    fn answers_the_hub_after_it_proves_its_key() {
        let f = fixture();
        let mut session = Session { identity: &f.identity, verified: false };
        let mut gathered = Vec::new();
        let mut gather = |options: DataOptions| {
            gathered.push(options);
            vec![0xa0]
        };

        let early = request(GET_DATA, Some(&data_options(60000, true)), Some(1));
        assert!(matches!(session.handle(&early, &mut gather), Step::Failed { reply: None, .. }));

        let forged = request(CHECK_FINGERPRINT, Some(&challenge(&[0; 64], false)), Some(2));
        assert!(matches!(session.handle(&forged, &mut gather), Step::Failed { reply: None, .. }));
        assert!(!session.verified);

        let signature = f.pair.sign(b"tok");
        let signed = request(CHECK_FINGERPRINT, Some(&challenge(signature.as_ref(), true)), Some(3));
        let (id, field, value) = envelope(&reply_of(session.handle(&signed, &mut gather)));
        assert!(session.verified);
        assert_eq!((id, field), (3, FINGERPRINT));
        let mut response = Vec::new();
        Dec::new(&value)
            .map(|d, key| {
                response.push((key.unwrap(), d.text()?));
                Ok(())
            })
            .unwrap();
        assert_eq!(response, [(0, "ab12".to_owned()), (1, "lgtv".to_owned()), (2, "45876".to_owned())]);

        let (id, field, value) = envelope(&reply_of(session.handle(&early, &mut gather)));
        assert_eq!((id, field, value), (1, SYSTEM_DATA, vec![0xa0]));

        // A hub that sends no id gets the bare value.
        let legacy = request(GET_DATA, None, None);
        assert_eq!(reply_of(session.handle(&legacy, &mut gather)), [0xa0]);
        assert_eq!(gathered, [DataOptions { cache_time_ms: 60000, include_details: true }, DataOptions::default()]);
    }

    #[test]
    fn features_the_tv_lacks_get_the_go_agents_empty_answers() {
        let f = fixture();
        let mut session = Session { identity: &f.identity, verified: true };
        let mut gather = |_: DataOptions| unreachable!();
        let mut answer = |action, data: Option<&[u8]>| envelope(&reply_of(session.handle(&request(action, data, Some(9)), &mut gather)));
        assert_eq!(answer(GET_SMART_DATA, None), (9, SMART_DATA, vec![0xa0]));
        assert_eq!(answer(GET_CONTAINER_LOGS, None), (9, STRING, vec![0x60]));
        assert_eq!(answer(GET_ZFS_DATA, None), (9, DATA, vec![0xf6]));
        assert_eq!(answer(GET_PACKAGE_UPDATES, None), (9, DATA, vec![0xa1, 0x03, 0xf6]));
        // The hub clears monitors on every connect with a replace that carries an empty list and a zero `Config`
        // ({0: "", 1: "", 2: "", 4: 0}).
        let replace = [0xa3, 0x00, 0x00, 0x01, 0xa4, 0x00, 0x60, 0x01, 0x60, 0x02, 0x60, 0x04, 0x00, 0x02, 0x80];
        assert_eq!(answer(SYNC_NETWORK_MONITORS, Some(&replace)), (9, DATA, vec![0xa0]));

        let upsert = request(SYNC_NETWORK_MONITORS, Some(&[0xa2, 0x00, 0x01, 0x01, 0xa0]), Some(4));
        let Step::Failed { reply: Some(reply), .. } = session.handle(&upsert, &mut gather) else { panic!() };
        assert_eq!(envelope(&reply).1, ERROR);
        let unknown = request(42, None, Some(5));
        assert!(matches!(session.handle(&unknown, &mut gather), Step::Failed { reply: Some(_), .. }));
    }

    #[test]
    fn a_protocol_this_agent_cannot_read_means_a_newer_hub() {
        let f = fixture();
        let mut session = Session { identity: &f.identity, verified: true };
        let mut gather = |_: DataOptions| vec![0xa0];
        let newer = |step: Step| matches!(step, Step::NewerHub(_));
        // Not a map; text keys; an action past u8; trailing bytes.
        for msg in [&[0x80][..], &[0xa1, 0x61, b'a', 0x00], &[0xa1, 0x00, 0x19, 0x01, 0x00], &[0xa0, 0x00]] {
            assert!(newer(session.handle(msg, &mut gather)));
        }
        // A cache time past u16, or a payload of the wrong shape.
        let wide = [0xa1, 0x00, 0x1a, 0x00, 0x01, 0x00, 0x00];
        assert!(newer(session.handle(&request(GET_DATA, Some(&wide), Some(1)), &mut gather)));
        assert!(newer(session.handle(&request(GET_DATA, Some(&[0x01]), Some(1)), &mut gather)));
        let text_signature = [0xa1, 0x00, 0x61, b's'];
        assert!(newer(session.handle(&request(CHECK_FINGERPRINT, Some(&text_signature), Some(1)), &mut gather)));
        // Unknown fields are fine.
        let extra = [0xa2, 0x00, 0x19, 0x03, 0xe8, 0x05, 0x63, b'n', b'e', b'w'];
        assert!(!newer(session.handle(&request(GET_DATA, Some(&extra), Some(1)), &mut gather)));

        assert!(refused_for_version(401, "Invalid agent version"));
        assert!(!refused_for_version(401, "Invalid token"));
        assert!(!refused_for_version(400, ""));
        assert!(!refused_for_version(502, "upstream version mismatch"));
        assert!(!refused_for_version(403, "<html><body>version</body></html>"));
    }
}
