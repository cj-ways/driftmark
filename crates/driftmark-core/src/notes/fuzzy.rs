//! Approximate search: where a pattern occurs in a text with the fewest
//! edits, counting each inserted, deleted or replaced character as one.
//!
//! It is the bit-vector algorithm of Gene Myers, "A fast bit-vector
//! algorithm for approximate string matching based on dynamic programming"
//! (Journal of the ACM 46(3), 1999), in the form Heikki Hyyrö explains in
//! "Explaining and extending the bit-parallel approximate string matching
//! algorithm of Myers" (2001), with the pattern cut into 64-bit blocks. Each
//! text character costs one step per block, so a search through a document is
//! fast even for a long quote. The Hypothesis client, whose quote matching
//! decision 0004 follows, uses the same algorithm through the
//! `approx-string-match` library; this is a new implementation from the
//! papers.
//!
//! Edits count characters (Unicode scalar values). [`search`] reads a text
//! as it is and reports byte offsets into it, so a search through a whole
//! document copies nothing.

use std::collections::HashMap;

/// One place where the pattern matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Found {
    /// The byte where the match starts in the text.
    pub(crate) start: usize,
    /// The byte after the match.
    pub(crate) end: usize,
    /// How many edits turn the matched text into the pattern.
    pub(crate) errors: usize,
}

/// A pattern prepared for the search: for every character it holds, the
/// rows where it stands, as bits.
struct Pattern {
    /// The number of 64-bit blocks.
    blocks: usize,
    /// The bit of the last row inside the last block.
    last: u64,
    /// The masks of the characters below 256, `blocks` per character, so the
    /// common case is one index instead of a hash lookup per text character.
    low: Vec<u64>,
    /// The masks of the other characters the pattern holds.
    high: HashMap<char, Vec<u64>>,
    /// The masks of a character the pattern does not hold.
    none: Vec<u64>,
}

impl Pattern {
    fn new(pattern: &[char]) -> Pattern {
        let blocks = pattern.len().div_ceil(64);
        let mut low = vec![0; 256 * blocks];
        let mut high: HashMap<char, Vec<u64>> = HashMap::new();
        for (row, &c) in pattern.iter().enumerate() {
            let bit = 1 << (row % 64);
            match u8::try_from(c) {
                Ok(byte) => low[usize::from(byte) * blocks + row / 64] |= bit,
                Err(_) => high.entry(c).or_insert_with(|| vec![0; blocks])[row / 64] |= bit,
            }
        }
        Pattern {
            blocks,
            last: 1 << ((pattern.len() - 1) % 64),
            low,
            high,
            none: vec![0; blocks],
        }
    }

    fn masks(&self, c: char) -> &[u64] {
        match u8::try_from(c) {
            Ok(byte) => {
                let at = usize::from(byte) * self.blocks;
                &self.low[at..at + self.blocks]
            }
            Err(_) => self.high.get(&c).map_or(&self.none, Vec::as_slice),
        }
    }
}

/// Where the search may start in the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Start {
    /// Anywhere: each column scores the best match that ends there.
    Anywhere,
    /// At the first character: each column scores the whole text so far.
    First,
}

/// Runs the pattern over the text and calls `each(column, score)` after
/// every character, where `column` is the number of characters read so far
/// and `score` the edit distance of the best match that ends there.
fn columns(
    pattern: &Pattern,
    rows: usize,
    text: impl Iterator<Item = char>,
    start: Start,
    mut each: impl FnMut(usize, usize),
) {
    // Vertical deltas: every row starts one more than the row above.
    let mut plus = vec![u64::MAX; pattern.blocks];
    let mut minus = vec![0_u64; pattern.blocks];
    let mut score = rows;
    let top = match start {
        Start::Anywhere => 0,
        Start::First => 1,
    };
    for (column, c) in text.enumerate() {
        let masks = pattern.masks(c);
        // The horizontal delta that enters the top of the first block: 0
        // when a match may start anywhere, +1 when it must start at the
        // first character.
        let mut carry: i8 = top;
        for block in 0..pattern.blocks {
            let high = if block + 1 == pattern.blocks {
                pattern.last
            } else {
                1 << 63
            };
            let (pv, mv) = (plus[block], minus[block]);
            let mut eq = masks[block];
            let xv = eq | mv;
            if carry < 0 {
                eq |= 1;
            }
            let xh = ((eq & pv).wrapping_add(pv) ^ pv) | eq;
            let mut ph = mv | !(xh | pv);
            let mut mh = pv & xh;
            let out: i8 = if ph & high != 0 {
                1
            } else if mh & high != 0 {
                -1
            } else {
                0
            };
            ph <<= 1;
            mh <<= 1;
            if carry < 0 {
                mh |= 1;
            } else if carry > 0 {
                ph |= 1;
            }
            plus[block] = mh | !(xv | ph);
            minus[block] = ph & xv;
            carry = out;
        }
        score = score.wrapping_add_signed(isize::from(carry));
        each(column + 1, score);
    }
}

