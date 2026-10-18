//! What a result shows: a short stretch of the matching block with the
//! matched words marked, and where the block sits in its document.

use std::ops::Range;

use serde::Serialize;
use unicode_segmentation::UnicodeSegmentation;

use super::query::Query;
use super::words::{fold, words};
use crate::document::{Block, BlockKind, Document};
use crate::lint::Anchors;

/// About how many characters of a block a snippet shows. A cut never splits
/// a word, so a snippet can run past this by up to one word at each end.
pub const SNIPPET_CHARS: usize = 160;

/// How many characters a snippet shows before its first match, at most,
/// when the block is cut.
pub const LEAD_CHARS: usize = 40;

/// What a cut end of a snippet shows.
pub const CUT: &str = "…";

/// One stretch of a snippet. Joined, the pieces give the snippet's text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Piece {
    /// The text. Runs of white space are one space; in a table row, the
    /// space between two cells is ` · `.
    pub text: String,
    /// Whether it is a match: a word the query asked for, or a whole phrase.
    pub hit: bool,
}

/// Where a block sits in its document.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Place {
    /// The titles of the headings above the block, outermost first. A
    /// heading outside any container opens a section, so its own title
    /// comes last.
    pub headings: Vec<String>,
    /// The page id of the nearest heading at or above the block, without
    /// `#`, for a link to that section. `None` when no heading comes before
    /// the block.
    pub anchor: Option<String>,
}

/// Where block `block` of `document` sits. A block index past the end gives
/// an empty place, which happens only when a file changed after it was
/// indexed.
#[must_use]
pub fn place(document: &Document, block: usize) -> Place {
    let Some(found) = document.blocks.get(block) else {
        return Place::default();
    };
    let opens = |block: &Block| {
        matches!(block.kind, BlockKind::Heading { .. }) && block.containers.is_empty()
    };
    let mut headings = found.heading_path.to_vec();
    if opens(found) {
        headings.push(found.text.clone());
    }
    let anchors = Anchors::by_block(document);
    let heading = if matches!(found.kind, BlockKind::Heading { .. }) {
        Some(block)
    } else {
        document.blocks[..block].iter().rposition(opens)
    };
    Place {
        headings,
        anchor: heading.and_then(|at| anchors[at].clone()),
    }
}

impl Query {
    /// The snippet of `block` for this query: up to about
    /// [`SNIPPET_CHARS`] characters of its plain text, around the place
    /// where the most matches stand close together, with each match marked.
    ///
    /// - A word of a clause is marked wherever it stands; a phrase is marked
    ///   only where its words stand together, as one piece.
    /// - A block that fits is shown whole. A longer one is cut: the snippet
    ///   starts up to [`LEAD_CHARS`] characters before the first match it
    ///   shows, and [`CUT`] marks each end that was cut.
    /// - A cut falls between words, never inside one.
    #[must_use]
    pub fn snippet(&self, block: &Block) -> Vec<Piece> {
        let text = block.text.as_str();
        let marks = self.marks(text);
        let total = text.chars().count();
        let window = if total <= SNIPPET_CHARS {
            0..text.len()
        } else {
            window(text, total, &marks)
        };
        let mut out = Pieces {
            pieces: Vec::new(),
            table: matches!(block.kind, BlockKind::TableRow { .. }),
        };
        if window.start > 0 {
            out.push(CUT, false);
        }
        // Marks are sorted and apart, so `at` never passes a mark's start.
        // A mark outside the window clamps to an empty piece at its edge.
        let mut at = window.start;
        for mark in &marks {
            let start = mark.start.clamp(window.start, window.end);
            let end = mark.end.clamp(start, window.end);
            out.push(&text[at..start], false);
            out.push(&text[start..end], true);
            at = end;
        }
        out.push(&text[at..window.end], false);
        if window.end < text.len() {
            out.push(CUT, false);
        }
        out.pieces
    }

