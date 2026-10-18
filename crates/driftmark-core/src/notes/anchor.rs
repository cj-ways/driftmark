//! Where a note sits: capturing a stretch of source when the note is made,
//! and finding the same stretch again in a later version.
//!
//! A note keeps the exact source text it was made on, about 32 characters
//! before and after it, its byte offsets, the headings above it and its
//! block, following the W3C Web Annotation model's quote and position
//! selectors (decision 0004). When the document changes, [`reanchor`] tries,
//! in order:
//!
//! 1. the same bytes: the version is the one the note was made on;
//! 2. the block diff: the note's block is followed to the block the change
//!    list pairs it with, and the quote is looked for there, exactly first,
//!    then fuzzily, expected at the same offset into the block;
//! 3. the same search in the note's section;
//! 4. the same search in the whole document.
//!
//! The scoring of a match follows the Hypothesis client's quote matcher
//! (`src/annotator/anchoring/match-quote.ts` in github.com/hypothesis/client,
//! BSD-2-Clause): the quote counts 50, the text before it 20, the text after
//! it 20 and the distance from the expected place 2, each as a similarity
//! from 0 to 1. A search allows at most half the quote's length in edits, up
//! to 256. This is new code that follows that published idea. Unlike
//! Hypothesis, a weak match is refused: a short quote needs its context to
//! agree as well ([`ACCEPT`], [`LONG_QUOTE`]).

use std::ops::Range;

use serde::{Deserialize, Serialize};

use super::fuzzy::{self, Found};
use crate::diff::{BlockMap, block_map};
use crate::document::Document;

/// How many characters of context a note keeps on each side of its quote:
/// the W3C model's suggestion, which Hypothesis also uses.
pub const CONTEXT: usize = 32;

/// The lowest similarity of the quote itself for a match to place a note:
/// three quarters of its characters right.
pub const ACCEPT_QUOTE: f64 = 0.75;

/// The lowest combined score, quote and context, for a match of a short
/// quote. A short quote such as one word occurs in many places, so its
/// context must agree too.
pub const ACCEPT: f64 = 0.75;

/// A quote of at least this many characters is evidence enough on its own:
/// it needs only [`ACCEPT_QUOTE`], so a sentence whose neighbours were
/// rewritten is still found.
pub const LONG_QUOTE: usize = 32;

/// The most edits any search allows, as in Hypothesis.
pub const MAX_ERRORS: usize = 256;

const QUOTE_WEIGHT: f64 = 50.0;
const PREFIX_WEIGHT: f64 = 20.0;
const SUFFIX_WEIGHT: f64 = 20.0;
const POSITION_WEIGHT: f64 = 2.0;

/// The text a note was made on, with its context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quote {
    /// The source text, exactly.
    pub exact: String,
    /// Up to [`CONTEXT`] characters of source just before it.
    pub prefix: String,
    /// Up to [`CONTEXT`] characters of source just after it.
    pub suffix: String,
}

/// Where a note was made in one version of a document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Anchor {
    /// The first byte of the quote in that version's source.
    pub start: usize,
    /// The byte after the quote.
    pub end: usize,
    /// The quote and its context.
    pub quote: Quote,
    /// The titles of the headings above the quote's first block, outermost
    /// first.
    pub heading_path: Vec<String>,
    /// The index of the quote's first block.
    pub block: usize,
    /// That block's source hash, as 16 hex digits.
    pub block_hash: String,
}

impl Anchor {
    /// The anchor of a stretch of `document`'s source, or `None` when the
    /// range is empty, outside the source, splits a character, holds only
    /// white space, or lies after the last block.
    #[must_use]
    pub fn capture(document: &Document, range: Range<usize>) -> Option<Anchor> {
        let source = &document.source;
        let exact = source.get(range.clone())?;
        if exact.trim().is_empty() {
            return None;
        }
        let block = block_of(document, range.start)?;
        let found = &document.blocks[block];
        Some(Anchor {
            start: range.start,
            end: range.end,
            quote: Quote {
                exact: exact.to_owned(),
                prefix: before(source, range.start, CONTEXT).into_iter().collect(),
                suffix: source[range.end..].chars().take(CONTEXT).collect(),
            },
            heading_path: found.heading_path.to_vec(),
            block,
            block_hash: format!("{:016x}", found.source_hash),
        })
    }
}

/// How a note was found again in a version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnchorState {
    /// The quote is unchanged and in the same place: in its own block, which
    /// did not move, or elsewhere in its section.
    Exact,
    /// The quote is unchanged, but it now sits in a block that moved, or in
    /// another section.
    Moved,
    /// The quote was found with small differences.
    Fuzzy,
    /// The quote's block is still there, but the text the note was made on
    /// changed after the note. The note sits on the block, or on what is
    /// left of the quote in it.
    Outdated,
    /// Neither the quote nor its block was found. The note is kept, shown
    /// apart from the text.
    Orphaned,
}

