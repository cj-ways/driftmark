//! Where notes sit: reading the versions of a note's document, finding each
//! note again in one of them, and handing a review's notes over as a prompt.
//!
//! The notes routes and the MCP connector's note tools both use this, so an
//! agent reads the same places, the same prompt and the same refusals a
//! reader's page shows. Nothing here speaks HTTP; a [`Failure`] carries the
//! status a route would answer with, and the connector uses its sentence.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use axum::http::StatusCode;
use driftmark_core::diff::DocumentChanges;
use driftmark_core::document::Document;
use driftmark_core::git::{GitError, GitRead, ObjectId, RepoPath, ResolvedRange, Snapshot};
use driftmark_core::notes::packet::{self, Item, Round};
use driftmark_core::notes::{AnchorState, Around, Ends, Note, Placement, Side, caused, change_key};
use driftmark_core::rules::Rules;
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use serde::Serialize;

use super::address::{self, Revision};
use super::cache::RulesId;
use super::compare::{self, RulesAt};
use super::repo::{Located, Problem, Repo};
use crate::notes::{self, Review, Scope, Store, StoreError};

/// The seconds since 1970 now, for the times on notes and replies.
pub(crate) use super::page::now;

/// The notes file of the repository the server or the connector reads,
/// opened on first use.
pub(crate) struct NotesFile {
    /// Where the file is.
    path: PathBuf,
    /// Where review files go.
    pub reviews: PathBuf,
    /// The reader's name, for notes, replies and seen marks.
    pub reader: String,
    /// The open file, once opened.
    open: Mutex<Option<Arc<Store>>>,
    /// Where notes were found before.
    pub placements: Placements,
}

impl NotesFile {
    /// The notes file at `path`, with review files in `reviews`.
    pub(crate) fn new(path: PathBuf, reviews: PathBuf, reader: String) -> NotesFile {
        NotesFile {
            path,
            reviews,
            reader,
            open: Mutex::new(None),
            placements: Placements::new(),
        }
    }

    /// Where the file is.
    pub(crate) fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// The file, opened and made when needed.
    ///
    /// # Errors
    ///
    /// When the file cannot be made or opened.
    pub(crate) fn store(&self) -> Result<Arc<Store>, StoreError> {
        let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(store) = open.as_ref() {
            return Ok(Arc::clone(store));
        }
        let store = Arc::new(Store::open(&self.path)?);
        *open = Some(Arc::clone(&store));
        Ok(store)
    }

    /// The file when it exists; `None` when no note was ever made, so a read
    /// makes nothing.
    ///
    /// # Errors
    ///
    /// When the file exists but cannot be opened.
    pub(crate) fn existing(&self) -> Result<Option<Arc<Store>>, StoreError> {
        let opened = self
            .open
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some();
        if opened || self.path.exists() {
            self.store().map(Some)
        } else {
            Ok(None)
        }
    }

    /// How many notes are on a document, for its page; `None` when the file
    /// cannot be read.
    pub(crate) fn count_on(&self, path: &RepoPath) -> Option<usize> {
        match self.existing() {
            Ok(Some(store)) => store.count_on(&path.to_lossy()).ok(),
            Ok(None) => Some(0),
            Err(_) => None,
        }
    }

    /// How many notes are on any of these documents; `None` when the file
    /// cannot be read.
    pub(crate) fn count_on_any(&self, paths: &HashSet<String>) -> Option<usize> {
        match self.existing() {
            Ok(Some(store)) => store.notes_on_any(paths).ok().map(|notes| notes.len()),
            Ok(None) => Some(0),
            Err(_) => None,
        }
    }

    /// Which of these changes the reader marked seen.
    pub(crate) fn seen(&self, keys: &[String]) -> HashSet<String> {
        match self.existing() {
            Ok(Some(store)) => store.seen(&self.reader, keys).unwrap_or_default(),
            _ => HashSet::new(),
        }
    }
}

/// The longest note text, reply or summary, in bytes.
pub(crate) const MAX_TEXT: usize = 20_000;

