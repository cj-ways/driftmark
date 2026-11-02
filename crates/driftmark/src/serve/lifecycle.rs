//! The status view of a revision: which documents are current, superseded,
//! archived or untouched for long (`docs/features/status-page.md`).
//!
//! `/api/status` is the server's own state, so the view's addresses take
//! their name from what it shows: `/lifecycle/<revision>` for the page and
//! `/api/lifecycle/<revision>` for the JSON. The MCP connector's
//! `documents_status` answers from the same view.
//!
//! A commit's documents, classified, and the last change of each of its
//! files are kept in the cache by the commit's id, so only the first
//! request reads them. The age of each document is worked out again on
//! every request, from the time now. The working tree is never kept: its
//! files that git calls unchanged since `HEAD` take `HEAD`'s entries, and
//! the others are read from the disk.

use std::mem::size_of;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use driftmark_core::git::{FileEntry, GitError, GitRead, LastChange, ObjectId, Roots, Snapshot};
use driftmark_core::rules::Rules;
use driftmark_core::status::{Counts, Entry, Filter, Stage, Target, Touched, View};
use serde::Serialize;

use super::address::Revision;
use super::cache::{Key, Sized};
use super::compare::RulesAt;
use super::repo::{Problem, Repo};
use crate::search::{File, Read};
use crate::status::{self, Unread};

/// How many files the working tree's view may read from the disk before it
/// gives memory back afterwards, as a bulk job does.
const BULK_READ: usize = 64;

/// The status view of one revision.
pub(crate) struct StatusAt {
    /// The commit the revision resolved to, or `None` for the working tree.
    pub commit: Option<ObjectId>,
    /// The view.
    pub view: View,
    /// The files that could not be read.
    pub skipped: Vec<Unread>,
}

/// Every document of a commit, classified, as the cache keeps it.
pub(crate) struct Statuses {
    /// Each document's file and entry, sorted by path.
    entries: Vec<(FileEntry, Entry)>,
    /// The files that could not be read.
    skipped: Vec<Unread>,
}

/// The heap memory of an entry's texts, estimated.
fn entry_bytes(entry: &Entry) -> usize {
    let text = |text: &Option<String>| text.as_ref().map_or(0, String::capacity);
    let links = |links: &[String]| links.iter().map(String::capacity).sum::<usize>();
    entry.path.capacity()
        + entry.doc_type.capacity()
        + text(&entry.title)
        + text(&entry.status)
        + text(&entry.date)
        + text(&entry.superseded_by)
        + links(&entry.status_links)
        + links(&entry.superseded_by_links)
}

/// The heap memory of the files left out, estimated.
fn skipped_bytes(skipped: &[Unread]) -> usize {
    skipped
        .iter()
        .map(|unread| size_of::<Unread>() + unread.path.len() + unread.reason.len())
        .sum()
}

impl Statuses {
    /// The heap memory it holds, estimated.
    fn bytes(&self) -> usize {
        self.entries
            .iter()
            .map(|(file, entry)| {
                size_of::<(FileEntry, Entry)>()
                    + file.path.as_bytes().len()
                    + 64
                    + entry_bytes(entry)
            })
            .sum::<usize>()
            + skipped_bytes(&self.skipped)
    }

    /// The entry of `file`, when this commit holds the same blob there.
    fn same(&self, file: &FileEntry) -> Option<&Entry> {
        let at = self
            .entries
            .binary_search_by(|(own, _)| own.path.cmp(&file.path))
            .ok()?;
        let (own, entry) = &self.entries[at];
        (own.id == file.id).then_some(entry)
    }
}

/// The working tree's documents with when each last changed, before their
/// ages are worked out: what the working tree's view is made of.
pub(crate) struct WorktreeInputs {
    /// Each document's entry and last change.
    entries: Vec<(Entry, Touched)>,
    /// The files that could not be read.
    skipped: Vec<Unread>,
}

