//! The web addresses of the server's pages, read and written.
//!
//! A page address names a revision or a range, then a document after the
//! separator `/-/`:
//!
//! - `/doc/<revision>/-/<path>`: a document at a revision;
//! - `/changes/<range>`: a range's change list;
//! - `/changes/<range>/-/<path>`: one document's changes in a range.
//!
//! Branch names hold `/`, so the separator cannot be a plain `/`. The path
//! comes after the separator so that a relative link in a document, such as
//! `../guide.md`, leads to the other document at the same revision without
//! being rewritten.
//!
//! Addresses are read from the raw request path, before percent-decoding, so
//! a `%2F` or `%2D` inside a branch name never splits it. The path is decoded
//! to bytes and checked like every path git stores: no empty, `.` or `..`
//! part. It is then looked up in git, never opened as a file by its name.

use driftmark_core::git::{RangeError, RangeSpec, RepoPath};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};

/// The separator between a revision or a range and a document's path.
pub(crate) const SEPARATOR: &str = "/-/";

/// The revision word that means the files on disk, committed or not. Like
/// `auto` for ranges, it always means the working tree, even if a branch has
/// that name; write `refs/heads/worktree` for such a branch.
pub(crate) const WORKTREE: &str = "worktree";

/// Characters left as they are in an address: letters, digits and the
/// characters that need no escaping in a path and read well in a branch
/// name or a range.
const KEPT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~')
    .remove(b'/')
    .remove(b'@')
    .remove(b':')
    .remove(b'!')
    .remove(b'$')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')')
    .remove(b'*')
    .remove(b'+')
    .remove(b',')
    .remove(b';')
    .remove(b'=');

/// The revision of a document page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Revision {
    /// The files on disk now.
    Worktree,
    /// A name git resolves to a commit: a branch, a tag, an id or an
    /// expression such as `main~2`.
    Name(String),
}

impl Revision {
    /// The revision as written in an address.
    pub(crate) fn label(&self) -> &str {
        match self {
            Self::Worktree => WORKTREE,
            Self::Name(name) => name,
        }
    }
}

/// A range as an address writes it, read into a [`RangeSpec`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RangeAddress {
    /// The range as written, after decoding, such as `main...feature` or
    /// `since/2026-10-01/main`.
    pub label: String,
    /// The range.
    pub spec: RangeSpec,
}

/// Why an address names no page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BadAddress(pub String);

/// Splits `<target>/-/<path>` at the first separator. Without one, the whole
/// text is the target.
pub(crate) fn split(raw: &str) -> (&str, Option<&str>) {
    match raw.split_once(SEPARATOR) {
        Some((target, path)) => (target, Some(path)),
        None => (raw, None),
    }
}

/// Reads the revision of `/doc/<revision>/-/<path>`, still percent-encoded.
pub(crate) fn revision(raw: &str) -> Result<Revision, BadAddress> {
    let text = decode_text(raw, "revision")?;
    Ok(if text == WORKTREE {
        Revision::Worktree
    } else {
        Revision::Name(text)
    })
}

/// Reads the range of `/changes/<range>`, still percent-encoded.
///
/// The forms are those of `driftmark diff`: `<base>..<head>`,
/// `<base>...<head>` and `auto`. `--since` becomes `since/<date>`, for the
/// current branch, or `since/<date>/<branch>`.
pub(crate) fn range(raw: &str) -> Result<RangeAddress, BadAddress> {
    range_text(decode_text(raw, "range")?)
}

/// Reads a range written as plain text, in the forms of [`range`]. The MCP
/// connector takes ranges this way, never percent-encoded.
pub(crate) fn range_text(label: String) -> Result<RangeAddress, BadAddress> {
    let since = label
        .strip_prefix("since/")
        .map(|rest| match rest.split_once('/') {
            Some((date, branch)) => (date, Some(branch)),
            None => (rest, None),
        });
    let spec = match since {
        Some((date, branch)) => RangeSpec::since(date, branch),
        None => RangeSpec::parse(&label),
    };
    let spec = spec.map_err(|error| match error {
        RangeError::NotARange { .. } => BadAddress(format!(
            "`{label}` is not a range; write `<base>..<head>`, `<base>...<head>`, `auto` or `since/<date>/<branch>`"
        )),
        other => BadAddress(other.to_string()),
    })?;
    Ok(RangeAddress { label, spec })
}

/// Reads a document path, still percent-encoded. Only a Markdown file that
/// git could store is accepted.
pub(crate) fn document_path(raw: &str) -> Result<RepoPath, BadAddress> {
    markdown_path(percent_decode_str(raw).collect())
}

/// Reads a document path given as bytes, already decoded. Only a Markdown
/// file that git could store is accepted.
pub(crate) fn markdown_path(bytes: Vec<u8>) -> Result<RepoPath, BadAddress> {
    let shown = String::from_utf8_lossy(&bytes).into_owned();
    let path = RepoPath::new(bytes).ok_or_else(|| {
        BadAddress(format!(
            "`{shown}` is not a path inside the repository; it cannot be empty or hold `.` or `..` parts"
        ))
    })?;
    if path.is_markdown() {
        Ok(path)
    } else {
        Err(BadAddress(format!(
            "`{shown}` is not a Markdown file; only `.md` and `.markdown` files are shown"
        )))
    }
}

fn decode_text(raw: &str, what: &str) -> Result<String, BadAddress> {
    percent_decode_str(raw)
        .decode_utf8()
        .map(std::borrow::Cow::into_owned)
        .map_err(|_| BadAddress(format!("the {what} in the address is not valid text")))
}

/// The address of a document at a revision.
pub(crate) fn document(revision: &str, path: &RepoPath) -> String {
    format!("/doc/{}{SEPARATOR}{}", target(revision), encode_path(path))
}

