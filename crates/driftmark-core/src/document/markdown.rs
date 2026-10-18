//! The comrak layer under the document model: the options driftmark parses
//! with, byte offsets for comrak's line and column positions, and the two
//! repairs for comrak 0.56 bugs that the anchoring spike found.
//!
//! The block list only needs block positions. The repairs for inline
//! positions live here anyway, so the note positions built later (M2) use the
//! same code as the blocks.

use std::borrow::Cow;
use std::ops::Range;

use comrak::Options;
use comrak::nodes::{AstNode, LineColumn, NodeValue};

/// The comrak options every part of driftmark parses with.
///
/// Blocks, pages and note positions must come from the same tree, so they all
/// take their options from here. The options turn on the GitHub extensions
/// agents use (tables, strikethrough, autolinks, task lists, footnotes and
/// alerts) and YAML front matter between `---` lines. comrak records the
/// source position of every node whatever the options say.
#[must_use]
pub fn options() -> Options<'static> {
    let mut options = Options::default();
    options.extension.table = true;
    options.extension.strikethrough = true;
    options.extension.autolink = true;
    options.extension.tasklist = true;
    options.extension.footnotes = true;
    options.extension.alerts = true;
    options.extension.front_matter_delimiter = Some("---".to_owned());
    options
}

/// Where each line of a text starts, so that comrak's line and column
/// positions can be turned into byte offsets.
///
/// comrak ends a line at a line feed, at a carriage return followed by a line
/// feed, and at a carriage return alone. This index counts lines the same way,
/// so old Mac files with bare carriage returns line up too.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    /// Indexes the line starts of `text`.
    #[must_use]
    pub fn new(text: &str) -> LineIndex {
        let bytes = text.as_bytes();
        let mut starts = vec![0];
        for (at, byte) in bytes.iter().enumerate() {
            let ends_line = match byte {
                b'\n' => true,
                b'\r' => bytes.get(at + 1) != Some(&b'\n'),
                _ => false,
            };
            if ends_line {
                starts.push(at + 1);
            }
        }
        LineIndex { starts }
    }

    /// The byte offset of a start position. comrak counts lines and columns
    /// from 1, and a column counts bytes. `None` when the position names a
    /// line the text does not have, or line or column 0.
    #[must_use]
    pub fn start_of(&self, position: LineColumn) -> Option<usize> {
        let line = *self.starts.get(position.line.checked_sub(1)?)?;
        Some(line + position.column.checked_sub(1)?)
    }

    /// The byte offset just after an end position. comrak's end column names
    /// the last byte of a node, so the range ends one byte later. Column 0
    /// gives the start of the line; comrak uses it for some list items that
    /// end with a line ending.
    #[must_use]
    pub fn end_of(&self, position: LineColumn) -> Option<usize> {
        let line = *self.starts.get(position.line.checked_sub(1)?)?;
        Some(line + position.column)
    }

    /// The 1-based number of the line that holds byte `offset`.
    #[must_use]
    pub fn line_of(&self, offset: usize) -> usize {
        self.starts.partition_point(|&start| start <= offset)
    }

    /// The byte where 1-based line `line` starts, if the text has that line.
    #[must_use]
    pub fn line_start(&self, line: usize) -> Option<usize> {
        self.starts.get(line.checked_sub(1)?).copied()
    }
}

/// Byte ranges of comrak nodes in the text they were parsed from.
#[derive(Debug, Clone)]
pub struct Positions<'t> {
    text: &'t str,
    lines: LineIndex,
}

impl<'t> Positions<'t> {
    /// Prepares positions for nodes parsed from `text`.
    #[must_use]
    pub fn new(text: &'t str) -> Positions<'t> {
        Positions {
            text,
            lines: LineIndex::new(text),
        }
    }

    /// The line index of the text.
    #[must_use]
    pub fn lines(&self) -> &LineIndex {
        &self.lines
    }

    /// The source bytes a node came from, end exclusive.
    ///
    /// `None` when comrak's position falls outside the text or inside a
    /// character. For an inline node inside a table cell the range is
    /// repaired first. comrak removes the backslash of each `\|` in a cell
    /// before it reads the cell's inlines, then reports their positions as if
    /// no byte was removed, so each escaped pipe moves later inlines one byte
    /// too early. Block nodes, cells included, have correct positions.
    #[must_use]
    pub fn range<'a>(&self, node: &'a AstNode<'a>) -> Option<Range<usize>> {
        let data = node.data();
        let start = self.lines.start_of(data.sourcepos.start)?;
        let end = self.lines.end_of(data.sourcepos.end)?;
        let cell = if data.value.block() {
            None
        } else {
            node.ancestors()
                .skip(1)
                .find(|ancestor| matches!(ancestor.data().value, NodeValue::TableCell))
        };
        match cell {
            Some(cell) => {
                let shift = self.escaped_pipes(cell)?;
                self.check(shift.apply(start), shift.apply(end))
            }
            None => self.check(start, end),
        }
    }

