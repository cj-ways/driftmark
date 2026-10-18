//! The notes routes: making, changing and listing notes, marking changes
//! seen, and handing a review to an agent as a prompt or a review file.
//!
//! Every route reads or writes the notes file through
//! [`NotesFile`](super::placing::NotesFile), which opens it on first use. A page that only reads never makes the file: until
//! the first note or seen mark, there is nothing to read. Every request with
//! a body has already passed the token rule and the same-origin JSON rule of
//! `guard.rs`, and carries where it came from (`marker.rs`), which the notes
//! file keeps with the action. `docs/features/commands/serve.md` ("Notes")
//! is the contract.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use driftmark_core::document::Document;
use driftmark_core::git::{RepoPath, Snapshot};
use driftmark_core::notes::packet::Round;
use driftmark_core::notes::{
    Anchor, AnchorState, Ask, Note, Outcome, Placement, ReplyStance, Role, Side, Stance, Target,
    locate_text,
};
use percent_encoding::percent_decode_str;
use serde::{Deserialize, Serialize};

use super::address;
use super::app::App;
use super::page;
use super::placing::{
    Failure, MAX_TEXT, NoteJson, Packet, PlacedJson, Placements, Version, caused_in, document_path,
    placed, range, revision, version_at, version_on,
};
use super::repo::Repo;
use crate::notes::{Act, NewNote, NoteEdit, Review, Scope, Store, StoreError, Via};

/// The version of the notes routes' JSON.
pub(crate) const SCHEMA_VERSION: u32 = 1;

/// The longest quote, in bytes of source. A note sits on a sentence or a
/// few; a longer selection is refused so every search stays fast.
const MAX_QUOTE: usize = 20_000;

// ---- answers ------------------------------------------------------------------

fn json<T: Serialize>(status: StatusCode, value: &T) -> Response {
    match serde_json::to_vec(value) {
        Ok(bytes) => (
            status,
            [
                (header::CONTENT_TYPE, "application/json"),
                (header::CACHE_CONTROL, "no-store"),
            ],
            bytes,
        )
            .into_response(),
        Err(error) => problem(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("the answer could not be written as JSON: {error}"),
        ),
    }
}

fn problem(status: StatusCode, message: &str) -> Response {
    #[derive(Serialize)]
    struct Error<'a> {
        error: &'a str,
    }
    json(status, &Error { error: message })
}

impl Failure {
    fn answer(&self) -> Response {
        problem(self.status, &self.message)
    }
}

/// Runs notes work on a blocking thread: git and the notes file both block.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, Failure> + Send + 'static,
) -> Result<T, Failure> {
    tokio::task::spawn_blocking(work).await.unwrap_or_else(|_| {
        Err(Failure {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: "driftmark stopped while answering; this is a bug in driftmark".into(),
        })
    })
}

fn body<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T, Failure> {
    serde_json::from_slice(bytes).map_err(|error| Failure::bad(format!("the request: {error}")))
}

fn checked_text(name: &str, text: &str) -> Result<(), Failure> {
    if text.len() > MAX_TEXT {
        return Err(Failure::bad(format!(
            "`{name}` is longer than {MAX_TEXT} bytes"
        )));
    }
    Ok(())
}

// ---- reading what the routes name ---------------------------------------------

/// The query of a request as name and value pairs, form-decoded.
fn query(uri: &Uri) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for part in uri.query().unwrap_or_default().split('&') {
        let (name, value) = part.split_once('=').unwrap_or((part, ""));
        let decode = |text: &str| {
            percent_decode_str(&text.replace('+', " "))
                .decode_utf8_lossy()
                .into_owned()
        };
        if !name.is_empty() {
            out.insert(decode(name), decode(value));
        }
    }
    out
}

// ---- GET /api/notes ----------------------------------------------------------

