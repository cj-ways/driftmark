//! Lining up two sequences: which items of the old one are kept, in order, in
//! the new one. The block list, the words of a block, the lines of a code
//! block and the sentences of a paragraph are all lined up with this.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::hash::Hash;

/// The largest table the exact alignment fills, in cells (old items times new
/// items, after the common start and end are set aside).
///
/// The table holds one `u32` per cell, so this caps it at 16 MB. The speed
/// spike's largest block table over 30 days of a 3,400-file corpus had 0.78
/// million cells, so real documents stay far below it. Larger inputs are first
/// split at items that occur once on each side, as patience diff does.
pub const MAX_CELLS: usize = 4_000_000;

/// The pairs `(old, new)` of items kept in order, sorted by both indexes.
///
/// The result is a longest common subsequence whenever the table fits in
/// [`MAX_CELLS`]. Otherwise it is built from items that occur once on each
/// side, and each stretch between two of them is lined up again.
pub(crate) fn align<T: Eq + Hash>(old: &[T], new: &[T]) -> Vec<(usize, usize)> {
    align_within(old, new, MAX_CELLS)
}

/// [`align`] with the table size as a parameter, so tests can reach the
/// fallback with small inputs.
pub(crate) fn align_within<T: Eq + Hash>(
    old: &[T],
    new: &[T],
    max_cells: usize,
) -> Vec<(usize, usize)> {
    let mut pairs = Vec::new();
    // Stretches still to line up: old start, old end, new start, new end.
    let mut work = vec![(0, old.len(), 0, new.len())];
    while let Some((mut old_start, mut old_end, mut new_start, mut new_end)) = work.pop() {
        while old_start < old_end && new_start < new_end && old[old_start] == new[new_start] {
            pairs.push((old_start, new_start));
            old_start += 1;
            new_start += 1;
        }
        while old_start < old_end && new_start < new_end && old[old_end - 1] == new[new_end - 1] {
            old_end -= 1;
            new_end -= 1;
            pairs.push((old_end, new_end));
        }
        let old_part = &old[old_start..old_end];
        let new_part = &new[new_start..new_end];
        if old_part.len().saturating_mul(new_part.len()) <= max_cells {
            pairs.extend(
                table(old_part, new_part)
                    .into_iter()
                    .map(|(o, n)| (o + old_start, n + new_start)),
            );
        } else {
            // Each stretch between two anchors is smaller than this one, so
            // the work ends. Without anchors, the stretch stays unaligned.
            let anchors = unique_anchors(old_part, new_part);
            let mut previous = (old_start, new_start);
            for &(o, n) in &anchors {
                let (o, n) = (o + old_start, n + new_start);
                pairs.push((o, n));
                work.push((previous.0, o, previous.1, n));
                previous = (o + 1, n + 1);
            }
            if !anchors.is_empty() {
                work.push((previous.0, old_end, previous.1, new_end));
            }
        }
    }
    pairs.sort_unstable();
    pairs
}

/// A longest common subsequence by the classic table. `lengths[i][j]` holds
/// the length for `old[i..]` and `new[j..]`, so the walk that reads it goes
/// forward and yields the pairs in order.
fn table<T: Eq>(old: &[T], new: &[T]) -> Vec<(usize, usize)> {
    let width = new.len() + 1;
    let mut lengths = vec![0_u32; (old.len() + 1) * width];
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            lengths[i * width + j] = if old[i] == new[j] {
                lengths[(i + 1) * width + j + 1] + 1
            } else {
                lengths[(i + 1) * width + j].max(lengths[i * width + j + 1])
            };
        }
    }
    let mut pairs = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < old.len() && j < new.len() {
        if old[i] == new[j] {
            pairs.push((i, j));
            i += 1;
            j += 1;
        } else if lengths[(i + 1) * width + j] == lengths[i * width + j] {
            // Dropping the old item keeps the length, so a longest common
            // subsequence goes that way. Ties drop the old item first.
            i += 1;
        } else {
            j += 1;
        }
    }
    pairs
}