/// Why notes work failed: the HTTP status a route answers with, and the
/// sentence. The MCP connector gives the agent the sentence alone.
pub(crate) struct Failure {
    /// The status a route answers with.
    pub status: StatusCode,
    /// What went wrong, as a sentence.
    pub message: String,
}

impl Failure {
    /// A request that is wrong, with the reason.
    pub(crate) fn bad(message: impl Into<String>) -> Failure {
        Failure {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }
}

impl From<StoreError> for Failure {
    fn from(error: StoreError) -> Failure {
        let status = match &error {
            StoreError::NoNote(_) | StoreError::NoReview(_) => StatusCode::NOT_FOUND,
            StoreError::Refused(_) => StatusCode::CONFLICT,
            StoreError::File(_) | StoreError::Newer { .. } => StatusCode::SERVICE_UNAVAILABLE,
        };
        Failure {
            status,
            message: error.to_string(),
        }
    }
}

impl From<Problem> for Failure {
    fn from(problem: Problem) -> Failure {
        Failure {
            status: if problem.missing {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            },
            message: problem.message,
        }
    }
}

impl From<address::BadAddress> for Failure {
    fn from(bad: address::BadAddress) -> Failure {
        Failure::bad(bad.0)
    }
}

// ---- reading what is asked for ---------------------------------------------

/// Escapes `%`, so text from JSON reads back as itself through the address
/// rules, which decode `%XX`.
const PERCENT: &AsciiSet = &CONTROLS.add(b'%');

/// A revision written as text, read by the address rules.
pub(crate) fn revision(text: &str) -> Result<Revision, Failure> {
    Ok(address::revision(
        &utf8_percent_encode(text, PERCENT).to_string(),
    )?)
}

/// A range written as text, read by the address rules.
pub(crate) fn range(text: &str) -> Result<address::RangeAddress, Failure> {
    Ok(address::range(
        &utf8_percent_encode(text, PERCENT).to_string(),
    )?)
}

/// A document's path written as text, read by the address rules.
pub(crate) fn document_path(text: &str) -> Result<RepoPath, Failure> {
    Ok(address::document_path(
        &utf8_percent_encode(text, PERCENT).to_string(),
    )?)
}

/// One version of a document, parsed, with the id of its bytes.
pub(crate) struct Version {
    /// The commit, or `None` for the working tree.
    pub commit: Option<ObjectId>,
    /// The blob id of its bytes.
    pub blob: String,
    /// The parsed document.
    pub document: Arc<Document>,
}

/// A document at a revision.
pub(crate) fn version_at(
    repo: &Repo,
    revision: &Revision,
    path: &RepoPath,
) -> Result<Version, Failure> {
    let found = repo.document_at(revision, path)?;
    let document = match found.parsed.as_ref() {
        Ok(document) => Arc::new(document.clone()),
        Err(error) => {
            return Err(Failure {
                status: StatusCode::UNPROCESSABLE_ENTITY,
                message: format!("`{}` cannot be read: {error}", path.to_lossy()),
            });
        }
    };
    let blob = found.blob.as_ref().map_or_else(
        || notes::blob_id(document.source.as_bytes()),
        |blob| blob.as_hex().to_owned(),
    );
    Ok(Version {
        commit: found.commit,
        blob,
        document,
    })
}

/// A document at one side of a range, or `None` when it is not there.
pub(crate) fn version_on(
    repo: &Repo,
    side: &Snapshot,
    path: &RepoPath,
) -> Result<Option<Version>, Failure> {
    let revision = match side {
        Snapshot::Commit(commit) => Revision::Name(commit.as_hex().to_owned()),
        Snapshot::Worktree => Revision::Worktree,
    };
    match version_at(repo, &revision, path) {
        Ok(version) => Ok(Some(version)),
        Err(failure) if failure.status == StatusCode::NOT_FOUND => Ok(None),
        Err(failure) => Err(failure),
    }
}

/// The rules for reading old versions: only their blocks matter.
fn plain_rules() -> RulesAt {
    RulesAt {
        id: RulesId::Default,
        rules: Rules::default(),
    }
}

/// The version a note was made on, by its blob id: from git, or from the
/// notes file when git never held it.
pub(crate) fn old_version(repo: &Repo, store: &Store, blob: &str) -> Option<Arc<Document>> {
    if let Some(id) = ObjectId::from_hex(blob) {
        match compare::document(&repo.git, &repo.cache, &id, &plain_rules()) {
            Ok(parsed) => return parsed.as_ref().as_ref().ok().map(|d| Arc::new(d.clone())),
            Err(GitError::MissingObject { .. } | GitError::WrongKind { .. }) => {}
            Err(_) => return None,
        }
    }
    let bytes = store.version(blob).ok().flatten()?;
    Document::parse(bytes).ok().map(Arc::new)
}

// ---- notes as JSON ---------------------------------------------------------

/// Where a note sits on the page that asked.
#[derive(Serialize)]
pub(crate) struct PlacedJson {
    /// `reading`, `new` or `old`: the side of the page it sits on.
    pub side: Side,
    /// How the note was found there.
    pub state: AnchorState,
    /// The first source byte it sits on; `None` when orphaned.
    pub start: Option<usize>,
    /// The byte after its last one; `None` when orphaned.
    pub end: Option<usize>,
    /// How well the quote matched, from 0 to 1.
    pub score: f64,
    /// The block the place starts in.
    pub block: Option<usize>,
    /// The 1-based first and last line.
    pub lines: Option<[usize; 2]>,
    /// The source text there, when it is not the quote.
    pub text: Option<String>,
}

impl PlacedJson {
    /// Where `placement` sits in `document`, as a page shows it.
    pub(crate) fn new(
        side: Side,
        placement: &Placement,
        document: &Document,
        quote: &str,
    ) -> PlacedJson {
        let range = placement.range.clone();
        let text = range
            .as_ref()
            .map(|range| &document.source[range.clone()])
            .filter(|text| *text != quote)
            .map(str::to_owned);
        PlacedJson {
            side,
            state: placement.state,
            start: range.as_ref().map(|range| range.start),
            end: range.as_ref().map(|range| range.end),
            score: placement.score,
            block: placement.block,
            lines: range.as_ref().map(|range| {
                let (first, last) = packet::lines(&document.source, range);
                [first, last]
            }),
            text,
        }
    }
}

/// A note as the notes routes give it: the note, where it sits on the page
/// that asked, and on a change page the changes its commits made there.
#[derive(Serialize)]
pub(crate) struct NoteJson {
    /// The note.
    #[serde(flatten)]
    pub note: Note,
    /// Where it sits on this page; `null` when it is not looked for here,
    /// such as a note on the old side of a change seen from a reading page.
    pub placement: Option<PlacedJson>,
    /// On a range's notes: the changes of the range that the note's applied
    /// commits made, for "changed because of note n12". Left out elsewhere.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caused: Option<Vec<CausedJson>>,
}

impl NoteJson {
    /// A note with its place, and no changes listed.
    pub(crate) fn new(note: Note, placement: Option<PlacedJson>) -> NoteJson {
        NoteJson {
            note,
            placement,
            caused: None,
        }
    }
}

/// Places notes on one version and pairs each with its placement.
pub(crate) fn placed(
    repo: &Repo,
    store: &Store,
    cache: &Placements,
    notes: &[Note],
    version: &Version,
    side: Side,
) -> Vec<(Placement, PlacedJson)> {
    let known: Vec<Option<Placement>> = notes
        .iter()
        .map(|note| cache.get(note.id, &version.blob))
        .collect();
    let missing: Vec<Note> = notes
        .iter()
        .zip(&known)
        .filter(|(_, known)| known.is_none())
        .map(|(note, _)| note.clone())
        .collect();
    let mut fresh = notes::place(&missing, &version.document, &version.blob, |blob| {
        old_version(repo, store, blob)
    })
    .into_iter();
    let placements: Vec<Placement> = notes
        .iter()
        .zip(known)
        .map(|(note, known)| {
            known.unwrap_or_else(|| {
                let placement = fresh.next().unwrap_or_else(lost);
                cache.put(note.id, &version.blob, placement.clone());
                placement
            })
        })
        .collect();
    placements
        .into_iter()
        .zip(notes)
        .map(|(placement, note)| {
            let json = PlacedJson::new(
                side,
                &placement,
                &version.document,
                &note.target.anchor.quote.exact,
            );
            (placement, json)
        })
        .collect()
}

/// A placement that says the note was not found.
pub(crate) fn lost() -> Placement {
    Placement {
        state: AnchorState::Orphaned,
        range: None,
        score: 0.0,
        block: None,
    }
}

/// Where notes were found before, by note and version. A note's target
/// never changes, and neither do a version's bytes, so a page opened again
/// re-anchors nothing.
pub(crate) struct Placements(Mutex<HashMap<(u64, String), Placement>>);

impl Placements {
    /// The most placements kept; past it, the memory starts again empty.
    const MOST: usize = 50_000;

