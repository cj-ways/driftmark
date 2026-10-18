//! Search on a real repo, for the checks the M2 search brief asks for. It is
//! ignored in normal runs because it needs a repo that is not in this one.
//!
//! It reads the repo through the git reader only, so it never changes it,
//! and it prints counts, sizes and timings only: never a path, a title, a
//! query or any text from a file. Queries are made from words found in the
//! repo's own documents, with a fixed seed, so every run asks the same.
//!
//! ```text
//! DRIFTMARK_CORPUS=/path/to/repo DRIFTMARK_CORPUS_REF=origin/main \
//! cargo test --release -p driftmark --test search_corpus -- --ignored --nocapture
//! ```

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use driftmark::git::Git;
use driftmark::search::{File, Layer, Read, build, matches};
use driftmark_core::document::Document;
use driftmark_core::git::{GitRead, RepoPath, Roots, Snapshot};
use driftmark_core::rules::Rules;
use driftmark_core::search::{Entry, Filter, Index, Query, Shard, search, terms};

/// A small fixed-seed random number generator, so the queries are the same
/// on every run.
struct Random(u64);

impl Random {
    fn next(&mut self, below: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        usize::try_from((self.0 >> 33) % u64::try_from(below).expect("small")).expect("small")
    }
}

/// The process's resident memory in MB, from `/proc` on Linux.
fn resident_mb() -> f64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))
        .and_then(|rest| {
            rest.trim()
                .trim_end_matches(" kB")
                .trim()
                .parse::<f64>()
                .ok()
        })
        .map_or(0.0, |kb| kb / 1024.0)
}