#[derive(Serialize)]
struct ListJson {
    schema_version: u32,
    reader: String,
    /// The notes file, so a reader knows where the notes live.
    file: String,
    review: Option<Review>,
    /// Every review the listed notes belong to.
    reviews: Vec<Review>,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    commit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    range: Option<String>,
    notes: Vec<NoteJson>,
}

/// `GET /api/notes?path=<path>&rev=<revision>`, `?range=<range>[&path=]` or
/// `?review=<number>`.
pub(crate) async fn list(State(app): State<Arc<App>>, uri: Uri) -> Response {
    let params = query(&uri);
    let work = Arc::clone(&app);
    let listed = blocking(move || {
        let mut list = list_of(&work, &params)?;
        let mut ids: Vec<u64> = list.notes.iter().map(|entry| entry.note.review).collect();
        ids.sort_unstable();
        ids.dedup();
        if let Some(store) = work.notes.existing()? {
            for id in ids {
                list.reviews.push(store.review(id)?);
            }
        }
        Ok(list)
    });
    match listed.await {
        Ok(list) => json(StatusCode::OK, &list),
        Err(failure) => failure.answer(),
    }
}

fn list_of(app: &App, params: &HashMap<String, String>) -> Result<ListJson, Failure> {
    let notes_file = &app.notes;
    let mut list = ListJson {
        schema_version: SCHEMA_VERSION,
        reader: notes_file.reader.clone(),
        file: notes_file.path().display().to_string(),
        review: None,
        reviews: Vec::new(),
        path: None,
        revision: None,
        commit: None,
        range: None,
        notes: Vec::new(),
    };
    let store = notes_file.existing()?;
    if let Some(number) = params.get("review") {
        let id = number
            .trim_start_matches('r')
            .parse::<u64>()
            .map_err(|_| Failure::bad(format!("`{number}` is not a review number")))?;
        let store = store.ok_or(StoreError::NoReview(id))?;
        list.review = Some(store.review(id)?);
        list.notes = store
            .notes_in(id)?
            .into_iter()
            .map(|note| NoteJson::new(note, None))
            .collect();
        return Ok(list);
    }
    if let Some(label) = params.get("range") {
        let address = range(label)?;
        let path = params
            .get("path")
            .map(|text| document_path(text))
            .transpose()?;
        list.range = Some(address.label.clone());
        list.path = path.as_ref().map(|path| path.to_lossy().into_owned());
        let Some(store) = store else {
            return Ok(list);
        };
        list.review = store.find_review(&Scope::Range(address.label.clone()))?;
        list.notes = on_range(
            &app.repo,
            &store,
            &app.notes.placements,
            &address,
            path.as_ref(),
        )?;
        return Ok(list);
    }
    let path = params
        .get("path")
        .ok_or_else(|| Failure::bad("name a `path`, a `range` or a `review`"))?;
    let path = document_path(path)?;
    let asked = params.get("rev").map_or_else(
        || {
            if app.repo.git.workdir().is_some() {
                address::WORKTREE.to_owned()
            } else {
                "HEAD".to_owned()
            }
        },
        Clone::clone,
    );
    let at = revision(&asked)?;
    let version = version_at(&app.repo, &at, &path)?;
    list.path = Some(path.to_lossy().into_owned());
    list.revision = Some(asked);
    list.commit = Some(
        version
            .commit
            .as_ref()
            .map_or_else(|| address::WORKTREE.to_owned(), |c| c.as_hex().to_owned()),
    );
    let Some(store) = store else {
        return Ok(list);
    };
    list.review = store.find_review(&Scope::Document(path.to_lossy().into_owned()))?;
    list.notes = on_document(app, &store, &path, &version)?;
    Ok(list)
}

