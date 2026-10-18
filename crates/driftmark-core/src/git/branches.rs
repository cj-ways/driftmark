//! Which branches the home page compares with the default branch.
//!
//! The home page of `driftmark serve` is an inbox of branches whose documents
//! differ from the default branch (decision 0010). This module decides which
//! branches those are and in which order, before any document is read. The
//! program then compares each one with the default branch, as `auto` does for
//! the working tree.

use std::collections::HashSet;

use super::range::first_default_branch;
use super::{CommitInfo, GitRead, ObjectId, RangeError};

/// The branch every other branch is compared with: the first of
/// [`super::AUTO_BASES`] that exists, the same one `auto` uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefaultBranch {
    /// Its name, such as `origin/main`.
    pub name: &'static str,
    /// The commit at its tip.
    pub tip: ObjectId,
}

/// A branch to compare with the default branch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenBranch {
    /// The name git shows and resolves, such as `feature/login` or
    /// `origin/feature/login`.
    pub name: String,
    /// Whether it is a remote-tracking branch.
    pub remote: bool,
    /// The commit at its tip.
    pub tip: ObjectId,
    /// When the tip was committed, in seconds since 1970-01-01 00:00 UTC.
    pub committed_at: i64,
}

/// The branches the home page lists, before their documents are compared.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchInbox {
    /// The default branch, or `None` when none of [`super::AUTO_BASES`]
    /// exists. Then nothing is compared and `branches` is empty.
    pub default: Option<DefaultBranch>,
    /// The local branch `HEAD` points at, when it points at one.
    pub current: Option<String>,
    /// The other branches, newest tip first.
    pub branches: Vec<OpenBranch>,
}

/// Lists the branches to compare with the default branch.
///
/// 1. A branch whose tip is the default branch's tip is left out, because
///    nothing differs. This leaves out the default branch itself.
/// 2. Several branches at one tip are listed once, under the first name in
///    [`GitRead::branches`] order: local before remote-tracking, then by
///    name. A local branch and the remote-tracking branch it was pushed to
///    usually share a tip, and one entry is enough.
/// 3. The newest tip commit comes first, by committer date; equal dates go by
///    name.
///
/// Whether a branch's documents differ is not decided here: a branch merged
/// into the default branch is still listed, and comparing it finds nothing.
///
/// # Errors
///
/// A git error from listing the branches, resolving the default branch or
/// reading a tip commit, and [`RangeError::BadCommit`] for a tip whose bytes
/// are not a commit.
pub fn inbox(git: &dyn GitRead) -> Result<BranchInbox, RangeError> {
    let all = git.branches()?;
    let current = all.iter().find(|b| b.current).map(|b| b.name.clone());
    let default = match first_default_branch(git) {
        Ok((name, tip)) => DefaultBranch { name, tip },
        Err(RangeError::NoDefaultBranch) => {
            return Ok(BranchInbox {
                default: None,
                current,
                branches: Vec::new(),
            });
        }
        Err(error) => return Err(error),
    };
    let mut seen = HashSet::from([default.tip.clone()]);
    let mut branches = Vec::new();
    for branch in all {
        if !seen.insert(branch.tip.clone()) {
            continue;
        }
        let raw = git.read_commit(&branch.tip)?;
        let info = CommitInfo::parse(&raw).ok_or_else(|| RangeError::BadCommit {
            id: branch.tip.clone(),
        })?;
        branches.push(OpenBranch {
            name: branch.name,
            remote: branch.remote,
            tip: branch.tip,
            committed_at: info.committed_at,
        });
    }
    branches.sort_by(|a, b| {
        b.committed_at
            .cmp(&a.committed_at)
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(BranchInbox {
        default: Some(default),
        current,
        branches,
    })
}

#[cfg(test)]
mod tests {
    use super::super::GitError;
    use super::super::fake::{FakeGit, id};
    use super::*;

    fn open(name: &str, remote: bool, digit: char, committed_at: i64) -> OpenBranch {
        OpenBranch {
            name: name.into(),
            remote,
            tip: id(digit),
            committed_at,
        }
    }

    #[test]
    fn branches_come_newest_first_once_per_tip_without_the_default() {
        let git = FakeGit::new()
            .name("origin/main", '1')
            .commit('2', &[], 200)
            .commit('3', &[], 300)
            .commit('4', &[], 300)
            .commit('5', &[], 100)
            .branch("docs", false, '4')
            .current_branch("feature", '3')
            .branch("main", false, '1')
            .branch("old", false, '5')
            .branch("zeta", false, '2')
            .branch("origin/feature", true, '3')
            .branch("origin/main", true, '1')
            .branch("origin/other", true, '2');
        let inbox = inbox(&git).unwrap();
        assert_eq!(
            inbox.default,
            Some(DefaultBranch {
                name: "origin/main",
                tip: id('1')
            })
        );
        assert_eq!(inbox.current.as_deref(), Some("feature"));
        assert_eq!(
            inbox.branches,
            vec![
                open("docs", false, '4', 300),
                open("feature", false, '3', 300),
                open("zeta", false, '2', 200),
                open("old", false, '5', 100),
            ]
        );
    }

    #[test]
    fn a_remote_branch_alone_at_its_tip_is_listed() {
        let git = FakeGit::new()
            .name("main", '1')
            .commit('2', &[], 50)
            .branch("main", false, '1')
            .branch("origin/agent/plan", true, '2');
        let inbox = inbox(&git).unwrap();
        assert_eq!(inbox.current, None);
        assert_eq!(
            inbox.default.map(|d| d.name),
            Some("main"),
            "main is used when origin/main is missing"
        );
        assert_eq!(
            inbox.branches,
            vec![open("origin/agent/plan", true, '2', 50)]
        );
    }

    #[test]
    fn without_a_default_branch_nothing_is_compared() {
        let git = FakeGit::new()
            .current_branch("trunk", '1')
            .branch("feature", false, '2');
        assert_eq!(
            inbox(&git).unwrap(),
            BranchInbox {
                default: None,
                current: Some("trunk".into()),
                branches: Vec::new(),
            }
        );
    }

    #[test]
    fn git_problems_stop_the_list() {
        let broken_refs = FakeGit::new().name("main", '1').branches_fail();
        let other = |message: &str| {
            Err(RangeError::Git(GitError::Other {
                message: message.into(),
            }))
        };
        assert_eq!(inbox(&broken_refs), other("no refs"));

        let broken_default = FakeGit::new().fail_name("origin/main");
        assert_eq!(inbox(&broken_default), other("broken ref"));

        let missing_tip = FakeGit::new()
            .name("main", '1')
            .branch("feature", false, '2');
        assert_eq!(
            inbox(&missing_tip),
            Err(RangeError::Git(GitError::MissingObject { id: id('2') }))
        );

        let bad_tip = FakeGit::new()
            .name("main", '1')
            .raw_commit('2', b"not a commit")
            .branch("feature", false, '2');
        assert_eq!(inbox(&bad_tip), Err(RangeError::BadCommit { id: id('2') }));
    }
}
