//! Reading, parsing and comparing changed files for the server.
//!
//! This is the work of `driftmark diff`, and the results and the reasons for
//! skipping a file are the same as the command's. How parsed documents are
//! kept depends on the work:
//!
//! - A count of many files, such as a range's change list or the home page's
//!   branches, parses each blob once for the whole count and drops it after
//!   the last file that needs it, as `driftmark diff` does. It does not push
//!   thousands of documents through the server's [`Cache`], which would drop
//!   every page a reader opened.
//! - One document's changes go through the [`Cache`], so a document that a
//!   page opened before is not parsed again.

use std::collections::{HashMap, HashSet};
use std::mem::size_of;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use driftmark_core::diff::{DocumentChanges, Version, compare};
use driftmark_core::document::{
    Block, BlockKind, Document, Field, MetaValue, ParseError, SpanKind, TextSpan,
};
use driftmark_core::git::{FileChange, GitError, GitRead, ObjectId, RepoPath};
use driftmark_core::rules::Rules;
use serde::Serialize;

use super::cache::{Cache, Key, RulesId, Sized};

/// A parsed version, or why it could not be parsed.
pub(crate) type Parsed = Result<Document, ParseError>;

/// The rules file of one side of a range, and what fixes its version.
pub(crate) struct RulesAt {
    /// Which rules file this is.
    pub id: RulesId,
    /// Its rules.
    pub rules: Rules,
}

/// One changed file to compare.
pub(crate) struct Job {
    /// The file and its blobs.
    pub change: FileChange,
    /// The rules at the newer side of its range.
    pub rules: Arc<RulesAt>,
    /// Whether the newer side is the working tree, read from disk.
    pub worktree: bool,
}

/// A changed file left out of a change list, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Skipped {
    /// The file's path.
    pub path: String,
    /// Why, as a sentence.
    pub reason: String,
}

/// What comparing one file gave.
#[derive(Debug, Clone)]
pub(crate) enum Outcome {
    /// Its changes.
    Compared(DocumentChanges),
    /// It could not be compared.
    Skipped(Skipped),
}

/// A committed version, parsed with the rules' masthead keys, from the cache
/// when it was parsed before.
///
/// # Errors
///
/// When git cannot read the blob.
pub(crate) fn document(
    git: &dyn GitRead,
    cache: &Cache,
    blob: &ObjectId,
    rules: &RulesAt,
) -> Result<Arc<Parsed>, GitError> {
    cache.get_or_load(&Key::Document(blob.clone(), rules.id.clone()), || {
        let parsed = Document::parse_with(git.read_blob(blob)?, rules.rules.masthead_keys());
        let bytes = parsed.as_ref().map_or(0, weight);
        Ok(Sized {
            value: parsed,
            bytes,
        })
    })
}

/// A working-tree version, parsed. It is never cached: its bytes can change
/// while its name stays.
///
/// # Errors
///
/// When the file cannot be read.
pub(crate) fn worktree_document(
    git: &dyn GitRead,
    path: &RepoPath,
    rules: &RulesAt,
) -> Result<Arc<Parsed>, GitError> {
    let bytes = git.read_worktree_file(path)?;
    Ok(Arc::new(Document::parse_with(
        bytes,
        rules.rules.masthead_keys(),
    )))
}

/// What a comparison keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Keep {
    /// Each document's summary only, with blobs shared within the run.
    Counts,
    /// Every change, with blobs from the server's cache.
    Changes,
}

/// Compares every job on up to `threads` threads, and returns the outcomes
/// in the order of the jobs. With one thread, the work runs on the calling
/// thread.
///
/// # Errors
///
/// The first git error, by job order. The threads stop taking new jobs once
/// one happens.
pub(crate) fn compare_all(
    git: &dyn GitRead,
    cache: &Cache,
    jobs: &[Job],
    threads: usize,
    keep: Keep,
) -> Result<Vec<Outcome>, GitError> {
    let run = RunCache::new(jobs, keep);
    let load = |id: &ObjectId, rules: &RulesAt| match keep {
        Keep::Counts => run.get(git, id, rules),
        Keep::Changes => document(git, cache, id, rules),
    };
    let threads = threads.clamp(1, jobs.len().max(1));
    if threads == 1 {
        return jobs.iter().map(|job| one(git, &load, job, keep)).collect();
    }
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let mut done: Vec<(usize, Result<Outcome, GitError>)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut local = Vec::new();
                    while !stop.load(Ordering::Relaxed) {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some(job) = jobs.get(index) else {
                            break;
                        };
                        let outcome = one(git, &load, job, keep);
                        if outcome.is_err() {
                            stop.store(true, Ordering::Relaxed);
                        }
                        local.push((index, outcome));
                    }
                    local
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            })
            .collect()
    });
    done.sort_by_key(|(index, _)| *index);
    done.into_iter().map(|(_, outcome)| outcome).collect()
}

