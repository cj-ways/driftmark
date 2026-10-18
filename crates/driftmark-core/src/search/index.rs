//! The index: for every word, the documents that hold it and the places in
//! each document where it stands.
//!
//! A document's words are numbered in reading order. Each block that holds
//! words starts a new run, one place after the previous block's last word,
//! so the last word of one block and the first of the next are never next
//! to each other and a phrase never spans two blocks. A word's places, read
//! against the starts of the blocks, tell which block holds it and whether
//! that block is the title, a heading or the body.
//!
//! Postings are written compactly: each number as a variable-length integer
//! (seven bits per byte), each document and each place as the difference
//! from the one before it. That keeps the index of a 3,500-document corpus
//! in about twenty megabytes.

use std::collections::HashMap;
use std::mem::size_of;
use std::ops::Range;

use super::words::{MAX_WORD_BYTES, each_word, fold_into};
use crate::document::{BlockKind, Document};
use crate::git::RepoPath;

/// Where a block sits in its document, which decides how much a word in it
/// weighs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Field {
    /// Any block that is not a heading.
    Body,
    /// A heading other than the title.
    Heading,
    /// The document's title: its first level-1 heading outside any container.
    Title,
}

impl Field {
    /// How much a match here weighs. Each step is more than any number of
    /// further matches can add to one clause, so a title match always
    /// outranks a heading match of the same words, and a heading match a
    /// body match.
    pub(crate) fn weight(self) -> f64 {
        match self {
            Field::Body => 1.0,
            Field::Heading => 2.0,
            Field::Title => 4.0,
        }
    }
}

/// One document to index, as the program found it.
#[derive(Debug, Clone, Copy)]
pub struct Entry<'a> {
    /// Its path in the repository.
    pub path: &'a RepoPath,
    /// Its type from the rules file, such as `decision`, or `document`.
    pub doc_type: &'a str,
    /// The document, parsed.
    pub document: &'a Document,
}

/// A block that holds words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct IndexedBlock {
    /// The place of its first word.
    pub(crate) start: u32,
    /// Its index in the document's block list.
    pub(crate) index: u32,
    /// Its weight class.
    pub(crate) field: Field,
}

/// One indexed document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DocInfo {
    pub(crate) path: RepoPath,
    pub(crate) title: Option<String>,
    pub(crate) doc_type: String,
    /// Its blocks in [`Index::blocks`].
    pub(crate) blocks: Range<usize>,
}

/// A place in the document list: a document, or the mark that a path has
/// no document here. A removed path hides an older version of the document
/// in an earlier index when several are searched together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DocSlot {
    Live(DocInfo),
    Removed(RepoPath),
}

impl DocSlot {
    /// The document, when the slot holds one.
    pub(crate) fn live(&self) -> Option<&DocInfo> {
        match self {
            DocSlot::Live(info) => Some(info),
            DocSlot::Removed(_) => None,
        }
    }

    pub(crate) fn path(&self) -> &RepoPath {
        match self {
            DocSlot::Live(info) => &info.path,
            DocSlot::Removed(path) => path,
        }
    }
}

/// The postings of one word in a [`Shard`].
#[derive(Debug, Clone, Default)]
struct TermData {
    /// The last document added, to write the next one as a difference.
    last: u32,
    /// How many documents hold the word.
    docs: u32,
    bytes: Vec<u8>,
}

/// Part of an index, built from a run of documents in order. The program
/// builds several shards at once, one per thread, and joins them with
/// [`Index::from_shards`]; documents are numbered by their order across the
/// shards.
#[derive(Debug, Default)]
pub struct Shard {
    docs: Vec<DocSlot>,
    blocks: Vec<IndexedBlock>,
    terms: HashMap<Box<str>, u32>,
    data: Vec<TermData>,
    /// Each word of the document being added, as (term, place).
    scratch: Vec<(u32, u32)>,
    folded: String,
}

impl Shard {
    /// An empty shard.
    #[must_use]
    pub fn new() -> Shard {
        Shard::default()
    }

