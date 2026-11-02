//! What the server keeps between requests: parsed documents, finished pages
//! and counted ranges, behind one memory cap.
//!
//! Every entry is keyed by what fixes its content: a blob id, a pair of
//! blob ids, or a pair of commit ids. A branch name never keys an entry,
//! because a branch moves; the server resolves the name on each request and
//! looks up what the commit it leads to holds. So an entry never goes stale,
//! and the only reason to drop one is memory: when the entries' sizes add up
//! to more than the cap, the least recently used go first.
//!
//! Sizes are estimates of the heap memory an entry holds, counted when it is
//! stored. Files read from the working tree are never stored: their bytes
//! can change without their name changing.
//!
//! Two requests that need the same missing entry at once share one load: the
//! second waits for the first instead of parsing the same file again.

use std::any::Any;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use driftmark_core::git::{ObjectId, RepoPath};
use serde::Serialize;

/// The memory the cache may hold, in bytes.
///
/// The speed spike measured 152 to 186 MB for the source text and rendered
/// HTML of every document in the private corpus, and about 5 MB for block
/// lists alone. Parsed documents are larger than block lists, so the whole
/// corpus would not fit; a day's reading does, and a document that was
/// dropped costs one parse, a few milliseconds, when it is opened again.
pub(crate) const CAP_BYTES: usize = 128 * 1024 * 1024;

/// Bytes counted for every entry on top of its own estimate: the key, the
/// map slots and the bookkeeping.
const ENTRY_OVERHEAD: usize = 160;

/// What fixes the version of the rules file a document was read with.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum RulesId {
    /// No rules file: the default rules.
    Default,
    /// The rules file's blob at a commit.
    Blob(ObjectId),
    /// The rules file on disk, by a hash of its bytes.
    Worktree(u64),
    /// Rules with presets, by a hash of the rules file and of each preset in
    /// use with the folders it reads.
    Presets(u64),
}

/// The key of a cache entry. Each kind of key holds one type of value.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum Key {
    /// A rules file blob, parsed.
    Rules(ObjectId),
    /// A Markdown blob parsed with the masthead keys of a rules file.
    Document(ObjectId, RulesId),
    /// The finished HTML of a reading page's document, without the page
    /// around it. The path is part of the key because a page shows it.
    Reading(ObjectId, RulesId, RepoPath),
    /// The finished HTML of one document's changes between two blobs.
    Changes(Option<ObjectId>, Option<ObjectId>, RulesId, RepoPath),
    /// The counted change list of a range between two commits.
    Range(Option<ObjectId>, ObjectId),
    /// The counted comparison of a branch tip with the default branch's tip.
    Branch(ObjectId, ObjectId),
    /// The search index of every document in a commit's tree. Two commits
    /// with the same tree hold the same files and the same rules file, so
    /// they share one index.
    Search(ObjectId),
    /// Every document of a commit, classified with a rules file, for the
    /// status view.
    Statuses(ObjectId, RulesId),
    /// When each Markdown file of a commit last changed.
    LastChanges(ObjectId),
    /// The working tree's documents for the status view, while the watcher
    /// has announced this many changes, `HEAD` is at this commit and the
    /// rules are these. A working-tree file is read again whenever it
    /// changes, so only the watcher's count can key it.
    WorktreeStatus(u64, Option<ObjectId>, RulesId),
}

impl Key {
    /// The kind's name, for the counts.
    fn kind(&self) -> Kind {
        match self {
            Key::Rules(_) => Kind::Rules,
            Key::Document(..) => Kind::Documents,
            Key::Reading(..) | Key::Changes(..) => Kind::Pages,
            Key::Range(..) => Kind::Ranges,
            Key::Branch(..) => Kind::Branches,
            Key::Search(_) => Kind::Search,
            Key::Statuses(..) | Key::LastChanges(_) | Key::WorktreeStatus(..) => Kind::Status,
        }
    }
}

/// The kinds of entries, counted apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind {
    /// Parsed rules files.
    Rules,
    /// Parsed documents.
    Documents,
    /// Finished pages.
    Pages,
    /// Counted ranges.
    Ranges,
    /// Counted branches.
    Branches,
    /// Search indexes of revisions.
    Search,
    /// The status view's documents and their last changes, per commit.
    Status,
}

