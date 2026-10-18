//! Golden tests of re-anchoring on the made-up edits in `/fixtures/notes`.
//!
//! Each folder holds `a.md`, the version notes were made on, `b.md`, a later
//! version, and `notes.toml`, the notes: each names its `quote`, which of
//! its occurrences in `a.md` it is (`nth`, from 0), and what the anchoring
//! knows of `a.md` (`origin`: `known`, the default; `same`, when `b.md` is
//! the very same bytes; or `lost`, when `a.md` cannot be read any more).
//! `expected.txt` is where each note lands in `b.md`, written out. Every run
//! also writes it to `CARGO_TARGET_TMPDIR/notes-dumps/`, so a deliberate
//! change can be reviewed and copied over by hand.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use driftmark_core::document::Document;
use driftmark_core::notes::{Anchor, AnchorState, Origin, reanchor};
use serde::Deserialize;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/notes")
}

#[derive(Deserialize)]
struct Notes {
    note: Vec<NoteSpec>,
}

#[derive(Deserialize)]
struct NoteSpec {
    quote: String,
    #[serde(default)]
    nth: usize,
    #[serde(default)]
    origin: Option<String>,
}

fn parse(folder: &Path, name: &str) -> Document {
    let bytes = fs::read(folder.join(name)).expect("the version is there");
    Document::parse(bytes).expect("it parses")
}

fn shown(text: &str) -> String {
    text.replace('\n', "⏎")
}

/// Where every note of one folder lands, written out.
fn run(folder: &Path) -> String {
    let old = parse(folder, "a.md");
    let new = parse(folder, "b.md");
    let spec: Notes = toml::from_str(
        &fs::read_to_string(folder.join("notes.toml")).expect("the notes are there"),
    )
    .expect("the notes parse");
    let mut out = String::new();
    for (number, note) in spec.note.iter().enumerate() {
        let start = old
            .source
            .match_indices(&note.quote)
            .nth(note.nth)
            .map_or_else(
                || panic!("{}: quote {} not in a.md", folder.display(), number + 1),
                |(at, _)| at,
            );
        let anchor =
            Anchor::capture(&old, start..start + note.quote.len()).expect("the quote captures");
        let origin = match note.origin.as_deref() {
            None | Some("known") => Origin::Known(&old),
            Some("same") => Origin::Same,
            Some("lost") => Origin::Lost,
            Some(other) => panic!("unknown origin {other}"),
        };
        let placement = reanchor(&anchor, origin, &new);
        writeln!(
            out,
            "note {}: \"{}\" (bytes {}..{}, block {}, under {})",
            number + 1,
            shown(&note.quote),
            anchor.start,
            anchor.end,
            anchor.block,
            anchor.heading_path.join(" › ")
        )
        .expect("write");
        let mut line = format!("  {}, score {}", placement.state.name(), placement.score);
        if let Some(block) = placement.block {
            write!(line, ", block {block}").expect("write");
        }
        writeln!(out, "{line}").expect("write");
        if let Some(range) = &placement.range {
            let block = &new.blocks[placement.block.expect("a placed note has a block")];
            writeln!(
                out,
                "  now: \"{}\" (bytes {}..{}, under {})",
                shown(&new.source[range.clone()]),
                range.start,
                range.end,
                block.heading_path.join(" › ")
            )
            .expect("write");
        } else {
            assert_eq!(placement.state, AnchorState::Orphaned);
            writeln!(out, "  not in b.md").expect("write");
        }
    }
    out
}

#[test]
fn every_note_lands_where_its_fixture_expects() {
    let dumps = Path::new(env!("CARGO_TARGET_TMPDIR")).join("notes-dumps");
    fs::create_dir_all(&dumps).expect("the dump folder");
    let mut folders: Vec<PathBuf> = fs::read_dir(fixtures())
        .expect("the fixtures folder")
        .map(|entry| entry.expect("an entry").path())
        .filter(|path| path.is_dir())
        .collect();
    folders.sort();
    assert!(folders.len() >= 10, "fixtures are missing");
    let mut failures = Vec::new();
    let mut states = Vec::new();
    for folder in folders {
        let name = folder
            .file_name()
            .expect("a name")
            .to_string_lossy()
            .into_owned();
        let text = run(&folder);
        for state in ["exact", "moved", "fuzzy", "outdated", "orphaned"] {
            if text.contains(&format!("  {state}, ")) && !states.contains(&state) {
                states.push(state);
            }
        }
        fs::write(dumps.join(format!("{name}.txt")), &text).expect("a dump");
        let expected = fs::read_to_string(folder.join("expected.txt")).unwrap_or_default();
        if expected != text {
            failures.push(name);
        }
    }
    assert!(
        failures.is_empty(),
        "these differ from the fresh dumps in {}: {failures:?}",
        dumps.display()
    );
    states.sort_unstable();
    assert_eq!(
        states,
        ["exact", "fuzzy", "moved", "orphaned", "outdated"],
        "the fixtures cover every state"
    );
}