    /// Adds the next document.
    ///
    /// Every block's plain text is indexed, except link definitions and
    /// thematic breaks, which a reader does not see as text.
    pub fn add(&mut self, entry: Entry<'_>) {
        let id = count(self.docs.len());
        let document = entry.document;
        let title = title_block(document);
        let first_block = self.blocks.len();
        self.scratch.clear();
        let mut place: u32 = 0;
        for (index, block) in document.blocks.iter().enumerate() {
            if matches!(block.kind, BlockKind::Unrendered | BlockKind::ThematicBreak) {
                continue;
            }
            let start = place;
            each_word(&block.text, |_, word| {
                fold_into(word, &mut self.folded);
                if self.folded.len() <= MAX_WORD_BYTES {
                    let term = self.term();
                    self.scratch.push((term, place));
                }
                place += 1;
            });
            if place > start {
                let field = match block.kind {
                    _ if Some(index) == title => Field::Title,
                    BlockKind::Heading { .. } => Field::Heading,
                    _ => Field::Body,
                };
                self.blocks.push(IndexedBlock {
                    start,
                    index: count(index),
                    field,
                });
                place += 1;
            }
        }
        self.scratch.sort_unstable();
        let mut at = 0;
        while let Some(&(term, _)) = self.scratch.get(at) {
            let run = self.scratch[at..].partition_point(|&(t, _)| t == term);
            let data = &mut self.data[term as usize];
            put(&mut data.bytes, id - data.last);
            put(&mut data.bytes, count(run));
            let mut previous = 0;
            for &(_, place) in &self.scratch[at..at + run] {
                put(&mut data.bytes, place - previous);
                previous = place;
            }
            data.last = id;
            data.docs += 1;
            at += run;
        }
        self.docs.push(DocSlot::Live(DocInfo {
            path: entry.path.clone(),
            title: document.meta.title.clone(),
            doc_type: entry.doc_type.to_owned(),
            blocks: first_block..self.blocks.len(),
        }));
    }

    /// Adds the next document as removed: the path has no document that can
    /// be indexed, because it was deleted or cannot be parsed.
    pub fn remove(&mut self, path: &RepoPath) {
        self.docs.push(DocSlot::Removed(path.clone()));
    }

    /// The id of the folded word in `self.folded`, adding it when it is new.
    fn term(&mut self) -> u32 {
        if let Some(&id) = self.terms.get(self.folded.as_str()) {
            return id;
        }
        let id = count(self.data.len());
        self.terms.insert(self.folded.as_str().into(), id);
        self.data.push(TermData::default());
        id
    }
}

/// The title block: the first level-1 heading outside any container, the
/// block the document's title comes from.
fn title_block(document: &Document) -> Option<usize> {
    document.blocks.iter().position(|block| {
        matches!(block.kind, BlockKind::Heading { level: 1 }) && block.containers.is_empty()
    })
}

/// A count as `u32`. Documents, blocks and words are far fewer than four
/// billion, since a document is at most 2 MiB.
fn count(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// Writes `value` as a variable-length integer: seven bits per byte, low
/// bits first, the high bit set on every byte but the last.
pub(crate) fn put(out: &mut Vec<u8>, mut value: u32) {
    while value >= 0x80 {
        out.push(value.to_le_bytes()[0] | 0x80);
        value >>= 7;
    }
    out.push(value.to_le_bytes()[0]);
}

/// Reads a variable-length integer at `*at`, and moves `*at` past it.
pub(crate) fn get(bytes: &[u8], at: &mut usize) -> u32 {
    let mut value = 0;
    let mut shift = 0;
    loop {
        let byte = bytes[*at];
        *at += 1;
        value |= u32::from(byte & 0x7F) << shift;
        if byte < 0x80 {
            return value;
        }
        shift += 7;
    }
}

/// One word of the index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TermInfo {
    /// How many documents hold it.
    pub(crate) docs: u32,
    /// Where its postings start in [`Index::postings`]; they end where the
    /// next word's start.
    pub(crate) start: usize,
}

/// A search index over the documents of one revision, or over the documents
/// that changed since an earlier index was built.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Index {
    /// Every word, folded and sorted, so the words that start with a prefix
    /// sit together.
    pub(crate) terms: Vec<Box<str>>,
    pub(crate) info: Vec<TermInfo>,
    pub(crate) postings: Vec<u8>,
    pub(crate) docs: Vec<DocSlot>,
    pub(crate) blocks: Vec<IndexedBlock>,
    /// Document ids sorted by path, to find a path's document.
    pub(crate) by_path: Vec<u32>,
}