/// The notes on one document, placed in one version of it. A note on the old
/// side of a change has no place on a reading page.
fn on_document(
    app: &App,
    store: &Store,
    path: &RepoPath,
    version: &Version,
) -> Result<Vec<NoteJson>, Failure> {
    let (old, rest): (Vec<Note>, Vec<Note>) = store
        .notes_on(&path.to_lossy())?
        .into_iter()
        .partition(|note| note.target.side == Side::Old);
    let found = placed(
        &app.repo,
        store,
        &app.notes.placements,
        &rest,
        version,
        Side::Reading,
    );
    let mut notes: Vec<NoteJson> = rest
        .into_iter()
        .zip(found)
        .map(|(note, (_, json))| NoteJson::new(note, Some(json)))
        .chain(old.into_iter().map(|note| NoteJson::new(note, None)))
        .collect();
    notes.sort_by_key(|entry| entry.note.id);
    Ok(notes)
}

/// The notes on a range's changed documents, each placed on the side of the
/// change page it belongs to: a note on the old side stays at the older
/// version; any other is looked for in the newer version first, and in the
/// older one when its text was deleted.
fn on_range(
    repo: &Repo,
    store: &Store,
    cache: &Placements,
    address: &address::RangeAddress,
    only: Option<&RepoPath>,
) -> Result<Vec<NoteJson>, Failure> {
    let resolved = repo.resolve(address)?;
    let paths: HashSet<String> = match only {
        Some(path) => HashSet::from([path.to_lossy().into_owned()]),
        None => repo
            .range_count(&resolved)?
            .list
            .documents
            .iter()
            .map(|document| document.path.clone())
            .collect(),
    };
    let notes = store.notes_on_any(&paths)?;
    let mut by_path: HashMap<String, Vec<Note>> = HashMap::new();
    for note in notes {
        by_path
            .entry(note.target.path.clone())
            .or_default()
            .push(note);
    }
    let mut out = Vec::new();
    for (path, notes) in by_path {
        let Some(repo_path) = RepoPath::new(path.as_bytes()) else {
            continue;
        };
        let head = version_on(repo, &resolved.head, &repo_path)?;
        let base = match &resolved.base {
            Some(base) => version_on(repo, &Snapshot::Commit(base.clone()), &repo_path)?,
            None => None,
        };
        let (old_side, new_side): (Vec<Note>, Vec<Note>) = notes
            .into_iter()
            .partition(|note| note.target.side == Side::Old);
        let on_base: Vec<Option<PlacedJson>> = match &base {
            Some(base) => placed(repo, store, cache, &old_side, base, Side::Old)
                .into_iter()
                .map(|(_, json)| Some(json))
                .collect(),
            None => old_side.iter().map(|_| None).collect(),
        };
        for (note, placement) in old_side.into_iter().zip(on_base) {
            out.push(NoteJson::new(note, placement));
        }
        let on_head: Vec<Option<(Placement, PlacedJson)>> = match &head {
            Some(head) => placed(repo, store, cache, &new_side, head, Side::New)
                .into_iter()
                .map(Some)
                .collect(),
            None => new_side.iter().map(|_| None).collect(),
        };
        for (note, fresh) in new_side.into_iter().zip(on_head) {
            let lost = fresh
                .as_ref()
                .is_none_or(|(placement, _)| placement.state == AnchorState::Orphaned);
            let from_base = if lost {
                base.as_ref()
                    .and_then(|base| {
                        placed(
                            repo,
                            store,
                            cache,
                            std::slice::from_ref(&note),
                            base,
                            Side::Old,
                        )
                        .pop()
                    })
                    .filter(|(placement, _)| placement.state != AnchorState::Orphaned)
            } else {
                None
            };
            let placement = from_base.or(fresh).map(|(_, json)| json);
            out.push(NoteJson::new(note, placement));
        }
    }
    out.sort_by_key(|entry| entry.note.id);
    let notes: Vec<Note> = out.iter().map(|entry| entry.note.clone()).collect();
    let mut caused = caused_in(repo, &resolved, &notes);
    for entry in &mut out {
        entry.caused = Some(caused.remove(&entry.note.id).unwrap_or_default());
    }
    Ok(out)
}

// ---- POST /api/notes ------------------------------------------------------------

