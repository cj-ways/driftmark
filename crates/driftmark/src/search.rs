//! Search for the program: an index of a revision or of the working tree,
//! built on all cores, and the results a reader sees.
//!
//! `driftmark-core` holds the index and the ranking and touches no file.
//! This module reads the documents for it: blobs from git for a commit, and
//! files from the disk for the working tree. The server keeps the indexes
//! (`docs/features/commands/serve.md`); M3's MCP connector will search
//! through the same functions.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use driftmark_core::document::{Document, ParseError};
use driftmark_core::git::{GitError, GitRead, ObjectId, RepoPath};
use driftmark_core::rules::{DEFAULT_TYPE, Rules};
use driftmark_core::search::{Entry, Found, Hit, Index, Piece, Place, Query, Shard, place};
use serde::Serialize;

/// The fewest documents one shard holds. Smaller shards would cost more to
/// join than they save.
const MIN_SHARD: usize = 32;

/// Where a file's bytes come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Read {
    /// A blob of a commit.
    Blob(ObjectId),
    /// The file on disk. A file that is gone by the time it is read counts
    /// as removed.
    Disk,
    /// Nowhere: the path has no document any more. In an index of changed
    /// files, this hides the path's older version in an earlier index.
    Gone,
}

/// A file to index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct File {
    /// Where it is.
    pub path: RepoPath,
    /// Where its bytes come from.
    pub read: Read,
}

/// A file left out of an index, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Unindexed {
    /// Its path.
    pub path: String,
    /// Why, as a sentence.
    pub reason: String,
}

/// An index with what the program needs to show its results.
#[derive(Debug)]
pub struct Layer {
    /// The index.
    pub index: Index,
    /// Each document's blob, by its id in the index; `None` for a file read
    /// from the disk.
    pub blobs: Vec<Option<ObjectId>>,
    /// The files that could not be parsed.
    pub unindexed: Vec<Unindexed>,
}

impl Layer {
    /// The memory the layer holds, estimated in bytes.
    #[must_use]
    pub fn bytes(&self) -> usize {
        let blobs: usize = self
            .blobs
            .iter()
            .map(|blob| {
                size_of::<Option<ObjectId>>() + blob.as_ref().map_or(0, |id| id.as_hex().len())
            })
            .sum();
        let unindexed: usize = self
            .unindexed
            .iter()
            .map(|file| size_of::<Unindexed>() + file.path.len() + file.reason.len())
            .sum();
        self.index.bytes() + blobs + unindexed
    }
}

/// Indexes `files` with `rules` on up to `threads` threads: reads each
/// file, parses it with the rules' masthead keys, and adds it with its type.
/// Documents are numbered in the order of `files`.
///
/// # Errors
///
/// The first error from reading git, in the order of `files`. A file that
/// cannot be parsed is not an error: it is listed in
/// [`Layer::unindexed`].
pub fn build(
    git: &dyn GitRead,
    files: &[File],
    rules: &Rules,
    threads: usize,
) -> Result<Layer, GitError> {
    // One run of files, and one shard, per thread. More, smaller shards
    // balanced no better on a 3,500-file corpus, and each shard keeps its
    // own copy of the words it holds until the join, which the system's
    // allocator then keeps: 48 shards left the process 33 MB larger than 12.
    let size = files.len().div_ceil(threads.max(1)).max(MIN_SHARD);
    let chunks: Vec<&[File]> = files.chunks(size).collect();
    let done = parallel(
        &chunks,
        threads,
        |chunk| shard(git, chunk, rules),
        Result::is_err,
    );
    let mut shards = Vec::with_capacity(done.len());
    let mut unindexed = Vec::new();
    for shard in done {
        let (shard, left_out) = shard?;
        shards.push(shard);
        unindexed.extend(left_out);
    }
    Ok(Layer {
        index: Index::from_shards(shards),
        blobs: files
            .iter()
            .map(|file| match &file.read {
                Read::Blob(blob) => Some(blob.clone()),
                Read::Disk | Read::Gone => None,
            })
            .collect(),
        unindexed,
    })
}

