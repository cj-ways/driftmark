//! What the pages show, read from git when a page is opened.
//!
//! Each question here resolves names to commits first, then looks up what
//! those commits hold in the [`Cache`]. A branch that moved therefore shows
//! its new tip on the next request, with no timer and no rebuild.

use std::collections::HashMap;
use std::mem::size_of;
use std::sync::{Arc, Mutex, PoisonError};

use driftmark_core::diff::{ChangeList, DocumentChanges, RangeInfo, Summary};
use driftmark_core::git::{
    BranchInbox, DefaultBranch, FileChange, FileEntry, GitError, GitRead, ObjectId, OpenBranch,
    RangeError, RangeSpec, RepoPath, ResolvedRange, Roots, Snapshot, diff_files, inbox, merge_base,
};
use driftmark_core::rules::Rules;

use super::address::{RangeAddress, Revision};
use super::cache::{Cache, Key, RulesId, Sized};
use super::compare::{self, Job, Keep, Outcome, Parsed, RulesAt, Skipped};
use crate::diff::rules_file;
use crate::git::Git;
use crate::lint::RULES_FILE;

/// Why a page cannot be shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Problem {
    /// Whether the address names nothing: an unknown revision, a range that
    /// cannot be read, or a document that is not there. Otherwise reading
    /// git failed.
    pub missing: bool,
    /// What went wrong, as a sentence.
    pub message: String,
}

impl Problem {
    /// The address names nothing.
    pub(crate) fn missing(message: String) -> Problem {
        Problem {
            missing: true,
            message,
        }
    }

    /// Reading failed.
    pub(crate) fn failed(message: String) -> Problem {
        Problem {
            missing: false,
            message,
        }
    }
}

impl From<GitError> for Problem {
    fn from(error: GitError) -> Problem {
        let missing = matches!(
            error,
            GitError::UnknownRevision { .. } | GitError::NoWorktree | GitError::MissingFile { .. }
        );
        let message = if missing {
            error.to_string()
        } else {
            format!("git could not be read: {error}")
        };
        Problem { missing, message }
    }
}

impl From<RangeError> for Problem {
    fn from(error: RangeError) -> Problem {
        match error {
            RangeError::Git(error) => error.into(),
            RangeError::BadCommit { .. } => Problem::failed(error.to_string()),
            RangeError::NotARange { .. }
            | RangeError::BadDate { .. }
            | RangeError::NoMergeBase { .. }
            | RangeError::NoDefaultBranch => Problem::missing(error.to_string()),
        }
    }
}

/// A document at a revision, parsed.
pub(crate) struct DocumentAt {
    /// The commit, or `None` for the working tree.
    pub commit: Option<ObjectId>,
    /// The blob, or `None` for the working tree.
    pub blob: Option<ObjectId>,
    /// The rules file it was read with.
    pub rules: Arc<RulesAt>,
    /// The parsed document, or why it could not be parsed.
    pub parsed: Arc<Parsed>,
}

/// The counted change list of a range: each document's counts, without its
/// changes. The range's own text is added per request, because the same two
/// commits can be written several ways.
pub(crate) struct RangeCount {
    /// The documents with their summaries, sorted; `range` is empty.
    pub list: ChangeList,
    /// The files that could not be compared.
    pub skipped: Vec<Skipped>,
}

/// One document's changes in a range.
pub(crate) struct DocumentChangesAt {
    /// The change list with this one document, or none when it was skipped.
    pub list: ChangeList,
    /// Why the document could not be compared, when it could not.
    pub skipped: Vec<Skipped>,
}

/// One document of a range, found but not compared yet.
#[derive(Clone)]
pub(crate) struct Located {
    /// Its two blobs.
    pub change: FileChange,
    /// The rules of the newer side.
    pub rules: Arc<RulesAt>,
    /// Whether the newer side is the working tree.
    pub worktree: bool,
}

impl Located {
    /// The cache key of its finished changes page, or `None` when the newer
    /// side is the working tree, whose bytes are never cached.
    pub(crate) fn page_key(&self) -> Option<Key> {
        (!self.worktree).then(|| {
            Key::Changes(
                self.change.old_blob().cloned(),
                self.change.new_blob().cloned(),
                self.rules.id.clone(),
                self.change.path().clone(),
            )
        })
    }
}

/// A branch compared with the default branch: where it left it, and what
/// its documents changed.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BranchCount {
    /// The merge-base the branch was compared from.
    pub base: ObjectId,
    /// Notes on how the base was chosen.
    pub notes: Vec<String>,
    /// Changed Markdown files that were compared.
    pub documents: usize,
    /// Changed Markdown files that could not be compared.
    pub skipped: usize,
    /// The counts over its documents.
    pub totals: Summary,
}

