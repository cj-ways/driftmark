//! Ranges: the two git points a change view compares.

use std::cmp::Ordering;
use std::fmt;

use super::{
    CommitInfo, FileChange, FileEntry, GitError, GitRead, ObjectId, Roots, Snapshot, parse_date,
};

/// The branches `auto` compares with, in this order. The first that exists is
/// used. Remote branches come before local ones because a local `main` is
/// often behind.
pub const AUTO_BASES: [&str; 6] = [
    "origin/main",
    "main",
    "origin/master",
    "master",
    "origin/dev",
    "dev",
];

/// A range as the user wrote it, before git is asked anything.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RangeSpec {
    /// `<base>..<head>`: compare the two points directly, like `git diff A..B`.
    Direct {
        /// The older point, as written; `HEAD` when left out.
        base: String,
        /// The newer point, as written; `HEAD` when left out.
        head: String,
    },
    /// `<base>...<head>`: compare `head` with the commit where it left `base`
    /// (their merge-base), like `git diff A...B`. This shows only what `head`
    /// added, not what `base` gained since.
    FromMergeBase {
        /// The branch `head` started from, as written; `HEAD` when left out.
        base: String,
        /// The newer point, as written; `HEAD` when left out.
        head: String,
    },
    /// `--since <date>`: everything that reached `branch` from that date on.
    Since {
        /// The date, in seconds since 1970-01-01 00:00 UTC.
        since: i64,
        /// The branch, as written; `HEAD` when left out.
        branch: String,
    },
    /// `auto`: this branch's work since it left the default branch, including
    /// changes not committed yet.
    Auto,
}

impl RangeSpec {
    /// Reads `<base>..<head>`, `<base>...<head>` or `auto`.
    ///
    /// Either side may be left out and then means `HEAD`, as in git: `main..`
    /// is `main..HEAD`. The word `auto` always means the `auto` range, even if
    /// a branch has that name.
    ///
    /// # Errors
    ///
    /// [`RangeError::NotARange`] for anything else, such as one name alone,
    /// `..` alone, or a stray extra dot.
    pub fn parse(text: &str) -> Result<Self, RangeError> {
        if text == "auto" {
            return Ok(Self::Auto);
        }
        let not_a_range = || RangeError::NotARange {
            text: text.to_owned(),
        };
        let (base, head, from_merge_base) = if let Some((base, head)) = text.split_once("...") {
            (base, head, true)
        } else {
            let (base, head) = text.split_once("..").ok_or_else(not_a_range)?;
            (base, head, false)
        };
        let stray_dot = head.starts_with('.') || base.contains("..") || head.contains("..");
        if stray_dot || (base.is_empty() && head.is_empty()) {
            return Err(not_a_range());
        }
        let or_head = |side: &str| if side.is_empty() { "HEAD" } else { side }.to_owned();
        let (base, head) = (or_head(base), or_head(head));
        Ok(if from_merge_base {
            Self::FromMergeBase { base, head }
        } else {
            Self::Direct { base, head }
        })
    }

    /// The `--since <date>` range on `branch`, or on `HEAD` when no branch is
    /// given. [`parse_date`] says which dates are accepted.
    ///
    /// # Errors
    ///
    /// [`RangeError::BadDate`] when the date cannot be read.
    pub fn since(date: &str, branch: Option<&str>) -> Result<Self, RangeError> {
        let since = parse_date(date).ok_or_else(|| RangeError::BadDate {
            text: date.to_owned(),
        })?;
        Ok(Self::Since {
            since,
            branch: branch.unwrap_or("HEAD").to_owned(),
        })
    }

