//! Golden tests of the document model on the made-up documents in
//! `/fixtures`.
//!
//! Each `<name>.md` with a `<name>.expected.txt` beside it is parsed and
//! dumped, and the dump must equal the expected file. Every run also writes
//! the dump to `CARGO_TARGET_TMPDIR`, so a changed contract can be reviewed
//! and copied over by hand.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use driftmark_core::document::{BlockKind, Document, MAX_DEPTH, ParseError};

const GOLDEN: &[&str] = &[
    "block-kinds/input.md",
    "comrak-code-span-crlf/input.md",
    "comrak-escaped-pipes/input.md",
    "front-matter/input.md",
    "front-matter/not-yaml.md",
    "line-endings/bare-cr.md",
    "line-endings/byte-order-mark.md",
    "line-endings/crlf.md",
    "masthead/list.md",
    "masthead/paragraph.md",
    "masthead/quote-line.md",
    "masthead/quote-lines.md",
    "masthead/summary.md",
    "nesting/input.md",
    "no-title/input.md",
    "unclosed-fence/input.md",
    "unicode-text/input.md",
    "unrendered/input.md",
];

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

fn read(relative: &str) -> Vec<u8> {
    fs::read(fixtures().join(relative)).expect("the fixture exists")
}

fn parse(relative: &str) -> Document {
    Document::parse(read(relative)).expect("the fixture parses")
}

/// The document model as text: metadata first, then one entry per block.
fn dump(document: &Document) -> String {
    let mut out = String::new();
    let meta = &document.meta;
    writeln!(out, "title: {:?}", meta.title).expect("writing to a string");
    writeln!(out, "summary: {:?}", meta.summary).expect("writing to a string");
    writeln!(out, "front matter error: {:?}", meta.front_matter_error)
        .expect("writing to a string");
    for field in &meta.fields {
        writeln!(
            out,
            "field: {:?} {:?} = {:?}, links {:?}",
            field.origin, field.key, field.value, field.links
        )
        .expect("writing to a string");
    }
    for (index, block) in document.blocks.iter().enumerate() {
        writeln!(out, "\n[{index}] {:?} at {:?}", block.kind, block.range)
            .expect("writing to a string");
        writeln!(out, "    headings: {:?}", block.heading_path).expect("writing to a string");
        writeln!(
            out,
            "    containers: {:?}, parent: {:?}",
            block.containers, block.parent
        )
        .expect("writing to a string");
        writeln!(out, "    text: {:?}", block.text).expect("writing to a string");
        writeln!(out, "    source: {:?}", document.source_of(block)).expect("writing to a string");
    }
    out
}

/// The guarantees every parsed document keeps.
fn check_invariants(name: &str, document: &Document) {
    let source = &document.source;
    let mut covered = vec![false; source.len()];
    let mut previous_end = 0;
    for (index, block) in document.blocks.iter().enumerate() {
        let range = block.range.clone();
        assert!(
            source.get(range.clone()).is_some(),
            "{name} block {index}: the range must slice"
        );
        assert!(
            range.start >= previous_end,
            "{name} block {index}: blocks overlap or are out of order"
        );
        assert!(
            !range.is_empty(),
            "{name} block {index}: the range is empty"
        );
        let slice = &source[range.clone()];
        assert_eq!(
            slice.trim_end(),
            slice,
            "{name} block {index}: the range ends with blank characters"
        );
        previous_end = range.end;
        covered[range].fill(true);
        assert!(
            block.parent.is_none_or(|parent| parent < index),
            "{name} block {index}: a parent comes before its children"
        );
    }
    for (at, character) in source.char_indices() {
        let markup = character.is_ascii_whitespace() || character == '>' || character == '\u{feff}';
        assert!(
            covered[at] || markup,
            "{name}: {character:?} at byte {at} is in no block"
        );
    }
}

#[test]
fn golden_dumps_match() {
    let out_dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("document-dumps");
    let mut mismatches = Vec::new();
    for relative in GOLDEN {
        let document = parse(relative);
        check_invariants(relative, &document);
        let actual = dump(&document);
        let expected_name = relative.replace(".md", ".expected.txt");
        let written = out_dir.join(&expected_name);
        fs::create_dir_all(written.parent().expect("a folder")).expect("the dump folder");
        fs::write(&written, &actual).expect("the dump is written");
        let expected = fs::read_to_string(fixtures().join(&expected_name)).unwrap_or_default();
        mismatches
            .extend((actual != expected).then(|| format!("{relative}: see {}", written.display())));
    }
    assert_eq!(
        mismatches,
        Vec::<String>::new(),
        "dumps differ from the expected files"
    );
}

