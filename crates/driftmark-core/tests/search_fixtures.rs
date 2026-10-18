//! The search fixtures in `/fixtures/search/`, one folder per behaviour.
//!
//! Each folder is a small made-up repository: its Markdown files, an
//! optional `driftmark.toml`, and `queries.toml`, the queries to run. The
//! results are written out and compared with `expected.txt`: for each query,
//! the documents found in order, with their score, type, block, section and
//! snippet, matches in brackets. Each run also writes its fresh output under
//! the build folder's `tmp/search-fixtures/`, so a deliberate change can be
//! reviewed and copied over. `words/terms.expected.txt` shows how each block
//! of `words/words.md` splits into words.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use driftmark_core::document::Document;
use driftmark_core::git::RepoPath;
use driftmark_core::rules::{DEFAULT_TYPE, Rules};
use driftmark_core::search::{Entry, Filter, Index, Query, Shard, place, search, terms};
use serde::Deserialize;

#[derive(Deserialize)]
struct Queries {
    query: Vec<Asked>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Asked {
    q: String,
    path: Option<String>,
    #[serde(default)]
    types: Vec<String>,
    limit: Option<usize>,
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/search")
}

fn out_dir() -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("search-fixtures");
    fs::create_dir_all(&dir).expect("the output folder is made");
    dir
}

/// Every Markdown file under `dir`, as paths relative to it, sorted.
fn markdown(dir: &Path, prefix: &str, out: &mut Vec<String>) {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .expect("the folder is read")
        .map(|entry| entry.expect("an entry").path())
        .collect();
    entries.sort();
    for entry in entries {
        let name = entry.file_name().expect("a name").to_string_lossy();
        let relative = format!("{prefix}{name}");
        if entry.is_dir() {
            markdown(&entry, &format!("{relative}/"), out);
        } else if entry
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
        {
            out.push(relative);
        }
    }
}

fn compare(name: &str, actual: &str, expected: &Path) {
    let written = out_dir().join(name);
    fs::write(&written, actual).expect("the output is written");
    let wanted = fs::read_to_string(expected).unwrap_or_default();
    assert_eq!(
        actual,
        wanted,
        "the output changed; see {}",
        written.display()
    );
}

fn run_folder(folder: &str) {
    let dir = root().join(folder);
    let rules = fs::read_to_string(dir.join("driftmark.toml"))
        .map(|text| Rules::parse(&text).expect("the rules file is valid"))
        .unwrap_or_default();
    let mut paths = Vec::new();
    markdown(&dir, "", &mut paths);
    let documents: Vec<(RepoPath, Document)> = paths
        .iter()
        .map(|path| {
            let bytes = fs::read(dir.join(path)).expect("the document is read");
            let document =
                Document::parse_with(bytes, rules.masthead_keys()).expect("the document parses");
            (RepoPath::new(path.as_str()).expect("a path"), document)
        })
        .collect();
    let mut shard = Shard::new();
    for (path, document) in &documents {
        let doc_type = rules
            .type_for(&path.to_lossy())
            .map_or(DEFAULT_TYPE, |rule| rule.name.as_str());
        shard.add(Entry {
            path,
            doc_type,
            document,
        });
    }
    let index = Index::from_shards(vec![shard]);
    let text = fs::read_to_string(dir.join("queries.toml")).expect("the queries are read");
    let queries: Queries = toml::from_str(&text).expect("the queries are valid");
    let mut out = String::new();
    for asked in queries.query {
        write!(out, "q = {:?}", asked.q).expect("writing");
        if let Some(path) = &asked.path {
            write!(out, "  path = {path:?}").expect("writing");
        }
        if !asked.types.is_empty() {
            write!(out, "  types = {:?}", asked.types).expect("writing");
        }
        if let Some(limit) = asked.limit {
            write!(out, "  limit = {limit}").expect("writing");
        }
        out.push('\n');
        let query = Query::parse(&asked.q).expect("the query is valid");
        let filter = Filter {
            path: asked.path,
            types: asked.types,
        };
        let found = search(&[&index], &query, &filter, asked.limit.unwrap_or(20));
        writeln!(out, "  {} of {} documents", found.total, found.documents).expect("writing");
        for (rank, hit) in found.hits.iter().enumerate() {
            let document = &documents[hit.doc].1;
            let place = place(document, hit.block);
            let snippet: String = query
                .snippet(&document.blocks[hit.block])
                .iter()
                .map(|piece| {
                    if piece.hit {
                        format!("[{}]", piece.text)
                    } else {
                        piece.text.clone()
                    }
                })
                .collect();
            writeln!(
                out,
                "  {}. {}  {:.3}  {}  {:?}\n     block {}  {}  {}\n     {}",
                rank + 1,
                hit.path.to_lossy(),
                hit.score,
                hit.doc_type,
                hit.title.as_deref().unwrap_or("-"),
                hit.block,
                if place.headings.is_empty() {
                    "-".to_owned()
                } else {
                    place.headings.join(" › ")
                },
                place
                    .anchor
                    .map_or_else(|| "-".to_owned(), |anchor| format!("#{anchor}")),
                snippet,
            )
            .expect("writing");
        }
    }
    compare(
        &format!("{folder}.expected.txt"),
        &out,
        &dir.join("expected.txt"),
    );
}

#[test]
fn words_split_at_unicode_word_boundaries_and_fold_case() {
    let dir = root().join("words");
    let bytes = fs::read(dir.join("words.md")).expect("the document is read");
    let document = Document::parse(bytes).expect("the document parses");
    let mut out = String::new();
    for (index, block) in document.blocks.iter().enumerate() {
        writeln!(out, "{index}: {}", terms(&block.text).join(" ")).expect("writing");
    }
    compare(
        "words.terms.expected.txt",
        &out,
        &dir.join("terms.expected.txt"),
    );
    run_folder("words");
}

#[test]
fn phrases_need_their_words_together() {
    run_folder("phrases");
}

#[test]
fn the_last_word_is_a_prefix_while_typing() {
    run_folder("prefixes");
}

#[test]
fn filters_keep_folders_and_types() {
    run_folder("filters");
}

#[test]
fn titles_beat_headings_beat_the_body_and_rare_words_weigh_more() {
    run_folder("ranking");
}

#[test]
fn snippets_cut_long_blocks_and_mark_matches() {
    run_folder("snippets");
}
