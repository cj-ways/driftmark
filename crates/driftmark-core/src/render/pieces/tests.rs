//! Tests of the source map: each kind of text against its source bytes.

use comrak::nodes::LineColumn;

use super::*;
use crate::document::markdown::options;

fn find(root: Node<'_>, wanted: impl Fn(&NodeValue) -> bool) -> Node<'_> {
    root.descendants()
        .find(|node| wanted(&node.data().value))
        .expect("the test text has this node")
}

fn text_node<'a>(root: Node<'a>, wanted: &str) -> Node<'a> {
    find(
        root,
        |value| matches!(value, NodeValue::Text(text) if text == wanted),
    )
}

fn literal(node: Node<'_>) -> String {
    match &node.data().value {
        NodeValue::Text(text) => text.to_string(),
        NodeValue::Code(code) => code.literal.clone(),
        NodeValue::CodeBlock(code) => code.literal.clone(),
        _ => String::new(),
    }
}

/// The pieces as `(kind, text, source text)`, for reading in a test.
fn shown<'s>(source: &'s str, pieces: &[Piece]) -> Vec<(Kind, String, &'s str)> {
    pieces
        .iter()
        .map(|piece| {
            (
                piece.kind,
                piece.text.clone(),
                &source[piece.source.clone()],
            )
        })
        .collect()
}

fn position(line: usize, column: usize) -> LineColumn {
    LineColumn { line, column }
}

#[test]
fn plain_text_is_one_verbatim_piece() {
    let source = "Some plain text.\n";
    let arena = Arena::new();
    let root = parse_document(&arena, source, &options());
    let mut mapper = Mapper::new(source);
    let node = text_node(root, "Some plain text.");
    let pieces = mapper.text(node, &literal(node)).expect("a range");
    assert_eq!(
        shown(source, &pieces),
        [(
            Kind::Verbatim,
            "Some plain text.".to_owned(),
            "Some plain text."
        )]
    );
}

#[test]
fn escapes_and_entities_map_only_whole() {
    let source = "a \\* b &amp; c &#x1F600; d &bogus; e \\q f &x y\n";
    let arena = Arena::new();
    let root = parse_document(&arena, source, &options());
    let mut mapper = Mapper::new(source);
    let node = find(root, |value| matches!(value, NodeValue::Text(_)));
    let pieces = mapper.text(node, &literal(node)).expect("a range");
    assert_eq!(
        shown(source, &pieces),
        [
            (Kind::Verbatim, "a ".to_owned(), "a "),
            (Kind::Atomic, "*".to_owned(), "\\*"),
            (Kind::Verbatim, " b ".to_owned(), " b "),
            (Kind::Atomic, "&".to_owned(), "&amp;"),
            (Kind::Verbatim, " c ".to_owned(), " c "),
            (Kind::Atomic, "😀".to_owned(), "&#x1F600;"),
            (
                Kind::Verbatim,
                " d &bogus; e \\q f &x y".to_owned(),
                " d &bogus; e \\q f &x y"
            ),
        ]
    );
    // An entity seen before is decoded from the cache.
    assert_eq!(mapper.decode("&amp;").as_deref(), Some("&"));
    assert_eq!(mapper.decode("&bogus;"), None);
}

#[test]
fn text_that_starts_with_an_escape_has_no_empty_piece_before_it() {
    let source = "&amp;x \\*\n";
    let arena = Arena::new();
    let root = parse_document(&arena, source, &options());
    let mut mapper = Mapper::new(source);
    let node = find(root, |value| matches!(value, NodeValue::Text(_)));
    let pieces = mapper.text(node, &literal(node)).expect("a range");
    assert_eq!(
        shown(source, &pieces),
        [
            (Kind::Atomic, "&".to_owned(), "&amp;"),
            (Kind::Verbatim, "x ".to_owned(), "x "),
            (Kind::Atomic, "*".to_owned(), "\\*"),
        ]
    );
}

#[test]
fn an_entity_needs_a_name_and_its_semicolon_within_reach() {
    let mapper_source = format!("&; &a-b; &{};\n", "a".repeat(45));
    let mut mapper = Mapper::new(&mapper_source);
    let end = mapper_source.len();
    assert_eq!(mapper.entity_at(0, end), None);
    assert_eq!(mapper.entity_at(3, end), None);
    assert_eq!(mapper.entity_at(10, end), None);
    // The window ends at the text's end, even inside a character.
    let multibyte = "&é";
    let mut mapper = Mapper::new(multibyte);
    assert_eq!(mapper.entity_at(0, 2), None);
}