/// A new note, as a page sends it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
    /// The document's path from the repository root.
    path: String,
    /// On a reading page: the revision it shows.
    #[serde(default)]
    revision: Option<String>,
    /// On a change page: the range it shows.
    #[serde(default)]
    range: Option<String>,
    /// On a change page: `old` or `new`.
    #[serde(default)]
    side: Option<Side>,
    /// The source bytes the selection maps to, from the position tables.
    #[serde(default)]
    start: Option<usize>,
    #[serde(default)]
    end: Option<usize>,
    /// On a change page: the block the selection is in, its text as shown,
    /// and how many characters into the block's text it starts. Without
    /// `text`, the note is on the whole block.
    #[serde(default)]
    block: Option<usize>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    hint: Option<usize>,
    stance: Stance,
    ask: Ask,
    #[serde(default)]
    blocking: bool,
    body: String,
}

#[derive(Serialize)]
struct OneJson {
    schema_version: u32,
    note: NoteJson,
}

/// `POST /api/notes`: a new note on a reading page or a change page.
pub(crate) async fn create(
    State(app): State<Arc<App>>,
    Extension(via): Extension<Via>,
    bytes: Bytes,
) -> Response {
    let work = Arc::clone(&app);
    match blocking(move || make(&work, &bytes, &via)).await {
        Ok(note) => json(
            StatusCode::CREATED,
            &OneJson {
                schema_version: SCHEMA_VERSION,
                note,
            },
        ),
        Err(failure) => failure.answer(),
    }
}

fn make(app: &App, bytes: &[u8], via: &Via) -> Result<NoteJson, Failure> {
    let asked: Create = body(bytes)?;
    checked_text("body", &asked.body)?;
    let path = document_path(&asked.path)?;
    let text_path = path.to_lossy().into_owned();
    let (scope, version, side) = noted_on(app, &asked, &path)?;
    let document = &version.document;
    let range = selected(document, &asked)?;
    if range.end.saturating_sub(range.start) > MAX_QUOTE {
        return Err(Failure::bad(format!(
            "a note can quote at most {MAX_QUOTE} bytes; select less"
        )));
    }
    let anchor = Anchor::capture(document, range).ok_or_else(|| {
        Failure::bad("the selection is empty, white space only, or not inside the document")
    })?;
    let store = app.notes.store()?;
    if version.commit.is_none() {
        // Git does not hold working-tree bytes; keep them, so the note can
        // be followed from this version later.
        store.keep_version(&version.blob, document.source.as_bytes())?;
    }
    let note = store.add(
        &scope,
        &NewNote {
            target: Target {
                path: text_path,
                commit: version.commit.as_ref().map(|c| c.as_hex().to_owned()),
                blob: version.blob.clone(),
                side,
                anchor,
            },
            stance: asked.stance,
            ask: asked.ask,
            blocking: asked.blocking,
            body: asked.body,
            author: app.notes.reader.clone(),
        },
        page::now(),
        via,
    )?;
    let mut found = placed(
        &app.repo,
        &store,
        &app.notes.placements,
        std::slice::from_ref(&note),
        &version,
        side,
    );
    let placement = found.pop().map(|(_, json)| json);
    Ok(NoteJson::new(note, placement))
}

