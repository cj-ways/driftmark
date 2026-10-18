//! Comparing two versions of one document: which blocks changed, how, how
//! each change should be drawn, and how much it matters.
//!
//! [`compare`] takes the two parsed versions of one file with what the rules
//! file says about each, and returns its [`DocumentChanges`]. [`ChangeList`]
//! gathers the documents of a range and sorts them for a reader, and
//! [`SummaryList`] is the same list with only each document's counts. All
//! are plain values that serialize to the JSON contract written out in
//! `docs/features/engine/change-list.md`.

mod align;
mod pair;
mod rank;
mod words;

use std::ops::Range;
use std::sync::Arc;

use serde::Serialize;

pub use align::MAX_CELLS;
pub use pair::{MAX_POSTING, MIN_SHARED, MIN_SHARED_MOVED};
pub use rank::{Level, NORMATIVE_WORDS, Signal};
pub use words::{Mark, MarkOp};

use crate::document::markdown::LineIndex;
use crate::document::{Block, BlockKind, Document, TextSpan};
use crate::git::{ResolvedRange, Snapshot};
use crate::rules::Classification;
use words::{Token, WordDiff};

/// The version of the JSON shape. It goes up by one whenever a field changes
/// meaning or is removed; adding a field keeps it. Version 2 lists an added
/// or removed document as one change instead of one change per block.
pub const SCHEMA_VERSION: u32 = 2;

/// An edited block keeping at least this share of its words in order is
/// shown with inline word marks. From the private prototype.
pub const INLINE_MIN_KEPT: f64 = 0.85;

/// An edited block keeping less than this share of its words in order is
/// shown as rewritten: the old and the new version, each whole. From the
/// private prototype.
pub const REWRITTEN_BELOW: f64 = 0.35;

/// Between the two shares above, plain prose with at least this many
/// sentences on both sides is shown sentence by sentence. From the private
/// prototype.
pub const SENTENCE_VIEW_MIN: usize = 4;

/// One version of a document, with what the rules file says about it.
#[derive(Debug, Clone, Copy)]
pub struct Version<'a> {
    /// The parsed document.
    pub document: &'a Document,
    /// Its type, status and decision sections. With no rules file, use
    /// `Rules::default().classify(path, document)`.
    pub classification: &'a Classification,
}

/// The changes of every document in a range, sorted for a reader.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChangeList {
    /// [`SCHEMA_VERSION`].
    pub schema_version: u32,
    /// The two git points compared, when the program knows them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<RangeInfo>,
    /// The counts over all documents.
    pub totals: Summary,
    /// The changed documents: the highest level first, then the most changes,
    /// then by path.
    pub documents: Vec<DocumentChanges>,
}

impl ChangeList {
    /// The same list with only each document's counts, for a reader or a
    /// tool that wants the overview without every change.
    #[must_use]
    pub fn summary_list(&self) -> SummaryList<'_> {
        SummaryList {
            schema_version: self.schema_version,
            range: self.range.as_ref(),
            totals: self.totals,
            documents: self
                .documents
                .iter()
                .map(|document| DocumentSummary {
                    path: &document.path,
                    file: document.file,
                    summary: document.summary,
                })
                .collect(),
        }
    }

    /// Gathers documents into a change list, sorted for a reader.
    #[must_use]
    pub fn new(range: Option<RangeInfo>, mut documents: Vec<DocumentChanges>) -> ChangeList {
        documents.sort_by(|a, b| {
            b.summary
                .highest
                .cmp(&a.summary.highest)
                .then(b.summary.changes.cmp(&a.summary.changes))
                .then_with(|| a.path.cmp(&b.path))
        });
        let mut totals = Summary::default();
        for document in &documents {
            totals.merge(&document.summary);
        }
        ChangeList {
            schema_version: SCHEMA_VERSION,
            range,
            totals,
            documents,
        }
    }
}