/// How a branch's comparison with the default branch went.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum BranchState {
    /// It was compared.
    Counted(Arc<BranchCount>),
    /// It shares no history with the default branch.
    Unrelated,
}

/// The home page's branches.
pub(crate) struct Home {
    /// The branch every other one is compared with, or `None` when the repo
    /// has none of the default names.
    pub default: Option<DefaultBranch>,
    /// The local branch `HEAD` points at.
    pub current: Option<String>,
    /// The branches whose documents differ from the default branch, newest
    /// first.
    pub changed: Vec<(OpenBranch, Arc<BranchCount>)>,
    /// How many branches were compared and hold no document changes.
    pub unchanged: usize,
    /// How many branches share no history with the default branch.
    pub unrelated: usize,
}

/// The working tree compared with where its branch left the default branch:
/// the `auto` range, counted.
pub(crate) struct WorktreeCount {
    /// The Markdown files on disk that git lists, for watching.
    pub files: Vec<FileEntry>,
    /// The counts, or why `auto` cannot be compared, such as a repo without
    /// a default branch.
    pub counted: Result<(RangeInfo, RangeCount), String>,
}

/// The repository the server reads, and what it keeps.
pub(crate) struct Repo {
    /// The git reader.
    pub git: Git,
    /// Parsed documents, finished pages and counted ranges.
    pub cache: Cache,
    /// How many threads compare files.
    pub threads: usize,
    /// The rules without a rules file.
    default_rules: Arc<RulesAt>,
    /// Home pages are counted one at a time, so a second request finds the
    /// first one's branches in the cache instead of counting them again.
    home: Mutex<()>,
}

impl Repo {
    /// A repository read with `git`, keeping at most `cap` bytes.
    pub(crate) fn new(git: Git, cap: usize, threads: usize) -> Repo {
        Repo {
            git,
            cache: Cache::new(cap),
            threads,
            default_rules: Arc::new(RulesAt {
                id: RulesId::Default,
                rules: Rules::default(),
            }),
            home: Mutex::new(()),
        }
    }

    /// The rules file at one side of a range: from a commit, cached by its
    /// blob, or from the disk.
    ///
    /// # Errors
    ///
    /// When it cannot be read or is broken; the message names the line, as
    /// `driftmark diff` does.
    pub(crate) fn rules_at(&self, side: &Snapshot) -> Result<Arc<RulesAt>, Problem> {
        let Some(path) = RepoPath::new(RULES_FILE) else {
            return Ok(Arc::clone(&self.default_rules));
        };
        match side {
            Snapshot::Commit(commit) => {
                let Some(blob) = self.git.blob_at(commit, &path)? else {
                    return Ok(Arc::clone(&self.default_rules));
                };
                self.cache.get_or_load(&Key::Rules(blob.clone()), || {
                    let rules = rules_file::read(&self.git, side).map_err(Problem::failed)?;
                    Ok(Sized {
                        value: RulesAt {
                            id: RulesId::Blob(blob.clone()),
                            rules,
                        },
                        bytes: 16 * 1024,
                    })
                })
            }
            Snapshot::Worktree => match self.git.read_worktree_file(&path) {
                Err(GitError::MissingFile { .. }) => Ok(Arc::clone(&self.default_rules)),
                Err(error) => Err(error.into()),
                Ok(bytes) => Ok(Arc::new(RulesAt {
                    id: RulesId::Worktree(super::fingerprint(&bytes)),
                    rules: rules_file::read(&self.git, side).map_err(Problem::failed)?,
                })),
            },
        }
    }

    /// A document at a revision.
    ///
    /// # Errors
    ///
    /// A missing problem when the revision names no commit or the document is
    /// not there; a failed one when git cannot be read.
    pub(crate) fn document_at(
        &self,
        revision: &Revision,
        path: &RepoPath,
    ) -> Result<DocumentAt, Problem> {
        match revision {
            Revision::Name(name) => {
                let commit = self.git.resolve(name)?;
                let blob = self.git.blob_at(&commit, path)?.ok_or_else(|| {
                    Problem::missing(format!(
                        "`{}` has no Markdown document `{}`",
                        name,
                        path.to_lossy()
                    ))
                })?;
                let rules = self.rules_at(&Snapshot::Commit(commit.clone()))?;
                let parsed = compare::document(&self.git, &self.cache, &blob, &rules)?;
                Ok(DocumentAt {
                    commit: Some(commit),
                    blob: Some(blob),
                    rules,
                    parsed,
                })
            }
            Revision::Worktree => {
                self.known_worktree_file(path)?;
                let rules = self.rules_at(&Snapshot::Worktree)?;
                let parsed = compare::worktree_document(&self.git, path, &rules)?;
                Ok(DocumentAt {
                    commit: None,
                    blob: None,
                    rules,
                    parsed,
                })
            }
        }
    }