/// The review, the version and the side a new note belongs to: a reading
/// page's revision, or a change page's range and side.
fn noted_on(app: &App, asked: &Create, path: &RepoPath) -> Result<(Scope, Version, Side), Failure> {
    let text_path = path.to_lossy().into_owned();
    Ok(match (&asked.revision, &asked.range) {
        (Some(_), Some(_)) | (None, None) => {
            return Err(Failure::bad(
                "name either the `revision` of a reading page or the `range` of a change page",
            ));
        }
        (Some(asked_revision), None) => {
            if asked.side.is_some_and(|side| side != Side::Reading) {
                return Err(Failure::bad("a reading page has no `side`"));
            }
            let version = version_at(&app.repo, &revision(asked_revision)?, path)?;
            (Scope::Document(text_path.clone()), version, Side::Reading)
        }
        (None, Some(label)) => {
            let address = range(label)?;
            let resolved = app.repo.resolve(&address)?;
            let side = asked
                .side
                .ok_or_else(|| Failure::bad("a change page names the `side`, `old` or `new`"))?;
            let snapshot = match side {
                Side::New => resolved.head.clone(),
                Side::Old => Snapshot::Commit(resolved.base.clone().ok_or_else(|| {
                    Failure::bad("this range has no older side to put a note on")
                })?),
                Side::Reading => {
                    return Err(Failure::bad("a change page's `side` is `old` or `new`"));
                }
            };
            let version = version_on(&app.repo, &snapshot, path)?.ok_or_else(|| {
                Failure::bad(format!(
                    "`{text_path}` is not on the {} side of this range",
                    if side == Side::Old { "old" } else { "new" }
                ))
            })?;
            (Scope::Range(address.label), version, side)
        }
    })
}

/// The source bytes a new note quotes: from the position tables, or found
/// in a change card's block by its text, or the whole block.
fn selected(document: &Document, asked: &Create) -> Result<std::ops::Range<usize>, Failure> {
    Ok(match (asked.start, asked.end, asked.block) {
        (Some(start), Some(end), None) => start..end,
        (None, None, Some(block)) => {
            let found = document
                .blocks
                .get(block)
                .ok_or_else(|| Failure::bad(format!("this version has no block {block}")))?;
            match asked.text.as_deref().filter(|text| !text.trim().is_empty()) {
                None => found.range.clone(),
                Some(text) => {
                    checked_text("text", text)?;
                    locate_text(document, block, text, asked.hint.unwrap_or(0)).ok_or_else(
                        || Failure::bad("the selected text was not found in its block"),
                    )?
                }
            }
        }
        _ => {
            return Err(Failure::bad(
                "give `start` and `end` from the position tables, or a `block`",
            ));
        }
    })
}

// ---- one note -------------------------------------------------------------------

fn number(text: &str) -> Result<u64, Failure> {
    text.trim_start_matches('n').parse().map_err(|_| Failure {
        status: StatusCode::NOT_FOUND,
        message: format!("`{text}` is not a note number"),
    })
}

/// `GET /api/notes/<number>`: one note, with its thread and commits.
pub(crate) async fn one(State(app): State<Arc<App>>, Path(id): Path<String>) -> Response {
    let work = Arc::clone(&app);
    let found = blocking(move || {
        let id = number(&id)?;
        let store = work.notes.existing()?.ok_or(StoreError::NoNote(id))?;
        Ok(store.note(id)?)
    })
    .await;
    match found {
        Ok(note) => json(
            StatusCode::OK,
            &OneJson {
                schema_version: SCHEMA_VERSION,
                note: NoteJson::new(note, None),
            },
        ),
        Err(failure) => failure.answer(),
    }
}

/// A change to a note's own fields.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EditBody {
    #[serde(default)]
    stance: Option<Stance>,
    #[serde(default)]
    ask: Option<Ask>,
    #[serde(default)]
    blocking: Option<bool>,
    #[serde(default)]
    body: Option<String>,
}

/// A reply.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplyBody {
    body: String,
    #[serde(default)]
    stance: Option<ReplyStance>,
}

/// A decision.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DecideBody {
    outcome: Outcome,
    #[serde(default)]
    summary: String,
}

/// The commits that applied a note.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AppliedBody {
    commits: Vec<String>,
}

/// Nothing.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}

/// `PATCH /api/notes/<number>`: the reader changes the note's text, stance,
/// ask or blocking flag.
pub(crate) async fn edit(
    State(app): State<Arc<App>>,
    Extension(via): Extension<Via>,
    Path(id): Path<String>,
    bytes: Bytes,
) -> Response {
    act_on(app, via, id, "edit".to_owned(), bytes).await
}

