//! Version strings as the suite writes them: `2.8.0`, `app-v2.8.0`, `1.4.0.0`, `2.0.0-beta.1`.

use std::cmp::Ordering;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub numbers: Vec<u64>,
    /// Empty for a release. A release sorts above any pre-release of the same numbers.
    pub pre: Vec<String>,
}

/// Parse leniently: anything before the first digit is a tag prefix, `+build` is ignored.
/// `None` only when there is no number in it at all.
pub fn parse(text: &str) -> Option<Version> {
    let start = text.find(|c: char| c.is_ascii_digit())?;
    let text = text[start..].trim();
    let text = text.split('+').next().unwrap_or(text);
    let (core, pre) = match text.split_once('-') {
        Some((core, pre)) => (core, pre),
        None => (text, ""),
    };
    let mut numbers = Vec::new();
    for part in core.split('.') {
        numbers.push(part.trim().parse::<u64>().ok()?);
    }
    while numbers.len() > 1 && numbers.last() == Some(&0) {
        numbers.pop();
    }
    let pre = pre.split('.').filter(|p| !p.is_empty()).map(str::to_string).collect();
    Some(Version { numbers, pre })
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        let n = self.numbers.len().max(other.numbers.len());
        for i in 0..n {
            let a = self.numbers.get(i).copied().unwrap_or(0);
            let b = other.numbers.get(i).copied().unwrap_or(0);
            if a != b {
                return a.cmp(&b);
            }
        }
        match (self.pre.is_empty(), other.pre.is_empty()) {
            (true, true) => return Ordering::Equal,
            (true, false) => return Ordering::Greater,
            (false, true) => return Ordering::Less,
            _ => {}
        }
        for (a, b) in self.pre.iter().zip(other.pre.iter()) {
            let order = match (a.parse::<u64>(), b.parse::<u64>()) {
                (Ok(a), Ok(b)) => a.cmp(&b),
                (Ok(_), Err(_)) => Ordering::Less,
                (Err(_), Ok(_)) => Ordering::Greater,
                _ => a.cmp(b),
            };
            if order != Ordering::Equal {
                return order;
            }
        }
        self.pre.len().cmp(&other.pre.len())
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Compare two version strings. Strings that are not versions compare as text, so two identical
/// odd strings are still "the same version".
pub fn compare(a: &str, b: &str) -> Ordering {
    match (parse(a), parse(b)) {
        (Some(a), Some(b)) => a.cmp(&b),
        _ => a.trim().cmp(b.trim()),
    }
}

/// What a row says about one app. The words are the contract with the frontend.
pub fn status(installed: Option<&str>, offered: Option<&str>) -> &'static str {
    match (installed, offered) {
        (None, None) => "unavailable",
        (None, Some(_)) => "not-installed",
        (Some(_), None) => "installed",
        (Some(have), Some(offer)) => match compare(have, offer) {
            Ordering::Less => "update",
            Ordering::Equal => "current",
            Ordering::Greater => "ahead",
        },
    }
}

/// The version as shown: without a tag prefix and without the fourth `.0` Windows adds.
pub fn display(text: &str) -> String {
    let Some(start) = text.find(|c: char| c.is_ascii_digit()) else {
        return text.trim().to_string();
    };
    let text = text[start..].trim();
    let (core, rest) = match text.find(['-', '+']) {
        Some(i) => text.split_at(i),
        None => (text, ""),
    };
    let mut parts: Vec<&str> = core.split('.').collect();
    while parts.len() > 3 && parts.last() == Some(&"0") {
        parts.pop();
    }
    format!("{}{}", parts.join("."), rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orders_plain_versions_by_number_not_by_text() {
        assert_eq!(compare("2.10.0", "2.9.0"), Ordering::Greater);
        assert_eq!(compare("1.4.3", "2.0.0"), Ordering::Less);
        assert_eq!(compare("2.8.0", "2.8.0"), Ordering::Equal);
    }

    #[test]
    fn ignores_tag_prefixes_and_trailing_zeros() {
        assert_eq!(compare("app-v2.8.0", "2.8.0"), Ordering::Equal);
        assert_eq!(compare("v2.0.0", "2.0"), Ordering::Equal);
        // The Sim's exe carries 1.4.0.0 while its registry entry says 1.4.0.
        assert_eq!(compare("1.4.0.0", "1.4.0"), Ordering::Equal);
        assert_eq!(compare("1.4.0.1", "1.4.0"), Ordering::Greater);
    }

    #[test]
    fn a_release_is_newer_than_its_own_prereleases() {
        assert_eq!(compare("2.0.0", "2.0.0-beta.2"), Ordering::Greater);
        assert_eq!(compare("2.0.0-beta.2", "2.0.0-beta.10"), Ordering::Less);
        assert_eq!(compare("2.0.0-alpha.5", "2.0.0-beta.1"), Ordering::Less);
        assert_eq!(compare("2.0.0-rc.1", "2.0.0-rc.1"), Ordering::Equal);
    }

    #[test]
    fn status_words() {
        assert_eq!(status(None, Some("2.8.0")), "not-installed");
        assert_eq!(status(Some("2.7.0"), Some("2.8.0")), "update");
        assert_eq!(status(Some("2.8.0"), Some("app-v2.8.0")), "current");
        assert_eq!(status(Some("2.9.0"), Some("2.8.0")), "ahead");
        assert_eq!(status(Some("2.9.0"), None), "installed");
        assert_eq!(status(None, None), "unavailable");
    }

    #[test]
    fn display_drops_the_prefix_and_the_fourth_zero() {
        assert_eq!(display("app-v2.8.0"), "2.8.0");
        assert_eq!(display("1.4.0.0"), "1.4.0");
        assert_eq!(display("v2.0.0-beta.1"), "2.0.0-beta.1");
        assert_eq!(display("154.0.4258.53"), "154.0.4258.53");
    }

    #[test]
    fn nonsense_is_not_a_version() {
        assert!(parse("latest").is_none());
        assert!(parse("2.x.0").is_none());
    }
}