impl AnchorState {
    /// The state's name in JSON.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            AnchorState::Exact => "exact",
            AnchorState::Moved => "moved",
            AnchorState::Fuzzy => "fuzzy",
            AnchorState::Outdated => "outdated",
            AnchorState::Orphaned => "orphaned",
        }
    }
}

/// Where a note sits in one version.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    /// How it was found.
    pub state: AnchorState,
    /// The source bytes it sits on; `None` when orphaned.
    pub range: Option<Range<usize>>,
    /// How well the quote matched, from 0 to 1, rounded to three decimals: 1
    /// for an unchanged quote, 0 when nothing of it was found.
    pub score: f64,
    /// The index of the block the range starts in.
    pub block: Option<usize>,
}

impl Placement {
    pub(crate) fn orphaned() -> Placement {
        Placement {
            state: AnchorState::Orphaned,
            range: None,
            score: 0.0,
            block: None,
        }
    }
}

/// The version a note was made on, as far as the caller knows it.
#[derive(Debug, Clone, Copy)]
pub enum Origin<'a> {
    /// The version being anchored in has the same bytes.
    Same,
    /// The version the note was made on.
    Known(&'a Document),
    /// The version the note was made on, with its blocks already paired
    /// with the new version's by [`block_map`]. Many notes on one version
    /// then share one pairing.
    Paired(&'a Document, &'a BlockMap),
    /// The version cannot be read any more; only the quote, its context and
    /// its headings are left.
    Lost,
}

/// Finds a note again in `new`.
///
/// See the module documentation for the order of the steps. A note whose
/// quote and block are both gone is [`AnchorState::Orphaned`]; it is never
/// dropped.
#[must_use]
pub fn reanchor(anchor: &Anchor, origin: Origin<'_>, new: &Document) -> Placement {
    let source = &new.source;
    if matches!(origin, Origin::Same)
        && source.get(anchor.start..anchor.end) == Some(anchor.quote.exact.as_str())
    {
        return Placement {
            state: AnchorState::Exact,
            range: Some(anchor.start..anchor.end),
            score: 1.0,
            block: block_of(new, anchor.start),
        };
    }
    let followed = match origin {
        Origin::Known(old) => follow(anchor, old, new, &block_map(old, new)),
        Origin::Paired(old, map) => follow(anchor, old, new, map),
        Origin::Same | Origin::Lost => None,
    };
    let search = Search::new(anchor, new, followed.as_ref());
    let mut weak = None;
    if let Some(followed) = &followed {
        let found = search.best(followed.window.clone());
        if let Some(found) = found.as_ref().filter(|found| search.accepted(found)) {
            return search.place(found);
        }
        weak = found;
    }
    if let Some(window) = search.section_window()
        && let Some(found) = search.best(window).filter(|found| search.accepted(found))
    {
        return search.place(&found);
    }
    if let Some(found) = search
        .best(0..source.len())
        .filter(|found| search.accepted(found))
    {
        return search.place(&found);
    }
    let long = search.quote.len() >= LONG_QUOTE;
    match followed {
        Some(followed) => {
            // What is left of a long quote counts where its context agrees.
            // A short quote, or one whose context changed too, gives the
            // whole block: a fragment of a few characters says little.
            let (range, score) = match weak.filter(|found| long && found.score >= ACCEPT) {
                Some(found) => (found.range, found.quote),
                None => (followed.window, 0.0),
            };
            Placement {
                state: AnchorState::Outdated,
                block: block_of(new, range.start),
                range: Some(range),
                score: rounded(score),
            }
        }
        None => Placement::orphaned(),
    }
}

/// Finds selected text inside one block: the stretch of the block's source
/// that best matches `text`, closest to `hint` characters from the block's
/// start. A change page shows a block's plain text, so the selection holds
/// no markup; the search lets the source's markup count as edits. A table
/// row's cells are joined by a tab there and by ` | ` in the source, so a
/// tab is looked for as ` | `.
///
/// `None` when the block does not exist, the text is only white space, or
/// nothing in the block is close enough: at most half the text's characters
/// may differ.
#[must_use]
pub fn locate_text(
    document: &Document,
    block: usize,
    text: &str,
    hint: usize,
) -> Option<Range<usize>> {
    let wanted: Vec<char> = text.trim().replace('\t', " | ").chars().collect();
    if wanted.is_empty() {
        return None;
    }
    let range = document.blocks.get(block)?.range.clone();
    let source = &document.source;
    let found = find(source, range.clone(), &wanted, wanted.len() / 2);
    let best = found.iter().min_by_key(|found| {
        let into = source[range.start..found.start].chars().count();
        (found.errors, into.abs_diff(hint))
    })?;
    Some(best.start..best.end)
}

