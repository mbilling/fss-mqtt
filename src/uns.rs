//! A unified namespace (UNS): a connection's topics as one hierarchy, its
//! levels named in the connections file (`uns = ["enterprise", "site",
//! "area", "line", "cell"]`). A topic that doesn't follow it is flagged (see
//! watch.rs), its details name each level, and the filter takes
//! `level=value` (`site=oslo`).

/// What's wrong with `topic` in a namespace of `levels`: too few levels for
/// all of them, or a level not named in lower case letters, digits, `-` and
/// `_`. The first thing wrong, if any.
pub fn problem(levels: &[String], topic: &str) -> Option<String> {
    if levels.is_empty() {
        return None;
    }
    let segs: Vec<&str> = topic.split('/').collect();
    for (i, s) in segs.iter().enumerate() {
        let what = match levels.get(i) {
            Some(l) => l.clone(),
            None => format!("level {}", i + 1),
        };
        if s.is_empty() {
            return Some(format!("{what} is empty"));
        }
        if !s.chars().all(name_char) {
            return Some(format!("{what} {s:?}: lower case letters, digits, - and _ only"));
        }
    }
    if segs.len() < levels.len() {
        let missing = &levels[segs.len()..];
        let list = match missing {
            [one] => one.clone(),
            [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
            [] => unreachable!(),
        };
        let s = if missing.len() == 1 { "" } else { "s" };
        return Some(format!("missing the {list} level{s}"));
    }
    None
}

/// A level name, in the connections file: as a level's value must be.
pub fn valid_level(name: &str) -> bool {
    !name.is_empty() && name.chars().all(name_char)
}

fn name_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_'
}

/// The levels `topic` fills, with their names: ("site", "oslo"), …
pub fn levels<'a>(levels: &'a [String], topic: &'a str) -> Vec<(&'a str, &'a str)> {
    levels.iter().map(String::as_str).zip(topic.split('/')).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn isa95() -> Vec<String> {
        ["enterprise", "site", "area", "line", "cell"]
            .map(String::from)
            .to_vec()
    }

    #[test]
    fn a_topic_follows_the_namespace_or_says_why_not() {
        let l = isa95();
        assert_eq!(problem(&l, "acme/oslo/packaging/line1/filler/speed"), None);
        assert_eq!(problem(&l, "acme/oslo/packaging/line1/filler"), None);
        assert_eq!(
            problem(&l, "acme/bergen/energy").as_deref(),
            Some("missing the line and cell levels")
        );
        assert_eq!(
            problem(&l, "acme/bergen/energy/meter").as_deref(),
            Some("missing the cell level")
        );
        assert_eq!(
            problem(&l, "acme/oslo/Packaging Hall/line3/wrapper").as_deref(),
            Some("area \"Packaging Hall\": lower case letters, digits, - and _ only")
        );
        assert_eq!(
            problem(&l, "acme/oslo/packaging/line1/filler/Speed").as_deref(),
            Some("level 6 \"Speed\": lower case letters, digits, - and _ only")
        );
        assert_eq!(problem(&l, "acme//packaging").as_deref(), Some("site is empty"));
        assert_eq!(problem(&[], "Anything Goes"), None);
        assert_eq!(
            levels(&l, "acme/oslo/packaging"),
            [("enterprise", "acme"), ("site", "oslo"), ("area", "packaging")]
        );
    }
}
