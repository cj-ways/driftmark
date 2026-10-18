//! The starter layout as a whole: its parts, its preset, and that every file
//! it writes passes the writing check under that preset.

use std::sync::Arc;

use super::{Kind, LAYOUT, Part, new_document, preset};
use crate::document::{DiagramLanguage, Document};
use crate::lint::{Anchors, DiagramCheck, DiagramProblem, LinkTargets, LintSettings, Linter};
use crate::rules::{Lifecycle, Rules};

/// A repository that holds the layout's files and the documents `new` made.
struct Repository(Vec<(String, String)>);

impl LinkTargets for Repository {
    fn exists(&self, path: &str) -> bool {
        self.0.iter().any(|(file, _)| {
            file == path
                || file
                    .strip_prefix(path)
                    .is_some_and(|rest| rest.starts_with('/'))
        })
    }

    fn anchors(&self, path: &str) -> Option<Arc<Anchors>> {
        let (_, text) = self.0.iter().find(|(file, _)| file == path)?;
        let document = Document::parse(text.as_bytes().to_vec()).expect("the test files parse");
        Some(Arc::new(Anchors::of(&document)))
    }
}

/// Accepts every diagram. The program's own tests parse the writing
/// standard's Mermaid diagram with the real parser.
struct AnyDiagram;

impl DiagramCheck for AnyDiagram {
    fn check(&self, _: DiagramLanguage, _: &str) -> Result<(), DiagramProblem> {
        Ok(())
    }
}

fn layout_files() -> Vec<(String, String)> {
    LAYOUT
        .into_iter()
        .filter_map(|part| match part {
            Part::File { path, text } => Some((path.to_owned(), text.to_owned())),
            Part::Folder(_) => None,
        })
        .collect()
}

fn starter_rules() -> Rules {
    Rules::parse(preset::RULES).expect("the starter preset is a valid rules file")
}

#[test]
fn each_folder_comes_before_its_files() {
    let mut folder = "";
    for part in LAYOUT {
        match part {
            Part::Folder(path) => folder = path,
            Part::File { path, .. } if path == preset::FILE => {}
            Part::File { path, .. } => {
                let inside = path
                    .strip_prefix(folder)
                    .and_then(|rest| rest.strip_prefix('/'));
                assert!(inside.is_some_and(|name| !name.contains('/')), "{path}");
            }
        }
    }
    let paths: Vec<&str> = LAYOUT.into_iter().map(Part::path).collect();
    assert_eq!(paths.first(), Some(&"docs/decisions"));
    assert_eq!(paths.last(), Some(&"driftmark.toml"));
    let folders = LAYOUT
        .into_iter()
        .filter(|part| matches!(part, Part::Folder(_)))
        .count();
    assert_eq!(folders, 5);
    assert_eq!(layout_files().len(), 7);
}

#[test]
fn the_preset_is_named_and_written_at_the_root() {
    assert_eq!(preset::NAME, "starter");
    assert_eq!(preset::FILE, "driftmark.toml");
    assert!(LAYOUT.contains(&Part::File {
        path: "driftmark.toml",
        text: preset::RULES,
    }));
}

#[test]
fn the_presets_lint_settings_are_the_defaults() {
    assert_eq!(starter_rules().lint(), &LintSettings::default());
}

/// Made-up files of the layout: path, type, the status written in the file
/// (or none), and the lifecycle the preset gives it.
const TYPED: [(&str, &str, Option<&str>, Option<Lifecycle>); 17] = [
    ("docs/README.md", "readme", None, Some(Lifecycle::Current)),
    (
        "docs/decisions/README.md",
        "readme",
        None,
        Some(Lifecycle::Current),
    ),
    (
        "docs/working/README.md",
        "readme",
        None,
        Some(Lifecycle::Current),
    ),
    (
        "docs/decisions/0001-a.md",
        "decision",
        Some("accepted"),
        Some(Lifecycle::Current),
    ),
    (
        "docs/decisions/0002-b.md",
        "decision",
        Some("superseded"),
        Some(Lifecycle::Superseded),
    ),
    (
        "docs/decisions/0003-c.md",
        "decision",
        Some("rejected"),
        Some(Lifecycle::Archived),
    ),
    (
        "docs/decisions/0004-d.md",
        "decision",
        Some("proposed"),
        None,
    ),
    (
        "docs/standards/writing.md",
        "standard",
        Some("active"),
        Some(Lifecycle::Current),
    ),
    (
        "docs/standards/api/errors.md",
        "standard",
        Some("retired"),
        Some(Lifecycle::Archived),
    ),
    ("docs/standards/b.md", "standard", Some("draft"), None),
    (
        "docs/features/export.md",
        "feature",
        Some("shipped"),
        Some(Lifecycle::Current),
    ),
    (
        "docs/features/search/ranking.md",
        "feature",
        Some("dropped"),
        Some(Lifecycle::Archived),
    ),
    (
        "docs/working/faster-search/plan.md",
        "plan",
        Some("active"),
        Some(Lifecycle::Current),
    ),
    (
        "docs/working/faster-search/status.md",
        "status",
        Some("done"),
        Some(Lifecycle::Archived),
    ),
    (
        "docs/working/faster-search/notes/timings.md",
        "working",
        Some("paused"),
        Some(Lifecycle::Current),
    ),
    (
        "docs/archive/faster-search/plan.md",
        "archive",
        None,
        Some(Lifecycle::Archived),
    ),
    ("README.md", "document", None, None),
];