    /// Asks git which commits the range means.
    ///
    /// - `A..B`: base `A`, head `B`.
    /// - `A...B`: base is the merge-base of `A` and `B`, head `B`.
    /// - `--since D` on branch `B`: walking back from `B` along first parents,
    ///   the base is the first commit made before `D`, so every commit from `D`
    ///   on is inside the range. When the whole history is from `D` on, there is
    ///   no base and every file counts as added.
    /// - `auto`: base is the merge-base of `HEAD` with the first of
    ///   [`AUTO_BASES`] that exists; head is the working tree.
    ///
    /// # Errors
    ///
    /// A [`RangeError`] saying which step failed.
    pub fn resolve(&self, git: &dyn GitRead) -> Result<ResolvedRange, RangeError> {
        match self {
            Self::Direct { base, head } => Ok(ResolvedRange {
                base: Some(git.resolve(base)?),
                head: Snapshot::Commit(git.resolve(head)?),
                notes: Vec::new(),
            }),
            Self::FromMergeBase { base, head } => {
                let (base, head) = (git.resolve(base)?, git.resolve(head)?);
                let (base, notes) = merge_base(git, &base, &head)?;
                Ok(ResolvedRange {
                    base: Some(base),
                    head: Snapshot::Commit(head),
                    notes,
                })
            }
            Self::Since { since, branch } => {
                let tip = git.resolve(branch)?;
                let base = last_commit_before(git, &tip, *since)?;
                Ok(ResolvedRange {
                    base,
                    head: Snapshot::Commit(tip),
                    notes: Vec::new(),
                })
            }
            Self::Auto => {
                let head = git.resolve("HEAD")?;
                let (name, default) = first_default_branch(git)?;
                let (base, mut notes) = merge_base(git, &head, &default)?;
                notes.insert(0, RangeNote::AutoBase { name });
                Ok(ResolvedRange {
                    base: Some(base),
                    head: Snapshot::Worktree,
                    notes,
                })
            }
        }
    }
}

/// A range turned into git points.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedRange {
    /// The older side. `None` means an empty history, before the first commit.
    pub base: Option<ObjectId>,
    /// The newer side.
    pub head: Snapshot,
    /// What the reader should be told about how the points were chosen.
    pub notes: Vec<RangeNote>,
}

impl ResolvedRange {
    /// The Markdown files under `roots` that differ between the two sides,
    /// sorted by path.
    ///
    /// # Errors
    ///
    /// Any error from the git reader.
    pub fn changes(&self, git: &dyn GitRead, roots: &Roots) -> Result<Vec<FileChange>, GitError> {
        match (&self.base, &self.head) {
            (None, head) => Ok(diff_files(&[], &git.markdown_files(head, roots)?)),
            (Some(base), Snapshot::Commit(head)) => git.markdown_changes(base, head, roots),
            (Some(base), Snapshot::Worktree) => {
                let old = git.markdown_files(&Snapshot::Commit(base.clone()), roots)?;
                let new = git.markdown_files(&Snapshot::Worktree, roots)?;
                Ok(diff_files(&old, &new))
            }
        }
    }
}

/// Something about a resolved range the reader should know.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RangeNote {
    /// `auto` compared with this branch, the first of [`AUTO_BASES`] that
    /// exists.
    AutoBase {
        /// The branch.
        name: &'static str,
    },
    /// The two commits have several best common ancestors, which happens
    /// after criss-cross merges. The one with the lowest id was used, so the
    /// choice is stable, but another one would show different changes.
    SeveralMergeBases {
        /// The merge-base used.
        used: ObjectId,
        /// The others, by id.
        others: Vec<ObjectId>,
    },
}

impl fmt::Display for RangeNote {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AutoBase { name } => write!(f, "compared with `{name}`"),
            Self::SeveralMergeBases { used, others } => {
                let others: Vec<&str> = others.iter().map(ObjectId::as_hex).collect();
                let others = others.join(" ");
                write!(
                    f,
                    "the branches have several merge-bases; used {used}, not {others}"
                )
            }
        }
    }
}