/// Runs `work` on each item on up to `threads` threads, the calling thread
/// among them, and returns the results in the order of the items. The
/// threads take items in order, so once `stop` holds for a result, no later
/// item is started, and every earlier item has its result.
fn parallel<T: Sync, R: Send>(
    items: &[T],
    threads: usize,
    work: impl Fn(&T) -> R + Sync,
    stop: impl Fn(&R) -> bool + Sync,
) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let stopped = AtomicBool::new(false);
    let done: Mutex<Vec<(usize, R)>> = Mutex::new(Vec::with_capacity(items.len()));
    let run = || {
        while !stopped.load(Ordering::Relaxed) {
            let at = next.fetch_add(1, Ordering::Relaxed);
            let Some(item) = items.get(at) else {
                break;
            };
            let result = work(item);
            if stop(&result) {
                stopped.store(true, Ordering::Relaxed);
            }
            done.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((at, result));
        }
    };
    std::thread::scope(|scope| {
        for _ in 1..threads.min(items.len()) {
            scope.spawn(run);
        }
        run();
    });
    let mut done = done.into_inner().unwrap_or_else(PoisonError::into_inner);
    done.sort_by_key(|(at, _)| *at);
    done.into_iter().map(|(_, result)| result).collect()
}

/// One run of files indexed, with the files that could not be parsed.
type Built = Result<(Shard, Vec<Unindexed>), GitError>;

/// Indexes one run of files in order.
fn shard(git: &dyn GitRead, files: &[File], rules: &Rules) -> Built {
    let mut shard = Shard::new();
    let mut unindexed = Vec::new();
    for file in files {
        let read = match &file.read {
            Read::Blob(blob) => git.read_blob(blob),
            Read::Disk => git.read_worktree_file(&file.path),
            Read::Gone => Err(GitError::MissingFile {
                path: file.path.clone(),
            }),
        };
        let bytes = match read {
            Ok(bytes) => bytes,
            Err(GitError::MissingFile { .. }) => {
                shard.remove(&file.path);
                continue;
            }
            Err(error) => return Err(error),
        };
        let text_path = file.path.to_lossy();
        match Document::parse_with(bytes, rules.masthead_keys()) {
            Ok(document) => {
                let doc_type = rules
                    .type_for(&text_path)
                    .map_or(DEFAULT_TYPE, |rule| rule.name.as_str());
                shard.add(Entry {
                    path: &file.path,
                    doc_type,
                    document: &document,
                });
            }
            Err(error) => {
                shard.remove(&file.path);
                unindexed.push(Unindexed {
                    path: text_path.into_owned(),
                    reason: format!("it cannot be searched: {error}"),
                });
            }
        }
    }
    Ok((shard, unindexed))
}

/// One result as a reader sees it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Match {
    /// The document's path.
    pub path: String,
    /// Its title, when it has one.
    pub title: Option<String>,
    /// Its type from the rules file.
    #[serde(rename = "type")]
    pub doc_type: String,
    /// The block that matches best, by its index in the document model; the
    /// page element with this `data-k` holds it.
    pub block: usize,
    /// The titles of the headings above that block, outermost first, ending
    /// with the block's own title when it is a heading that opens a section.
    pub headings: Vec<String>,
    /// The id of the heading that opens that section on the page, without
    /// `#`.
    pub anchor: Option<String>,
    /// A short stretch of the block with the matched words marked.
    pub snippet: Vec<Piece>,
}

/// A parsed document, or why it could not be parsed.
pub type Parsed = Result<Document, ParseError>;