#[test]
fn text_that_does_not_line_up_is_one_atomic_piece() {
    let source = "abc def\n";
    let arena = Arena::new();
    let root = parse_document(&arena, source, &options());
    let mut mapper = Mapper::new(source);
    let node = text_node(root, "abc def");
    // A literal that differs from its source, and one longer than it.
    for wrong in ["abX def", "abc defg"] {
        let pieces = mapper.text(node, wrong).expect("a range");
        assert_eq!(
            shown(source, &pieces),
            [(Kind::Atomic, wrong.to_owned(), "abc def")]
        );
    }
    // An escape at the very end of the node is not read past its end.
    let escaped = "x \\*\n";
    let root = parse_document(&arena, escaped, &options());
    let node = find(root, |value| matches!(value, NodeValue::Text(_)));
    node.data_mut().sourcepos.end = position(1, 3);
    let pieces = Mapper::new(escaped).text(node, "x \\").expect("a range");
    assert_eq!(
        shown(escaped, &pieces),
        [(Kind::Verbatim, "x \\".to_owned(), "x \\")]
    );
}

#[test]
fn an_escape_is_read_only_inside_the_node_and_only_before_punctuation() {
    // The node ends at the backslash: the `*` after it belongs to no
    // escape here, so a literal that wants it does not line up.
    let source = "x \\*\n";
    let arena = Arena::new();
    let root = parse_document(&arena, source, &options());
    let node = find(root, |value| matches!(value, NodeValue::Text(_)));
    node.data_mut().sourcepos.end = position(1, 3);
    let pieces = Mapper::new(source).text(node, "x *").expect("a range");
    assert_eq!(
        shown(source, &pieces),
        [(Kind::Atomic, "x *".to_owned(), "x \\")]
    );
    // A backslash before a letter is no escape, even when the literal
    // skips it.
    let letter = "\\qz\n";
    let root = parse_document(&arena, letter, &options());
    let node = find(root, |value| matches!(value, NodeValue::Text(_)));
    let pieces = Mapper::new(letter).text(node, "qz").expect("a range");
    assert_eq!(
        shown(letter, &pieces),
        [(Kind::Atomic, "qz".to_owned(), "\\qz")]
    );
}

#[test]
fn nodes_with_bad_positions_give_no_pieces() {
    let source = "Some *text* and `code` and ![x](y.png).\n";
    let arena = Arena::new();
    let root = parse_document(&arena, source, &options());
    let mut mapper = Mapper::new(source);
    for node in root.descendants().skip(2) {
        node.data_mut().sourcepos.start = position(9, 1);
    }
    let text = text_node(root, "Some ");
    assert_eq!(mapper.text(text, "Some "), None);
    let code = find(root, |value| matches!(value, NodeValue::Code(_)));
    assert_eq!(mapper.code_span(code, "code"), None);
    let image = find(root, |value| matches!(value, NodeValue::Image(_)));
    assert_eq!(mapper.whole(image, Kind::Object, ""), None);
    let block = "```\ncode\n```\n";
    let root = parse_document(&arena, block, &options());
    let code = find(root, |value| matches!(value, NodeValue::CodeBlock(_)));
    code.data_mut().sourcepos.start = position(7, 1);
    assert_eq!(Mapper::new(block).code_block(code, "code\n", true), None);
}

#[test]
fn whole_nodes_map_as_one_piece() {
    let source = "x ![alt](a.png) y\n";
    let arena = Arena::new();
    let root = parse_document(&arena, source, &options());
    let mapper = Mapper::new(source);
    let image = find(root, |value| matches!(value, NodeValue::Image(_)));
    let piece = mapper.whole(image, Kind::Object, "").expect("a range");
    assert_eq!(&source[piece.source], "![alt](a.png)");
}

