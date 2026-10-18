//! Reading a document's metadata: its title, its summary, and the key-value
//! fields of its front matter and masthead.
//!
//! The masthead is the `Key: Value` block right under the title. The brief's
//! form is a quote line such as `> Status: Accepted | Date: 2026-01-05`. The
//! same lines are also read from a plain paragraph (GitHub Spec Kit writes
//! `**Status**: Draft`) and from a list (MADR 2 writes `* Status: accepted`),
//! so presets for those layouts can name masthead keys too.
//!
//! Only known keys are metadata ([`MastheadKeys`]). A block whose lines read
//! as `Key: Value` but hold no known key is not a masthead, so a quote such
//! as `> Short version: …` is the summary however short it is.

use std::ops::Range;

use comrak::arena_tree::NodeEdge;
use comrak::nodes::{AstNode, NodeValue};

use super::content::front_matter_body;
use super::markdown::{code_span_text, inline_text};
use super::{Field, FieldOrigin, MastheadKeys, MetaValue, Metadata, yaml};

/// How many blocks right under the title are looked at for the summary and
/// the masthead.
const HEAD_BLOCKS: usize = 3;
/// The most words a masthead key may have.
const KEY_WORDS: usize = 4;
/// The most characters a masthead key may have.
const KEY_CHARACTERS: usize = 40;

/// Reads the metadata of a parsed document. Masthead fields are kept only
/// when `keys` knows their key.
pub(crate) fn read<'a>(root: &'a AstNode<'a>, keys: &MastheadKeys) -> Metadata {
    let mut meta = Metadata::default();
    for node in root.children() {
        match &node.data().value {
            NodeValue::FrontMatter(literal) => match yaml::read(front_matter_body(literal)) {
                Ok(entries) => meta
                    .fields
                    .extend(entries.into_iter().map(|(key, value)| Field {
                        key,
                        value,
                        origin: FieldOrigin::FrontMatter,
                        links: Vec::new(),
                    })),
                Err(error) => meta.front_matter_error = Some(error),
            },
            NodeValue::Heading(heading) if heading.level == 1 => {
                meta.title = Some(inline_text(node).trim().to_owned());
                read_head(node, keys, &mut meta);
                break;
            }
            _ => {}
        }
    }
    meta
}

/// What a block right under the title turned out to be. The writing check
/// uses it too, to tell a summary from a masthead.
pub(crate) enum Head {
    Masthead(Vec<Field>),
    Prose(String),
    Other,
}

/// Reads the summary and the masthead from the blocks right under the title.
fn read_head<'a>(title: &'a AstNode<'a>, keys: &MastheadKeys, meta: &mut Metadata) {
    let mut next = title.next_sibling();
    for _ in 0..HEAD_BLOCKS {
        let Some(node) = next else { break };
        match head(node, keys) {
            Head::Masthead(fields) => meta.fields.extend(fields),
            Head::Prose(text) if meta.summary.is_none() => meta.summary = Some(text),
            Head::Prose(_) | Head::Other => break,
        }
        next = node.next_sibling();
    }
}