/// A change list without its changes: one entry per document with its path,
/// what happened to the file, and its counts, which hold its highest level.
/// The fields mean what they mean in [`ChangeList`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SummaryList<'a> {
    /// [`SCHEMA_VERSION`].
    pub schema_version: u32,
    /// The two git points compared, when the program knows them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<&'a RangeInfo>,
    /// The counts over all documents.
    pub totals: Summary,
    /// The changed documents, in the order of the change list.
    pub documents: Vec<DocumentSummary<'a>>,
}

/// One document of a [`SummaryList`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DocumentSummary<'a> {
    /// The file's path.
    pub path: &'a str,
    /// Whether the file was added, removed or modified.
    pub file: FileState,
    /// Its counts per level and kind, and its highest level.
    pub summary: Summary,
}

/// The two git points of a change list, as text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RangeInfo {
    /// The older commit's id; `None` before the first commit.
    pub base: Option<String>,
    /// The newer commit's id, or `worktree` for the files on disk.
    pub head: String,
    /// What the reader should know about how the points were chosen.
    pub notes: Vec<String>,
}

impl RangeInfo {
    /// The points of a resolved range.
    #[must_use]
    pub fn of(range: &ResolvedRange) -> RangeInfo {
        RangeInfo {
            base: range.base.as_ref().map(|id| id.as_hex().to_owned()),
            head: match &range.head {
                Snapshot::Commit(id) => id.as_hex().to_owned(),
                Snapshot::Worktree => "worktree".to_owned(),
            },
            notes: range.notes.iter().map(ToString::to_string).collect(),
        }
    }
}

/// What happened to one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileState {
    /// The file is new.
    Added,
    /// The file is gone.
    Removed,
    /// The file is on both sides with other bytes.
    Modified,
}

/// The changes of one document.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DocumentChanges {
    /// The file's path, as the caller gave it.
    pub path: String,
    /// Whether the file was added, removed or modified.
    pub file: FileState,
    /// The status before and after, when a modified document's status
    /// changed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<StatusChange>,
    /// Counts per level and kind, and the highest level.
    pub summary: Summary,
    /// The changes, in reading order: where each one shows in the new
    /// version, a removed block where it used to be.
    pub changes: Vec<Change>,
    /// The blocks that did not change, as `[old index, new index]` pairs.
    /// Pages use them to line the two versions up, and notes to follow a
    /// sentence from one version to the next.
    pub unchanged: Vec<[usize; 2]>,
}

/// A document's status on both sides, as written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StatusChange {
    /// The old status, if there was one.
    pub old: Option<String>,
    /// The new status, if there is one.
    pub new: Option<String>,
}

/// Counts of changes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Summary {
    /// How many changes.
    pub changes: usize,
    /// The highest level among them; `None` when there are none.
    pub highest: Option<Level>,
    /// How many changes have each level.
    pub levels: LevelCounts,
    /// How many changes are of each kind.
    pub kinds: KindCounts,
}

impl Summary {
    fn add(&mut self, change: &Change) {
        self.changes += 1;
        self.highest = self.highest.max(Some(change.level));
        *self.levels.slot(change.level) += 1;
        *self.kinds.slot(change.kind) += 1;
    }

    fn merge(&mut self, other: &Summary) {
        self.changes += other.changes;
        self.highest = self.highest.max(other.highest);
        for level in Level::HIGHEST_FIRST {
            *self.levels.slot(level) += other.levels.get(level);
        }
        for kind in ChangeKind::ALL {
            *self.kinds.slot(kind) += other.kinds.get(kind);
        }
    }
}

/// How many changes have each level.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct LevelCounts {
    /// Changes at level decision.
    pub decision: usize,
    /// Changes at level rule.
    pub rule: usize,
    /// Changes at level fact.
    pub fact: usize,
    /// Changes at level wording.
    pub wording: usize,
    /// Changes at level formatting.
    pub formatting: usize,
}

impl LevelCounts {
    /// The count for one level.
    #[must_use]
    pub fn get(&self, level: Level) -> usize {
        match level {
            Level::Decision => self.decision,
            Level::Rule => self.rule,
            Level::Fact => self.fact,
            Level::Wording => self.wording,
            Level::Formatting => self.formatting,
        }
    }

