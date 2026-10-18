//! The rules file example in `/fixtures/rules-file`, applied to its made-up
//! docs tree. The same example appears in
//! `docs/features/engine/rules-file.md`, so the documented format is tested.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use driftmark_core::document::Document;
use driftmark_core::rules::Rules;

const DOCUMENTS: &[&str] = &[
    "doc/adr/0001-record-architecture-decisions.md",
    "docs/archive/2026-01-old-plan.md",
    "docs/decisions/0001-use-markdown.md",
    "docs/decisions/0002-one-big-file.md",
    "docs/decisions/0003-many-small-files.md",
    "docs/features/export.md",
    "docs/features/search/search.md",
    "docs/standards/writing.md",
    "notes/ideas.md",
];

fn folder() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/rules-file")
}

#[test]
fn the_example_rules_classify_the_example_tree() {
    let text = fs::read_to_string(folder().join("driftmark.toml")).expect("the rules file exists");
    let rules = Rules::parse(&text).expect("the example rules file is valid");
    let mut actual = String::new();
    for path in DOCUMENTS {
        let bytes = fs::read(folder().join(path)).expect("the document exists");
        let document =
            Document::parse_with(bytes, rules.masthead_keys()).expect("the document parses");
        let classification = rules.classify(path, &document);
        writeln!(actual, "{path}\n    {classification:?}").expect("writing to a string");
    }
    let written = Path::new(env!("CARGO_TARGET_TMPDIR")).join("rules-file.expected.txt");
    fs::write(&written, &actual).expect("the dump is written");
    let expected = fs::read_to_string(folder().join("expected.txt")).unwrap_or_default();
    assert_eq!(
        actual,
        expected,
        "the classification changed; see {}",
        written.display()
    );
}
