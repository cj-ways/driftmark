//! The two block hashes, and the text clean-ups they and the plain text share.
//!
//! The content hash covers a canonical form of the block. That form keeps
//! everything a reader would call content and drops formatting: emphasis
//! markers, code-span backticks, line wrapping, extra spaces, table padding,
//! list marker style, heading style, fence style and trailing spaces. Equal
//! content hashes with different source hashes therefore mean "formatting
//! only". `docs/features/engine/document-model.md` lists the rules.

use std::borrow::Cow;

use comrak::arena_tree::NodeEdge;
use comrak::nodes::{AstNode, NodeValue};
use twox_hash::XxHash3_64;

use super::markdown::code_span_text;
use super::{SpanKind, TextSpan};

/// Starts a mark in the canonical form. UTF-8 text never holds this byte, so
/// a mark can never be mistaken for text.
const MARK: u8 = 0xFF;
/// Ends a mark. UTF-8 text never holds this byte either.
const MARK_END: u8 = 0xFE;

/// Tags of the marks in the canonical form. Each is one ASCII letter.
pub(crate) mod tag {
    pub(crate) const FRONT_MATTER: u8 = b'F';
    pub(crate) const HEADING: u8 = b'H';
    pub(crate) const PARAGRAPH: u8 = b'P';
    pub(crate) const LIST_ITEM: u8 = b'I';
    pub(crate) const TABLE_ROW: u8 = b'R';
    pub(crate) const CODE: u8 = b'C';
    pub(crate) const DIAGRAM: u8 = b'D';
    pub(crate) const ALERT: u8 = b'A';
    pub(crate) const FOOTNOTE_DEFINITION: u8 = b'N';
    pub(crate) const HTML: u8 = b'M';
    pub(crate) const THEMATIC_BREAK: u8 = b'T';
    pub(crate) const UNRENDERED: u8 = b'U';
    pub(crate) const BODY: u8 = b'b';
    pub(crate) const CELL: u8 = b'c';
    pub(crate) const NEXT_PARAGRAPH: u8 = b'p';
    pub(crate) const LINK: u8 = b'l';
    pub(crate) const IMAGE: u8 = b'i';
    pub(crate) const TARGET: u8 = b'u';
    pub(crate) const TARGET_TITLE: u8 = b'v';
    pub(crate) const STRIKE: u8 = b's';
    pub(crate) const STRIKE_END: u8 = b'e';
    pub(crate) const INLINE_HTML: u8 = b'h';
    pub(crate) const FOOTNOTE_REFERENCE: u8 = b'f';
}

/// The hash driftmark uses for blocks: 64-bit XXH3 with seed 0.
///
/// XXH3 has a published specification, so the value is the same on every
/// system and in every version of driftmark that keeps this function. The
/// standard library's hasher promises neither.
#[must_use]
pub(crate) fn hash(bytes: &[u8]) -> u64 {
    XxHash3_64::oneshot(bytes)
}

/// Builds the canonical form of one block and hashes it.
pub(crate) struct Canon {
    bytes: Vec<u8>,
    /// True until something follows the last boundary, so white space at the
    /// start of a block or a cell is dropped.
    at_start: bool,
    /// White space seen since the last word, written only if a word follows.
    pending_space: bool,
}

impl Canon {
    /// Starts the canonical form of a block of one kind. `details` holds the
    /// parts of the kind that count as content, such as a heading's level.
    /// `size` is the block's source length, to size the buffer once.
    pub(crate) fn new(kind: u8, details: &str, size: usize) -> Canon {
        let mut canon = Canon {
            bytes: Vec::with_capacity(size),
            at_start: true,
            pending_space: false,
        };
        canon.write_mark(kind, details);
        canon
    }

    /// Adds text. Every run of white space becomes one space, and white space
    /// at the start or end of the block is dropped.
    pub(crate) fn text(&mut self, text: &str) {
        let mut rest = text;
        while !rest.is_empty() {
            let word = rest.trim_start();
            if word.len() < rest.len() {
                self.space();
            }
            let end = word.find(char::is_whitespace).unwrap_or(word.len());
            if end > 0 {
                self.flush_space();
                self.bytes.extend_from_slice(&word.as_bytes()[..end]);
                self.at_start = false;
            }
            rest = &word[end..];
        }
    }