impl WorktreeInputs {
    /// The heap memory it holds, estimated.
    fn bytes(&self) -> usize {
        self.entries
            .iter()
            .map(|(entry, _)| size_of::<(Entry, Touched)>() + 64 + entry_bytes(entry))
            .sum::<usize>()
            + skipped_bytes(&self.skipped)
    }
}

/// When `file` last changed, from the history of its commit.
fn touched(history: &[LastChange], file: &FileEntry) -> Touched {
    history
        .binary_search_by(|change| change.path.cmp(&file.path))
        .ok()
        .map_or(Touched::Uncommitted, |at| Touched::Committed {
            commit: history[at].commit.clone(),
            at: history[at].committed_at,
        })
}

/// Whether a type of the rules reads a value from a file beside the
/// document, such as cc-sdd's `spec.json`. Such a value can differ in the
/// working tree while the document does not.
fn reads_files_beside(rules: &Rules) -> bool {
    rules.types().iter().any(|rule| {
        [&rule.status, &rule.date, &rule.superseded_by]
            .into_iter()
            .flatten()
            .any(|location| location.file.is_some())
    })
}

impl Repo {
    /// The status view of a revision at the time `now`, in seconds since
    /// 1970.
    ///
    /// `watched` is the watcher's count of changes it announced, once it has
    /// listed the working tree, or `None` without a watcher. With it, the
    /// working tree's view is kept until a file changes, `HEAD` moves or the
    /// rules change: listing the working tree hashes every file, which
    /// costs more than the rest of the view.
    ///
    /// # Errors
    ///
    /// A missing problem when the revision names no commit, or names the
    /// working tree of a bare repository; a failed one when git cannot be
    /// read or the rules file is broken.
    pub(crate) fn status(
        &self,
        revision: &Revision,
        now: i64,
        watched: Option<u64>,
    ) -> Result<StatusAt, Problem> {
        match revision {
            Revision::Name(name) => {
                let commit = self.git.resolve(name)?;
                let rules = self.rules_at(&Snapshot::Commit(commit.clone()))?;
                let loaded = AtomicBool::new(false);
                let (statuses, history) = self.both(&commit, &rules, &loaded);
                self.give_back_if(&loaded);
                let (statuses, history) = (statuses?, history?);
                let entries = statuses
                    .entries
                    .iter()
                    .map(|(file, entry)| (entry.clone(), touched(&history, file)))
                    .collect();
                Ok(StatusAt {
                    commit: Some(commit),
                    view: View::build(entries, &rules.rules, now),
                    skipped: statuses.skipped.clone(),
                })
            }
            Revision::Worktree => self.worktree_status(now, watched),
        }
    }

    /// A commit's classified documents and its history, read at the same
    /// time: the history walk on one thread while the documents are read
    /// on the others.
    fn both(&self, commit: &ObjectId, rules: &RulesAt, loaded: &AtomicBool) -> Both {
        std::thread::scope(|scope| {
            let history = scope.spawn(|| self.history(commit, loaded));
            let statuses = self.statuses(commit, rules, loaded);
            let history = history
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
            (statuses, history)
        })
    }

    /// Gives memory back after a bulk read, when one ran.
    fn give_back_if(&self, loaded: &AtomicBool) {
        if loaded.load(Ordering::Acquire) {
            crate::memory::give_back(&self.git);
        }
    }

    /// Every document of `commit`, read and classified with `rules`, from
    /// the cache when it was read before.
    fn statuses(
        &self,
        commit: &ObjectId,
        rules: &RulesAt,
        loaded: &AtomicBool,
    ) -> Result<Arc<Statuses>, Problem> {
        let key = Key::Statuses(commit.clone(), rules.id.clone());
        self.cache.get_or_load(&key, || {
            loaded.store(true, Ordering::Release);
            let side = Snapshot::Commit(commit.clone());
            let files = self.git.markdown_files(&side, &Roots::all())?;
            let read: Vec<File> = files
                .iter()
                .map(|file| File {
                    path: file.path.clone(),
                    read: Read::Blob(file.id.clone()),
                })
                .collect();
            let beside = crate::presets::files_at(&self.git, &side);
            let (found, skipped) =
                status::entries(&self.git, &read, &rules.rules, &beside, self.threads)?;
            let statuses = Statuses {
                entries: found
                    .into_iter()
                    .map(|(at, entry)| (files[at].clone(), entry))
                    .collect(),
                skipped,
            };
            let bytes = statuses.bytes();
            Ok::<_, Problem>(Sized {
                value: statuses,
                bytes,
            })
        })
    }

