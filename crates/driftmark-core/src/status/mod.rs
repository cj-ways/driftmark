//! The status view: which documents are current, superseded, archived or
//! untouched for a long time, by document type.
//!
//! The program reads every document at one revision, classifies it with the
//! rules file, and finds the last commit that changed it. This module turns
//! those facts into the view: counts per type and lifecycle, the document
//! that replaced each superseded one, and how many days ago each document
//! last changed. It reads no clock and no git: the program passes the time
//! now and the commit times.
//!
//! "Untouched for long" is a fact shown beside a document, never a problem:
//! nothing here sorts or ranks by it. A decision record that nobody changed
//! for a year may be exactly right.
//!
//! `docs/features/status-page.md` states the contract.

mod head;
mod successor;

pub use head::needed;

use std::collections::HashMap;

use crate::git::ObjectId;
use crate::rules::{Classification, DEFAULT_TYPE, Lifecycle, Rules};

/// Seconds in a day.
const DAY: i64 = 86_400;

/// One document as the program read it, before the view is built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Its path from the repository root, with `/` between folders.
    pub path: String,
    /// Its title, from its metadata.
    pub title: Option<String>,
    /// Its type from the rules file.
    pub doc_type: String,
    /// Its status as written.
    pub status: Option<String>,
    /// The targets of the links in its status.
    pub status_links: Vec<String>,
    /// What its status means.
    pub lifecycle: Option<Lifecycle>,
    /// Its date as written.
    pub date: Option<String>,
    /// The text naming the document that replaced it.
    pub superseded_by: Option<String>,
    /// The targets of the links in that text.
    pub superseded_by_links: Vec<String>,
}

impl Entry {
    /// The entry of the document at `path`, from its title and what the
    /// rules say about it. The decision sections are not kept.
    #[must_use]
    pub fn new(path: String, title: Option<String>, class: Classification) -> Entry {
        Entry {
            path,
            title,
            doc_type: class.doc_type,
            status: class.status,
            status_links: class.status_links,
            lifecycle: class.lifecycle,
            date: class.date,
            superseded_by: class.superseded_by,
            superseded_by_links: class.superseded_by_links,
        }
    }
}

/// When a document last changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Touched {
    /// The last commit that changed it, along first parents, and that
    /// commit's committer date in seconds since 1970-01-01 00:00 UTC.
    Committed {
        /// The commit.
        commit: ObjectId,
        /// When it was committed.
        at: i64,
    },
    /// The working tree holds a version no commit has: a new file, or one
    /// edited and not committed yet. Someone is working on it now.
    Uncommitted,
}

/// What became of a document's "superseded by".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Successor {
    /// The text that names the newer document, as written; `None` when the
    /// document says it is superseded but not by what.
    pub text: Option<String>,
    /// Which document that is, or why none was found.
    pub target: Target,
}

/// The document a "superseded by" leads to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// This document, by its index in [`View::documents`].
    Found(usize),
    /// This document, but following "superseded by" from it leads back to
    /// the first one, so neither is folded under the other.
    Circle(usize),
    /// A link or a file name names this path, which no document at this
    /// revision has.
    Missing(String),
    /// The text names no document driftmark can find: no link, no file
    /// name, no title and no number leads to one.
    Unmatched,
    /// The document says it is superseded, but not by what.
    Unnamed,
}

/// One document in the view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Doc {
    /// Its path from the repository root.
    pub path: String,
    /// Its title.
    pub title: Option<String>,
    /// Its type.
    pub doc_type: String,
    /// Its status as written.
    pub status: Option<String>,
    /// What its status means.
    pub lifecycle: Option<Lifecycle>,
    /// Its date as written.
    pub date: Option<String>,
    /// The document that replaced it, when it names one or says it is
    /// superseded.
    pub successor: Option<Successor>,
    /// The superseded documents it replaced, by index, sorted by path:
    /// those whose [`Target::Found`] leads here. The page folds them under
    /// this one.
    pub replaces: Vec<usize>,
    /// When it last changed.
    pub touched: Touched,
    /// Whole days since its last commit; `None` when it is uncommitted.
    pub days: Option<i64>,
    /// Whether that is at least the rules file's `untouched-days`.
    pub untouched: bool,
}

impl Doc {
    /// The folder the document sits in, ending in `/`, or the empty text
    /// at the repository root.
    #[must_use]
    pub fn folder(&self) -> &str {
        folder_of(&self.path)
    }

    /// The document that replaced this one, when it was found and is not
    /// part of a circle: the one it folds under.
    #[must_use]
    pub fn folds_under(&self) -> Option<usize> {
        match (self.lifecycle, &self.successor) {
            (
                Some(Lifecycle::Superseded),
                Some(Successor {
                    target: Target::Found(index),
                    ..
                }),
            ) => Some(*index),
            _ => None,
        }
    }
}

/// The folder of a path, ending in `/`, or the empty text at the root.
fn folder_of(path: &str) -> &str {
    path.rfind('/').map_or("", |at| &path[..=at])
}