    /// Adds white space, which only counts once something follows it.
    pub(crate) fn space(&mut self) {
        if !self.at_start {
            self.pending_space = true;
        }
    }

    fn flush_space(&mut self) {
        if self.pending_space {
            self.bytes.push(b' ');
            self.pending_space = false;
        }
    }

    fn write_mark(&mut self, tag: u8, payload: &str) {
        self.bytes.push(MARK);
        self.bytes.push(tag);
        self.bytes.extend_from_slice(payload.as_bytes());
        self.bytes.push(MARK_END);
    }

    /// Adds a mark inside running text, such as the target of a link. White
    /// space before it counts, because `a[b](c)` and `a [b](c)` read
    /// differently.
    pub(crate) fn mark(&mut self, tag: u8, payload: &str) {
        self.flush_space();
        self.write_mark(tag, payload);
        self.at_start = false;
    }

    /// Adds a mark between two parts of a block, such as between two table
    /// cells. White space on either side of it does not count.
    pub(crate) fn boundary(&mut self, tag: u8) {
        self.pending_space = false;
        self.write_mark(tag, "");
        self.at_start = true;
    }

    /// Adds the inline content under `node`: its words, the targets of its
    /// links and images, struck-out text, raw HTML and footnote references.
    /// Emphasis, strong emphasis and code-span backticks are formatting and
    /// leave no trace.
    ///
    /// The same walk writes the block's plain text to `text`, by the rules of
    /// [`super::markdown::push_inline_text`] with soft breaks as spaces, and
    /// adds a [`TextSpan`] to `spans` for each link, image and code span: one
    /// walk instead of three, because parsing speed matters (decision 0008).
    /// The walk keeps its own stack.
    pub(crate) fn inline<'a>(
        &mut self,
        node: &'a AstNode<'a>,
        text: &mut String,
        spans: &mut Vec<TextSpan>,
    ) {
        // Where the text of each link or image still open starts.
        let mut open = Vec::new();
        for edge in node.traverse() {
            match edge {
                NodeEdge::Start(inner) => match &inner.data().value {
                    NodeValue::Text(literal) => {
                        self.text(literal);
                        text.push_str(literal);
                    }
                    NodeValue::Code(code) => {
                        let literal = code_span_text(&code.literal);
                        self.text(&literal);
                        let start = text.len();
                        text.push_str(&literal);
                        spans.push(TextSpan {
                            range: start..text.len(),
                            kind: SpanKind::Code,
                        });
                    }
                    NodeValue::SoftBreak => {
                        self.space();
                        text.push(' ');
                    }
                    NodeValue::LineBreak => {
                        self.space();
                        text.push('\n');
                    }
                    NodeValue::Link(_) => {
                        self.mark(tag::LINK, "");
                        open.push(text.len());
                    }
                    NodeValue::Image(_) => {
                        self.mark(tag::IMAGE, "");
                        open.push(text.len());
                    }
                    NodeValue::Strikethrough => self.mark(tag::STRIKE, ""),
                    NodeValue::HtmlInline(html) => self.mark(tag::INLINE_HTML, html),
                    NodeValue::FootnoteReference(reference) => {
                        self.mark(tag::FOOTNOTE_REFERENCE, &reference.name);
                    }
                    _ => {}
                },
                NodeEdge::End(inner) => match &inner.data().value {
                    NodeValue::Link(link) => {
                        self.target(&link.url, &link.title);
                        let kind = SpanKind::Link {
                            target: link.url.clone(),
                        };
                        close_span(&mut open, text, spans, kind);
                    }
                    NodeValue::Image(link) => {
                        self.target(&link.url, &link.title);
                        let kind = SpanKind::Image {
                            target: link.url.clone(),
                        };
                        close_span(&mut open, text, spans, kind);
                    }
                    NodeValue::Strikethrough => self.mark(tag::STRIKE_END, ""),
                    _ => {}
                },
            }
        }
    }

    /// Adds the target and title of a link or an image.
    fn target(&mut self, url: &str, title: &str) {
        self.mark(tag::TARGET, url);
        self.mark(tag::TARGET_TITLE, title);
    }

    /// The content hash.
    pub(crate) fn finish(&self) -> u64 {
        hash(&self.bytes)
    }
}