    /// No placement kept yet.
    pub(crate) fn new() -> Placements {
        Placements(Mutex::new(HashMap::new()))
    }

    fn get(&self, note: u64, version: &str) -> Option<Placement> {
        let kept = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        kept.get(&(note, version.to_owned())).cloned()
    }

    fn put(&self, note: u64, version: &str, placement: Placement) {
        let mut kept = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if kept.len() >= Self::MOST {
            kept.clear();
        }
        kept.insert((note, version.to_owned()), placement);
    }
}

// ---- handing a review over -------------------------------------------------

/// Where a note of a packet sits in the version an agent will edit.
pub(crate) struct Place {
    placement: Option<Placement>,
    lines: Option<(usize, usize)>,
    current: Option<String>,
}

/// The version an agent will edit for a review: the range's newer side, or
/// the working tree (`HEAD` in a bare repository) for a document's review.
pub(crate) fn edited_side(repo: &Repo, review: &Review) -> Result<Snapshot, Failure> {
    Ok(match &review.scope {
        Scope::Range(label) => repo.resolve(&range(label)?)?.head,
        Scope::Document(_) if repo.git.workdir().is_some() => Snapshot::Worktree,
        Scope::Document(_) => Snapshot::Commit(repo.git.resolve("HEAD").map_err(Problem::from)?),
    })
}

/// Each note found again in the edited version. A note on the old side of a
/// change keeps its own lines, at the version it was made on.
fn places(
    repo: &Repo,
    store: &Store,
    cache: &Placements,
    notes: &[Note],
    edited: &Snapshot,
) -> Result<Vec<Place>, Failure> {
    let mut versions: HashMap<String, Option<Version>> = HashMap::new();
    let mut found = Vec::new();
    for note in notes {
        if note.target.side == Side::Old {
            let lines = old_version(repo, store, &note.target.blob).map(|old| {
                packet::lines(
                    &old.source,
                    &(note.target.anchor.start..note.target.anchor.end),
                )
            });
            found.push(Place {
                placement: None,
                lines,
                current: None,
            });
            continue;
        }
        let path = note.target.path.clone();
        if !versions.contains_key(&path) {
            let version = match RepoPath::new(path.as_bytes()) {
                Some(repo_path) => version_on(repo, edited, &repo_path)?,
                None => None,
            };
            versions.insert(path.clone(), version);
        }
        let place = match versions.get(&path).and_then(Option::as_ref) {
            Some(version) => {
                let (placement, json) = placed(
                    repo,
                    store,
                    cache,
                    std::slice::from_ref(note),
                    version,
                    Side::New,
                )
                .pop()
                .ok_or_else(|| Failure::bad("a note could not be placed"))?;
                Place {
                    placement: Some(placement),
                    lines: json.lines.map(|[first, last]| (first, last)),
                    current: json.text,
                }
            }
            None => Place {
                placement: Some(lost()),
                lines: None,
                current: None,
            },
        };
        found.push(place);
    }
    Ok(found)
}

/// One round of a review, each note placed in the version an agent will
/// edit: what "Copy as prompt" and "Write review file" hand over, and what
/// the MCP connector's `review_get` gives an agent.
pub(crate) struct Packet {
    /// The review.
    pub review: Review,
    /// The round.
    pub round: Round,
    /// The notes of the round, in the review's order.
    pub notes: Vec<Note>,
    /// Where each note sits.
    places: Vec<Place>,
    /// The version the places count in: a short commit id, or `worktree`.
    at: String,
}

impl Packet {
    /// Review `id`'s notes of `round`, placed.
    ///
    /// # Errors
    ///
    /// When the review does not exist, or a version cannot be read.
    pub(crate) fn of(
        repo: &Repo,
        store: &Store,
        cache: &Placements,
        id: u64,
        round: Round,
    ) -> Result<Packet, Failure> {
        let review = store.review(id)?;
        let notes: Vec<Note> = store
            .notes_in(id)?
            .into_iter()
            .filter(|note| round.includes(note))
            .collect();
        let edited = edited_side(repo, &review)?;
        let at = match &edited {
            Snapshot::Commit(commit) => commit.as_hex()[..7].to_owned(),
            Snapshot::Worktree => address::WORKTREE.to_owned(),
        };
        let places = places(repo, store, cache, &notes, &edited)?;
        Ok(Packet {
            review,
            round,
            notes,
            places,
            at,
        })
    }