impl Index {
    /// Joins shards into one index. The documents are numbered in the order
    /// the shards hold them, the first shard's first document being 0.
    #[must_use]
    pub fn from_shards(shards: Vec<Shard>) -> Index {
        // Each word gets one id over all shards, and a list of the shards
        // that hold it, in shard order.
        let mut ids: HashMap<&str, usize> = HashMap::new();
        let mut names: Vec<&str> = Vec::new();
        let mut parts: Vec<Vec<(usize, u32)>> = Vec::new();
        for (shard_at, shard) in shards.iter().enumerate() {
            for (name, &local) in &shard.terms {
                let id = *ids.entry(name).or_insert_with(|| {
                    names.push(name);
                    parts.push(Vec::new());
                    names.len() - 1
                });
                parts[id].push((shard_at, local));
            }
        }
        for part in &mut parts {
            part.sort_unstable();
        }
        let mut order: Vec<usize> = (0..names.len()).collect();
        order.sort_unstable_by_key(|&id| names[id]);
        let mut first_doc = Vec::with_capacity(shards.len());
        let mut first_block = Vec::with_capacity(shards.len());
        let (mut docs, mut blocks) = (0, 0);
        for shard in &shards {
            first_doc.push(count(docs));
            first_block.push(blocks);
            docs += shard.docs.len();
            blocks += shard.blocks.len();
        }
        let mut index = Index::default();
        for id in order {
            let start = index.postings.len();
            let (mut last, mut held) = (0, 0);
            for &(shard_at, local) in &parts[id] {
                let data = &shards[shard_at].data[local as usize];
                // The shard's first document is a difference from 0; here
                // it becomes a difference from the word's last document in
                // the shards before.
                let mut at = 0;
                let first = first_doc[shard_at] + get(&data.bytes, &mut at);
                put(&mut index.postings, first - last);
                index.postings.extend_from_slice(&data.bytes[at..]);
                last = first_doc[shard_at] + data.last;
                held += data.docs;
            }
            index.terms.push(names[id].into());
            index.info.push(TermInfo { docs: held, start });
        }
        for (shard_at, shard) in shards.into_iter().enumerate() {
            index.blocks.extend(shard.blocks);
            index
                .docs
                .extend(shard.docs.into_iter().map(|slot| match slot {
                    DocSlot::Live(mut info) => {
                        let shift = first_block[shard_at];
                        info.blocks = info.blocks.start + shift..info.blocks.end + shift;
                        DocSlot::Live(info)
                    }
                    removed @ DocSlot::Removed(_) => removed,
                }));
        }
        let mut by_path: Vec<u32> = (0..count(index.docs.len())).collect();
        by_path.sort_by(|&a, &b| {
            index.docs[a as usize]
                .path()
                .cmp(index.docs[b as usize].path())
        });
        index.by_path = by_path;
        // The lists grew while they were filled; an index is kept for long,
        // so the spare room is given back, and `bytes` counts what is held.
        index.terms.shrink_to_fit();
        index.info.shrink_to_fit();
        index.postings.shrink_to_fit();
        index.docs.shrink_to_fit();
        index.blocks.shrink_to_fit();
        index
    }

    /// How many documents the index holds, removed paths not counted.
    #[must_use]
    pub fn documents(&self) -> usize {
        self.docs.iter().filter_map(DocSlot::live).count()
    }

    /// The memory the index holds, estimated in bytes.
    #[must_use]
    pub fn bytes(&self) -> usize {
        let terms: usize = self
            .terms
            .iter()
            .map(|term| size_of::<Box<str>>() + term.len())
            .sum();
        let docs: usize = self
            .docs
            .iter()
            .map(|slot| {
                size_of::<DocSlot>()
                    + slot.path().as_bytes().len()
                    + match slot {
                        DocSlot::Live(info) => {
                            info.title.as_ref().map_or(0, String::len) + info.doc_type.len()
                        }
                        DocSlot::Removed(_) => 0,
                    }
            })
            .sum();
        size_of::<Index>()
            + terms
            + self.info.len() * size_of::<TermInfo>()
            + self.postings.len()
            + docs
            + self.blocks.len() * size_of::<IndexedBlock>()
            + self.by_path.len() * size_of::<u32>()
    }

    /// The id of the document or removed mark at `path`, if the index holds
    /// one.
    pub(crate) fn find(&self, path: &RepoPath) -> Option<usize> {
        let at = self
            .by_path
            .partition_point(|&id| self.docs[id as usize].path() < path);
        let id = *self.by_path.get(at)? as usize;
        (self.docs[id].path() == path).then_some(id)
    }

