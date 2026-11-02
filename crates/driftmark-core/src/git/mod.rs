//! What driftmark asks of a git repo, and the rules for reading the answers.
//!
//! driftmark needs eight answers from git: which commit a name points to,
//! where two branches parted, which Markdown files exist at a commit, which of
//! them changed between two points, where one other file is, such as the rules
//! file, which branches exist, the bytes of a file, and when each file last
//! changed. [`GitRead`] states those questions. The `driftmark` program answers them twice, with gix and
//! with the `git` command, and checks the two against each other.
//!
//! Everything that is a rule rather than a lookup lives here, so both readers
//! follow the same rules: which files count as Markdown ([`RepoPath::is_markdown`]),
//! which folders are read ([`Roots`]), how a range is written ([`RangeSpec`]),
//! how a date is read ([`parse_date`]), how a commit object is read
//! ([`CommitInfo`]) and which branches the home page compares ([`inbox`]).

mod branches;
mod commit;
mod date;
mod history;
mod path;
mod range;

use std::fmt;

pub use branches::{BranchInbox, DefaultBranch, OpenBranch, inbox};
pub use commit::{CommitInfo, tree_of};
pub use date::parse_date;
pub use history::{LastChange, LastChanges, walk as walk_last_changes};
pub use path::{BadRoot, RepoPath, Roots};
pub use range::{
    AUTO_BASES, BaseBranch, RangeError, RangeNote, RangeSpec, ResolvedRange, diff_files, merge_base,
};

/// The id of a git object, in lowercase hex.
///
/// SHA-1 ids have 40 characters and SHA-256 ids have 64. Both are accepted so
/// this type does not change when driftmark learns SHA-256 repos; the gix
/// reader only opens SHA-1 repos today.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjectId(String);

impl ObjectId {
    /// Reads an id from lowercase hex. Returns `None` for anything else, so a
    /// reader that gets a garbled answer from git reports it instead of
    /// passing it on.
    #[must_use]
    pub fn from_hex(hex: &str) -> Option<Self> {
        let right_length = hex.len() == 40 || hex.len() == 64;
        let lower_hex = hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        (right_length && lower_hex).then(|| Self(hex.to_owned()))
    }

    /// The id as lowercase hex.
    #[must_use]
    pub fn as_hex(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One side of a comparison: a commit, or the files on disk now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Snapshot {
    /// The files as they were committed.
    Commit(ObjectId),
    /// The files in the working tree now, committed or not.
    ///
    /// This is how `auto` shows work an agent has not committed yet. See
    /// [`GitRead::markdown_files`] for which files it holds.
    Worktree,
}

/// A Markdown file at one snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    /// Where the file is.
    pub path: RepoPath,
    /// The blob that holds its bytes. For a working-tree file this is the id
    /// git would give the file if it were added now.
    pub id: ObjectId,
}

/// A Markdown file that differs between two snapshots.
///
/// Renames are not detected: a moved file is one `Removed` and one `Added`.
/// A change of file mode alone, with the same bytes, is not a change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileChange {
    /// The file exists only in the newer snapshot.
    Added {
        /// Where the file is.
        path: RepoPath,
        /// Its bytes in the newer snapshot.
        new: ObjectId,
    },
    /// The file exists only in the older snapshot.
    Removed {
        /// Where the file was.
        path: RepoPath,
        /// Its bytes in the older snapshot.
        old: ObjectId,
    },
    /// The file exists in both snapshots with different bytes.
    Modified {
        /// Where the file is.
        path: RepoPath,
        /// Its bytes in the older snapshot.
        old: ObjectId,
        /// Its bytes in the newer snapshot.
        new: ObjectId,
    },
}

impl FileChange {
    /// The change between two versions of one path, or `None` when nothing
    /// changed: the file is missing on both sides, or both sides hold the same
    /// blob. Both readers build every change through this, so they agree on
    /// what counts.
    #[must_use]
    pub fn between(path: RepoPath, old: Option<ObjectId>, new: Option<ObjectId>) -> Option<Self> {
        match (old, new) {
            (None, None) => None,
            (None, Some(new)) => Some(Self::Added { path, new }),
            (Some(old), None) => Some(Self::Removed { path, old }),
            (Some(old), Some(new)) if old == new => None,
            (Some(old), Some(new)) => Some(Self::Modified { path, old, new }),
        }
    }

