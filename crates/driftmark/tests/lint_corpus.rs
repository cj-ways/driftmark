//! The lint over a real repo's docs at one commit, for the check the M4 lint
//! brief asks for. It is ignored in normal runs because it needs a repo that
//! is not in this one.
//!
//! It reads the repo with `git ls-tree` and `git cat-file` only, so it never
//! changes it, and it prints counts and timings only, never a path, a name or
//! any text from a file. Run it with a release build:
//!
//! ```text
//! DRIFTMARK_CORPUS=/path/to/repo DRIFTMARK_CORPUS_REF=origin/main \
//! DRIFTMARK_CORPUS_ROOTS=docs \
//! cargo test --release -p driftmark --test lint_corpus -- --ignored --nocapture
//! ```

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use driftmark::lint::MermaidCheck;
use driftmark_core::document::Document;
use driftmark_core::git::RepoPath;
use driftmark_core::lint::{Anchors, Finding, LinkTargets, Linter, Rule};
use driftmark_core::rules::Rules;

/// Every file of the tree, and the Markdown files' text, in memory.
struct Tree {
    files: BTreeSet<String>,
    folders: BTreeSet<String>,
    markdown: HashMap<String, Vec<u8>>,
    anchors: Mutex<HashMap<String, Option<Arc<Anchors>>>>,
}

impl LinkTargets for Tree {
    fn exists(&self, path: &str) -> bool {
        self.files.contains(path) || self.folders.contains(path)
    }

    fn anchors(&self, path: &str) -> Option<Arc<Anchors>> {
        let mut cache = self.anchors.lock().expect("the cache lock");
        cache
            .entry(path.to_owned())
            .or_insert_with(|| {
                let bytes = self.markdown.get(path)?.clone();
                Document::parse(bytes)
                    .ok()
                    .map(|document| Arc::new(Anchors::of(&document)))
            })
            .clone()
    }
}

fn is_markdown(path: &str) -> bool {
    RepoPath::new(path).is_some_and(|path| path.is_markdown())
}

