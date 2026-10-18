//! Pairing the blocks of two versions of one document.
//!
//! 1. **Skeleton.** The blocks whose content and nesting are equal are lined
//!    up in order. These pairs are unchanged, or changed in formatting only.
//!    The stretches between two skeleton pairs are the gaps.
//! 2. **Equal content elsewhere.** A block whose content hash equals a block
//!    still unpaired on the other side pairs with it, preferring one in the
//!    same gap. Out of order, that is a move.
//! 3. **Front matter** pairs with front matter: a document has at most one.
//! 4. **Table rows by their first cell**, within one gap.
//! 5. **The lone pair.** A gap with exactly one unpaired block on each side,
//!    of the same kind, pairs them: it is the same block, edited.
//! 6. **Shared words.** The rest pair by the words they share, counted in any
//!    order, through an index of words. Only blocks of the same kind pair.
//!
//! Last, the pairs inside each gap that run in the same order on both sides
//! stay in place; the others, and every pair across gaps, moved.

use std::collections::HashMap;
use std::mem::{Discriminant, discriminant};
use std::sync::Arc;

use super::align::{align, increasing};
use crate::document::{Block, BlockKind, Container};

/// The share of distinct words two blocks in the same gap must share to pair,
/// as twice the shared words over both blocks' word counts (the Dice
/// coefficient). Below it, they show as one removed and one added block.
///
/// 0.4 sits in the 35 to 40 % word overlap that other Markdown diff tools
/// use to match blocks. It pairs `Pages expire after one hour.` with `Pages
/// expire after two hours.` (0.6) and leaves `The cache keeps pages.` and
/// `The server logs requests.` apart (0.25).
pub const MIN_SHARED: f64 = 0.4;

/// The share two blocks in different gaps must share to pair. A pair across
/// gaps claims that a block moved and was edited, so it needs more evidence.
pub const MIN_SHARED_MOVED: f64 = 0.6;

/// Words that occur in more than this many unpaired new blocks are not used
/// to find candidates, because comparing every block that holds `the` with
/// every other would compare all pairs again. Blocks are still scored on all
/// their words once they are candidates. A document with at most this many
/// unpaired new blocks is compared in full.
pub const MAX_POSTING: usize = 64;

/// How two versions' blocks pair up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Pairing {
    /// For each old block, the new block it pairs with.
    pub(crate) old_to_new: Vec<Option<usize>>,
    /// For each new block, the old block it pairs with.
    pub(crate) new_to_old: Vec<Option<usize>>,
    /// For each old block, whether its pair stayed in place.
    pub(crate) in_place: Vec<bool>,
}

/// Which blocks may pair: the same kind, and for table rows the same role,
/// header or body.
type Class = (Discriminant<BlockKind>, bool);

fn class(kind: &BlockKind) -> Class {
    let header = matches!(kind, BlockKind::TableRow { header: true, .. });
    (discriminant(kind), header)
}

/// Pairs the blocks of two versions.
pub(crate) fn pair(old: &[Block], new: &[Block]) -> Pairing {
    let key = |block: &Block| (block.content_hash, Arc::clone(&block.containers));
    let old_keys: Vec<(u64, Arc<[Container]>)> = old.iter().map(key).collect();
    let new_keys: Vec<(u64, Arc<[Container]>)> = new.iter().map(key).collect();
    let skeleton = align(&old_keys, &new_keys);
    let mut pairing = Pairing {
        old_to_new: vec![None; old.len()],
        new_to_old: vec![None; new.len()],
        in_place: vec![false; old.len()],
    };
    for &(o, n) in &skeleton {
        pairing.link(o, n);
        pairing.in_place[o] = true;
    }
    let old_gaps = gaps(old.len(), skeleton.iter().map(|&(o, _)| o));
    let new_gaps = gaps(new.len(), skeleton.iter().map(|&(_, n)| n));
    let sides = Sides {
        old,
        new,
        old_gaps: &old_gaps,
        new_gaps: &new_gaps,
    };
    sides.equal_content(&mut pairing);
    sides.front_matter(&mut pairing);
    sides.first_cells(&mut pairing);
    sides.lone_pairs(&mut pairing);
    sides.shared_words(&mut pairing);
    sides.settle(&mut pairing, &skeleton);
    pairing
}