    /// `start..end` when it is a valid range of whole characters in the text.
    #[must_use]
    pub fn check(&self, start: usize, end: usize) -> Option<Range<usize>> {
        let whole =
            start <= end && self.text.is_char_boundary(start) && self.text.is_char_boundary(end);
        whole.then_some(start..end)
    }

    /// The escaped pipes of a table cell, as comrak's shifted positions see
    /// them.
    fn escaped_pipes<'a>(&self, cell: &'a AstNode<'a>) -> Option<PipeShift> {
        let data = cell.data();
        let start = self.lines.start_of(data.sourcepos.start)?;
        let end = self.lines.end_of(data.sourcepos.end)?.min(self.text.len());
        // A pair starting at a pipe can never be `\|`, so looking at every
        // pair finds the same pipes as stepping over each one found.
        let cell_bytes = self.text.as_bytes().get(start..end).unwrap_or_default();
        let pipes = cell_bytes
            .windows(2)
            .enumerate()
            .filter(|(_, pair)| *pair == b"\\|")
            .enumerate()
            // Where the pipe sits once the backslashes before it are gone.
            .map(|(earlier, (at, _))| start + at - earlier)
            .collect();
        Some(PipeShift { pipes })
    }
}

/// How far comrak's inline positions in one table cell are off.
struct PipeShift {
    pipes: Vec<usize>,
}

impl PipeShift {
    /// The true offset of a position comrak reported: one byte later for
    /// every escaped pipe before it.
    fn apply(&self, offset: usize) -> usize {
        offset + self.pipes.iter().filter(|&&pipe| pipe < offset).count()
    }
}

/// The text of a code span, with one comrak 0.56 bug repaired.
///
/// Markdown turns a line ending inside a code span into one space. comrak
/// does that for a line feed but keeps the carriage return of a Windows line
/// ending, so `` `a\r\nb` `` comes out as `"a\r b"` instead of `"a b"`. A bare
/// carriage return is handled correctly, so `"\r "` only ever comes from a
/// Windows line ending.
#[must_use]
pub fn code_span_text(literal: &str) -> Cow<'_, str> {
    if literal.contains("\r ") {
        Cow::Owned(literal.replace("\r ", " "))
    } else {
        Cow::Borrowed(literal)
    }
}

/// Writes the rendered text of the inline content under `node` into `out`.
///
/// Text, code spans and the alt text of images count. Markup, raw HTML tags
/// and footnote numbers do not. A soft line break becomes `soft_break`, a hard
/// one a line feed. The walk keeps its own stack, so deeply nested emphasis
/// cannot overflow the thread's stack.
pub(crate) fn push_inline_text<'a>(node: &'a AstNode<'a>, out: &mut String, soft_break: char) {
    for edge in node.traverse() {
        if let comrak::arena_tree::NodeEdge::Start(inner) = edge {
            match &inner.data().value {
                NodeValue::Text(text) => out.push_str(text),
                NodeValue::Code(code) => out.push_str(&code_span_text(&code.literal)),
                NodeValue::SoftBreak => out.push(soft_break),
                NodeValue::LineBreak => out.push('\n'),
                _ => {}
            }
        }
    }
}