/// How many documents of one kind are in each lifecycle.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    /// Every document.
    pub documents: usize,
    /// Those whose lifecycle is current.
    pub current: usize,
    /// Those whose lifecycle is superseded.
    pub superseded: usize,
    /// Those whose lifecycle is archived.
    pub archived: usize,
    /// Those with no lifecycle.
    pub none: usize,
    /// Those untouched for at least the rules file's days, whatever their
    /// lifecycle.
    pub untouched: usize,
}

impl Counts {
    fn add(&mut self, doc: &Doc) {
        self.documents += 1;
        match doc.lifecycle {
            Some(Lifecycle::Current) => self.current += 1,
            Some(Lifecycle::Superseded) => self.superseded += 1,
            Some(Lifecycle::Archived) => self.archived += 1,
            None => self.none += 1,
        }
        self.untouched += usize::from(doc.untouched);
    }

    /// The count of one stage.
    #[must_use]
    pub fn of(&self, stage: Stage) -> usize {
        match stage {
            Stage::Current => self.current,
            Stage::Superseded => self.superseded,
            Stage::Archived => self.archived,
            Stage::None => self.none,
        }
    }
}

/// The documents of one type, counted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeCounts {
    /// The type's name.
    pub name: String,
    /// Its documents, by lifecycle.
    pub counts: Counts,
}

/// A lifecycle as a filter can ask for it: one of the three, or none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// The document says what is true now.
    Current,
    /// Another document replaced it.
    Superseded,
    /// It is kept only as a record.
    Archived,
    /// Its status, or its type, gives no lifecycle.
    None,
}

impl Stage {
    /// Every stage, in the order the view shows them.
    pub const ALL: [Stage; 4] = [
        Stage::Current,
        Stage::Superseded,
        Stage::Archived,
        Stage::None,
    ];

    /// The stage of a lifecycle.
    #[must_use]
    pub fn of(lifecycle: Option<Lifecycle>) -> Stage {
        match lifecycle {
            Some(Lifecycle::Current) => Stage::Current,
            Some(Lifecycle::Superseded) => Stage::Superseded,
            Some(Lifecycle::Archived) => Stage::Archived,
            None => Stage::None,
        }
    }

    /// Its name in addresses, JSON and filters: `current`, `superseded`,
    /// `archived` or `none`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Stage::Current => "current",
            Stage::Superseded => "superseded",
            Stage::Archived => "archived",
            Stage::None => "none",
        }
    }

    /// The stage a name means, ignoring case and spaces around it.
    #[must_use]
    pub fn parse(name: &str) -> Option<Stage> {
        let name = name.trim();
        Stage::ALL
            .into_iter()
            .find(|stage| stage.name().eq_ignore_ascii_case(name))
    }
}

/// Which documents a view lists. Every part that is set must match; an
/// empty filter lists every document.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filter {
    /// Keep only these types; none means every type.
    pub types: Vec<String>,
    /// Keep only this stage.
    pub stage: Option<Stage>,
    /// Keep only documents whose path starts with this text, as written:
    /// `docs/decisions/` keeps that folder.
    pub folder: Option<String>,
    /// Keep only documents untouched for at least the rules file's days.
    pub untouched: bool,
}

/// The rows of one type: the documents a filter lists that fold under no
/// other listed document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    /// The type's name.
    pub name: String,
    /// The documents, by index, sorted by path. Each carries the documents
    /// it replaced in [`Doc::replaces`].
    pub rows: Vec<usize>,
}

/// The status view of every document at one revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    /// After how many days without a commit a document counts as untouched.
    pub untouched_days: u32,
    /// Every document, sorted by path.
    pub documents: Vec<Doc>,
    /// The types that have documents, in the order of the rules file, then
    /// any other type by name, then [`DEFAULT_TYPE`].
    pub types: Vec<TypeCounts>,
    /// Every document, counted.
    pub totals: Counts,
}

impl View {
    /// Builds the view of `entries`, read with `rules`, at the time `now` in
    /// seconds since 1970-01-01 00:00 UTC.
    #[must_use]
    pub fn build(mut entries: Vec<(Entry, Touched)>, rules: &Rules, now: i64) -> View {
        entries.sort_by(|a, b| a.0.path.cmp(&b.0.path));
        let untouched_days = rules.untouched_days();
        let index = successor::Index::new(entries.iter().map(|(entry, _)| entry));
        let successors: Vec<Option<Successor>> = entries
            .iter()
            .map(|(entry, _)| successor::find(entry, &index))
            .collect();
        let mut documents: Vec<Doc> = entries
            .into_iter()
            .zip(successors)
            .map(|((entry, touched), successor)| {
                let days = match &touched {
                    Touched::Committed { at, .. } => Some((now - at).div_euclid(DAY).max(0)),
                    Touched::Uncommitted => None,
                };
                Doc {
                    path: entry.path,
                    title: entry.title,
                    doc_type: entry.doc_type,
                    status: entry.status,
                    lifecycle: entry.lifecycle,
                    date: entry.date,
                    successor,
                    replaces: Vec::new(),
                    touched,
                    days,
                    untouched: days.is_some_and(|days| days >= i64::from(untouched_days)),
                }
            })
            .collect();
        break_circles(&mut documents);
        for at in 0..documents.len() {
            if let Some(newer) = documents[at].folds_under() {
                documents[newer].replaces.push(at);
            }
        }
        let types = type_counts(&documents, rules);
        let mut totals = Counts::default();
        for doc in &documents {
            totals.add(doc);
        }
        View {
            untouched_days,
            documents,
            types,
            totals,
        }
    }