    fn slot(&mut self, level: Level) -> &mut usize {
        match level {
            Level::Decision => &mut self.decision,
            Level::Rule => &mut self.rule,
            Level::Fact => &mut self.fact,
            Level::Wording => &mut self.wording,
            Level::Formatting => &mut self.formatting,
        }
    }
}

/// How many changes are of each kind.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct KindCounts {
    /// Formatting-only changes.
    pub formatting: usize,
    /// Edited blocks.
    pub edited: usize,
    /// Added blocks.
    pub added: usize,
    /// Removed blocks.
    pub removed: usize,
    /// Moved blocks.
    pub moved: usize,
    /// Blocks that moved and were edited.
    pub moved_edited: usize,
}

impl KindCounts {
    /// The count for one kind.
    #[must_use]
    pub fn get(&self, kind: ChangeKind) -> usize {
        match kind {
            ChangeKind::Formatting => self.formatting,
            ChangeKind::Edited => self.edited,
            ChangeKind::Added => self.added,
            ChangeKind::Removed => self.removed,
            ChangeKind::Moved => self.moved,
            ChangeKind::MovedEdited => self.moved_edited,
        }
    }

    fn slot(&mut self, kind: ChangeKind) -> &mut usize {
        match kind {
            ChangeKind::Formatting => &mut self.formatting,
            ChangeKind::Edited => &mut self.edited,
            ChangeKind::Added => &mut self.added,
            ChangeKind::Removed => &mut self.removed,
            ChangeKind::Moved => &mut self.moved,
            ChangeKind::MovedEdited => &mut self.moved_edited,
        }
    }
}

/// What happened to one block. An unchanged block is not a change; its pair
/// is in [`DocumentChanges::unchanged`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    /// The same content, written differently: wrapping, emphasis style, list
    /// markers, numbering, or nesting.
    Formatting,
    /// The content changed in place.
    Edited,
    /// The block is new.
    Added,
    /// The block is gone.
    Removed,
    /// The same content in another place.
    Moved,
    /// The block moved and its content changed.
    MovedEdited,
}

impl ChangeKind {
    /// Every kind.
    pub const ALL: [ChangeKind; 6] = [
        ChangeKind::Formatting,
        ChangeKind::Edited,
        ChangeKind::Added,
        ChangeKind::Removed,
        ChangeKind::Moved,
        ChangeKind::MovedEdited,
    ];
}

/// One change.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Change {
    /// What happened to the block, or to the whole document.
    pub kind: ChangeKind,
    /// How much it matters: the highest level among its signals.
    pub level: Level,
    /// Why it matters, in the order of [`Signal`].
    pub signals: Vec<Signal>,
    /// What changed: one block, or a whole document. In JSON its fields sit
    /// beside the ones above: `old` and `new`, or `document`.
    #[serde(flatten)]
    pub subject: Subject,
    /// How to draw an edit, for edited and moved-and-edited blocks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edit: Option<Edit>,
}

impl Change {
    /// Where a block change is in the old and the new version; `None` for a
    /// whole document.
    #[must_use]
    pub fn sides(&self) -> Option<(&Side, &Side)> {
        match &self.subject {
            Subject::Block { old, new } => Some((old, new)),
            Subject::Document { .. } => None,
        }
    }
}

/// What one change is about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Subject {
    /// One block of a document on both sides.
    Block {
        /// Where the change is in the old version.
        old: Side,
        /// Where the change is in the new version.
        new: Side,
    },
    /// A whole document that was added or removed. Its blocks are not listed
    /// one by one: a reader opens the document to read it all.
    Document {
        /// The document, from the side that has it.
        document: WholeDocument,
    },
}

/// An added or removed document, as one change shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WholeDocument {
    /// Its title, when it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Its summary, when it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Its size in bytes.
    pub bytes: usize,
    /// How many lines it has.
    pub lines: usize,
    /// Its full text, as written, so a tool can show the whole document
    /// without reading git again.
    pub source: String,
}