#[test]
fn the_preset_types_every_file_of_the_layout() {
    let rules = starter_rules();
    for (path, doc_type, status, lifecycle) in TYPED {
        // A type that reads no status gets a file whose status it must
        // ignore.
        let text = match status {
            Some(status) => format!("---\nstatus: {status}\ndate: 2026-01-05\n---\n\n# A\n\nB.\n"),
            None => "---\nstatus: done\n---\n\n# A\n\nB.\n".to_owned(),
        };
        let document = Document::parse(text.into_bytes()).expect("the test document parses");
        let found = rules.classify(path, &document);
        assert_eq!(found.doc_type, doc_type, "{path}");
        assert_eq!(found.status.as_deref(), status, "{path}");
        assert_eq!(found.lifecycle, lifecycle, "{path}");
    }
}

#[test]
fn only_decision_sections_are_decisions() {
    let rules = starter_rules();
    let decision = rules
        .type_for("docs/decisions/0001-a.md")
        .expect("decisions are typed");
    assert!(decision.is_decision_heading("Decision"));
    assert!(!decision.is_decision_heading("Options"));
    for path in [
        "docs/standards/a.md",
        "docs/features/a.md",
        "docs/working/a/plan.md",
    ] {
        let rule = rules.type_for(path).expect("the path is typed");
        assert!(!rule.is_decision_heading("Decision"), "{path}");
    }
}

#[test]
fn every_file_of_the_layout_and_every_template_passes_the_lint() {
    let rules = starter_rules();
    let mut files = layout_files();
    for kind in Kind::ALL {
        let made = new_document(kind, "Faster search", "2026-10-18", 1).expect("a good title");
        files.push((made.path, made.text));
    }
    let repository = Repository(files);
    let linter = Linter {
        rules: &rules,
        targets: &repository,
        diagrams: &AnyDiagram,
    };
    for (path, text) in &repository.0 {
        if path != preset::FILE {
            let findings = linter.lint(path, text.as_bytes().to_vec());
            assert!(findings.is_empty(), "{path}: {findings:?}");
        }
    }
}

#[test]
fn the_lint_check_here_finds_broken_links() {
    // The check above passes only because the layout's links lead to its
    // own files. A link to a missing file shows that it is really checked.
    let rules = starter_rules();
    let repository = Repository(vec![("docs/a.md".to_owned(), "# A\n\nB.\n".to_owned())]);
    let linter = Linter {
        rules: &rules,
        targets: &repository,
        diagrams: &AnyDiagram,
    };
    let text = "# A\n\n[Gone](gone.md), [here](a.md), [heading](a.md#a), [no heading](a.md#b).\n";
    let found: Vec<&str> = linter
        .lint("docs/a.md", text.as_bytes().to_vec())
        .iter()
        .map(|finding| finding.rule.id())
        .collect();
    assert_eq!(found, ["link", "anchor"]);
    assert!(repository.anchors("docs/gone.md").is_none());
    assert!(
        AnyDiagram
            .check(DiagramLanguage::Mermaid, "flowchart LR")
            .is_ok()
    );
}

#[test]
fn templates_hold_one_guidance_comment_per_section() {
    for kind in Kind::ALL {
        let made = new_document(kind, "Faster search", "2026-10-18", 1).expect("a good title");
        let sections = made.text.matches("\n## ").count();
        let comments = made.text.matches("<!-- ").count();
        assert!(sections >= 3, "{}", kind.name());
        assert_eq!(comments, sections + 1, "{}", kind.name());
        for line in made.text.lines().filter(|line| line.contains("<!--")) {
            assert!(line.trim_end().ends_with("-->"), "{line}");
        }
    }
}
