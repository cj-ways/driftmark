//! The made-up layouts in `/fixtures/presets/<name>/`, one per built-in
//! preset. Each tree has no rules file, so driftmark finds its presets by
//! their markers. Every Markdown file is classified, and the result is
//! compared with the tree's `expected.txt`. The same tree is then read with a
//! rules file that names the preset, which must give the same types.
//!
//! Each run writes its result to the build folder's
//! `tmp/presets/<name>.expected.txt`, so a deliberate change can be reviewed
//! and copied over.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use driftmark_core::document::{BlockKind, Document};
use driftmark_core::git::RepoPath;
use driftmark_core::presets::Setup;
use driftmark_core::rules::{Classification, RepoFiles, Rules};

fn is_markdown(path: &str) -> bool {
    RepoPath::new(path).is_some_and(|path| path.is_markdown())
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/presets")
}

/// A fixture tree on disk.
struct Tree(PathBuf);

impl Tree {
    /// Every file's path from the tree's root, with `/` between folders.
    fn files(&self) -> Vec<String> {
        let mut found = Vec::new();
        let mut pending = vec![(self.0.clone(), String::new())];
        while let Some((folder, prefix)) = pending.pop() {
            for entry in fs::read_dir(&folder).expect("the folder lists") {
                let entry = entry.expect("an entry");
                let name = entry.file_name().into_string().expect("a UTF-8 name");
                let path = if prefix.is_empty() {
                    name
                } else {
                    format!("{prefix}/{name}")
                };
                if entry.file_type().expect("a type").is_dir() {
                    pending.push((entry.path(), path));
                } else {
                    found.push(path);
                }
            }
        }
        found.sort();
        found
    }
}

impl RepoFiles for Tree {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        fs::read(self.0.join(path)).ok()
    }

    fn markdown_in(&self, folders: &[&str]) -> Vec<String> {
        self.files()
            .into_iter()
            .filter(|path| is_markdown(path))
            .filter(|path| {
                folders
                    .iter()
                    .any(|folder| path.starts_with(&format!("{folder}/")))
            })
            .collect()
    }
}

/// One file's classification, written out on one line.
fn line(document: &Document, class: &Classification) -> String {
    let headings: Vec<&str> = class
        .decision_blocks
        .iter()
        .map(|&index| &document.blocks[index])
        .filter(|block| matches!(block.kind, BlockKind::Heading { .. }))
        .map(|block| block.text.as_str())
        .collect();
    let or_none = |value: &Option<String>| value.clone().unwrap_or_else(|| "-".to_owned());
    format!(
        "type {} | status {} | lifecycle {} | date {} | decisions {}",
        class.doc_type,
        or_none(&class.status),
        class.lifecycle.map_or_else(
            || "-".to_owned(),
            |lifecycle| format!("{lifecycle:?}").to_lowercase()
        ),
        or_none(&class.date),
        if headings.is_empty() {
            "-".to_owned()
        } else {
            format!(
                "{} blocks under {}",
                class.decision_blocks.len(),
                headings.join(", ")
            )
        }
    )
}

/// Every Markdown file of the tree classified with `rules`.
fn classified(tree: &Tree, rules: &Rules) -> String {
    let mut out = String::new();
    for path in tree.files().iter().filter(|path| is_markdown(path)) {
        let bytes = tree.read(path).expect("the file reads");
        let document = Document::parse_with(bytes, rules.masthead_keys()).expect("it parses");
        let class = rules.classify_with(path, &document, tree);
        writeln!(out, "{path}\n    {}", line(&document, &class)).expect("writing to a string");
    }
    out
}

fn check(name: &str) {
    let tree = Tree(fixtures().join(name));
    let found = Setup::build(None, &tree).expect("the presets build");
    let mut actual = String::new();
    for in_use in &found.presets {
        writeln!(
            actual,
            "preset {} found by {}{}",
            in_use.preset.name,
            in_use.found.as_deref().unwrap_or("its name"),
            if in_use.folders.is_empty() {
                String::new()
            } else {
                format!(", also reading {}", in_use.folders.join(", "))
            }
        )
        .expect("writing to a string");
    }
    actual.push('\n');
    let types = classified(&tree, &found.rules);
    actual.push_str(&types);
    let written =
        Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("presets/{name}.expected.txt"));
    fs::create_dir_all(written.parent().expect("a folder")).expect("the folder is made");
    fs::write(&written, &actual).expect("the dump is written");
    let expected = fs::read_to_string(tree.0.join("expected.txt")).unwrap_or_default();
    assert_eq!(
        actual,
        expected,
        "the classification changed; see {}",
        written.display()
    );

    let named = Rules::parse(&format!("presets = [\"{name}\"]\n")).expect("valid");
    let named = Setup::build(Some(named), &tree).expect("the presets build");
    assert_eq!(
        classified(&tree, &named.rules),
        types,
        "named, {name} gives the same types"
    );
}

#[test]
fn openspec() {
    check("openspec");
}

#[test]
fn spec_kit() {
    check("spec-kit");
}

#[test]
fn adr() {
    check("adr");
}

#[test]
fn kiro() {
    check("kiro");
}
