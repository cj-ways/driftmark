//! When each file last changed: one walk back along a branch's first
//! parents, newest commit first.
//!
//! The status view shows how long ago each document last changed. Asking
//! git once per file would walk the history once per file. This walks it
//! once for every file, and stops as soon as each one is found.
//!
//! The rule, which every reader follows through [`LastChanges`]:
//!
//! - Only first parents are followed. A merge counts as changing what it
//!   brought in, so a file edited on a branch counts as changed when the
//!   branch was merged: when the change reached this branch.
//! - A commit changes a file when the file's blob differs from the one in
//!   its first parent, as [`super::FileChange::between`] decides: a new
//!   mode with the same bytes is no change.
//! - The first commit of the history changes every file it holds. So does
//!   the oldest commit of a shallow clone, the furthest back git can see.

use std::collections::HashSet;

use super::{CommitInfo, FileChange, GitError, GitRead, ObjectId, RepoPath, Roots, Snapshot};

/// When one file last changed on a branch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LastChange {
    /// The file.
    pub path: RepoPath,
    /// The newest commit, along first parents, that changed it.
    pub commit: ObjectId,
    /// That commit's committer date, in seconds since 1970-01-01 00:00 UTC,
    /// as [`CommitInfo::committed_at`] reads it.
    pub committed_at: i64,
}

/// The files of a branch's tip still waiting for their last change, and
/// those found so far. A reader walks the history newest first and tells
/// this what each commit changed.
#[derive(Debug, Default)]
pub struct LastChanges {
    pending: HashSet<RepoPath>,
    found: Vec<LastChange>,
}

impl LastChanges {
    /// Starts with every file of the tip, none found yet.
    pub fn new(files: impl IntoIterator<Item = RepoPath>) -> LastChanges {
        LastChanges {
            pending: files.into_iter().collect(),
            found: Vec::new(),
        }
    }

    /// Whether every file has been found, so the walk can stop.
    #[must_use]
    pub fn is_done(&self) -> bool {
        self.pending.is_empty()
    }

    /// The files still waiting, in no order. While a file waits, every
    /// commit the walk reaches holds the same blob for it as the tip, so a
    /// reader only needs to ask whether the first parent holds that blob
    /// too.
    pub fn pending(&self) -> impl Iterator<Item = &RepoPath> {
        self.pending.iter()
    }

    /// Records a commit whose comparison with its first parent changed
    /// `paths`. Each file still waiting gets this commit; a file found at a
    /// newer commit keeps that one, and a path the tip does not hold is
    /// ignored.
    pub fn changed<'a>(
        &mut self,
        commit: &ObjectId,
        committed_at: i64,
        paths: impl IntoIterator<Item = &'a RepoPath>,
    ) {
        for path in paths {
            if self.pending.remove(path) {
                self.found.push(LastChange {
                    path: path.clone(),
                    commit: commit.clone(),
                    committed_at,
                });
            }
        }
    }

    /// Records the oldest commit the walk reaches: the first of the
    /// history, or the oldest a shallow clone holds. It changed every file
    /// still waiting.
    pub fn first(&mut self, commit: &ObjectId, committed_at: i64) {
        for path in std::mem::take(&mut self.pending) {
            self.found.push(LastChange {
                path,
                commit: commit.clone(),
                committed_at,
            });
        }
    }

    /// Every file found, sorted by path.
    #[must_use]
    pub fn finish(mut self) -> Vec<LastChange> {
        self.found.sort_by(|a, b| a.path.cmp(&b.path));
        self.found
    }
}

/// The walk any reader can do with its other answers: one commit read and
/// one comparison per commit. [`GitRead::last_changes`] uses it unless a
/// reader answers faster.
///
/// A comparison that fails because the first parent is missing, as in a
/// shallow clone whose reader reports it that way, ends the walk there.
///
/// # Errors
///
/// When a commit, a tree or the files of `head` cannot be read, and when a
/// commit is not one git would write.
pub fn walk<G: GitRead + ?Sized>(
    git: &G,
    head: &ObjectId,
    roots: &Roots,
) -> Result<Vec<LastChange>, GitError> {
    let files = git.markdown_files(&Snapshot::Commit(head.clone()), roots)?;
    let mut changes = LastChanges::new(files.into_iter().map(|file| file.path));
    let mut commit = head.clone();
    while !changes.is_done() {
        let raw = git.read_commit(&commit)?;
        let info = CommitInfo::parse(&raw).ok_or_else(|| GitError::Other {
            message: format!("the commit {commit} cannot be read"),
        })?;
        let Some(parent) = info.parents.into_iter().next() else {
            changes.first(&commit, info.committed_at);
            break;
        };
        match git.markdown_changes(&parent, &commit, roots) {
            Ok(diff) => {
                changes.changed(
                    &commit,
                    info.committed_at,
                    diff.iter().map(FileChange::path),
                );
            }
            Err(GitError::MissingObject { id }) if id == parent => {
                changes.first(&commit, info.committed_at);
                break;
            }
            Err(error) => return Err(error),
        }
        commit = parent;
    }
    Ok(changes.finish())
}

#[cfg(test)]
mod tests {
    use super::super::fake::{FakeGit, entry, id};
    use super::*;

