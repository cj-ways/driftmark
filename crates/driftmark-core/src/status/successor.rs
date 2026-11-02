//! Finding the document that replaced a superseded one.
//!
//! A document names its successor in its "superseded by" value, or in its
//! status, as in `Superseded by [0009](0009-new.md)`. The value is tried
//! four ways, and the first that leads to exactly one document wins:
//!
//! 1. its links, resolved from the document's folder, or from the
//!    repository root when they start with `/`;
//! 2. the text itself, when it is a Markdown file name such as
//!    `0005-split-the-database.md`: from the document's folder, then from
//!    the root;
//! 3. a document whose title is the text, ignoring case and runs of spaces,
//!    as adr-tools writes `Superceded by [3. Store files on disk](…)`;
//! 4. the first number in the text, as in `Superseded by 0009` or
//!    `ADR-12`: the one file in the same folder whose name starts with that
//!    number, leading zeros and an `adr-` prefix aside.
//!
//! When none leads to a document, the reason says so plainly: the path a
//! link named that is not there, a text that names nothing found, or no
//! text at all.

use std::collections::HashMap;

use super::{Entry, Successor, Target, folder_of};
use crate::rules::Lifecycle;

/// The documents of one revision, looked up by path, by title and by
/// folder.
pub(super) struct Index<'a> {
    paths: HashMap<&'a str, usize>,
    titles: HashMap<String, Vec<usize>>,
    folders: HashMap<&'a str, Vec<(usize, &'a str)>>,
}

impl<'a> Index<'a> {
    /// The index of `entries`, each known by its position.
    pub(super) fn new(entries: impl Iterator<Item = &'a Entry>) -> Index<'a> {
        let mut index = Index {
            paths: HashMap::new(),
            titles: HashMap::new(),
            folders: HashMap::new(),
        };
        for (at, entry) in entries.enumerate() {
            index.paths.entry(entry.path.as_str()).or_insert(at);
            if let Some(title) = &entry.title {
                index.titles.entry(same_words(title)).or_default().push(at);
            }
            let folder = folder_of(&entry.path);
            let name = &entry.path[folder.len()..];
            index.folders.entry(folder).or_default().push((at, name));
        }
        index
    }

    /// The one document with this title.
    fn titled(&self, text: &str) -> Option<usize> {
        match self.titles.get(&same_words(text))?.as_slice() {
            [only] => Some(*only),
            _ => None,
        }
    }

    /// The one document in `folder` whose file name starts with `number`.
    fn numbered(&self, folder: &str, number: &str) -> Option<usize> {
        let mut found = self
            .folders
            .get(folder)
            .into_iter()
            .flatten()
            .filter(|(_, name)| leading_number(name).is_some_and(|own| own == number));
        match (found.next(), found.next()) {
            (Some((at, _)), None) => Some(*at),
            _ => None,
        }
    }
}

/// The successor of `entry`: what its "superseded by" or its superseded
/// status names, and which document that is. `None` when it names none
/// and is not superseded.
pub(super) fn find(entry: &Entry, index: &Index<'_>) -> Option<Successor> {
    let (text, links) = match (&entry.superseded_by, entry.lifecycle) {
        (Some(text), _) => (Some(text.as_str()), &entry.superseded_by_links),
        (None, Some(Lifecycle::Superseded)) => (
            entry.status.as_deref().and_then(after_by),
            &entry.status_links,
        ),
        (None, _) => return None,
    };
    Some(Successor {
        text: text.map(str::to_owned),
        target: resolve(&entry.path, text, links, index),
    })
}

/// Which document `text` and `links`, written in the document at `from`,
/// lead to.
fn resolve(from: &str, text: Option<&str>, links: &[String], index: &Index<'_>) -> Target {
    let found = |path: &str| index.paths.get(path).copied();
    let mut missing = None;
    for link in links {
        if let Some(path) = link_target(from, link) {
            if let Some(at) = found(&path) {
                return Target::Found(at);
            }
            if missing.is_none() && is_markdown_name(&path) {
                missing = Some(path);
            }
        }
    }
    if let Some(text) = text.map(str::trim) {
        if is_markdown_name(text) && !text.contains(char::is_whitespace) {
            let near = normalize(&format!("{}{text}", folder_of(from)));
            let rooted = normalize(text.trim_start_matches('/'));
            if let Some(at) = [&near, &rooted]
                .into_iter()
                .flatten()
                .find_map(|path| found(path))
            {
                return Target::Found(at);
            }
            missing = missing.or(near);
        }
        if let Some(at) = index.titled(text) {
            return Target::Found(at);
        }
        if let Some(at) =
            first_number(text).and_then(|number| index.numbered(folder_of(from), &number))
        {
            return Target::Found(at);
        }
    }
    match (missing, text) {
        (Some(path), _) => Target::Missing(path),
        (None, Some(_)) => Target::Unmatched,
        (None, None) => Target::Unnamed,
    }
}

/// What a status names after the word "by": `Superseded by: 0009` gives
/// `0009`. `None` when it has no such word, or nothing after it.
fn after_by(status: &str) -> Option<&str> {
    let lower = status.to_ascii_lowercase();
    lower
        .match_indices("by")
        .find_map(|(at, _)| {
            let alone = !lower[..at].ends_with(char::is_alphanumeric)
                && !lower[at + 2..].starts_with(char::is_alphanumeric);
            alone.then(|| {
                status[at + 2..]
                    .trim_start_matches(|c: char| c == ':' || c.is_whitespace())
                    .trim_end()
            })
        })
        .filter(|rest| !rest.is_empty())
}

/// The repository path a link leads to from the document at `from`, or
/// `None` for a link to another site, a link to a place in the same
/// document, or one that climbs out of the repository.
fn link_target(from: &str, link: &str) -> Option<String> {
    let file = link.split(['#', '?']).next().unwrap_or_default();
    if file.is_empty() || file.contains("://") || file.starts_with("mailto:") {
        return None;
    }
    let file = percent_decoded(file);
    match file.strip_prefix('/') {
        Some(rooted) => normalize(rooted),
        None => normalize(&format!("{}{file}", folder_of(from))),
    }
}

/// A path with its `.` and `..` parts worked out and empty parts dropped.
/// `None` when it climbs above the repository root or names nothing.
fn normalize(path: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            _ => parts.push(part),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// A link's `%XX` escapes decoded, as a browser follows them; the link as
/// written when the result is not text.
fn percent_decoded(link: &str) -> String {
    let bytes = link.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let escape = bytes
            .get(at + 1..at + 3)
            .filter(|_| bytes[at] == b'%')
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        if let Some(byte) = escape {
            out.push(byte);
            at += 3;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| link.to_owned())
}

/// Whether a name ends in `.md` or `.markdown`, in any case.
fn is_markdown_name(name: &str) -> bool {
    std::path::Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("md") || extension.eq_ignore_ascii_case("markdown")
        })
}