/// Where a change is in one version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Side {
    /// The block's index in that version when it has the block. Otherwise
    /// the place it would be: the index of the block it would come before,
    /// or the number of blocks when it would come last.
    pub at: usize,
    /// The block, when this version has it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block: Option<BlockRef>,
}

/// One block of one version, as a change list shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BlockRef {
    /// What the block is.
    pub kind: BlockKind,
    /// The byte where its source starts.
    pub start: usize,
    /// The byte after its source ends.
    pub end: usize,
    /// The 1-based line it starts on.
    pub line: usize,
    /// The headings it sits under, outermost first. Blocks of one section
    /// share it.
    pub heading_path: Arc<[String]>,
    /// Its plain text.
    pub text: String,
}

/// How to draw an edited block.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Edit {
    /// The view.
    pub view: View,
    /// The share of words kept in the same order, on the longer version,
    /// rounded to three decimals. For code and diagrams, the share of lines.
    pub kept_in_order: f64,
    /// Word marks, for the inline and side-by-side views.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub marks: Vec<Mark>,
    /// One row of word marks per sentence, for the sentence view.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sentences: Vec<Vec<Mark>>,
    /// One mark per line, for the lines view.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<Mark>,
    /// For a table row, the positions of the cells that changed, from 0.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cells: Vec<usize>,
    /// True when the marks failed the guard, so the block is shown side by
    /// side without marks.
    #[serde(skip_serializing_if = "is_false")]
    pub fallback: bool,
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde passes the field by reference"
)]
fn is_false(value: &bool) -> bool {
    !*value
}

/// How an edited block is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum View {
    /// One text with deleted and inserted words marked.
    Inline,
    /// One row per sentence, each with its word marks.
    Sentences,
    /// The old and the new version next to each other, words marked.
    SideBySide,
    /// The old and the new version, each whole, without marks.
    Rewritten,
    /// Code and diagrams: one row per line, kept, deleted or inserted.
    Lines,
}

/// Which block of a new version each block of an old version became, as the
/// change list pairs them. Notes use it to follow a sentence from one version
/// to the next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockMap {
    /// For each old block, the new block it pairs with, if any.
    pub old_to_new: Vec<Option<usize>>,
    /// For each old block, whether its pair stayed in place. False for a
    /// block without a pair and for one that moved.
    pub in_place: Vec<bool>,
}

/// Pairs the blocks of two versions of one document, exactly as [`compare`]
/// does.
#[must_use]
pub fn block_map(old: &Document, new: &Document) -> BlockMap {
    let pairing = pair::pair(&old.blocks, &new.blocks);
    BlockMap {
        old_to_new: pairing.old_to_new,
        in_place: pairing.in_place,
    }
}

/// Compares two versions of the document at `path`.
///
/// Give `None` for the side where the file does not exist. Both `None` gives
/// `None`.
#[must_use]
pub fn compare(
    path: &str,
    old: Option<Version<'_>>,
    new: Option<Version<'_>>,
) -> Option<DocumentChanges> {
    let (file, changes, unchanged, status) = match (old, new) {
        (None, None) => return None,
        (None, Some(new)) => (FileState::Added, whole(new, true), Vec::new(), None),
        (Some(old), None) => (FileState::Removed, whole(old, false), Vec::new(), None),
        (Some(old), Some(new)) => {
            let (changes, unchanged, status) = modified(old, new);
            (FileState::Modified, changes, unchanged, status)
        }
    };
    let mut summary = Summary::default();
    for change in &changes {
        summary.add(change);
    }
    Some(DocumentChanges {
        path: path.to_owned(),
        file,
        status,
        summary,
        changes,
        unchanged,
    })
}

/// One version's blocks, with what the comparison needs to know about them.
struct Sided<'a> {
    document: &'a Document,
    lines: LineIndex,
    decision: Vec<bool>,
}