/// Ends the span of the innermost open link or image at the end of `text`.
/// comrak ends every node it starts, so an open start is always there.
fn close_span(open: &mut Vec<usize>, text: &str, spans: &mut Vec<TextSpan>, kind: SpanKind) {
    if let Some(start) = open.pop() {
        spans.push(TextSpan {
            range: start..text.len(),
            kind,
        });
    }
}

/// Turns Windows line endings and bare carriage returns into line feeds.
#[must_use]
pub(crate) fn unify_line_endings(text: &str) -> Cow<'_, str> {
    if text.contains('\r') {
        Cow::Owned(text.replace("\r\n", "\n").replace('\r', "\n"))
    } else {
        Cow::Borrowed(text)
    }
}

/// Text whose line breaks matter, such as code, in the shape the content
/// hash compares: line feeds only, no spaces at line ends, and no blank lines
/// at the start or end.
#[must_use]
pub(crate) fn normalize_lines(text: &str) -> String {
    let unified = unify_line_endings(text);
    let lines: Vec<&str> = unified.split('\n').map(str::trim_end).collect();
    let first = lines.iter().position(|line| !line.is_empty());
    let last = lines.iter().rposition(|line| !line.is_empty());
    match (first, last) {
        (Some(first), Some(last)) => lines[first..=last].join("\n"),
        _ => String::new(),
    }
}

/// The YAML between the two `---` lines of front matter, given the front
/// matter's source from the opening line to the closing one.
#[must_use]
pub(crate) fn front_matter_body(source: &str) -> &str {
    let after_opening = source.split_once('\n').map_or("", |(_, rest)| rest);
    let trimmed = after_opening.trim_end();
    let before_closing = trimmed.rfind('\n').map_or("", |at| &trimmed[..at]);
    before_closing.trim_end_matches('\r')
}

