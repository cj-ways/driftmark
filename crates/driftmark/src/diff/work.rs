//! Reading, parsing and comparing the changed files, on several threads.
//!
//! Each thread takes the next changed file, reads and parses both versions,
//! classifies them with the rules and compares them. A blob that several
//! files share, such as a file moved to a new path, is read and parsed once.

use std::collections::HashMap;
use std::ops::AddAssign;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use driftmark_core::diff::{DocumentChanges, Version, compare};
use driftmark_core::document::{Document, ParseError};
use driftmark_core::git::{FileChange, GitError, GitRead, ObjectId, RepoPath, Snapshot};
use driftmark_core::rules::{Classification, Rules};

/// Time spent in each step of the per-file work, summed over the threads.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct Steps {
    /// Reading versions from git or from disk.
    pub read: Duration,
    /// Parsing them into documents.
    pub parse: Duration,
    /// Classifying them with the rules and comparing them.
    pub compare: Duration,
}

impl AddAssign for Steps {
    fn add_assign(&mut self, other: Steps) {
        self.read += other.read;
        self.parse += other.parse;
        self.compare += other.compare;
    }
}

/// How many file versions the changes have, and how many were parsed. The
/// difference is what the cache saved.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Counts {
    /// Versions the changes name: one for an added or removed file, two for
    /// a modified one.
    pub versions: usize,
    /// Versions parsed.
    pub parsed: usize,
}

impl AddAssign for Counts {
    fn add_assign(&mut self, other: Counts) {
        self.versions += other.versions;
        self.parsed += other.parsed;
    }
}

/// A changed file left out of the change list, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Skipped {
    /// The file.
    pub path: RepoPath,
    /// Why, as a sentence.
    pub reason: String,
}

/// What the work produced.
pub(crate) struct Compared {
    /// The changes of each file that was compared, in the order given.
    pub documents: Vec<DocumentChanges>,
    /// The files that could not be compared, in the order given.
    pub skipped: Vec<Skipped>,
    /// Time per step, summed over the threads.
    pub steps: Steps,
    /// Versions named and parsed.
    pub counts: Counts,
    /// The threads that did the work.
    pub threads: usize,
}

/// Reads, parses and compares every change on up to `threads` threads.
///
/// A version that is not valid Markdown, such as a file that is not UTF-8,
/// skips its file instead of stopping the run.
///
/// # Errors
///
/// The first git error, by file order. The threads stop taking new files
/// once one happens.
pub(crate) fn compare_all(
    git: &dyn GitRead,
    rules: &Rules,
    changes: &[FileChange],
    head: &Snapshot,
    threads: usize,
) -> Result<Compared, GitError> {
    let worktree = *head == Snapshot::Worktree;
    let cached = changes.iter().filter_map(FileChange::old_blob).chain(
        changes
            .iter()
            .filter(|_| !worktree)
            .filter_map(FileChange::new_blob),
    );
    let work = Work {
        git,
        rules,
        worktree,
        cache: Cache::new(cached),
    };
    let threads = threads.clamp(1, changes.len().max(1));
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let locals: Vec<Local> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut local = Local::default();
                    while !stop.load(Ordering::Relaxed) {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some(change) = changes.get(index) else {
                            break;
                        };
                        let done = work.one(change, &mut local);
                        if matches!(done, Done::Failed(_)) {
                            stop.store(true, Ordering::Relaxed);
                        }
                        local.done.push((index, done));
                    }
                    local
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            })
            .collect()
    });
    let mut steps = Steps::default();
    let mut counts = Counts::default();
    let mut done = Vec::with_capacity(changes.len());
    for local in locals {
        steps += local.steps;
        counts += local.counts;
        done.extend(local.done);
    }
    done.sort_by_key(|(index, _)| *index);
    let mut documents = Vec::with_capacity(done.len());
    let mut skipped = Vec::new();
    for (_, outcome) in done {
        match outcome {
            Done::Compared(changes) => documents.push(changes),
            Done::Skipped(file) => skipped.push(file),
            Done::Failed(error) => return Err(error),
        }
    }
    Ok(Compared {
        documents,
        skipped,
        steps,
        counts,
        threads,
    })
}

