//! `GET /api/search`: full-text search over every document at a revision.
//!
//! A revision's index is built on first use, from every Markdown file of its
//! commit, on all cores. It is kept in the server's cache under the commit's
//! tree id, beside the parsed documents and pages, within the same memory
//! cap: two commits with the same tree share it, and while a reader types,
//! each request that arrives during the build waits for that one build.
//!
//! The working tree has an index of its own, never cached, because its
//! files change under the same names. It is built on the first search, then
//! kept up to date from the watcher's change signal, the same one that
//! reloads open pages: the files that changed since the full build are read
//! again into a small second index that replaces their old versions. When
//! too many files changed, or the rules file did, the whole working tree is
//! indexed again. Searches keep using the last index while an update runs.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError};

use driftmark_core::git::{GitError, GitRead, ObjectId, RepoPath, Roots, Snapshot, tree_of};
use driftmark_core::search::{Filter, Query, search};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use serde::Serialize;
use tokio::sync::broadcast;

use super::address::{self, WORKTREE};
use super::cache::{Key, Sized};
use super::compare::{self, RulesAt};
use super::repo::{Problem, Repo};
use super::watch::{Message, Watch};
use crate::lint::RULES_FILE;
use crate::search::{self as program, File, Layer, Read, Unindexed};

/// How many results an answer holds when `limit` is not given.
pub(crate) const DEFAULT_LIMIT: usize = 20;

/// The most results one answer holds.
pub(crate) const MAX_LIMIT: usize = 100;

/// The most working-tree files kept in the small index of changes. Past
/// this, the whole working tree is indexed again, which takes about as long
/// as reading these files does.
pub(crate) const MAX_CHANGED: usize = 256;

/// The parameters of a search, read from the address.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Params {
    /// The query as typed.
    pub text: String,
    /// The query, parsed.
    pub query: Query,
    /// The revision, or `None` for the default.
    pub rev: Option<String>,
    /// What the results are filtered by.
    pub filter: Filter,
    /// The most results.
    pub limit: usize,
}

impl Params {
    /// Reads `q`, `rev`, `path`, `type` and `limit` from the query part of
    /// an address. Other parameters are ignored, and the last of a repeated
    /// one wins, except `type`, which may be given several times.
    ///
    /// # Errors
    ///
    /// A sentence for a parameter that is not valid text, a `limit` that is
    /// not a number from 1 to [`MAX_LIMIT`], or a query with too many words.
    pub(crate) fn read(raw: Option<&str>) -> Result<Params, String> {
        let mut text = String::new();
        let mut rev = None;
        let mut filter = Filter::default();
        let mut limit = DEFAULT_LIMIT;
        for pair in raw.unwrap_or_default().split('&') {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            let value = decode(value)?;
            match decode(key)?.as_str() {
                "q" => text = value,
                "rev" => rev = Some(value).filter(|rev| !rev.is_empty()),
                "path" => filter.path = Some(value).filter(|path| !path.is_empty()),
                "type" if !value.is_empty() => filter.types.push(value),
                "limit" => {
                    limit = value
                        .parse()
                        .ok()
                        .filter(|limit| (1..=MAX_LIMIT).contains(limit))
                        .ok_or_else(|| format!("`limit` must be a number from 1 to {MAX_LIMIT}"))?;
                }
                _ => {}
            }
        }
        let query = Query::parse(&text).map_err(|error| error.to_string())?;
        Ok(Params {
            text,
            query,
            rev,
            filter,
            limit,
        })
    }
}

/// A parameter's value: `+` is a space, then `%XX` escapes are decoded.
fn decode(raw: &str) -> Result<String, String> {
    let spaced = raw.replace('+', " ");
    percent_decode_str(&spaced)
        .decode_utf8()
        .map(Cow::into_owned)
        .map_err(|_| "a search parameter is not valid text".to_owned())
}

/// The working tree's index: one of every file as it was at the last full
/// build, and one of the files changed since.
pub(crate) struct Layers {
    /// The rules file both were built with.
    rules: Arc<RulesAt>,
    /// Shared by every update until the next full build.
    base: Arc<Layer>,
    changes: Layer,
    /// The files in `changes`: every file the watcher reported since `base`
    /// was built.
    changed: BTreeSet<RepoPath>,
}