/// A value and the bytes it holds. The load function of
/// [`Cache::get_or_load`] returns one.
pub(crate) struct Sized<T> {
    /// The value.
    pub value: T,
    /// Its heap memory, estimated.
    pub bytes: usize,
}

type Stored = Arc<dyn Any + Send + Sync>;

/// A load under way: the value, or the error of the load, both stored as
/// [`Stored`] so one map holds every type.
type Cell = Arc<OnceLock<Result<Stored, Stored>>>;

/// The counts of one kind of entry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub(crate) struct Counts {
    /// Entries held now.
    pub entries: usize,
    /// Their estimated bytes.
    pub bytes: usize,
    /// Lookups that found an entry, or waited for another request's load.
    pub hits: u64,
    /// Lookups that loaded the value.
    pub misses: u64,
    /// Entries dropped to stay under the cap.
    pub evicted: u64,
}

/// What the cache holds now, for `/api/status` and the tests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Stats {
    /// [`CAP_BYTES`].
    pub cap_bytes: usize,
    /// The estimated bytes of every entry.
    pub bytes: usize,
    /// The counts per kind.
    pub kinds: BTreeMap<Kind, Counts>,
}

/// A byte-capped cache with least-recently-used eviction.
pub(crate) struct Cache {
    cap: usize,
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    entries: HashMap<Key, Entry>,
    /// Keys by the tick of their last use, oldest first.
    order: BTreeMap<u64, Key>,
    tick: u64,
    bytes: usize,
    /// Loads under way, so a second request for the same key waits.
    loading: HashMap<Key, Cell>,
    counts: BTreeMap<Kind, Counts>,
}

struct Entry {
    value: Stored,
    bytes: usize,
    tick: u64,
}

impl Inner {
    fn count(&mut self, kind: Kind) -> &mut Counts {
        self.counts.entry(kind).or_default()
    }

    /// Marks `key` as used now.
    fn touch(&mut self, key: &Key) -> Option<Stored> {
        self.tick += 1;
        let tick = self.tick;
        let entry = self.entries.get_mut(key)?;
        self.order.remove(&entry.tick);
        entry.tick = tick;
        self.order.insert(tick, key.clone());
        Some(Arc::clone(&entry.value))
    }

    fn store(&mut self, cap: usize, key: Key, value: Stored, bytes: usize) {
        let bytes = bytes + ENTRY_OVERHEAD;
        if bytes > cap || self.entries.contains_key(&key) {
            return;
        }
        self.tick += 1;
        let kind = key.kind();
        self.order.insert(self.tick, key.clone());
        self.entries.insert(
            key,
            Entry {
                value,
                bytes,
                tick: self.tick,
            },
        );
        self.bytes += bytes;
        let counts = self.count(kind);
        counts.entries += 1;
        counts.bytes += bytes;
        while self.bytes > cap {
            let Some((_, oldest)) = self.order.pop_first() else {
                break;
            };
            if let Some(entry) = self.entries.remove(&oldest) {
                self.bytes -= entry.bytes;
                let counts = self.count(oldest.kind());
                counts.entries -= 1;
                counts.bytes -= entry.bytes;
                counts.evicted += 1;
            }
        }
    }
}