    /// When each Markdown file of `commit` last changed, from the cache when
    /// it was walked before.
    fn history(
        &self,
        commit: &ObjectId,
        loaded: &AtomicBool,
    ) -> Result<Arc<Vec<LastChange>>, Problem> {
        self.cache
            .get_or_load(&Key::LastChanges(commit.clone()), || {
                loaded.store(true, Ordering::Release);
                let found = self.git.last_changes(commit, &Roots::all())?;
                let bytes = found
                    .iter()
                    .map(|change| size_of::<LastChange>() + change.path.as_bytes().len() + 64)
                    .sum();
                Ok::<_, Problem>(Sized {
                    value: found,
                    bytes,
                })
            })
    }

    /// The status view of the working tree, from the cache while the
    /// watcher has seen no change since it was made (see
    /// [`Repo::status`]). Rules that read a file beside a document, which
    /// the watcher does not watch, are read again every time.
    fn worktree_status(&self, now: i64, watched: Option<u64>) -> Result<StatusAt, Problem> {
        if self.git.workdir().is_none() {
            return Err(GitError::NoWorktree.into());
        }
        let rules = self.rules_at(&Snapshot::Worktree)?;
        let head = self.git.resolve("HEAD").ok();
        let inputs = match watched.filter(|_| !reads_files_beside(&rules.rules)) {
            Some(changes) => {
                let key = Key::WorktreeStatus(changes, head.clone(), rules.id.clone());
                self.cache.get_or_load(&key, || {
                    let inputs = self.worktree_inputs(&rules, head.as_ref())?;
                    let bytes = inputs.bytes();
                    Ok::<_, Problem>(Sized {
                        value: inputs,
                        bytes,
                    })
                })?
            }
            None => Arc::new(self.worktree_inputs(&rules, head.as_ref())?),
        };
        Ok(StatusAt {
            commit: None,
            view: View::build(inputs.entries.clone(), &rules.rules, now),
            skipped: inputs.skipped.clone(),
        })
    }

    /// Reads the working tree's documents. A file whose blob is the one
    /// `HEAD` holds has `HEAD`'s last change, and takes `HEAD`'s entry when
    /// both sides read the same rules; any other file is read from the disk
    /// and counts as uncommitted. The working tree is listed on one thread
    /// while `HEAD`'s documents and history are read on the others.
    fn worktree_inputs(
        &self,
        rules: &RulesAt,
        head: Option<&ObjectId>,
    ) -> Result<WorktreeInputs, Problem> {
        let loaded = AtomicBool::new(false);
        let (files, at_head) = std::thread::scope(|scope| {
            let listing =
                scope.spawn(|| self.git.markdown_files(&Snapshot::Worktree, &Roots::all()));
            let at_head = head
                .map(|head| self.head_inputs(head, rules, &loaded))
                .transpose();
            let files = listing
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
            (files, at_head)
        });
        let files = files?;
        let (committed, known, history) =
            at_head?.unwrap_or_else(|| (Vec::new(), None, Arc::new(Vec::new())));
        self.give_back_if(&loaded);
        let when = |file: &FileEntry| {
            let unchanged = committed
                .binary_search_by(|own: &FileEntry| own.path.cmp(&file.path))
                .is_ok_and(|at| committed[at].id == file.id);
            if unchanged {
                touched(&history, file)
            } else {
                Touched::Uncommitted
            }
        };
        let mut entries = Vec::with_capacity(files.len());
        let mut to_read: Vec<&FileEntry> = Vec::new();
        for file in &files {
            match known.as_ref().and_then(|known| known.same(file)) {
                Some(entry) => entries.push((entry.clone(), when(file))),
                None => to_read.push(file),
            }
        }
        let read: Vec<File> = to_read
            .iter()
            .map(|file| File {
                path: file.path.clone(),
                read: Read::Disk,
            })
            .collect();
        let beside = crate::presets::files_at(&self.git, &Snapshot::Worktree);
        let (found, skipped) =
            status::entries(&self.git, &read, &rules.rules, &beside, self.threads)?;
        if read.len() >= BULK_READ {
            crate::memory::give_back(&self.git);
        }
        entries.extend(
            found
                .into_iter()
                .map(|(at, entry)| (entry, when(to_read[at]))),
        );
        Ok(WorktreeInputs { entries, skipped })
    }