/// What one thread did.
#[derive(Default)]
struct Local {
    steps: Steps,
    counts: Counts,
    done: Vec<(usize, Done)>,
}

/// What happened to one changed file.
enum Done {
    Compared(DocumentChanges),
    Skipped(Skipped),
    Failed(GitError),
}

/// A parsed version, or why there is none.
type Loaded = Result<Arc<Document>, Failure>;

#[derive(Clone)]
enum Failure {
    Git(GitError),
    Parse(ParseError),
}

impl Failure {
    /// What this failure means for the file: a git error stops the run, a
    /// parse error skips the file.
    fn done(self, path: &RepoPath, side: &str) -> Done {
        match self {
            Failure::Git(error) => Done::Failed(error),
            Failure::Parse(error) => Done::Skipped(Skipped {
                path: path.clone(),
                reason: format!("the {side} version cannot be compared: {error}"),
            }),
        }
    }
}

/// What every thread shares.
struct Work<'a> {
    git: &'a dyn GitRead,
    rules: &'a Rules,
    worktree: bool,
    cache: Cache,
}

impl Work<'_> {
    fn one(&self, change: &FileChange, local: &mut Local) -> Done {
        let path = change.path();
        let old = change.old_blob().map(|id| self.blob(id, local));
        let new = change.new_blob().map(|id| {
            if self.worktree {
                self.worktree_file(path, local)
            } else {
                self.blob(id, local)
            }
        });
        let old = match old.transpose() {
            Ok(old) => old,
            Err(failure) => return failure.done(path, "old"),
        };
        let new = match new.transpose() {
            Ok(new) => new,
            Err(failure) => return failure.done(path, "new"),
        };
        let start = Instant::now();
        let text_path = path.to_lossy();
        // A panic here would be a bug in the comparison. It skips this file
        // and keeps the others, as the parser does for a bad file.
        let compared = catch_unwind(AssertUnwindSafe(|| {
            let old_class = old.as_deref().map(|d| self.rules.classify(&text_path, d));
            let new_class = new.as_deref().map(|d| self.rules.classify(&text_path, d));
            compare(
                &text_path,
                version(old.as_deref(), old_class.as_ref()),
                version(new.as_deref(), new_class.as_ref()),
            )
        }));
        local.steps.compare += start.elapsed();
        match compared {
            Ok(Some(changes)) => Done::Compared(changes),
            Ok(None) => Done::Skipped(Skipped {
                path: path.clone(),
                reason: "it has no version on either side".to_owned(),
            }),
            Err(_) => Done::Skipped(Skipped {
                path: path.clone(),
                reason: "driftmark stopped while comparing it; this is a bug in driftmark"
                    .to_owned(),
            }),
        }
    }

    /// A committed version, from the cache when another file needed the same
    /// blob.
    fn blob(&self, id: &ObjectId, local: &mut Local) -> Loaded {
        local.counts.versions += 1;
        self.cache.get(id, || {
            let start = Instant::now();
            let bytes = self.git.read_blob(id);
            local.steps.read += start.elapsed();
            parse(bytes.map_err(Failure::Git)?, self.rules, local)
        })
    }

    /// A working-tree version. Its bytes are never cached by blob id: the id
    /// is the one git would store, after line-ending conversion, and the
    /// bytes on disk may differ from it.
    fn worktree_file(&self, path: &RepoPath, local: &mut Local) -> Loaded {
        local.counts.versions += 1;
        let start = Instant::now();
        let bytes = self.git.read_worktree_file(path);
        local.steps.read += start.elapsed();
        parse(bytes.map_err(Failure::Git)?, self.rules, local)
    }
}

/// Parses one version with the masthead keys the rules file knows, so its
/// title, summary and masthead read as the rules say.
fn parse(bytes: Vec<u8>, rules: &Rules, local: &mut Local) -> Loaded {
    let start = Instant::now();
    let parsed = Document::parse_with(bytes, rules.masthead_keys());
    local.steps.parse += start.elapsed();
    local.counts.parsed += 1;
    parsed.map(Arc::new).map_err(Failure::Parse)
}

