//! The change list on a real repo, for the M1 time limit (decision 0008,
//! item 4). It is ignored in normal runs because it needs a repo that is not
//! in this one.
//!
//! Run it with:
//!
//! ```text
//! DRIFTMARK_CORPUS=/path/to/repo \
//! DRIFTMARK_CORPUS_ROOTS=docs \
//! DRIFTMARK_CORPUS_RANGES='branch=B...C;days=since:2026-09-18:origin/main' \
//! DRIFTMARK_CORPUS_THREADS=12,1 \
//! cargo test --release -p driftmark --test diff_corpus -- --ignored --nocapture
//! ```
//!
//! Each run reads both sides of every changed Markdown file with gix, parses
//! them, classifies them with the default rules, compares them and gathers the
//! change list, on the given number of threads. It prints counts and timings
//! only, never a path, a name, an id or any text.

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use driftmark::git::GixReader;
use driftmark_core::diff::{
    ChangeKind, ChangeList, DocumentChanges, Level, NORMATIVE_WORDS, RangeInfo, Version, View,
    compare,
};
use driftmark_core::document::Document;
use driftmark_core::git::{FileChange, GitRead, RangeSpec, Roots};
use driftmark_core::rules::{Classification, Rules};

/// The time each step took, summed over the threads.
#[derive(Default, Clone, Copy)]
struct Steps {
    read: Duration,
    parse: Duration,
    compare: Duration,
}

impl Steps {
    fn add(&mut self, other: Steps) {
        self.read += other.read;
        self.parse += other.parse;
        self.compare += other.compare;
    }
}

struct Run {
    list: ChangeList,
    blocks: (usize, usize),
    resolve_and_list: Duration,
    steps: Steps,
    gather: Duration,
    wall: Duration,
}

/// One file's change list, with its block counts and step times.
fn one_file(
    reader: &dyn GitRead,
    rules: &Rules,
    change: &FileChange,
) -> (DocumentChanges, (usize, usize), Steps) {
    let mut steps = Steps::default();
    let start = Instant::now();
    let read = |id| reader.read_blob(id).expect("a blob");
    let old_bytes = change.old_blob().map(read);
    let new_bytes = change.new_blob().map(read);
    steps.read = start.elapsed();
    let start = Instant::now();
    let parse = |bytes: Vec<u8>| Document::parse(bytes).expect("the corpus parses");
    let old = old_bytes.map(parse);
    let new = new_bytes.map(parse);
    steps.parse = start.elapsed();
    let start = Instant::now();
    let path = change.path().to_lossy().into_owned();
    let old_class = old.as_ref().map(|document| rules.classify(&path, document));
    let new_class = new.as_ref().map(|document| rules.classify(&path, document));
    let old_version = version(old.as_ref(), old_class.as_ref());
    let new_version = version(new.as_ref(), new_class.as_ref());
    let changes = compare(&path, old_version, new_version).expect("one side exists");
    steps.compare = start.elapsed();
    let count = |document: &Option<Document>| document.as_ref().map_or(0, |d| d.blocks.len());
    check(&changes, count(&old), count(&new));
    (changes, (count(&old), count(&new)), steps)
}

fn version<'a>(
    document: Option<&'a Document>,
    classification: Option<&'a Classification>,
) -> Option<Version<'a>> {
    document
        .zip(classification)
        .map(|(document, classification)| Version {
            document,
            classification,
        })
}

/// The guarantees of the contract: every block once, changes in reading
/// order, and marks that spell the new text.
fn check(changes: &DocumentChanges, old_blocks: usize, new_blocks: usize) {
    let mut old_seen = vec![0_u8; old_blocks];
    let mut new_seen = vec![0_u8; new_blocks];
    for &[o, n] in &changes.unchanged {
        old_seen[o] += 1;
        new_seen[n] += 1;
    }
    let mut last = 0;
    for change in &changes.changes {
        if change.old.block.is_some() {
            old_seen[change.old.at] += 1;
        }
        if let Some(block) = &change.new.block {
            new_seen[change.new.at] += 1;
            if let Some(edit) = &change.edit {
                let shown: String = edit
                    .marks
                    .iter()
                    .chain(edit.sentences.iter().flatten())
                    .filter(|mark| mark.op != driftmark_core::diff::MarkOp::Deleted)
                    .map(|mark| mark.text.as_str())
                    .collect();
                assert!(
                    shown.is_empty() || shown == block.text,
                    "marks change the text"
                );
            }
        }
        assert!(change.new.at >= last, "changes out of reading order");
        last = change.new.at;
    }
    assert!(
        old_seen.iter().all(|&seen| seen == 1),
        "an old block is not listed once"
    );
    assert!(
        new_seen.iter().all(|&seen| seen == 1),
        "a new block is not listed once"
    );
}

