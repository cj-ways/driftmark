//! Links: a relative link points to a file or folder that exists, and an
//! `#anchor` to a heading that exists.
//!
//! Links and images come from the comrak tree, so reference links count with
//! the target their definition gives. A link with a scheme (`https:`,
//! `mailto:`) or one that starts with `//` leaves the repository and is not
//! checked. A path that starts with `/` starts at the repository root, as on
//! GitHub; any other path starts at the linking file's folder.

use std::borrow::Cow;
use std::sync::Arc;

use comrak::nodes::NodeValue;

use super::{Anchors, Context, Report, Rule};
use crate::git::RepoPath;

pub(super) fn check(context: &Context<'_>, report: &mut Report<'_>) {
    if !report.wants(Rule::Link) && !report.wants(Rule::Anchor) {
        return;
    }
    let mut own_anchors = None;
    let links = context
        .root
        .descendants()
        .filter_map(|node| match &node.data().value {
            NodeValue::Link(link) | NodeValue::Image(link) => context
                .positions
                .range(node)
                .map(|range| (link.url.clone(), range.start)),
            _ => None,
        });
    for (url, start) in links {
        let Some(target) = Target::read(&url) else {
            continue;
        };
        let at = context.lines.at(start);
        let path = if target.path.is_empty() {
            Some(context.path.to_owned())
        } else {
            resolve(context.path, &target.path)
        };
        let Some(path) = path else {
            report.add(
                Rule::Link,
                at,
                format!("the link `{url}` leads outside the repository; link to a file inside it, or write a full web address"),
            );
            continue;
        };
        let exists = path.is_empty() || path == context.path || context.targets.exists(&path);
        if !exists {
            let message = if url == path {
                format!(
                    "the link `{url}` names a file that does not exist; fix the path, or create the file"
                )
            } else {
                format!(
                    "the link `{url}` leads to `{path}`, which does not exist; fix the path, or create the file"
                )
            };
            report.add(Rule::Link, at, message);
            continue;
        }
        let Some(fragment) = target.fragment.filter(|fragment| !fragment.is_empty()) else {
            continue;
        };
        let anchors = if path == context.path {
            Some(Arc::clone(own_anchors.get_or_insert_with(|| {
                Arc::new(Anchors::of(context.document))
            })))
        } else if RepoPath::new(path.as_str()).is_some_and(|path| path.is_markdown()) {
            context.targets.anchors(&path)
        } else {
            None
        };
        if let Some(anchors) = anchors.filter(|anchors| !anchors.contains(&fragment)) {
            let hint = anchors.matching_case(&fragment).map_or_else(
                || "link to a heading that exists; its anchor is its text in lower case, with spaces as `-` and punctuation left out".to_owned(),
                |anchor| format!("anchors are lower case, so write `#{anchor}`"),
            );
            report.add(
                Rule::Anchor,
                at,
                format!("`#{fragment}` is not a heading in `{path}`; {hint}"),
            );
        }
    }
}

/// The parts of a link target the lint checks.
struct Target {
    /// The path, percent-decoded; empty for a link to the same file.
    path: String,
    /// The text after `#`, percent-decoded.
    fragment: Option<String>,
}

impl Target {
    /// Reads a link target, or gives `None` for one that leaves the
    /// repository or names nothing: `https://…`, `mailto:…`, `//host/…`, an
    /// empty target.
    fn read(url: &str) -> Option<Target> {
        if url.is_empty() || url.starts_with("//") || has_scheme(url) {
            return None;
        }
        let (rest, fragment) = match url.split_once('#') {
            Some((rest, fragment)) => (rest, Some(decode(fragment).into_owned())),
            None => (url, None),
        };
        let path = rest.split_once('?').map_or(rest, |(path, _)| path);
        Some(Target {
            path: decode(path).into_owned(),
            fragment,
        })
    }
}

