//! Golden tests of the change list on the made-up document pairs in
//! `/fixtures/diff`.
//!
//! Each folder holds `a.md` (the old version) and `b.md` (the new one); a
//! missing file means the document was added or removed. An optional
//! `driftmark.toml` is the rules file, and the document's path is `doc.md`.
//! `expected.txt` is the change list written out for reading; `expected.json`,
//! where present, is the JSON contract itself. Every run also writes both to
//! `CARGO_TARGET_TMPDIR/diff-dumps/`, so a deliberate change can be reviewed
//! and copied over by hand.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use driftmark_core::diff::{
    Change, ChangeKind, DocumentChanges, Edit, Mark, MarkOp, Side, Version, View, compare,
};
use driftmark_core::document::Document;
use driftmark_core::rules::{Classification, Rules};

const PATH: &str = "doc.md";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/diff")
}

fn read(folder: &Path, name: &str) -> Option<Vec<u8>> {
    fs::read(folder.join(name)).ok()
}

/// The change list of one fixture folder, with the parsed documents.
fn run(folder: &Path) -> (DocumentChanges, Option<Document>, Option<Document>) {
    let rules = read(folder, "driftmark.toml").map_or_else(Rules::default, |text| {
        Rules::parse(&String::from_utf8(text).expect("UTF-8")).expect("the rules parse")
    });
    let parse =
        |name: &str| read(folder, name).map(|bytes| Document::parse(bytes).expect("parses"));
    let old = parse("a.md");
    let new = parse("b.md");
    let old_class = old.as_ref().map(|document| rules.classify(PATH, document));
    let new_class = new.as_ref().map(|document| rules.classify(PATH, document));
    let old_version = version(old.as_ref(), old_class.as_ref());
    let new_version = version(new.as_ref(), new_class.as_ref());
    let changes = compare(PATH, old_version, new_version).expect("at least one side exists");
    (changes, old, new)
}

fn version<'a>(
    document: Option<&'a Document>,
    classification: Option<&'a Classification>,
) -> Option<Version<'a>> {
    document
        .zip(classification)
        .map(|(document, classification)| Version {
            document,
            classification,
        })
}

fn marks(marks: &[Mark]) -> String {
    marks
        .iter()
        .map(|mark| match mark.op {
            MarkOp::Kept => mark.text.clone(),
            MarkOp::Deleted => format!("[-{}-]", mark.text),
            MarkOp::Inserted => format!("{{+{}+}}", mark.text),
        })
        .collect::<String>()
        .replace('\n', "⏎")
        .replace('\t', "⇥")
}

fn side(name: &str, side: &Side) -> String {
    match &side.block {
        Some(block) => format!("{name} {} (line {})", side.at, block.line),
        None => format!("{name} @{}", side.at),
    }
}

fn edit(out: &mut String, edit: &Edit) {
    let view = match edit.view {
        View::Inline => "inline",
        View::Sentences => "sentences",
        View::SideBySide => "side by side",
        View::Rewritten => "rewritten",
        View::Lines => "lines",
    };
    writeln!(
        out,
        "    view: {view}, kept in order {}",
        edit.kept_in_order
    )
    .expect("write");
    if !edit.cells.is_empty() {
        writeln!(out, "    cells: {:?}", edit.cells).expect("write");
    }
    if edit.fallback {
        writeln!(out, "    fallback").expect("write");
    }
    if !edit.marks.is_empty() {
        writeln!(out, "    marks: {}", marks(&edit.marks)).expect("write");
    }
    for row in &edit.sentences {
        writeln!(out, "    | {}", marks(row)).expect("write");
    }
    for line in &edit.lines {
        let sign = match line.op {
            MarkOp::Kept => ' ',
            MarkOp::Deleted => '-',
            MarkOp::Inserted => '+',
        };
        writeln!(out, "    {sign} {}", line.text).expect("write");
    }
}

fn kind(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Formatting => "formatting",
        ChangeKind::Edited => "edited",
        ChangeKind::Added => "added",
        ChangeKind::Removed => "removed",
        ChangeKind::Moved => "moved",
        ChangeKind::MovedEdited => "moved and edited",
    }
}

fn text(change: &Change) -> String {
    let block = change
        .new
        .block
        .as_ref()
        .or(change.old.block.as_ref())
        .expect("a change has a block on one side");
    block.text.replace('\n', "⏎").replace('\t', "⇥")
}