impl Pairing {
    fn link(&mut self, old: usize, new: usize) {
        self.old_to_new[old] = Some(new);
        self.new_to_old[new] = Some(old);
    }
}

/// For each of `count` blocks, the gap it sits in: the number of skeleton
/// pairs before it. A skeleton block gets the gap after it, which no other
/// step reads.
fn gaps(count: usize, skeleton: impl Iterator<Item = usize>) -> Vec<usize> {
    let mut out = vec![0; count];
    let mut next = skeleton.peekable();
    let mut gap = 0;
    for (index, slot) in out.iter_mut().enumerate() {
        if next.peek() == Some(&index) {
            next.next();
            gap += 1;
        }
        *slot = gap;
    }
    out
}

/// The old blocks no step has paired yet, in order.
fn unpaired_old(pairing: &Pairing) -> impl Iterator<Item = usize> + '_ {
    (0..pairing.old_to_new.len()).filter(|&o| pairing.old_to_new[o].is_none())
}

/// The new blocks no step has paired yet, in order.
fn unpaired_new(pairing: &Pairing) -> impl Iterator<Item = usize> + '_ {
    (0..pairing.new_to_old.len()).filter(|&n| pairing.new_to_old[n].is_none())
}

struct Sides<'d> {
    old: &'d [Block],
    new: &'d [Block],
    old_gaps: &'d [usize],
    new_gaps: &'d [usize],
}