    /// The postings of the word at `term`: for each document that holds it,
    /// its id and the places of the word, in order.
    pub(crate) fn postings(&self, term: usize) -> Postings<'_> {
        let end = self
            .info
            .get(term + 1)
            .map_or(self.postings.len(), |next| next.start);
        Postings {
            bytes: &self.postings[self.info[term].start..end],
            at: 0,
            doc: 0,
        }
    }

    /// The ids of the words that fill `word`: the word itself, or every word
    /// that starts with it.
    pub(crate) fn terms_for(&self, word: &str, prefix: bool) -> Range<usize> {
        let start = self.terms.partition_point(|term| &**term < word);
        let end = if prefix {
            start + self.terms[start..].partition_point(|term| term.starts_with(word))
        } else {
            start + usize::from(self.terms.get(start).is_some_and(|term| &**term == word))
        };
        start..end
    }

    /// Where in [`Index::blocks`] the block of a document that holds the
    /// word at `place` is.
    pub(crate) fn block_number(&self, info: &DocInfo, place: u32) -> usize {
        let blocks = &self.blocks[info.blocks.clone()];
        info.blocks.start + blocks.partition_point(|block| block.start <= place) - 1
    }

    /// The block of a document that holds the word at `place`.
    pub(crate) fn block_at(&self, info: &DocInfo, place: u32) -> &IndexedBlock {
        &self.blocks[self.block_number(info, place)]
    }
}

/// Reads the postings of one word.
pub(crate) struct Postings<'a> {
    bytes: &'a [u8],
    at: usize,
    doc: u32,
}