    /// Where the file is, or was.
    #[must_use]
    pub fn path(&self) -> &RepoPath {
        match self {
            Self::Added { path, .. } | Self::Removed { path, .. } | Self::Modified { path, .. } => {
                path
            }
        }
    }

    /// The blob in the older snapshot, if the file existed there.
    #[must_use]
    pub fn old_blob(&self) -> Option<&ObjectId> {
        match self {
            Self::Added { .. } => None,
            Self::Removed { old, .. } | Self::Modified { old, .. } => Some(old),
        }
    }

    /// The blob in the newer snapshot, if the file exists there.
    #[must_use]
    pub fn new_blob(&self) -> Option<&ObjectId> {
        match self {
            Self::Removed { .. } => None,
            Self::Added { new, .. } | Self::Modified { new, .. } => Some(new),
        }
    }
}

/// A branch and the commit at its tip.
///
/// Both local branches (`refs/heads/…`) and remote-tracking branches
/// (`refs/remotes/…`) count, because an agent's work may sit in either: a
/// branch in a worktree on this machine, or one pushed from another.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Branch {
    /// The name git shows and resolves: `main` for a local branch,
    /// `origin/main` for a remote-tracking one.
    pub name: String,
    /// Whether the branch is remote-tracking.
    pub remote: bool,
    /// The commit at the tip.
    pub tip: ObjectId,
    /// Whether `HEAD` points at this branch: it is checked out in the working
    /// tree the reader opened.
    pub current: bool,
}

/// The kind of git object a question expected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectKind {
    /// File contents.
    Blob,
    /// A commit.
    Commit,
}

impl fmt::Display for ObjectKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Blob => "blob",
            Self::Commit => "commit",
        })
    }
}

/// Why a git question had no answer.
///
/// The first five cases mean the same thing whichever reader answered, so
/// callers can act on them. [`GitError::Other`] carries the reader's own words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitError {
    /// The name leads to no commit: no such branch, tag or id, or it names a
    /// tree or a file instead.
    UnknownRevision {
        /// The name as given.
        name: String,
    },
    /// An object the answer needs is not in the repo. Partial and shallow
    /// clones leave such gaps.
    MissingObject {
        /// The missing object.
        id: ObjectId,
    },
    /// The object exists but is of another kind than the question needs.
    WrongKind {
        /// The object.
        id: ObjectId,
        /// The kind the question needed.
        expected: ObjectKind,
    },
    /// The repo has no working tree (it is bare), so uncommitted files cannot
    /// be read.
    NoWorktree,
    /// The working tree has no regular file at this path.
    MissingFile {
        /// The path asked for.
        path: RepoPath,
    },
    /// Anything else, in the reader's own words.
    Other {
        /// What went wrong.
        message: String,
    },
}