    fn items(&self) -> Vec<Item<'_>> {
        self.notes
            .iter()
            .zip(&self.places)
            .map(|(note, place)| Item {
                note,
                placement: place.placement.as_ref(),
                lines: place.lines,
                current: place.current.as_deref(),
            })
            .collect()
    }

    fn head(&self) -> packet::Review<'_> {
        packet::Review {
            id: self.review.id,
            scope: self.review.scope.text(),
            at: &self.at,
        }
    }

    /// The round as a prompt to paste into an agent's session.
    pub(crate) fn prompt(&self) -> String {
        packet::prompt(&self.head(), &self.items(), self.round)
    }

    /// The round as a review file, written at the time `written` names.
    pub(crate) fn file(&self, written: &str) -> String {
        packet::review_file(&self.head(), &self.items(), self.round, written)
    }
}

// ---- changed because of a note ---------------------------------------------

/// One change of a range that a note's applied commit made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct CausedJson {
    /// The document.
    pub path: String,
    /// The change's key, as in its section's `data-keys`.
    pub key: String,
    /// Its index in the document's change list.
    pub change: usize,
    /// The commit that made it, in full.
    pub commit: String,
}

/// One document of a range, compared, with its two ends parsed the way the
/// comparison parsed them.
struct Compared {
    located: Located,
    changes: DocumentChanges,
    base: Option<Arc<Document>>,
    head: Option<Arc<Document>>,
}