impl<'a> Sided<'a> {
    fn new(version: Version<'a>) -> Sided<'a> {
        let mut decision = vec![false; version.document.blocks.len()];
        for &index in &version.classification.decision_blocks {
            if let Some(slot) = decision.get_mut(index) {
                *slot = true;
            }
        }
        Sided {
            document: version.document,
            lines: LineIndex::new(&version.document.source),
            decision,
        }
    }

    fn block(&self, index: usize) -> &'a Block {
        &self.document.blocks[index]
    }

    fn side(&self, index: usize) -> Side {
        let block = self.block(index);
        Side {
            at: index,
            block: Some(BlockRef {
                kind: block.kind.clone(),
                start: block.range.start,
                end: block.range.end,
                line: self.lines.line_of(block.range.start),
                heading_path: Arc::clone(&block.heading_path),
                text: block.text.clone(),
            }),
        }
    }
}

/// A side without the block: only the place it would be.
fn gap_at(at: usize) -> Side {
    Side { at, block: None }
}

/// The one change of a file that exists on one side only. Its signals are
/// those its blocks would get as added or removed blocks, so a new decision
/// record still ranks as a decision.
fn whole(version: Version<'_>, added: bool) -> Vec<Change> {
    let document = version.document;
    let sided = Sided::new(version);
    let mut signals = Vec::new();
    for (index, block) in document.blocks.iter().enumerate() {
        block_signals(block, sided.decision[index], &mut signals);
    }
    let signals = finish_signals(signals);
    let lines = match document.source.len() {
        0 => 0,
        size => sided.lines.line_of(size - 1),
    };
    let whole = WholeDocument {
        title: document.meta.title.clone(),
        summary: document.meta.summary.clone(),
        bytes: document.source.len(),
        lines,
        source: document.source.clone(),
    };
    let kind = if added {
        ChangeKind::Added
    } else {
        ChangeKind::Removed
    };
    vec![Change {
        kind,
        level: rank::level_of(&signals),
        signals,
        subject: Subject::Document { document: whole },
        edit: None,
    }]
}

fn change(
    kind: ChangeKind,
    signals: Vec<Signal>,
    old: Side,
    new: Side,
    edit: Option<Edit>,
) -> Change {
    Change {
        kind,
        level: rank::level_of(&signals),
        signals,
        subject: Subject::Block { old, new },
        edit,
    }
}

/// The changes, the unchanged pairs and the status change of a document on
/// both sides.
fn modified(
    old_version: Version<'_>,
    new_version: Version<'_>,
) -> (Vec<Change>, Vec<[usize; 2]>, Option<StatusChange>) {
    let old = Sided::new(old_version);
    let new = Sided::new(new_version);
    let pairing = pair::pair(&old.document.blocks, &new.document.blocks);
    let chain: Vec<(usize, usize)> = pairing
        .old_to_new
        .iter()
        .enumerate()
        .filter_map(|(o, partner)| partner.filter(|_| pairing.in_place[o]).map(|n| (o, n)))
        .collect();
    let ends = (old.document.blocks.len(), new.document.blocks.len());
    let mut changes = Vec::new();
    let mut unchanged = Vec::new();
    // The old blocks of a gap run from the chain block before it, which is
    // paired and so never listed as removed. The new blocks of a gap start
    // after it, because a paired new block is listed as moved.
    let (mut from_old, mut next_new) = (0, 0);
    for (o, n) in chain.iter().copied().chain(std::iter::once(ends)) {
        for i in from_old..o {
            if pairing.old_to_new[i].is_none() {
                let signals = lone_signals(old.block(i), old.decision[i]);
                changes.push(change(
                    ChangeKind::Removed,
                    signals,
                    old.side(i),
                    gap_at(next_new),
                    None,
                ));
            }
        }
        for j in next_new..n {
            match pairing.new_to_old[j] {
                None => {
                    let signals = lone_signals(new.block(j), new.decision[j]);
                    changes.push(change(
                        ChangeKind::Added,
                        signals,
                        gap_at(o),
                        new.side(j),
                        None,
                    ));
                }
                Some(i) => changes.push(paired(&old, &new, i, j, true)),
            }
        }
        if (o, n) == ends {
            break;
        }
        let (old_block, new_block) = (old.block(o), new.block(n));
        if old_block.source_hash == new_block.source_hash
            && old_block.content_hash == new_block.content_hash
            && old_block.containers == new_block.containers
        {
            unchanged.push([o, n]);
        } else {
            changes.push(paired(&old, &new, o, n, false));
        }
        from_old = o;
        next_new = n + 1;
    }
    let status = status_change(old_version.classification, new_version.classification);
    if let Some(status) = &status {
        mark_status(&mut changes, status);
    }
    (changes, unchanged, status)
}