/// `POST /api/notes/<number>/<action>`: the reader withdraws, replies,
/// decides or records the commits that applied the note.
pub(crate) async fn action(
    State(app): State<Arc<App>>,
    Extension(via): Extension<Via>,
    Path((id, action)): Path<(String, String)>,
    bytes: Bytes,
) -> Response {
    act_on(app, via, id, action, bytes).await
}

async fn act_on(app: Arc<App>, via: Via, id: String, action: String, bytes: Bytes) -> Response {
    let work = Arc::clone(&app);
    let done = blocking(move || {
        let id = number(&id)?;
        let act = match action.as_str() {
            "edit" => {
                let edit: EditBody = body(&bytes)?;
                if let Some(text) = &edit.body {
                    checked_text("body", text)?;
                }
                Act::Edit(NoteEdit {
                    stance: edit.stance,
                    ask: edit.ask,
                    blocking: edit.blocking,
                    body: edit.body,
                })
            }
            "withdraw" => {
                let _: Empty = body(&bytes)?;
                Act::Withdraw
            }
            "reply" => {
                let reply: ReplyBody = body(&bytes)?;
                checked_text("body", &reply.body)?;
                if reply.body.trim().is_empty() {
                    return Err(Failure::bad("a reply needs a `body`"));
                }
                Act::Reply {
                    body: reply.body,
                    stance: reply.stance,
                }
            }
            "decide" => {
                let decide: DecideBody = body(&bytes)?;
                checked_text("summary", &decide.summary)?;
                Act::Decide {
                    outcome: decide.outcome,
                    summary: decide.summary,
                }
            }
            "applied" => {
                let applied: AppliedBody = body(&bytes)?;
                let bad = applied.commits.iter().find(|commit| {
                    !(7..=64).contains(&commit.len()) || !commit.bytes().all(|b| b.is_ascii_hexdigit())
                });
                if let Some(bad) = bad {
                    return Err(Failure::bad(format!("`{bad}` is not a commit id")));
                }
                if applied.commits.is_empty() {
                    return Err(Failure::bad("name at least one commit"));
                }
                Act::Applied {
                    commits: applied.commits,
                }
            }
            other => {
                return Err(Failure {
                    status: StatusCode::NOT_FOUND,
                    message: format!(
                        "there is no action `{other}`; the actions are withdraw, reply, decide and applied"
                    ),
                });
            }
        };
        let store = work.notes.existing()?.ok_or(StoreError::NoNote(id))?;
        Ok(store.act(
            id,
            Role::Reader,
            &work.notes.reader,
            &act,
            page::now(),
            &via,
        )?)
    })
    .await;
    match done {
        Ok(note) => json(
            StatusCode::OK,
            &OneJson {
                schema_version: SCHEMA_VERSION,
                note: NoteJson::new(note, None),
            },
        ),
        Err(failure) => failure.answer(),
    }
}

// ---- seen ------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SeenBody {
    key: String,
    seen: bool,
}

#[derive(Serialize)]
struct SeenJson {
    schema_version: u32,
    key: String,
    seen: bool,
}

/// `POST /api/seen`: the reader marks a change seen, or not seen.
pub(crate) async fn seen(
    State(app): State<Arc<App>>,
    Extension(via): Extension<Via>,
    bytes: Bytes,
) -> Response {
    let work = Arc::clone(&app);
    let done = blocking(move || {
        let asked: SeenBody = body(&bytes)?;
        if asked.key.len() != 16 || !asked.key.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Failure::bad("a change's `key` is 16 hex digits"));
        }
        let store = work.notes.store()?;
        store.set_seen(
            &work.notes.reader,
            &asked.key,
            asked.seen,
            page::now(),
            &via,
        )?;
        Ok(asked)
    })
    .await;
    match done {
        Ok(asked) => json(
            StatusCode::OK,
            &SeenJson {
                schema_version: SCHEMA_VERSION,
                key: asked.key,
                seen: asked.seen,
            },
        ),
        Err(failure) => failure.answer(),
    }
}

