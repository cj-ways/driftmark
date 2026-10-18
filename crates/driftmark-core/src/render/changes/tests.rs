//! Tests of the change list HTML: the shapes the fixture pairs do not
//! reach, and hand-built change lists for cases a comparison never gives.

use std::sync::Arc;

use super::*;
use crate::diff::{Edit, KindCounts, LevelCounts, Summary, Version, compare};
use crate::document::Document;
use crate::rules::Rules;

fn changes_of(old: Option<&str>, new: Option<&str>) -> DocumentChanges {
    let rules = Rules::default();
    let parse = |text: &str| Document::parse(text.as_bytes().to_vec()).expect("parses");
    let old = old.map(parse);
    let new = new.map(parse);
    let old_class = old
        .as_ref()
        .map(|document| rules.classify("doc.md", document));
    let new_class = new
        .as_ref()
        .map(|document| rules.classify("doc.md", document));
    let old_version = old
        .as_ref()
        .zip(old_class.as_ref())
        .map(|(document, classification)| Version {
            document,
            classification,
        });
    let new_version = new
        .as_ref()
        .zip(new_class.as_ref())
        .map(|(document, classification)| Version {
            document,
            classification,
        });
    compare("doc.md", old_version, new_version).expect("a change list")
}

fn block(kind: BlockKind, text: &str) -> BlockRef {
    BlockRef {
        kind,
        start: 0,
        end: text.len(),
        line: 1,
        heading_path: Arc::from([]),
        text: text.to_owned(),
    }
}

fn mark(op: MarkOp, text: &str) -> Mark {
    Mark {
        op,
        text: text.to_owned(),
    }
}

fn one(change: Change) -> DocumentChanges {
    DocumentChanges {
        path: "doc.md".to_owned(),
        file: FileState::Modified,
        status: None,
        summary: Summary::default(),
        changes: vec![change],
        unchanged: Vec::new(),
    }
}

fn edited(edit: Edit, old: BlockRef, new: BlockRef) -> Change {
    Change {
        kind: ChangeKind::Edited,
        level: Level::Wording,
        signals: vec![Signal::Wording],
        subject: Subject::Block {
            old: Side {
                at: 0,
                block: Some(old),
            },
            new: Side {
                at: 0,
                block: Some(new),
            },
        },
        edit: Some(edit),
    }
}

fn edit(view: View) -> Edit {
    Edit {
        view,
        kept_in_order: 0.5,
        marks: Vec::new(),
        sentences: Vec::new(),
        lines: Vec::new(),
        cells: Vec::new(),
        fallback: false,
    }
}

/// Every pair of the change fixtures, with its rules file.
fn fixture_changes() -> Vec<DocumentChanges> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
    let mut folders = Vec::new();
    for parent in ["diff", "render/changes"] {
        for entry in std::fs::read_dir(root.join(parent)).expect("the folder exists") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                folders.push(path);
            }
        }
    }
    folders.sort();
    folders
        .into_iter()
        .map(|folder| {
            let read = |name: &str| std::fs::read_to_string(folder.join(name)).ok();
            let rules = read("driftmark.toml").map_or_else(Rules::default, |text| {
                Rules::parse(&text).expect("the rules parse")
            });
            let parse = |name: &str| {
                read(name).map(|text| Document::parse(text.into_bytes()).expect("parses"))
            };
            let (old, new) = (parse("a.md"), parse("b.md"));
            let old_class = old
                .as_ref()
                .map(|document| rules.classify("doc.md", document));
            let new_class = new
                .as_ref()
                .map(|document| rules.classify("doc.md", document));
            let old_version =
                old.as_ref()
                    .zip(old_class.as_ref())
                    .map(|(document, classification)| Version {
                        document,
                        classification,
                    });
            let new_version =
                new.as_ref()
                    .zip(new_class.as_ref())
                    .map(|(document, classification)| Version {
                        document,
                        classification,
                    });
            compare("doc.md", old_version, new_version).expect("a change list")
        })
        .collect()
}

#[test]
fn every_fixture_renders_in_the_unit_build_too() {
    // The golden tests run against the library built for integration tests;
    // rendering the same pairs here keeps every writer covered within this
    // build as well, which the coverage gate measures on its own.
    let all = fixture_changes();
    assert!(all.len() >= 32);
    for document in &all {
        let html = changes(document, "u");
        assert!(html.starts_with("<ol class=\"changes\""));
    }
}

#[test]
fn a_new_or_deleted_document_is_one_item_with_its_title_summary_and_size() {
    let empty = changes(&changes_of(None, Some("")), "d");
    assert_eq!(
        empty,
        "<ol class=\"changes\" data-path=\"doc.md\" data-file=\"added\">\n<li id=\"d.0\" class=\"change document\" data-kind=\"added\" data-level=\"wording\" data-signals=\"wording\" data-bytes=\"0\" data-lines=\"0\">\n<p class=\"size\" data-gen>0 lines</p>\n</li>\n</ol>\n"
    );
    let one_line = changes(&changes_of(Some("# Cache & \"quotes\" > 1\n"), None), "d");
    assert_eq!(
        one_line,
        "<ol class=\"changes\" data-path=\"doc.md\" data-file=\"removed\">\n<li id=\"d.0\" class=\"change document\" data-kind=\"removed\" data-level=\"fact\" data-signals=\"heading number\" data-bytes=\"23\" data-lines=\"1\">\n<p class=\"title\">Cache &amp; &quot;quotes&quot; &gt; 1</p>\n<p class=\"size\" data-gen>1 line</p>\n</li>\n</ol>\n"
    );
    let summary = changes(
        &changes_of(None, Some("# T\n\n> Short version: what it is for.\n")),
        "d",
    );
    assert!(
        summary.contains("<p class=\"title\">T</p>\n<p class=\"summary\">Short version: what it is for.</p>\n<p class=\"size\" data-gen>3 lines</p>"),
        "{summary}"
    );
}

