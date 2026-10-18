//! Giving memory back after bulk work (decision 0011, items 3 and 4).
//!
//! A bulk job runs on every core: counting a range, the home page's branches
//! or the working tree, or building a search index. It leaves two things
//! behind in a long-running process:
//!
//! - a spare gix reader for each thread it ran on, each keeping caches of up
//!   to 20 MB for reuse ([`crate::git::GixReader::drop_spare`]);
//! - on Linux with glibc, the memory its threads freed, which glibc keeps in
//!   each thread's own pool instead of handing it back to the system.
//!
//! Page requests leave freed memory behind too, a little each. So
//! `driftmark serve` and `driftmark mcp` call [`give_back`] after each bulk
//! job, and an [`Idle`] watcher calls it once no request ran for
//! [`IDLE_AFTER`]. The one-shot commands, `diff` and `lint`, exit right
//! after their bulk job, which frees everything.

use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::git::Git;

/// How long the program must have had no request before the [`Idle`]
/// watcher gives memory back.
pub(crate) const IDLE_AFTER: Duration = Duration::from_secs(1);

/// Gives back what bulk work left: the spare gix readers, and on Linux with
/// glibc the memory the threads freed.
pub(crate) fn give_back(git: &Git) {
    git.drop_spare_readers();
    trim();
}

// The one `unsafe` in driftmark (decision 0011, item 4): the workspace lint
// `unsafe_code` is `deny`, and only this block allows it.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
#[allow(unsafe_code)]
unsafe extern "C" {
    /// glibc's `int malloc_trim(size_t pad)` from `<malloc.h>`: it hands
    /// every whole free page in every one of glibc's pools back to the
    /// system.
    ///
    /// Declaring it `safe`, so that callers need no `unsafe`, is sound:
    ///
    /// - The declaration is glibc's own prototype. `usize` is `size_t` and
    ///   `c_int` is `int` on every target Rust builds for glibc.
    /// - It takes a number and no pointer, so it cannot read or write memory
    ///   it was not given.
    /// - It has no precondition and is thread-safe: glibc's manual lists it
    ///   as MT-Safe, and it locks each pool while it trims it, so other
    ///   threads may allocate meanwhile.
    /// - The `cfg` limits it to Linux with glibc, where every version Rust
    ///   supports (2.17 and later) has the symbol and trims every pool (since
    ///   2.8). Rust's allocator there is glibc's `malloc`, so the trim reaches
    ///   the memory Rust freed; under another allocator it would stay safe
    ///   and return little.
    safe fn malloc_trim(pad: usize) -> std::ffi::c_int;
}

/// Asks the C library to hand freed memory back to the system:
/// `malloc_trim(0)` on Linux with glibc, nothing elsewhere. `Some(true)` when
/// glibc handed memory back, `Some(false)` when it had none to give, `None`
/// where no such call exists.
#[allow(
    clippy::unnecessary_wraps,
    reason = "`None` is the answer on every system without glibc; one build sees one system"
)]
pub(crate) fn trim() -> Option<bool> {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        Some(malloc_trim(0) == 1)
    }
    #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
    {
        None
    }
}

/// Watches for work and runs a release once no work ran for a while, then
/// waits for the next work. The watching thread stops when this is dropped.
pub(crate) struct Idle {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    wake: Condvar,
}

#[derive(Default)]
struct State {
    /// Work that began and has not ended.
    busy: usize,
    /// When the last work ended, while no release followed it yet.
    ended: Option<Instant>,
    /// Set when the [`Idle`] is dropped.
    stop: bool,
}

fn lock(shared: &Shared) -> MutexGuard<'_, State> {
    shared.state.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Work in progress for an [`Idle`] watcher; it ends when dropped.
pub(crate) struct Busy(Arc<Shared>);