/// For each note with applied commits inside `range`, the changes of the
/// range those commits made in the note's own document, by note number.
///
/// A commit is inside the range when the head holds it and the base does
/// not; for a range that ends in the working tree, `HEAD` holds it. A note
/// whose commits lie elsewhere, or whose document did not change in the
/// range, has no entry. A version that cannot be read leaves its note out
/// rather than fail the page.
pub(crate) fn caused_in(
    repo: &Repo,
    range: &ResolvedRange,
    notes: &[Note],
) -> HashMap<u64, Vec<CausedJson>> {
    let mut found = HashMap::new();
    if notes.iter().all(|note| note.applied.is_empty()) {
        return found;
    }
    let head = match &range.head {
        Snapshot::Commit(commit) => Some(commit.clone()),
        Snapshot::Worktree => repo.git.resolve("HEAD").ok(),
    };
    let Some(head) = head else {
        return found;
    };
    let mut inside: HashMap<String, Option<ObjectId>> = HashMap::new();
    let mut documents: HashMap<String, Option<Compared>> = HashMap::new();
    for note in notes {
        let Some(path) = RepoPath::new(note.target.path.as_bytes()) else {
            continue;
        };
        let mut seen = HashSet::new();
        let mut list = Vec::new();
        for applied in &note.applied {
            let commit = inside
                .entry(applied.commit.clone())
                .or_insert_with(|| within(repo, &applied.commit, &head, range.base.as_ref()));
            let Some(commit) = commit.clone() else {
                continue;
            };
            let compared = documents
                .entry(note.target.path.clone())
                .or_insert_with(|| compared(repo, range, &path));
            let Some(compared) = compared.as_ref() else {
                continue;
            };
            let Some((before, after)) = around(repo, &commit, &path, &compared.located.rules)
            else {
                continue;
            };
            let indices = caused(
                &compared.changes,
                Around {
                    before: before.as_deref(),
                    after: after.as_deref(),
                },
                Ends {
                    base: compared.base.as_deref(),
                    head: compared.head.as_deref(),
                },
            );
            for index in indices {
                let key = change_key(&note.target.path, &compared.changes.changes[index]);
                if seen.insert(key.clone()) {
                    list.push(CausedJson {
                        path: note.target.path.clone(),
                        key,
                        change: index,
                        commit: commit.as_hex().to_owned(),
                    });
                }
            }
        }
        if !list.is_empty() {
            found.insert(note.id, list);
        }
    }
    found
}