/// The text of an HTML block as a reader sees it: tags and comments removed,
/// the rest kept as written.
#[must_use]
pub(crate) fn strip_html(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    while let Some(at) = rest.find('<') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let skip = if tail.starts_with("<!--") {
            tail.find("-->").map_or(tail.len(), |end| end + 3)
        } else if tail[1..].starts_with(|c: char| c.is_ascii_alphabetic() || "/!?".contains(c)) {
            tail.find('>').map_or(tail.len(), |end| end + 1)
        } else {
            out.push('<');
            1
        };
        rest = &tail[skip..];
    }
    out.push_str(rest);
    unify_line_endings(out.trim()).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::markdown::options;
    use comrak::{Arena, parse_document};

    fn inline_hash(markdown: &str) -> u64 {
        let arena = Arena::new();
        let root = parse_document(&arena, markdown, &options());
        let mut canon = Canon::new(tag::PARAGRAPH, "", 0);
        let mut text = String::new();
        let mut spans = Vec::new();
        canon.inline(
            root.first_child().expect("a paragraph"),
            &mut text,
            &mut spans,
        );
        canon.finish()
    }

    #[test]
    fn the_inline_walk_writes_the_plain_text_too() {
        let arena = Arena::new();
        let root = parse_document(
            &arena,
            "A *b* `c\r\nd` <kbd>e</kbd> ![f g](x.png) h[^1]\\\ni\nj\n\n[^1]: note\n",
            &options(),
        );
        let paragraph = root.first_child().expect("a paragraph");
        let mut text = String::new();
        let mut spans = Vec::new();
        Canon::new(tag::PARAGRAPH, "", 0).inline(paragraph, &mut text, &mut spans);
        assert_eq!(text, crate::document::markdown::inline_text(paragraph));
        assert_eq!(text, "A b c d e f g h\ni j");
        assert_eq!(
            spans,
            [
                TextSpan {
                    range: 4..7,
                    kind: SpanKind::Code
                },
                TextSpan {
                    range: 10..13,
                    kind: SpanKind::Image {
                        target: "x.png".to_owned()
                    }
                },
            ]
        );
    }

    #[test]
    fn the_inline_walk_records_links_images_and_code_spans() {
        let arena = Arena::new();
        let root = parse_document(
            &arena,
            "See [the `x` guide](a.md), [![logo](l.png)](b.md), [](c.md) and <https://e.org>.\n",
            &options(),
        );
        let paragraph = root.first_child().expect("a paragraph");
        let mut text = String::new();
        let mut spans = Vec::new();
        Canon::new(tag::PARAGRAPH, "", 0).inline(paragraph, &mut text, &mut spans);
        assert_eq!(text, "See the x guide, logo,  and https://e.org.");
        let found: Vec<(&str, &SpanKind)> = spans
            .iter()
            .map(|span| (&text[span.range.clone()], &span.kind))
            .collect();
        let link = |target: &str| SpanKind::Link {
            target: target.to_owned(),
        };
        let image = SpanKind::Image {
            target: "l.png".to_owned(),
        };
        assert_eq!(
            found,
            [
                ("x", &SpanKind::Code),
                ("the x guide", &link("a.md")),
                ("logo", &image),
                ("logo", &link("b.md")),
                ("", &link("c.md")),
                ("https://e.org", &link("https://e.org")),
            ]
        );
    }

    #[test]
    fn a_span_closes_only_when_one_is_open() {
        let mut open = vec![2];
        let mut spans = Vec::new();
        close_span(&mut open, "abcd", &mut spans, SpanKind::Code);
        assert_eq!(
            spans,
            [TextSpan {
                range: 2..4,
                kind: SpanKind::Code
            }]
        );
        close_span(&mut open, "abcd", &mut spans, SpanKind::Code);
        assert_eq!(spans.len(), 1);
    }

    #[test]
    fn the_hash_is_xxh3_with_seed_zero() {
        // Published test value of XXH3-64 for an empty input.
        assert_eq!(hash(b""), 0x2D06_8005_38D3_94C2);
        assert_ne!(hash(b"a"), hash(b"b"));
    }

    #[test]
    fn white_space_collapses_and_is_trimmed() {
        let mut spaced = Canon::new(tag::PARAGRAPH, "", 0);
        spaced.text("  a \n\t b  ");
        let mut tight = Canon::new(tag::PARAGRAPH, "", 0);
        tight.text("a b");
        assert_eq!(spaced.bytes, tight.bytes);
        let mut joined = Canon::new(tag::PARAGRAPH, "", 0);
        joined.text("ab");
        assert_ne!(joined.bytes, tight.bytes);
    }

    #[test]
    fn a_mark_keeps_the_space_before_it_and_a_boundary_drops_it() {
        let mut spaced = Canon::new(tag::PARAGRAPH, "", 0);
        spaced.text("a ");
        spaced.mark(tag::LINK, "");
        let mut tight = Canon::new(tag::PARAGRAPH, "", 0);
        tight.text("a");
        tight.mark(tag::LINK, "");
        assert_ne!(spaced.bytes, tight.bytes);
        let mut cell_spaced = Canon::new(tag::TABLE_ROW, "", 0);
        cell_spaced.text("a ");
        cell_spaced.boundary(tag::CELL);
        let mut cell_tight = Canon::new(tag::TABLE_ROW, "", 0);
        cell_tight.text("a");
        cell_tight.boundary(tag::CELL);
        cell_spaced.text(" b");
        cell_tight.text("b");
        assert_eq!(cell_spaced.bytes, cell_tight.bytes);
    }

    #[test]
    fn marks_are_framed_so_payloads_cannot_run_together() {
        let mut first = Canon::new(tag::CODE, "ab", 0);
        first.mark(tag::BODY, "c");
        let mut second = Canon::new(tag::CODE, "a", 0);
        second.mark(tag::BODY, "bc");
        assert_ne!(first.finish(), second.finish());
        assert_eq!(first.bytes[0], MARK);
        assert_eq!(first.bytes[4], MARK_END);
    }

    #[test]
    fn emphasis_style_wrapping_and_code_backticks_are_formatting() {
        let plain = inline_hash("a bold word and code here\n");
        assert_eq!(plain, inline_hash("a **bold** word\nand `code` here\n"));
        assert_eq!(plain, inline_hash("a __bold__ word and  code here\n"));
        assert_eq!(plain, inline_hash("a bold word and code\\\nhere\n"));
        assert_eq!(inline_hash("`a\r\nb`\n"), inline_hash("`a b`\n"));
    }

    #[test]
    fn links_images_strikes_html_and_footnotes_are_content() {
        let plain = inline_hash("see the guide\n");
        let linked = inline_hash("see the [guide](a.md)\n");
        assert_ne!(plain, linked);
        assert_ne!(linked, inline_hash("see the [guide](b.md)\n"));
        assert_ne!(linked, inline_hash("see the [guide](a.md \"Title\")\n"));
        assert_ne!(linked, inline_hash("see [the guide](a.md)\n"));
        assert_ne!(plain, inline_hash("see the ![guide](a.png)\n"));
        assert_ne!(plain, inline_hash("see the ~~guide~~\n"));
        assert_ne!(
            inline_hash("see ~~the~~ guide\n"),
            inline_hash("see ~~the guide~~\n")
        );
        assert_ne!(inline_hash("~~a b~~\n"), inline_hash("a ~~b~~\n"));
        assert_ne!(
            inline_hash("![big cat](c.png)\n"),
            inline_hash("big ![cat](c.png)\n")
        );
        assert_ne!(plain, inline_hash("see the <b>guide\n"));
        assert_ne!(
            inline_hash("see the guide[^a]\n\n[^a]: x\n"),
            inline_hash("see the guide[^b]\n\n[^b]: x\n")
        );
        assert_eq!(
            inline_hash("see <https://example.com>\n"),
            inline_hash("see https://example.com\n")
        );
    }

    #[test]
    fn line_endings_become_line_feeds() {
        assert_eq!(unify_line_endings("a\r\nb\rc\n"), "a\nb\nc\n");
        assert!(matches!(unify_line_endings("a\nb"), Cow::Borrowed("a\nb")));
    }

    #[test]
    fn normalized_lines_drop_trailing_spaces_and_outer_blank_lines() {
        assert_eq!(normalize_lines("\n  \n  a  \r\n\r\n b\t\n\n"), "  a\n\n b");
        assert_eq!(normalize_lines("x"), "x");
        assert_eq!(normalize_lines(" \n\t\n"), "");
        assert_eq!(normalize_lines(""), "");
    }

    #[test]
    fn front_matter_body_is_the_text_between_the_fences() {
        assert_eq!(front_matter_body("---\na: 1\nb: 2\n---"), "a: 1\nb: 2");
        assert_eq!(front_matter_body("---\r\na: 1\r\n---"), "a: 1");
        assert_eq!(front_matter_body("---\na: 1\n---\n\n"), "a: 1");
        assert_eq!(front_matter_body("---\n---"), "");
        assert_eq!(front_matter_body("---"), "");
    }

    #[test]
    fn html_text_drops_tags_and_comments() {
        assert_eq!(
            strip_html("<details>\r\n<summary>More &amp; less</summary>\n"),
            "More &amp; less"
        );
        assert_eq!(strip_html("<!-- note -->a < b<br/>c"), "a < bc");
        assert_eq!(strip_html("<!-- open"), "");
        assert_eq!(strip_html("x <div"), "x");
        assert_eq!(strip_html("<?php x ?>y<!DOCTYPE html>z</p>"), "yz");
        assert_eq!(strip_html("<"), "<");
    }
}
