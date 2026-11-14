//! `NICS` and `SENSORS`: comma-separated names, `-` in front for a blacklist, `*` and `?` wildcards.

pub struct NameFilter {
    names: Vec<String>,
    blacklist: bool,
    wildcards: bool,
}

impl NameFilter {
    pub fn parse(value: &str) -> NameFilter {
        let (blacklist, value) = match value.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, value),
        };
        let names: Vec<String> = value.split(',').map(str::trim).filter(|name| !name.is_empty()).map(str::to_owned).collect();
        let wildcards = names.iter().any(|name| name.contains('*'));
        NameFilter { names, blacklist, wildcards }
    }

    pub fn is_blacklist(&self) -> bool {
        self.blacklist
    }

    /// Whether `name` passes. `if_empty` answers for a filter that names nothing: the two settings differ.
    pub fn allows(&self, name: &str, if_empty: bool) -> bool {
        if self.names.is_empty() {
            return if_empty;
        }
        if self.names.iter().any(|n| n == name) {
            return !self.blacklist;
        }
        if self.wildcards && self.names.iter().any(|pattern| pattern.contains('*') && matches(pattern.as_bytes(), name.as_bytes())) {
            return !self.blacklist;
        }
        self.blacklist
    }
}

/// Go's `path.Match` for the patterns these settings use: `*` is any run of characters but `/`,
/// `?` any one byte but `/`.
fn matches(pattern: &[u8], name: &[u8]) -> bool {
    match pattern.split_first() {
        None => name.is_empty(),
        Some((b'*', rest)) => (0..=name.len()).take_while(|&i| i == 0 || name[i - 1] != b'/').any(|i| matches(rest, &name[i..])),
        Some((b'?', rest)) => name.first().is_some_and(|&c| c != b'/') && matches(rest, &name[1..]),
        Some((c, rest)) => name.first() == Some(c) && matches(rest, &name[1..]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_names_like_the_go_agent() {
        let only = NameFilter::parse("eth0, wlan*");
        assert!(only.allows("eth0", false));
        assert!(only.allows("wlan0", false));
        assert!(!only.allows("eth1", false));

        let except = NameFilter::parse("-veth*,lo");
        assert!(except.is_blacklist());
        assert!(!except.allows("lo", true));
        assert!(!except.allows("veth12", true));
        assert!(except.allows("eth0", true));

        assert!(NameFilter::parse("").allows("eth0", true));
        assert!(!NameFilter::parse("").allows("eth0", false));
        assert!(matches(b"cpu?_*", b"cpu1_core_0"));
        assert!(!matches(b"a*", b"a/b"));
    }
}
