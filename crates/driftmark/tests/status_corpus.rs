//! The status view on a real repo, for the checks the M5 status brief asks
//! for. It is ignored in normal runs because it needs a repo that is not in
//! this one.
//!
//! It reads the repo through the git readers only, so it never changes it,
//! and it prints counts and timings only: never a path, a title or any text
//! from a file.
//!
//! ```text
//! DRIFTMARK_CORPUS=/path/to/repo DRIFTMARK_CORPUS_REF=origin/main \
//! cargo test --release -p driftmark --test status_corpus -- --ignored --nocapture
//! ```

use std::path::PathBuf;
use std::time::Instant;

use driftmark::git::{CommandReader, GixReader};
use driftmark_core::git::{GitRead, Roots};

fn corpus() -> (PathBuf, String) {
    let repo = std::env::var_os("DRIFTMARK_CORPUS").expect("DRIFTMARK_CORPUS names a repo");
    let reference = std::env::var("DRIFTMARK_CORPUS_REF").unwrap_or_else(|_| "HEAD".to_owned());
    (PathBuf::from(repo), reference)
}

/// The CPU time this process used so far, in milliseconds, from `/proc`.
fn cpu_ms() -> f64 {
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    let fields: Vec<&str> = stat
        .rsplit_once(')')
        .map_or("", |(_, rest)| rest)
        .split_whitespace()
        .collect();
    let ticks = |at: usize| {
        fields
            .get(at)
            .and_then(|f| f.parse::<f64>().ok())
            .unwrap_or(0.0)
    };
    (ticks(11) + ticks(12)) * 10.0
}

fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

#[test]
#[ignore = "needs DRIFTMARK_CORPUS"]
fn history_walk_timings() {
    let (repo, reference) = corpus();
    let gix = GixReader::open(&repo).expect("gix opens the corpus");
    let command = CommandReader::open(&repo).expect("git opens the corpus");
    let head = gix.resolve(&reference).expect("the ref resolves");
    for round in 0..3 {
        let start = Instant::now();
        let cpu = cpu_ms();
        let from_gix = gix.last_changes(&head, &Roots::all()).expect("gix walks");
        let gix_ms = ms(start);
        let gix_cpu = cpu_ms() - cpu;
        let start = Instant::now();
        let from_command = command
            .last_changes(&head, &Roots::all())
            .expect("git walks");
        let command_ms = ms(start);
        assert_eq!(from_gix, from_command, "the readers agree");
        println!(
            "round {round}: {} files, gix {gix_ms:.0} ms ({gix_cpu:.0} ms of CPU), git command {command_ms:.0} ms",
            from_gix.len()
        );
    }
}

#[test]
#[ignore = "needs DRIFTMARK_CORPUS"]
fn reading_every_document_timings() {
    use driftmark::git::Git;
    use driftmark::search::{File, Read};
    use driftmark_core::git::Snapshot;
    use driftmark_core::presets::Setup;

    let (repo, reference) = corpus();
    let git = Git::open(&repo).expect("the corpus opens");
    let head = git.resolve(&reference).expect("the ref resolves");
    let side = Snapshot::Commit(head);
    let beside = driftmark::presets::files_at(&git, &side);
    let rules = Setup::build(None, &beside)
        .expect("the presets build")
        .rules;
    let files: Vec<File> = git
        .markdown_files(&side, &Roots::all())
        .expect("files")
        .into_iter()
        .map(|file| File {
            path: file.path,
            read: Read::Blob(file.id),
        })
        .collect();
    let threads = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    for round in 0..3 {
        let start = Instant::now();
        let cpu = cpu_ms();
        let (entries, unread) =
            driftmark::status::entries(&git, &files, &rules, &beside, threads).expect("read");
        println!(
            "round {round}: {} entries, {} unread, {:.0} ms ({:.0} ms of CPU) on {threads} threads",
            entries.len(),
            unread.len(),
            ms(start),
            cpu_ms() - cpu
        );
    }
}

#[test]
#[ignore = "needs DRIFTMARK_CORPUS"]
fn the_head_reads_like_the_whole_document() {
    use driftmark::git::Git;
    use driftmark_core::document::Document;
    use driftmark_core::git::Snapshot;
    use driftmark_core::presets::Setup;
    use driftmark_core::status::{Entry, needed};

    let (repo, reference) = corpus();
    let git = Git::open(&repo).expect("the corpus opens");
    let head = git.resolve(&reference).expect("the ref resolves");
    let side = Snapshot::Commit(head);
    let beside = driftmark::presets::files_at(&git, &side);
    let rules = Setup::build(None, &beside)
        .expect("the presets build")
        .rules;
    let (mut cut, mut same, mut different, mut whole_bytes, mut head_bytes) = (0, 0, 0, 0, 0);
    for file in git.markdown_files(&side, &Roots::all()).expect("files") {
        let bytes = git.read_blob(&file.id).expect("a blob");
        let path = file.path.to_lossy().into_owned();
        let part = needed(&bytes, rules.type_for(&path)).to_vec();
        whole_bytes += bytes.len();
        head_bytes += part.len();
        if part.len() == bytes.len() {
            continue;
        }
        cut += 1;
        let entry = |bytes: Vec<u8>| {
            Document::parse_with(bytes, rules.masthead_keys())
                .ok()
                .map(|document| {
                    let class = rules.classify_with(&path, &document, &beside);
                    Entry::new(path.clone(), document.meta.title, class)
                })
        };
        if entry(bytes) == entry(part) {
            same += 1;
        } else {
            different += 1;
        }
    }
    println!(
        "{cut} documents cut to their head, {same} read the same, {different} differ; {head_bytes} of {whole_bytes} bytes parsed"
    );
    assert_eq!(different, 0);
}
