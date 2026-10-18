//! Where each stretch of rendered text came from in the source.
//!
//! A piece ties rendered text to a byte range of the Markdown source. The
//! position tables for notes are built from pieces, and the page script uses
//! them to turn a selection into source bytes and back. The method comes from
//! the anchoring spike (`docs/working/version-1/spikes/anchoring.md`).

use std::collections::HashMap;
use std::ops::Range;

use comrak::nodes::{AstNode, NodeValue};
use comrak::{Arena, Options, parse_document};

use crate::document::markdown::{Positions, code_span_text};

/// How a piece of rendered text relates to its source bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// The rendered text is the source bytes, character for character, so
    /// any offset inside maps exactly.
    Verbatim,
    /// The rendered text came from source of another shape: an entity, an
    /// escape, a footnote number, an image shown as its alt text. It maps
    /// only as a whole.
    Atomic,
    /// An element with no text, an image. It maps as a whole.
    Object,
}

/// One stretch of rendered text and the source bytes it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Piece {
    /// How the text relates to the source.
    pub(crate) kind: Kind,
    /// The rendered text; empty for an object.
    pub(crate) text: String,
    /// The source bytes, end exclusive.
    pub(crate) source: Range<usize>,
}

impl Piece {
    fn new(kind: Kind, text: impl Into<String>, source: Range<usize>) -> Piece {
        Piece {
            kind,
            text: text.into(),
            source,
        }
    }
}

/// Builds pieces for the nodes of one parsed document.
pub(crate) struct Mapper<'s> {
    source: &'s str,
    positions: Positions<'s>,
    /// Entities seen so far, with the text they stand for.
    entities: HashMap<String, Option<String>>,
}

type Node<'a> = &'a AstNode<'a>;