impl Postings<'_> {
    /// The next document and its places, written into `places`, or `None`
    /// after the last.
    pub(crate) fn next_into(&mut self, places: &mut Vec<u32>) -> Option<u32> {
        if self.at == self.bytes.len() {
            return None;
        }
        self.doc += get(self.bytes, &mut self.at);
        let held = get(self.bytes, &mut self.at);
        places.clear();
        let mut place = 0;
        for _ in 0..held {
            place += get(self.bytes, &mut self.at);
            places.push(place);
        }
        Some(self.doc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document(text: &str) -> Document {
        Document::parse(text.as_bytes().to_vec()).unwrap()
    }

    fn path(text: &str) -> RepoPath {
        RepoPath::new(text).unwrap()
    }

    /// Every posting of the index, as `word: doc@places`.
    fn dump(index: &Index) -> Vec<String> {
        (0..index.terms.len())
            .map(|term| {
                let mut postings = index.postings(term);
                let mut places = Vec::new();
                let mut out = Vec::new();
                while let Some(doc) = postings.next_into(&mut places) {
                    out.push(format!("{doc}@{places:?}"));
                }
                assert_eq!(index.info[term].docs as usize, out.len());
                format!("{}: {}", index.terms[term], out.join(" "))
            })
            .collect()
    }

    fn add(shard: &mut Shard, name: &str, text: &str, doc_type: &str) {
        shard.add(Entry {
            path: &path(name),
            doc_type,
            document: &document(text),
        });
    }

    #[test]
    fn variable_length_integers_read_back() {
        let values = [0, 1, 127, 128, 255, 300, 16_383, 16_384, u32::MAX];
        let mut bytes = Vec::new();
        for value in values {
            put(&mut bytes, value);
        }
        assert_eq!(&bytes[..4], [0, 1, 127, 0x80]);
        assert_eq!(bytes.len(), 1 + 1 + 1 + 2 + 2 + 2 + 2 + 3 + 5);
        let mut at = 0;
        for value in values {
            assert_eq!(get(&bytes, &mut at), value);
        }
        assert_eq!(at, bytes.len());
    }

    #[test]
    fn blocks_are_numbered_with_a_gap_between_them() {
        let mut shard = Shard::new();
        add(
            &mut shard,
            "a.md",
            "# Cache Pages\n\nKeep pages.\n\n✅ 👍\n\n---\n\n## Pages\n\n[x]: https://example.com/cache\n\n> # Quoted\n",
            "decision",
        );
        let index = Index::from_shards(vec![shard]);
        assert_eq!(
            dump(&index),
            [
                "cache: 0@[0]",
                "keep: 0@[3]",
                "pages: 0@[1, 4, 6]",
                "quoted: 0@[8]"
            ]
        );
        let starts: Vec<(u32, u32, Field)> = index
            .blocks
            .iter()
            .map(|block| (block.start, block.index, block.field))
            .collect();
        assert_eq!(
            starts,
            [
                (0, 0, Field::Title),
                (3, 1, Field::Body),
                (6, 4, Field::Heading),
                (8, 6, Field::Heading)
            ]
        );
        let info = index.docs[0].live().unwrap();
        assert_eq!(info.title.as_deref(), Some("Cache Pages"));
        assert_eq!(info.doc_type, "decision");
        assert_eq!(index.block_at(info, 0).field, Field::Title);
        assert_eq!(index.block_at(info, 2).index, 0, "the gap after the title");
        assert_eq!(index.block_at(info, 3).index, 1);
        assert_eq!(index.block_at(info, 4).index, 1);
        assert_eq!(index.block_at(info, 6).index, 4);
        assert_eq!(index.block_at(info, 8).index, 6);
    }

    #[test]
    fn only_the_first_level_one_heading_outside_containers_is_the_title() {
        let text = "> # Quoted\n\n## Second\n\n# Title\n\n# Another\n";
        assert_eq!(title_block(&document(text)), Some(2));
        assert_eq!(title_block(&document("## Only\n")), None);
    }

    #[test]
    fn long_words_keep_their_place_but_are_not_kept() {
        let long = "x".repeat(MAX_WORD_BYTES + 1);
        let exact = "y".repeat(MAX_WORD_BYTES);
        let mut shard = Shard::new();
        add(
            &mut shard,
            "a.md",
            &format!("one {long} two {exact}\n"),
            "d",
        );
        let index = Index::from_shards(vec![shard]);
        assert_eq!(
            dump(&index),
            ["one: 0@[0]", "two: 0@[2]", &format!("{exact}: 0@[3]")]
        );
    }

    #[test]
    fn shards_join_in_order_and_words_keep_their_documents() {
        let mut first = Shard::new();
        add(&mut first, "b.md", "alpha beta\n", "d");
        first.remove(&path("gone.md"));
        add(&mut first, "c.md", "# Beta\n\nbeta\n", "d");
        let mut second = Shard::new();
        add(&mut second, "a.md", "beta gamma\n\nbeta\n", "t");
        let mut third = Shard::new();
        add(&mut third, "d.md", &"word ".repeat(200), "d");
        add(&mut third, "e.md", "alpha\n", "d");
        let index = Index::from_shards(vec![first, second, Shard::new(), third]);
        let mut dumped = dump(&index);
        let word = dumped.remove(3);
        assert!(word.starts_with("word: 4@[0, 1, 2,"), "{word}");
        assert_eq!(
            dumped,
            [
                "alpha: 0@[0] 5@[0]",
                "beta: 0@[1] 2@[0, 2] 3@[0, 3]",
                "gamma: 3@[1]",
            ]
        );
        assert_eq!(index.documents(), 5);
        assert_eq!(index.docs.len(), 6);
        assert_eq!(index.docs[1], DocSlot::Removed(path("gone.md")));
        let fourth = index.docs[3].live().unwrap();
        assert_eq!(index.docs[1].live(), None);
        assert_eq!(fourth.blocks, 3..5, "blocks shift past the first shard's");
        assert_eq!(fourth.title, None);
        assert_eq!(index.blocks[fourth.blocks.start].index, 0);
        assert_eq!(index.find(&path("a.md")), Some(3));
        assert_eq!(index.find(&path("gone.md")), Some(1));
        assert_eq!(index.find(&path("e.md")), Some(5));
        assert_eq!(index.find(&path("aa.md")), None);
        assert_eq!(index.find(&path("z.md")), None);
        assert_eq!(index.terms_for("beta", false), 1..2);
        assert_eq!(index.terms_for("bet", false), 1..1);
        assert_eq!(index.terms_for("zeta", false), 4..4);
        assert_eq!(index.terms_for("", true), 0..4);
        assert_eq!(index.terms_for("g", true), 2..3);
        assert_eq!(index.terms_for("w", true), 3..4);
        assert_eq!(index.terms_for("x", true), 4..4);
    }

    #[test]
    fn the_size_estimate_grows_with_what_is_held() {
        let empty = Index::from_shards(Vec::new());
        assert_eq!(empty.documents(), 0);
        assert_eq!(empty.bytes(), size_of::<Index>());
        let mut shard = Shard::new();
        add(
            &mut shard,
            "docs/a.md",
            "# Title\n\nSome words here.\n",
            "decision",
        );
        shard.remove(&path("docs/b.md"));
        let index = Index::from_shards(vec![shard]);
        let terms = ["some", "title", "words", "here"]
            .iter()
            .map(|t| size_of::<Box<str>>() + t.len())
            .sum::<usize>();
        let docs = 2 * size_of::<DocSlot>() + 9 + 5 + 8 + 9;
        let expected = size_of::<Index>()
            + terms
            + 4 * size_of::<TermInfo>()
            + index.postings.len()
            + docs
            + 2 * size_of::<IndexedBlock>()
            + 2 * size_of::<u32>();
        assert_eq!(index.bytes(), expected);
    }

    #[test]
    fn field_weights_rise_from_body_to_title() {
        let weights = [Field::Body, Field::Heading, Field::Title].map(Field::weight);
        assert!(
            weights
                .iter()
                .zip([1.0, 2.0, 4.0])
                .all(|(w, e)| (w - e).abs() < 1e-9)
        );
    }
}