    fn path(text: &str) -> RepoPath {
        RepoPath::new(text).unwrap()
    }

    fn change(path_text: &str, digit: char, time: i64) -> LastChange {
        LastChange {
            path: path(path_text),
            commit: id(digit),
            committed_at: time,
        }
    }

    #[test]
    fn a_file_keeps_its_newest_change() {
        let mut changes = LastChanges::new([path("a.md"), path("b.md"), path("c.md")]);
        assert!(!changes.is_done());
        changes.changed(&id('3'), 30, [&path("a.md"), &path("gone.md")]);
        let mut waiting: Vec<&RepoPath> = changes.pending().collect();
        waiting.sort();
        assert_eq!(waiting, [&path("b.md"), &path("c.md")]);
        changes.changed(&id('2'), 20, [&path("a.md"), &path("b.md")]);
        assert!(!changes.is_done());
        changes.first(&id('1'), 10);
        assert!(changes.is_done());
        changes.changed(&id('0'), 0, [&path("c.md")]);
        assert_eq!(
            changes.finish(),
            [
                change("a.md", '3', 30),
                change("b.md", '2', 20),
                change("c.md", '1', 10)
            ]
        );
        assert!(LastChanges::new([]).is_done());
        assert_eq!(LastChanges::default().finish(), Vec::new());
    }

    /// Commit 1 holds a.md and b.md; 2 edits a.md; 3 is a merge whose first
    /// parent is 2 and which adds c.md; 4 edits nothing. 5 is a commit whose
    /// parent is missing, as in a shallow clone.
    fn history() -> FakeGit {
        FakeGit::new()
            .commit('1', &[], 100)
            .commit('2', &['1'], 200)
            .commit('3', &['2', '9'], 300)
            .commit('4', &['3'], 400)
            .files('1', &[entry("a.md", 'a'), entry("b.md", 'b')])
            .files('2', &[entry("a.md", 'c'), entry("b.md", 'b')])
            .files(
                '3',
                &[entry("a.md", 'c'), entry("b.md", 'b'), entry("c.md", 'd')],
            )
            .files(
                '4',
                &[entry("a.md", 'c'), entry("b.md", 'b'), entry("c.md", 'd')],
            )
    }

    #[test]
    fn the_walk_follows_first_parents_to_the_first_commit() {
        let git = history();
        assert_eq!(
            walk(&git, &id('4'), &Roots::all()),
            Ok(vec![
                change("a.md", '2', 200),
                change("b.md", '1', 100),
                change("c.md", '3', 300),
            ])
        );
        assert_eq!(
            git.last_changes(&id('4'), &Roots::all()),
            walk(&git, &id('4'), &Roots::all())
        );
    }

    #[test]
    fn the_walk_stops_once_every_file_is_found() {
        // Commit 1's object is broken, but every file of 3 is found before
        // the walk would read it.
        let git = history()
            .raw_commit('1', b"not a commit")
            .files('3', &[entry("a.md", 'c'), entry("c.md", 'd')])
            .files('2', &[entry("a.md", 'c')]);
        assert_eq!(
            walk(&git, &id('3'), &Roots::all()),
            Ok(vec![change("a.md", '2', 200), change("c.md", '3', 300)])
        );
        assert_eq!(
            walk(&git, &id('1'), &Roots::all()).unwrap_err().to_string(),
            format!("the commit {} cannot be read", id('1'))
        );
        let empty = FakeGit::new().files('5', &[]);
        assert_eq!(walk(&empty, &id('5'), &Roots::all()), Ok(Vec::new()));
    }

    #[test]
    fn a_missing_first_parent_ends_the_walk_there() {
        let git = FakeGit::new()
            .commit('5', &['6'], 500)
            .commit('7', &['5'], 700)
            .files('5', &[entry("a.md", 'a'), entry("b.md", 'b')])
            .files('7', &[entry("a.md", 'a'), entry("b.md", 'c')])
            .missing('6');
        assert_eq!(
            walk(&git, &id('7'), &Roots::all()),
            Ok(vec![change("a.md", '5', 500), change("b.md", '7', 700)])
        );
    }

    #[test]
    fn other_errors_end_the_walk() {
        let unknown = FakeGit::new();
        assert!(walk(&unknown, &id('1'), &Roots::all()).is_err());
        let no_commit = FakeGit::new().files('1', &[entry("a.md", 'a')]);
        assert_eq!(
            walk(&no_commit, &id('1'), &Roots::all()),
            Err(GitError::MissingObject { id: id('1') })
        );
        // Commit 2's parent 1 has no files listed, so comparing them fails.
        // A missing commit other than the parent is not a shallow boundary.
        let broken = FakeGit::new()
            .commit('2', &['1'], 200)
            .files('2', &[entry("a.md", 'a')]);
        assert_eq!(
            walk(&broken, &id('2'), &Roots::all()),
            Err(GitError::Other {
                message: "no files".into()
            })
        );
        let elsewhere = FakeGit::new()
            .commit('2', &['1'], 200)
            .files('2', &[entry("a.md", 'a')])
            .missing('2');
        assert_eq!(
            walk(&elsewhere, &id('2'), &Roots::all()),
            Err(GitError::MissingObject { id: id('2') })
        );
    }
}
