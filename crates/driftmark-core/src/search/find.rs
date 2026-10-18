//! Running a query against one or more indexes, and ranking what it finds.

use std::collections::BTreeMap;

use super::index::{DocInfo, Field, Index};
use super::query::{Clause, Query, Slot};
use crate::git::RepoPath;

/// Which documents a search may return.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filter {
    /// Keep only documents whose path starts with this text, as written:
    /// `docs/decisions/` keeps that folder, `docs/d` also keeps `docs/drafts/`.
    pub path: Option<String>,
    /// Keep only documents of these types from the rules file. Empty keeps
    /// every type. Files that no type matches have the type `document`.
    pub types: Vec<String>,
}

impl Filter {
    fn allows(&self, info: &DocInfo) -> bool {
        let path_ok = self
            .path
            .as_ref()
            .is_none_or(|prefix| info.path.as_bytes().starts_with(prefix.as_bytes()));
        path_ok && (self.types.is_empty() || self.types.contains(&info.doc_type))
    }
}

/// One document a search found.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    /// The index that holds it, by its position in the list searched.
    pub layer: usize,
    /// Its id in that index: its place in the order it was added.
    pub doc: usize,
    /// Its path.
    pub path: RepoPath,
    /// Its title, when it has one.
    pub title: Option<String>,
    /// Its type from the rules file.
    pub doc_type: String,
    /// The block that matches best, by its index in the document's block
    /// list. The snippet comes from it.
    pub block: usize,
    /// Its score; higher ranks first.
    pub score: f64,
}

/// What a search found.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Found {
    /// How many documents were searched.
    pub documents: usize,
    /// How many matched, before the limit.
    pub total: usize,
    /// The best matches, best first, at most the limit.
    pub hits: Vec<Hit>,
}

/// Runs `query` against `layers`, keeps what `filter` allows, and returns
/// at most `limit` documents, best first.
///
/// Several indexes are searched as one. A later index replaces every
/// document of an earlier one that has the same path, so the working tree
/// can keep one large index of a past state and a small one of the files
/// changed since; a path the later index marks as removed hides the earlier
/// document too.
///
/// A document matches when it holds every clause. Its score adds, for each
/// clause, the clause's weight times where its best match stands:
///
/// - the weight is `ln(1 + N / n)`, where `N` documents were searched and
///   `n` of them hold the clause, so a rarer word counts more;
/// - a match in the title counts 4, in a heading 2 and in the body 1, and
///   every further match of the clause adds a little, never as much as 0.5
///   in all, so it cannot lift a body match over a heading match.
///
/// Equal scores rank by path.
#[must_use]
pub fn search(layers: &[&Index], query: &Query, filter: &Filter, limit: usize) -> Found {
    let alive: Vec<Vec<bool>> = layers
        .iter()
        .enumerate()
        .map(|(at, index)| {
            let mut alive: Vec<bool> = index
                .docs
                .iter()
                .map(|slot| slot.live().is_some())
                .collect();
            for later in &layers[at + 1..] {
                for slot in &later.docs {
                    if let Some(id) = index.find(slot.path()) {
                        alive[id] = false;
                    }
                }
            }
            alive
        })
        .collect();
    let documents = alive.iter().flatten().filter(|&&alive| alive).count();
    let mut found = Found {
        documents,
        ..Found::default()
    };
    // For each clause, layer and document: where the clause starts.
    let mut places: Vec<Vec<Vec<Vec<u32>>>> = Vec::new();
    let mut weights: Vec<f64> = Vec::new();
    for clause in &query.clauses {
        let at: Vec<Vec<Vec<u32>>> = layers
            .iter()
            .zip(&alive)
            .map(|(index, alive)| clause_places(index, clause, alive))
            .collect();
        let held = at.iter().flatten().filter(|p| !p.is_empty()).count();
        if held == 0 {
            return found;
        }
        weights.push((1.0 + ratio(documents, held)).ln());
        places.push(at);
    }
    if weights.is_empty() {
        return found;
    }
    let mut scored: Vec<(f64, usize, usize, &DocInfo)> = Vec::new();
    for (layer, index) in layers.iter().enumerate() {
        for (doc, slot) in index.docs.iter().enumerate() {
            let Some(info) = slot.live() else {
                continue;
            };
            if !alive[layer][doc] || !filter.allows(info) {
                continue;
            }
            let score: Option<f64> = weights
                .iter()
                .zip(&places)
                .map(|(weight, at)| {
                    let found = &at[layer][doc];
                    (!found.is_empty()).then(|| {
                        let best = found
                            .iter()
                            .map(|&place| index.block_at(info, place).field)
                            .fold(Field::Body, Ord::max);
                        let more = 0.5 - 0.5 / f64::from(count(found.len()));
                        weight * (best.weight() + more)
                    })
                })
                .sum();
            if let Some(score) = score {
                scored.push((score, layer, doc, info));
            }
        }
    }
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.3.path.cmp(&b.3.path)));
    found.total = scored.len();
    for (score, layer, doc, info) in scored.into_iter().take(limit) {
        let index = layers[layer];
        let at: Vec<&Vec<u32>> = places.iter().map(|at| &at[layer][doc]).collect();
        found.hits.push(Hit {
            layer,
            doc,
            path: info.path.clone(),
            title: info.title.clone(),
            doc_type: info.doc_type.clone(),
            block: best_block(index, info, &at, &weights),
            score,
        });
    }
    found
}

