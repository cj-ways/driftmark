//! Which changes of a range a commit made, so a change page can say
//! "changed because of note n12" on the cards of the commits recorded on a
//! note.
//!
//! A range's change list compares its two ends, its base and its head. The
//! commit sits somewhere between them, and later commits may have moved or
//! edited what it wrote. So the commit's own changes are found first, by
//! comparing the document before and after it, and then followed to the
//! range's ends with the same block pairing the change list uses:
//!
//! - a block the commit wrote, edited or moved is followed forward, from
//!   the version after the commit to the head;
//! - a block the commit deleted is followed back, from the version before
//!   the commit to the base.
//!
//! A change of the range is the commit's when its new block is one the
//! commit wrote, or its old block is one the commit deleted. A new or
//! deleted document is one change, and it is the commit's when the commit
//! changed any block of it.

use std::collections::HashSet;

use crate::diff::{DocumentChanges, Subject, block_map};
use crate::document::Document;

/// One document around a commit: before it, at the commit's first parent,
/// and after it. `None` where the document does not exist.
#[derive(Debug, Clone, Copy)]
pub struct Around<'a> {
    /// The document at the commit's first parent.
    pub before: Option<&'a Document>,
    /// The document at the commit.
    pub after: Option<&'a Document>,
}

/// The same document at a range's two ends. `None` where it does not exist.
#[derive(Debug, Clone, Copy)]
pub struct Ends<'a> {
    /// The document at the range's base.
    pub base: Option<&'a Document>,
    /// The document at the range's head.
    pub head: Option<&'a Document>,
}

/// The indices of the changes in `changes`, the document's change list for
/// a range, that the commit made, in list order.
///
/// `ends` must be the versions `changes` compared, parsed the same way, so
/// block indices agree.
#[must_use]
pub fn caused(changes: &DocumentChanges, commit: Around<'_>, ends: Ends<'_>) -> Vec<usize> {
    let (wrote, deleted) = touched(commit);
    if wrote.is_empty() && deleted.is_empty() {
        return Vec::new();
    }
    let at_head = forward(commit.after, ends.head, &wrote);
    let at_base = back(ends.base, commit.before, &deleted);
    changes
        .changes
        .iter()
        .enumerate()
        .filter(|(_, change)| match &change.subject {
            Subject::Document { .. } => true,
            Subject::Block { old, new } => {
                (new.block.is_some() && at_head.contains(&new.at))
                    || (old.block.is_some() && at_base.contains(&old.at))
            }
        })
        .map(|(index, _)| index)
        .collect()
}

/// The blocks a commit wrote, by their index after it, and the blocks it
/// deleted, by their index before it. A block it left as it was, in its
/// place, is in neither.
fn touched(commit: Around<'_>) -> (HashSet<usize>, HashSet<usize>) {
    match (commit.before, commit.after) {
        (None, None) => (HashSet::new(), HashSet::new()),
        (None, Some(after)) => ((0..after.blocks.len()).collect(), HashSet::new()),
        (Some(before), None) => (HashSet::new(), (0..before.blocks.len()).collect()),
        (Some(before), Some(after)) => {
            let map = block_map(before, after);
            let mut wrote: HashSet<usize> = (0..after.blocks.len()).collect();
            let mut deleted = HashSet::new();
            for (old, partner) in map.old_to_new.iter().enumerate() {
                match partner {
                    Some(new)
                        if map.in_place[old]
                            && before.blocks[old].source_hash == after.blocks[*new].source_hash =>
                    {
                        wrote.remove(new);
                    }
                    Some(_) => {}
                    None => {
                        deleted.insert(old);
                    }
                }
            }
            (wrote, deleted)
        }
    }
}

/// Where the blocks `wrote` of the version after the commit are at the
/// head, by the head's block indices.
fn forward(
    after: Option<&Document>,
    head: Option<&Document>,
    wrote: &HashSet<usize>,
) -> HashSet<usize> {
    let (Some(after), Some(head)) = (after, head) else {
        return HashSet::new();
    };
    let map = block_map(after, head);
    wrote
        .iter()
        .filter_map(|&block| map.old_to_new[block])
        .collect()
}

/// Which blocks of the base became the blocks `deleted` of the version
/// before the commit, by the base's block indices.
fn back(
    base: Option<&Document>,
    before: Option<&Document>,
    deleted: &HashSet<usize>,
) -> HashSet<usize> {
    let (Some(base), Some(before)) = (base, before) else {
        return HashSet::new();
    };
    block_map(base, before)
        .old_to_new
        .iter()
        .enumerate()
        .filter(|(_, partner)| partner.is_some_and(|partner| deleted.contains(&partner)))
        .map(|(block, _)| block)
        .collect()
}

#[cfg(test)]
mod tests;