/// The address of a revision's status view.
pub(crate) fn lifecycle(revision: &str) -> String {
    format!("/lifecycle/{}", target(revision))
}

/// The address of a range's change list.
pub(crate) fn changes(range: &str) -> String {
    format!("/changes/{}", target(range))
}

/// The address of one document's changes in a range.
pub(crate) fn document_changes(range: &str, path: &RepoPath) -> String {
    format!("/changes/{}{SEPARATOR}{}", target(range), encode_path(path))
}

/// Puts `page` under `/api`, where the same address answers with JSON.
pub(crate) fn json(page: &str) -> String {
    format!("/api{page}")
}

/// A revision or a range, escaped so it reads back the same. A part that is
/// exactly `-` is escaped too, so it cannot be taken for the separator.
fn target(text: &str) -> String {
    let encoded = utf8_percent_encode(text, KEPT).to_string();
    encoded
        .split('/')
        .map(|part| if part == "-" { "%2D" } else { part })
        .collect::<Vec<_>>()
        .join("/")
}

fn encode_path(path: &RepoPath) -> String {
    percent_encoding::percent_encode(path.as_bytes(), KEPT).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(text: &str) -> RepoPath {
        RepoPath::new(text).unwrap()
    }

    #[test]
    fn addresses_split_at_the_first_separator() {
        assert_eq!(split("main/-/docs/a.md"), ("main", Some("docs/a.md")));
        assert_eq!(
            split("feature/login/-/docs/-/a.md"),
            ("feature/login", Some("docs/-/a.md"))
        );
        assert_eq!(split("main...feature"), ("main...feature", None));
    }

    #[test]
    fn written_addresses_read_back() {
        let odd = path("docs/caf\u{e9} notes/a#1?.md");
        let address = document("feature/x-1", &odd);
        assert_eq!(
            address,
            "/doc/feature/x-1/-/docs/caf%C3%A9%20notes/a%231%3F.md"
        );
        let rest = address.strip_prefix("/doc/").unwrap();
        let (target, raw_path) = split(rest);
        assert_eq!(revision(target), Ok(Revision::Name("feature/x-1".into())));
        assert_eq!(document_path(raw_path.unwrap()), Ok(odd.clone()));

        let tricky = "team/-/x^{commit}";
        let address = document(tricky, &path("a.md"));
        assert_eq!(address, "/doc/team/%2D/x%5E%7Bcommit%7D/-/a.md");
        let (target, _) = split(address.strip_prefix("/doc/").unwrap());
        assert_eq!(revision(target), Ok(Revision::Name(tricky.into())));

        assert_eq!(changes("main...feature"), "/changes/main...feature");
        assert_eq!(
            document_changes("since/2026-10-01/main", &path("a.md")),
            "/changes/since/2026-10-01/main/-/a.md"
        );
        assert_eq!(json("/changes/auto"), "/api/changes/auto");
    }

    #[cfg(unix)]
    #[test]
    fn paths_that_are_not_utf8_survive_the_round_trip() {
        let latin1 = RepoPath::new(b"caf\xe9.md".to_vec()).unwrap();
        let address = document("main", &latin1);
        assert!(address.ends_with("/-/caf%E9.md"), "{address}");
        assert_eq!(document_path("caf%E9.md"), Ok(latin1));
    }

    #[test]
    fn worktree_is_a_word_and_other_names_are_revisions() {
        assert_eq!(revision("worktree"), Ok(Revision::Worktree));
        assert_eq!(revision("worktree").unwrap().label(), "worktree");
        assert_eq!(revision("main%7E2").unwrap().label(), "main~2");
        assert_eq!(
            revision("%FF"),
            Err(BadAddress(
                "the revision in the address is not valid text".into()
            ))
        );
    }

    #[test]
    fn ranges_take_the_diff_forms_and_since() {
        let read = |raw: &str| range(raw).map(|r| (r.label, r.spec));
        assert_eq!(
            read("main...feature/x"),
            Ok((
                "main...feature/x".into(),
                RangeSpec::FromMergeBase {
                    base: "main".into(),
                    head: "feature/x".into()
                }
            ))
        );
        assert_eq!(read("auto").unwrap().1, RangeSpec::Auto);
        assert_eq!(
            read("since/2026-10-01").unwrap().1,
            RangeSpec::since("2026-10-01", None).unwrap()
        );
        assert_eq!(
            read("since/2026-10-01/origin/dev").unwrap().1,
            RangeSpec::since("2026-10-01", Some("origin/dev")).unwrap()
        );
        assert!(
            range("main")
                .unwrap_err()
                .0
                .starts_with("`main` is not a range; write"),
            "a name alone"
        );
        assert!(
            range("since/last-week")
                .unwrap_err()
                .0
                .starts_with("`last-week` is not a date")
        );
        assert!(range("%C3").unwrap_err().0.contains("not valid text"));
    }

    #[test]
    fn only_markdown_paths_inside_the_repository_are_documents() {
        assert_eq!(document_path("docs/A.MD"), Ok(path("docs/A.MD")));
        for (raw, why) in [
            ("../etc/passwd.md", "is not a path inside"),
            ("docs/%2E%2E/%2E%2E/x.md", "is not a path inside"),
            ("docs//a.md", "is not a path inside"),
            ("/abs.md", "is not a path inside"),
            ("a%00b.md", "is not a path inside"),
            ("", "is not a path inside"),
            ("docs/notes.txt", "is not a Markdown file"),
            (".git/config", "is not a Markdown file"),
        ] {
            let error = document_path(raw).unwrap_err().0;
            assert!(error.contains(why), "{raw}: {error}");
        }
    }
}
