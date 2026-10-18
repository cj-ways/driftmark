//! The JSON shape of the change list, which is a public contract
//! (`docs/features/engine/change-list.md`): every type, every variant, and
//! the fields left out when empty.

use driftmark_core::diff::{
    ChangeKind, ChangeList, DocumentChanges, Edit, FileState, Level, Mark, MarkOp, RangeInfo,
    SCHEMA_VERSION, Side, Signal, StatusChange, Summary, Version, View, compare,
};
use driftmark_core::document::{
    AlertKind, BlockKind, DiagramLanguage, Document, ListItem, SpanKind, TaskState,
};
use driftmark_core::git::{ObjectId, RangeNote, ResolvedRange, Snapshot};
use driftmark_core::rules::Rules;
use serde::Serialize;
use serde_json::{Value, json};

fn parse(source: &str) -> Document {
    Document::parse(source.as_bytes().to_vec()).expect("parses")
}

fn diff(old: &str, new: &str) -> DocumentChanges {
    let (old, new) = (parse(old), parse(new));
    let rules = Rules::default();
    let (old_class, new_class) = (
        rules.classify("doc.md", &old),
        rules.classify("doc.md", &new),
    );
    compare(
        "doc.md",
        Some(Version {
            document: &old,
            classification: &old_class,
        }),
        Some(Version {
            document: &new,
            classification: &new_class,
        }),
    )
    .expect("both sides")
}

fn to_json<T: Serialize>(value: &T) -> Value {
    serde_json::to_value(value).expect("serializes")
}

fn mark(op: MarkOp, text: &str) -> Mark {
    Mark {
        op,
        text: text.to_owned(),
    }
}

#[test]
fn nothing_on_either_side_is_no_document() {
    assert_eq!(compare("doc.md", None, None), None);
}

#[test]
fn block_kinds_serialize_with_a_type_tag() {
    let kinds = [
        (BlockKind::FrontMatter, json!({"type": "front_matter"})),
        (
            BlockKind::Heading { level: 2 },
            json!({"type": "heading", "level": 2}),
        ),
        (BlockKind::Paragraph, json!({"type": "paragraph"})),
        (
            BlockKind::ListItem(ListItem {
                depth: 1,
                ordered: true,
                number: Some(3),
                task: Some(TaskState::Open),
            }),
            json!({"type": "list_item", "depth": 1, "ordered": true, "number": 3, "task": "open"}),
        ),
        (
            BlockKind::ListItem(ListItem {
                depth: 0,
                ordered: false,
                number: None,
                task: Some(TaskState::Done),
            }),
            json!({"type": "list_item", "depth": 0, "ordered": false, "number": null, "task": "done"}),
        ),
        (
            BlockKind::TableRow {
                header: true,
                cells: vec!["a".to_owned()],
            },
            json!({"type": "table_row", "header": true, "cells": ["a"]}),
        ),
        (
            BlockKind::Code {
                language: Some("rust".to_owned()),
                closed: false,
            },
            json!({"type": "code", "language": "rust", "closed": false}),
        ),
        (
            BlockKind::Diagram {
                language: DiagramLanguage::Mermaid,
                closed: true,
            },
            json!({"type": "diagram", "language": "mermaid", "closed": true}),
        ),
        (
            BlockKind::Diagram {
                language: DiagramLanguage::D2,
                closed: true,
            },
            json!({"type": "diagram", "language": "d2", "closed": true}),
        ),
        (
            BlockKind::FootnoteDefinition {
                name: "n".to_owned(),
            },
            json!({"type": "footnote_definition", "name": "n"}),
        ),
        (BlockKind::Html, json!({"type": "html"})),
        (BlockKind::ThematicBreak, json!({"type": "thematic_break"})),
        (BlockKind::Unrendered, json!({"type": "unrendered"})),
    ];
    for (kind, expected) in kinds {
        assert_eq!(to_json(&kind), expected);
    }
    let alerts = [
        (AlertKind::Note, "note"),
        (AlertKind::Tip, "tip"),
        (AlertKind::Important, "important"),
        (AlertKind::Warning, "warning"),
        (AlertKind::Caution, "caution"),
    ];
    for (kind, name) in alerts {
        let alert = BlockKind::Alert {
            kind,
            title: Some("Heads up".to_owned()),
        };
        assert_eq!(
            to_json(&alert),
            json!({"type": "alert", "kind": name, "title": "Heads up"})
        );
    }
}

