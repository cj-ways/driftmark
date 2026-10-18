//! The lint on the made-up fixtures in `/fixtures/lint`, with the real
//! program parts: files on disk for links, and merman for diagrams.
//!
//! Each folder is named after one rule and is a small repository of its
//! own, with an optional `driftmark.toml`. Its `pass.md` must have no
//! finding of any rule. Its `fail.md` must have findings of that rule only,
//! and they must equal `fail.expected.txt`. Every run also writes the fresh
//! output to `CARGO_TARGET_TMPDIR`, so a deliberate change can be reviewed
//! and copied over.

use std::fs;
use std::path::{Path, PathBuf};

use driftmark::lint::{Workspace, to_text};
use driftmark_core::lint::Rule;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/lint")
}

#[test]
fn every_rule_has_a_fixture_folder() {
    let mut folders: Vec<String> = fs::read_dir(fixtures())
        .expect("the fixtures folder exists")
        .map(|entry| {
            entry
                .expect("a folder entry")
                .file_name()
                .into_string()
                .expect("a UTF-8 name")
        })
        .collect();
    folders.sort();
    let mut ids: Vec<String> = Rule::ALL.iter().map(|rule| rule.id().to_owned()).collect();
    ids.sort();
    assert_eq!(folders, ids);
}

#[test]
fn each_rule_passes_its_good_file_and_fails_its_bad_one() {
    let written = Path::new(env!("CARGO_TARGET_TMPDIR")).join("lint-fixtures");
    let mut problems = Vec::new();
    for rule in Rule::ALL {
        let folder = fixtures().join(rule.id());
        let workspace = Workspace::open(&folder).expect("the fixture's rules file is valid");
        let passing = workspace.lint(&["pass.md".to_owned()]);
        if passing.iter().any(|report| !report.findings.is_empty()) {
            problems.push(format!("{}/pass.md:\n{}", rule.id(), to_text(&passing)));
        }
        let failing = workspace.lint(&["fail.md".to_owned()]);
        let findings: Vec<_> = failing.iter().flat_map(|report| &report.findings).collect();
        if findings.is_empty() || findings.iter().any(|finding| finding.rule != rule) {
            problems.push(format!(
                "{}/fail.md must fail this rule only:\n{}",
                rule.id(),
                to_text(&failing)
            ));
        }
        let actual = to_text(&failing);
        let out = written.join(rule.id());
        fs::create_dir_all(&out).expect("the output folder is made");
        fs::write(out.join("fail.expected.txt"), &actual).expect("the output is written");
        let expected = fs::read_to_string(folder.join("fail.expected.txt")).unwrap_or_default();
        if actual != expected {
            problems.push(format!(
                "{}/fail.md changed; see {}\n{actual}",
                rule.id(),
                out.display()
            ));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