    /// Whether git knows a working-tree path: tracked at `HEAD`, or listed
    /// among the working tree's files. Only such a path is read from disk.
    fn known_worktree_file(&self, path: &RepoPath) -> Result<(), Problem> {
        if let Ok(head) = self.git.resolve("HEAD")
            && self.git.blob_at(&head, path)?.is_some()
        {
            return Ok(());
        }
        let files = self
            .git
            .markdown_files(&Snapshot::Worktree, &Roots::all())?;
        if files.iter().any(|file| &file.path == path) {
            Ok(())
        } else {
            Err(GitError::MissingFile { path: path.clone() }.into())
        }
    }

    /// The two points of a range.
    ///
    /// # Errors
    ///
    /// A missing problem when the range cannot be read or resolved.
    pub(crate) fn resolve(&self, address: &RangeAddress) -> Result<ResolvedRange, Problem> {
        Ok(address.spec.resolve(&self.git)?)
    }

    /// The counted change list of a resolved range. A range between two
    /// commits is cached; one that ends in the working tree is counted on
    /// every request.
    ///
    /// # Errors
    ///
    /// When git cannot be read or the rules file is broken.
    pub(crate) fn range_count(&self, range: &ResolvedRange) -> Result<Arc<RangeCount>, Problem> {
        let count = || {
            let changes = range.changes(&self.git, &Roots::all())?;
            self.count(&range.head, changes)
        };
        match &range.head {
            Snapshot::Commit(head) => {
                self.cache
                    .get_or_load(&Key::Range(range.base.clone(), head.clone()), || {
                        count().map(|count| {
                            let bytes = count_bytes(&count);
                            Sized {
                                value: count,
                                bytes,
                            }
                        })
                    })
            }
            Snapshot::Worktree => count().map(Arc::new),
        }
    }

    /// Compares changed files with the rules of the newer side, keeping only
    /// each document's counts.
    fn count(&self, head: &Snapshot, changes: Vec<FileChange>) -> Result<RangeCount, Problem> {
        let rules = self.rules_at(head)?;
        let worktree = *head == Snapshot::Worktree;
        let jobs: Vec<Job> = changes
            .into_iter()
            .map(|change| Job {
                change,
                rules: Arc::clone(&rules),
                worktree,
            })
            .collect();
        let outcomes =
            compare::compare_all(&self.git, &self.cache, &jobs, self.threads, Keep::Counts)?;
        let (documents, skipped) = split(outcomes);
        Ok(RangeCount {
            list: ChangeList::new(None, documents),
            skipped,
        })
    }

    /// Finds one document in a resolved range: its two blobs and the rules
    /// of the newer side. Nothing is parsed yet, so a cached page can be
    /// found first.
    ///
    /// # Errors
    ///
    /// A missing problem when the document did not change in the range.
    pub(crate) fn locate(
        &self,
        range: &ResolvedRange,
        path: &RepoPath,
    ) -> Result<Located, Problem> {
        let old = match &range.base {
            Some(base) => self.git.blob_at(base, path)?,
            None => None,
        };
        let (new, worktree) = match &range.head {
            Snapshot::Commit(head) => (self.git.blob_at(head, path)?, false),
            Snapshot::Worktree => {
                let files = self
                    .git
                    .markdown_files(&Snapshot::Worktree, &Roots::all())?;
                let listed = files.into_iter().find(|file| &file.path == path);
                (listed.map(|file| file.id), true)
            }
        };
        let change = FileChange::between(path.clone(), old, new).ok_or_else(|| {
            Problem::missing(format!(
                "`{}` did not change in this range",
                path.to_lossy()
            ))
        })?;
        Ok(Located {
            change,
            rules: self.rules_at(&range.head)?,
            worktree,
        })
    }

    /// One located document's changes in its range.
    ///
    /// # Errors
    ///
    /// When git cannot read a version.
    pub(crate) fn document_changes(
        &self,
        range: &ResolvedRange,
        located: Located,
    ) -> Result<DocumentChangesAt, Problem> {
        let job = Job {
            change: located.change,
            rules: located.rules,
            worktree: located.worktree,
        };
        let outcomes = compare::compare_all(&self.git, &self.cache, &[job], 1, Keep::Changes)?;
        let (documents, skipped) = split(outcomes);
        Ok(DocumentChangesAt {
            list: ChangeList::new(Some(RangeInfo::of(range)), documents),
            skipped,
        })
    }