#[test]
fn every_enum_serializes_in_snake_case() {
    let levels: Vec<Value> = Level::HIGHEST_FIRST.iter().map(to_json).collect();
    assert_eq!(
        levels,
        [
            json!("decision"),
            json!("rule"),
            json!("fact"),
            json!("wording"),
            json!("formatting")
        ]
    );
    let kinds: Vec<Value> = ChangeKind::ALL.iter().map(to_json).collect();
    assert_eq!(
        kinds,
        [
            json!("formatting"),
            json!("edited"),
            json!("added"),
            json!("removed"),
            json!("moved"),
            json!("moved_edited")
        ]
    );
    let signals = [
        Signal::Status,
        Signal::DecisionSection,
        Signal::Normative,
        Signal::Heading,
        Signal::Number,
        Signal::Link,
        Signal::TableCell,
        Signal::CodeLine,
        Signal::Moved,
        Signal::Wording,
        Signal::Formatting,
    ];
    assert_eq!(
        to_json(&signals),
        json!([
            "status",
            "decision_section",
            "normative",
            "heading",
            "number",
            "link",
            "table_cell",
            "code_line",
            "moved",
            "wording",
            "formatting"
        ])
    );
    let views = [
        View::Inline,
        View::Sentences,
        View::SideBySide,
        View::Rewritten,
        View::Lines,
    ];
    assert_eq!(
        to_json(&views),
        json!(["inline", "sentences", "side_by_side", "rewritten", "lines"])
    );
    let ops = [MarkOp::Kept, MarkOp::Deleted, MarkOp::Inserted];
    assert_eq!(to_json(&ops), json!(["kept", "deleted", "inserted"]));
    let files = [FileState::Added, FileState::Removed, FileState::Modified];
    assert_eq!(to_json(&files), json!(["added", "removed", "modified"]));
}

#[test]
fn optional_fields_are_left_out_when_empty() {
    let edit = Edit {
        view: View::Inline,
        kept_in_order: 0.5,
        marks: Vec::new(),
        sentences: Vec::new(),
        lines: Vec::new(),
        cells: Vec::new(),
        fallback: false,
    };
    assert_eq!(
        to_json(&edit),
        json!({"view": "inline", "kept_in_order": 0.5})
    );
    let full = Edit {
        marks: vec![mark(MarkOp::Kept, "a")],
        sentences: vec![vec![mark(MarkOp::Deleted, "b")]],
        lines: vec![mark(MarkOp::Inserted, "c")],
        cells: vec![2],
        fallback: true,
        ..edit
    };
    assert_eq!(
        to_json(&full),
        json!({
            "view": "inline",
            "kept_in_order": 0.5,
            "marks": [{"op": "kept", "text": "a"}],
            "sentences": [[{"op": "deleted", "text": "b"}]],
            "lines": [{"op": "inserted", "text": "c"}],
            "cells": [2],
            "fallback": true
        })
    );
    assert_eq!(to_json(&Side { at: 3, block: None }), json!({"at": 3}));
    let list = ChangeList::new(None, Vec::new());
    assert_eq!(
        to_json(&list),
        json!({
            "schema_version": SCHEMA_VERSION,
            "totals": {
                "changes": 0,
                "highest": null,
                "levels": {"decision": 0, "rule": 0, "fact": 0, "wording": 0, "formatting": 0},
                "kinds": {"formatting": 0, "edited": 0, "added": 0, "removed": 0, "moved": 0, "moved_edited": 0}
            },
            "documents": []
        })
    );
}

#[test]
fn a_document_serializes_with_its_status_and_range() {
    let changes = DocumentChanges {
        path: "doc.md".to_owned(),
        file: FileState::Modified,
        status: Some(StatusChange {
            old: None,
            new: Some("draft".to_owned()),
        }),
        summary: Summary::default(),
        changes: Vec::new(),
        unchanged: vec![[0, 1]],
    };
    let range = RangeInfo {
        base: None,
        head: "worktree".to_owned(),
        notes: vec!["a note".to_owned()],
    };
    let value = to_json(&ChangeList::new(Some(range), vec![changes]));
    assert_eq!(
        value["range"],
        json!({"base": null, "head": "worktree", "notes": ["a note"]})
    );
    let document = &value["documents"][0];
    assert_eq!(document["status"], json!({"old": null, "new": "draft"}));
    assert_eq!(document["unchanged"], json!([[0, 1]]));
    assert_eq!(document["file"], json!("modified"));
}