/// Reads one top-level block as a masthead, as prose or as neither. Only
/// fields whose key `keys` knows make a masthead.
pub(crate) fn head<'a>(node: &'a AstNode<'a>, keys: &MastheadKeys) -> Head {
    let children: Vec<&'a AstNode<'a>> = node.children().collect();
    let only_paragraphs = !children.is_empty()
        && children
            .iter()
            .all(|child| matches!(child.data().value, NodeValue::Paragraph));
    match &node.data().value {
        NodeValue::BlockQuote if only_paragraphs => {
            let lines = children
                .iter()
                .flat_map(|child| lines(child, true))
                .collect::<Vec<_>>();
            masthead(&lines, keys).map_or_else(
                || {
                    let paragraphs: Vec<String> = children
                        .iter()
                        .map(|child| inline_text(child).trim().to_owned())
                        .collect();
                    Head::Prose(paragraphs.join("\n\n"))
                },
                Head::Masthead,
            )
        }
        NodeValue::Paragraph => {
            masthead(&lines(node, true), keys).map_or(Head::Other, Head::Masthead)
        }
        NodeValue::List(_) => {
            // Each item must hold exactly one paragraph, which is one line.
            let paragraphs: Vec<&'a AstNode<'a>> = children
                .iter()
                .filter_map(|item| item.first_child().filter(|_| item.children().count() == 1))
                .filter(|child| matches!(child.data().value, NodeValue::Paragraph))
                .collect();
            if paragraphs.len() == children.len() {
                let lines: Vec<Line> = paragraphs
                    .iter()
                    .flat_map(|child| lines(child, false))
                    .collect();
                masthead(&lines, keys).map_or(Head::Other, Head::Masthead)
            } else {
                Head::Other
            }
        }
        _ => Head::Other,
    }
}

/// One line of rendered text, with the targets of the links on it.
#[derive(Default)]
struct Line {
    text: String,
    /// Where each link starts in `text`, and its target.
    links: Vec<(usize, String)>,
}

/// The rendered lines of a paragraph. Hard line breaks always start a new
/// line; soft ones do when `split_soft` is set.
fn lines<'a>(paragraph: &'a AstNode<'a>, split_soft: bool) -> Vec<Line> {
    let mut lines = vec![Line::default()];
    for edge in paragraph.traverse() {
        if let NodeEdge::Start(inner) = edge {
            let current = lines.len() - 1;
            match &inner.data().value {
                NodeValue::Text(text) => lines[current].text.push_str(text),
                NodeValue::Code(code) => {
                    lines[current].text.push_str(&code_span_text(&code.literal));
                }
                NodeValue::SoftBreak if !split_soft => lines[current].text.push(' '),
                NodeValue::SoftBreak | NodeValue::LineBreak => lines.push(Line::default()),
                NodeValue::Link(link) => {
                    let at = lines[current].text.len();
                    lines[current].links.push((at, link.url.clone()));
                }
                _ => {}
            }
        }
    }
    lines
}

/// The fields of masthead lines whose key `keys` knows, when every line is
/// `Key: Value` and at least one key is known. A field with another key is
/// left out: it is not metadata. A block with no known key is not a
/// masthead, whatever the length of its values.
fn masthead(lines: &[Line], keys: &MastheadKeys) -> Option<Vec<Field>> {
    let mut fields = Vec::new();
    for line in lines {
        fields.extend(parse_line(line)?);
    }
    fields.retain(|field| keys.knows(&field.key));
    (!fields.is_empty()).then_some(fields)
}

/// Splits one line into `Key: Value` fields at ` | ` and ` · `. A piece that
/// is not `Key: Value` belongs to the value before it; the first piece must
/// be one.
fn parse_line(line: &Line) -> Option<Vec<Field>> {
    let text = line.text.as_str();
    let mut found: Vec<(String, Range<usize>, Range<usize>)> = Vec::new();
    for piece in pieces(text) {
        if let Some((key, value_start)) = key_value(&text[piece.clone()]) {
            found.push((key, piece.start + value_start..piece.end, piece));
        } else {
            let (_, value, span) = found.last_mut()?;
            value.end = piece.end;
            span.end = piece.end;
        }
    }
    Some(
        found
            .into_iter()
            .map(|(key, value, span)| Field {
                key,
                value: MetaValue::Text(text[value].trim().to_owned()),
                origin: FieldOrigin::Masthead,
                links: line
                    .links
                    .iter()
                    .filter(|(at, _)| span.contains(at))
                    .map(|(_, target)| target.clone())
                    .collect(),
            })
            .collect(),
    )
}