/// `a / b` for counts.
fn ratio(a: usize, b: usize) -> f64 {
    f64::from(count(a)) / f64::from(count(b))
}

fn count(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// Where `clause` starts in each document of `index` that `alive` allows.
/// A phrase starts where its first word stands and each later word stands
/// one place further.
fn clause_places(index: &Index, clause: &Clause, alive: &[bool]) -> Vec<Vec<u32>> {
    let mut starts = slot_places(index, &clause.slots[0], alive);
    for (offset, slot) in clause.slots.iter().enumerate().skip(1) {
        let wanted: Vec<bool> = starts.iter().map(|places| !places.is_empty()).collect();
        let next = slot_places(index, slot, &wanted);
        let offset = count(offset);
        for (starts, next) in starts.iter_mut().zip(&next) {
            starts.retain(|&place| next.binary_search(&(place + offset)).is_ok());
        }
    }
    starts
}

/// Where the words that fill `slot` stand in each document `wanted`
/// allows, sorted.
fn slot_places(index: &Index, slot: &Slot, wanted: &[bool]) -> Vec<Vec<u32>> {
    let mut out = vec![Vec::new(); index.docs.len()];
    let mut places = Vec::new();
    for term in index.terms_for(&slot.word, slot.prefix) {
        let mut postings = index.postings(term);
        while let Some(doc) = postings.next_into(&mut places) {
            if wanted[doc as usize] {
                out[doc as usize].extend_from_slice(&places);
            }
        }
    }
    for places in &mut out {
        places.sort_unstable();
    }
    out
}

/// The block of a matched document that the snippet should show: the one
/// holding the largest weight of distinct clauses. On equal weight a heading
/// wins over the body, then the earlier block. The title is chosen only when
/// no other block matches, because a result shows the title anyway.
fn best_block(index: &Index, info: &DocInfo, at: &[&Vec<u32>], weights: &[f64]) -> usize {
    let mut held: BTreeMap<usize, u64> = BTreeMap::new();
    for (clause, places) in at.iter().enumerate() {
        for &place in *places {
            *held.entry(index.block_number(info, place)).or_default() |= 1 << clause;
        }
    }
    let rank = |&(&number, &mask): &(&usize, &u64)| {
        let field = index.blocks[number].field;
        let weight: f64 = weights
            .iter()
            .enumerate()
            .filter(|&(clause, _)| mask >> clause & 1 == 1)
            .map(|(_, weight)| weight)
            .sum();
        (field != Field::Title, weight, field, number)
    };
    held.iter()
        .max_by(|a, b| {
            let (a, b) = (rank(a), rank(b));
            a.0.cmp(&b.0)
                .then(a.1.total_cmp(&b.1))
                .then(a.2.cmp(&b.2))
                .then(b.3.cmp(&a.3))
        })
        .map_or(0, |(&number, _)| index.blocks[number].index as usize)
}

#[cfg(test)]
mod tests;