    /// The branches whose documents differ from the default branch, each
    /// compared from where it left it, as `<default>...<branch>` shows.
    ///
    /// A branch's count is cached by its tip and the default branch's tip.
    /// The branches that are not cached are compared together, so a blob
    /// that several of them change is parsed once.
    ///
    /// # Errors
    ///
    /// When git cannot be read.
    pub(crate) fn home(&self) -> Result<Home, Problem> {
        let _one_at_a_time = self.home.lock().unwrap_or_else(PoisonError::into_inner);
        let BranchInbox {
            default,
            current,
            branches,
        } = inbox(&self.git)?;
        let mut home = Home {
            default,
            current,
            changed: Vec::new(),
            unchanged: 0,
            unrelated: 0,
        };
        let Some(default) = &home.default else {
            return Ok(home);
        };
        let steps = parallel(&branches, self.threads, |branch| {
            self.branch_step(default, branch)
        });
        // A change that several branches share, the same file between the
        // same two blobs under the same rules, is compared once.
        let mut jobs: Vec<Job> = Vec::new();
        let mut seen: HashMap<(RepoPath, Option<ObjectId>, Option<ObjectId>, RulesId), usize> =
            HashMap::new();
        let mut states = Vec::with_capacity(steps.len());
        for step in steps {
            match step? {
                Step::Known(state) => states.push(Ok(state)),
                Step::Compare {
                    base,
                    notes,
                    changes,
                    rules,
                } => {
                    let mine: Vec<usize> = changes
                        .into_iter()
                        .map(|change| {
                            let key = (
                                change.path().clone(),
                                change.old_blob().cloned(),
                                change.new_blob().cloned(),
                                rules.id.clone(),
                            );
                            *seen.entry(key).or_insert_with(|| {
                                jobs.push(Job {
                                    change,
                                    rules: Arc::clone(&rules),
                                    worktree: false,
                                });
                                jobs.len() - 1
                            })
                        })
                        .collect();
                    states.push(Err((base, notes, mine)));
                }
            }
        }
        let outcomes =
            compare::compare_all(&self.git, &self.cache, &jobs, self.threads, Keep::Counts)?;
        let default_tip = default.tip.clone();
        for (branch, state) in branches.into_iter().zip(states) {
            let state = match state {
                Ok(state) => state,
                Err((base, notes, mine)) => {
                    let mine = mine.iter().map(|&at| outcomes[at].clone()).collect();
                    let count = Arc::new(branch_count(base, notes, mine));
                    let key = Key::Branch(default_tip.clone(), branch.tip.clone());
                    self.cache
                        .put(&key, BranchState::Counted(Arc::clone(&count)), 512);
                    BranchState::Counted(count)
                }
            };
            match state {
                BranchState::Unrelated => home.unrelated += 1,
                BranchState::Counted(count) if count.documents + count.skipped == 0 => {
                    home.unchanged += 1;
                }
                BranchState::Counted(count) => home.changed.push((branch, count)),
            }
        }
        Ok(home)
    }

    /// A branch's cached state, or what is needed to count it. A branch
    /// merged into the default branch left it at its own tip, so it changes
    /// nothing, and its files are not listed.
    fn branch_step(&self, default: &DefaultBranch, branch: &OpenBranch) -> Result<Step, Problem> {
        let key = Key::Branch(default.tip.clone(), branch.tip.clone());
        if let Some(state) = self.cache.get::<BranchState>(&key) {
            return Ok(Step::Known((*state).clone()));
        }
        let (base, notes) = match merge_base(&self.git, &default.tip, &branch.tip) {
            Ok(found) => found,
            Err(RangeError::NoMergeBase { .. }) => {
                self.cache.put(&key, BranchState::Unrelated, 64);
                return Ok(Step::Known(BranchState::Unrelated));
            }
            Err(error) => return Err(error.into()),
        };
        let notes: Vec<String> = notes.iter().map(ToString::to_string).collect();
        if base == branch.tip {
            let state = BranchState::Counted(Arc::new(branch_count(base, notes, Vec::new())));
            self.cache.put(&key, state.clone(), 512);
            return Ok(Step::Known(state));
        }
        let changes = self
            .git
            .markdown_changes(&base, &branch.tip, &Roots::all())?;
        let rules = self.rules_at(&Snapshot::Commit(branch.tip.clone()))?;
        Ok(Step::Compare {
            base,
            notes,
            changes,
            rules,
        })
    }