fn run(reader: &GixReader, rules: &Rules, spec: &RangeSpec, roots: &Roots, threads: usize) -> Run {
    let wall = Instant::now();
    let range = spec.resolve(reader).expect("the range resolves");
    let changes = range.changes(reader, roots).expect("changes");
    let resolve_and_list = wall.elapsed();
    let next = AtomicUsize::new(0);
    let results = Mutex::new(Vec::with_capacity(changes.len()));
    let totals = Mutex::new((Steps::default(), (0, 0)));
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                let mut steps = Steps::default();
                let mut blocks = (0, 0);
                let mut mine = Vec::new();
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(change) = changes.get(index) else {
                        break;
                    };
                    let (document, counts, took) = one_file(reader, rules, change);
                    steps.add(took);
                    blocks.0 += counts.0;
                    blocks.1 += counts.1;
                    mine.push(document);
                }
                results.lock().expect("not poisoned").extend(mine);
                let mut totals = totals.lock().expect("not poisoned");
                totals.0.add(steps);
                totals.1.0 += blocks.0;
                totals.1.1 += blocks.1;
            });
        }
    });
    let start = Instant::now();
    let list = ChangeList::new(
        Some(RangeInfo::of(&range)),
        results.into_inner().expect("not poisoned"),
    );
    let gather = start.elapsed();
    let (steps, blocks) = totals.into_inner().expect("not poisoned");
    Run {
        list,
        blocks,
        resolve_and_list,
        steps,
        gather,
        wall: wall.elapsed(),
    }
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn describe(label: &str, list: &ChangeList, blocks: (usize, usize)) {
    let totals = &list.totals;
    let kinds: Vec<String> = ChangeKind::ALL
        .iter()
        .map(|&kind| format!("{kind:?} {}", totals.kinds.get(kind)))
        .collect();
    let levels: Vec<String> = Level::HIGHEST_FIRST
        .iter()
        .map(|&level| format!("{level:?} {}", totals.levels.get(level)))
        .collect();
    let edits: Vec<_> = list
        .documents
        .iter()
        .flat_map(|document| &document.changes)
        .filter_map(|change| change.edit.as_ref())
        .collect();
    let views = [
        View::Inline,
        View::Sentences,
        View::SideBySide,
        View::Rewritten,
        View::Lines,
    ]
    .map(|view| {
        format!(
            "{view:?} {}",
            edits.iter().filter(|e| e.view == view).count()
        )
    });
    let fallbacks = edits.iter().filter(|edit| edit.fallback).count();
    let highest: Vec<String> = Level::HIGHEST_FIRST
        .iter()
        .map(|&level| {
            let count = list
                .documents
                .iter()
                .filter(|document| document.summary.highest == Some(level))
                .count();
            format!("{level:?} {count}")
        })
        .collect();
    let without = list
        .documents
        .iter()
        .filter(|document| document.summary.highest.is_none())
        .count();
    println!(
        "{label}: documents {}, blocks old {} new {}, changes {}, unchanged pairs {}",
        list.documents.len(),
        blocks.0,
        blocks.1,
        totals.changes,
        list.documents
            .iter()
            .map(|d| d.unchanged.len())
            .sum::<usize>(),
    );
    println!("{label}: kinds: {}", kinds.join(", "));
    println!("{label}: levels: {}", levels.join(", "));
    println!(
        "{label}: documents by highest level: {}, no change {without}",
        highest.join(", ")
    );
    println!(
        "{label}: edits {}: {}; guard fell back {fallbacks}",
        edits.len(),
        views.join(", ")
    );
    shares_and_rules(label, list, &edits);
}