/// The change between two paired blocks that are not unchanged.
fn paired(old: &Sided<'_>, new: &Sided<'_>, o: usize, n: usize, moved: bool) -> Change {
    let (old_block, new_block) = (old.block(o), new.block(n));
    let in_decision = old.decision[o] || new.decision[n];
    let same_content = old_block.content_hash == new_block.content_hash;
    let kind = match (moved, same_content) {
        (false, true) => ChangeKind::Formatting,
        (false, false) => ChangeKind::Edited,
        (true, true) => ChangeKind::Moved,
        (true, false) => ChangeKind::MovedEdited,
    };
    let mut signals = Vec::new();
    let mut edit = None;
    if same_content {
        if moved {
            signals.push(Signal::Moved);
            if in_decision {
                signals.push(Signal::DecisionSection);
            }
        } else {
            signals.push(Signal::Formatting);
        }
    } else {
        if moved {
            signals.push(Signal::Moved);
        }
        if in_decision {
            signals.push(Signal::DecisionSection);
        }
        rank::kind_signals(&old_block.kind, &mut signals);
        rank::kind_signals(&new_block.kind, &mut signals);
        if rank::task_changed(&old_block.kind, &new_block.kind) {
            signals.push(Signal::TaskState);
        }
        let old_tokens = words::tokens(&old_block.text, &old_block.spans);
        let new_tokens = words::tokens(&new_block.text, &new_block.spans);
        if !is_code(&new_block.kind) {
            let spans = (old_block.spans.as_slice(), new_block.spans.as_slice());
            content_signals(spans, (&old_tokens, &new_tokens), &mut signals);
        }
        edit = Some(edit_of(old_block, new_block, &old_tokens, &new_tokens));
        if rank::wording_only(&signals) {
            signals.push(Signal::Wording);
        }
    }
    signals.sort_unstable();
    signals.dedup();
    change(kind, signals, old.side(o), new.side(n), edit)
}

fn is_code(kind: &BlockKind) -> bool {
    matches!(kind, BlockKind::Code { .. } | BlockKind::Diagram { .. })
}

/// The signals that compare what two versions of a block say: normative
/// words, numbers and identifiers, and link targets. Each argument holds the
/// old side, then the new side.
fn content_signals(
    (old_spans, new_spans): (&[TextSpan], &[TextSpan]),
    (old_tokens, new_tokens): (&[Token<'_>], &[Token<'_>]),
    signals: &mut Vec<Signal>,
) {
    if rank::normative_counts(old_tokens) != rank::normative_counts(new_tokens) {
        signals.push(Signal::Normative);
    }
    if rank::facts(old_tokens) != rank::facts(new_tokens) {
        signals.push(Signal::Number);
    }
    if rank::targets(old_spans) != rank::targets(new_spans) {
        signals.push(Signal::Link);
    }
}

/// The signals of a block that was added to or removed from a document that
/// is on both sides.
fn lone_signals(block: &Block, in_decision: bool) -> Vec<Signal> {
    let mut signals = Vec::new();
    block_signals(block, in_decision, &mut signals);
    finish_signals(signals)
}

/// Adds the signals a block gets for being there at all, as an added or
/// removed block. Its normative words give none: such a word is a rule
/// change only when an edit added or removed it (decision 0009).
fn block_signals(block: &Block, in_decision: bool, signals: &mut Vec<Signal>) {
    if in_decision {
        signals.push(Signal::DecisionSection);
    }
    rank::kind_signals(&block.kind, signals);
    if !is_code(&block.kind) {
        let tokens = words::tokens(&block.text, &block.spans);
        if rank::has_fact(&tokens) {
            signals.push(Signal::Number);
        }
        if rank::has_target(&block.spans) {
            signals.push(Signal::Link);
        }
    }
}