/// The commit `written` names, in full, when `head` holds it and `base`
/// does not.
fn within(
    repo: &Repo,
    written: &str,
    head: &ObjectId,
    base: Option<&ObjectId>,
) -> Option<ObjectId> {
    let commit = repo.git.resolve(written).ok()?;
    let holds = |tip: &ObjectId| {
        repo.git
            .merge_bases(&commit, tip)
            .is_ok_and(|bases| bases.contains(&commit))
    };
    (holds(head) && !base.is_some_and(holds)).then_some(commit)
}

/// One document's changes in the range, with both ends parsed by the rules
/// of the newer side, as the change list parses them.
fn compared(repo: &Repo, range: &ResolvedRange, path: &RepoPath) -> Option<Compared> {
    let located = repo.locate(range, path).ok()?;
    let changes = repo
        .document_changes(range, located.clone())
        .ok()?
        .list
        .documents
        .into_iter()
        .next()?;
    let base = match located.change.old_blob() {
        Some(blob) => Some(parsed(repo, blob, &located.rules)?),
        None => None,
    };
    let head = match (located.change.new_blob(), located.worktree) {
        (None, _) => None,
        (Some(_), true) => {
            let read = compare::worktree_document(&repo.git, path, &located.rules).ok()?;
            Some(Arc::new(read.as_ref().as_ref().ok()?.clone()))
        }
        (Some(blob), false) => Some(parsed(repo, blob, &located.rules)?),
    };
    Some(Compared {
        located,
        changes,
        base,
        head,
    })
}

/// The document at `path` before and after `commit`: at its first parent,
/// and at the commit.
#[expect(
    clippy::type_complexity,
    reason = "a pair of optional documents reads plainer than a new type"
)]
fn around(
    repo: &Repo,
    commit: &ObjectId,
    path: &RepoPath,
    rules: &RulesAt,
) -> Option<(Option<Arc<Document>>, Option<Arc<Document>>)> {
    let info = driftmark_core::git::CommitInfo::parse(&repo.git.read_commit(commit).ok()?)?;
    let at = |commit: &ObjectId| -> Option<Option<Arc<Document>>> {
        match repo.git.blob_at(commit, path).ok()? {
            Some(blob) => Some(Some(parsed(repo, &blob, rules)?)),
            None => Some(None),
        }
    };
    let before = match info.parents.first() {
        Some(parent) => at(parent)?,
        None => None,
    };
    Some((before, at(commit)?))
}

/// A blob parsed with `rules`, from the cache when it was parsed before.
fn parsed(repo: &Repo, blob: &ObjectId, rules: &RulesAt) -> Option<Arc<Document>> {
    let parsed = compare::document(&repo.git, &repo.cache, blob, rules).ok()?;
    Some(Arc::new(parsed.as_ref().as_ref().ok()?.clone()))
}