/// A loaded version, shared between the jobs that need it.
type Loaded = Result<Arc<Parsed>, GitError>;

/// The parsed blobs of one count. It knows up front how many jobs need each
/// blob; the first to ask parses it, others asking at the same time wait,
/// and the last one takes it out, so it is freed as soon as no job needs it.
struct RunCache {
    slots: Mutex<HashMap<(ObjectId, RulesId), Slot>>,
}

/// A blob's load, and how many jobs still need it.
type Slot = (Arc<OnceLock<Loaded>>, usize);

impl RunCache {
    fn new(jobs: &[Job], keep: Keep) -> RunCache {
        let mut slots: HashMap<_, Slot> = HashMap::new();
        if keep == Keep::Counts {
            for job in jobs {
                let new = job.change.new_blob().filter(|_| !job.worktree);
                for id in job.change.old_blob().into_iter().chain(new) {
                    let key = (id.clone(), job.rules.id.clone());
                    slots.entry(key).or_default().1 += 1;
                }
            }
        }
        RunCache {
            slots: Mutex::new(slots),
        }
    }

    fn get(&self, git: &dyn GitRead, id: &ObjectId, rules: &RulesAt) -> Loaded {
        let key = (id.clone(), rules.id.clone());
        let cell = {
            let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
            match slots.get_mut(&key) {
                Some((cell, users)) => {
                    *users -= 1;
                    let cell = Arc::clone(cell);
                    if *users == 0 {
                        slots.remove(&key);
                    }
                    cell
                }
                None => Arc::default(),
            }
        };
        cell.get_or_init(|| {
            let bytes = git.read_blob(id)?;
            Ok(Arc::new(Document::parse_with(
                bytes,
                rules.rules.masthead_keys(),
            )))
        })
        .clone()
    }
}

fn one(
    git: &dyn GitRead,
    load: &(dyn Fn(&ObjectId, &RulesAt) -> Loaded + Sync),
    job: &Job,
    keep: Keep,
) -> Result<Outcome, GitError> {
    let path = job.change.path();
    let text_path = path.to_lossy().into_owned();
    let old = job
        .change
        .old_blob()
        .map(|id| load(id, &job.rules))
        .transpose()?;
    let new = match job.change.new_blob() {
        None => None,
        Some(_) if job.worktree => Some(worktree_document(git, path, &job.rules)?),
        Some(id) => Some(load(id, &job.rules)?),
    };
    let skipped = |side: &str, error: &ParseError| {
        Ok(Outcome::Skipped(Skipped {
            path: text_path.clone(),
            reason: format!("the {side} version cannot be compared: {error}"),
        }))
    };
    let old = match old.as_deref() {
        Some(Err(error)) => return skipped("old", error),
        Some(Ok(document)) => Some(document),
        None => None,
    };
    let new = match new.as_deref() {
        Some(Err(error)) => return skipped("new", error),
        Some(Ok(document)) => Some(document),
        None => None,
    };
    let rules = &job.rules.rules;
    // A panic here would be a bug in the comparison. It skips this file and
    // keeps the others, as `driftmark diff` does.
    let compared = catch_unwind(AssertUnwindSafe(|| {
        let old_class = old.map(|d| rules.classify(&text_path, d));
        let new_class = new.map(|d| rules.classify(&text_path, d));
        compare(
            &text_path,
            version(old, old_class.as_ref()),
            version(new, new_class.as_ref()),
        )
    }));
    Ok(match compared {
        Ok(Some(mut changes)) => {
            if keep == Keep::Counts {
                changes.changes = Vec::new();
                changes.unchanged = Vec::new();
            }
            Outcome::Compared(changes)
        }
        Ok(None) => Outcome::Skipped(Skipped {
            path: text_path,
            reason: "it has no version on either side".to_owned(),
        }),
        Err(_) => Outcome::Skipped(Skipped {
            path: text_path,
            reason: "driftmark stopped while comparing it; this is a bug in driftmark".to_owned(),
        }),
    })
}