/// Every place where `pattern` occurs in `text` with the fewest edits, if
/// that is at most `max_errors`, in byte offsets into `text`. An empty
/// pattern occurs nowhere.
///
/// Matches come in text order. A match ends where the score is lowest; its
/// start is the earliest that gives the same number of edits, so the match
/// is as long as it can be, as in `approx-string-match`.
pub(crate) fn search(text: &str, pattern: &[char], max_errors: usize) -> Vec<Found> {
    if pattern.is_empty() {
        return Vec::new();
    }
    let prepared = Pattern::new(pattern);
    let mut best = max_errors;
    let mut ends: Vec<usize> = Vec::new();
    columns(
        &prepared,
        pattern.len(),
        text.chars(),
        Start::Anywhere,
        |end, score| {
            if score < best {
                best = score;
                ends.clear();
            }
            if score == best {
                ends.push(end);
            }
        },
    );
    let reversed: Vec<char> = pattern.iter().rev().copied().collect();
    let backwards = Pattern::new(&reversed);
    bytes_after(text, &ends)
        .into_iter()
        .map(|end| Found {
            start: start_of(&backwards, pattern.len(), text, end, best),
            end,
            errors: best,
        })
        .collect()
}

/// The byte offset after each of `columns` characters of `text`; `columns`
/// rise.
fn bytes_after(text: &str, columns: &[usize]) -> Vec<usize> {
    let mut ends = text.char_indices().map(|(at, c)| at + c.len_utf8());
    let (mut read, mut byte) = (0, 0);
    columns
        .iter()
        .map(|&column| {
            while read < column {
                byte = ends.next().unwrap_or(text.len());
                read += 1;
            }
            byte
        })
        .collect()
}

/// Where a match that ends at byte `end` with `errors` edits starts. Reading
/// the text backwards from `end`, the reversed pattern scores every length;
/// the longest that scores `errors` wins. No match is longer than the
/// pattern plus its edits.
fn start_of(backwards: &Pattern, rows: usize, text: &str, end: usize, errors: usize) -> usize {
    let before = &text[..end];
    let mut longest = 0;
    columns(
        backwards,
        rows,
        before.chars().rev().take(rows + errors),
        Start::First,
        |length, score| {
            if score == errors {
                longest = length;
            }
        },
    );
    before
        .char_indices()
        .rev()
        .take(longest)
        .last()
        .map_or(end, |(at, _)| at)
}