    /// What the working tree's view needs of `HEAD`: its files, its
    /// documents when they are read with the same rules as the working
    /// tree's, and its history.
    fn head_inputs(
        &self,
        head: &ObjectId,
        rules: &RulesAt,
        loaded: &AtomicBool,
    ) -> Result<HeadInputs, Problem> {
        let committed = self
            .git
            .markdown_files(&Snapshot::Commit(head.clone()), &Roots::all())?;
        let head_rules = self.rules_at(&Snapshot::Commit(head.clone()))?;
        let same = head_rules.rules == rules.rules && !reads_files_beside(&rules.rules);
        if same {
            let (statuses, history) = self.both(head, &head_rules, loaded);
            Ok((committed, Some(statuses?), history?))
        } else {
            Ok((committed, None, self.history(head, loaded)?))
        }
    }
}

/// A commit's documents and its history, each read or refused.
type Both = (
    Result<Arc<Statuses>, Problem>,
    Result<Arc<Vec<LastChange>>, Problem>,
);

/// `HEAD`'s files, its documents when the working tree reads them alike,
/// and its history.
type HeadInputs = (Vec<FileEntry>, Option<Arc<Statuses>>, Arc<Vec<LastChange>>);

/// The filter and the page of a status address: `type` (any number of
/// times), `lifecycle`, `folder`, `untouched` and `offset`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Params {
    /// The filter.
    pub filter: Filter,
    /// How many rows of the page to skip.
    pub offset: usize,
}

impl Params {
    /// Reads the query part of an address. Other parameters are ignored, and
    /// the last of a repeated one wins, except `type`.
    ///
    /// # Errors
    ///
    /// A sentence for a parameter that is not valid text, a `lifecycle`
    /// that names none, an `untouched` that is not yes or no, or an `offset`
    /// that is not a number.
    pub(crate) fn read(raw: Option<&str>) -> Result<Params, String> {
        let mut params = Params::default();
        for pair in raw.unwrap_or_default().split('&') {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            let value = decode(value)?;
            match decode(key)?.as_str() {
                "type" if !value.is_empty() => params.filter.types.push(value),
                "lifecycle" if value.is_empty() => params.filter.stage = None,
                "lifecycle" => {
                    params.filter.stage = Some(Stage::parse(&value).ok_or_else(|| {
                        format!(
                            "`lifecycle` is {value:?}; write current, superseded, archived or none"
                        )
                    })?);
                }
                "folder" => params.filter.folder = Some(value).filter(|folder| !folder.is_empty()),
                "untouched" => {
                    params.filter.untouched = match value.as_str() {
                        "" | "no" | "0" | "false" => false,
                        "yes" | "1" | "true" | "on" => true,
                        _ => return Err(format!("`untouched` is {value:?}; write yes or no")),
                    };
                }
                "offset" => {
                    params.offset = value
                        .parse()
                        .map_err(|_| format!("`offset` is {value:?}; write a whole number"))?;
                }
                _ => {}
            }
        }
        Ok(params)
    }
}

/// A parameter's value: `+` is a space, then `%XX` escapes are decoded.
fn decode(raw: &str) -> Result<String, String> {
    let spaced = raw.replace('+', " ");
    percent_encoding::percent_decode_str(&spaced)
        .decode_utf8()
        .map(std::borrow::Cow::into_owned)
        .map_err(|_| "a parameter of the address is not valid text".to_owned())
}

