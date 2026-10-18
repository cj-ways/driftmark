//! Paths inside a repo, and the folders driftmark reads.

use std::borrow::Cow;
use std::fmt;

/// A path inside the repo, as git stores it: bytes with `/` between folders,
/// relative to the repo's top folder.
///
/// Git allows any byte but NUL in a name, so this is not a `String`. A file
/// whose name is not UTF-8 still works; [`RepoPath::to_lossy`] shows it.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RepoPath(Vec<u8>);

impl RepoPath {
    /// Checks a path the way git checks the paths it stores: not empty, no NUL
    /// byte, no `/` at either end, and no empty, `.` or `..` part. Returns
    /// `None` otherwise, so a path can never climb out of the repo.
    #[must_use]
    pub fn new(path: impl Into<Vec<u8>>) -> Option<Self> {
        let path = path.into();
        let parts_ok = path
            .split(|&b| b == b'/')
            .all(|part| !matches!(part, b"" | b"." | b".."));
        (parts_ok && !path.contains(&0)).then_some(Self(path))
    }

    /// The path's bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// The path as text, with bytes that are not UTF-8 shown as `�`. For
    /// messages only; it may not lead back to the file.
    #[must_use]
    pub fn to_lossy(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.0)
    }

    /// Whether the file counts as Markdown: its name ends in `.md` or
    /// `.markdown`, in any mix of upper and lower case.
    ///
    /// Case is ignored because `README.MD` is common on systems that ignore
    /// case, and GitHub renders it as Markdown.
    #[must_use]
    pub fn is_markdown(&self) -> bool {
        ends_with_ignoring_case(&self.0, b".md") || ends_with_ignoring_case(&self.0, b".markdown")
    }

    /// Whether this path is `folder` itself or lies inside it.
    fn is_within(&self, folder: &Self) -> bool {
        let inner = self.0.strip_prefix(folder.0.as_slice());
        inner.is_some_and(|rest| rest.is_empty() || rest.starts_with(b"/"))
    }
}

impl fmt::Debug for RepoPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RepoPath({:?})", self.to_lossy())
    }
}

fn ends_with_ignoring_case(text: &[u8], end: &[u8]) -> bool {
    text.len() >= end.len() && text[text.len() - end.len()..].eq_ignore_ascii_case(end)
}

/// The folders driftmark reads. Files elsewhere in the repo are ignored.
///
/// No folders means the whole repo. A rules file can later narrow this to the
/// folders that hold documents, such as `docs`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Roots(Vec<RepoPath>);

impl Roots {
    /// The whole repo.
    #[must_use]
    pub fn all() -> Self {
        Self(Vec::new())
    }

    /// These folders, given relative to the repo's top folder, such as `docs`
    /// or `specs/api/`. A trailing `/` is allowed. No folders means the whole
    /// repo. A folder inside another one adds nothing and is dropped.
    ///
    /// # Errors
    ///
    /// [`BadRoot`] for a folder that is empty, starts with `/`, or has an
    /// empty, `.` or `..` part.
    pub fn new<I, S>(folders: I) -> Result<Self, BadRoot>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut roots = Vec::new();
        for folder in folders {
            let text = folder.as_ref();
            let trimmed = text.strip_suffix('/').unwrap_or(text);
            let path = RepoPath::new(trimmed).ok_or_else(|| BadRoot {
                text: text.to_owned(),
            })?;
            roots.push(path);
        }
        roots.sort();
        roots.dedup();
        let inside_another = |root: &RepoPath| {
            roots
                .iter()
                .any(|outer| outer != root && root.is_within(outer))
        };
        let kept = roots
            .iter()
            .filter(|root| !inside_another(root))
            .cloned()
            .collect();
        Ok(Self(kept))
    }

    /// The folders, sorted, none inside another. Empty means the whole repo.
    #[must_use]
    pub fn folders(&self) -> &[RepoPath] {
        &self.0
    }

    /// Whether a path lies inside one of the folders.
    #[must_use]
    pub fn contains(&self, path: &RepoPath) -> bool {
        self.0.is_empty()
            || self
                .0
                .iter()
                .any(|root| path.is_within(root) && path != root)
    }

    /// Whether driftmark reads this file: it is Markdown and inside the roots.
    /// Both git readers filter with this, so they pick the same files.
    #[must_use]
    pub fn selects(&self, path: &RepoPath) -> bool {
        path.is_markdown() && self.contains(path)
    }
}

