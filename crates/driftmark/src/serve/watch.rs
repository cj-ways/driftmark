//! Watching the working tree, so open pages can reload when it changes.
//!
//! The watcher polls instead of asking the system for file events. A file
//! event library would add a licence that `deny.toml` does not allow yet,
//! and polling looks only at what git lists, never at build folders or the
//! `.git` folder, which the git reader keeps to itself.
//!
//! Once a second it checks the size and time of every Markdown file git
//! lists in the working tree, of the folders that hold them, and of the
//! rules file. A file that changed is announced at once. A folder that
//! changed may hold a new or removed file, so the working tree is listed
//! again through the git reader, which follows the ignore rules.
//!
//! Listing the working tree is slow in a large repo, because git's ids are
//! hashes of every file. So it runs here, in the background: each listing
//! also counts the `auto` range for the home page, which shows the last
//! count instead of waiting for a new one.
//!
//! A new file in a folder that held no Markdown file before is found by the
//! next listing: after another change, or within [`SAFETY_RECOUNT`] while a
//! page is open.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use driftmark_core::diff::RangeInfo;
use driftmark_core::git::RepoPath;
use tokio::sync::broadcast;

use super::repo::{RangeCount, Repo};
use crate::git::worktree::os_str;
use crate::lint::RULES_FILE;

/// How often the files are checked.
pub(crate) const POLL: Duration = Duration::from_secs(1);

/// How often the working tree is listed again while a page is open, to find
/// a new file in a folder that held no Markdown file.
pub(crate) const SAFETY_RECOUNT: Duration = Duration::from_secs(60);

/// What open pages are told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Message {
    /// These working-tree files changed, came or went.
    Changed(Vec<String>),
    /// The working tree was counted again for the home page.
    Counted,
}

/// The last count of the working tree.
pub(crate) struct Latest {
    /// When it finished, in seconds since 1970-01-01 00:00 UTC.
    pub counted_at: i64,
    /// How long it took.
    pub took: Duration,
    /// When it finished, for deciding whether it is old.
    finished: Instant,
    /// The `auto` range and its counts, or why it could not be counted.
    pub counted: Result<(RangeInfo, Arc<RangeCount>), String>,
}

/// The size, time and kind of a file or folder, or `None` when nothing is
/// there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    len: u64,
    modified: Option<SystemTime>,
    file: bool,
}

fn stamp(path: &Path) -> Option<Stamp> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    Some(Stamp {
        len: meta.len(),
        modified: meta.modified().ok(),
        file: meta.is_file(),
    })
}

/// What the watcher knows of the working tree.
#[derive(Default)]
struct Known {
    /// Whether the working tree was listed at least once.
    listed: bool,
    files: BTreeMap<RepoPath, Option<Stamp>>,
    folders: BTreeMap<PathBuf, Option<Stamp>>,
    rules: Option<Stamp>,
}

/// What one check found.
#[derive(Default)]
struct Found {
    paths: Vec<String>,
    folders: bool,
}

/// Watches the working tree of one repo.
pub(crate) struct Watch {
    repo: Arc<Repo>,
    workdir: PathBuf,
    events: broadcast::Sender<Message>,
    known: Mutex<Known>,
    latest: Mutex<Option<Arc<Latest>>>,
    counting: AtomicBool,
    /// Set when a page asks for a new count; the condition variable wakes the
    /// watcher early.
    wanted: Mutex<bool>,
    wake: Condvar,
    stop: AtomicBool,
    /// How many pages listen for events now.
    pub clients: AtomicUsize,
    /// How many changes were announced.
    generation: AtomicU64,
}

impl Watch {
    /// A watcher for the working tree at `workdir`. Nothing runs until
    /// [`Watch::start`].
    pub(crate) fn new(
        repo: Arc<Repo>,
        workdir: PathBuf,
        events: broadcast::Sender<Message>,
    ) -> Watch {
        Watch {
            repo,
            workdir,
            events,
            known: Mutex::default(),
            latest: Mutex::default(),
            counting: AtomicBool::new(true),
            wanted: Mutex::new(false),
            wake: Condvar::new(),
            stop: AtomicBool::new(false),
            clients: AtomicUsize::new(0),
            generation: AtomicU64::new(0),
        }
    }

    /// Starts watching on a thread of its own. The first thing it does is
    /// count the working tree.
    pub(crate) fn start(self: &Arc<Self>) {
        let watch = Arc::clone(self);
        std::thread::Builder::new()
            .name("driftmark-watch".to_owned())
            .spawn(move || watch.run())
            .map_or_else(
                |error| eprintln!("driftmark serve: pages will not reload by themselves: {error}"),
                drop,
            );
    }

    /// Stops the thread at its next wake-up.
    pub(crate) fn stop(&self) {
        self.stop.store(true, Ordering::Release);
        self.wake.notify_all();
    }

    /// Asks for a new count soon, unless one is running.
    pub(crate) fn request_count(&self) {
        *self.wanted.lock().unwrap_or_else(PoisonError::into_inner) = true;
        self.wake.notify_all();
    }