#[test]
fn every_block_kind_occurs_in_the_fixtures() {
    let mut seen = Vec::new();
    for relative in GOLDEN {
        for block in parse(relative).blocks {
            let name = match block.kind {
                BlockKind::FrontMatter => "front matter",
                BlockKind::Heading { .. } => "heading",
                BlockKind::Paragraph => "paragraph",
                BlockKind::ListItem(_) => "list item",
                BlockKind::TableRow { .. } => "table row",
                BlockKind::Code { .. } => "code",
                BlockKind::Diagram { .. } => "diagram",
                BlockKind::Alert { .. } => "alert",
                BlockKind::FootnoteDefinition { .. } => "footnote definition",
                BlockKind::Html => "html",
                BlockKind::ThematicBreak => "thematic break",
                BlockKind::Unrendered => "unrendered",
            };
            seen.push(name);
        }
    }
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), 12, "{seen:?}");
}

/// The names of the `<name>.a.md` / `<name>.b.md` pairs in a folder.
fn pairs(folder: &str) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(fixtures().join(folder))
        .expect("the pair folder exists")
        .map(|entry| {
            entry
                .expect("a folder entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter_map(|file| file.strip_suffix(".a.md").map(str::to_owned))
        .collect();
    names.sort();
    names
}

#[test]
fn formatting_only_changes_keep_the_content_hash() {
    let names = pairs("formatting-only");
    for required in ["wrapping", "emphasis", "table-padding"] {
        assert!(
            names.iter().any(|name| name == required),
            "the pair {required} is missing"
        );
    }
    for name in &names {
        let a = parse(&format!("formatting-only/{name}.a.md"));
        let b = parse(&format!("formatting-only/{name}.b.md"));
        assert_eq!(
            a.blocks.len(),
            b.blocks.len(),
            "{name}: block counts differ"
        );
        let mut source_changes = 0;
        for (left, right) in a.blocks.iter().zip(&b.blocks) {
            assert_eq!(left.kind, right.kind, "{name}: kinds differ");
            assert_eq!(
                left.content_hash, right.content_hash,
                "{name}: content hashes differ"
            );
            source_changes += usize::from(left.source_hash != right.source_hash);
        }
        assert!(
            source_changes > 0,
            "{name}: the pair must differ in its source"
        );
    }
}

#[test]
fn edits_change_the_content_hash() {
    let names = pairs("edited");
    assert!(names.len() >= 20, "{names:?}");
    for name in &names {
        let a = parse(&format!("edited/{name}.a.md"));
        let b = parse(&format!("edited/{name}.b.md"));
        let content = |document: &Document| -> Vec<u64> {
            document
                .blocks
                .iter()
                .map(|block| block.content_hash)
                .collect()
        };
        assert_ne!(
            content(&a),
            content(&b),
            "{name}: the edit must change a content hash"
        );
    }
}

#[test]
fn equal_source_gives_equal_hashes() {
    let document =
        Document::parse(b"Same words.\n\nSame words.\n\n- Same words.\n".to_vec()).expect("parses");
    let blocks = &document.blocks;
    assert_eq!(blocks[0].source_hash, blocks[1].source_hash);
    assert_eq!(blocks[0].content_hash, blocks[1].content_hash);
    assert_eq!(blocks[0].text, blocks[2].text);
    assert_ne!(
        blocks[0].content_hash, blocks[2].content_hash,
        "a list item is not a paragraph"
    );
}

#[test]
fn a_reference_link_takes_its_target_from_the_definition() {
    let a = parse("edited/reference-target.a.md");
    let b = parse("edited/reference-target.b.md");
    assert_eq!(a.blocks[0].source_hash, b.blocks[0].source_hash);
    assert_ne!(a.blocks[0].content_hash, b.blocks[0].content_hash);
    assert_ne!(a.blocks[1].content_hash, b.blocks[1].content_hash);
}

#[test]
fn a_file_that_is_not_utf8_is_refused() {
    assert_eq!(
        Document::parse(read("not-utf8/input.md")),
        Err(ParseError::NotUtf8 {
            offset: 19,
            line: 3
        })
    );
}

#[test]
fn a_file_nested_too_deep_is_refused() {
    assert_eq!(
        Document::parse(read("too-deep/input.md")),
        Err(ParseError::TooDeep {
            line: 3,
            limit: MAX_DEPTH
        })
    );
}

#[test]
fn every_fixture_keeps_the_guarantees() {
    for folder in ["formatting-only", "edited"] {
        for name in pairs(folder) {
            for side in ["a", "b"] {
                let relative = format!("{folder}/{name}.{side}.md");
                check_invariants(&relative, &parse(&relative));
            }
        }
    }
}