/// The counts of one type, or of every document, as JSON.
#[derive(Serialize)]
pub(crate) struct CountsJson<'a> {
    /// The type, or `None` for the totals.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub doc_type: Option<&'a str>,
    /// Every document.
    pub documents: usize,
    /// Current ones.
    pub current: usize,
    /// Superseded ones.
    pub superseded: usize,
    /// Archived ones.
    pub archived: usize,
    /// Those with no lifecycle.
    pub none: usize,
    /// Those untouched for long.
    pub untouched: usize,
}

impl<'a> CountsJson<'a> {
    fn of(doc_type: Option<&'a str>, counts: &Counts) -> CountsJson<'a> {
        CountsJson {
            doc_type,
            documents: counts.documents,
            current: counts.current,
            superseded: counts.superseded,
            archived: counts.archived,
            none: counts.none,
            untouched: counts.untouched,
        }
    }
}

/// What a document names as its successor, as JSON.
#[derive(Serialize)]
pub(crate) struct SuccessorJson<'a> {
    /// The text that names it, as written.
    pub text: Option<&'a str>,
    /// The path it leads to: the document, for `found` and `circle`; the
    /// path no document has, for `missing`.
    pub path: Option<&'a str>,
    /// What became of it: `found`, `circle`, `missing`, `unmatched` or
    /// `unnamed` ([`Target`]).
    pub state: &'static str,
    /// Why it is not folded under that document, or why none was found.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

/// When a document last changed, as JSON.
#[derive(Serialize)]
pub(crate) struct LastChangeJson<'a> {
    /// The commit.
    pub commit: &'a str,
    /// Its committer date, in seconds since 1970.
    pub committed_at: i64,
    /// Whole days since then.
    pub days: i64,
}

/// One document of the view, as JSON.
#[derive(Serialize)]
pub(crate) struct DocJson<'a> {
    /// Its path.
    pub path: &'a str,
    /// Its title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<&'a str>,
    /// Its type.
    #[serde(rename = "type")]
    pub doc_type: &'a str,
    /// Its status as written.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<&'a str>,
    /// Its lifecycle: `current`, `superseded`, `archived` or `none`.
    pub lifecycle: &'static str,
    /// Its date as written.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<&'a str>,
    /// What it names as its successor.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<SuccessorJson<'a>>,
    /// The superseded documents it replaced.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub replaces: Vec<&'a str>,
    /// Its last commit, or `None` when it is uncommitted.
    pub last_change: Option<LastChangeJson<'a>>,
    /// Whether it is untouched for long.
    pub untouched: bool,
}

/// The word for what became of a "superseded by".
pub(crate) fn state_of(target: &Target) -> &'static str {
    match target {
        Target::Found(_) => "found",
        Target::Circle(_) => "circle",
        Target::Missing(_) => "missing",
        Target::Unmatched => "unmatched",
        Target::Unnamed => "unnamed",
    }
}

/// Why a successor is not one to fold under, in words.
pub(crate) fn successor_problem(view: &View, target: &Target) -> Option<String> {
    match target {
        Target::Found(_) => None,
        Target::Circle(at) => Some(format!(
            "following \"superseded by\" from `{}` leads back to this document",
            view.documents[*at].path
        )),
        Target::Missing(path) => Some(format!(
            "`{path}` is not a Markdown document at this revision"
        )),
        Target::Unmatched => {
            Some("no document at this revision has that path, title or number".to_owned())
        }
        Target::Unnamed => Some("the document says it is superseded, but not by what".to_owned()),
    }
}