/// The index of the block that holds byte `offset`, or the next block when
/// the offset falls between blocks.
fn block_of(document: &Document, offset: usize) -> Option<usize> {
    let at = document
        .blocks
        .partition_point(|block| block.range.end <= offset);
    (at < document.blocks.len()).then_some(at)
}

/// Up to `count` characters of `source` that end at byte `end`, in order.
fn before(source: &str, end: usize, count: usize) -> Vec<char> {
    let mut chars: Vec<char> = source[..end].chars().rev().take(count).collect();
    chars.reverse();
    chars
}

fn rounded(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

/// A note's block followed into the new version.
struct Followed {
    /// The source bytes of the new blocks paired with the note's blocks.
    window: Range<usize>,
    /// Where the quote would start if its block kept its text before it.
    offset: Option<usize>,
    /// For each new block, whether the pairing says it moved.
    moved: Vec<bool>,
    /// The new blocks paired with the note's blocks that stayed in place.
    kept: Vec<usize>,
}

/// Follows the blocks the note covers into `new`, through the pairing the
/// change list uses. `None` when none of them has a pair.
fn follow(anchor: &Anchor, old: &Document, new: &Document, map: &BlockMap) -> Option<Followed> {
    let mut moved = vec![false; new.blocks.len()];
    for (o, partner) in map.old_to_new.iter().enumerate() {
        if let Some(n) = *partner {
            moved[n] = !map.in_place[o];
        }
    }
    let first = block_of(old, anchor.start)?;
    let covered = old.blocks[first..]
        .iter()
        .take_while(|block| block.range.start < anchor.end)
        .count();
    let paired: Vec<usize> = map.old_to_new[first..first + covered]
        .iter()
        .filter_map(|partner| *partner)
        .collect();
    let kept: Vec<usize> = paired.iter().copied().filter(|&n| !moved[n]).collect();
    let lowest = paired.iter().copied().min()?;
    let highest = paired.iter().copied().max().unwrap_or(lowest);
    let window = new.blocks[lowest].range.start..new.blocks[highest].range.end;
    let offset = map.old_to_new[first].and_then(|n| {
        let into = anchor.start.checked_sub(old.blocks[first].range.start)?;
        Some(new.blocks[n].range.start + into)
    });
    Some(Followed {
        window,
        offset,
        moved,
        kept,
    })
}

/// The places of `wanted` in `range` of `source`, in byte offsets into
/// `source`: exact ones when there are any, otherwise the closest ones
/// within `max_errors` edits.
fn find(source: &str, range: Range<usize>, wanted: &[char], max_errors: usize) -> Vec<Found> {
    let text = &source[range.clone()];
    let needle: String = wanted.iter().collect();
    let exact: Vec<Found> = text
        .match_indices(&needle)
        .map(|(at, _)| Found {
            start: range.start + at,
            end: range.start + at + needle.len(),
            errors: 0,
        })
        .collect();
    if !exact.is_empty() {
        return exact;
    }
    fuzzy::search(text, wanted, max_errors)
        .into_iter()
        .map(|found| Found {
            start: range.start + found.start,
            end: range.start + found.end,
            errors: found.errors,
        })
        .collect()
}

/// A scored match in the new version.
#[derive(Debug, Clone)]
struct Scored {
    /// Its source bytes.
    range: Range<usize>,
    /// How many edits from the quote.
    errors: usize,
    /// The quote's similarity, from 0 to 1.
    quote: f64,
    /// The combined score, from 0 to 1.
    score: f64,
}

/// Everything one re-anchoring needs to score matches.
struct Search<'a> {
    anchor: &'a Anchor,
    new: &'a Document,
    quote: Vec<char>,
    prefix: Vec<char>,
    suffix: Vec<char>,
    /// The byte where the quote is expected.
    hint: usize,
    /// For each new block, whether it moved; empty when not known.
    moved: &'a [bool],
    /// The new blocks paired with the note's own that stayed in place.
    kept: &'a [usize],
}