    /// The index of the document at `path`.
    #[must_use]
    pub fn find(&self, path: &str) -> Option<usize> {
        self.documents
            .binary_search_by(|doc| doc.path.as_str().cmp(path))
            .ok()
    }

    /// Whether the document at `index` matches every part of `filter`.
    #[must_use]
    pub fn matches(&self, index: usize, filter: &Filter) -> bool {
        let doc = &self.documents[index];
        (filter.types.is_empty() || filter.types.contains(&doc.doc_type))
            && filter
                .stage
                .is_none_or(|stage| Stage::of(doc.lifecycle) == stage)
            && filter
                .folder
                .as_deref()
                .is_none_or(|folder| doc.path.starts_with(folder))
            && (!filter.untouched || doc.untouched)
    }

    /// Every document `filter` matches, by index, sorted by path.
    #[must_use]
    pub fn listed(&self, filter: &Filter) -> Vec<usize> {
        (0..self.documents.len())
            .filter(|&index| self.matches(index, filter))
            .collect()
    }

    /// The rows a page shows for `filter`, one group per type in the order
    /// of [`View::types`], types without rows left out.
    ///
    /// A document that matches is a row of its own type, unless a newer
    /// document it folds under, directly or through others, matches too:
    /// then it is shown folded under that one. So a superseded chain shows
    /// once, under its newest document that matches.
    #[must_use]
    pub fn groups(&self, filter: &Filter) -> Vec<Group> {
        let matching: Vec<bool> = (0..self.documents.len())
            .map(|index| self.matches(index, filter))
            .collect();
        let folded = |mut index: usize| {
            while let Some(newer) = self.documents[index].folds_under() {
                if matching[newer] {
                    return true;
                }
                index = newer;
            }
            false
        };
        let mut rows: HashMap<&str, Vec<usize>> = HashMap::new();
        for index in (0..self.documents.len()).filter(|&index| matching[index] && !folded(index)) {
            rows.entry(self.documents[index].doc_type.as_str())
                .or_default()
                .push(index);
        }
        self.types
            .iter()
            .filter_map(|counts| {
                rows.remove(counts.name.as_str()).map(|rows| Group {
                    name: counts.name.clone(),
                    rows,
                })
            })
            .collect()
    }
}

/// Turns every [`Target::Found`] that is part of a circle of superseded
/// documents into [`Target::Circle`], so folding always ends. Each
/// superseded document folds under at most one other, so following those
/// links from any document either ends or enters one circle.
fn break_circles(documents: &mut [Doc]) {
    #[derive(Clone, Copy, PartialEq)]
    enum Seen {
        Not,
        OnThisWalk,
        Done,
    }
    let mut seen = vec![Seen::Not; documents.len()];
    let mut in_circle = vec![false; documents.len()];
    for start in 0..documents.len() {
        let mut walk = Vec::new();
        let mut at = Some(start);
        while let Some(index) = at.filter(|&index| seen[index] == Seen::Not) {
            seen[index] = Seen::OnThisWalk;
            walk.push(index);
            at = documents[index].folds_under();
        }
        if let Some(back) = at.filter(|&index| seen[index] == Seen::OnThisWalk)
            && let Some(from) = walk.iter().position(|&index| index == back)
        {
            for &index in &walk[from..] {
                in_circle[index] = true;
            }
        }
        for index in walk {
            seen[index] = Seen::Done;
        }
    }
    for (doc, circle) in documents.iter_mut().zip(in_circle) {
        if let (true, Some(successor)) = (circle, doc.successor.as_mut())
            && let Target::Found(index) = successor.target
        {
            successor.target = Target::Circle(index);
        }
    }
}

/// The counts of every type that has documents, in the rules file's order,
/// then other types by name, then [`DEFAULT_TYPE`].
fn type_counts(documents: &[Doc], rules: &Rules) -> Vec<TypeCounts> {
    let mut order: Vec<&str> = Vec::new();
    for rule in rules.types() {
        if !order.contains(&rule.name.as_str()) {
            order.push(&rule.name);
        }
    }
    let mut others: Vec<&str> = documents
        .iter()
        .map(|doc| doc.doc_type.as_str())
        .filter(|name| !order.contains(name) && *name != DEFAULT_TYPE)
        .collect();
    others.sort_unstable();
    others.dedup();
    order.extend(others);
    order.retain(|name| *name != DEFAULT_TYPE);
    order.push(DEFAULT_TYPE);
    order
        .into_iter()
        .filter_map(|name| {
            let mut counts = Counts::default();
            for doc in documents.iter().filter(|doc| doc.doc_type == name) {
                counts.add(doc);
            }
            (counts.documents > 0).then(|| TypeCounts {
                name: name.to_owned(),
                counts,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests;