/// Text in the form titles are compared in: lower case, with every run of
/// white space as one space and none at either end.
fn same_words(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// The first run of digits in `text`, without leading zeros: `0009` gives
/// `9`, and `000` gives `0`.
fn first_number(text: &str) -> Option<String> {
    let start = text.find(|c: char| c.is_ascii_digit())?;
    let digits: &str = text[start..]
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .unwrap_or_default();
    let trimmed = digits.trim_start_matches('0');
    Some(if trimmed.is_empty() { "0" } else { trimmed }.to_owned())
}

/// The number a file name starts with, after an optional `adr-` prefix in
/// any case, without leading zeros: `0009-new.md` and `ADR-9-new.md` both
/// give `9`. `None` when the name does not start with a digit.
fn leading_number(name: &str) -> Option<String> {
    let rest = name
        .get(..4)
        .filter(|prefix| prefix.eq_ignore_ascii_case("adr-"))
        .map_or(name, |_| &name[4..]);
    rest.starts_with(|c: char| c.is_ascii_digit())
        .then(|| first_number(rest))
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_resolve_from_the_document_folder_or_the_root() {
        let from = "docs/decisions/0002-a.md";
        assert_eq!(
            link_target(from, "0003-b.md#decision"),
            Some("docs/decisions/0003-b.md".into())
        );
        assert_eq!(
            link_target(from, "./0003-b.md?plain=1"),
            Some("docs/decisions/0003-b.md".into())
        );
        assert_eq!(
            link_target(from, "../features/x%20y.md"),
            Some("docs/features/x y.md".into())
        );
        assert_eq!(link_target(from, "/README.md"), Some("README.md".into()));
        assert_eq!(link_target("a.md", "b.md"), Some("b.md".into()));
        for none in [
            "https://example.com/a.md",
            "mailto:ana@example.com",
            "#decision",
            "",
            "../../../up.md",
        ] {
            assert_eq!(link_target(from, none), None, "{none}");
        }
        assert_eq!(normalize("a//./b/../c"), Some("a/c".into()));
        assert_eq!(normalize("./"), None);
        assert_eq!(normalize(".."), None);
    }

    #[test]
    fn escapes_decode_only_when_they_make_text() {
        assert_eq!(percent_decoded("a%2Fb%41"), "a/bA");
        assert_eq!(percent_decoded("100%"), "100%");
        assert_eq!(percent_decoded("%zz%4"), "%zz%4");
        assert_eq!(percent_decoded("%C3%A9"), "é");
        assert_eq!(percent_decoded("%FF.md"), "%FF.md");
        assert_eq!(percent_decoded("é%"), "é%");
    }

    #[test]
    fn the_word_by_starts_the_successor() {
        assert_eq!(after_by("Superseded by 0009"), Some("0009"));
        assert_eq!(after_by("superseded BY:  ADR-12 "), Some("ADR-12"));
        assert_eq!(after_by("By 7"), Some("7"));
        assert_eq!(after_by("Superseded (by 3)"), Some("3)"));
        assert_eq!(after_by("Superseded"), None);
        assert_eq!(after_by("Superseded by"), None);
        assert_eq!(after_by("Superseded by: "), None);
        assert_eq!(after_by("Bypassed, nearby"), None);
        assert_eq!(after_by("standby by 4"), Some("4"));
    }

    #[test]
    fn numbers_and_names_compare_without_leading_zeros() {
        assert_eq!(first_number("ADR-0012 and 4"), Some("12".into()));
        assert_eq!(first_number("000"), Some("0".into()));
        assert_eq!(first_number("none"), None);
        assert_eq!(leading_number("0009-new.md"), Some("9".into()));
        assert_eq!(leading_number("ADR-9-new.md"), Some("9".into()));
        assert_eq!(leading_number("adr-0010.md"), Some("10".into()));
        assert_eq!(leading_number("adr-x1.md"), None);
        assert_eq!(leading_number("template-1.md"), None);
        assert_eq!(leading_number("é1.md"), None);
        assert_eq!(leading_number("ad"), None);
        assert!(is_markdown_name("A.MarkDown"));
        assert!(is_markdown_name("x.MD"));
        assert!(!is_markdown_name("x.txt"));
        assert_eq!(same_words("  3.  Store\tFiles "), "3. store files");
    }
}