    /// The last finished count, if any.
    pub(crate) fn latest(&self) -> Option<Arc<Latest>> {
        self.latest
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Whether a count is running now.
    pub(crate) fn counting(&self) -> bool {
        self.counting.load(Ordering::Acquire)
    }

    /// Whether the last count is older than [`SAFETY_RECOUNT`].
    pub(crate) fn latest_is_old(&self) -> bool {
        self.latest()
            .is_none_or(|latest| latest.finished.elapsed() > SAFETY_RECOUNT)
    }

    /// How many changes were announced, and how many files are watched.
    pub(crate) fn state(&self) -> (u64, usize, bool) {
        let known = self.known.lock().unwrap_or_else(PoisonError::into_inner);
        (
            self.generation.load(Ordering::Acquire),
            known.files.len(),
            known.listed,
        )
    }

    fn run(&self) {
        self.count();
        while !self.stop.load(Ordering::Acquire) {
            let wanted = {
                let wanted = self.wanted.lock().unwrap_or_else(PoisonError::into_inner);
                let (mut wanted, _) = self
                    .wake
                    .wait_timeout_while(wanted, POLL, |wanted| {
                        !*wanted && !self.stop.load(Ordering::Acquire)
                    })
                    .unwrap_or_else(PoisonError::into_inner);
                std::mem::take(&mut *wanted)
            };
            if self.stop.load(Ordering::Acquire) {
                return;
            }
            let found = self.check();
            let changed = !found.paths.is_empty();
            if changed {
                self.announce(found.paths);
            }
            let open = self.clients.load(Ordering::Acquire) > 0;
            if wanted || changed || found.folders || (open && self.latest_is_old()) {
                self.count();
            }
        }
    }

    /// Compares every watched file and folder with its last stamp.
    fn check(&self) -> Found {
        let mut known = self.known.lock().unwrap_or_else(PoisonError::into_inner);
        let mut found = Found::default();
        for (path, last) in &mut known.files {
            let now = full_path(&self.workdir, path).and_then(|full| stamp(&full));
            if now != *last {
                *last = now;
                found.paths.push(path.to_lossy().into_owned());
            }
        }
        for (folder, last) in &mut known.folders {
            let now = stamp(folder);
            if now != *last {
                *last = now;
                found.folders = true;
            }
        }
        let rules = stamp(&self.workdir.join(RULES_FILE));
        if rules != known.rules {
            known.rules = rules;
            found.paths.push(RULES_FILE.to_owned());
        }
        found
    }

    /// Lists and counts the working tree, then watches what it lists.
    ///
    /// Files listed before keep their stamps from before the listing, so an
    /// edit made while it ran is found by the next check.
    fn count(&self) {
        self.counting.store(true, Ordering::Release);
        let started = Instant::now();
        match self.repo.count_worktree() {
            Ok(count) => {
                let listed: BTreeSet<RepoPath> = count.files.into_iter().map(|f| f.path).collect();
                let came_or_went = self.watch_files(listed);
                if !came_or_went.is_empty() {
                    self.announce(came_or_went);
                }
                self.finish(
                    started,
                    count.counted.map(|(info, count)| (info, Arc::new(count))),
                );
            }
            Err(problem) => self.finish(started, Err(problem.message)),
        }
        self.counting.store(false, Ordering::Release);
        let _ = self.events.send(Message::Counted);
    }

    /// Watches the listed files and their folders, and returns the files
    /// that came or went since the last listing.
    fn watch_files(&self, listed: BTreeSet<RepoPath>) -> Vec<String> {
        let mut known = self.known.lock().unwrap_or_else(PoisonError::into_inner);
        let before: BTreeSet<&RepoPath> = known.files.keys().collect();
        let came_or_went: Vec<String> = if known.listed {
            before
                .symmetric_difference(&listed.iter().collect())
                .map(|path| path.to_lossy().into_owned())
                .collect()
        } else {
            Vec::new()
        };
        let mut files = BTreeMap::new();
        let mut folders = BTreeMap::new();
        folders.insert(self.workdir.clone(), None);
        for path in listed {
            let last = known.files.get(&path).copied();
            let full = full_path(&self.workdir, &path);
            let mut folder = full.as_deref().and_then(Path::parent);
            while let Some(dir) = folder.filter(|dir| dir.starts_with(&self.workdir)) {
                folders.insert(dir.to_path_buf(), None);
                folder = dir.parent();
            }
            let now = last.unwrap_or_else(|| full.as_deref().and_then(stamp));
            files.insert(path, now);
        }
        for (folder, last) in &mut folders {
            *last = known
                .folders
                .get(folder)
                .copied()
                .unwrap_or_else(|| stamp(folder));
        }
        if !known.listed {
            known.rules = stamp(&self.workdir.join(RULES_FILE));
        }
        known.files = files;
        known.folders = folders;
        known.listed = true;
        came_or_went
    }

    fn finish(&self, started: Instant, counted: Result<(RangeInfo, Arc<RangeCount>), String>) {
        let counted_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| {
                i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
            });
        let latest = Latest {
            counted_at,
            took: started.elapsed(),
            finished: Instant::now(),
            counted,
        };
        *self.latest.lock().unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(latest));
    }

    fn announce(&self, paths: Vec<String>) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        let _ = self.events.send(Message::Changed(paths));
    }
}

/// Where a repo path is on disk, or `None` when a part cannot be an OS name
/// here.
fn full_path(workdir: &Path, path: &RepoPath) -> Option<PathBuf> {
    let mut full = workdir.to_path_buf();
    for part in path.as_bytes().split(|&b| b == b'/') {
        full.push(os_str(part)?);
    }
    Some(full)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamps_tell_files_folders_and_nothing_apart() {
        let dir = std::env::temp_dir().join(format!("driftmark-stamp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.md");
        std::fs::write(&file, "one").unwrap();
        let first = stamp(&file).unwrap();
        assert!(first.file);
        assert_eq!(first.len, 3);
        std::fs::write(&file, "three").unwrap();
        assert_ne!(stamp(&file), Some(first));
        assert!(!stamp(&dir).unwrap().file);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(stamp(&file), None);
    }

    #[test]
    fn repo_paths_become_paths_under_the_working_tree() {
        let path = RepoPath::new("docs/a b/c.md").unwrap();
        let full = full_path(Path::new("/top"), &path).unwrap();
        assert_eq!(
            full,
            Path::new("/top").join("docs").join("a b").join("c.md")
        );
    }
}