fn version<'a>(
    document: Option<&'a Document>,
    classification: Option<&'a Classification>,
) -> Option<Version<'a>> {
    let (document, classification) = document.zip(classification)?;
    Some(Version {
        document,
        classification,
    })
}

/// Parsed documents by blob id, for one run.
///
/// It knows up front how many changes need each blob. The first to ask
/// loads it; others asking at the same time wait for that load instead of
/// starting their own. The last one to ask takes the entry out, so a parsed
/// document is freed as soon as no change still needs it.
struct Cache {
    slots: Mutex<HashMap<ObjectId, Slot>>,
}

struct Slot {
    cell: Arc<OnceLock<Loaded>>,
    users: usize,
}

impl Cache {
    fn new<'a>(ids: impl IntoIterator<Item = &'a ObjectId>) -> Cache {
        let mut slots: HashMap<ObjectId, Slot> = HashMap::new();
        for id in ids {
            let slot = slots.entry(id.clone()).or_insert_with(|| Slot {
                cell: Arc::default(),
                users: 0,
            });
            slot.users += 1;
        }
        Cache {
            slots: Mutex::new(slots),
        }
    }

    /// The blob's document, loaded with `load` unless another change loaded
    /// it first. A blob the cache was not told about is loaded each time.
    fn get(&self, id: &ObjectId, load: impl FnOnce() -> Loaded) -> Loaded {
        let cell = {
            let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
            match slots.get_mut(id) {
                Some(slot) => {
                    slot.users -= 1;
                    let cell = Arc::clone(&slot.cell);
                    if slot.users == 0 {
                        slots.remove(id);
                    }
                    cell
                }
                None => Arc::default(),
            }
        };
        cell.get_or_init(load).clone()
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(digit: char) -> ObjectId {
        ObjectId::from_hex(&digit.to_string().repeat(40)).unwrap()
    }

    #[expect(clippy::unnecessary_wraps, reason = "it stands in for a load")]
    fn document(text: &str) -> Loaded {
        Ok(Arc::new(Document::parse(text.as_bytes().to_vec()).unwrap()))
    }

    #[test]
    fn a_shared_blob_is_loaded_once_and_dropped_after_its_last_user() {
        let (a, b) = (id('a'), id('b'));
        let cache = Cache::new([&a, &b, &a]);
        assert_eq!(cache.len(), 2);
        let loads = AtomicUsize::new(0);
        let load = || {
            loads.fetch_add(1, Ordering::Relaxed);
            document("# A\n")
        };
        let first = cache.get(&a, load).ok().unwrap();
        assert_eq!(cache.len(), 2, "a has one more user");
        let second = cache.get(&a, load).ok().unwrap();
        assert!(Arc::ptr_eq(&first, &second), "the same parsed document");
        assert_eq!(loads.load(Ordering::Relaxed), 1);
        assert_eq!(cache.len(), 1, "a is gone after its last user");
        let _ = cache.get(&b, load);
        assert_eq!(cache.len(), 0);
        let _ = cache.get(&b, load);
        assert_eq!(
            loads.load(Ordering::Relaxed),
            3,
            "an unknown blob loads again"
        );
    }

    #[test]
    fn threads_asking_at_once_share_one_load() {
        let a = id('a');
        let ids = vec![&a; 8];
        let cache = Cache::new(ids);
        let loads = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    let _ = cache.get(&a, || {
                        loads.fetch_add(1, Ordering::Relaxed);
                        std::thread::sleep(Duration::from_millis(5));
                        document("# A\n")
                    });
                });
            }
        });
        assert_eq!(loads.load(Ordering::Relaxed), 1);
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn a_failed_load_is_shared_too() {
        let a = id('a');
        let cache = Cache::new([&a, &a]);
        let missing = || Err(Failure::Git(GitError::MissingObject { id: id('a') }));
        assert!(cache.get(&a, missing).is_err());
        assert!(
            cache.get(&a, || document("# never loaded\n")).is_err(),
            "the second user gets the first answer"
        );
    }
}