/// Sorts the signals of an added or removed block or document, keeps each
/// once, and adds the wording signal when no content signal is there.
fn finish_signals(mut signals: Vec<Signal>) -> Vec<Signal> {
    signals.sort_unstable();
    signals.dedup();
    if rank::wording_only(&signals) {
        signals.push(Signal::Wording);
    }
    signals
}

/// How to draw an edit between two blocks.
fn edit_of(old: &Block, new: &Block, old_tokens: &[Token<'_>], new_tokens: &[Token<'_>]) -> Edit {
    let cells = changed_cells(&old.kind, &new.kind);
    if is_code(&new.kind) {
        let (lines, share) = words::diff_lines(&old.text, &new.text);
        return Edit {
            view: View::Lines,
            kept_in_order: rounded(share),
            marks: Vec::new(),
            sentences: Vec::new(),
            lines,
            cells,
            fallback: false,
        };
    }
    let WordDiff {
        marks,
        kept_in_order,
        ..
    } = words::diff_words(old_tokens, new_tokens);
    let view = choose_view(kept_in_order, old, new);
    let sentences = if view == View::Sentences {
        sentence_rows(old, new)
    } else {
        Vec::new()
    };
    checked(view, kept_in_order, marks, sentences, cells, &new.text)
}

/// The view for an edited block that is not code, by the share of words
/// kept in order.
fn choose_view(kept_in_order: f64, old: &Block, new: &Block) -> View {
    if kept_in_order >= INLINE_MIN_KEPT {
        View::Inline
    } else if kept_in_order < REWRITTEN_BELOW {
        View::Rewritten
    } else if is_prose(old) && is_prose(new) {
        View::Sentences
    } else {
        View::SideBySide
    }
}

/// Plain prose with enough sentences for the sentence view.
fn is_prose(block: &Block) -> bool {
    let prose_kind = matches!(
        block.kind,
        BlockKind::Paragraph
            | BlockKind::ListItem(_)
            | BlockKind::Alert { .. }
            | BlockKind::FootnoteDefinition { .. }
    );
    prose_kind && words::sentences(&block.text).len() >= SENTENCE_VIEW_MIN
}

/// Applies the guard and builds the edit. Marks that do not spell the new
/// text exactly are dropped, and the block is shown side by side.
fn checked(
    view: View,
    kept_in_order: f64,
    marks: Vec<Mark>,
    sentences: Vec<Vec<Mark>>,
    cells: Vec<usize>,
    new_text: &str,
) -> Edit {
    let (marks, sentences) = match view {
        View::Inline | View::SideBySide => (marks, Vec::new()),
        View::Sentences => (Vec::new(), sentences),
        View::Rewritten | View::Lines => (Vec::new(), Vec::new()),
    };
    let shown = marks.iter().chain(sentences.iter().flatten());
    let fallback = view != View::Rewritten && !words::keeps_new_text(shown, new_text);
    if fallback {
        return Edit {
            view: View::SideBySide,
            kept_in_order: rounded(kept_in_order),
            marks: Vec::new(),
            sentences: Vec::new(),
            lines: Vec::new(),
            cells,
            fallback,
        };
    }
    Edit {
        view,
        kept_in_order: rounded(kept_in_order),
        marks,
        sentences,
        lines: Vec::new(),
        cells,
        fallback,
    }
}

/// A share rounded to three decimals, for reading. Views are chosen on the
/// exact share.
fn rounded(share: f64) -> f64 {
    (share * 1000.0).round() / 1000.0
}