#[test]
fn code_spans_map_their_content() {
    let source = "a `` b`c `` d ` padded ` e `x\ny` f\n";
    let arena = Arena::new();
    let root = parse_document(&arena, source, &options());
    let mapper = Mapper::new(source);
    let spans: Vec<Node<'_>> = root
        .descendants()
        .filter(|node| matches!(node.data().value, NodeValue::Code(_)))
        .collect();
    let all: Vec<Vec<(Kind, String, &str)>> = spans
        .iter()
        .map(|span| {
            shown(
                source,
                &mapper.code_span(span, &literal(span)).expect("a range"),
            )
        })
        .collect();
    assert_eq!(
        all,
        [
            vec![(Kind::Verbatim, "b`c".to_owned(), "b`c")],
            vec![(Kind::Verbatim, "padded".to_owned(), "padded")],
            vec![
                (Kind::Verbatim, "x".to_owned(), "x"),
                (Kind::Verbatim, " ".to_owned(), "\n"),
                (Kind::Verbatim, "y".to_owned(), "y")
            ],
        ]
    );
}

#[test]
fn a_code_span_over_line_endings_skips_the_next_lines_markers() {
    let source = "> a `span\r\n> with` b\n\n- c `one\n  two` d\n\nx `k\r\nl` z\n";
    let arena = Arena::new();
    let root = parse_document(&arena, source, &options());
    let mapper = Mapper::new(source);
    let spans: Vec<Vec<(Kind, String, &str)>> = root
        .descendants()
        .filter(|node| matches!(node.data().value, NodeValue::Code(_)))
        .map(|span| {
            shown(
                source,
                &mapper.code_span(span, &literal(span)).expect("a range"),
            )
        })
        .collect();
    assert_eq!(
        spans,
        [
            vec![
                (Kind::Verbatim, "span".to_owned(), "span"),
                (Kind::Atomic, " ".to_owned(), "\r\n"),
                (Kind::Verbatim, "with".to_owned(), "with"),
            ],
            vec![
                (Kind::Verbatim, "one".to_owned(), "one"),
                (Kind::Verbatim, " ".to_owned(), "\n"),
                (Kind::Verbatim, "two".to_owned(), "two"),
            ],
            vec![
                (Kind::Verbatim, "k".to_owned(), "k"),
                (Kind::Atomic, " ".to_owned(), "\r\n"),
                (Kind::Verbatim, "l".to_owned(), "l"),
            ],
        ]
    );
}

#[test]
fn a_bare_carriage_return_in_a_code_span_is_one_byte() {
    let source = "a `x\ry` b\r";
    let arena = Arena::new();
    let root = parse_document(&arena, source, &options());
    let mapper = Mapper::new(source);
    let span = find(root, |value| matches!(value, NodeValue::Code(_)));
    let pieces = mapper.code_span(span, &literal(span)).expect("a range");
    assert_eq!(
        shown(source, &pieces),
        [
            (Kind::Verbatim, "x".to_owned(), "x"),
            (Kind::Verbatim, " ".to_owned(), "\r"),
            (Kind::Verbatim, "y".to_owned(), "y"),
        ]
    );
}

#[test]
fn a_pipe_in_a_code_span_outside_a_table_is_plain_text() {
    let source = "a `p|q` and `r\\|s` b\n";
    let arena = Arena::new();
    let root = parse_document(&arena, source, &options());
    let mapper = Mapper::new(source);
    let spans: Vec<Vec<(Kind, String, &str)>> = root
        .descendants()
        .filter(|node| matches!(node.data().value, NodeValue::Code(_)))
        .map(|span| {
            shown(
                source,
                &mapper.code_span(span, &literal(span)).expect("a range"),
            )
        })
        .collect();
    assert_eq!(
        spans,
        [
            vec![(Kind::Verbatim, "p|q".to_owned(), "p|q")],
            vec![(Kind::Verbatim, "r\\|s".to_owned(), "r\\|s")],
        ]
    );
}

#[test]
fn escaped_pipes_in_a_table_cell_code_span_map_whole() {
    let source = "| a |\n|---|\n| `p \\| q` |\n";
    let arena = Arena::new();
    let root = parse_document(&arena, source, &options());
    let mapper = Mapper::new(source);
    let span = find(root, |value| matches!(value, NodeValue::Code(_)));
    let pieces = mapper.code_span(span, &literal(span)).expect("a range");
    assert_eq!(
        shown(source, &pieces),
        [
            (Kind::Verbatim, "p ".to_owned(), "p "),
            (Kind::Atomic, "|".to_owned(), "\\|"),
            (Kind::Verbatim, " q".to_owned(), " q"),
        ]
    );
}