impl Cache {
    /// An empty cache that holds at most `cap` bytes.
    pub(crate) fn new(cap: usize) -> Cache {
        Cache {
            cap,
            inner: Mutex::new(Inner::default()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The value under `key`, or the one `load` makes, which is then stored.
    ///
    /// An error is not stored, so the next request tries again. A value
    /// larger than the whole cap is returned but not stored.
    ///
    /// # Errors
    ///
    /// The error of `load`, also for a request that waited for it.
    ///
    /// # Panics
    ///
    /// When `key` already holds a value of another type than `T`, which
    /// would be a bug in the choice of keys.
    pub(crate) fn get_or_load<T, E>(
        &self,
        key: &Key,
        load: impl FnOnce() -> Result<Sized<T>, E>,
    ) -> Result<Arc<T>, E>
    where
        T: Send + Sync + 'static,
        E: Clone + Send + Sync + 'static,
    {
        let (cell, first) = {
            let mut inner = self.lock();
            if let Some(found) = inner.touch(key) {
                inner.count(key.kind()).hits += 1;
                return Ok(typed(found));
            }
            if let Some(cell) = inner.loading.get(key).map(Arc::clone) {
                inner.count(key.kind()).hits += 1;
                (cell, false)
            } else {
                inner.count(key.kind()).misses += 1;
                let cell = Cell::default();
                inner.loading.insert(key.clone(), Arc::clone(&cell));
                (cell, true)
            }
        };
        // The first request clears the load from `loading` when it ends,
        // even when the load panics, so later requests do not wait on it.
        let _clear = first.then(|| Loading { cache: self, key });
        let mut made = None;
        let result = cell.get_or_init(|| match load() {
            Ok(sized) => {
                let value: Stored = Arc::new(sized.value);
                made = Some((Arc::clone(&value), sized.bytes));
                Ok(value)
            }
            Err(error) => Err(Arc::new(error)),
        });
        let result = result.clone();
        if let Some((value, bytes)) = made {
            self.lock().store(self.cap, key.clone(), value, bytes);
        }
        match result {
            Ok(value) => Ok(typed(value)),
            Err(error) => Err(typed::<E>(error).as_ref().clone()),
        }
    }

    /// The value under `key`, if the cache holds one. A lookup that finds
    /// nothing counts as a miss: the caller then makes the value and
    /// [`Cache::put`]s it.
    pub(crate) fn get<T: Send + Sync + 'static>(&self, key: &Key) -> Option<Arc<T>> {
        let mut inner = self.lock();
        let found = inner.touch(key);
        let counts = inner.count(key.kind());
        if found.is_some() {
            counts.hits += 1;
        } else {
            counts.misses += 1;
        }
        found.map(typed)
    }

    /// Stores a value made outside [`Cache::get_or_load`], unless the key
    /// already holds one.
    pub(crate) fn put<T: Send + Sync + 'static>(&self, key: &Key, value: T, bytes: usize) {
        self.lock()
            .store(self.cap, key.clone(), Arc::new(value), bytes);
    }

    /// What the cache holds now.
    pub(crate) fn stats(&self) -> Stats {
        let inner = self.lock();
        Stats {
            cap_bytes: self.cap,
            bytes: inner.bytes,
            kinds: inner.counts.clone(),
        }
    }
}

/// Removes a load from `loading` when the request that started it ends.
struct Loading<'a> {
    cache: &'a Cache,
    key: &'a Key,
}

impl Drop for Loading<'_> {
    fn drop(&mut self) {
        self.cache.lock().loading.remove(self.key);
    }
}