impl Layers {
    /// The files that could not be indexed, as the layers stand together.
    fn unindexed(&self) -> Vec<Unindexed> {
        self.base
            .unindexed
            .iter()
            .filter(|file| {
                RepoPath::new(file.path.as_bytes()).is_none_or(|path| !self.changed.contains(&path))
            })
            .chain(&self.changes.unindexed)
            .cloned()
            .collect()
    }
}

/// What `/api/status` shows of the working tree's index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) struct WorktreeStatus {
    /// The documents searched.
    pub documents: usize,
    /// The files read again since the last full build.
    pub changed: usize,
    /// The memory both indexes hold, estimated.
    pub bytes: usize,
}

/// The search index of the working tree, kept up to date in the background.
pub(crate) struct WorktreeIndex {
    repo: Arc<Repo>,
    watch: Arc<Watch>,
    current: Mutex<Option<Arc<Layers>>>,
    /// One build or update at a time.
    building: Mutex<()>,
}

impl WorktreeIndex {
    /// An index of the working tree the watcher watches. Nothing is built
    /// until the first search.
    pub(crate) fn new(repo: Arc<Repo>, watch: Arc<Watch>) -> WorktreeIndex {
        WorktreeIndex {
            repo,
            watch,
            current: Mutex::new(None),
            building: Mutex::new(()),
        }
    }

    /// Follows the watcher's announcements on a thread of its own, until the
    /// server stops.
    pub(crate) fn start(self: &Arc<Self>, mut events: broadcast::Receiver<Message>) {
        let index = Arc::clone(self);
        let follow = move || {
            loop {
                let paths = match events.blocking_recv() {
                    Ok(Message::Changed(paths)) => Some(paths),
                    Ok(Message::Counted) => continue,
                    // Announcements were missed, so any file may have changed.
                    Err(broadcast::error::RecvError::Lagged(_)) => None,
                    Err(broadcast::error::RecvError::Closed) => return,
                };
                index.changed(paths);
            }
        };
        std::thread::Builder::new()
            .name("driftmark-search".to_owned())
            .spawn(follow)
            .map_or_else(
                |error| {
                    eprintln!(
                        "driftmark serve: search will not follow working-tree changes: {error}"
                    );
                },
                drop,
            );
    }