#[test]
fn a_place_holder_after_every_change_comes_last() {
    let html = changes(
        &changes_of(
            Some("# T\n\nAlpha paragraph.\n\nBeta paragraph.\n\nGamma moves to the top.\n"),
            Some("# T\n\nGamma moves to the top.\n\nAlpha paragraph.\n\nBeta paragraph.\n"),
        ),
        "d",
    );
    let place_holder = html.find("class=\"moved-from\"").expect("a place-holder");
    let moved = html.find("class=\"moved\"").expect("the moved block");
    assert!(moved < place_holder, "{html}");
    assert!(html.contains("data-new=\"4\""), "{html}");
}

#[test]
fn whole_code_blocks_and_rare_kinds_are_drawn_whole() {
    let html = changes(
        &changes_of(
            Some("# T\n"),
            Some(
                "# T\n\n```d2\na -> b\n```\n\n---\n\n<div>x</div>\n\na[^n]\n\n[^n]: The note.\n\n| H |\n|---|\n| c |\n\n[x]: https://example.com\n",
            ),
        ),
        "d",
    );
    for expected in [
        "data-block=\"diagram\" data-diagram=\"d2\">",
        "<div class=\"side\" data-side=\"new\"><pre><code><ins>a -&gt; b</ins></code></pre></div>",
        "data-block=\"thematic_break\"",
        "data-block=\"html\"",
        "data-block=\"footnote_definition\" data-footnote=\"n\"",
        "data-block=\"table_row\" data-header=\"\"",
        "data-block=\"unrendered\"",
    ] {
        assert!(html.contains(expected), "{expected}\n{html}");
    }
}

#[test]
fn the_lines_view_names_the_language_when_there_is_one() {
    let html = changes(
        &changes_of(
            Some("```\na\nb\n```\n\n```d2\nx\ny\n```\n"),
            Some("```\na\nc\n```\n\n```d2\nx\nz\n```\n"),
        ),
        "d",
    );
    assert!(html.contains("<pre class=\"lines\"><code>a\n"), "{html}");
    assert!(
        html.contains("<pre class=\"lines\"><code class=\"language-d2\">x\n"),
        "{html}"
    );
    // A lines view of a block that is not code has no language.
    let text = block(BlockKind::Paragraph, "a");
    let mut view = edit(View::Lines);
    view.lines = vec![mark(MarkOp::Kept, "a")];
    let html = changes(&one(edited(view, text.clone(), text)), "d");
    assert!(
        html.contains("<pre class=\"lines\"><code>a\n</code></pre>"),
        "{html}"
    );
}

#[test]
fn sentence_rows_show_deleted_and_inserted_sentences_on_their_side() {
    let mut view = edit(View::Sentences);
    view.sentences = vec![
        vec![mark(MarkOp::Kept, "One. ")],
        vec![mark(MarkOp::Deleted, "Two. ")],
        vec![mark(MarkOp::Inserted, "Three.")],
    ];
    let text = block(BlockKind::Paragraph, "x");
    let html = changes(&one(edited(view, text.clone(), text)), "d");
    assert!(
        html.contains("<div class=\"rows\">\n<div class=\"row\" data-row=\"kept\"><div class=\"side\" data-side=\"both\">One. </div>\n</div>\n<div class=\"row\" data-row=\"deleted\"><div class=\"side\" data-side=\"old\"><del>Two. </del></div>\n</div>\n<div class=\"row\" data-row=\"inserted\"><div class=\"side\" data-side=\"new\"><ins>Three.</ins></div>\n</div>\n</div>"),
        "{html}"
    );
}

#[test]
fn a_fallback_shows_both_versions_whole() {
    let mut view = edit(View::SideBySide);
    view.fallback = true;
    let html = changes(
        &one(edited(
            view,
            block(BlockKind::Paragraph, "old <text>"),
            block(BlockKind::Paragraph, "new\0\r text"),
        )),
        "d",
    );
    assert!(
        html.contains(" data-view=\"side_by_side\" data-fallback "),
        "{html}"
    );
    assert!(
        html.contains("<div class=\"side\" data-side=\"old\">old &lt;text&gt;</div>\n<div class=\"side\" data-side=\"new\">new\u{fffd} text</div>"),
        "{html}"
    );
}

#[test]
fn a_change_without_blocks_still_has_its_element() {
    let change = Change {
        kind: ChangeKind::Edited,
        level: Level::Wording,
        signals: vec![Signal::Wording],
        subject: Subject::Block {
            old: Side { at: 0, block: None },
            new: Side { at: 0, block: None },
        },
        edit: None,
    };
    let html = changes(&one(change), "d");
    assert_eq!(
        html,
        "<ol class=\"changes\" data-path=\"doc.md\" data-file=\"modified\">\n<li id=\"d.0\" class=\"change\" data-kind=\"edited\" data-level=\"wording\" data-signals=\"wording\" data-old=\"0\" data-new=\"0\">\n<div class=\"side\" data-side=\"new\"></div>\n</li>\n</ol>\n"
    );
}

#[test]
fn old_details_list_only_what_changed() {
    let old = vec![("depth", "0".to_owned()), ("task", "open".to_owned())];
    let new = vec![("depth", "0".to_owned()), ("number", "2".to_owned())];
    assert_eq!(
        old_changes(&old, &new),
        [("task", "open".to_owned()), ("number", String::new())]
    );
    let summary = Summary {
        changes: 1,
        highest: Some(Level::Fact),
        levels: LevelCounts::default(),
        kinds: KindCounts::default(),
    };
    assert_eq!(summary.highest.map(level_name), Some("fact"));
}