#[test]
fn a_code_span_that_does_not_line_up_is_one_atomic_piece() {
    let source = "a `code` b\n";
    let arena = Arena::new();
    let root = parse_document(&arena, source, &options());
    let mapper = Mapper::new(source);
    let span = find(root, |value| matches!(value, NodeValue::Code(_)));
    for wrong in ["coXe", "codes", " cod"] {
        let pieces = mapper.code_span(span, wrong).expect("a range");
        assert_eq!(
            shown(source, &pieces),
            [(Kind::Atomic, wrong.to_owned(), "`code`")],
            "{wrong:?}"
        );
    }
    // A padded span whose second try also fails.
    let padded = "a ` x ` b\n";
    let root = parse_document(&arena, padded, &options());
    let span = find(root, |value| matches!(value, NodeValue::Code(_)));
    let pieces = Mapper::new(padded).code_span(span, "yy").expect("a range");
    assert_eq!(
        shown(padded, &pieces),
        [(Kind::Atomic, "yy".to_owned(), "` x `")]
    );
    // An empty span.
    let empty = "a `` `` b\n";
    let root = parse_document(&arena, empty, &options());
    let span = find(root, |value| matches!(value, NodeValue::Code(_)));
    let pieces = Mapper::new(empty)
        .code_span(span, &literal(span))
        .expect("a range");
    let joined: String = pieces.iter().map(|piece| piece.text.as_str()).collect();
    assert_eq!(joined, literal(span));
}

#[test]
fn code_block_lines_are_the_ends_of_their_source_lines() {
    let source = "- item\n\n  ```sh\n  one\n\n  two\r\n  ```\n";
    let arena = Arena::new();
    let root = parse_document(&arena, source, &options());
    let mapper = Mapper::new(source);
    let block = find(root, |value| matches!(value, NodeValue::CodeBlock(_)));
    let pieces = mapper
        .code_block(block, &literal(block), true)
        .expect("lines");
    assert_eq!(
        shown(source, &pieces),
        [
            (Kind::Verbatim, "one\n\n".to_owned(), "one\n\n"),
            (Kind::Verbatim, "two".to_owned(), "two"),
            (Kind::Atomic, "\n".to_owned(), "\r\n"),
        ]
    );
}

#[test]
fn a_code_line_that_does_not_end_its_source_line_maps_whole() {
    let source = "```\nabc\n```\n";
    let arena = Arena::new();
    let root = parse_document(&arena, source, &options());
    let mapper = Mapper::new(source);
    let block = find(root, |value| matches!(value, NodeValue::CodeBlock(_)));
    let pieces = mapper.code_block(block, "xyz\nlast", true).expect("lines");
    assert_eq!(
        shown(source, &pieces),
        [
            (Kind::Atomic, "xyz".to_owned(), "abc"),
            (Kind::Verbatim, "\n".to_owned(), "\n"),
            (Kind::Atomic, "last".to_owned(), "```"),
        ]
    );
    // The last line of an unclosed block at the end of the file has no line
    // ending in the source.
    let unclosed = "```\ncode";
    let root = parse_document(&arena, unclosed, &options());
    let block = find(root, |value| matches!(value, NodeValue::CodeBlock(_)));
    let pieces = Mapper::new(unclosed)
        .code_block(block, &literal(block), true)
        .expect("lines");
    let texts: Vec<(Kind, String, &str)> = shown(unclosed, &pieces);
    assert_eq!(texts[0], (Kind::Verbatim, "code".to_owned(), "code"));
    assert!(texts.iter().all(|(_, _, source)| !source.contains('`')));
}

#[test]
fn merged_joins_only_touching_verbatim_pieces() {
    let pieces = vec![
        Piece::new(Kind::Verbatim, "a", 0..1),
        Piece::new(Kind::Verbatim, "b", 1..2),
        Piece::new(Kind::Verbatim, "c", 3..4),
        Piece::new(Kind::Atomic, "d", 4..6),
        Piece::new(Kind::Verbatim, "e", 6..7),
    ];
    let merged = merged(pieces);
    let texts: Vec<(&str, Range<usize>)> = merged
        .iter()
        .map(|piece| (piece.text.as_str(), piece.source.clone()))
        .collect();
    assert_eq!(texts, [("ab", 0..2), ("c", 3..4), ("d", 4..6), ("e", 6..7)]);
}