impl<'a> Search<'a> {
    fn new(anchor: &'a Anchor, new: &'a Document, followed: Option<&'a Followed>) -> Search<'a> {
        Search {
            anchor,
            new,
            quote: anchor.quote.exact.chars().collect(),
            prefix: anchor.quote.prefix.chars().collect(),
            suffix: anchor.quote.suffix.chars().collect(),
            hint: followed
                .and_then(|followed| followed.offset)
                .unwrap_or(anchor.start),
            moved: followed.map_or(&[], |followed| followed.moved.as_slice()),
            kept: followed.map_or(&[], |followed| followed.kept.as_slice()),
        }
    }

    /// The match among `window`'s best places with the highest score.
    fn best(&self, window: Range<usize>) -> Option<Scored> {
        let max = (self.quote.len() / 2).min(MAX_ERRORS);
        let mut best: Option<Scored> = None;
        for found in find(&self.new.source, window, &self.quote, max) {
            let mut range = found.start..found.end;
            if found.errors > 0 {
                range = whole_words(&self.new.source, range);
            }
            let scored = self.score(range.start, range.end, found.errors);
            if best.as_ref().is_none_or(|best| scored.score > best.score) {
                best = Some(scored);
            }
        }
        best
    }

    /// Scores the match at bytes `start..end` with `errors` edits.
    fn score(&self, start: usize, end: usize, errors: usize) -> Scored {
        let source = &self.new.source;
        let similarity = |text: &[char], wanted: &[char]| {
            if wanted.is_empty() {
                1.0
            } else {
                1.0 - share(fuzzy::fewest_errors(text, wanted), wanted.len())
            }
        };
        let quote = 1.0 - share(errors, self.quote.len());
        let prefix = similarity(&before(source, start, self.prefix.len()), &self.prefix);
        let after: Vec<char> = source[end..].chars().take(self.suffix.len()).collect();
        let suffix = similarity(&after, &self.suffix);
        let position = 1.0 - share(start.abs_diff(self.hint), source.len().max(1));
        let total = QUOTE_WEIGHT * quote
            + PREFIX_WEIGHT * prefix
            + SUFFIX_WEIGHT * suffix
            + POSITION_WEIGHT * position;
        Scored {
            range: start..end,
            errors,
            quote,
            score: total / (QUOTE_WEIGHT + PREFIX_WEIGHT + SUFFIX_WEIGHT + POSITION_WEIGHT),
        }
    }

    /// Whether a match is good enough to place the note: a long quote needs
    /// only its own similarity, a short one its context as well.
    fn accepted(&self, found: &Scored) -> bool {
        found.quote >= ACCEPT_QUOTE && (self.quote.len() >= LONG_QUOTE || found.score >= ACCEPT)
    }

    /// The note's section in the new version: the blocks under the note's
    /// own headings, from the first to the last.
    fn section_window(&self) -> Option<Range<usize>> {
        let blocks = &self.new.blocks;
        let mut under = blocks
            .iter()
            .filter(|block| *block.heading_path == *self.anchor.heading_path);
        let first = under.next()?;
        let last = under.next_back().unwrap_or(first);
        Some(first.range.start..last.range.end)
    }

    /// The placement of an accepted match.
    fn place(&self, found: &Scored) -> Placement {
        let block = block_of(self.new, found.range.start);
        let state = if found.errors > 0 {
            AnchorState::Fuzzy
        } else {
            let own = block.is_some_and(|block| self.kept.contains(&block));
            let same_section = block.is_some_and(|block| {
                *self.new.blocks[block].heading_path == *self.anchor.heading_path
            });
            let moved = block.is_some_and(|block| self.moved.get(block) == Some(&true));
            if own || (same_section && !moved) {
                AnchorState::Exact
            } else {
                AnchorState::Moved
            }
        };
        Placement {
            state,
            range: Some(found.range.clone()),
            score: rounded(found.quote),
            block,
        }
    }
}

/// Grows a fuzzy match to whole words. The fewest edits often match only
/// the end of a changed word: "two days" finds "ree days" in "three days".
/// A note shown on a fragment reads badly, so a match that starts or ends
/// inside a word takes the whole word.
fn whole_words(source: &str, range: Range<usize>) -> Range<usize> {
    let word = |c: &char| c.is_alphanumeric();
    let mut start = range.start;
    if source[start..].chars().next().as_ref().is_some_and(word) {
        while let Some(c) = source[..start].chars().next_back().filter(word) {
            start -= c.len_utf8();
        }
    }
    let mut end = range.end;
    if source[..end].chars().next_back().as_ref().is_some_and(word) {
        while let Some(c) = source[end..].chars().next().filter(word) {
            end += c.len_utf8();
        }
    }
    start..end
}

/// `part / whole` as a fraction.
#[expect(
    clippy::cast_precision_loss,
    reason = "documents are far below 2^52 bytes"
)]
fn share(part: usize, whole: usize) -> f64 {
    part as f64 / whole as f64
}

#[cfg(test)]
mod tests;