impl Idle {
    /// Starts watching: `release` runs on a thread of its own, `after` the
    /// end of the last work, once per quiet spell.
    pub(crate) fn start(after: Duration, release: impl Fn() + Send + 'static) -> Idle {
        let shared = Arc::new(Shared::default());
        let watched = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("driftmark-idle".to_owned())
            .spawn(move || watch(&watched, after, &release))
            .map_err(|error| {
                eprintln!("driftmark: memory will not be given back when idle: {error}");
            })
            .ok();
        Idle { shared, thread }
    }

    /// Marks work as running until the returned value is dropped. No
    /// release runs while any work runs.
    pub(crate) fn begin(&self) -> Busy {
        lock(&self.shared).busy += 1;
        Busy(Arc::clone(&self.shared))
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        let mut state = lock(&self.0);
        state.busy = state.busy.saturating_sub(1);
        if state.busy == 0 {
            state.ended = Some(Instant::now());
            self.0.wake.notify_all();
        }
    }
}

impl Drop for Idle {
    fn drop(&mut self) {
        lock(&self.shared).stop = true;
        self.shared.wake.notify_all();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The watching thread: sleeps until work ended, waits `after` while no new
/// work begins, releases, and sleeps again.
fn watch(shared: &Shared, after: Duration, release: &dyn Fn()) {
    let mut state = lock(shared);
    loop {
        if state.stop {
            return;
        }
        let quiet_since = state.ended.filter(|_| state.busy == 0);
        state = match quiet_since {
            None => shared
                .wake
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner),
            Some(ended) if ended.elapsed() >= after => {
                state.ended = None;
                drop(state);
                release();
                lock(shared)
            }
            Some(ended) => {
                shared
                    .wake
                    .wait_timeout(state, after.saturating_sub(ended.elapsed()))
                    .unwrap_or_else(PoisonError::into_inner)
                    .0
            }
        };
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use driftmark_core::git::{GitRead, ObjectId, ResolvedRange, Snapshot};

    use super::*;
    use crate::serve::repo::Repo;

    #[test]
    fn the_trim_runs_only_with_glibc() {
        let trimmed = trim();
        if cfg!(all(target_os = "linux", target_env = "gnu")) {
            assert!(trimmed.is_some(), "glibc's malloc_trim runs");
        } else {
            assert_eq!(trimmed, None, "no trim without glibc");
        }
    }

    /// Waits up to 10 seconds for `count` to reach `want`.
    fn reaches(count: &AtomicUsize, want: usize) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if count.load(Ordering::SeqCst) == want {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    #[test]
    fn the_idle_watcher_releases_once_after_work_and_never_during_it() {
        let releases = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&releases);
        let idle = Idle::start(Duration::from_millis(20), move || {
            counted.fetch_add(1, Ordering::SeqCst);
        });

        // Nothing ran yet, so nothing is released.
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(releases.load(Ordering::SeqCst), 0);

        // Long work: no release while it runs, one after it.
        let busy = idle.begin();
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(releases.load(Ordering::SeqCst), 0);
        drop(busy);
        assert!(reaches(&releases, 1), "one release after the work");

        // A quiet spell releases once, not again and again.
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(releases.load(Ordering::SeqCst), 1);

        // Overlapping work counts as one spell.
        let first = idle.begin();
        let second = idle.begin();
        drop(first);
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(releases.load(Ordering::SeqCst), 1, "work still runs");
        drop(second);
        assert!(reaches(&releases, 2), "one release after both");

        drop(idle);
    }

    /// A small repo made with the `git` command, removed when dropped:
    /// `main` holds ten documents, and the branch `feature` changes each.
    pub(crate) struct Scratch {
        root: PathBuf,
        pub dir: PathBuf,
        pub main: ObjectId,
        pub feature: ObjectId,
    }

    impl Scratch {
        pub(crate) fn new(label: &str) -> Scratch {
            let root = std::env::temp_dir()
                .join(format!("driftmark-memory-{label}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).expect("make a temporary folder");
            let config = root.join("gitconfig");
            std::fs::write(&config, "").expect("write an empty git config");
            let dir = root.join("repo");
            let git = |args: &[&str]| -> String {
                let output = Command::new("git")
                    .arg("-C")
                    .arg(if args[0] == "init" { &root } else { &dir })
                    .args(args)
                    .env("GIT_CONFIG_NOSYSTEM", "1")
                    .env("GIT_CONFIG_GLOBAL", &config)
                    .env("GIT_AUTHOR_NAME", "Test Author")
                    .env("GIT_AUTHOR_EMAIL", "author@example.com")
                    .env("GIT_COMMITTER_NAME", "Test Committer")
                    .env("GIT_COMMITTER_EMAIL", "committer@example.com")
                    .env("GIT_AUTHOR_DATE", "@1767225600 +0000")
                    .env("GIT_COMMITTER_DATE", "@1767225600 +0000")
                    .env_remove("GIT_DIR")
                    .env_remove("GIT_WORK_TREE")
                    .env_remove("GIT_INDEX_FILE")
                    .output()
                    .expect("run git");
                assert!(
                    output.status.success(),
                    "git {args:?}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                String::from_utf8_lossy(&output.stdout).trim().to_owned()
            };
            git(&["init", "-q", "-b", "main", "repo"]);
            git(&["config", "core.autocrlf", "false"]);
            let write = |version: &str| {
                for n in 0..10 {
                    let text = format!(
                        "# Document {n}\n\n> Short version: one of ten.\n\n## Decision\n\nKeep {version} copies of page {n}.\n"
                    );
                    std::fs::write(dir.join(format!("doc-{n}.md")), text).expect("write a file");
                }
            };
            write("two");
            git(&["add", "-A"]);
            git(&["commit", "-q", "-m", "first"]);
            let main = git(&["rev-parse", "HEAD"]);
            git(&["switch", "-q", "-c", "feature"]);
            write("three");
            git(&["commit", "-q", "-am", "second"]);
            let feature = git(&["rev-parse", "HEAD"]);
            git(&["switch", "-q", "main"]);
            Scratch {
                root,
                dir,
                main: ObjectId::from_hex(&main).expect("a commit id"),
                feature: ObjectId::from_hex(&feature).expect("a commit id"),
            }
        }

        /// The repo as the server reads it, comparing on 4 threads.
        pub(crate) fn repo(&self) -> Repo {
            let git = Git::open(&self.dir).expect("open the repo");
            Repo::new(git, 8 * 1024 * 1024, 4, Some("main".to_owned()))
        }

        /// Reads a document on this thread, which leaves one spare reader.
        pub(crate) fn read_one(&self, repo: &Repo) {
            let path = driftmark_core::git::RepoPath::new("doc-0.md").expect("a path");
            repo.git
                .blob_at(&self.main, &path)
                .expect("read the tree")
                .expect("the document is there");
            assert_eq!(
                repo.git.spare_readers(),
                1,
                "a single read keeps its reader"
            );
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn bulk_jobs_drop_the_spare_readers() {
        let scratch = Scratch::new("bulk");
        let repo = scratch.repo();

        scratch.read_one(&repo);
        let range = ResolvedRange {
            base: Some(scratch.main.clone()),
            head: Snapshot::Commit(scratch.feature.clone()),
            notes: Vec::new(),
        };
        let count = repo.range_count(&range).expect("count the range");
        assert_eq!(count.list.documents.len(), 10);
        assert_eq!(repo.git.spare_readers(), 0, "after a range count");

        scratch.read_one(&repo);
        let home = repo.home().expect("count the branches");
        assert_eq!(home.changed.len(), 1);
        assert_eq!(repo.git.spare_readers(), 0, "after the branch count");

        scratch.read_one(&repo);
        repo.count_worktree().expect("count the working tree");
        assert_eq!(repo.git.spare_readers(), 0, "after the working-tree count");

        scratch.read_one(&repo);
        crate::serve::search::commit_layer(&repo, &scratch.feature).expect("index a commit");
        assert_eq!(repo.git.spare_readers(), 0, "after a search index build");

        // A page that only reads keeps its reader for the next page.
        scratch.read_one(&repo);
    }
}