/// The items that occur exactly once on each side, as `(old, new)` pairs,
/// kept only where they run in the same order on both sides.
fn unique_anchors<T: Eq + Hash>(old: &[T], new: &[T]) -> Vec<(usize, usize)> {
    // Per item: how often it occurs on each side, and where it last did.
    let mut seen: HashMap<&T, (usize, usize, usize, usize)> = HashMap::new();
    for (index, item) in old.iter().enumerate() {
        let entry = seen.entry(item).or_insert((0, 0, 0, 0));
        entry.0 += 1;
        entry.2 = index;
    }
    for (index, item) in new.iter().enumerate() {
        if let Entry::Occupied(mut entry) = seen.entry(item) {
            let counts = entry.get_mut();
            counts.1 += 1;
            counts.3 = index;
        }
    }
    let mut candidates: Vec<(usize, usize)> = seen
        .into_values()
        .filter(|&(in_old, in_new, _, _)| in_old == 1 && in_new == 1)
        .map(|(_, _, o, n)| (o, n))
        .collect();
    candidates.sort_unstable();
    let order: Vec<usize> = candidates.iter().map(|&(_, n)| n).collect();
    increasing(&order)
        .into_iter()
        .map(|index| candidates[index])
        .collect()
}

/// The positions of a longest strictly increasing subsequence of `values`,
/// in order. Among equally long ones it keeps the one that ends lowest at each
/// length, which is patience sorting's choice.
pub(crate) fn increasing(values: &[usize]) -> Vec<usize> {
    // `tails[l]`: the position of the smallest last value of an increasing
    // run of length `l + 1` seen so far.
    let mut tails: Vec<usize> = Vec::new();
    let mut before: Vec<Option<usize>> = Vec::with_capacity(values.len());
    for (position, &value) in values.iter().enumerate() {
        let length = tails.partition_point(|&tail| values[tail] < value);
        before.push(length.checked_sub(1).map(|shorter| tails[shorter]));
        if length == tails.len() {
            tails.push(position);
        } else {
            tails[length] = position;
        }
    }
    let mut run = Vec::with_capacity(tails.len());
    let mut at = tails.last().copied();
    while let Some(position) = at {
        run.push(position);
        at = before[position];
    }
    run.reverse();
    run
}

#[cfg(test)]
mod tests {
    use super::*;

    fn letters(text: &str) -> Vec<char> {
        text.chars().collect()
    }

    /// The length of a longest common subsequence, by brute force over the
    /// table definition, to check the walk against.
    fn lcs_length(old: &[char], new: &[char]) -> usize {
        let mut lengths = vec![vec![0; new.len() + 1]; old.len() + 1];
        for i in 0..old.len() {
            for j in 0..new.len() {
                lengths[i + 1][j + 1] = if old[i] == new[j] {
                    lengths[i][j] + 1
                } else {
                    lengths[i][j + 1].max(lengths[i + 1][j])
                };
            }
        }
        lengths[old.len()][new.len()]
    }

    fn check(old: &str, new: &str, max_cells: usize) -> Vec<(usize, usize)> {
        let (old, new) = (letters(old), letters(new));
        let pairs = align_within(&old, &new, max_cells);
        for window in pairs.windows(2) {
            assert!(window[0].0 < window[1].0 && window[0].1 < window[1].1);
        }
        for &(o, n) in &pairs {
            assert_eq!(old[o], new[n]);
        }
        pairs
    }