/// Bytes as megabytes.
#[expect(clippy::cast_precision_loss, reason = "sizes are far below 2^52")]
fn mb(bytes: usize) -> f64 {
    bytes as f64 / 1e6
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

/// 100 queries from words of the documents: one word, two words, a phrase
/// of three, the first three letters of a word, and a word with the next
/// one half typed, 20 of each.
fn make_queries(documents: &[Arc<driftmark::search::Parsed>]) -> Vec<(&'static str, String)> {
    let mut random = Random(0x5EED);
    let mut words: Vec<Vec<String>> = Vec::new();
    for parsed in documents {
        if let Ok(document) = parsed.as_ref() {
            for block in &document.blocks {
                let found = terms(&block.text);
                if found.len() >= 3 {
                    words.push(found);
                }
            }
        }
    }
    let pick = |random: &mut Random| {
        let block = &words[random.next(words.len())];
        let at = random.next(block.len() - 2);
        (
            block[at].clone(),
            block[at + 1].clone(),
            block[at + 2].clone(),
        )
    };
    let mut queries: Vec<(&'static str, String)> = Vec::new();
    for n in 0..100 {
        let (a, b, c) = pick(&mut random);
        let (d, _, _) = pick(&mut random);
        let query = match n % 5 {
            0 => ("one word", format!("{a} ")),
            1 => ("two words", format!("{a} {d} ")),
            2 => ("phrase", format!("\"{a} {b} {c}\"")),
            3 => {
                let cut: String = a.chars().take(3).collect();
                ("prefix of 3", cut)
            }
            _ => (
                "typing",
                format!("{a} {}", b.chars().take(2).collect::<String>()),
            ),
        };
        queries.push(query);
    }

    queries
}

/// Runs each query three ways and prints the times: the index alone, then
/// with snippets from documents parsed before, then with snippets from
/// documents read from git and parsed again, as on a server whose cache is
/// cold. Returns the time of each query as a table.
fn time_queries(
    git: &Git,
    rules: &Rules,
    layer: &Layer,
    documents: &[Arc<driftmark::search::Parsed>],
    queries: &[(&'static str, String)],
) -> String {
    let threads = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    let by_doc = |hit: &driftmark_core::search::Hit| Some(Arc::clone(&documents[hit.doc]));
    let cold = |hit: &driftmark_core::search::Hit| {
        let blob = layer.blobs[hit.doc].as_ref()?;
        Some(Arc::new(Document::parse_with(
            git.read_blob(blob).ok()?,
            rules.masthead_keys(),
        )))
    };
    let mut core_ms = Vec::new();
    let mut warm_ms = Vec::new();
    let mut cold_ms = Vec::new();
    let mut by_kind: HashMap<&str, Vec<f64>> = HashMap::new();
    let mut found_any = 0;
    let mut raw = String::from("number\tkind\tcore_ms\twarm_ms\tcold_ms\ttotal\n");
    for (number, (kind, text)) in queries.iter().enumerate() {
        let query = Query::parse(text).expect("a query");
        let started = Instant::now();
        let found = search(&[&layer.index], &query, &Filter::default(), 20);
        let core = ms(started.elapsed());
        let started = Instant::now();
        let found_warm = search(&[&layer.index], &query, &Filter::default(), 20);
        let shown = matches(&query, &found_warm, threads, by_doc);
        let warm = ms(started.elapsed());
        let started = Instant::now();
        let found_cold = search(&[&layer.index], &query, &Filter::default(), 20);
        let shown_cold = matches(&query, &found_cold, threads, cold);
        let cold = ms(started.elapsed());
        assert_eq!(shown, shown_cold);
        if found.total > 0 {
            found_any += 1;
        }
        writeln!(
            raw,
            "{number}\t{kind}\t{core:.3}\t{warm:.3}\t{cold:.3}\t{}",
            found.total
        )
        .expect("writing");
        core_ms.push(core);
        warm_ms.push(warm);
        cold_ms.push(cold);
        by_kind.entry(kind).or_default().push(cold);
    }
    let slowest = |values: &[f64]| values.iter().copied().fold(0.0, f64::max);
    println!("queries: 100, {found_any} found at least one document");
    for (name, values) in [
        ("index only", &mut core_ms),
        ("with snippets, documents parsed", &mut warm_ms),
        ("with snippets, documents read and parsed", &mut cold_ms),
    ] {
        let top = slowest(values);
        println!(
            "  {name}: median {:.2} ms, slowest {top:.2} ms",
            median(values)
        );
    }
    let mut kinds: Vec<_> = by_kind.into_iter().collect();
    kinds.sort_by_key(|(kind, _)| *kind);
    for (kind, mut values) in kinds {
        let top = slowest(&values);
        println!(
            "  {kind} ({}): median {:.2} ms, slowest {top:.2} ms (read and parsed)",
            values.len(),
            median(&mut values)
        );
    }
    raw
}

/// One thread, step by step: reading every blob, parsing, adding to one
/// shard, and joining it into an index. Prints the time of each step.
fn phases(git: &Git, files: &[File], rules: &Rules) {
    let started = Instant::now();
    let blobs: Vec<Vec<u8>> = files
        .iter()
        .map(|file| match &file.read {
            Read::Blob(blob) => git.read_blob(blob).expect("a blob"),
            Read::Disk | Read::Gone => Vec::new(),
        })
        .collect();
    let read = ms(started.elapsed());
    let started = Instant::now();
    let documents: Vec<Document> = blobs
        .into_iter()
        .filter_map(|bytes| Document::parse_with(bytes, rules.masthead_keys()).ok())
        .collect();
    let parse = ms(started.elapsed());
    let started = Instant::now();
    let mut shard = Shard::new();
    for (file, document) in files.iter().zip(&documents) {
        shard.add(Entry {
            path: &file.path,
            doc_type: "document",
            document,
        });
    }
    let add = ms(started.elapsed());
    let started = Instant::now();
    let index = Index::from_shards(vec![shard]);
    let join = ms(started.elapsed());
    println!(
        "steps on one thread: read {read:.0} ms, parse {parse:.0} ms, index the words {add:.0} ms, join {join:.0} ms ({} documents)",
        index.documents()
    );
}

#[test]
#[ignore = "needs DRIFTMARK_CORPUS, a repo outside this one"]
fn search_on_a_real_corpus() {
    let repo = PathBuf::from(std::env::var("DRIFTMARK_CORPUS").expect("DRIFTMARK_CORPUS"));
    let reference = std::env::var("DRIFTMARK_CORPUS_REF").expect("DRIFTMARK_CORPUS_REF");
    let out = std::env::var("DRIFTMARK_SEARCH_OUT")
        .ok()
        .map(PathBuf::from);
    let git = Git::open(&repo).expect("the repo opens");
    let commit = git.resolve(&reference).expect("the reference resolves");
    let threads = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    let listed = Instant::now();
    let entries = git
        .markdown_files(&Snapshot::Commit(commit.clone()), &Roots::all())
        .expect("the files are listed");
    let list_ms = ms(listed.elapsed());
    let files: Vec<File> = entries
        .iter()
        .map(|entry| File {
            path: entry.path.clone(),
            read: Read::Blob(entry.id.clone()),
        })
        .collect();
    let rules = match git
        .blob_at(&commit, &RepoPath::new("driftmark.toml").expect("a path"))
        .expect("the tree is read")
    {
        Some(blob) => Rules::parse(&String::from_utf8_lossy(
            &git.read_blob(&blob).expect("read"),
        ))
        .unwrap_or_default(),
        None => Rules::default(),
    };
    let bytes: usize = entries
        .iter()
        .map(|entry| git.read_blob(&entry.id).expect("a blob").len())
        .sum();
    println!(
        "corpus: {} Markdown files, {:.1} MB; listing took {list_ms:.1} ms",
        files.len(),
        mb(bytes)
    );

    // Building, on all cores and on one, three times each. The process's
    // memory is read around the first build, before anything else is held.
    let before = resident_mb();
    let mut layer = None;
    let mut first = None;
    for count in [threads, 1] {
        let mut times = Vec::new();
        for _ in 0..3 {
            let started = Instant::now();
            let built = build(&git, &files, &rules, count).expect("the index is built");
            times.push(ms(started.elapsed()));
            first.get_or_insert_with(resident_mb);
            if count == threads {
                layer = Some(built);
            }
        }
        let slowest = times.iter().copied().fold(0.0, f64::max);
        println!(
            "build on {count} thread(s): median {:.0} ms, slowest {slowest:.0} ms",
            median(&mut times)
        );
    }
    let layer = layer.expect("an index");
    println!(
        "index: {} documents searched, {} not indexed, estimated {:.1} MB; process memory {:.0} MB before the first build, {:.0} MB after it, {:.0} MB after six",
        layer.index.documents(),
        layer.unindexed.len(),
        mb(layer.bytes()),
        before,
        first.unwrap_or_default(),
        resident_mb()
    );
    phases(&git, &files, &rules);

    // The documents, parsed once, to pick query words and for warm snippets.
    let documents: Vec<Arc<driftmark::search::Parsed>> = entries
        .iter()
        .map(|entry| {
            Arc::new(Document::parse_with(
                git.read_blob(&entry.id).expect("a blob"),
                rules.masthead_keys(),
            ))
        })
        .collect();
    let queries = make_queries(&documents);

    let raw = time_queries(&git, &rules, &layer, &documents, &queries);
    if let Some(out) = out {
        std::fs::create_dir_all(&out).expect("the output folder");
        std::fs::write(out.join("query-timings.tsv"), raw).expect("the timings are written");
    }
}