fn version<'a>(
    document: Option<&'a Document>,
    classification: Option<&'a driftmark_core::rules::Classification>,
) -> Option<Version<'a>> {
    let (document, classification) = document.zip(classification)?;
    Some(Version {
        document,
        classification,
    })
}

/// The heap memory a parsed document holds, estimated: its text, each
/// block's plain text, spans and details, and the heading paths and
/// containers that blocks share, counted once.
pub(crate) fn weight(document: &Document) -> usize {
    let mut shared = HashSet::new();
    let mut bytes = size_of::<Document>() + document.source.capacity();
    for block in &document.blocks {
        bytes += size_of::<Block>() + block.text.capacity() + kind_bytes(block);
        bytes += block.spans.capacity() * size_of::<TextSpan>();
        bytes += block
            .spans
            .iter()
            .map(|span| match &span.kind {
                SpanKind::Code => 0,
                SpanKind::Link { target } | SpanKind::Image { target } => target.capacity(),
            })
            .sum::<usize>();
        if shared.insert(Arc::as_ptr(&block.heading_path).cast::<u8>().addr()) {
            bytes += block
                .heading_path
                .iter()
                .map(|title| size_of::<String>() + title.capacity())
                .sum::<usize>();
        }
        if shared.insert(Arc::as_ptr(&block.containers).cast::<u8>().addr()) {
            bytes += size_of_val(&*block.containers);
        }
    }
    let meta = &document.meta;
    bytes += [&meta.title, &meta.summary, &meta.front_matter_error]
        .into_iter()
        .flatten()
        .map(String::capacity)
        .sum::<usize>();
    bytes += meta.fields.iter().map(field_bytes).sum::<usize>();
    bytes
}

fn kind_bytes(block: &Block) -> usize {
    match &block.kind {
        BlockKind::TableRow { cells, .. } => cells
            .iter()
            .map(|cell| size_of::<String>() + cell.capacity())
            .sum(),
        BlockKind::Code {
            language: Some(text),
            ..
        }
        | BlockKind::Alert {
            title: Some(text), ..
        }
        | BlockKind::FootnoteDefinition { name: text } => text.capacity(),
        _ => 0,
    }
}

fn field_bytes(field: &Field) -> usize {
    size_of::<Field>()
        + field.key.capacity()
        + value_bytes(&field.value)
        + field
            .links
            .iter()
            .map(|link| size_of::<String>() + link.capacity())
            .sum::<usize>()
}

fn value_bytes(value: &MetaValue) -> usize {
    match value {
        MetaValue::Text(text) => text.capacity(),
        MetaValue::List(items) => items
            .iter()
            .map(|item| size_of::<MetaValue>() + value_bytes(item))
            .sum(),
        MetaValue::Map(pairs) => pairs
            .iter()
            .map(|(key, item)| {
                size_of::<(String, MetaValue)>() + key.capacity() + value_bytes(item)
            })
            .sum(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weight_counts_the_text_and_each_block() {
        let small = Document::parse(b"# A\n\nOne line.\n".to_vec()).unwrap();
        let text = "---\ntags: [a, b]\nowner: {name: x}\n---\n\n# Title\n\n> Short version: a summary.\n\n| a | b |\n|---|---|\n| c | d |\n\n- one\n- two\n\n```rust\nfn main() {}\n```\n\n> [!NOTE] Look\n> here\n\n[link](x.md) and `code`[^1].\n\n[^1]: A note.\n";
        let large = Document::parse(text.as_bytes().to_vec()).unwrap();
        let (small_weight, large_weight) = (weight(&small), weight(&large));
        assert!(small_weight > small.source.len(), "{small_weight}");
        assert!(large_weight > small_weight + text.len(), "{large_weight}");
        assert!(
            large_weight < 40 * text.len(),
            "a sane estimate: {large_weight}"
        );
    }
}