    /// The working tree's Markdown files and the `auto` range, counted. The
    /// watcher runs this in the background, so the home page never waits for
    /// the working tree to be read.
    ///
    /// # Errors
    ///
    /// When the working tree cannot be listed.
    pub(crate) fn count_worktree(&self) -> Result<WorktreeCount, Problem> {
        let files = self
            .git
            .markdown_files(&Snapshot::Worktree, &Roots::all())?;
        let counted = self.count_auto(&files).map_err(|problem| problem.message);
        Ok(WorktreeCount { files, counted })
    }

    fn count_auto(&self, files: &[FileEntry]) -> Result<(RangeInfo, RangeCount), Problem> {
        let range = RangeSpec::Auto.resolve(&self.git)?;
        let old = match &range.base {
            Some(base) => self
                .git
                .markdown_files(&Snapshot::Commit(base.clone()), &Roots::all())?,
            None => Vec::new(),
        };
        let count = self.count(&Snapshot::Worktree, diff_files(&old, files))?;
        Ok((RangeInfo::of(&range), count))
    }
}

/// What one branch needs before its files are compared.
enum Step {
    /// Its state is known: cached, or it shares no history.
    Known(BranchState),
    /// Its changed files are to be compared.
    Compare {
        base: ObjectId,
        notes: Vec<String>,
        changes: Vec<FileChange>,
        rules: Arc<RulesAt>,
    },
}

/// The documents and the skipped files of some outcomes.
fn split(outcomes: Vec<Outcome>) -> (Vec<DocumentChanges>, Vec<Skipped>) {
    let mut documents = Vec::new();
    let mut skipped = Vec::new();
    for outcome in outcomes {
        match outcome {
            Outcome::Compared(changes) => documents.push(changes),
            Outcome::Skipped(file) => skipped.push(file),
        }
    }
    (documents, skipped)
}

fn branch_count(base: ObjectId, notes: Vec<String>, outcomes: Vec<Outcome>) -> BranchCount {
    let (documents, skipped) = split(outcomes);
    let list = ChangeList::new(None, documents);
    BranchCount {
        base,
        notes,
        documents: list.documents.len(),
        skipped: skipped.len(),
        totals: list.totals,
    }
}

/// The memory a counted range holds, estimated.
fn count_bytes(count: &RangeCount) -> usize {
    let documents: usize = count
        .list
        .documents
        .iter()
        .map(|document| size_of::<DocumentChanges>() + document.path.capacity())
        .sum();
    let skipped: usize = count
        .skipped
        .iter()
        .map(|file| size_of::<Skipped>() + file.path.capacity() + file.reason.capacity())
        .sum();
    documents + skipped
}

/// Runs `work` on every item on up to `threads` threads, and returns the
/// results in the items' order.
pub(crate) fn parallel<T: Sync, R: Send>(
    items: &[T],
    threads: usize,
    work: impl Fn(&T) -> R + Sync,
) -> Vec<R> {
    let threads = threads.clamp(1, items.len().max(1));
    if threads == 1 {
        return items.iter().map(work).collect();
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let mut done: Vec<(usize, R)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut local = Vec::new();
                    loop {
                        let index = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(item) = items.get(index) else {
                            return local;
                        };
                        local.push((index, work(item)));
                    }
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            })
            .collect()
    });
    done.sort_by_key(|(index, _)| *index);
    done.into_iter().map(|(_, result)| result).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn problems_tell_a_missing_page_from_a_failed_read() {
        let id = ObjectId::from_hex(&"a".repeat(40)).unwrap();
        let missing: Problem = GitError::UnknownRevision {
            name: "nope".into(),
        }
        .into();
        assert_eq!(
            missing,
            Problem::missing("`nope` does not name a commit".into())
        );
        let failed: Problem = GitError::MissingObject { id: id.clone() }.into();
        assert!(!failed.missing);
        assert!(failed.message.starts_with("git could not be read: object"));
        let range: Problem = RangeError::NoDefaultBranch.into();
        assert!(range.missing);
        let bad: Problem = RangeError::BadCommit { id }.into();
        assert!(!bad.missing);
        let through: Problem = RangeError::Git(GitError::NoWorktree).into();
        assert_eq!(
            through,
            Problem::missing("the repo has no working tree".into())
        );
    }

    #[test]
    fn parallel_work_keeps_the_order() {
        let items: Vec<usize> = (0..100).collect();
        assert_eq!(
            parallel(&items, 7, |n| n * 2),
            (0..100).map(|n| n * 2).collect::<Vec<_>>()
        );
        assert_eq!(parallel(&[] as &[usize], 4, |n| *n), Vec::<usize>::new());
    }
}
