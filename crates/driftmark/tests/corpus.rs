//! Both readers on a real repo, for the check the M1 brief asks for. It is
//! ignored in normal runs because it needs a repo that is not in this one.
//!
//! Run it with:
//!
//! ```text
//! DRIFTMARK_CORPUS=/path/to/repo \
//! DRIFTMARK_CORPUS_ROOTS=docs \
//! DRIFTMARK_CORPUS_RANGES='commit=A~1..A;branch=B...C;days=since:2026-09-18:origin/main' \
//! cargo test --release -p driftmark --test corpus -- --ignored --nocapture
//! ```
//!
//! It prints counts and timings only, never a path, a name or an id.

mod support;

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use driftmark::git::{CommandReader, GixReader};
use driftmark_core::git::{
    Branch, FileChange, FileEntry, GitError, GitRead, ObjectId, RangeSpec, RepoPath, ResolvedRange,
    Roots, Snapshot,
};

/// Counts the commits a range walk reads.
struct Counting<'a> {
    inner: &'a dyn GitRead,
    commits: AtomicUsize,
}

impl GitRead for Counting<'_> {
    fn resolve(&self, name: &str) -> Result<ObjectId, GitError> {
        self.inner.resolve(name)
    }
    fn merge_bases(&self, a: &ObjectId, b: &ObjectId) -> Result<Vec<ObjectId>, GitError> {
        self.inner.merge_bases(a, b)
    }
    fn read_commit(&self, id: &ObjectId) -> Result<Vec<u8>, GitError> {
        self.commits.fetch_add(1, Ordering::Relaxed);
        self.inner.read_commit(id)
    }
    fn markdown_files(&self, at: &Snapshot, roots: &Roots) -> Result<Vec<FileEntry>, GitError> {
        self.inner.markdown_files(at, roots)
    }
    fn markdown_changes(
        &self,
        a: &ObjectId,
        b: &ObjectId,
        roots: &Roots,
    ) -> Result<Vec<FileChange>, GitError> {
        self.inner.markdown_changes(a, b, roots)
    }
    fn blob_at(&self, commit: &ObjectId, path: &RepoPath) -> Result<Option<ObjectId>, GitError> {
        self.inner.blob_at(commit, path)
    }
    fn branches(&self) -> Result<Vec<Branch>, GitError> {
        self.inner.branches()
    }
    fn read_blob(&self, id: &ObjectId) -> Result<Vec<u8>, GitError> {
        self.inner.read_blob(id)
    }
    fn read_worktree_file(&self, path: &RepoPath) -> Result<Vec<u8>, GitError> {
        self.inner.read_worktree_file(path)
    }
}

/// What one reader answered for one range, and how long each step took.
struct Answer {
    range: ResolvedRange,
    changes: Vec<FileChange>,
    files_at_head: Vec<FileEntry>,
    blobs: Vec<Vec<u8>>,
    commits_walked: usize,
    resolve: Duration,
    list: Duration,
    list_all: Duration,
    read: Duration,
}