/// Why a range could not be read or turned into git points.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RangeError {
    /// The text is none of the range forms.
    NotARange {
        /// The text as given.
        text: String,
    },
    /// The `--since` date is in neither accepted form.
    BadDate {
        /// The date as given.
        text: String,
    },
    /// The two commits share no history.
    NoMergeBase {
        /// One commit.
        a: ObjectId,
        /// The other commit.
        b: ObjectId,
    },
    /// `auto` found none of [`AUTO_BASES`].
    NoDefaultBranch,
    /// A commit's bytes are not a commit git would write.
    BadCommit {
        /// The commit.
        id: ObjectId,
    },
    /// The git reader could not answer.
    Git(GitError),
}

impl From<GitError> for RangeError {
    fn from(error: GitError) -> Self {
        Self::Git(error)
    }
}

impl fmt::Display for RangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotARange { text } => write!(
                f,
                "`{text}` is not a range; write `<base>..<head>`, `<base>...<head>` or `auto`"
            ),
            Self::BadDate { text } => write!(
                f,
                "`{text}` is not a date; write `2026-10-01` or `2026-10-01T09:30:00+04:00`"
            ),
            Self::NoMergeBase { a, b } => {
                write!(
                    f,
                    "commits {a} and {b} share no history, so they have no merge-base"
                )
            }
            Self::NoDefaultBranch => write!(
                f,
                "`auto` found no default branch; it looks for {}",
                AUTO_BASES.join(", ")
            ),
            Self::BadCommit { id } => write!(f, "commit {id} could not be read"),
            Self::Git(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for RangeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Git(error) => Some(error),
            _ => None,
        }
    }
}

/// The changes between two file lists, each sorted by path. The `auto` range
/// uses it to compare a commit with the working tree; readers use it to
/// compare their own lists.
#[must_use]
pub fn diff_files(old: &[FileEntry], new: &[FileEntry]) -> Vec<FileChange> {
    let (mut old, mut new) = (old.iter().peekable(), new.iter().peekable());
    let mut changes = Vec::new();
    loop {
        let order = match (old.peek(), new.peek()) {
            (None, None) => return changes,
            (Some(o), Some(n)) => o.path.cmp(&n.path),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
        };
        let o = if order.is_le() { old.next() } else { None };
        let n = if order.is_ge() { new.next() } else { None };
        changes.extend(change(o, n));
    }
}

/// The change between two entries for one path, either of which may be
/// missing.
fn change(old: Option<&FileEntry>, new: Option<&FileEntry>) -> Option<FileChange> {
    let path = new.or(old)?.path.clone();
    FileChange::between(path, old.map(|e| e.id.clone()), new.map(|e| e.id.clone()))
}

fn merge_base(
    git: &dyn GitRead,
    a: &ObjectId,
    b: &ObjectId,
) -> Result<(ObjectId, Vec<RangeNote>), RangeError> {
    let mut bases = git.merge_bases(a, b)?;
    bases.sort();
    bases.dedup();
    let mut bases = bases.into_iter();
    let used = bases.next().ok_or_else(|| RangeError::NoMergeBase {
        a: a.clone(),
        b: b.clone(),
    })?;
    let others: Vec<ObjectId> = bases.collect();
    let notes = if others.is_empty() {
        Vec::new()
    } else {
        vec![RangeNote::SeveralMergeBases {
            used: used.clone(),
            others,
        }]
    };
    Ok((used, notes))
}

fn first_default_branch(git: &dyn GitRead) -> Result<(&'static str, ObjectId), RangeError> {
    for name in AUTO_BASES {
        match git.resolve(name) {
            Ok(id) => return Ok((name, id)),
            Err(GitError::UnknownRevision { .. }) => {}
            Err(other) => return Err(other.into()),
        }
    }
    Err(RangeError::NoDefaultBranch)
}