    fn current(&self) -> Option<Arc<Layers>> {
        self.current
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn set(&self, layers: Option<Arc<Layers>>) {
        *self.current.lock().unwrap_or_else(PoisonError::into_inner) = layers;
    }

    /// The index, built now when there is none yet.
    ///
    /// # Errors
    ///
    /// When the working tree or the rules file cannot be read.
    pub(crate) fn layers(&self) -> Result<Arc<Layers>, Problem> {
        if let Some(layers) = self.current() {
            return Ok(layers);
        }
        let _one = self.building.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(layers) = self.current() {
            return Ok(layers);
        }
        let layers = Arc::new(self.build_all()?);
        self.set(Some(Arc::clone(&layers)));
        Ok(layers)
    }

    /// Takes in files the watcher reported, or `None` when it may have
    /// missed some. Nothing happens before the first search built an index.
    fn changed(&self, paths: Option<Vec<String>>) {
        let _one = self.building.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(current) = self.current() else {
            return;
        };
        let mut changed = current.changed.clone();
        let mut whole = paths.is_none();
        for path in paths.unwrap_or_default() {
            // A path shown with `�` cannot be read back; the rules file
            // changes every type and title.
            match RepoPath::new(path.as_bytes()) {
                Some(path)
                    if !path.to_lossy().contains('\u{FFFD}')
                        && path.as_bytes() != RULES_FILE.as_bytes() =>
                {
                    changed.insert(path);
                }
                _ => whole = true,
            }
        }
        let rebuilt = if whole || changed.len() > MAX_CHANGED {
            self.build_all()
        } else {
            self.build_changes(&current, changed)
        };
        // On a failure the next search builds again and reports why.
        self.set(rebuilt.ok().map(Arc::new));
    }

    /// Indexes every Markdown file of the working tree.
    fn build_all(&self) -> Result<Layers, Problem> {
        let rules = self.repo.rules_at(&Snapshot::Worktree)?;
        let paths = match self.watch.listed() {
            Some(paths) => paths,
            None => self
                .repo
                .git
                .markdown_files(&Snapshot::Worktree, &Roots::all())?
                .into_iter()
                .map(|file| file.path)
                .collect(),
        };
        let files: Vec<File> = paths
            .into_iter()
            .map(|path| File {
                path,
                read: Read::Disk,
            })
            .collect();
        let base = program::build(&self.repo.git, &files, &rules.rules, self.repo.threads)?;
        let changes = program::build(&self.repo.git, &[], &rules.rules, 1)?;
        Ok(Layers {
            rules,
            base: Arc::new(base),
            changes,
            changed: BTreeSet::new(),
        })
    }

    /// Reads `changed` again into a new index of changes beside the current
    /// full one. A file git no longer lists counts as removed, even when it
    /// is still on the disk, such as a file that became ignored.
    fn build_changes(
        &self,
        current: &Layers,
        changed: BTreeSet<RepoPath>,
    ) -> Result<Layers, Problem> {
        let listed: BTreeSet<RepoPath> = self
            .watch
            .listed()
            .unwrap_or_default()
            .into_iter()
            .collect();
        let files: Vec<File> = changed
            .iter()
            .map(|path| File {
                path: path.clone(),
                read: if listed.contains(path) {
                    Read::Disk
                } else {
                    Read::Gone
                },
            })
            .collect();
        let rules = &current.rules;
        let fresh = program::build(&self.repo.git, &files, &rules.rules, self.repo.threads)?;
        Ok(Layers {
            rules: Arc::clone(rules),
            base: Arc::clone(&current.base),
            changes: fresh,
            changed,
        })
    }

    /// The index's size, or `None` before the first search.
    pub(crate) fn status(&self) -> Option<WorktreeStatus> {
        self.current().map(|layers| WorktreeStatus {
            // A search for nothing counts the documents it would search.
            documents: search(
                &[&layers.base.index, &layers.changes.index],
                &Query::default(),
                &Filter::default(),
                0,
            )
            .documents,
            changed: layers.changed.len(),
            bytes: layers.base.bytes() + layers.changes.bytes(),
        })
    }
}

/// One result of `/api/search`.
#[derive(Debug, Serialize)]
pub(crate) struct ResultJson {
    path: String,
    url: String,
    title: Option<String>,
    #[serde(rename = "type")]
    doc_type: String,
    block: usize,
    headings: Vec<String>,
    anchor: Option<String>,
    snippet: Vec<driftmark_core::search::Piece>,
}

/// The answer of `/api/search`.
#[derive(Debug, Serialize)]
pub(crate) struct Answer {
    schema_version: u32,
    query: String,
    revision: String,
    commit: String,
    documents: usize,
    total: usize,
    results: Vec<ResultJson>,
    skipped: Vec<Unindexed>,
}

/// Runs a search.
///
/// # Errors
///
/// A missing problem when the revision names no commit, or names the working
/// tree of a bare repository; a failed one when git cannot be read or the
/// rules file is broken.
pub(crate) fn answer(
    repo: &Repo,
    worktree: Option<&WorktreeIndex>,
    params: &Params,
) -> Result<Answer, Problem> {
    let revision = params
        .rev
        .clone()
        .unwrap_or_else(|| if worktree.is_some() { WORKTREE } else { "HEAD" }.to_owned());
    let (found, matches, commit, skipped) = if revision == WORKTREE {
        let worktree = worktree.ok_or(GitError::NoWorktree)?;
        let layers = worktree.layers()?;
        let found = search(
            &[&layers.base.index, &layers.changes.index],
            &params.query,
            &params.filter,
            params.limit,
        );
        let matches = program::matches(&params.query, &found, repo.threads, |hit| {
            compare::worktree_document(&repo.git, &hit.path, &layers.rules).ok()
        });
        (found, matches, WORKTREE.to_owned(), layers.unindexed())
    } else {
        let commit = repo.git.resolve(&revision)?;
        let (layer, rules) = commit_layer(repo, &commit)?;
        let found = search(&[&layer.index], &params.query, &params.filter, params.limit);
        let matches = program::matches(&params.query, &found, repo.threads, |hit| {
            let blob = layer.blobs[hit.doc].as_ref()?;
            compare::document(&repo.git, &repo.cache, blob, &rules).ok()
        });
        (
            found,
            matches,
            commit.as_hex().to_owned(),
            layer.unindexed.clone(),
        )
    };
    let results = matches
        .into_iter()
        .zip(&found.hits)
        .map(|(found, hit)| ResultJson {
            url: url(&revision, &hit.path, found.anchor.as_deref()),
            path: found.path,
            title: found.title,
            doc_type: found.doc_type,
            block: found.block,
            headings: found.headings,
            anchor: found.anchor,
            snippet: found.snippet,
        })
        .collect();
    Ok(Answer {
        schema_version: super::app::SCHEMA_VERSION,
        query: params.text.clone(),
        revision,
        commit,
        documents: found.documents,
        total: found.total,
        results,
        skipped,
    })
}

/// The search index of every Markdown file of `commit`, with the rules file
/// it was built with. It is built on first use and kept in the cache under
/// the commit's tree id.
///
/// # Errors
///
/// When git cannot be read or the rules file is broken.
pub(crate) fn commit_layer(
    repo: &Repo,
    commit: &ObjectId,
) -> Result<(Arc<Layer>, Arc<RulesAt>), Problem> {
    let tree = tree_of(&repo.git.read_commit(commit)?).ok_or_else(|| {
        Problem::failed(format!(
            "git could not be read: commit {commit} names no tree"
        ))
    })?;
    let rules = repo.rules_at(&Snapshot::Commit(commit.clone()))?;
    let layer = repo.cache.get_or_load(&Key::Search(tree), || {
        let files: Vec<File> = repo
            .git
            .markdown_files(&Snapshot::Commit(commit.clone()), &Roots::all())?
            .into_iter()
            .map(|file| File {
                path: file.path,
                read: Read::Blob(file.id),
            })
            .collect();
        let layer = program::build(&repo.git, &files, &rules.rules, repo.threads)?;
        let bytes = layer.bytes();
        Ok::<_, Problem>(Sized {
            value: layer,
            bytes,
        })
    })?;
    Ok((layer, rules))
}

/// Characters left as they are in an anchor after `#`: letters, digits, `-`
/// and `_`, the characters a heading anchor holds besides other scripts.
const ANCHOR: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'_');

