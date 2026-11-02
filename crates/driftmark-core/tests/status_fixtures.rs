//! The made-up repositories in `/fixtures/status/<behaviour>/`. Each holds
//! Markdown files, a `driftmark.toml`, and `touched.toml`, which says how
//! many whole days before `now` each file last changed. Every file is
//! parsed and classified, the status view is built, and it is written out
//! and compared with the folder's `expected.txt`.
//!
//! Each run writes its result to the build folder's
//! `tmp/status/<behaviour>.expected.txt`, so a deliberate change can be
//! reviewed and copied over. The program's test
//! (`crates/driftmark/tests/status.rs`) commits the same trees to git and
//! checks the server's answer against the same files.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use driftmark_core::document::Document;
use driftmark_core::git::{ObjectId, RepoPath};
use driftmark_core::rules::Rules;
use driftmark_core::status::{Doc, Entry, Filter, Stage, Target, Touched, View};
use serde::Deserialize;

/// `touched.toml`.
#[derive(Deserialize)]
struct Times {
    now: i64,
    days: BTreeMap<String, toml::Value>,
}

fn folder(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/status")
        .join(name)
}

/// Every Markdown file under `root`, by its path from `root`.
fn markdown(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut pending = vec![(root.to_path_buf(), String::new())];
    while let Some((at, prefix)) = pending.pop() {
        for entry in fs::read_dir(&at).expect("the folder lists") {
            let entry = entry.expect("an entry");
            let name = entry.file_name().into_string().expect("a UTF-8 name");
            let path = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            if entry.file_type().expect("a type").is_dir() {
                pending.push((entry.path(), path));
            } else if RepoPath::new(path.as_str()).is_some_and(|path| path.is_markdown()) {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// What a "superseded by" became, as one word and the path it leads to.
fn successor(view: &View, doc: &Doc) -> String {
    let Some(successor) = &doc.successor else {
        return "by -".to_owned();
    };
    let text = successor
        .text
        .as_deref()
        .map_or_else(String::new, |text| format!(" \"{text}\""));
    let target = match &successor.target {
        Target::Found(at) => format!("found {}", view.documents[*at].path),
        Target::Circle(at) => format!("circle {}", view.documents[*at].path),
        Target::Missing(path) => format!("missing {path}"),
        Target::Unmatched => "unmatched".to_owned(),
        Target::Unnamed => "unnamed".to_owned(),
    };
    format!("by{text}: {target}")
}

/// One document on one line.
fn line(view: &View, doc: &Doc) -> String {
    let age = match (&doc.touched, doc.days) {
        (Touched::Committed { .. }, Some(days)) => {
            let untouched = if doc.untouched { ", untouched" } else { "" };
            format!("{days} days{untouched}")
        }
        _ => "uncommitted".to_owned(),
    };
    format!(
        "{} | {} | status {} | {} | {age}",
        doc.path,
        Stage::of(doc.lifecycle).name(),
        doc.status.as_deref().unwrap_or("-"),
        successor(view, doc)
    )
}

fn rows(out: &mut String, view: &View, index: usize, depth: usize) {
    let doc = &view.documents[index];
    writeln!(out, "{}{}", "  ".repeat(depth), line(view, doc)).expect("a string");
    for &older in &doc.replaces {
        rows(out, view, older, depth + 1);
    }
}

/// The view written out: the counts, then each type's rows with the
/// documents they replaced indented under them.
fn written(view: &View) -> String {
    let mut out = format!("untouched-days {}\n\n", view.untouched_days);
    let counts = view
        .types
        .iter()
        .map(|counts| (counts.name.as_str(), counts.counts))
        .chain([("all", view.totals)]);
    for (name, counts) in counts {
        writeln!(
            out,
            "{name}: {} documents, {} current, {} superseded, {} archived, {} none, {} untouched",
            counts.documents,
            counts.current,
            counts.superseded,
            counts.archived,
            counts.none,
            counts.untouched
        )
        .expect("a string");
    }
    for group in view.groups(&Filter::default()) {
        writeln!(out, "\n## {}", group.name).expect("a string");
        for index in group.rows {
            rows(&mut out, view, index, 0);
        }
    }
    out
}

fn check(name: &str) {
    let root = folder(name);
    let rules = Rules::parse(&fs::read_to_string(root.join("driftmark.toml")).unwrap_or_default())
        .expect("the rules parse");
    let times: Times =
        toml::from_str(&fs::read_to_string(root.join("touched.toml")).expect("touched.toml reads"))
            .expect("touched.toml parses");
    let commit = ObjectId::from_hex(&"c".repeat(40)).expect("an id");
    let mut entries = Vec::new();
    for path in markdown(&root) {
        let bytes = fs::read(root.join(&path)).expect("the file reads");
        let document = Document::parse_with(bytes, rules.masthead_keys()).expect("it parses");
        let class = rules.classify(&path, &document);
        let touched = match times.days.get(&path) {
            Some(toml::Value::Integer(days)) => Touched::Committed {
                commit: commit.clone(),
                at: times.now - days * 86_400 - 3_600,
            },
            Some(toml::Value::String(word)) if word == "uncommitted" => Touched::Uncommitted,
            other => panic!("touched.toml gives {path} {other:?}"),
        };
        entries.push((Entry::new(path, document.meta.title, class), touched));
    }
    assert_eq!(
        entries.len(),
        times.days.len(),
        "touched.toml names every file"
    );
    let view = View::build(entries, &rules, times.now);
    let actual = written(&view);
    let dump = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("status/{name}.expected.txt"));
    fs::create_dir_all(dump.parent().expect("a folder")).expect("the folder is made");
    fs::write(&dump, &actual).expect("the dump is written");
    let expected = fs::read_to_string(root.join("expected.txt")).unwrap_or_default();
    assert_eq!(actual, expected, "the view changed; see {}", dump.display());
}

#[test]
fn every_lifecycle() {
    check("lifecycles");
}

#[test]
fn a_superseded_chain() {
    check("superseded-chain");
}

#[test]
fn a_broken_superseded_by() {
    check("broken-superseded-by");
}

#[test]
fn the_untouched_rule() {
    check("untouched");
}