/// Reads every file path and the bytes of every Markdown file at `reference`.
fn read_tree(repo: &PathBuf, reference: &str) -> Tree {
    let listing = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["ls-tree", "-r", "-z", reference])
        .output()
        .expect("git ls-tree runs");
    assert!(listing.status.success(), "git ls-tree failed");
    let mut files = BTreeSet::new();
    let mut folders = BTreeSet::new();
    let mut wanted = Vec::new();
    for entry in listing.stdout.split(|&b| b == 0).filter(|e| !e.is_empty()) {
        let text = std::str::from_utf8(entry).expect("a UTF-8 listing");
        let (meta, path) = text.split_once('\t').expect("mode type id, tab, path");
        let mut meta = meta.split(' ');
        let (mode, kind, id) = (meta.next(), meta.next(), meta.next());
        if kind != Some("blob") {
            continue;
        }
        let mut folder = path;
        while let Some((parent, _)) = folder.rsplit_once('/') {
            folders.insert(parent.to_owned());
            folder = parent;
        }
        files.insert(path.to_owned());
        if is_markdown(path) && mode != Some("120000") {
            wanted.push((path.to_owned(), id.expect("an id").to_owned()));
        }
    }
    let mut cat = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["cat-file", "--batch"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("git cat-file runs");
    let mut input = cat.stdin.take().expect("stdin");
    let ids: Vec<String> = wanted.iter().map(|(_, id)| id.clone()).collect();
    let writer = std::thread::spawn(move || {
        for id in ids {
            writeln!(input, "{id}").expect("an id is written");
        }
    });
    let mut output = BufReader::new(cat.stdout.take().expect("stdout"));
    let mut markdown = HashMap::new();
    for (path, _) in &wanted {
        let mut header = String::new();
        output.read_line(&mut header).expect("a header");
        let size: usize = header
            .split_whitespace()
            .nth(2)
            .and_then(|size| size.parse().ok())
            .expect("a size");
        let mut bytes = vec![0; size + 1];
        output.read_exact(&mut bytes).expect("the blob");
        bytes.pop();
        markdown.insert(path.clone(), bytes);
    }
    writer.join().expect("the writer finishes");
    assert!(cat.wait().expect("git cat-file exits").success());
    Tree {
        files,
        folders,
        markdown,
        anchors: Mutex::new(HashMap::new()),
    }
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

#[test]
#[ignore = "needs DRIFTMARK_CORPUS, a repo outside this one"]
fn lint_a_real_repo() {
    let repo = PathBuf::from(std::env::var("DRIFTMARK_CORPUS").expect("DRIFTMARK_CORPUS"));
    let reference = std::env::var("DRIFTMARK_CORPUS_REF").unwrap_or_else(|_| "HEAD".to_owned());
    let roots = std::env::var("DRIFTMARK_CORPUS_ROOTS").unwrap_or_default();
    let roots: Vec<String> = roots
        .split(',')
        .filter(|root| !root.is_empty())
        .map(|root| format!("{}/", root.trim_end_matches('/')))
        .collect();
    let rules = match std::env::var("DRIFTMARK_CORPUS_RULES") {
        Ok(path) => Rules::parse(&std::fs::read_to_string(path).expect("the rules file"))
            .expect("a valid rules file"),
        Err(_) => Rules::default(),
    };

    let start = Instant::now();
    let tree = read_tree(&repo, &reference);
    let read = start.elapsed();
    let mut paths: Vec<&String> = tree
        .markdown
        .keys()
        .filter(|path| roots.is_empty() || roots.iter().any(|root| path.starts_with(root.as_str())))
        .collect();
    paths.sort();
    let bytes: usize = paths.iter().map(|path| tree.markdown[*path].len()).sum();
    println!(
        "corpus: {} Markdown files to check ({:.1} MB), {} files in the tree; read in {:.0} ms",
        paths.len(),
        f64::from(u32::try_from(bytes / 1000).expect("fits")) / 1000.0,
        tree.files.len(),
        ms(read)
    );

    let diagrams = MermaidCheck::new();
    let linter = Linter {
        rules: &rules,
        targets: &tree,
        diagrams: &diagrams,
    };
    // Warm the anchor cache and the Mermaid parser, as a long-running
    // process would be, then time one file at a time on one thread.
    let mut findings: Vec<Vec<Finding>> = paths
        .iter()
        .map(|path| linter.lint(path, tree.markdown[*path].clone()))
        .collect();
    let mut times = Vec::with_capacity(paths.len());
    let start = Instant::now();
    for (index, path) in paths.iter().enumerate() {
        let bytes = tree.markdown[*path].clone();
        let one = Instant::now();
        findings[index] = linter.lint(path, bytes);
        times.push(one.elapsed());
    }
    let one_thread = start.elapsed();

    let (threads, all_cores) = cold_run(&rules, &tree, &paths);

    times.sort();
    let at = |percent: usize| ms(times[(times.len() * percent / 100).min(times.len() - 1)]);
    println!(
        "time: {:.0} ms on one thread (warm), {:.0} ms on {threads} threads (cold); per file: median {:.2} ms, p99 {:.2} ms, slowest {:.2} ms",
        ms(one_thread),
        ms(all_cores),
        at(50),
        at(99),
        ms(*times.last().expect("one file at least"))
    );
    let over_50 = times.iter().filter(|time| ms(**time) >= 50.0).count();
    println!("files that took 50 ms or more: {over_50}");

    // As a hook process sees it: nothing cached, so the linked files are
    // parsed for their anchors and the Mermaid parser is made again.
    let mut cold_times = Vec::with_capacity(paths.len());
    for path in &paths {
        tree.anchors.lock().expect("the cache lock").clear();
        let fresh = MermaidCheck::new();
        let linter = Linter {
            rules: &rules,
            targets: &tree,
            diagrams: &fresh,
        };
        let bytes = tree.markdown[*path].clone();
        let one = Instant::now();
        let _ = linter.lint(path, bytes);
        cold_times.push(one.elapsed());
    }
    cold_times.sort();
    let cold_at = |percent: usize| {
        ms(cold_times[(cold_times.len() * percent / 100).min(cold_times.len() - 1)])
    };
    println!(
        "per file with nothing cached: median {:.2} ms, p99 {:.2} ms, slowest {:.2} ms; 50 ms or more: {}",
        cold_at(50),
        cold_at(99),
        ms(*cold_times.last().expect("one file at least")),
        cold_times.iter().filter(|time| ms(**time) >= 50.0).count()
    );

    print_counts(&findings);

    self_check(&linter, &tree, &paths);
}

/// Prints the findings per rule and severity, and how many files each
/// touches.
fn print_counts(findings: &[Vec<Finding>]) {
    let mut counts: BTreeMap<(Rule, &str), usize> = BTreeMap::new();
    let mut files_with: BTreeMap<Rule, usize> = BTreeMap::new();
    for file in findings {
        let mut seen = BTreeSet::new();
        for finding in file {
            *counts
                .entry((finding.rule, finding.severity.name()))
                .or_default() += 1;
            seen.insert(finding.rule);
        }
        for rule in seen {
            *files_with.entry(rule).or_default() += 1;
        }
    }
    let clean = findings.iter().filter(|file| file.is_empty()).count();
    let without_errors = findings
        .iter()
        .filter(|file| file.iter().all(|f| f.severity.name() != "error"))
        .count();
    println!(
        "files with no finding: {clean}; files with no error: {without_errors}; of {}",
        findings.len()
    );
    println!("rule, severity, findings, files");
    for rule in Rule::ALL {
        for severity in ["error", "warning"] {
            if let Some(count) = counts.get(&(rule, severity)) {
                println!(
                    "{}, {severity}, {count}, {}",
                    rule.id(),
                    files_with.get(&rule).copied().unwrap_or(0)
                );
            }
        }
    }
    let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
    for finding in findings.iter().flatten().filter(|f| f.rule == Rule::Link) {
        let target = finding.message.split('`').nth(1).unwrap_or_default();
        let kind = if finding.message.contains("outside the repository") {
            "climbs out of the repository"
        } else if target.contains(':') {
            "holds a colon, such as a line number"
        } else if target.ends_with('/')
            || !target.rsplit('/').next().unwrap_or_default().contains('.')
        {
            "a folder or a name without extension"
        } else if is_markdown(target.split(['#', '?']).next().unwrap_or_default()) {
            "a Markdown file"
        } else {
            "another file"
        };
        *kinds.entry(kind).or_default() += 1;
    }
    println!("link findings by kind: {kinds:?}");
    let short_label = findings
        .iter()
        .flatten()
        .filter(|f| f.rule == Rule::Summary && f.message.contains("reads as a masthead field"))
        .count();
    println!(
        "summary findings where a short labelled quote reads as a masthead field: {short_label}"
    );
    let total: usize = counts.values().sum();
    println!("total findings: {total}");
}

/// Lints every file once on every core, with a fresh anchor cache and a
/// fresh parser, as a `driftmark lint` run does. Returns the thread count
/// and the time.
fn cold_run(rules: &Rules, tree: &Tree, paths: &[&String]) -> (usize, Duration) {
    // A cold run: a fresh anchor cache and a fresh parser, every core.
    let cold = Tree {
        files: tree.files.clone(),
        folders: tree.folders.clone(),
        markdown: tree.markdown.clone(),
        anchors: Mutex::new(HashMap::new()),
    };
    let cold_diagrams = MermaidCheck::new();
    let cold_linter = Linter {
        rules,
        targets: &cold,
        diagrams: &cold_diagrams,
    };
    let start = Instant::now();
    let threads = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    let next = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(path) = paths.get(index) else { break };
                    let _ = cold_linter.lint(path, cold.markdown[*path].clone());
                }
            });
        }
    });
    let all_cores = start.elapsed();
    (threads, all_cores)
}

/// Lints every file again with its same-file anchors and relative Markdown
/// links broken on purpose, to show the link checks can fail on real text.
fn self_check(linter: &Linter<'_>, tree: &Tree, paths: &[&String]) {
    // The check must be able to fail on real text: break every same-file
    // anchor and every relative Markdown link on purpose, and count what the
    // lint then finds.
    let (mut anchors, mut links) = (0, 0);
    for path in paths {
        let text = String::from_utf8_lossy(&tree.markdown[*path])
            .replace("](#", "](#zz-")
            .replace(".md)", "-zz.md)");
        for finding in linter.lint(path, text.into_bytes()) {
            match finding.rule {
                Rule::Anchor => anchors += 1,
                Rule::Link => links += 1,
                _ => {}
            }
        }
    }
    println!(
        "self-check with anchors and links broken on purpose: {anchors} anchor findings, {links} link findings"
    );
}