    /// The byte ranges of `text` that match, sorted, with overlapping and
    /// touching ones joined, as Chinese words that stand side by side.
    fn marks(&self, text: &str) -> Vec<Range<usize>> {
        let found: Vec<(Range<usize>, String)> = words(text)
            .into_iter()
            .map(|(at, word)| (at..at + word.len(), fold(word)))
            .collect();
        let mut marks: Vec<Range<usize>> = Vec::new();
        for clause in &self.clauses {
            for run in found.windows(clause.slots.len()) {
                let all = run
                    .iter()
                    .zip(&clause.slots)
                    .all(|((_, term), slot)| slot.matches(term));
                if all {
                    marks.push(run[0].0.start..run[run.len() - 1].0.end);
                }
            }
        }
        marks.sort_by_key(|mark| mark.start);
        let mut joined: Vec<Range<usize>> = Vec::new();
        for mark in marks {
            match joined.last_mut() {
                Some(last) if mark.start <= last.end => last.end = last.end.max(mark.end),
                _ => joined.push(mark),
            }
        }
        joined
    }
}

/// The pieces of a snippet as they are written, with text that is not a
/// match joined into one piece.
struct Pieces {
    pieces: Vec<Piece>,
    table: bool,
}

impl Pieces {
    fn push(&mut self, text: &str, hit: bool) {
        let text = tidy(text, self.table);
        if text.is_empty() {
            return;
        }
        match self.pieces.last_mut() {
            Some(last) if !last.hit && !hit => last.text.push_str(&text),
            _ => self.pieces.push(Piece { text, hit }),
        }
    }
}

/// The bytes of a long text a snippet shows: the run of marks that fits in
/// [`SNIPPET_CHARS`] characters and holds the most of them, the first such
/// run on a tie, with up to [`LEAD_CHARS`] before it, then as much after it
/// as the length allows. Both ends move out to the nearest word boundary,
/// and white space at the ends is left out.
fn window(text: &str, total: usize, marks: &[Range<usize>]) -> Range<usize> {
    let offsets = char_offsets(text, marks.iter().flat_map(|mark| [mark.start, mark.end]));
    let (from, to) = (
        |mark: usize| offsets[2 * mark],
        |mark: usize| offsets[2 * mark + 1],
    );
    let mut best = (0, 0..0);
    let mut last = 0;
    for first in 0..marks.len() {
        last = last.max(first);
        while last + 1 < marks.len() && to(last + 1) - from(first) <= SNIPPET_CHARS {
            last += 1;
        }
        let held = last + 1 - first;
        if held > best.0 {
            best = (held, from(first)..to(last));
        }
    }
    let span = best.1;
    let start = span
        .start
        .saturating_sub(LEAD_CHARS)
        .min(total - SNIPPET_CHARS);
    let end = (start + SNIPPET_CHARS).max(span.end);
    let byte = |at: usize| {
        text.char_indices()
            .nth(at)
            .map_or(text.len(), |(byte, _)| byte)
    };
    let (start, end) = (byte(start), byte(end));
    let bounds: Vec<usize> = text
        .split_word_bound_indices()
        .map(|(at, _)| at)
        .chain(std::iter::once(text.len()))
        .collect();
    let start = bounds[bounds.partition_point(|&at| at <= start) - 1];
    let end = bounds[bounds.partition_point(|&at| at < end)];
    let inner = &text[start..end];
    let lead = inner.len() - inner.trim_start().len();
    let trail = inner.len() - inner.trim_end().len();
    start + lead..end - trail
}

/// The character offsets of byte offsets in `text`, which come in order.
fn char_offsets(text: &str, bytes: impl Iterator<Item = usize>) -> Vec<usize> {
    let mut chars = text.char_indices().map(|(at, _)| at).peekable();
    let mut seen = 0;
    bytes
        .map(|byte| {
            while chars.next_if(|&at| at < byte).is_some() {
                seen += 1;
            }
            seen
        })
        .collect()
}

/// Text as a snippet shows it: each run of white space becomes one space,
/// or ` · ` in a table row where the run holds the tab between two cells.
fn tidy(text: &str, table: bool) -> String {
    let mut out = String::with_capacity(text.len());
    let mut run = String::new();
    for character in text.chars() {
        if character.is_whitespace() {
            run.push(character);
            continue;
        }
        flush(&mut out, &mut run, table);
        out.push(character);
    }
    flush(&mut out, &mut run, table);
    out
}

fn flush(out: &mut String, run: &mut String, table: bool) {
    if run.is_empty() {
        return;
    }
    out.push_str(if table && run.contains('\t') {
        " · "
    } else {
        " "
    });
    run.clear();
}

#[cfg(test)]
mod tests;