/// The spread of the kept shares, and the normative words behind the
/// changes ranked as rules.
fn shares_and_rules(label: &str, list: &ChangeList, edits: &[&driftmark_core::diff::Edit]) {
    // How the share of words kept in order spreads over the prose edits, to
    // judge the view thresholds.
    let edges = [0.0, 0.35, 0.5, 0.7, 0.85, 1.0];
    let shares: Vec<f64> = edits
        .iter()
        .filter(|edit| edit.view != View::Lines)
        .map(|edit| edit.kept_in_order)
        .collect();
    let mut buckets: Vec<String> = edges
        .windows(2)
        .map(|pair| {
            let count = shares
                .iter()
                .filter(|&&s| s >= pair[0] && s < pair[1])
                .count();
            format!("[{:.2}, {:.2}) {count}", pair[0], pair[1])
        })
        .collect();
    buckets.push(format!(
        "1.00 {}",
        shares.iter().filter(|&&s| s >= 1.0).count()
    ));
    println!("{label}: kept in order: {}", buckets.join(", "));
    // Which normative words appear in the changes ranked as rules.
    let rule_texts: Vec<String> = list
        .documents
        .iter()
        .flat_map(|document| &document.changes)
        .filter(|change| change.level == Level::Rule)
        .filter_map(|change| change.new.block.as_ref().or(change.old.block.as_ref()))
        .map(|block| block.text.to_lowercase())
        .collect();
    let words: Vec<String> = NORMATIVE_WORDS
        .iter()
        .map(|&entry| {
            let count = rule_texts
                .iter()
                .filter(|text| {
                    let words: Vec<&str> = text
                        .split(|c: char| !c.is_alphanumeric())
                        .filter(|w| !w.is_empty())
                        .collect();
                    let parts: Vec<&str> = entry.split(' ').collect();
                    words
                        .windows(parts.len())
                        .any(|window| window == parts.as_slice())
                })
                .count();
            format!("{entry} {count}")
        })
        .collect();
    println!(
        "{label}: changes ranked rule, by the normative words they hold: {}",
        words.join(", ")
    );
}

#[test]
#[ignore = "needs DRIFTMARK_CORPUS, a repo outside this one"]
fn the_change_list_meets_the_time_limit_on_a_real_repo() {
    let repo = PathBuf::from(std::env::var("DRIFTMARK_CORPUS").expect("DRIFTMARK_CORPUS"));
    let roots = std::env::var("DRIFTMARK_CORPUS_ROOTS").unwrap_or_default();
    let roots = Roots::new(roots.split(',').filter(|r| !r.is_empty())).expect("roots");
    let ranges = std::env::var("DRIFTMARK_CORPUS_RANGES").expect("DRIFTMARK_CORPUS_RANGES");
    let runs: usize =
        std::env::var("DRIFTMARK_CORPUS_RUNS").map_or(3, |r| r.parse().expect("a number"));
    let threads: Vec<usize> = std::env::var("DRIFTMARK_CORPUS_THREADS")
        .unwrap_or_else(|_| "1".to_owned())
        .split(',')
        .map(|n| n.parse().expect("a number"))
        .collect();
    let reader = GixReader::open(&repo).expect("gix opens");
    let rules = Rules::default();
    for item in ranges.split(';').filter(|r| !r.is_empty()) {
        let (label, text) = item.split_once('=').expect("label=range");
        let spec = match text.strip_prefix("since:") {
            Some(rest) => {
                let (date, branch) = rest.split_once(':').expect("since:date:branch");
                RangeSpec::since(date, Some(branch)).expect("a date")
            }
            None => RangeSpec::parse(text).expect("a range"),
        };
        // A first run warms the caches and gives the counts.
        let first = run(&reader, &rules, &spec, &roots, threads[0]);
        describe(label, &first.list, first.blocks);
        let json = Instant::now();
        let bytes = serde_json::to_vec(&first.list).expect("serializes").len();
        println!(
            "{label}: JSON {:.1} MB in {:.0} ms",
            f64::from(u32::try_from(bytes / 1000).unwrap_or(u32::MAX)) / 1000.0,
            ms(json.elapsed())
        );
        for &count in &threads {
            let all: Vec<Run> = (0..runs)
                .map(|_| run(&reader, &rules, &spec, &roots, count))
                .collect();
            assert!(all.iter().all(|r| r.list == first.list), "runs differ");
            let step = |f: fn(&Run) -> Duration| median(all.iter().map(|r| ms(f(r))).collect());
            println!(
                "{label} threads {count} ms (median of {runs}): total {:.1}; resolve and list {:.1}; \
                 summed over threads: read {:.1}, parse {:.1}, classify and compare {:.1}; gather {:.2}",
                step(|r| r.wall),
                step(|r| r.resolve_and_list),
                step(|r| r.steps.read),
                step(|r| r.steps.parse),
                step(|r| r.steps.compare),
                step(|r| r.gather),
            );
        }
    }
}