/// The stored value as the type its key holds.
fn typed<T: Send + Sync + 'static>(value: Stored) -> Arc<T> {
    value
        .downcast::<T>()
        .unwrap_or_else(|_| unreachable!("each kind of key holds one type of value"))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use super::*;

    fn id(digit: char) -> ObjectId {
        ObjectId::from_hex(&digit.to_string().repeat(40)).unwrap()
    }

    fn key(digit: char) -> Key {
        Key::Rules(id(digit))
    }

    #[expect(clippy::unnecessary_wraps, reason = "it stands in for a load")]
    fn sized(text: &str, bytes: usize) -> Result<Sized<String>, String> {
        Ok(Sized {
            value: text.to_owned(),
            bytes,
        })
    }

    #[test]
    fn a_value_is_loaded_once_and_then_found() {
        let cache = Cache::new(10_000);
        let first = cache.get_or_load(&key('1'), || sized("one", 100)).unwrap();
        let again = cache
            .get_or_load(&key('1'), || -> Result<Sized<String>, String> {
                panic!("loaded twice")
            })
            .unwrap();
        assert!(Arc::ptr_eq(&first, &again));
        let stats = cache.stats();
        assert_eq!(stats.bytes, 100 + ENTRY_OVERHEAD);
        assert_eq!(
            stats.kinds[&Kind::Rules],
            Counts {
                entries: 1,
                bytes: 100 + ENTRY_OVERHEAD,
                hits: 1,
                misses: 1,
                evicted: 0
            }
        );
    }

    #[test]
    fn the_least_recently_used_entry_goes_first() {
        let size = 1000;
        let cache = Cache::new(3 * (size + ENTRY_OVERHEAD));
        for digit in ['1', '2', '3'] {
            cache.get_or_load(&key(digit), || sized("x", size)).unwrap();
        }
        // Using 1 again makes 2 the oldest.
        cache.get_or_load(&key('1'), || sized("x", size)).unwrap();
        cache.get_or_load(&key('4'), || sized("x", size)).unwrap();
        let loads = AtomicUsize::new(0);
        let count = || {
            loads.fetch_add(1, Ordering::Relaxed);
            sized("x", size)
        };
        for digit in ['1', '3', '4'] {
            cache.get_or_load(&key(digit), count).unwrap();
        }
        assert_eq!(loads.load(Ordering::Relaxed), 0, "1, 3 and 4 stayed");
        cache.get_or_load(&key('2'), count).unwrap();
        assert_eq!(loads.load(Ordering::Relaxed), 1, "2 was dropped");
        let stats = cache.stats();
        assert_eq!(stats.bytes, 3 * (size + ENTRY_OVERHEAD));
        assert_eq!(stats.kinds[&Kind::Rules].evicted, 2, "2, then 1");
    }

    #[test]
    fn errors_and_oversized_values_are_not_kept() {
        let cache = Cache::new(1000);
        let failed: Result<Arc<String>, String> =
            cache.get_or_load(&key('1'), || Err("broken".to_owned()));
        assert_eq!(failed.unwrap_err(), "broken");
        assert_eq!(
            *cache.get_or_load(&key('1'), || sized("ok", 1)).unwrap(),
            "ok"
        );
        let big = cache.get_or_load(&key('2'), || sized("big", 5000)).unwrap();
        assert_eq!(*big, "big");
        let reloaded = cache
            .get_or_load(&key('2'), || sized("again", 5000))
            .unwrap();
        assert_eq!(*reloaded, "again");
        assert_eq!(cache.stats().kinds[&Kind::Rules].entries, 1);
    }

    #[test]
    fn requests_for_one_missing_key_share_one_load() {
        let cache = Cache::new(10_000);
        let loads = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    let value = cache
                        .get_or_load(&key('1'), || {
                            loads.fetch_add(1, Ordering::Relaxed);
                            std::thread::sleep(Duration::from_millis(20));
                            sized("shared", 10)
                        })
                        .unwrap();
                    assert_eq!(*value, "shared");
                });
            }
        });
        assert_eq!(loads.load(Ordering::Relaxed), 1);
        let counts = cache.stats().kinds[&Kind::Rules];
        assert_eq!((counts.misses, counts.hits), (1, 7));
    }

    #[test]
    fn kinds_are_counted_apart() {
        let cache = Cache::new(100_000);
        let path = RepoPath::new("a.md").unwrap();
        let keys = [
            (Key::Document(id('1'), RulesId::Default), Kind::Documents),
            (
                Key::Reading(id('1'), RulesId::Worktree(7), path.clone()),
                Kind::Pages,
            ),
            (
                Key::Changes(None, Some(id('2')), RulesId::Blob(id('3')), path),
                Kind::Pages,
            ),
            (Key::Range(None, id('4')), Kind::Ranges),
            (Key::Branch(id('4'), id('5')), Kind::Branches),
            (Key::Search(id('6')), Kind::Search),
            (Key::Statuses(id('7'), RulesId::Default), Kind::Status),
            (Key::LastChanges(id('7')), Kind::Status),
            (Key::WorktreeStatus(3, None, RulesId::Default), Kind::Status),
        ];
        for (key, _) in &keys {
            cache.get_or_load(key, || sized("v", 10)).unwrap();
        }
        let stats = cache.stats();
        assert_eq!(stats.kinds[&Kind::Pages].entries, 2);
        for (key, kind) in keys {
            assert_eq!(key.kind(), kind);
            assert!(stats.kinds[&kind].entries >= 1);
        }
    }
}