/// Whether a link target starts with a scheme such as `https:` or `mailto:`.
fn has_scheme(url: &str) -> bool {
    url.split_once(':').is_some_and(|(scheme, _)| {
        scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    })
}

/// Turns `%20` and the like back into the characters they stand for. Text
/// that does not decode to UTF-8 is kept as written.
fn decode(text: &str) -> Cow<'_, str> {
    if !text.contains('%') {
        return Cow::Borrowed(text);
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let pair = bytes.get(at + 1..at + 3).and_then(|pair| {
            std::str::from_utf8(pair)
                .ok()
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        });
        match (bytes[at], pair) {
            (b'%', Some(byte)) => {
                out.push(byte);
                at += 3;
            }
            (byte, _) => {
                out.push(byte);
                at += 1;
            }
        }
    }
    String::from_utf8(out).map_or(Cow::Borrowed(text), Cow::Owned)
}

/// Where a relative link from the file at `from` leads, as a path from the
/// repository root; `None` when it climbs out of the repository.
fn resolve(from: &str, link: &str) -> Option<String> {
    let mut parts: Vec<&str> = if link.starts_with('/') {
        Vec::new()
    } else {
        from.split('/').collect()
    };
    if !link.starts_with('/') {
        parts.pop();
    }
    for part in link.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            name => parts.push(name),
        }
    }
    Some(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::super::tests::{lint_in, messages};
    use super::{Target, decode, has_scheme, resolve};

    const HEAD: &str = "# Title\n\nSummary.\n\n";

    /// The made-up repository the link tests run in.
    const FILES: &[(&str, &str)] = &[
        ("docs/guide.md", "# Guide\n\n## Set up\n\n## Run it\n"),
        ("docs/notes.txt", "not Markdown"),
        ("docs/img/cat.png", ""),
        ("docs/sub/deep.md", "# Deep\n"),
        ("README.md", "# Read me\n\n## Install\n"),
        ("docs/broken.md", "UNREADABLE"),
    ];

    fn found(text: &str) -> Vec<String> {
        messages(&lint_in(FILES, "docs/here.md", &format!("{HEAD}{text}")))
    }

    #[test]
    fn links_that_resolve_pass() {
        let text = "\
[guide](guide.md), [set up](guide.md#set-up), [same](./guide.md?plain=1#run-it),
[folder](sub/), [up](../README.md#install), [root](/README.md), [deep](sub/deep.md),
[here](#summary-heading), [self](here.md#summary-heading), [top](#), [empty](),
![cat](img/cat.png), [web](https://example.com/x.md), [mail](mailto:a@example.com),
<https://example.com>, [proto](//example.com/a), [text](notes.txt#L10),
[spaces](sub%2Fdeep.md), [unreadable](broken.md#x), [root](/), [ref][r].

[r]: guide.md#run-it

## Summary heading
";
        assert_eq!(found(text), Vec::<String>::new());
    }

    #[test]
    fn missing_files_and_escapes_are_reported() {
        assert_eq!(
            found(
                "[a](guid.md) and ![b](img/dog.png)\n\n[c](../../outside.md) and [d][d]\n\n[d]: Guide.md\n"
            ),
            [
                "5:1 error link: the link `guid.md` leads to `docs/guid.md`, which does not exist; fix the path, or create the file",
                "5:18 error link: the link `img/dog.png` leads to `docs/img/dog.png`, which does not exist; fix the path, or create the file",
                "7:1 error link: the link `../../outside.md` leads outside the repository; link to a file inside it, or write a full web address",
                "7:27 error link: the link `Guide.md` leads to `docs/Guide.md`, which does not exist; fix the path, or create the file",
            ]
        );
        let at_the_root = lint_in(FILES, "here.md", &format!("{HEAD}[a](guid.md)\n"));
        assert_eq!(
            messages(&at_the_root),
            [
                "5:1 error link: the link `guid.md` names a file that does not exist; fix the path, or create the file"
            ]
        );
    }

    #[test]
    fn missing_anchors_are_reported_with_a_hint() {
        assert_eq!(
            found("[a](guide.md#Set-Up), [b](guide.md#setup), [c](#nowhere)\n"),
            [
                "5:1 error anchor: `#Set-Up` is not a heading in `docs/guide.md`; anchors are lower case, so write `#set-up`",
                "5:23 error anchor: `#setup` is not a heading in `docs/guide.md`; link to a heading that exists; its anchor is its text in lower case, with spaces as `-` and punctuation left out",
                "5:44 error anchor: `#nowhere` is not a heading in `docs/here.md`; link to a heading that exists; its anchor is its text in lower case, with spaces as `-` and punctuation left out",
            ]
        );
    }

    #[test]
    fn each_check_can_be_turned_off() {
        let source = format!("{HEAD}[a](nope.md) [b](#nope)\n");
        let only_anchor = "[lint.rules]\nlink = \"off\"\n";
        let only_link = "[lint.rules]\nanchor = \"off\"\n";
        let both = "[lint.rules]\nlink = \"off\"\nanchor = \"off\"\n";
        let rules = |text: &str| {
            messages(&super::super::tests::lint_in_with(
                text,
                FILES,
                "docs/here.md",
                &source,
            ))
        };
        assert_eq!(rules(only_anchor).len(), 1);
        assert!(rules(only_anchor)[0].contains("anchor"));
        assert_eq!(rules(only_link).len(), 1);
        assert!(rules(only_link)[0].contains("link"));
        assert_eq!(rules(both), Vec::<String>::new());
    }

    #[test]
    fn targets_split_into_path_and_fragment() {
        let target = Target::read("a%20b.md?x=1#part%201").expect("relative");
        assert_eq!(target.path, "a b.md");
        assert_eq!(target.fragment.as_deref(), Some("part 1"));
        assert!(Target::read("").is_none());
        assert!(Target::read("//host/x").is_none());
        assert!(Target::read("HTTP://x").is_none());
        assert_eq!(Target::read("#top").map(|t| t.path), Some(String::new()));
    }

    #[test]
    fn schemes_are_letters_then_letters_digits_and_plus_minus_dot() {
        assert!(has_scheme("git+ssh://x"));
        assert!(has_scheme("a1-b.c:x"));
        assert!(!has_scheme("1a:x"));
        assert!(!has_scheme("a b:x"));
        assert!(!has_scheme("path/a:b"));
        assert!(!has_scheme(":x"));
        assert!(!has_scheme("plain.md"));
    }

    #[test]
    fn percent_escapes_decode_only_when_whole() {
        assert_eq!(decode("a%2Fb%41"), "a/bA");
        assert_eq!(decode("100%"), "100%");
        assert_eq!(decode("%4"), "%4");
        assert_eq!(decode("%zz%41"), "%zzA");
        assert_eq!(decode("%C3%A9"), "é");
        assert_eq!(decode("%FF"), "%FF");
        assert_eq!(decode("%é1"), "%é1");
    }

    #[test]
    fn paths_resolve_from_the_folder_or_the_root() {
        assert_eq!(resolve("docs/a.md", "b.md").as_deref(), Some("docs/b.md"));
        assert_eq!(
            resolve("docs/a.md", "./x/../b.md").as_deref(),
            Some("docs/b.md")
        );
        assert_eq!(resolve("docs/a.md", "../b.md").as_deref(), Some("b.md"));
        assert_eq!(resolve("docs/a.md", "/c/d.md").as_deref(), Some("c/d.md"));
        assert_eq!(resolve("a.md", "sub/").as_deref(), Some("sub"));
        assert_eq!(resolve("a.md", "..").as_deref(), None);
        assert_eq!(resolve("docs/a.md", "../..").as_deref(), None);
        assert_eq!(resolve("a.md", "/").as_deref(), Some(""));
    }
}
