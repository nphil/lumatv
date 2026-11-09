//! Pairing: the owner opens a window on the TV, which shows an 8-character code; the phone derives the pairing key from the code and the
//! TV's id and runs the handshake with it, and the TV answers with the phone's own key. The window lasts [`WINDOW`], closes after [`TRIES`]
//! failed tries and pairs one phone.

use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use ring::pbkdf2;
use ring::rand::SecureRandom;

/// 2–9 and A–Z without I and O: 32 characters, 5 bits each.
pub const ALPHABET: &[u8; 32] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";
pub const CODE_LEN: usize = 8;
pub const ITERATIONS: u32 = 131_072;
/// The salt's prefix; the TV id's 8 bytes follow.
pub const SALT: &[u8] = b"LumaRemote pair v1";
pub const WINDOW: Duration = Duration::from_secs(300);
pub const TRIES: u8 = 5;

/// A new code from the system's random source. 32 characters divide 256, so masking a byte to 5 bits is uniform.
pub fn new_code(random: &dyn SecureRandom) -> String {
    let mut bytes = [0u8; CODE_LEN];
    random.fill(&mut bytes).expect("system random source");
    bytes.iter().map(|b| ALPHABET[usize::from(b & 31)] as char).collect()
}

/// A typed or scanned code as the key is derived from it: upper case, spaces and dashes removed. `None` unless that is 8 characters of the
/// alphabet.
pub fn normalize(typed: &str) -> Option<String> {
    let code: String = typed.chars().filter(|c| *c != ' ' && *c != '-').map(|c| c.to_ascii_uppercase()).collect();
    (code.len() == CODE_LEN && code.bytes().all(|b| ALPHABET.contains(&b))).then_some(code)
}

/// The code as the TV shows it: `XXXX-XXXX`.
pub fn display(code: &str) -> String {
    format!("{}-{}", &code[..4], &code[4..])
}

/// The pairing key: PBKDF2-HMAC-SHA256 of the normalized code, salted with [`SALT`] and the TV id.
pub fn key(code: &str, tv: &[u8; 8]) -> [u8; 32] {
    let mut salt = SALT.to_vec();
    salt.extend_from_slice(tv);
    let mut out = [0u8; 32];
    pbkdf2::derive(pbkdf2::PBKDF2_HMAC_SHA256, NonZeroU32::new(ITERATIONS).expect("non-zero"), &salt, code.as_bytes(), &mut out);
    out
}

/// One open pairing window. A pairing connection claims it for its handshake, so two phones cannot pair on one code; the claim ends in
/// [`Window::failed`] (a try used) or with the window, which pairs once.
pub struct Window {
    code: String,
    key: [u8; 32],
    until: Instant,
    tries: u8,
    claimed: bool,
}

impl Window {
    /// Derives the key now (a few milliseconds on the TV), so a phone's handshake does not wait for it.
    pub fn new(code: String, tv: &[u8; 8], now: Instant) -> Self {
        let key = key(&code, tv);
        Self::with_key(code, key, now)
    }

    /// A window whose key is already derived.
    pub fn with_key(code: String, key: [u8; 32], now: Instant) -> Self {
        Self { code, key, until: now + WINDOW, tries: TRIES, claimed: false }
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    pub fn until(&self) -> Instant {
        self.until
    }

    /// Whether the window can still pair: not expired and tries left. A window that cannot is closed by its owner.
    pub fn alive(&self, now: Instant) -> bool {
        self.tries > 0 && now < self.until
    }

    /// The key for one pairing connection's handshake, if the window is alive and no other connection holds it.
    pub fn claim(&mut self, now: Instant) -> Option<[u8; 32]> {
        if self.claimed || !self.alive(now) {
            return None;
        }
        self.claimed = true;
        Some(self.key)
    }

    /// The claiming connection failed (a wrong code, or a refused `pair`): one try is used and the window is free again.
    pub fn failed(&mut self) {
        self.claimed = false;
        self.tries = self.tries.saturating_sub(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_eight_characters_of_the_alphabet_and_typing_is_forgiving() {
        let code = new_code(&ring::rand::SystemRandom::new());
        assert_eq!(normalize(&code).as_deref(), Some(code.as_str()));
        assert_eq!(normalize(" k7wd-x2ab ").as_deref(), Some("K7WDX2AB"));
        assert_eq!(normalize("K7WD-X2A"), None);
        assert_eq!(normalize("K7WD-X2AO"), None, "O is not in the alphabet");
        assert_eq!(normalize("K7WD-X2A1"), None, "nor 1");
        assert_eq!(display("K7WDX2AB"), "K7WD-X2AB");
    }

    #[test]
    fn a_window_pairs_one_connection_at_a_time_and_closes_after_five_failures_or_five_minutes() {
        let start = Instant::now();
        let mut window = Window::with_key("K7WDX2AB".into(), [9; 32], start);
        assert_eq!(window.claim(start), Some([9; 32]));
        assert_eq!(window.claim(start), None, "held by the first connection");
        for _ in 0..TRIES - 1 {
            window.failed();
            assert!(window.claim(start).is_some());
        }
        window.failed();
        assert!(!window.alive(start));
        assert_eq!(window.claim(start), None);

        let window = Window::with_key("K7WDX2AB".into(), [9; 32], start);
        assert!(window.alive(start + WINDOW - Duration::from_secs(1)));
        assert!(!window.alive(start + WINDOW));
    }
}