fn ask(reader: &dyn GitRead, spec: &RangeSpec, roots: &Roots) -> Answer {
    let counting = Counting {
        inner: reader,
        commits: AtomicUsize::new(0),
    };
    let start = Instant::now();
    let range = spec.resolve(&counting).expect("the range resolves");
    let resolve = start.elapsed();
    let start = Instant::now();
    let changes = range.changes(reader, roots).expect("changes");
    let list = start.elapsed();
    let start = Instant::now();
    let files_at_head = reader.markdown_files(&range.head, roots).expect("files");
    let list_all = start.elapsed();
    let start = Instant::now();
    let ids = changes
        .iter()
        .flat_map(|c| [c.old_blob(), c.new_blob()])
        .flatten();
    let blobs: Vec<Vec<u8>> = ids
        .map(|id| reader.read_blob(id).expect("a blob"))
        .collect();
    let read = start.elapsed();
    let commits_walked = counting.commits.load(Ordering::Relaxed);
    Answer {
        range,
        changes,
        files_at_head,
        blobs,
        commits_walked,
        resolve,
        list,
        list_all,
        read,
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

#[test]
#[ignore = "needs DRIFTMARK_CORPUS, a repo outside this one"]
fn both_readers_agree_on_a_real_repo() {
    let repo = PathBuf::from(std::env::var("DRIFTMARK_CORPUS").expect("DRIFTMARK_CORPUS"));
    let roots = std::env::var("DRIFTMARK_CORPUS_ROOTS").unwrap_or_default();
    let roots = Roots::new(roots.split(',').filter(|r| !r.is_empty())).expect("roots");
    let ranges = std::env::var("DRIFTMARK_CORPUS_RANGES").expect("DRIFTMARK_CORPUS_RANGES");
    let runs: usize =
        std::env::var("DRIFTMARK_CORPUS_RUNS").map_or(3, |r| r.parse().expect("a number"));

    let open = Instant::now();
    let gix = GixReader::open(&repo).expect("gix opens");
    let gix_open = open.elapsed();
    let open = Instant::now();
    let command = CommandReader::open(&repo).expect("git opens");
    let command_open = open.elapsed();
    println!(
        "open ms: gix {:.2}, git {:.2}",
        ms(gix_open),
        ms(command_open)
    );

    for item in ranges.split(';').filter(|r| !r.is_empty()) {
        let (label, text) = item.split_once('=').expect("label=range");
        let spec = match text.strip_prefix("since:") {
            Some(rest) => {
                let (date, branch) = rest.split_once(':').expect("since:date:branch");
                RangeSpec::since(date, Some(branch)).expect("a date")
            }
            None => RangeSpec::parse(text).expect("a range"),
        };

        // A first pass warms the caches and gives the answers to compare.
        let from_gix = ask(&gix, &spec, &roots);
        let from_command = ask(&command, &spec, &roots);
        assert_eq!(from_gix.range, from_command.range, "{label}: ranges differ");
        assert_eq!(
            from_gix.changes, from_command.changes,
            "{label}: changes differ"
        );
        assert_eq!(
            from_gix.files_at_head, from_command.files_at_head,
            "{label}: files differ"
        );
        assert!(
            from_gix.blobs == from_command.blobs,
            "{label}: blob bytes differ"
        );
        assert_eq!(from_gix.commits_walked, from_command.commits_walked);

        let count =
            |kind: fn(&FileChange) -> bool| from_gix.changes.iter().filter(|c| kind(c)).count();
        let added = count(|c| matches!(c, FileChange::Added { .. }));
        let removed = count(|c| matches!(c, FileChange::Removed { .. }));
        let modified = count(|c| matches!(c, FileChange::Modified { .. }));
        let bytes: usize = from_gix.blobs.iter().map(Vec::len).sum();
        let megabytes =
            u32::try_from(bytes / 1000).map_or(f64::INFINITY, |kb| f64::from(kb) / 1000.0);
        println!(
            "{label}: identical; changed {} (modified {modified}, added {added}, removed {removed}); \
             blobs {} ({:.1} MB); files at head {}; commits walked {}; notes {}",
            from_gix.changes.len(),
            from_gix.blobs.len(),
            megabytes,
            from_gix.files_at_head.len(),
            from_gix.commits_walked,
            from_gix.range.notes.len(),
        );

        for (name, reader) in [("gix", &gix as &dyn GitRead), ("git", &command)] {
            let answers: Vec<Answer> = (0..runs).map(|_| ask(reader, &spec, &roots)).collect();
            let step =
                |f: fn(&Answer) -> Duration| median(answers.iter().map(|a| ms(f(a))).collect());
            println!(
                "{label} {name} ms (median of {runs}): resolve {:.2}, list changes {:.2}, \
                 list all files {:.2}, read blobs {:.2}",
                step(|a| a.resolve),
                step(|a| a.list),
                step(|a| a.list_all),
                step(|a| a.read),
            );
        }
    }
}