/// The address of a result: the document's reading page at the revision,
/// with the section's anchor.
fn url(revision: &str, path: &RepoPath, anchor: Option<&str>) -> String {
    let page = address::document(revision, path);
    match anchor {
        Some(anchor) => format!("{page}#{}", utf8_percent_encode(anchor, ANCHOR)),
        None => page,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(raw: &str) -> Result<Params, String> {
        Params::read(Some(raw))
    }

    #[test]
    fn parameters_are_read_from_the_address() {
        let params = read("q=cache+%22one%20hour%22&rev=main&path=docs%2F&type=decision&type=&type=feature&limit=5&other=x").unwrap();
        assert_eq!(params.text, "cache \"one hour\"");
        assert_eq!(params.query, Query::parse("cache \"one hour\"").unwrap());
        assert_eq!(params.rev.as_deref(), Some("main"));
        assert_eq!(params.filter.path.as_deref(), Some("docs/"));
        assert_eq!(params.filter.types, ["decision", "feature"]);
        assert_eq!(params.limit, 5);
        let empty = Params::read(None).unwrap();
        assert_eq!(
            (empty.text.as_str(), empty.rev, empty.limit),
            ("", None, DEFAULT_LIMIT)
        );
        assert!(empty.query.is_empty());
        let blank = read("q&rev=&path=").unwrap();
        assert_eq!((blank.rev, blank.filter.path), (None, None));
    }

    #[test]
    fn bad_parameters_are_refused_with_a_reason() {
        for limit in ["0", "101", "ten", ""] {
            assert_eq!(
                read(&format!("q=a&limit={limit}")).unwrap_err(),
                "`limit` must be a number from 1 to 100"
            );
        }
        assert_eq!(read("limit=100").unwrap().limit, 100);
        assert_eq!(
            read("q=%FF").unwrap_err(),
            "a search parameter is not valid text"
        );
        assert_eq!(
            read("%FF=1").unwrap_err(),
            "a search parameter is not valid text"
        );
        let words = vec!["w"; 33].join("+");
        assert_eq!(
            read(&format!("q={words}")).unwrap_err(),
            "the query holds 33 words; a query holds at most 32"
        );
    }

    #[test]
    fn result_addresses_lead_to_the_section() {
        let path = RepoPath::new("docs/a b.md").unwrap();
        assert_eq!(url("main", &path, None), "/doc/main/-/docs/a%20b.md");
        assert_eq!(
            url("worktree", &path, Some("ჯგუფი-1_a")),
            "/doc/worktree/-/docs/a%20b.md#%E1%83%AF%E1%83%92%E1%83%A3%E1%83%A4%E1%83%98-1_a"
        );
    }
}