/// Walks back from `tip` along first parents to the first commit made before
/// `since`. `None` when every commit is from `since` on.
fn last_commit_before(
    git: &dyn GitRead,
    tip: &ObjectId,
    since: i64,
) -> Result<Option<ObjectId>, RangeError> {
    let mut id = tip.clone();
    loop {
        let raw = git.read_commit(&id)?;
        let info =
            CommitInfo::parse(&raw).ok_or_else(|| RangeError::BadCommit { id: id.clone() })?;
        if info.committed_at < since {
            return Ok(Some(id));
        }
        match info.parents.into_iter().next() {
            Some(parent) => id = parent,
            None => return Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::{FakeGit, entry, id};
    use super::super::{FileEntry, RepoPath};
    use super::*;

    fn spec(text: &str) -> RangeSpec {
        RangeSpec::parse(text).unwrap()
    }

    fn direct(base: &str, head: &str) -> RangeSpec {
        RangeSpec::Direct {
            base: base.into(),
            head: head.into(),
        }
    }

    fn from_merge_base(base: &str, head: &str) -> RangeSpec {
        RangeSpec::FromMergeBase {
            base: base.into(),
            head: head.into(),
        }
    }

    #[test]
    fn two_dots_compare_directly() {
        assert_eq!(spec("main..feature"), direct("main", "feature"));
        assert_eq!(spec("v1.0..v2.0"), direct("v1.0", "v2.0"));
        assert_eq!(spec("main.."), direct("main", "HEAD"));
        assert_eq!(spec("..feature"), direct("HEAD", "feature"));
        assert_eq!(spec("HEAD~3..HEAD"), direct("HEAD~3", "HEAD"));
    }

    #[test]
    fn three_dots_start_from_the_merge_base() {
        assert_eq!(spec("main...feature"), from_merge_base("main", "feature"));
        assert_eq!(spec("main..."), from_merge_base("main", "HEAD"));
        assert_eq!(spec("...feature"), from_merge_base("HEAD", "feature"));
    }

    #[test]
    fn auto_is_a_word_of_its_own() {
        assert_eq!(spec("auto"), RangeSpec::Auto);
        assert_eq!(spec("auto.."), direct("auto", "HEAD"));
    }

    #[test]
    fn other_text_is_not_a_range() {
        for text in [
            "", "main", "..", "...", "....", "a....b", "a..b..c", "a..b...c", "Auto",
        ] {
            let error = RangeSpec::parse(text).unwrap_err();
            assert_eq!(error, RangeError::NotARange { text: text.into() }, "{text}");
        }
    }

    #[test]
    fn since_reads_the_date_and_defaults_to_head() {
        assert_eq!(
            RangeSpec::since("2026-10-01", None).unwrap(),
            RangeSpec::Since {
                since: 1_790_812_800,
                branch: "HEAD".into()
            }
        );
        assert_eq!(
            RangeSpec::since("2026-10-01T04:00:00+04:00", Some("main")).unwrap(),
            RangeSpec::Since {
                since: 1_790_812_800,
                branch: "main".into()
            }
        );
        assert_eq!(
            RangeSpec::since("last week", None).unwrap_err(),
            RangeError::BadDate {
                text: "last week".into()
            }
        );
    }

    #[test]
    fn direct_ranges_resolve_both_names() {
        let git = FakeGit::new().name("main", '1').name("feature", '2');
        let range = direct("main", "feature").resolve(&git).unwrap();
        assert_eq!(
            range,
            ResolvedRange {
                base: Some(id('1')),
                head: Snapshot::Commit(id('2')),
                notes: vec![]
            }
        );
        let unknown = direct("main", "nope").resolve(&git).unwrap_err();
        assert_eq!(
            unknown,
            RangeError::Git(GitError::UnknownRevision {
                name: "nope".into()
            })
        );
        let unknown_base = direct("nope", "main").resolve(&git).unwrap_err();
        assert_eq!(
            unknown_base,
            RangeError::Git(GitError::UnknownRevision {
                name: "nope".into()
            })
        );
    }

    #[test]
    fn merge_base_ranges_use_the_lowest_of_several_bases() {
        let git = FakeGit::new()
            .name("main", '1')
            .name("feature", '2')
            .bases(&['7']);
        let range = from_merge_base("main", "feature").resolve(&git).unwrap();
        assert_eq!(
            range,
            ResolvedRange {
                base: Some(id('7')),
                head: Snapshot::Commit(id('2')),
                notes: vec![]
            }
        );

        let git = FakeGit::new()
            .name("main", '1')
            .name("feature", '2')
            .bases(&['9', '7', '8', '9']);
        let range = from_merge_base("main", "feature").resolve(&git).unwrap();
        let note = RangeNote::SeveralMergeBases {
            used: id('7'),
            others: vec![id('8'), id('9')],
        };
        assert_eq!(range.base, Some(id('7')));
        assert_eq!(range.notes, std::slice::from_ref(&note));
        assert_eq!(
            note.to_string(),
            format!(
                "the branches have several merge-bases; used {}, not {} {}",
                id('7'),
                id('8'),
                id('9')
            )
        );

        let git = FakeGit::new().name("main", '1').name("feature", '2');
        let error = from_merge_base("main", "feature")
            .resolve(&git)
            .unwrap_err();
        assert_eq!(
            error,
            RangeError::NoMergeBase {
                a: id('1'),
                b: id('2')
            }
        );

        let git = FakeGit::new()
            .name("main", '1')
            .name("feature", '2')
            .bases_fail();
        let error = from_merge_base("main", "feature")
            .resolve(&git)
            .unwrap_err();
        assert_eq!(
            error,
            RangeError::Git(GitError::Other {
                message: "no bases".into()
            })
        );
    }

    #[test]
    fn merge_base_ranges_need_both_names() {
        let git = FakeGit::new().name("main", '1').bases(&['7']);
        for (base, head) in [("main", "nope"), ("nope", "main")] {
            let error = from_merge_base(base, head).resolve(&git).unwrap_err();
            assert_eq!(
                error,
                RangeError::Git(GitError::UnknownRevision {
                    name: "nope".into()
                })
            );
        }
    }

    /// A first-parent line `a` (oldest) to `d` (tip), with a side commit `f`
    /// merged as the second parent of `d`.
    fn history() -> FakeGit {
        FakeGit::new()
            .name("main", 'd')
            .name("HEAD", 'd')
            .commit('a', &[], 100)
            .commit('b', &['a'], 200)
            .commit('c', &['b'], 300)
            .commit('f', &['b'], 50)
            .commit('d', &['c', 'f'], 400)
    }

    fn since(at: i64, branch: &str) -> RangeSpec {
        RangeSpec::Since {
            since: at,
            branch: branch.into(),
        }
    }

    #[test]
    fn since_takes_the_first_commit_before_the_date_as_base() {
        let git = history();
        let base = |at: i64| since(at, "main").resolve(&git).unwrap().base;
        assert_eq!(base(250), Some(id('b')));
        assert_eq!(
            base(300),
            Some(id('b')),
            "a commit made at the date is inside"
        );
        assert_eq!(base(301), Some(id('c')));
        assert_eq!(
            base(401),
            Some(id('d')),
            "nothing since the date: an empty range"
        );
        assert_eq!(base(100), None, "the whole history is inside");
        assert_eq!(base(60), None, "a second parent is never walked");
        let range = since(250, "HEAD").resolve(&git).unwrap();
        assert_eq!(range.head, Snapshot::Commit(id('d')));
        assert_eq!(range.notes, []);
    }

    #[test]
    fn since_reports_unknown_branches_missing_and_broken_commits() {
        let error = since(250, "nope").resolve(&history()).unwrap_err();
        assert_eq!(
            error,
            RangeError::Git(GitError::UnknownRevision {
                name: "nope".into()
            })
        );

        let shallow = FakeGit::new().name("main", 'c').commit('c', &['b'], 300);
        let error = since(250, "main").resolve(&shallow).unwrap_err();
        assert_eq!(
            error,
            RangeError::Git(GitError::MissingObject { id: id('b') })
        );

        let broken = FakeGit::new()
            .name("main", 'c')
            .raw_commit('c', b"tree x\n");
        let error = since(250, "main").resolve(&broken).unwrap_err();
        assert_eq!(error, RangeError::BadCommit { id: id('c') });
    }

    #[test]
    fn auto_compares_the_working_tree_with_the_first_default_branch() {
        let git = FakeGit::new()
            .name("HEAD", '1')
            .name("master", '2')
            .name("dev", '3')
            .bases(&['5']);
        let range = RangeSpec::Auto.resolve(&git).unwrap();
        assert_eq!(range.base, Some(id('5')));
        assert_eq!(range.head, Snapshot::Worktree);
        assert_eq!(range.notes, [RangeNote::AutoBase { name: "master" }]);
        assert_eq!(range.notes[0].to_string(), "compared with `master`");

        let git = FakeGit::new()
            .name("HEAD", '1')
            .name("origin/main", '2')
            .bases(&['6', '5']);
        let range = RangeSpec::Auto.resolve(&git).unwrap();
        assert_eq!(range.notes.len(), 2);
        assert_eq!(
            range.notes[0],
            RangeNote::AutoBase {
                name: "origin/main"
            }
        );
    }

    #[test]
    fn auto_stops_at_the_first_real_error() {
        let none = FakeGit::new().name("HEAD", '1');
        assert_eq!(
            RangeSpec::Auto.resolve(&none).unwrap_err(),
            RangeError::NoDefaultBranch
        );

        let unborn = FakeGit::new().name("main", '1');
        let error = RangeSpec::Auto.resolve(&unborn).unwrap_err();
        assert_eq!(
            error,
            RangeError::Git(GitError::UnknownRevision {
                name: "HEAD".into()
            })
        );

        let broken = FakeGit::new()
            .name("HEAD", '1')
            .name("main", '2')
            .fail_name("origin/main");
        let error = RangeSpec::Auto.resolve(&broken).unwrap_err();
        assert_eq!(
            error,
            RangeError::Git(GitError::Other {
                message: "broken ref".into()
            })
        );

        let unrelated = FakeGit::new().name("HEAD", '1').name("main", '2');
        let error = RangeSpec::Auto.resolve(&unrelated).unwrap_err();
        assert_eq!(
            error,
            RangeError::NoMergeBase {
                a: id('1'),
                b: id('2')
            }
        );
    }

    fn path(p: &str) -> RepoPath {
        RepoPath::new(p).unwrap()
    }

    #[test]
    fn file_lists_diff_by_path() {
        let old = [
            entry("a.md", '1'),
            entry("b.md", '2'),
            entry("d.md", '4'),
            entry("e.md", '5'),
        ];
        let new = [
            entry("b.md", '2'),
            entry("c.md", '3'),
            entry("d.md", '6'),
            entry("f.md", '7'),
        ];
        assert_eq!(
            diff_files(&old, &new),
            [
                FileChange::Removed {
                    path: path("a.md"),
                    old: id('1')
                },
                FileChange::Added {
                    path: path("c.md"),
                    new: id('3')
                },
                FileChange::Modified {
                    path: path("d.md"),
                    old: id('4'),
                    new: id('6')
                },
                FileChange::Removed {
                    path: path("e.md"),
                    old: id('5')
                },
                FileChange::Added {
                    path: path("f.md"),
                    new: id('7')
                },
            ]
        );
        assert_eq!(diff_files(&[], &[]), []);
        assert_eq!(change(None, None), None);
    }

    #[test]
    fn changes_follow_the_kind_of_range() {
        let docs = Roots::new(["docs"]).unwrap();
        let git = FakeGit::new()
            .files(
                '1',
                &[
                    entry("docs/a.md", 'a'),
                    entry("docs/b.md", 'b'),
                    entry("x.md", '8'),
                ],
            )
            .files('2', &[entry("docs/a.md", 'c'), entry("x.md", '9')])
            .worktree(&[entry("docs/a.md", 'a'), entry("docs/new.md", 'e')]);

        let commits = ResolvedRange {
            base: Some(id('1')),
            head: Snapshot::Commit(id('2')),
            notes: vec![],
        };
        assert_eq!(
            commits.changes(&git, &docs).unwrap(),
            [
                FileChange::Modified {
                    path: path("docs/a.md"),
                    old: id('a'),
                    new: id('c')
                },
                FileChange::Removed {
                    path: path("docs/b.md"),
                    old: id('b')
                },
            ]
        );

        let worktree = ResolvedRange {
            base: Some(id('1')),
            head: Snapshot::Worktree,
            notes: vec![],
        };
        assert_eq!(
            worktree.changes(&git, &docs).unwrap(),
            [
                FileChange::Removed {
                    path: path("docs/b.md"),
                    old: id('b')
                },
                FileChange::Added {
                    path: path("docs/new.md"),
                    new: id('e')
                },
            ]
        );

        let from_nothing = ResolvedRange {
            base: None,
            head: Snapshot::Commit(id('2')),
            notes: vec![],
        };
        assert_eq!(
            from_nothing.changes(&git, &Roots::all()).unwrap(),
            [
                FileChange::Added {
                    path: path("docs/a.md"),
                    new: id('c')
                },
                FileChange::Added {
                    path: path("x.md"),
                    new: id('9')
                },
            ]
        );
    }

    #[test]
    fn changes_pass_reader_errors_on() {
        let git = FakeGit::new().files('1', &[entry("a.md", 'a')]);
        let missing = GitError::Other {
            message: "no files".into(),
        };
        let cases = [
            ResolvedRange {
                base: Some(id('1')),
                head: Snapshot::Worktree,
                notes: vec![],
            },
            ResolvedRange {
                base: Some(id('9')),
                head: Snapshot::Worktree,
                notes: vec![],
            },
            ResolvedRange {
                base: None,
                head: Snapshot::Commit(id('9')),
                notes: vec![],
            },
            ResolvedRange {
                base: Some(id('9')),
                head: Snapshot::Commit(id('1')),
                notes: vec![],
            },
            ResolvedRange {
                base: Some(id('1')),
                head: Snapshot::Commit(id('9')),
                notes: vec![],
            },
        ];
        for range in cases {
            assert_eq!(
                range.changes(&git, &Roots::all()).unwrap_err(),
                missing,
                "{range:?}"
            );
        }
    }

    #[test]
    fn errors_say_what_to_write_instead() {
        let cases = [
            (
                RangeError::NotARange {
                    text: "main".into(),
                },
                "`main` is not a range; write `<base>..<head>`, `<base>...<head>` or `auto`"
                    .to_owned(),
            ),
            (
                RangeError::BadDate {
                    text: "soon".into(),
                },
                "`soon` is not a date; write `2026-10-01` or `2026-10-01T09:30:00+04:00`"
                    .to_owned(),
            ),
            (
                RangeError::NoMergeBase {
                    a: id('1'),
                    b: id('2'),
                },
                format!(
                    "commits {} and {} share no history, so they have no merge-base",
                    id('1'),
                    id('2')
                ),
            ),
            (
                RangeError::NoDefaultBranch,
                "`auto` found no default branch; it looks for origin/main, main, origin/master, \
                 master, origin/dev, dev"
                    .to_owned(),
            ),
            (
                RangeError::BadCommit { id: id('3') },
                format!("commit {} could not be read", id('3')),
            ),
            (
                RangeError::Git(GitError::NoWorktree),
                "the repo has no working tree".to_owned(),
            ),
        ];
        for (error, text) in cases {
            assert_eq!(error.to_string(), text);
            let has_source = matches!(error, RangeError::Git(_));
            assert_eq!(std::error::Error::source(&error).is_some(), has_source);
        }
    }

    #[test]
    fn file_entries_compare_by_path_then_id() {
        let a: FileEntry = entry("a.md", '1');
        assert_eq!(a, entry("a.md", '1'));
        assert_ne!(a, entry("a.md", '2'));
    }
}