impl Sides<'_> {
    /// Step 2: equal content hashes, in order, preferring the same gap.
    fn equal_content(&self, pairing: &mut Pairing) {
        let mut waiting: HashMap<u64, Vec<usize>> = HashMap::new();
        for o in unpaired_old(pairing) {
            waiting.entry(self.old[o].content_hash).or_default().push(o);
        }
        let new_unpaired: Vec<usize> = unpaired_new(pairing).collect();
        for n in new_unpaired {
            let Some(candidates) = waiting.get_mut(&self.new[n].content_hash) else {
                continue;
            };
            // Looking a few candidates ahead is enough; a file with thousands
            // of equal blocks then costs no more than one with a few.
            let same_gap = candidates
                .iter()
                .take(MAX_POSTING)
                .position(|&o| self.old_gaps[o] == self.new_gaps[n]);
            if let Some(at) = same_gap.or((!candidates.is_empty()).then_some(0)) {
                let o = candidates.remove(at);
                pairing.link(o, n);
            }
        }
    }

    /// Step 3: the front matter of both sides.
    fn front_matter(&self, pairing: &mut Pairing) {
        let is_front = |block: &Block| matches!(block.kind, BlockKind::FrontMatter);
        let old = unpaired_old(pairing).find(|&o| is_front(&self.old[o]));
        let new = unpaired_new(pairing).find(|&n| is_front(&self.new[n]));
        if let (Some(o), Some(n)) = (old, new) {
            pairing.link(o, n);
        }
    }

    /// Step 4: table rows of one gap whose first cells are equal, when that
    /// first cell is written once among the gap's unpaired rows on each side.
    fn first_cells(&self, pairing: &mut Pairing) {
        type Rows = HashMap<(usize, bool, String), (Vec<usize>, Vec<usize>)>;
        let mut rows: Rows = HashMap::new();
        let first_cell = |block: &Block| match &block.kind {
            BlockKind::TableRow { header, cells } => cells
                .first()
                .filter(|cell| !cell.is_empty())
                .map(|cell| (*header, cell.clone())),
            _ => None,
        };
        for o in unpaired_old(pairing) {
            if let Some((header, cell)) = first_cell(&self.old[o]) {
                let key = (self.old_gaps[o], header, cell);
                rows.entry(key).or_default().0.push(o);
            }
        }
        for n in unpaired_new(pairing) {
            if let Some((header, cell)) = first_cell(&self.new[n]) {
                let key = (self.new_gaps[n], header, cell);
                rows.entry(key).or_default().1.push(n);
            }
        }
        let mut found: Vec<(usize, usize)> = rows
            .into_values()
            .filter_map(|(old, new)| match (old.as_slice(), new.as_slice()) {
                (&[o], &[n]) => Some((o, n)),
                _ => None,
            })
            .collect();
        found.sort_unstable();
        for (o, n) in found {
            pairing.link(o, n);
        }
    }

    /// Step 5: a gap whose only unpaired blocks are one old and one new block
    /// of the same kind.
    fn lone_pairs(&self, pairing: &mut Pairing) {
        let mut by_gap: HashMap<usize, (Vec<usize>, Vec<usize>)> = HashMap::new();
        for o in unpaired_old(pairing) {
            by_gap.entry(self.old_gaps[o]).or_default().0.push(o);
        }
        for n in unpaired_new(pairing) {
            by_gap.entry(self.new_gaps[n]).or_default().1.push(n);
        }
        let mut found: Vec<(usize, usize)> = by_gap
            .into_values()
            .filter_map(|(old, new)| match (old.as_slice(), new.as_slice()) {
                (&[o], &[n]) if class(&self.old[o].kind) == class(&self.new[n].kind) => {
                    Some((o, n))
                }
                _ => None,
            })
            .collect();
        found.sort_unstable();
        for (o, n) in found {
            pairing.link(o, n);
        }
    }

    /// Step 6: pairs by shared words, best first.
    fn shared_words(&self, pairing: &mut Pairing) {
        let mut words = Words::default();
        let old: Vec<(usize, Vec<u32>)> = unpaired_old(pairing)
            .map(|o| (o, words.of(&self.old[o].text)))
            .filter(|(_, ids)| !ids.is_empty())
            .collect();
        let new: Vec<(usize, Vec<u32>)> = unpaired_new(pairing)
            .map(|n| (n, words.of(&self.new[n].text)))
            .filter(|(_, ids)| !ids.is_empty())
            .collect();
        let mut postings: HashMap<u32, Vec<usize>> = HashMap::new();
        for (position, (_, ids)) in new.iter().enumerate() {
            for &id in ids {
                postings.entry(id).or_default().push(position);
            }
        }
        let mut candidates: Vec<Candidate> = Vec::new();
        let mut seen: Vec<usize> = Vec::new();
        let mut marked = vec![false; new.len()];
        for (o, old_ids) in &old {
            let o = *o;
            let old_class = class(&self.old[o].kind);
            for id in old_ids {
                let Some(list) = postings.get(id) else {
                    continue;
                };
                if list.len() > MAX_POSTING {
                    continue;
                }
                for &position in list {
                    if !marked[position] {
                        marked[position] = true;
                        seen.push(position);
                    }
                }
            }
            for position in seen.drain(..) {
                marked[position] = false;
                let (n, new_ids) = (new[position].0, &new[position].1);
                if class(&self.new[n].kind) != old_class {
                    continue;
                }
                let score = dice(old_ids, new_ids);
                let same_gap = self.old_gaps[o] == self.new_gaps[n];
                let needed = if same_gap {
                    MIN_SHARED
                } else {
                    MIN_SHARED_MOVED
                };
                if score >= needed {
                    candidates.push(Candidate {
                        score,
                        same_gap,
                        distance: o.abs_diff(n),
                        old: o,
                        new: n,
                    });
                }
            }
        }
        candidates.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then(b.same_gap.cmp(&a.same_gap))
                .then(a.distance.cmp(&b.distance))
                .then(a.old.cmp(&b.old))
                .then(a.new.cmp(&b.new))
        });
        for candidate in candidates {
            if pairing.old_to_new[candidate.old].is_none()
                && pairing.new_to_old[candidate.new].is_none()
            {
                pairing.link(candidate.old, candidate.new);
            }
        }
    }

    /// Last: which pairs stayed in place. Skeleton pairs did. Inside each gap,
    /// the longest run of pairs in the same order on both sides did. Every
    /// other pair moved.
    fn settle(&self, pairing: &mut Pairing, skeleton: &[(usize, usize)]) {
        let mut by_gap: HashMap<usize, Vec<(usize, usize)>> = HashMap::new();
        let in_skeleton: Vec<bool> = {
            let mut marks = vec![false; self.old.len()];
            for &(o, _) in skeleton {
                marks[o] = true;
            }
            marks
        };
        for (o, partner) in pairing.old_to_new.iter().enumerate() {
            if let Some(n) = *partner
                && !in_skeleton[o]
                && self.old_gaps[o] == self.new_gaps[n]
            {
                by_gap.entry(self.old_gaps[o]).or_default().push((o, n));
            }
        }
        for pairs in by_gap.into_values() {
            let order: Vec<usize> = pairs.iter().map(|&(_, n)| n).collect();
            for index in increasing(&order) {
                pairing.in_place[pairs[index].0] = true;
            }
        }
    }
}