impl<'s> Mapper<'s> {
    /// A mapper for nodes parsed from `source`.
    pub(crate) fn new(source: &'s str) -> Mapper<'s> {
        Mapper {
            source,
            positions: Positions::new(source),
            entities: HashMap::new(),
        }
    }

    /// The source range of a node, repaired for escaped pipes in table
    /// cells.
    pub(crate) fn range(&self, node: Node<'_>) -> Option<Range<usize>> {
        self.positions.range(node)
    }

    /// One piece that covers a whole node: an image, a footnote number, raw
    /// HTML shown as text.
    pub(crate) fn whole(&self, node: Node<'_>, kind: Kind, text: &str) -> Option<Piece> {
        let range = self.range(node)?;
        Some(Piece::new(kind, text, range))
    }

    /// The pieces of a text node. Escapes and entities become atomic pieces;
    /// the rest is verbatim. A literal that does not line up with its source
    /// is one atomic piece.
    pub(crate) fn text(&mut self, node: Node<'_>, literal: &str) -> Option<Vec<Piece>> {
        let range = self.range(node)?;
        let pieces = self.align_text(range.clone(), literal);
        Some(pieces.unwrap_or_else(|| vec![Piece::new(Kind::Atomic, literal, range)]))
    }

    fn align_text(&mut self, range: Range<usize>, literal: &str) -> Option<Vec<Piece>> {
        let source = self.source;
        let bytes = source.as_bytes();
        let end = range.end;
        let mut pieces = Vec::new();
        let mut run = Run::new(range.start);
        let (mut at, mut lit) = (range.start, 0);
        while let Some(wanted) = literal[lit..].chars().next() {
            if at >= end {
                return None;
            }
            let escaped = bytes[at] == b'\\'
                && at + 1 < end
                && bytes[at + 1].is_ascii_punctuation()
                && char::from(bytes[at + 1]) == wanted;
            if escaped {
                run.flush(&mut pieces, literal, at, lit);
                pieces.push(Piece::new(Kind::Atomic, wanted.to_string(), at..at + 2));
                at += 2;
                lit += 1;
                run = Run { start: at, lit };
                continue;
            }
            if bytes[at] == b'&'
                && let Some((after, decoded)) = self.entity_at(at, end)
                && literal[lit..].starts_with(&decoded)
            {
                run.flush(&mut pieces, literal, at, lit);
                lit += decoded.len();
                pieces.push(Piece::new(Kind::Atomic, decoded, at..after));
                at = after;
                run = Run { start: at, lit };
                continue;
            }
            if !source[at..].starts_with(wanted) {
                return None;
            }
            at += wanted.len_utf8();
            lit += wanted.len_utf8();
        }
        run.flush(&mut pieces, literal, at, lit);
        Some(pieces)
    }

    /// The entity that starts at byte `at`, if one does before `end`: the
    /// byte after it and the text it stands for. An entity ends at a `;`
    /// within 40 bytes; comrak decides whether what lies between is one.
    fn entity_at(&mut self, at: usize, end: usize) -> Option<(usize, String)> {
        // Bytes, not text: the window may end inside a character.
        let window = &self.source.as_bytes()[at + 1..end.min(at + 40)];
        let length = window.iter().position(|&b| b == b';')?;
        let entity = &self.source[at..at + length + 2];
        let decoded = self.decode(entity)?;
        Some((at + length + 2, decoded))
    }

    /// The text an entity stands for, as comrak reads it, or `None` when it
    /// is not one. comrak's entity table is private, so a one-entity
    /// document is parsed instead, once per entity.
    fn decode(&mut self, entity: &str) -> Option<String> {
        self.entities
            .entry(entity.to_owned())
            .or_insert_with(|| {
                let arena = Arena::new();
                let root = parse_document(&arena, entity, &Options::default());
                let text: String = root
                    .descendants()
                    .filter_map(|node| match &node.data().value {
                        NodeValue::Text(text) => Some(text.to_string()),
                        _ => None,
                    })
                    .collect();
                (text != entity).then_some(text)
            })
            .clone()
    }

    /// The pieces of a code span: the backticks, an optional stripped space,
    /// line endings that became spaces, and `\|` inside a table cell.
    pub(crate) fn code_span(&self, node: Node<'_>, literal: &str) -> Option<Vec<Piece>> {
        let range = self.range(node)?;
        let text = code_span_text(literal);
        let bytes = self.source.as_bytes();
        let ticks = bytes[range.clone()]
            .iter()
            .take_while(|&&b| b == b'`')
            .count();
        let content_start = range.start + ticks;
        let content_end = range.end.saturating_sub(ticks).max(content_start);
        let in_cell = node
            .ancestors()
            .any(|ancestor| matches!(ancestor.data().value, NodeValue::TableCell));
        let first = self.align_code(content_start..content_end, &text, in_cell);
        // comrak strips one space, or a line ending, from each side of a
        // padded code span; try again past it.
        let second = || {
            let padded = matches!(bytes.get(content_start), Some(b' ' | b'\n' | b'\r'));
            padded
                .then(|| self.align_code(content_start + 1..content_end, &text, in_cell))
                .flatten()
        };
        Some(
            first
                .or_else(second)
                .unwrap_or_else(|| vec![Piece::new(Kind::Atomic, text.as_ref(), range)]),
        )
    }

    fn align_code(&self, range: Range<usize>, text: &str, in_cell: bool) -> Option<Vec<Piece>> {
        let source = self.source;
        let bytes = source.as_bytes();
        let end = range.end;
        let mut pieces = Vec::new();
        let mut run = Run::new(range.start);
        let (mut at, mut lit) = (range.start, 0);
        while let Some(wanted) = text[lit..].chars().next() {
            if at >= end {
                return None;
            }
            let line_end = match bytes[at] {
                b'\r' if bytes.get(at + 1) == Some(&b'\n') => 2,
                b'\n' | b'\r' => 1,
                _ => 0,
            };
            if line_end > 0 && wanted == ' ' {
                // One line ending inside a code span shows as one space. A
                // Windows line ending is two bytes for one space, so it
                // maps as a whole.
                run.flush(&mut pieces, text, at, lit);
                let kind = if line_end == 1 {
                    Kind::Verbatim
                } else {
                    Kind::Atomic
                };
                pieces.push(Piece::new(kind, " ", at..at + line_end));
                at += line_end;
                lit += 1;
                at = self.skip_line_prefix(at, end);
                run = Run { start: at, lit };
                continue;
            }
            if in_cell && source[at..end].starts_with("\\|") && wanted == '|' {
                run.flush(&mut pieces, text, at, lit);
                pieces.push(Piece::new(Kind::Atomic, "|", at..at + 2));
                at += 2;
                lit += 1;
                run = Run { start: at, lit };
                continue;
            }
            if !source[at..].starts_with(wanted) {
                return None;
            }
            at += wanted.len_utf8();
            lit += wanted.len_utf8();
        }
        run.flush(&mut pieces, text, at, lit);
        Some(pieces)
    }

    /// After a line ending inside a code span, the next line's container
    /// markers (list indent, `>`) and leading spaces are not part of the
    /// span's text. Returns where its text starts again. A `>` there is
    /// always a marker: one that started the line's text would start a block
    /// quote, which ends the paragraph and so the span.
    fn skip_line_prefix(&self, at: usize, end: usize) -> usize {
        let prefix = self.source[at..end]
            .bytes()
            .take_while(|&b| matches!(b, b' ' | b'\t' | b'>'))
            .count();
        at + prefix
    }

    /// The pieces of a code block: each line of its literal is the end of
    /// one source line, after the container's indent or `>` markers. `None`
    /// when a line does not exist in the source.
    pub(crate) fn code_block(
        &self,
        node: Node<'_>,
        literal: &str,
        fenced: bool,
    ) -> Option<Vec<Piece>> {
        let first_line = node.data().sourcepos.start.line + usize::from(fenced);
        let lines = self.positions.lines();
        let mut pieces = Vec::new();
        for (index, line) in literal.split_inclusive('\n').enumerate() {
            let content = line.trim_end_matches(['\n', '\r']);
            let start = lines.line_start(first_line + index)?;
            let next = lines
                .line_start(first_line + index + 1)
                .unwrap_or(self.source.len());
            let source_line = self.source[start..next].trim_end_matches(['\n', '\r']);
            let content_end = start + source_line.len();
            if source_line.ends_with(content) {
                if !content.is_empty() {
                    let from = content_end - content.len();
                    pieces.push(Piece::new(Kind::Verbatim, content, from..content_end));
                }
            } else {
                pieces.push(Piece::new(Kind::Atomic, content, start..content_end));
            }
            if line.ends_with('\n') {
                let ending = &self.source[content_end..next];
                let kind = if ending == "\n" {
                    Kind::Verbatim
                } else {
                    Kind::Atomic
                };
                pieces.push(Piece::new(kind, "\n", content_end..next));
            }
        }
        Some(merged(pieces))
    }
}

/// A stretch of verbatim text still being read: where it started in the
/// source and in the literal.
struct Run {
    start: usize,
    lit: usize,
}

impl Run {
    fn new(start: usize) -> Run {
        Run { start, lit: 0 }
    }

    /// Ends the run at source byte `at` and literal byte `lit`, adding it as
    /// a verbatim piece when it holds text.
    fn flush(&self, pieces: &mut Vec<Piece>, literal: &str, at: usize, lit: usize) {
        if at > self.start {
            pieces.push(Piece::new(
                Kind::Verbatim,
                &literal[self.lit..lit],
                self.start..at,
            ));
        }
    }
}

/// Joins verbatim pieces that touch in the source, so a code block of many
/// lines is a few pieces, not two per line.
fn merged(pieces: Vec<Piece>) -> Vec<Piece> {
    let mut out: Vec<Piece> = Vec::with_capacity(pieces.len());
    for piece in pieces {
        if let Some(last) = out.last_mut()
            && last.kind == Kind::Verbatim
            && piece.kind == Kind::Verbatim
            && last.source.end == piece.source.start
        {
            last.text.push_str(&piece.text);
            last.source.end = piece.source.end;
        } else {
            out.push(piece);
        }
    }
    out
}

#[cfg(test)]
mod tests;