impl fmt::Display for GitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownRevision { name } => write!(f, "`{name}` does not name a commit"),
            Self::MissingObject { id } => write!(f, "object {id} is not in the repo"),
            Self::WrongKind { id, expected } => write!(f, "object {id} is not a {expected}"),
            Self::NoWorktree => f.write_str("the repo has no working tree"),
            Self::MissingFile { path } => {
                write!(f, "the working tree has no file `{}`", path.to_lossy())
            }
            Self::Other { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for GitError {}

/// The questions driftmark asks a git repo.
///
/// Two readers answer them in the `driftmark` program: one with gix, one with
/// the `git` command. Every answer must be the same from both, and tests check
/// that. Implementations are shared between threads, because pages read many
/// files at once.
///
/// Lists come back sorted by path, in byte order, with each path once.
/// Only regular files count: symbolic links and submodules are skipped.
pub trait GitRead: Send + Sync {
    /// The commit a revision name points to.
    ///
    /// A name is anything git accepts: a branch, a tag, an id or part of one,
    /// or an expression such as `HEAD~2`. Tags are followed to their commit.
    ///
    /// # Errors
    ///
    /// [`GitError::UnknownRevision`] when the name leads to no commit.
    fn resolve(&self, name: &str) -> Result<ObjectId, GitError>;

    /// The best common ancestors of two commits, in any order.
    ///
    /// Usually there is one. There are none when the two histories share no
    /// commit, and several after criss-cross merges.
    ///
    /// # Errors
    ///
    /// When either commit cannot be read.
    fn merge_bases(&self, a: &ObjectId, b: &ObjectId) -> Result<Vec<ObjectId>, GitError>;

    /// The raw bytes of a commit object, as `git cat-file commit` prints them.
    /// [`CommitInfo::parse`] reads them.
    ///
    /// # Errors
    ///
    /// [`GitError::MissingObject`] when the commit is not in the repo, and
    /// [`GitError::WrongKind`] when the id names another kind of object.
    fn read_commit(&self, id: &ObjectId) -> Result<Vec<u8>, GitError>;

    /// Every Markdown file under `roots` at one snapshot.
    ///
    /// At a commit, these are the tracked files of that commit.
    ///
    /// In the working tree, these are the files on disk that git tracks (they
    /// are in the index) or would offer to track (untracked and not ignored).
    /// A tracked file missing from disk is left out. Each id is the blob id git
    /// would store if the file were added now: git's line-ending conversion and
    /// filters apply, so a file git calls unchanged keeps its committed id.
    ///
    /// # Errors
    ///
    /// [`GitError::NoWorktree`] for the working tree of a bare repo, and any
    /// error from reading the commit, its trees or the files.
    fn markdown_files(&self, at: &Snapshot, roots: &Roots) -> Result<Vec<FileEntry>, GitError>;

    /// The Markdown files under `roots` that differ between two commits.
    ///
    /// # Errors
    ///
    /// When either commit or one of its trees cannot be read.
    fn markdown_changes(
        &self,
        base: &ObjectId,
        head: &ObjectId,
        roots: &Roots,
    ) -> Result<Vec<FileChange>, GitError>;

    /// The blob of the regular file at `path` in a commit, or `None` when no
    /// regular file is there: nothing has that path, it is a folder, or it is
    /// a symbolic link or a submodule.
    ///
    /// The other questions only see Markdown files. This one finds any file,
    /// such as the rules file at the head of a range.
    ///
    /// # Errors
    ///
    /// When the commit or one of its trees cannot be read.
    fn blob_at(&self, commit: &ObjectId, path: &RepoPath) -> Result<Option<ObjectId>, GitError>;

    /// Every local branch, then every remote-tracking branch, each sorted by
    /// name in byte order.
    ///
    /// A symbolic ref such as `origin/HEAD` is left out: it only names
    /// another branch. So is a ref that cannot be read, one whose tip is not a
    /// commit, and one whose name is not UTF-8, because a name driftmark
    /// cannot write as text cannot be resolved again.
    ///
    /// # Errors
    ///
    /// When the refs cannot be listed at all.
    fn branches(&self) -> Result<Vec<Branch>, GitError>;

    /// The bytes of a blob, exactly as stored.
    ///
    /// # Errors
    ///
    /// [`GitError::MissingObject`] when the blob is not in the repo, and
    /// [`GitError::WrongKind`] when the id names another kind of object.
    fn read_blob(&self, id: &ObjectId) -> Result<Vec<u8>, GitError>;

    /// The bytes of a working-tree file, as they are on disk now.
    ///
    /// These can differ from the blob named by the file's id in
    /// [`GitRead::markdown_files`]: git's line-ending conversion applies to the
    /// id, not to these bytes. The file may also have changed since it was
    /// listed. So these bytes must never be cached under that id.
    ///
    /// # Errors
    ///
    /// [`GitError::NoWorktree`] in a bare repo, and [`GitError::MissingFile`]
    /// when no regular file is at that path. A path that passes through a
    /// symbolic link counts as missing, so nothing outside the repo is read.
    fn read_worktree_file(&self, path: &RepoPath) -> Result<Vec<u8>, GitError>;

    /// When each Markdown file under `roots` at `head` last changed: the
    /// newest commit, following first parents only, whose version of the
    /// file differs from its first parent's. The first commit of the
    /// history changes every file it holds, and so does a shallow clone's
    /// oldest commit. One entry per file of `head`, sorted by path.
    ///
    /// The history is walked once, and the walk stops when every file is
    /// found. By default it asks [`GitRead::read_commit`] and
    /// [`GitRead::markdown_changes`] once per commit ([`walk_last_changes`]);
    /// a reader may answer faster, with the same answer.
    ///
    /// # Errors
    ///
    /// When a commit or a tree on the way cannot be read.
    fn last_changes(&self, head: &ObjectId, roots: &Roots) -> Result<Vec<LastChange>, GitError> {
        history::walk(self, head, roots)
    }
}

#[cfg(test)]
pub(crate) mod fake;

#[cfg(test)]
mod tests {
    use super::*;

    fn id(c: char) -> ObjectId {
        ObjectId::from_hex(&c.to_string().repeat(40)).unwrap()
    }

    fn path(p: &str) -> RepoPath {
        RepoPath::new(p).unwrap()
    }

    #[test]
    fn object_ids_take_lowercase_hex_of_the_two_lengths() {
        let sha1 = "0123456789abcdef0123456789abcdef01234567";
        assert_eq!(ObjectId::from_hex(sha1).unwrap().as_hex(), sha1);
        let sha256 = "f".repeat(64);
        assert_eq!(ObjectId::from_hex(&sha256).unwrap().to_string(), sha256);
        for bad in [
            "",
            &"a".repeat(39),
            &"a".repeat(41),
            &"a".repeat(63),
            &"a".repeat(65),
            &"A".repeat(40),
            &"g".repeat(40),
            &format!("{}/", "a".repeat(39)),
            &format!("{}:", "a".repeat(39)),
            &format!("{}`", "a".repeat(39)),
        ] {
            assert_eq!(ObjectId::from_hex(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_change_is_built_only_when_something_changed() {
        let p = path("docs/a.md");
        assert_eq!(FileChange::between(p.clone(), None, None), None);
        assert_eq!(
            FileChange::between(p.clone(), Some(id('1')), Some(id('1'))),
            None
        );

        let added = FileChange::between(p.clone(), None, Some(id('2'))).unwrap();
        assert_eq!(
            added,
            FileChange::Added {
                path: p.clone(),
                new: id('2')
            }
        );
        assert_eq!((added.old_blob(), added.new_blob()), (None, Some(&id('2'))));

        let removed = FileChange::between(p.clone(), Some(id('1')), None).unwrap();
        assert_eq!(
            removed,
            FileChange::Removed {
                path: p.clone(),
                old: id('1')
            }
        );
        assert_eq!(
            (removed.old_blob(), removed.new_blob()),
            (Some(&id('1')), None)
        );

        let modified = FileChange::between(p.clone(), Some(id('1')), Some(id('2'))).unwrap();
        assert_eq!(
            (modified.old_blob(), modified.new_blob()),
            (Some(&id('1')), Some(&id('2')))
        );
        for change in [added, removed, modified] {
            assert_eq!(change.path(), &p);
        }
    }

    #[test]
    fn errors_say_what_went_wrong_in_plain_words() {
        let cases = [
            (
                GitError::UnknownRevision {
                    name: "nope".into(),
                },
                "`nope` does not name a commit".to_owned(),
            ),
            (
                GitError::MissingObject { id: id('1') },
                format!("object {} is not in the repo", id('1')),
            ),
            (
                GitError::WrongKind {
                    id: id('2'),
                    expected: ObjectKind::Blob,
                },
                format!("object {} is not a blob", id('2')),
            ),
            (
                GitError::WrongKind {
                    id: id('3'),
                    expected: ObjectKind::Commit,
                },
                format!("object {} is not a commit", id('3')),
            ),
            (
                GitError::NoWorktree,
                "the repo has no working tree".to_owned(),
            ),
            (
                GitError::MissingFile {
                    path: path("docs/x.md"),
                },
                "the working tree has no file `docs/x.md`".to_owned(),
            ),
            (
                GitError::Other {
                    message: "disk on fire".into(),
                },
                "disk on fire".to_owned(),
            ),
        ];
        for (error, text) in cases {
            assert_eq!(error.to_string(), text);
            let as_error: &dyn std::error::Error = &error;
            assert!(as_error.source().is_none());
        }
    }
}