/// A folder given to [`Roots::new`] that cannot name a folder in a repo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BadRoot {
    /// The folder as given.
    pub text: String,
}

impl fmt::Display for BadRoot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "`{}` is not a folder inside the repo; write it like `docs` or `specs/api`",
            self.text
        )
    }
}

impl std::error::Error for BadRoot {}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(p: &str) -> RepoPath {
        RepoPath::new(p).unwrap()
    }

    #[test]
    fn paths_follow_git_rules() {
        for good in [
            "a",
            "a.md",
            "docs/a.md",
            ".md",
            "..md",
            ".hidden/x",
            "a b/c\"d.md",
        ] {
            assert_eq!(
                RepoPath::new(good).unwrap().as_bytes(),
                good.as_bytes(),
                "{good}"
            );
        }
        for bad in [
            "", "/a", "a/", "a//b", "./a", "a/.", "../a", "a/../b", "a\0b",
        ] {
            assert_eq!(RepoPath::new(bad), None, "{bad:?}");
        }
        let not_utf8 = RepoPath::new(b"caf\xe9.md".to_vec()).unwrap();
        assert_eq!(not_utf8.to_lossy(), "caf\u{fffd}.md");
        assert_eq!(format!("{not_utf8:?}"), "RepoPath(\"caf\u{fffd}.md\")");
    }

    #[test]
    fn markdown_is_md_or_markdown_in_any_case() {
        for yes in ["a.md", "A.MD", "x/b.Md", "c.markdown", "d.MarkDown", ".md"] {
            assert!(path(yes).is_markdown(), "{yes}");
        }
        for no in [
            "md",
            "a.mdx",
            "a.txt",
            "a.md.bak",
            "amd",
            "a.markdow",
            "markdown",
        ] {
            assert!(!path(no).is_markdown(), "{no}");
        }
    }

    #[test]
    fn roots_are_cleaned_up() {
        assert_eq!(Roots::all(), Roots::new(Vec::<String>::new()).unwrap());
        assert_eq!(Roots::all().folders(), []);

        let roots = Roots::new(["specs/", "docs/adr", "docs", "docs/", "docs-old"]).unwrap();
        assert_eq!(
            roots.folders(),
            [path("docs"), path("docs-old"), path("specs")]
        );

        let nested_first = Roots::new(["a/b/c", "a/b", "a/bc"]).unwrap();
        assert_eq!(nested_first.folders(), [path("a/b"), path("a/bc")]);

        for bad in ["", "/", "/docs", "docs//", "./docs", "docs/..", "."] {
            let error = Roots::new(["docs", bad]).unwrap_err();
            assert_eq!(
                error,
                BadRoot {
                    text: bad.to_owned()
                }
            );
            assert_eq!(
                error.to_string(),
                format!(
                    "`{bad}` is not a folder inside the repo; write it like `docs` or `specs/api`"
                )
            );
        }
    }

    #[test]
    fn roots_hold_what_is_inside_them() {
        let all = Roots::all();
        assert!(all.contains(&path("a.md")));
        assert!(all.contains(&path("deep/down/b.md")));

        let docs = Roots::new(["docs", "specs/api"]).unwrap();
        for inside in ["docs/a.md", "docs/x/y.md", "specs/api/v1.md"] {
            assert!(docs.contains(&path(inside)), "{inside}");
        }
        for outside in [
            "docs",
            "docs.md",
            "docsx/a.md",
            "specs/a.md",
            "specs/api",
            "a.md",
        ] {
            assert!(!docs.contains(&path(outside)), "{outside}");
        }
    }

    #[test]
    fn selected_files_are_markdown_inside_the_roots() {
        let docs = Roots::new(["docs"]).unwrap();
        assert!(docs.selects(&path("docs/a.md")));
        assert!(!docs.selects(&path("docs/a.txt")));
        assert!(!docs.selects(&path("other/a.md")));
    }
}