/// The fewest edits that turn some stretch of `text` into `pattern`: 0 when
/// the pattern occurs as it is, at most the pattern's length.
pub(crate) fn fewest_errors(text: &[char], pattern: &[char]) -> usize {
    if pattern.is_empty() {
        return 0;
    }
    let mut best = pattern.len();
    let prepared = Pattern::new(pattern);
    columns(
        &prepared,
        pattern.len(),
        text.iter().copied(),
        Start::Anywhere,
        |_, score| best = best.min(score),
    );
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(text: &str) -> Vec<char> {
        text.chars().collect()
    }

    /// The textbook edit-distance table, for checking the bit vectors.
    fn slow_best(text: &[char], pattern: &[char]) -> Vec<usize> {
        let mut column: Vec<usize> = (0..=pattern.len()).collect();
        let mut out = Vec::new();
        for &c in text {
            let mut next = vec![0; pattern.len() + 1];
            for row in 1..=pattern.len() {
                let replace = column[row - 1] + usize::from(pattern[row - 1] != c);
                next[row] = replace.min(column[row] + 1).min(next[row - 1] + 1);
            }
            out.push(next[pattern.len()]);
            column = next;
        }
        out
    }

    fn fast_best(text: &[char], pattern: &[char], start: Start) -> Vec<usize> {
        let mut out = Vec::new();
        columns(
            &Pattern::new(pattern),
            pattern.len(),
            text.iter().copied(),
            start,
            |column, score| {
                assert_eq!(column, out.len() + 1);
                out.push(score);
            },
        );
        out
    }

    /// The edit distance of the pattern and each prefix of the text.
    fn slow_whole(text: &[char], pattern: &[char]) -> Vec<usize> {
        let mut column: Vec<usize> = (0..=pattern.len()).collect();
        let mut out = Vec::new();
        for (at, &c) in text.iter().enumerate() {
            let mut next = vec![at + 1; pattern.len() + 1];
            for row in 1..=pattern.len() {
                let replace = column[row - 1] + usize::from(pattern[row - 1] != c);
                next[row] = replace.min(column[row] + 1).min(next[row - 1] + 1);
            }
            out.push(next[pattern.len()]);
            column = next;
        }
        out
    }

    /// A small deterministic generator, so the comparison covers many
    /// shapes without a dependency.
    struct Dice(u64);

    impl Dice {
        fn next(&mut self, below: u64) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 33) % below
        }

        fn text(&mut self, length: u64) -> Vec<char> {
            (0..=self.next(length))
                .map(|_| ['a', 'b', 'c', 'é', '€', ' '][usize::try_from(self.next(6)).unwrap()])
                .collect()
        }
    }

    #[test]
    fn the_bit_vectors_agree_with_the_table_for_every_length() {
        let mut dice = Dice(7);
        for round in 0..400 {
            // Patterns up to 200 characters cross the 64-bit block edges.
            let longest = if round % 4 == 0 { 200 } else { 70 };
            let pattern = dice.text(longest);
            let text = dice.text(120);
            assert_eq!(
                fast_best(&text, &pattern, Start::Anywhere),
                slow_best(&text, &pattern),
                "{pattern:?} in {text:?}"
            );
            assert_eq!(
                fast_best(&text, &pattern, Start::First),
                slow_whole(&text, &pattern),
                "{pattern:?} against {text:?}"
            );
        }
    }

    /// The edit distance of two whole strings.
    fn distance(a: &[char], b: &[char]) -> usize {
        *slow_whole(a, b).last().unwrap_or(&b.len())
    }

    #[test]
    fn matches_end_at_the_lowest_score_and_start_by_the_rule() {
        let mut dice = Dice(11);
        for _ in 0..300 {
            let pattern = dice.text(9);
            let text = dice.text(30);
            let max = usize::try_from(dice.next(5)).unwrap();
            let scores = slow_best(&text, &pattern);
            let lowest = scores.iter().copied().min().unwrap();
            let written: String = text.iter().collect();
            // The byte offset of each character, then the end.
            let bytes: Vec<usize> = written
                .char_indices()
                .map(|(at, _)| at)
                .chain([written.len()])
                .collect();
            let found = search(&written, &pattern, max);
            if lowest > max {
                assert_eq!(found, Vec::new());
                continue;
            }
            let ends: Vec<usize> = (0..scores.len())
                .filter(|&at| scores[at] == lowest)
                .map(|at| bytes[at + 1])
                .collect();
            assert_eq!(found.iter().map(|f| f.end).collect::<Vec<_>>(), ends);
            for f in found {
                assert_eq!(f.errors, lowest);
                // The earliest start that gives the same count.
                let end = bytes.iter().position(|&at| at == f.end).unwrap();
                let best = (0..end)
                    .find(|&start| distance(&text[start..end], &pattern) == lowest)
                    .unwrap();
                assert_eq!(f.start, bytes[best], "{pattern:?} in {text:?}");
            }
        }
    }

    #[test]
    fn exact_and_near_matches_are_found_with_their_starts() {
        let text = "the cat sat on the mat";
        assert_eq!(
            search(text, &chars("the"), 1),
            vec![
                Found {
                    start: 0,
                    end: 3,
                    errors: 0
                },
                Found {
                    start: 15,
                    end: 18,
                    errors: 0
                }
            ]
        );
        assert_eq!(
            search(text, &chars("sit on"), 2),
            vec![Found {
                start: 8,
                end: 14,
                errors: 1
            }]
        );
        assert_eq!(search(text, &chars("dog"), 1), Vec::new());
        assert_eq!(search(text, &[], 3), Vec::new());
        assert_eq!(search("", &chars("cat"), 3), Vec::new());
        // Offsets are bytes: "é" and "€" take two and three.
        assert_eq!(
            search("é€ cat", &chars("cut"), 1),
            vec![Found {
                start: 6,
                end: 9,
                errors: 1
            }]
        );
        assert_eq!(
            search("x é€ y", &chars("a€"), 1),
            vec![Found {
                start: 2,
                end: 7,
                errors: 1
            }]
        );
    }

    #[test]
    fn a_match_is_as_long_as_it_can_be() {
        // "xb" and "b" both end at 2 with one edit for "ab"; the longer wins.
        assert_eq!(
            search("xb", &chars("ab"), 1),
            vec![Found {
                start: 0,
                end: 2,
                errors: 1
            }]
        );
        // A deleted character: "abd" against "abcd" is one edit, and the
        // match keeps all three characters.
        assert_eq!(
            search("zzabdzz", &chars("abcd"), 2),
            vec![Found {
                start: 2,
                end: 5,
                errors: 1
            }]
        );
        // An added character: the match takes the five characters.
        assert_eq!(
            search("zzabXcdzz", &chars("abcd"), 2),
            vec![Found {
                start: 2,
                end: 7,
                errors: 1
            }]
        );
    }

    #[test]
    fn long_patterns_match_across_blocks() {
        let pattern: String = (0..150_u8).map(|i| char::from(b'a' + i % 26)).collect();
        let mut edited = pattern.clone();
        edited.replace_range(70..71, "#");
        let text = format!("noise {edited} noise");
        let found = search(&text, &chars(&pattern), 10);
        assert_eq!(
            found,
            vec![Found {
                start: 6,
                end: 156,
                errors: 1
            }]
        );
    }

    #[test]
    fn the_fewest_errors_of_a_stretch() {
        assert_eq!(fewest_errors(&chars("prefix here"), &chars("fix")), 0);
        assert_eq!(fewest_errors(&chars("prefix here"), &chars("fox")), 1);
        assert_eq!(fewest_errors(&chars("zz"), &chars("abc")), 3);
        assert_eq!(fewest_errors(&[], &chars("abc")), 3);
        assert_eq!(fewest_errors(&chars("abc"), &[]), 0);
    }
}