#[test]
fn a_change_serializes_both_sides() {
    let changes = diff("# T\n\nOld words here.\n", "# T\n\nOld words there.\n");
    let value = to_json(&changes.changes[0]);
    assert_eq!(value["kind"], json!("edited"));
    assert_eq!(value["level"], json!("wording"));
    assert_eq!(value["signals"], json!(["wording"]));
    assert_eq!(
        value["old"],
        json!({"at": 1, "block": {
            "kind": {"type": "paragraph"},
            "start": 5,
            "end": 20,
            "line": 3,
            "heading_path": ["T"],
            "text": "Old words here."
        }})
    );
    assert_eq!(value["new"]["block"]["text"], json!("Old words there."));
    assert_eq!(value["edit"]["view"], json!("side_by_side"));
}

#[test]
fn range_info_names_both_points() {
    let id = |c: char| ObjectId::from_hex(&c.to_string().repeat(40)).expect("an id");
    let range = ResolvedRange {
        base: Some(id('a')),
        head: Snapshot::Commit(id('b')),
        notes: vec![RangeNote::AutoBase { name: "main" }],
    };
    assert_eq!(
        RangeInfo::of(&range),
        RangeInfo {
            base: Some("a".repeat(40)),
            head: "b".repeat(40),
            notes: vec!["compared with `main`".to_owned()],
        }
    );
    let worktree = ResolvedRange {
        base: None,
        head: Snapshot::Worktree,
        notes: Vec::new(),
    };
    assert_eq!(
        RangeInfo::of(&worktree),
        RangeInfo {
            base: None,
            head: "worktree".to_owned(),
            notes: Vec::new(),
        }
    );
}

/// The JSON example in `docs/features/engine/change-list.md` is what the
/// code gives for the made-up pair in `fixtures/diff/contract-example`, so
/// the contract and the code cannot drift apart.
#[test]
fn the_documented_example_is_what_the_code_gives() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let fixture = root.join("fixtures/diff/contract-example");
    let read = |name: &str| std::fs::read_to_string(fixture.join(name)).expect("the fixture");
    let rules = Rules::parse(&read("driftmark.toml")).expect("the rules parse");
    let path = "docs/decisions/0002-cache-rendered-pages.md";
    let (old, new) = (parse(&read("a.md")), parse(&read("b.md")));
    let (old_class, new_class) = (rules.classify(path, &old), rules.classify(path, &new));
    let document = compare(
        path,
        Some(Version {
            document: &old,
            classification: &old_class,
        }),
        Some(Version {
            document: &new,
            classification: &new_class,
        }),
    )
    .expect("both sides");
    let range = RangeInfo {
        base: Some("a1b2c3d4e5f60718293a4b5c6d7e8f9012345678".to_owned()),
        head: "worktree".to_owned(),
        notes: vec!["compared with `origin/main`".to_owned()],
    };
    let json = serde_json::to_string_pretty(&ChangeList::new(Some(range), vec![document]))
        .expect("serializes");
    let written =
        std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("change-list-example.json");
    std::fs::write(&written, &json).expect("a dump");
    // Git for Windows checks Markdown out with CRLF line endings; the example is
    // compared as text, so the document's line-ending style must not matter.
    let doc = std::fs::read_to_string(root.join("docs/features/engine/change-list.md"))
        .expect("the contract document")
        .replace("\r\n", "\n");
    let example = doc
        .split("```json\n")
        .nth(1)
        .and_then(|rest| rest.split("\n```").next())
        .expect("a JSON example");
    assert_eq!(example, json, "see {}", written.display());
}

/// The spans example in `docs/features/engine/document-model.md`.
#[test]
fn the_documented_spans_example_holds() {
    let block = parse("Use [the guide](a.md) or `cargo doc`.\n")
        .blocks
        .remove(0);
    assert_eq!(block.text, "Use the guide or cargo doc.");
    let spans: Vec<(std::ops::Range<usize>, &SpanKind)> = block
        .spans
        .iter()
        .map(|span| (span.range.clone(), &span.kind))
        .collect();
    let link = SpanKind::Link {
        target: "a.md".to_owned(),
    };
    assert_eq!(spans, [(4..13, &link), (17..26, &SpanKind::Code)]);
}