/// The positions of the table cells that differ between two rows. A cell
/// that only one row has counts as changed.
fn changed_cells(old: &BlockKind, new: &BlockKind) -> Vec<usize> {
    match (old, new) {
        (BlockKind::TableRow { cells: old, .. }, BlockKind::TableRow { cells: new, .. }) => (0
            ..old.len().max(new.len()))
            .filter(|&index| old.get(index) != new.get(index))
            .collect(),
        _ => Vec::new(),
    }
}

/// The rows of the sentence view: sentences on both sides are lined up by
/// their text, and the sentences between two lined-up ones are paired in
/// order, each pair with its word marks.
fn sentence_rows(old: &Block, new: &Block) -> Vec<Vec<Mark>> {
    let old_ranges = words::sentences(&old.text);
    let new_ranges = words::sentences(&new.text);
    let old_keys: Vec<&str> = old_ranges
        .iter()
        .map(|r| old.text[r.clone()].trim())
        .collect();
    let new_keys: Vec<&str> = new_ranges
        .iter()
        .map(|r| new.text[r.clone()].trim())
        .collect();
    let mut rows = Vec::new();
    let (mut o, mut n) = (0, 0);
    let ends = (old_ranges.len(), new_ranges.len());
    for (old_at, new_at) in align::align(&old_keys, &new_keys)
        .into_iter()
        .chain(std::iter::once(ends))
    {
        let old_gap = &old_ranges[o..old_at];
        let new_gap = &new_ranges[n..new_at];
        let both = old_gap.len().min(new_gap.len());
        for (old_range, new_range) in old_gap.iter().zip(new_gap) {
            rows.push(sentence_marks(old, old_range, new, new_range));
        }
        for old_range in &old_gap[both..] {
            rows.push(vec![mark(MarkOp::Deleted, &old.text[old_range.clone()])]);
        }
        for new_range in &new_gap[both..] {
            rows.push(vec![mark(MarkOp::Inserted, &new.text[new_range.clone()])]);
        }
        if (old_at, new_at) == ends {
            break;
        }
        rows.push(vec![mark(
            MarkOp::Kept,
            &new.text[new_ranges[new_at].clone()],
        )]);
        o = old_at + 1;
        n = new_at + 1;
    }
    rows
}

fn mark(op: MarkOp, text: &str) -> Mark {
    Mark {
        op,
        text: text.to_owned(),
    }
}

/// The word marks between one old and one new sentence.
fn sentence_marks(
    old: &Block,
    old_range: &Range<usize>,
    new: &Block,
    new_range: &Range<usize>,
) -> Vec<Mark> {
    let old_spans = words::spans_within(&old.spans, old_range);
    let new_spans = words::spans_within(&new.spans, new_range);
    let old_tokens = words::tokens(&old.text[old_range.clone()], &old_spans);
    let new_tokens = words::tokens(&new.text[new_range.clone()], &new_spans);
    words::diff_words(&old_tokens, &new_tokens).marks
}

/// The status change of a modified document, when its status changed. Case
/// and white space around the value do not count as a change.
fn status_change(old: &Classification, new: &Classification) -> Option<StatusChange> {
    let normal = |status: &Option<String>| status.as_deref().map(|s| s.trim().to_lowercase());
    (normal(&old.status) != normal(&new.status)).then(|| StatusChange {
        old: old.status.clone(),
        new: new.status.clone(),
    })
}

/// Adds the status signal to the first change whose block holds the status:
/// the old block holding the old status, or the new block the new one.
fn mark_status(changes: &mut [Change], status: &StatusChange) {
    let holds = |side: &Side, status: &Option<String>| match (&side.block, status) {
        (Some(block), Some(status)) => block
            .text
            .to_lowercase()
            .contains(&status.trim().to_lowercase()),
        _ => false,
    };
    if let Some(change) = changes.iter_mut().find(|change| {
        change
            .sides()
            .is_some_and(|(old, new)| holds(old, &status.old) || holds(new, &status.new))
    }) {
        change.signals.retain(|signal| *signal != Signal::Wording);
        change.signals.push(Signal::Status);
        change.signals.sort_unstable();
        change.level = rank::level_of(&change.signals);
    }
}

#[cfg(test)]
mod tests;