/// The rendered text of the inline content under `node`, soft line breaks as
/// spaces.
pub(crate) fn inline_text<'a>(node: &'a AstNode<'a>) -> String {
    let mut out = String::new();
    push_inline_text(node, &mut out, ' ');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use comrak::{Arena, parse_document};

    fn position(line: usize, column: usize) -> LineColumn {
        LineColumn { line, column }
    }

    #[test]
    fn options_turn_on_the_github_extensions_and_front_matter() {
        let options = options();
        assert!(options.extension.table);
        assert!(options.extension.strikethrough);
        assert!(options.extension.autolink);
        assert!(options.extension.tasklist);
        assert!(options.extension.footnotes);
        assert!(options.extension.alerts);
        assert_eq!(
            options.extension.front_matter_delimiter.as_deref(),
            Some("---")
        );
    }

    #[test]
    fn lines_end_at_line_feeds_windows_endings_and_bare_carriage_returns() {
        let lines = LineIndex::new("a\nb\r\nc\rd");
        assert_eq!(lines.line_start(1), Some(0));
        assert_eq!(lines.line_start(2), Some(2));
        assert_eq!(lines.line_start(3), Some(5));
        assert_eq!(lines.line_start(4), Some(7));
        assert_eq!(lines.line_start(5), None);
        assert_eq!(lines.line_start(0), None);
    }

    #[test]
    fn a_text_ending_in_a_line_ending_has_an_empty_last_line() {
        let lines = LineIndex::new("a\n");
        assert_eq!(lines.line_start(2), Some(2));
    }

    #[test]
    fn positions_become_byte_offsets() {
        let lines = LineIndex::new("ab\ncde");
        assert_eq!(lines.start_of(position(2, 2)), Some(4));
        assert_eq!(lines.end_of(position(2, 2)), Some(5));
        assert_eq!(lines.end_of(position(2, 0)), Some(3));
        assert_eq!(lines.start_of(position(2, 0)), None);
        assert_eq!(lines.start_of(position(0, 1)), None);
        assert_eq!(lines.end_of(position(0, 1)), None);
        assert_eq!(lines.start_of(position(3, 1)), None);
        assert_eq!(lines.end_of(position(3, 1)), None);
    }

    #[test]
    fn offsets_become_line_numbers() {
        let lines = LineIndex::new("ab\ncd\n");
        assert_eq!(lines.line_of(0), 1);
        assert_eq!(lines.line_of(2), 1);
        assert_eq!(lines.line_of(3), 2);
        assert_eq!(lines.line_of(6), 3);
    }

    #[test]
    fn check_accepts_whole_characters_only() {
        let positions = Positions::new("aé");
        assert_eq!(positions.check(0, 3), Some(0..3));
        assert_eq!(positions.check(1, 1), Some(1..1));
        assert_eq!(positions.check(1, 2), None);
        assert_eq!(positions.check(2, 3), None);
        assert_eq!(positions.check(2, 1), None);
        assert_eq!(positions.check(0, 4), None);
    }

    fn find<'a>(root: &'a AstNode<'a>, wanted: &str) -> &'a AstNode<'a> {
        root.descendants()
            .find(|node| matches!(&node.data().value, NodeValue::Text(text) if text == wanted))
            .expect("the test text has this node")
    }

    #[test]
    fn inline_positions_after_escaped_pipes_are_repaired() {
        let source = "| a \\| b | c \\| d \\| e |\n|---|---|\n| x | `p \\| q` z |\n";
        let arena = Arena::new();
        let root = parse_document(&arena, source, &options());
        let positions = Positions::new(source);
        let first = positions.range(find(root, "a | b")).expect("a range");
        assert_eq!(&source[first], "a \\| b");
        let second = positions.range(find(root, "c | d | e")).expect("a range");
        assert_eq!(&source[second], "c \\| d \\| e");
        let after_code = positions.range(find(root, " z")).expect("a range");
        assert_eq!(&source[after_code], " z");
        let code = root
            .descendants()
            .find(|node| matches!(node.data().value, NodeValue::Code(_)))
            .expect("a code span");
        let code_range = positions.range(code).expect("a range");
        assert_eq!(&source[code_range], "`p \\| q`");
    }

    #[test]
    fn the_repair_counts_only_escaped_pipes_in_the_cell() {
        let source = "| h |\n|---|\n| \\* x \\| *y* |\n| a\\|b\\|*c* |\n";
        let arena = Arena::new();
        let root = parse_document(&arena, source, &options());
        let positions = Positions::new(source);
        let after_escape = positions.range(find(root, "y")).expect("a range");
        assert_eq!(&source[after_escape], "y");
        let after_two = positions.range(find(root, "c")).expect("a range");
        assert_eq!(&source[after_two], "c");
    }

    #[test]
    fn a_cell_ending_in_a_backslash_at_the_end_of_the_text_is_read_safely() {
        let source = "a | b\n--|--\nc | d\\";
        let arena = Arena::new();
        let root = parse_document(&arena, source, &options());
        let range = Positions::new(source)
            .range(find(root, "d\\"))
            .expect("a range");
        assert_eq!(&source[range], "d\\");
    }

    #[test]
    fn block_positions_need_no_repair() {
        let source = "| a \\| b | c |\n|---|---|\n";
        let arena = Arena::new();
        let root = parse_document(&arena, source, &options());
        let positions = Positions::new(source);
        let cell = root
            .descendants()
            .find(|node| matches!(node.data().value, NodeValue::TableCell))
            .expect("a cell");
        let range = positions.range(cell).expect("a range");
        assert_eq!(&source[range], " a \\| b ");
    }

    #[test]
    fn a_text_with_no_escaped_pipe_keeps_its_positions() {
        let source = "| a | b |\n|---|---|\n";
        let arena = Arena::new();
        let root = parse_document(&arena, source, &options());
        let positions = Positions::new(source);
        let range = positions.range(find(root, "b")).expect("a range");
        assert_eq!(&source[range], "b");
        assert_eq!(positions.lines().line_start(2), Some(10));
    }

    #[test]
    fn a_pipe_shift_counts_only_pipes_before_the_offset() {
        let shift = PipeShift { pipes: vec![3, 7] };
        assert_eq!(shift.apply(3), 3);
        assert_eq!(shift.apply(4), 5);
        assert_eq!(shift.apply(8), 10);
    }

    #[test]
    fn positions_outside_the_text_give_no_range() {
        let source = "a | b\n--|--\nc | d\n";
        let arena = Arena::new();
        let root = parse_document(&arena, source, &options());
        let text = find(root, "d");
        text.data_mut().sourcepos.start = position(9, 1);
        let positions = Positions::new(source);
        assert_eq!(positions.range(text), None);
        text.data_mut().sourcepos.start = position(3, 5);
        text.data_mut().sourcepos.end = position(9, 1);
        assert_eq!(positions.range(text), None);
        let cell = text.parent().expect("a cell");
        cell.data_mut().sourcepos.start = position(9, 1);
        text.data_mut().sourcepos.end = position(3, 5);
        assert_eq!(positions.range(text), None);
        cell.data_mut().sourcepos.start = position(3, 1);
        cell.data_mut().sourcepos.end = position(9, 1);
        assert_eq!(positions.range(text), None);
        let paragraph_source = "x\n";
        let paragraph_root = parse_document(&arena, paragraph_source, &options());
        let paragraph = paragraph_root.first_child().expect("a paragraph");
        paragraph.data_mut().sourcepos.end = position(7, 1);
        assert_eq!(Positions::new(paragraph_source).range(paragraph), None);
        paragraph.data_mut().sourcepos.start = position(0, 1);
        assert_eq!(Positions::new(paragraph_source).range(paragraph), None);
    }

    #[test]
    fn a_cell_that_ends_past_the_text_is_read_to_the_end() {
        let source = "a | b\n--|--\nc | \\|d";
        let arena = Arena::new();
        let root = parse_document(&arena, source, &options());
        let text = find(root, "|d");
        let cell = text.parent().expect("a cell");
        cell.data_mut().sourcepos.end = position(3, 40);
        let range = Positions::new(source).range(text).expect("a range");
        assert_eq!(&source[range], "\\|d");
    }

    #[test]
    fn code_spans_across_windows_line_endings_get_one_space() {
        assert_eq!(code_span_text("a\r b"), "a b");
        assert!(matches!(code_span_text("a b"), Cow::Borrowed("a b")));
    }

    #[test]
    fn comrak_still_keeps_the_carriage_return() {
        // If this starts failing, comrak fixed the bug and the repair can go.
        let arena = Arena::new();
        let root = parse_document(&arena, "`span\r\nwith`\r\n", &options());
        let code = root
            .descendants()
            .find_map(|node| match &node.data().value {
                NodeValue::Code(code) => Some(code.literal.clone()),
                _ => None,
            })
            .expect("a code span");
        assert_eq!(code, "span\r with");
    }

    #[test]
    fn inline_text_skips_markup_and_keeps_alt_text() {
        let source = "A *b* `c\r\nd` <kbd>e</kbd> ![f g](x.png) h[^1]\\\ni\nj\n\n[^1]: note\n";
        let arena = Arena::new();
        let root = parse_document(&arena, source, &options());
        let paragraph = root.first_child().expect("a paragraph");
        assert_eq!(inline_text(paragraph), "A b c d e f g h\ni j");
        let mut lines = String::new();
        push_inline_text(paragraph, &mut lines, '\n');
        assert_eq!(lines, "A b c d e f g h\ni\nj");
    }
}