/// A possible pair found by shared words.
struct Candidate {
    score: f64,
    same_gap: bool,
    distance: usize,
    old: usize,
    new: usize,
}

/// Word ids, shared by both sides of one comparison.
#[derive(Default)]
struct Words {
    ids: HashMap<String, u32>,
}

impl Words {
    /// The distinct words of a text, as sorted ids. A word is a run of letters
    /// and digits, compared in lower case.
    fn of(&mut self, text: &str) -> Vec<u32> {
        let mut ids: Vec<u32> = text
            .split(|c: char| !c.is_alphanumeric())
            .filter(|word| !word.is_empty())
            .map(|word| {
                let next = u32::try_from(self.ids.len()).unwrap_or(u32::MAX);
                *self.ids.entry(word.to_lowercase()).or_insert(next)
            })
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }
}

/// Twice the shared ids over both lists' lengths. Both lists are sorted and
/// hold each id once.
fn dice(a: &[u32], b: &[u32]) -> f64 {
    let shared = a.iter().filter(|id| b.binary_search(id).is_ok()).count();
    super::words::ratio(2 * shared, a.len() + b.len())
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::*;
    use crate::document::Document;

    fn blocks(source: &str) -> Vec<Block> {
        Document::parse(source.as_bytes().to_vec())
            .expect("parses")
            .blocks
    }

    /// Each old block's partner and whether it stayed in place, written as
    /// `old→new` or `old→new moved`, then the unpaired blocks.
    fn pairs(old: &str, new: &str) -> Vec<String> {
        let pairing = pair(&blocks(old), &blocks(new));
        pairing
            .old_to_new
            .iter()
            .enumerate()
            .map(|(o, partner)| match partner {
                Some(n) if pairing.in_place[o] => format!("{o}→{n}"),
                Some(n) => format!("{o}→{n} moved"),
                None => format!("{o}→none"),
            })
            .collect()
    }

    #[test]
    fn equal_blocks_line_up_in_order() {
        assert_eq!(
            pairs("a\n\nb\n\nc\n", "a\n\nb\n\nc\n"),
            ["0→0", "1→1", "2→2"]
        );
        assert_eq!(pairs("a\n\nc\n", "a\n\nb\n\nc\n"), ["0→0", "1→2"]);
        // Reflowed text keeps its content hash, so it is in the skeleton.
        assert_eq!(pairs("one\ntwo\n", "one two\n"), ["0→0"]);
    }

    #[test]
    fn nesting_is_part_of_the_skeleton_key() {
        // The same item one level deeper pairs by content, in place.
        let pairing = pair(&blocks("- a\n- b\n"), &blocks("- a\n  - b\n"));
        assert_eq!(pairing.old_to_new, [Some(0), Some(1)]);
        assert_eq!(pairing.in_place, [true, true]);
    }

    #[test]
    fn equal_content_out_of_order_is_a_move() {
        assert_eq!(
            pairs(
                "A one.\n\nB two.\n\nC three.\n",
                "B two.\n\nC three.\n\nA one.\n"
            ),
            ["0→2 moved", "1→0", "2→1"]
        );
    }

    #[test]
    fn equal_content_prefers_the_same_gap() {
        // The quoted "same text" has other nesting, so it is not in the
        // skeleton, but its content equals the new block in its gap. The
        // first "same text" comes earlier yet sits in another gap.
        let found = pairs(
            "same text\n\nkeep one\n\n> same text\n\nkeep two\n",
            "keep one\n\nsame text\n\nkeep two\n",
        );
        assert_eq!(found, ["0→none", "1→0", "2→1", "3→2"]);
        // Without a candidate in the same gap, the first one takes it.
        let found = pairs("same\n\nkeep\n\nother\n", "keep\n\nsame\n\nnew\n");
        assert_eq!(found, ["0→1 moved", "1→0", "2→2"]);
    }

    #[test]
    fn a_block_copied_twice_pairs_once() {
        // Both new copies look for the one old block; the second finds none
        // left and stays added.
        let found = pairs(
            "copied text\n\nkeep\n",
            "keep\n\ncopied text\n\ncopied text\n",
        );
        assert_eq!(found, ["0→1 moved", "1→0"]);
    }

    #[test]
    fn front_matter_pairs_with_front_matter() {
        // No words in common, and not alone in its gap, yet it pairs. The
        // paragraph left alone in the gap then pairs too.
        assert_eq!(
            pairs(
                "---\na: 1\n---\n\nintro\n\n# T\n",
                "---\nb: 2\n---\n\nother\n\n# T\n"
            ),
            ["0→0", "1→1", "2→2"]
        );
        // No front matter on the new side: nothing to pair.
        assert_eq!(pairs("---\na: 1\n---\n\n# T\n", "# T\n"), ["0→none", "1→0"]);
    }

    #[test]
    fn table_rows_pair_by_their_first_cell() {
        let found = pairs(
            "| k | v |\n|---|---|\n| home | ten minutes here |\n| away | one |\n",
            "| k | v |\n|---|---|\n| home | 15 seconds there |\n| away | two |\n",
        );
        assert_eq!(found, ["0→0", "1→1", "2→2"]);
        // A first cell written twice on one side is no key.
        let found = pairs(
            "| k | v |\n|---|---|\n| a | x y |\n| a | p q |\n| z |  |\n",
            "| k | v |\n|---|---|\n| a | 1 2 |\n| a | 3 4 |\n| z | 5 |\n",
        );
        assert_eq!(found, ["0→0", "1→none", "2→none", "3→3"]);
        // A header row never pairs with a body row by its first cell.
        let found = pairs("| a | b |\n|---|---|\n", "| x |\n|---|\n| a | c |\n");
        assert_eq!(found, ["0→none"]);
    }

    #[test]
    fn a_lone_pair_in_a_gap_is_one_block_edited() {
        assert_eq!(
            pairs("# T\n\n## Status\n\nend\n", "# T\n\n## State\n\nend\n"),
            ["0→0", "1→1", "2→2"]
        );
        // Of different kinds, they stay apart.
        assert_eq!(
            pairs("# T\n\n## Status\n\nend\n", "# T\n\nState\n\nend\n"),
            ["0→0", "1→none", "2→2"]
        );
    }

    #[test]
    fn shared_words_pair_edited_blocks() {
        let found = pairs(
            "Pages expire after one hour.\n\nThe cache keeps pages.\n",
            "Pages expire after two hours.\n\nThe server logs requests.\n",
        );
        assert_eq!(found, ["0→0", "1→none"]);
    }

    #[test]
    fn a_score_exactly_at_the_limit_pairs() {
        // One word shared out of two and three: 2 × 1 / 5 = 0.4.
        let found = pairs(
            "keep\n\nalpha beta\n\nx y z\n",
            "keep\n\nalpha gamma delta\n\nq\n",
        );
        assert_eq!(found, ["0→0", "1→1", "2→none"]);
        // Across gaps: three words shared out of five and five: 0.6.
        let found = pairs("a b c d e\n\nkept\n", "kept\n\na b c f g\n");
        assert_eq!(found, ["0→1 moved", "1→0"]);
    }

    #[test]
    fn the_best_score_wins() {
        let found = pairs(
            "alpha beta gamma delta\n\nalpha beta x y\n",
            "alpha beta gamma delta epsilon\n\nunrelated words only here\n",
        );
        assert_eq!(found, ["0→0", "1→none"]);
    }

    #[test]
    fn a_pair_across_gaps_needs_more_shared_words() {
        // Moved past a kept block and edited: 3 of 4 words kept (0.75).
        let found = pairs(
            "one two three four\n\nkept\n",
            "kept\n\none two three five\n",
        );
        assert_eq!(found, ["0→1 moved", "1→0"]);
        // Half the words kept (0.5) is enough in the same gap only.
        let found = pairs(
            "one two three four\n\nkept\n",
            "kept\n\none two six seven\n",
        );
        assert_eq!(found, ["0→none", "1→0"]);
        let found = pairs(
            "start\n\none two three four\n\nmid\n\nend\n",
            "start\n\none two six seven\n\nzzz\n\nmid\n\nend\n",
        );
        assert_eq!(found, ["0→0", "1→1", "2→3", "3→4"]);
    }

    #[test]
    fn only_blocks_of_the_same_kind_pair_by_words() {
        let found = pairs(
            "- shared words here\n\nother text\n",
            "shared words here too\n\nmore\n",
        );
        assert_eq!(found, ["0→none", "1→none"]);
    }

    #[test]
    fn swapped_edits_inside_a_gap_keep_one_in_place() {
        let found = pairs(
            "top\n\nfirst block words\n\nsecond block words\n\nbottom\n",
            "top\n\nsecond block words changed\n\nfirst block words changed\n\nbottom\n",
        );
        assert_eq!(found, ["0→0", "1→2 moved", "2→1", "3→3"]);
    }

    #[test]
    fn common_words_do_not_find_candidates_in_long_documents() {
        // The old block shares only "common" with the new blocks, which is
        // enough to pair (0.5) when the word is used.
        let document = |count: usize| {
            let mut new = String::from("anchor\n\n");
            for index in 0..count {
                write!(new, "common w{index}\n\n").expect("writing to a string");
            }
            new
        };
        let old = "anchor\n\ncommon qqq\n";
        assert_eq!(pairs(old, &document(MAX_POSTING + 1)), ["0→0", "1→none"]);
        assert_eq!(pairs(old, &document(MAX_POSTING)), ["0→0", "1→1"]);
    }

    #[test]
    fn blocks_without_words_never_pair_by_words() {
        assert_eq!(
            pairs("—\n\nx\n\n—\n", "–\n\ny\n\n–\n"),
            ["0→none", "1→none", "2→none"]
        );
    }

    #[test]
    fn gaps_count_the_skeleton_pairs_before_each_block() {
        assert_eq!(gaps(5, [1, 3].into_iter()), [0, 1, 1, 2, 2]);
        assert_eq!(gaps(2, std::iter::empty()), [0, 0]);
    }

    #[test]
    fn dice_counts_shared_ids() {
        assert!((dice(&[1, 2, 3], &[2, 3, 4, 5]) - 4.0 / 7.0).abs() < 1e-12);
        assert!((dice(&[1], &[1]) - 1.0).abs() < 1e-12);
        assert!(dice(&[1, 3], &[2, 4]).abs() < 1e-12);
    }

    #[test]
    fn words_are_lower_case_runs_of_letters_and_digits() {
        let mut words = Words::default();
        let first = words.of("The cat, the CAT!");
        assert_eq!(first, [0, 1]);
        assert_eq!(words.of("cat dog"), [1, 2]);
        assert_eq!(words.of("..."), Vec::<u32>::new());
    }
}
