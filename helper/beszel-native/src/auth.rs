//! The hub proves who it is by signing the agent's token with the private half of `KEY`.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use ring::signature::{ED25519, UnparsedPublicKey};

pub struct Keys(Vec<[u8; 32]>);

impl Keys {
    /// Reads `KEY` or the contents of `KEY_FILE`: authorized_keys lines, blank lines and comments
    /// skipped. The hub's keys are ed25519; any other type is refused.
    pub fn parse(text: &str) -> Result<Keys, String> {
        let mut keys = Vec::new();
        for line in text.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let key = parse_line(line).map_err(|err| format!("failed to parse key: {line}, error: {err}"))?;
            keys.push(key);
        }
        Ok(Keys(keys))
    }

    pub fn verify(&self, token: &str, signature: &[u8]) -> bool {
        self.0
            .iter()
            .any(|key| UnparsedPublicKey::new(&ED25519, key).verify(token.as_bytes(), signature).is_ok())
    }
}

/// Finds the key in `[options] ssh-ed25519 <base64> [comment]`.
fn parse_line(line: &str) -> Result<[u8; 32], &'static str> {
    let mut fields = line.split_whitespace();
    while let Some(field) = fields.next() {
        if field == "ssh-ed25519" {
            return fields.next().ok_or("no key found").and_then(decode_blob);
        }
    }
    Err("only ssh-ed25519 keys are supported")
}

fn decode_blob(text: &str) -> Result<[u8; 32], &'static str> {
    let blob = STANDARD.decode(text).map_err(|_| "key is not base64")?;
    let (kind, rest) = ssh_string(&blob)?;
    let (key, rest) = ssh_string(rest)?;
    if kind != b"ssh-ed25519" || !rest.is_empty() {
        return Err("malformed ssh-ed25519 key");
    }
    key.try_into().map_err(|_| "malformed ssh-ed25519 key")
}

fn ssh_string(data: &[u8]) -> Result<(&[u8], &[u8]), &'static str> {
    let len = data.get(..4).ok_or("malformed ssh-ed25519 key")?;
    let len = u32::from_be_bytes(len.try_into().unwrap()) as usize;
    let rest = &data[4..];
    if rest.len() < len {
        return Err("malformed ssh-ed25519 key");
    }
    Ok(rest.split_at(len))
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use ring::rand::SystemRandom;
    use ring::signature::{Ed25519KeyPair, KeyPair as _};

    /// A key pair and its `KEY` line, as the hub would hold and show them.
    pub fn hub_key() -> (Ed25519KeyPair, String) {
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
        let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
        let mut blob = Vec::new();
        for part in [&b"ssh-ed25519"[..], pair.public_key().as_ref()] {
            blob.extend_from_slice(&(part.len() as u32).to_be_bytes());
            blob.extend_from_slice(part);
        }
        let line = format!("ssh-ed25519 {} hub", STANDARD.encode(blob));
        (pair, line)
    }

    #[test]
    fn accepts_the_hubs_signature_and_nothing_else() {
        let (pair, line) = hub_key();
        let (other, _) = hub_key();
        let keys = Keys::parse(&format!("# hub\n\n{line}\n")).unwrap();
        let signature = pair.sign(b"token");
        assert!(keys.verify("token", signature.as_ref()));
        assert!(!keys.verify("other token", signature.as_ref()));
        assert!(!keys.verify("token", other.sign(b"token").as_ref()));
        assert!(!keys.verify("token", &[0; 10]));
    }

    #[test]
    fn reads_keys_behind_options_and_refuses_other_types() {
        let (_, line) = hub_key();
        assert!(Keys::parse(&format!("restrict,command=\"x\" {line}")).is_ok());
        assert!(Keys::parse("ssh-rsa AAAAB3NzaC1yc2E user").is_err());
        assert!(Keys::parse("ssh-ed25519 !!!").is_err());
    }
}