    #[test]
    fn the_table_finds_a_longest_common_subsequence() {
        assert_eq!(check("abcabba", "cbabac", MAX_CELLS).len(), 4);
        assert_eq!(check("", "abc", MAX_CELLS), []);
        assert_eq!(check("abc", "", MAX_CELLS), []);
        assert_eq!(check("abc", "abc", MAX_CELLS), [(0, 0), (1, 1), (2, 2)]);
        // When both choices keep as much, the walk drops the old item first.
        assert_eq!(check("ab", "ba", MAX_CELLS), [(1, 0)]);
        assert_eq!(check("xaby", "xbay", MAX_CELLS), [(0, 0), (2, 1), (3, 3)]);
        assert_eq!(check("ab", "ca", MAX_CELLS), [(0, 1)]);
        assert_eq!(check("ba", "ac", MAX_CELLS), [(1, 0)]);
    }

    #[test]
    fn the_table_matches_the_brute_force_length() {
        let samples = [
            ("kitten", "sitting"),
            ("aaaa", "aa"),
            ("abcdefg", "gfedcba"),
            ("mississippi", "missouri"),
            ("abab", "baba"),
            ("xyxyxzz", "zzxyyx"),
        ];
        for (old, new) in samples {
            let pairs = check(old, new, MAX_CELLS);
            assert_eq!(pairs.len(), lcs_length(&letters(old), &letters(new)));
        }
    }

    #[test]
    fn the_common_start_and_end_are_set_aside_first() {
        // With no room for a table, only the common ends line up.
        assert_eq!(check("abXcd", "abYcd", 0), [(0, 0), (1, 1), (3, 3), (4, 4)]);
        // The common end is set aside even when its items are not unique.
        assert_eq!(check("abXcc", "abYcc", 0), [(0, 0), (1, 1), (3, 3), (4, 4)]);
        assert_eq!(check("ab", "ab", 0), [(0, 0), (1, 1)]);
    }

    #[test]
    fn large_stretches_split_at_unique_items() {
        // "q" is the only item unique on both sides. It anchors the rest, and
        // each side of it is small enough for the table again.
        let pairs = check("aabqbba", "abaqbab", 9);
        assert_eq!(pairs, [(0, 0), (2, 1), (3, 3), (4, 4), (6, 5)]);
        // Anchors can cost pairs: "q" and "z" cross, so one is kept, and the
        // three "a" on either side of it can no longer line up.
        assert_eq!(check("qaaaz", "zaaaq", 3), [(4, 0)]);
        assert_eq!(check("qaaaz", "zaaaq", 25).len(), 3);
        // Without unique items, a stretch too large stays unaligned.
        assert_eq!(check("aabb", "bbaa", 3), []);
        // Exactly at the limit the table still runs.
        assert_eq!(check("aabb", "bbaa", 16).len(), 2);
        assert_eq!(check("aabb", "bbaa", 15), []);
        // Unique items that cross are not all kept.
        assert_eq!(check("xyzw", "zwxy", 1), [(2, 0), (3, 1)]);
    }

    #[test]
    fn an_item_unique_on_one_side_only_is_no_anchor() {
        assert_eq!(unique_anchors(&letters("aqa"), &letters("qq")), []);
        assert_eq!(unique_anchors(&letters("qq"), &letters("aqa")), []);
        assert_eq!(unique_anchors(&letters("aq"), &letters("qb")), [(1, 0)]);
        assert_eq!(unique_anchors(&letters("ab"), &letters("cd")), []);
        assert_eq!(
            unique_anchors(&letters("abc"), &letters("cab")),
            [(0, 1), (1, 2)]
        );
    }

    #[test]
    fn increasing_runs_are_longest_and_lowest() {
        assert_eq!(increasing(&[]), Vec::<usize>::new());
        assert_eq!(increasing(&[5]), [0]);
        assert_eq!(increasing(&[3, 1, 2]), [1, 2]);
        assert_eq!(increasing(&[2, 2, 2]), [2]);
        assert_eq!(increasing(&[0, 8, 4, 12, 2, 10, 6, 14, 1, 9]), [0, 4, 6, 9]);
        assert_eq!(increasing(&[1, 2, 3]), [0, 1, 2]);
        assert_eq!(increasing(&[3, 2, 1]), [2]);
    }
}