/// Turns what [`driftmark_core::search::search`] found into results.
/// `document` gives a hit's document, parsed again, for its snippet: from a
/// cache or git for a commit, from the disk for the working tree. When it
/// gives none, or a file that changed since it was indexed into one that
/// cannot be parsed, the result has no snippet and no headings.
///
/// Parsing the documents is most of the work when they are not cached, so
/// the hits are done on up to `threads` threads.
pub fn matches(
    query: &Query,
    found: &Found,
    threads: usize,
    document: impl Fn(&Hit) -> Option<Arc<Parsed>> + Sync,
) -> Vec<Match> {
    let shown = |hit: &Hit| {
        let parsed = document(hit);
        let (place, snippet) = match parsed.as_deref().and_then(|parsed| parsed.as_ref().ok()) {
            Some(document) => (
                place(document, hit.block),
                document
                    .blocks
                    .get(hit.block)
                    .map(|block| query.snippet(block))
                    .unwrap_or_default(),
            ),
            None => (Place::default(), Vec::new()),
        };
        Match {
            path: hit.path.to_lossy().into_owned(),
            title: hit.title.clone(),
            doc_type: hit.doc_type.clone(),
            block: hit.block,
            headings: place.headings,
            anchor: place.anchor,
            snippet,
        }
    };
    parallel(&found.hits, threads, shown, |_| false)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use driftmark_core::git::{Branch, FileChange, FileEntry, Roots, Snapshot};
    use driftmark_core::search::{Filter, search};

    use super::*;

    /// Blobs and disk files from maps, for building without a repo.
    #[derive(Default)]
    struct Files {
        blobs: HashMap<String, Vec<u8>>,
        disk: HashMap<String, Vec<u8>>,
        broken: bool,
    }

    fn unused<T>() -> Result<T, GitError> {
        Err(GitError::Other {
            message: "not used".into(),
        })
    }

    impl GitRead for Files {
        fn resolve(&self, _: &str) -> Result<ObjectId, GitError> {
            unused()
        }
        fn merge_bases(&self, _: &ObjectId, _: &ObjectId) -> Result<Vec<ObjectId>, GitError> {
            unused()
        }
        fn read_commit(&self, _: &ObjectId) -> Result<Vec<u8>, GitError> {
            unused()
        }
        fn markdown_files(&self, _: &Snapshot, _: &Roots) -> Result<Vec<FileEntry>, GitError> {
            unused()
        }
        fn markdown_changes(
            &self,
            _: &ObjectId,
            _: &ObjectId,
            _: &Roots,
        ) -> Result<Vec<FileChange>, GitError> {
            unused()
        }
        fn blob_at(&self, _: &ObjectId, _: &RepoPath) -> Result<Option<ObjectId>, GitError> {
            unused()
        }
        fn branches(&self) -> Result<Vec<Branch>, GitError> {
            unused()
        }
        fn read_blob(&self, id: &ObjectId) -> Result<Vec<u8>, GitError> {
            self.blobs
                .get(id.as_hex())
                .cloned()
                .ok_or_else(|| GitError::MissingObject { id: id.clone() })
        }
        fn read_worktree_file(&self, path: &RepoPath) -> Result<Vec<u8>, GitError> {
            if self.broken {
                return unused();
            }
            self.disk
                .get(path.to_lossy().as_ref())
                .cloned()
                .ok_or_else(|| GitError::MissingFile { path: path.clone() })
        }
    }

    fn id(n: usize) -> ObjectId {
        ObjectId::from_hex(&format!("{n:040x}")).unwrap()
    }

    fn file(path: &str, read: Read) -> File {
        File {
            path: RepoPath::new(path).unwrap(),
            read,
        }
    }

    #[test]
    fn a_commit_is_indexed_on_several_threads_in_order() {
        let mut git = Files::default();
        let mut files = Vec::new();
        for n in 0..200 {
            git.blobs.insert(
                id(n).as_hex().to_owned(),
                format!("# Doc {n}\n\nword{n} shared\n").into_bytes(),
            );
            files.push(file(&format!("docs/{n:03}.md"), Read::Blob(id(n))));
        }
        git.blobs.insert(id(999).as_hex().to_owned(), vec![0xFF]);
        files.push(file("docs/bad.md", Read::Blob(id(999))));
        files.push(file("docs/gone.md", Read::Gone));
        let rules =
            Rules::parse("[[type]]\nname = \"decision\"\npaths = [\"docs/1*.md\"]\n").unwrap();
        let layer = build(&git, &files, &rules, 4).unwrap();
        assert_eq!(layer.index.documents(), 200);
        assert_eq!(layer.blobs.len(), 202);
        assert_eq!(layer.blobs[5], Some(id(5)));
        assert_eq!(
            layer.unindexed,
            [Unindexed {
                path: "docs/bad.md".into(),
                reason: "it cannot be searched: the file is not UTF-8 text: byte 0, on line 1, is not valid UTF-8".into(),
            }]
        );
        let query = Query::parse("word150 ").unwrap();
        let found = search(&[&layer.index], &query, &Filter::default(), 10);
        assert_eq!(found.hits.len(), 1);
        assert_eq!(found.hits[0].doc, 150);
        assert_eq!(found.hits[0].doc_type, "decision");
        let shared = search(
            &[&layer.index],
            &Query::parse("shared ").unwrap(),
            &Filter::default(),
            300,
        );
        assert_eq!(shared.total, 200);
        let one = build(&git, &files, &rules, 1).unwrap();
        assert_eq!(one.index, layer.index, "threads do not change the index");
        assert!(layer.bytes() > layer.index.bytes());
    }

    #[test]
    fn a_missing_blob_fails_the_build() {
        let git = Files::default();
        let error = build(
            &git,
            &[file("a.md", Read::Blob(id(1)))],
            &Rules::default(),
            2,
        )
        .unwrap_err();
        assert_eq!(error, GitError::MissingObject { id: id(1) });
    }

    #[test]
    fn working_tree_files_that_are_gone_count_as_removed() {
        let mut git = Files::default();
        git.disk.insert("a.md".into(), b"# A\n\nhere\n".to_vec());
        let files = [file("a.md", Read::Disk), file("b.md", Read::Disk)];
        let layer = build(&git, &files, &Rules::default(), 2).unwrap();
        assert_eq!(layer.index.documents(), 1);
        assert_eq!(layer.blobs, [None, None]);
        git.broken = true;
        assert!(build(&git, &files, &Rules::default(), 2).is_err());
    }

    #[test]
    fn matches_carry_the_place_and_the_snippet() {
        let mut git = Files::default();
        let text = "# Cache pages\n\n## Decision\n\nKeep each page for one hour.\n";
        git.blobs
            .insert(id(1).as_hex().to_owned(), text.as_bytes().to_vec());
        let layer = build(
            &git,
            &[file("docs/a.md", Read::Blob(id(1)))],
            &Rules::default(),
            1,
        )
        .unwrap();
        let query = Query::parse("\"one hour\"").unwrap();
        let found = search(&[&layer.index], &query, &Filter::default(), 5);
        let document = Arc::new(Document::parse(text.as_bytes().to_vec()));
        let results = matches(&query, &found, 2, |_| Some(Arc::clone(&document)));
        assert_eq!(
            results,
            [Match {
                path: "docs/a.md".into(),
                title: Some("Cache pages".into()),
                doc_type: "document".into(),
                block: 2,
                headings: vec!["Cache pages".into(), "Decision".into()],
                anchor: Some("decision".into()),
                snippet: vec![
                    Piece {
                        text: "Keep each page for ".into(),
                        hit: false
                    },
                    Piece {
                        text: "one hour".into(),
                        hit: true
                    },
                    Piece {
                        text: ".".into(),
                        hit: false
                    },
                ],
            }]
        );
        let gone = matches(&query, &found, 1, |_| None);
        assert_eq!((gone[0].headings.len(), gone[0].snippet.len()), (0, 0));
        let short = Arc::new(Document::parse(b"# Cache pages\n".to_vec()));
        let changed = matches(&query, &found, 1, |_| Some(Arc::clone(&short)));
        assert_eq!(changed[0].snippet.len(), 0);
        let broken = Arc::new(Document::parse(vec![0xFF]));
        let unparsed = matches(&query, &found, 1, |_| Some(Arc::clone(&broken)));
        assert_eq!(unparsed[0].snippet.len(), 0);
    }
}