/// The ranges of `text` between the separators ` | ` and ` · `.
fn pieces(text: &str) -> Vec<Range<usize>> {
    let mut cuts: Vec<(usize, usize)> = text
        .match_indices(" | ")
        .chain(text.match_indices(" · "))
        .map(|(at, separator)| (at, at + separator.len()))
        .collect();
    cuts.sort_unstable();
    let mut pieces = Vec::new();
    let mut start = 0;
    for (at, after) in cuts {
        pieces.push(start..at);
        start = after;
    }
    pieces.push(start..text.len());
    pieces
}

/// The key of a `Key: Value` piece and where its value starts.
///
/// A key starts with a letter, has one to four words and at most 40
/// characters, and holds only letters, digits, spaces and `-_/.'()`. A value
/// starting with `//` means the colon belonged to a web address.
fn key_value(piece: &str) -> Option<(String, usize)> {
    let colon = piece.find(':')?;
    let key = piece[..colon].trim();
    let valid = is_key(key) && !piece[colon + 1..].starts_with("//");
    valid.then(|| (key.to_owned(), colon + 1))
}

/// Whether `key` has the shape of a masthead key: it starts with a letter,
/// has one to four words and at most 40 characters, and holds only letters,
/// digits, spaces and `-_/.'()`. The rules file's own masthead keys must have
/// it too, or no line could ever match them.
pub(crate) fn is_key(key: &str) -> bool {
    let words = key.split_whitespace().count();
    (1..=KEY_WORDS).contains(&words)
        && key.chars().count() <= KEY_CHARACTERS
        && key.starts_with(char::is_alphabetic)
        && key
            .chars()
            .all(|c| c.is_alphanumeric() || " -_/.'()".contains(c))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::markdown::options;
    use comrak::{Arena, parse_document};

    fn meta(source: &str) -> Metadata {
        meta_with(source, &MastheadKeys::default())
    }

    fn meta_with(source: &str, keys: &MastheadKeys) -> Metadata {
        let arena = Arena::new();
        let root = parse_document(&arena, source, &options());
        read(root, keys)
    }

    fn masthead_field(key: &str, value: &str, links: &[&str]) -> Field {
        Field {
            key: key.to_owned(),
            value: MetaValue::Text(value.to_owned()),
            origin: FieldOrigin::Masthead,
            links: links.iter().map(|&link| link.to_owned()).collect(),
        }
    }

    fn line(text: &str) -> Line {
        Line {
            text: text.to_owned(),
            links: Vec::new(),
        }
    }

    #[test]
    fn the_title_is_the_first_level_one_heading() {
        assert_eq!(
            meta("## Intro\n\n# Real *title*\n\n# Second\n")
                .title
                .as_deref(),
            Some("Real title")
        );
        assert_eq!(meta("Title\n=====\n").title.as_deref(), Some("Title"));
        assert_eq!(meta("## Only a section\n").title, None);
        assert_eq!(meta("> # Quoted\n").title, None);
    }

    #[test]
    fn the_summary_is_the_prose_quote_under_the_title() {
        let found = meta(
            "# T\n\n> Short version: a tool that *reads* docs and shows what changed in them since last week.\n>\n> Second part.\n\nBody.\n",
        );
        assert_eq!(
            found.summary.as_deref(),
            Some(
                "Short version: a tool that reads docs and shows what changed in them since last week.\n\nSecond part."
            )
        );
        assert_eq!(found.fields, []);
        assert_eq!(meta("# T\n\nBody.\n\n> Not right under.\n").summary, None);
        assert_eq!(meta("# T\n\n> - a list\n").summary, None);
    }

    #[test]
    fn a_masthead_line_gives_fields() {
        let found = meta(
            "# T\n\n> Status: Accepted | **Date:** 2026-01-05 · Superseded by: [0009](0009-x.md)\n",
        );
        assert_eq!(
            found.fields,
            vec![
                masthead_field("Status", "Accepted", &[]),
                masthead_field("Date", "2026-01-05", &[]),
                masthead_field("Superseded by", "0009", &["0009-x.md"]),
            ]
        );
        assert_eq!(found.summary, None);
    }

    #[test]
    fn masthead_and_summary_can_both_follow_the_title() {
        let found = meta(
            "# T\n\n> Status: Draft\n> Owner: Ana\n\n> The summary sentence.\n\n> A second quote.\n",
        );
        assert_eq!(found.fields.len(), 2);
        assert_eq!(found.summary.as_deref(), Some("The summary sentence."));
        let reversed = meta("# T\n\n> The summary sentence.\n\n> Status: Draft\n");
        assert_eq!(
            reversed.fields,
            vec![masthead_field("Status", "Draft", &[])]
        );
        assert_eq!(reversed.summary.as_deref(), Some("The summary sentence."));
    }

    #[test]
    fn only_three_blocks_under_the_title_are_read() {
        let found = meta("# T\n\n> Type: 1\n\n> Tags: 2\n\n> Owner: 3\n\n> Date: 4\n");
        assert_eq!(found.fields.len(), 3);
    }

    #[test]
    fn a_paragraph_or_a_list_can_be_the_masthead() {
        let keys = MastheadKeys::with(["Created", "Deciders"]);
        let spec_kit = meta_with(
            "# T\n\n**Created**: 2026-01-05  \n**Status**: Draft\n",
            &keys,
        );
        assert_eq!(
            spec_kit.fields,
            vec![
                masthead_field("Created", "2026-01-05", &[]),
                masthead_field("Status", "Draft", &[])
            ]
        );
        let madr = meta_with(
            "# T\n\n* Status: accepted\n* Deciders: Ana,\n  Ben\n",
            &keys,
        );
        assert_eq!(
            madr.fields,
            vec![
                masthead_field("Status", "accepted", &[]),
                masthead_field("Deciders", "Ana, Ben", &[])
            ]
        );
        assert_eq!(meta("# T\n\nPlain prose here.\n").fields, []);
        assert_eq!(meta("# T\n\n- Status: x\n\n  more\n").fields, []);
        assert_eq!(meta("# T\n\n- Status: x\n- not a field\n").fields, []);
        assert_eq!(meta("# T\n\n- ```\n  code\n  ```\n").fields, []);
    }

    #[test]
    fn only_known_keys_are_metadata_whatever_the_length() {
        let short = meta("# T\n\n> Short version: pages expire after an hour.\n");
        assert_eq!(short.fields, []);
        assert_eq!(
            short.summary.as_deref(),
            Some("Short version: pages expire after an hour.")
        );
        let long = format!("# T\n\n> Status: {}\n", "word ".repeat(30).trim_end());
        let found = meta(&long);
        assert_eq!(found.fields.len(), 1);
        assert_eq!(found.summary, None);
        let mixed = meta("# T\n\n> Status: Draft | Deciders: Ana\n> Owner: Ben\n");
        assert_eq!(
            mixed.fields,
            vec![
                masthead_field("Status", "Draft", &[]),
                masthead_field("Owner", "Ben", &[])
            ]
        );
        let unknown = meta("# T\n\n**Created**: 2026-01-05  \n**Input**: a request\n");
        assert_eq!(unknown.fields, []);
        assert_eq!(unknown.summary, None);
        let custom = meta_with(
            "# T\n\n> Short version: kept as a field.\n",
            &MastheadKeys::with(["short-version"]),
        );
        assert_eq!(custom.fields.len(), 1);
        assert_eq!(custom.summary, None);
    }

    #[test]
    fn front_matter_fields_come_first() {
        let found = meta("---\nstatus: accepted\nanything: [a]\n---\n\n# T\n\n> Owner: Ana\n");
        let keys: Vec<(&str, FieldOrigin)> = found
            .fields
            .iter()
            .map(|field| (field.key.as_str(), field.origin))
            .collect();
        assert_eq!(
            keys,
            vec![
                ("status", FieldOrigin::FrontMatter),
                ("anything", FieldOrigin::FrontMatter),
                ("Owner", FieldOrigin::Masthead)
            ]
        );
        assert_eq!(found.front_matter_error, None);
    }

    #[test]
    fn bad_front_matter_is_reported_not_fatal() {
        let found = meta("---\n- a list\n---\n\n# T\n");
        assert_eq!(
            found.front_matter_error.as_deref(),
            Some("the front matter is not a list of keys and values")
        );
        assert_eq!(found.title.as_deref(), Some("T"));
    }

    #[test]
    fn keys_follow_the_key_rules() {
        assert_eq!(key_value("Status: x"), Some(("Status".to_owned(), 7)));
        assert_eq!(
            key_value(" Last updated : x"),
            Some(("Last updated".to_owned(), 15))
        );
        assert_eq!(
            key_value("a b c d: x").map(|(key, _)| key),
            Some("a b c d".to_owned())
        );
        assert_eq!(key_value("a b c d e: x"), None);
        assert_eq!(
            key_value(&format!("{}: x", "k".repeat(40))).map(|(_, at)| at),
            Some(41)
        );
        assert_eq!(key_value(&format!("{}: x", "k".repeat(41))), None);
        assert_eq!(key_value("1st: x"), None);
        assert_eq!(key_value(": x"), None);
        assert_eq!(key_value("no colon"), None);
        assert_eq!(key_value("a*b: x"), None);
        assert_eq!(
            key_value("Owner (team) - it's/a_b.c: x").map(|(_, at)| at),
            Some(26)
        );
        assert_eq!(key_value("https://example.com"), None);
        assert!(is_key("Superseded by") && is_key("Feature Branch"));
        assert!(!is_key("") && !is_key("a*b") && !is_key("a b c d e"));
        assert_eq!(
            key_value("სტატუსი: x").map(|(key, _)| key),
            Some("სტატუსი".to_owned())
        );
    }

    #[test]
    fn pieces_split_at_both_separators() {
        assert_eq!(pieces("a | b · c"), vec![0..1, 4..5, 9..10]);
        assert_eq!(pieces("a · b | c"), vec![0..1, 5..6, 9..10]);
        assert_eq!(pieces("a|b"), vec![0..3]);
        assert_eq!(pieces(""), vec![0..0]);
    }

    #[test]
    fn a_piece_without_a_key_joins_the_value_before_it() {
        let fields = parse_line(&line("Affects: a | b | Date: c")).expect("a masthead line");
        assert_eq!(fields[0].value.as_text(), Some("a | b"));
        assert_eq!(fields[1].value.as_text(), Some("c"));
        assert_eq!(parse_line(&line("not a key | Date: c")), None);
    }

    #[test]
    fn a_link_belongs_to_the_field_it_starts_in() {
        let fields = parse_line(&Line {
            text: "A: x | B: y".to_owned(),
            links: vec![
                (3, "first".to_owned()),
                (5, "between".to_owned()),
                (7, "second".to_owned()),
                (10, "third".to_owned()),
            ],
        })
        .expect("a masthead line");
        assert_eq!(fields[0].links, vec!["first".to_owned()]);
        assert_eq!(
            fields[1].links,
            vec!["second".to_owned(), "third".to_owned()]
        );
    }

    #[test]
    fn lines_follow_line_breaks() {
        let arena = Arena::new();
        let root = parse_document(&arena, "a `b`\nc\\\nd\n", &options());
        let paragraph = root.first_child().expect("a paragraph");
        let split: Vec<String> = lines(paragraph, true)
            .into_iter()
            .map(|line| line.text)
            .collect();
        assert_eq!(split, vec!["a b", "c", "d"]);
        let joined: Vec<String> = lines(paragraph, false)
            .into_iter()
            .map(|line| line.text)
            .collect();
        assert_eq!(joined, vec!["a b c", "d"]);
    }
}