// ---- handing a review over ------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HandOff {
    round: u8,
}

#[derive(Serialize)]
struct HandOffJson {
    schema_version: u32,
    review: Review,
    round: u8,
    /// The notes the text holds, in its order.
    notes: Vec<u64>,
    /// The notes that moved from open to sent.
    sent: Vec<u64>,
    /// The text: the prompt, or the review file's content.
    text: String,
    /// Where the review file was written.
    #[serde(skip_serializing_if = "Option::is_none")]
    file: Option<String>,
}

/// `POST /api/reviews/<number>/prompt` and `/file`: one round of a review as
/// a prompt to paste, or as a review file written next to the notes file.
pub(crate) async fn hand_off(
    State(app): State<Arc<App>>,
    Extension(via): Extension<Via>,
    Path((id, kind)): Path<(String, String)>,
    bytes: Bytes,
) -> Response {
    let work = Arc::clone(&app);
    match blocking(move || hand_over(&work, &id, &kind, &bytes, &via)).await {
        Ok(answer) => json(StatusCode::OK, &answer),
        Err(failure) => failure.answer(),
    }
}

fn hand_over(
    app: &App,
    id: &str,
    kind: &str,
    bytes: &[u8],
    via: &Via,
) -> Result<HandOffJson, Failure> {
    let id: u64 = id.trim_start_matches('r').parse().map_err(|_| Failure {
        status: StatusCode::NOT_FOUND,
        message: format!("`{id}` is not a review number"),
    })?;
    if kind != "prompt" && kind != "file" {
        return Err(Failure {
            status: StatusCode::NOT_FOUND,
            message: format!("a review is handed over as `prompt` or `file`, not `{kind}`"),
        });
    }
    let asked: HandOff = body(bytes)?;
    let round = match asked.round {
        1 => Round::Discuss,
        2 => Round::Apply,
        other => {
            return Err(Failure::bad(format!(
                "there is no round {other}; it is 1 or 2"
            )));
        }
    };
    let store = app.notes.existing()?.ok_or(StoreError::NoReview(id))?;
    let packet = Packet::of(&app.repo, &store, &app.notes.placements, id, round)?;
    let (text, file) = if kind == "prompt" {
        (packet.prompt(), None)
    } else {
        let text = packet.file(&page::utc(page::now()));
        let file = write_review(&app.notes.reviews, id, &text)?;
        (text, Some(file))
    };
    let Packet { review, notes, .. } = packet;
    let mut sent = Vec::new();
    if round == Round::Discuss {
        for note in &notes {
            if note.status == driftmark_core::notes::Status::Open {
                store.act(
                    note.id,
                    Role::Reader,
                    &app.notes.reader,
                    &Act::Send,
                    page::now(),
                    via,
                )?;
                sent.push(note.id);
            }
        }
    }
    Ok(HandOffJson {
        schema_version: SCHEMA_VERSION,
        review,
        round: asked.round,
        notes: notes.iter().map(|note| note.id).collect(),
        sent,
        text,
        file,
    })
}

/// Writes a review file as `r<number>.md` in `folder`, whole or not at all,
/// and returns its path.
fn write_review(folder: &std::path::Path, id: u64, text: &str) -> Result<String, Failure> {
    let failed = |message: String| Failure {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        message,
    };
    std::fs::create_dir_all(folder)
        .map_err(|error| failed(format!("cannot make `{}`: {error}", folder.display())))?;
    let file = folder.join(format!("r{id}.md"));
    let partial = folder.join(format!(".r{id}.md.part"));
    std::fs::write(&partial, text)
        .and_then(|()| std::fs::rename(&partial, &file))
        .map_err(|error| failed(format!("cannot write `{}`: {error}", file.display())))?;
    Ok(file.display().to_string())
}