/// The change list as text: the summary, then one entry per change.
fn dump(changes: &DocumentChanges) -> String {
    let mut out = String::new();
    let summary = &changes.summary;
    writeln!(out, "file: {:?}", changes.file).expect("write");
    writeln!(out, "status: {:?}", changes.status).expect("write");
    writeln!(
        out,
        "changes: {}, highest: {:?}",
        summary.changes, summary.highest
    )
    .expect("write");
    writeln!(out, "levels: {:?}", summary.levels).expect("write");
    writeln!(out, "kinds: {:?}", summary.kinds).expect("write");
    writeln!(out, "unchanged: {:?}", changes.unchanged).expect("write");
    for change in &changes.changes {
        writeln!(
            out,
            "\n{} — {:?} {:?} — {}, {}",
            kind(change.kind),
            change.level,
            change.signals,
            side("old", &change.old),
            side("new", &change.new),
        )
        .expect("write");
        match &change.edit {
            Some(found) => edit(&mut out, found),
            None => writeln!(out, "    text: {}", text(change)).expect("write"),
        }
    }
    out
}

/// The guarantees every change list keeps.
fn check_invariants(
    name: &str,
    changes: &DocumentChanges,
    old: Option<&Document>,
    new: Option<&Document>,
) {
    let count = |document: Option<&Document>| document.map_or(0, |document| document.blocks.len());
    let mut old_seen = vec![0; count(old)];
    let mut new_seen = vec![0; count(new)];
    for &[o, n] in &changes.unchanged {
        old_seen[o] += 1;
        new_seen[n] += 1;
    }
    let mut last_at = 0;
    for change in &changes.changes {
        if change.old.block.is_some() {
            old_seen[change.old.at] += 1;
        }
        if change.new.block.is_some() {
            new_seen[change.new.at] += 1;
        }
        assert!(
            change.new.at >= last_at,
            "{name}: changes are not in the new version's order"
        );
        last_at = change.new.at;
        assert!(
            !change.signals.is_empty(),
            "{name}: a change without signals"
        );
        if let (Some(edit), Some(block)) = (&change.edit, &change.new.block) {
            let shown: String = edit
                .marks
                .iter()
                .chain(edit.sentences.iter().flatten())
                .filter(|mark| mark.op != MarkOp::Deleted)
                .map(|mark| mark.text.as_str())
                .collect();
            if !shown.is_empty() {
                assert_eq!(shown, block.text, "{name}: the marks change the new text");
            }
        }
    }
    assert!(
        old_seen.iter().all(|&seen| seen == 1),
        "{name}: old blocks {old_seen:?}"
    );
    assert!(
        new_seen.iter().all(|&seen| seen == 1),
        "{name}: new blocks {new_seen:?}"
    );
}

#[test]
fn every_fixture_gives_its_expected_change_list() {
    let dumps = Path::new(env!("CARGO_TARGET_TMPDIR")).join("diff-dumps");
    let mut folders: Vec<PathBuf> = fs::read_dir(fixtures())
        .expect("the fixtures folder")
        .map(|entry| entry.expect("an entry").path())
        .filter(|path| path.is_dir())
        .collect();
    folders.sort();
    assert!(folders.len() > 20, "fixtures are missing");
    let mut failures = Vec::new();
    for folder in folders {
        let name = folder
            .file_name()
            .expect("a name")
            .to_string_lossy()
            .into_owned();
        let (changes, old, new) = run(&folder);
        check_invariants(&name, &changes, old.as_ref(), new.as_ref());
        let text = dump(&changes);
        let json = serde_json::to_string_pretty(&changes).expect("serializes") + "\n";
        let out = dumps.join(&name);
        fs::create_dir_all(&out).expect("the dump folder");
        fs::write(out.join("expected.txt"), &text).expect("a dump");
        fs::write(out.join("expected.json"), &json).expect("a dump");
        match read(&folder, "expected.txt") {
            Some(expected) if expected == text.as_bytes() => {}
            _ => failures.push(format!("{name}/expected.txt")),
        }
        if let Some(expected) = read(&folder, "expected.json")
            && expected != json.as_bytes()
        {
            failures.push(format!("{name}/expected.json"));
        }
    }
    assert!(
        failures.is_empty(),
        "these differ from the fresh dumps in {}: {failures:?}",
        dumps.display()
    );
}