impl<'a> DocJson<'a> {
    /// The document at `index` of `view`.
    pub(crate) fn of(view: &'a View, index: usize) -> DocJson<'a> {
        let doc = &view.documents[index];
        DocJson {
            path: &doc.path,
            title: doc.title.as_deref(),
            doc_type: &doc.doc_type,
            status: doc.status.as_deref(),
            lifecycle: Stage::of(doc.lifecycle).name(),
            date: doc.date.as_deref(),
            superseded_by: doc.successor.as_ref().map(|successor| SuccessorJson {
                text: successor.text.as_deref(),
                path: match &successor.target {
                    Target::Found(at) | Target::Circle(at) => Some(&view.documents[*at].path),
                    Target::Missing(path) => Some(path),
                    Target::Unmatched | Target::Unnamed => None,
                },
                state: state_of(&successor.target),
                problem: successor_problem(view, &successor.target),
            }),
            replaces: doc
                .replaces
                .iter()
                .map(|&at| view.documents[at].path.as_str())
                .collect(),
            last_change: match (&doc.touched, doc.days) {
                (Touched::Committed { commit, at }, Some(days)) => Some(LastChangeJson {
                    commit: commit.as_hex(),
                    committed_at: *at,
                    days,
                }),
                _ => None,
            },
            untouched: doc.untouched,
        }
    }
}

/// The filter an answer used, as JSON.
#[derive(Serialize)]
pub(crate) struct FilterJson<'a> {
    /// The types kept; empty means every type.
    pub types: &'a [String],
    /// The lifecycle kept, or `None` for every one.
    pub lifecycle: Option<&'static str>,
    /// The path prefix kept.
    pub folder: Option<&'a str>,
    /// Whether only untouched documents are kept.
    pub untouched: bool,
}

impl<'a> FilterJson<'a> {
    pub(crate) fn of(filter: &'a Filter) -> FilterJson<'a> {
        FilterJson {
            types: &filter.types,
            lifecycle: filter.stage.map(Stage::name),
            folder: filter.folder.as_deref(),
            untouched: filter.untouched,
        }
    }
}

/// The parts of a status answer that do not depend on the documents
/// listed: the revision, the settings, the filter and the counts.
#[derive(Serialize)]
pub(crate) struct HeadJson<'a> {
    /// The server's JSON version.
    pub schema_version: u32,
    /// The revision as asked.
    pub revision: &'a str,
    /// The commit it resolved to, or `worktree`.
    pub commit: &'a str,
    /// The time the ages were counted at, in seconds since 1970.
    pub as_of: i64,
    /// After how many days a document counts as untouched.
    pub untouched_days: u32,
    /// The filter.
    pub filter: FilterJson<'a>,
    /// Every document, counted, whatever the filter.
    pub totals: CountsJson<'a>,
    /// Each type's documents, counted, whatever the filter.
    pub types: Vec<CountsJson<'a>>,
}

impl<'a> HeadJson<'a> {
    pub(crate) fn of(
        status: &'a StatusAt,
        revision: &'a str,
        filter: &'a Filter,
        now: i64,
    ) -> HeadJson<'a> {
        HeadJson {
            schema_version: super::app::SCHEMA_VERSION,
            revision,
            commit: status
                .commit
                .as_ref()
                .map_or(super::address::WORKTREE, ObjectId::as_hex),
            as_of: now,
            untouched_days: status.view.untouched_days,
            filter: FilterJson::of(filter),
            totals: CountsJson::of(None, &status.view.totals),
            types: status
                .view
                .types
                .iter()
                .map(|counts| CountsJson::of(Some(&counts.name), &counts.counts))
                .collect(),
        }
    }
}

/// The whole JSON answer of `/api/lifecycle/<revision>`.
#[derive(Serialize)]
pub(crate) struct ViewJson<'a> {
    #[serde(flatten)]
    pub head: HeadJson<'a>,
    /// The documents the filter keeps, sorted by path.
    pub documents: Vec<DocJson<'a>>,
    /// The files that could not be read.
    pub skipped: &'a [Unread],
}

impl<'a> ViewJson<'a> {
    pub(crate) fn of(
        status: &'a StatusAt,
        revision: &'a str,
        filter: &'a Filter,
        now: i64,
    ) -> ViewJson<'a> {
        ViewJson {
            head: HeadJson::of(status, revision, filter, now),
            documents: status
                .view
                .listed(filter)
                .into_iter()
                .map(|index| DocJson::of(&status.view, index))
                .collect(),
            skipped: &status.skipped,
        }
    }
}
