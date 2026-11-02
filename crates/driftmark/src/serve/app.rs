//! The routes, and the answer to each request.
//!
//! Every route that reads git runs its work on tokio's blocking threads, so
//! a large range never stalls the server's other requests. The answers are
//! compressed with gzip when the browser accepts it, except the event
//! stream, which must reach the page as it is written.

use std::borrow::Cow;
use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use axum::Router;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, Method, StatusCode, Uri, header};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use driftmark_core::diff::{ChangeList, DocumentChanges, Summary, SummaryList};
use driftmark_core::git::{ObjectId, RepoPath, ResolvedRange, Snapshot};
use driftmark_core::notes::Source;
use driftmark_core::render::{LinkKind, LinkResolver, Resolved, Section, Setup};
use serde::Serialize;
use tokio::sync::{broadcast, watch};
use tower_http::compression::CompressionLayer;

use super::address::{self, BadAddress, RangeAddress, Revision};
use super::assets::Assets;
use super::cache::{Key, Sized, Stats};
use super::compare::Skipped;
use super::guard::{self, Access, Verdict};
use super::lifecycle::{Params as StatusParams, ViewJson};
use super::marker::{self, Markers};
use super::notes;
use super::page::{self, Frame, Live, Tab, WorktreeLine};
use super::placing::NotesFile;
use super::repo::{DocumentAt, Located, Problem, Repo};
use super::search::{self, Params, WorktreeIndex, WorktreeStatus};
use super::watch::{Message, Watch};
use crate::diagram::Drawer;
use crate::notes::Via;

/// The version of the server's own JSON: the home page, a document and the
/// status. The change lists keep the change list's `schema_version`.
pub(crate) const SCHEMA_VERSION: u32 = 1;

/// How often an open event stream sends a comment, so that nothing between
/// the browser and the server closes it as idle.
const KEEP_ALIVE: Duration = Duration::from_secs(15);

/// Everything the routes share.
pub(crate) struct App {
    /// The repository.
    pub repo: Arc<Repo>,
    /// The working-tree watcher; `None` for a bare repository.
    pub watch: Option<Arc<Watch>>,
    /// The working tree's search index; `None` for a bare repository.
    pub search: Option<Arc<WorktreeIndex>>,
    /// Who may ask.
    pub access: Access,
    /// The markers handed out with pages, which tell a write from the page
    /// apart from other writes.
    pub markers: Markers,
    /// The page script, the style sheet and the other built-in files.
    pub assets: Assets,
    /// Who draws diagrams on reading pages.
    pub drawer: Arc<Drawer>,
    /// The reader's notes.
    pub notes: Arc<NotesFile>,
    /// Where the watcher announces changes.
    pub events: broadcast::Sender<Message>,
    /// Turns `true` when the server stops, which ends the event streams.
    pub stopping: watch::Receiver<bool>,
}

/// The routes.
pub(crate) fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/", get(home_page))
        .route("/api/home", get(home_json))
        .route("/api/status", get(status_json))
        .route("/doc/{*rest}", get(document_page))
        .route("/api/doc/{*rest}", get(document_json))
        .route("/changes/{*rest}", get(changes_page))
        .route("/api/changes/{*rest}", get(changes_json))
        .route("/api/search", get(search_json))
        .route("/lifecycle/{*rest}", get(lifecycle_page))
        .route("/api/lifecycle/{*rest}", get(lifecycle_json))
        .route("/api/positions/{*rest}", get(positions_json))
        .route("/api/notes", get(notes::list).post(notes::create))
        .route("/api/notes/{id}", get(notes::one).patch(notes::edit))
        .route("/api/notes/{id}/{action}", post(notes::action))
        .route("/api/reviews/{id}/{kind}", post(notes::hand_off))
        .route("/api/seen", post(notes::seen))
        .route("/events", get(events))
        .route("/static/{hash}/{name}", get(asset))
        .fallback(not_found)
        .layer(CompressionLayer::new())
        .layer(middleware::from_fn_with_state(Arc::clone(&app), caller))
        .layer(middleware::from_fn(body_guard))
        .layer(middleware::from_fn_with_state(
            Arc::clone(&app),
            access_guard,
        ))
        .layer(middleware::from_fn(security_headers))
        .with_state(app)
}

type Shared = State<Arc<App>>;

async fn security_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    guard::add_security_headers(response.headers_mut());
    response
}

async fn access_guard(State(app): Shared, request: Request, next: Next) -> Response {
    match app
        .access
        .check(request.method(), request.uri(), request.headers())
    {
        Verdict::Pass => next.run(request).await,
        Verdict::Remember { location, cookie } => (
            StatusCode::SEE_OTHER,
            [
                (header::LOCATION, location),
                (header::SET_COOKIE, cookie),
                (header::CACHE_CONTROL, "no-store".to_owned()),
            ],
        )
            .into_response(),
        Verdict::NeedToken { wrong, write: true } => json_problem(
            StatusCode::UNAUTHORIZED,
            &guard::token_to_write(wrong, app.access.cookie_name()),
        ),
        Verdict::NeedToken {
            wrong,
            write: false,
        } => html(
            StatusCode::UNAUTHORIZED,
            page::bare("Token needed", &page::token_needed(wrong)),
        ),
        Verdict::OtherHost => html(
            StatusCode::FORBIDDEN,
            page::bare(
                "Not this host",
                &page::problem(
                    false,
                    "This server listens on this machine only and answers only to requests for localhost. Open it as http://localhost or http://127.0.0.1, or start it with --addr and a token.",
                ),
            ),
        ),
    }
}

async fn body_guard(request: Request, next: Next) -> Response {
    match guard::check_body(request.method(), request.headers()) {
        Ok(()) => next.run(request).await,
        Err((status, message)) => json_problem(status, &message),
    }
}

/// Records where a write came from (decision 0012), for the route to keep
/// with the action. A write that asks to count as the page, and does not,
/// is refused with `412`, so the page can load a fresh marker and try again
/// rather than be recorded as the API.
async fn caller(State(app): Shared, mut request: Request, next: Next) -> Response {
    if matches!(*request.method(), Method::GET | Method::HEAD) {
        return next.run(request).await;
    }
    let headers = request.headers();
    let source = app.markers.source(headers);
    if source != Source::Page && headers.contains_key(marker::ONLY_HEADER) {
        return json_problem(
            StatusCode::PRECONDITION_FAILED,
            "this page's marker is out of date: the page was opened too long ago, or before the server started again; load the page again, then try again",
        );
    }
    let agent = headers
        .get(header::USER_AGENT)
        .map(|value| String::from_utf8_lossy(value.as_bytes()).into_owned());
    let via = Via::new(source, agent.as_deref());
    request.extensions_mut().insert(via);
    next.run(request).await
}

/// A whole page around `content`, with a new marker for its writes.
fn full_page(app: &App, frame: &Frame<'_>, content: &str) -> String {
    let marker = app.markers.issue(super::random_token);
    page::page(&app.assets, frame, content, marker.as_deref())
}

/// Runs git work on a blocking thread.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, Problem> + Send + 'static,
) -> Result<T, Problem> {
    tokio::task::spawn_blocking(work).await.unwrap_or_else(|_| {
        Err(Problem::failed(
            "driftmark stopped while making this page; this is a bug in driftmark".to_owned(),
        ))
    })
}

fn html(status: StatusCode, body: String) -> Response {
    (
        status,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        body,
    )
        .into_response()
}

fn json<T: Serialize>(value: &T) -> Response {
    match serde_json::to_vec(value) {
        Ok(bytes) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "application/json"),
                (header::CACHE_CONTROL, "no-cache"),
            ],
            bytes,
        )
            .into_response(),
        Err(error) => json_problem(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("the answer could not be written as JSON: {error}"),
        ),
    }
}

fn json_problem(status: StatusCode, message: &str) -> Response {
    #[derive(Serialize)]
    struct Error<'a> {
        error: &'a str,
    }
    let body = serde_json::to_vec(&Error { error: message }).unwrap_or_default();
    (
        status,
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        body,
    )
        .into_response()
}

fn status_of(problem: &Problem) -> StatusCode {
    if problem.missing {
        StatusCode::NOT_FOUND
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

fn problem_page(app: &App, problem: &Problem) -> Response {
    let title = if problem.missing {
        "Not found"
    } else {
        "Error"
    };
    html(
        status_of(problem),
        full_page(
            app,
            &Frame::plain(title, app.watch.is_some()),
            &page::problem(problem.missing, &problem.message),
        ),
    )
}

/// Answers a problem as a page or as JSON.
fn refuse(app: &App, problem: &Problem, as_json: bool) -> Response {
    if as_json {
        json_problem(status_of(problem), &problem.message)
    } else {
        problem_page(app, problem)
    }
}

impl From<BadAddress> for Problem {
    fn from(bad: BadAddress) -> Problem {
        Problem::missing(bad.0)
    }
}

async fn not_found(State(app): Shared, uri: Uri) -> Response {
    let problem = Problem::missing(format!(
        "nothing is at `{}`; the pages are /, /doc/<revision>/-/<path>, /changes/<range>, /changes/<range>/-/<path> and /lifecycle/<revision>",
        uri.path()
    ));
    refuse(&app, &problem, uri.path().starts_with("/api/"))
}

/// The raw text after a route's prefix, still percent-encoded.
fn after<'a>(uri: &'a Uri, prefix: &str) -> &'a str {
    uri.path().strip_prefix(prefix).unwrap_or_default()
}

// ---- the home page -------------------------------------------------------

/// What the home page shows of the working tree.
fn worktree_line<'a>(app: &App, latest: Option<&'a super::watch::Latest>) -> WorktreeLine<'a> {
    let Some(watch) = &app.watch else {
        return WorktreeLine::None;
    };
    if watch.latest_is_old() && !watch.counting() {
        watch.request_count();
    }
    match latest {
        None => WorktreeLine::Counting,
        Some(latest) => WorktreeLine::Counted {
            at: latest.counted_at,
            updating: watch.counting(),
            result: latest
                .counted
                .as_ref()
                .map(|(_, count)| &count.list)
                .map_err(String::as_str),
        },
    }
}

async fn home_page(State(app): Shared) -> Response {
    let repo = Arc::clone(&app.repo);
    match blocking(move || repo.home()).await {
        Ok(home) => {
            let latest = app.watch.as_ref().and_then(|watch| watch.latest());
            let line = worktree_line(&app, latest.as_deref());
            let body = page::home(&home, &line, page::now());
            let frame = Frame {
                title: "Inbox",
                live: Live::Home,
                tab: Tab::Inbox,
                worktree: app.watch.is_some(),
                revision: None,
            };
            html(StatusCode::OK, full_page(&app, &frame, &body))
        }
        Err(problem) => problem_page(&app, &problem),
    }
}

#[derive(Serialize)]
struct HomeJson<'a> {
    schema_version: u32,
    default_branch: Option<DefaultJson<'a>>,
    current_branch: Option<&'a str>,
    branches: Vec<BranchJson<'a>>,
    left_out: LeftOutJson,
    worktree: Option<WorktreeJson<'a>>,
}

#[derive(Serialize)]
struct DefaultJson<'a> {
    name: &'a str,
    tip: &'a str,
}

#[derive(Serialize)]
struct BranchJson<'a> {
    name: &'a str,
    remote: bool,
    tip: &'a str,
    committed_at: i64,
    range: String,
    base: &'a str,
    notes: &'a [String],
    documents: usize,
    skipped: usize,
    totals: Summary,
}

#[derive(Serialize)]
struct LeftOutJson {
    unchanged: usize,
    unrelated: usize,
}

#[derive(Serialize)]
struct WorktreeJson<'a> {
    range: &'static str,
    state: &'static str,
    updating: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    counted_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    base: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    notes: Option<&'a [String]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    documents: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    skipped: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    totals: Option<Summary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'a str>,
}

async fn home_json(State(app): Shared) -> Response {
    let repo = Arc::clone(&app.repo);
    let home = match blocking(move || repo.home()).await {
        Ok(home) => home,
        Err(problem) => return refuse(&app, &problem, true),
    };
    let latest = app.watch.as_ref().and_then(|watch| watch.latest());
    let line = worktree_line(&app, latest.as_deref());
    let worktree = match (&line, latest.as_deref()) {
        (WorktreeLine::None, _) => None,
        (_, None) => Some(WorktreeJson {
            range: "auto",
            state: "counting",
            updating: true,
            counted_at: None,
            base: None,
            notes: None,
            documents: None,
            skipped: None,
            totals: None,
            error: None,
        }),
        (_, Some(latest)) => {
            let updating = app.watch.as_ref().is_some_and(|watch| watch.counting());
            Some(match &latest.counted {
                Ok((info, count)) => WorktreeJson {
                    range: "auto",
                    state: "counted",
                    updating,
                    counted_at: Some(latest.counted_at),
                    base: info.base.as_deref(),
                    notes: Some(&info.notes),
                    documents: Some(count.list.documents.len()),
                    skipped: Some(count.skipped.len()),
                    totals: Some(count.list.totals),
                    error: None,
                },
                Err(error) => WorktreeJson {
                    range: "auto",
                    state: "failed",
                    updating,
                    counted_at: Some(latest.counted_at),
                    base: None,
                    notes: None,
                    documents: None,
                    skipped: None,
                    totals: None,
                    error: Some(error),
                },
            })
        }
    };
    let default_name = home
        .default
        .as_ref()
        .map_or("", |default| default.name.as_str());
    json(&HomeJson {
        schema_version: SCHEMA_VERSION,
        default_branch: home.default.as_ref().map(|default| DefaultJson {
            name: &default.name,
            tip: default.tip.as_hex(),
        }),
        current_branch: home.current.as_deref(),
        branches: home
            .changed
            .iter()
            .map(|(branch, count)| BranchJson {
                name: &branch.name,
                remote: branch.remote,
                tip: branch.tip.as_hex(),
                committed_at: branch.committed_at,
                range: format!("{default_name}...{}", branch.name),
                base: count.base.as_hex(),
                notes: &count.notes,
                documents: count.documents,
                skipped: count.skipped,
                totals: count.totals,
            })
            .collect(),
        left_out: LeftOutJson {
            unchanged: home.unchanged,
            unrelated: home.unrelated,
        },
        worktree,
    })
}

#[derive(Serialize)]
struct StatusJson {
    schema_version: u32,
    threads: usize,
    cache: Stats,
    worktree: Option<WatchJson>,
    search: SearchStatusJson,
}

#[derive(Serialize)]
struct SearchStatusJson {
    worktree: Option<WorktreeStatus>,
}

#[derive(Serialize)]
struct WatchJson {
    watching: bool,
    files: usize,
    changes: u64,
    counting: bool,
    listeners: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    counted_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    count_ms: Option<u128>,
}

async fn status_json(State(app): Shared) -> Response {
    let worktree = app.watch.as_ref().map(|watch| {
        let (changes, files, watching) = watch.state();
        let latest = watch.latest();
        WatchJson {
            watching,
            files,
            changes,
            counting: watch.counting(),
            listeners: watch.clients.load(Ordering::Acquire),
            counted_at: latest.as_ref().map(|latest| latest.counted_at),
            count_ms: latest.as_ref().map(|latest| latest.took.as_millis()),
        }
    });
    json(&StatusJson {
        schema_version: SCHEMA_VERSION,
        threads: app.repo.threads,
        cache: app.repo.cache.stats(),
        worktree,
        search: SearchStatusJson {
            worktree: app.search.as_ref().and_then(|index| index.status()),
        },
    })
}

// ---- documents -----------------------------------------------------------

/// The revision and path of `/doc/<revision>/-/<path>`.
fn document_address(raw: &str) -> Result<(Revision, RepoPath), Problem> {
    let (target, path) = address::split(raw);
    let path = path.ok_or_else(|| {
        Problem::missing("a document's address is /doc/<revision>/-/<path>".to_owned())
    })?;
    Ok((address::revision(target)?, address::document_path(path)?))
}

/// How many open branches a reading page lists as changing its document.
const WAITING_LIMIT: usize = 6;

/// The cached part of a reading page: the document's HTML and its headings.
pub(crate) struct ReadingPart {
    /// The document as HTML, or why it cannot be shown.
    html: String,
    /// The position tables of its blocks, which a page loads when the
    /// reader starts a note.
    positions: driftmark_core::render::PositionTables,
    /// The headings, for "on this page".
    outline: Vec<Section>,
    /// Whether the document could be read.
    readable: bool,
}

/// Where a document's links lead on its reading page.
///
/// A relative link to another Markdown file stays as written: the page's
/// address ends in the document's path, so the browser finds the other
/// document at the same revision. A link from the repository root, such as
/// `/docs/a.md`, becomes relative to the document's folder, so the cached
/// page serves every revision. The server shows only Markdown documents,
/// so a link to any other file, and every image, shows as text.
struct Links {
    /// `../` once per folder the document sits in.
    up: String,
}

impl Links {
    fn for_path(path: &RepoPath) -> Links {
        let folders = path.to_lossy().matches('/').count();
        Links {
            up: "../".repeat(folders),
        }
    }
}

impl LinkResolver for Links {
    fn resolve(&self, address: &str, kind: LinkKind) -> Resolved {
        if kind == LinkKind::Image {
            return Resolved::Text;
        }
        let file = address.split(['#', '?']).next().unwrap_or_default();
        let markdown = std::path::Path::new(file)
            .extension()
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case("md") || extension.eq_ignore_ascii_case("markdown")
            });
        if !markdown {
            return Resolved::Text;
        }
        match address.strip_prefix('/') {
            Some(from_root) => Resolved::Url(format!("{}{from_root}", self.up)),
            None => Resolved::Url(address.to_owned()),
        }
    }
}

fn reading_part(app: &App, document: &DocumentAt, path: &RepoPath) -> ReadingPart {
    match document.parsed.as_ref() {
        Ok(parsed) => {
            let links = Links::for_path(path);
            let setup = Setup {
                links: &links,
                diagrams: app.drawer.as_ref(),
                allowed_html: &document.rules.rules.lint().allowed_html,
            };
            let reading = driftmark_core::render::reading(parsed, &setup);
            ReadingPart {
                html: reading.html,
                positions: reading.positions,
                outline: reading.outline,
                readable: true,
            }
        }
        Err(error) => ReadingPart {
            html: format!(
                "<p class=\"problem\">This document cannot be shown: {}.</p>\n",
                page::escape(&error.to_string())
            ),
            positions: driftmark_core::render::PositionTables::default(),
            outline: Vec::new(),
            readable: false,
        },
    }
}

/// A document's reading part, from the cache when its blob was shown
/// before.
fn reading_cached(
    app: &App,
    document: &DocumentAt,
    path: &RepoPath,
) -> Result<Arc<ReadingPart>, Problem> {
    let make = || {
        let part = reading_part(app, document, path);
        let bytes = part.html.capacity()
            + part
                .positions
                .blocks
                .iter()
                .map(|table| 24 + table.capacity() * 8)
                .sum::<usize>()
            + part
                .outline
                .iter()
                .map(|section| 64 + section.id.capacity() + section.text.capacity())
                .sum::<usize>();
        Ok::<_, Problem>(Sized { value: part, bytes })
    };
    match &document.blob {
        Some(blob) => {
            let key = Key::Reading(blob.clone(), document.rules.id.clone(), path.clone());
            app.repo.cache.get_or_load(&key, make)
        }
        None => Ok(Arc::new(make()?.value)),
    }
}

/// `GET /api/positions/<revision>/-/<path>`: the position tables of a
/// reading page, which its script loads when the reader starts a note
/// (`docs/features/engine/html.md`).
async fn positions_json(State(app): Shared, uri: Uri) -> Response {
    let (revision, path) = match document_address(after(&uri, "/api/positions/")) {
        Ok(found) => found,
        Err(problem) => return refuse(&app, &problem, true),
    };
    let work = Arc::clone(&app);
    let made = blocking(move || {
        let document = work.repo.document_at(&revision, &path)?;
        reading_cached(&work, &document, &path)
    })
    .await;
    match made {
        Ok(part) => json(&part.positions),
        Err(problem) => refuse(&app, &problem, true),
    }
}

async fn document_page(State(app): Shared, uri: Uri) -> Response {
    let (revision, path) = match document_address(after(&uri, "/doc/")) {
        Ok(found) => found,
        Err(problem) => return problem_page(&app, &problem),
    };
    let work = Arc::clone(&app);
    let (asked, wanted) = (revision.clone(), path.clone());
    let made = blocking(move || {
        let document = work.repo.document_at(&asked, &wanted)?;
        let part = reading_cached(&work, &document, &wanted)?;
        // A page shows its document even when the branches cannot be read.
        let waiting = work
            .repo
            .waiting(&wanted, document.commit.as_ref(), WAITING_LIMIT)
            .unwrap_or_default();
        let notes = work.notes.count_on(&wanted);
        Ok((document, part, waiting, notes))
    })
    .await;
    match made {
        Ok((document, part, waiting, notes)) => {
            let parsed = document.parsed.as_ref().as_ref().ok();
            let title = parsed
                .and_then(|parsed| parsed.meta.title.clone())
                .unwrap_or_else(|| path.to_lossy().into_owned());
            let classification = document.classification.clone();
            let commit = document.commit.as_ref().map(ObjectId::as_hex);
            let live = if commit.is_some() {
                Live::No
            } else {
                Live::Worktree
            };
            let waiting: Vec<page::Waiting> = waiting
                .into_iter()
                .map(|on| page::Waiting {
                    range: format!("{}...{}", on.default, on.branch.name),
                    name: on.branch.name,
                    at: on.branch.committed_at,
                    changes: on.changes,
                })
                .collect();
            let content = page::reading(
                &page::Reading {
                    path: &path,
                    revision: revision.label(),
                    commit,
                    classification: classification.as_ref(),
                    fields: parsed.map_or(&[], |parsed| parsed.meta.fields.as_slice()),
                    outline: &part.outline,
                    body: &part.html,
                    readable: part.readable,
                    waiting: &waiting,
                    notes,
                },
                page::now(),
            );
            let frame = Frame {
                title: &title,
                live,
                tab: Tab::None,
                worktree: app.watch.is_some(),
                revision: Some(revision.label()),
            };
            html(StatusCode::OK, full_page(&app, &frame, &content))
        }
        Err(problem) => problem_page(&app, &problem),
    }
}

#[derive(Serialize)]
struct DocumentJson<'a> {
    schema_version: u32,
    path: Cow<'a, str>,
    revision: &'a str,
    commit: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    blob: Option<&'a str>,
    #[serde(rename = "type")]
    doc_type: Option<&'a str>,
    status: Option<&'a str>,
    title: Option<&'a str>,
    summary: Option<&'a str>,
    bytes: Option<usize>,
    lines: Option<usize>,
    source: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

async fn document_json(State(app): Shared, uri: Uri) -> Response {
    let (revision, path) = match document_address(after(&uri, "/api/doc/")) {
        Ok(found) => found,
        Err(problem) => return refuse(&app, &problem, true),
    };
    let work = Arc::clone(&app);
    let (asked, wanted) = (revision.clone(), path.clone());
    let document = match blocking(move || work.repo.document_at(&asked, &wanted)).await {
        Ok(document) => document,
        Err(problem) => return refuse(&app, &problem, true),
    };
    let parsed = document.parsed.as_ref().as_ref();
    let classification = document.classification.clone();
    json(&DocumentJson {
        schema_version: SCHEMA_VERSION,
        path: path.to_lossy(),
        revision: revision.label(),
        commit: document
            .commit
            .as_ref()
            .map_or("worktree", |id| id.as_hex()),
        blob: document.blob.as_ref().map(ObjectId::as_hex),
        doc_type: classification.as_ref().map(|c| c.doc_type.as_str()),
        status: classification.as_ref().and_then(|c| c.status.as_deref()),
        title: parsed.ok().and_then(|parsed| parsed.meta.title.as_deref()),
        summary: parsed
            .ok()
            .and_then(|parsed| parsed.meta.summary.as_deref()),
        bytes: parsed.ok().map(|parsed| parsed.source.len()),
        lines: parsed.ok().map(|parsed| parsed.source.lines().count()),
        source: parsed.ok().map(|parsed| parsed.source.as_str()),
        error: parsed.err().map(ToString::to_string),
    })
}

// ---- changes -------------------------------------------------------------

/// The range and the optional path of `/changes/<range>[/-/<path>]`.
fn changes_address(raw: &str) -> Result<(RangeAddress, Option<RepoPath>), Problem> {
    let (target, path) = address::split(raw);
    let range = address::range(target)?;
    let path = path.map(address::document_path).transpose()?;
    Ok((range, path))
}

/// A counted range's list with its range written in.
fn with_range(list: &ChangeList, range: &ResolvedRange) -> ChangeList {
    let mut list = list.clone();
    list.range = Some(driftmark_core::diff::RangeInfo::of(range));
    list
}

/// How much a range's page shows in full. Documents are taken in the
/// list's order, highest level first; one that would pass a limit is
/// skipped and listed with a link to its own page, and the next is tried.
/// A branch's review fits, and a month of a large repository still opens at
/// once, under the 100 KB first-visit limit (`docs/features/pages.md`).
struct Shown;

impl Shown {
    /// The change HTML of the documents shown, in bytes before compression.
    /// Cards compress about five to one, so this is about 40 KB on the
    /// network.
    const BYTES: usize = 200 * 1024;
    /// The changes shown, which the rail lists one by one.
    const CHANGES: usize = 300;
    /// The documents shown: each adds a section and a header.
    const DOCUMENTS: usize = 40;
    /// The documents tried, so a range of thousands never compares them
    /// all for one page.
    const TRIED: usize = 80;
}

/// The cached part of one document's changes page: the change HTML, the
/// changes for the rail, and the title.
pub(crate) struct ChangesPart {
    /// The change list with this one document, or none when it was skipped.
    list: ChangeList,
    /// Why the document could not be compared, when it could not.
    skipped: Vec<Skipped>,
    /// The document's change HTML.
    html: String,
    /// The document's title.
    title: Option<String>,
}

impl ChangesPart {
    fn document(&self) -> Option<&DocumentChanges> {
        self.list.documents.first()
    }
}

/// One located document's changes, from the cache when its two blobs were
/// compared before under the same rules.
fn changes_part(
    repo: &Repo,
    range: &ResolvedRange,
    located: &Located,
) -> Result<Arc<ChangesPart>, Problem> {
    let key = located.page_key();
    let make = || {
        let title = repo.title_of(located);
        let changes = repo.document_changes(range, located.clone())?;
        let html = changes
            .list
            .documents
            .first()
            .map_or_else(String::new, |document| {
                driftmark_core::render::changes(document, &page::change_prefix(&document.path))
            });
        let bytes = html.capacity() * 3;
        Ok::<_, Problem>(Sized {
            value: ChangesPart {
                list: changes.list,
                skipped: changes.skipped,
                html,
                title,
            },
            bytes,
        })
    };
    match key {
        Some(key) => repo.cache.get_or_load(&key, make),
        None => Ok(Arc::new(make()?.value)),
    }
}

/// What a range's page is called: the branch of `<default>...<branch>`,
/// "This checkout" for `auto`, otherwise the range.
fn range_heading(label: &str) -> String {
    if label == "auto" {
        return "This checkout".to_owned();
    }
    match label.split_once("...") {
        Some((_, head)) if !head.is_empty() => head.to_owned(),
        _ => label.to_owned(),
    }
}

/// The revisions a change page reads documents at: the newer side, and the
/// older side for deleted documents.
fn sides(range: &ResolvedRange) -> (String, Option<String>) {
    let head = match &range.head {
        Snapshot::Commit(commit) => commit.as_hex().to_owned(),
        Snapshot::Worktree => address::WORKTREE.to_owned(),
    };
    (
        head,
        range.base.as_ref().map(|base| base.as_hex().to_owned()),
    )
}

async fn changes_page(State(app): Shared, uri: Uri) -> Response {
    let (range, path) = match changes_address(after(&uri, "/changes/")) {
        Ok(found) => found,
        Err(problem) => return problem_page(&app, &problem),
    };
    let work = Arc::clone(&app);
    let label = range.label.clone();
    let single = path.clone();
    let made = blocking(move || {
        let resolved = work.repo.resolve(&range)?;
        match &path {
            None => {
                let count = work.repo.range_count(&resolved)?;
                let list = with_range(&count.list, &resolved);
                let mut parts = Vec::new();
                let (mut bytes, mut changes) = (0, 0);
                for document in list.documents.iter().take(Shown::TRIED) {
                    if parts.len() == Shown::DOCUMENTS {
                        break;
                    }
                    let Some(path) = RepoPath::new(document.path.as_bytes()) else {
                        continue;
                    };
                    if changes + document.summary.changes > Shown::CHANGES {
                        continue;
                    }
                    let located = work.repo.locate(&resolved, &path)?;
                    let part = changes_part(&work.repo, &resolved, &located)?;
                    if bytes + part.html.len() > Shown::BYTES {
                        continue;
                    }
                    bytes += part.html.len();
                    changes += document.summary.changes;
                    parts.push(part);
                }
                Ok((resolved, list, count.skipped.clone(), parts))
            }
            Some(path) => {
                let located = work.repo.locate(&resolved, path)?;
                let part = changes_part(&work.repo, &resolved, &located)?;
                let mut list = part.list.clone();
                list.range = Some(driftmark_core::diff::RangeInfo::of(&resolved));
                let skipped = part.skipped.clone();
                Ok((resolved, list, skipped, vec![part]))
            }
        }
    })
    .await;
    let (resolved, list, skipped, parts) = match made {
        Ok(made) => made,
        Err(problem) => return problem_page(&app, &problem),
    };
    let (seen, notes) = notes_of(&app, &parts, &list).await;
    let shown: Vec<page::Shown<'_>> = parts
        .iter()
        .filter_map(|part| {
            part.document().map(|document| page::Shown {
                document,
                title: part.title.clone(),
                html: &part.html,
            })
        })
        .collect();
    let heading = range_heading(&label);
    let (head, base) = sides(&resolved);
    let content = page::review(&page::Review {
        label: &label,
        heading: &heading,
        list: &list,
        shown: &shown,
        skipped: &skipped,
        head: &head,
        base: base.as_deref(),
        single: single.as_ref(),
        seen: &seen,
        notes,
    });
    let title = match &single {
        Some(path) => path.to_lossy().into_owned(),
        None => heading.clone(),
    };
    let frame = Frame {
        title: &title,
        live: live_for(&resolved),
        tab: if label == "auto" {
            Tab::Checkout
        } else {
            Tab::None
        },
        worktree: app.watch.is_some(),
        revision: None,
    };
    html(StatusCode::OK, full_page(&app, &frame, &content))
}

/// The reader's seen marks for the changes a page shows, and how many notes
/// are on the range's documents: read per request, never cached with the
/// page.
async fn notes_of(
    app: &Arc<App>,
    parts: &[Arc<ChangesPart>],
    list: &ChangeList,
) -> (std::collections::HashSet<String>, Option<usize>) {
    let keys: Vec<String> = parts
        .iter()
        .filter_map(|part| part.document())
        .flat_map(|document| {
            document
                .changes
                .iter()
                .map(|change| driftmark_core::notes::change_key(&document.path, change))
        })
        .collect();
    let paths: std::collections::HashSet<String> = list
        .documents
        .iter()
        .map(|document| document.path.clone())
        .collect();
    let work = Arc::clone(app);
    tokio::task::spawn_blocking(move || (work.notes.seen(&keys), work.notes.count_on_any(&paths)))
        .await
        .unwrap_or_default()
}

fn live_for(range: &ResolvedRange) -> Live {
    if range.head == Snapshot::Worktree {
        Live::Worktree
    } else {
        Live::No
    }
}

#[derive(Serialize)]
struct ListJson<'a, L: Serialize> {
    #[serde(flatten)]
    list: L,
    skipped: &'a [Skipped],
}

async fn changes_json(State(app): Shared, uri: Uri) -> Response {
    let (range, path) = match changes_address(after(&uri, "/api/changes/")) {
        Ok(found) => found,
        Err(problem) => return refuse(&app, &problem, true),
    };
    let work = Arc::clone(&app);
    match path {
        None => {
            let made = blocking(move || {
                let resolved = work.repo.resolve(&range)?;
                let count = work.repo.range_count(&resolved)?;
                Ok((resolved, count))
            })
            .await;
            match made {
                Ok((resolved, count)) => {
                    let info = driftmark_core::diff::RangeInfo::of(&resolved);
                    let summary = SummaryList {
                        range: Some(&info),
                        ..count.list.summary_list()
                    };
                    json(&ListJson {
                        list: summary,
                        skipped: &count.skipped,
                    })
                }
                Err(problem) => refuse(&app, &problem, true),
            }
        }
        Some(path) => {
            let made = blocking(move || {
                let resolved = work.repo.resolve(&range)?;
                let located = work.repo.locate(&resolved, &path)?;
                work.repo.document_changes(&resolved, located)
            })
            .await;
            match made {
                Ok(changes) => json(&ListJson {
                    list: &changes.list,
                    skipped: &changes.skipped,
                }),
                Err(problem) => refuse(&app, &problem, true),
            }
        }
    }
}

// ---- search ----------------------------------------------------------------

async fn search_json(State(app): Shared, uri: Uri) -> Response {
    let params = match Params::read(uri.query()) {
        Ok(params) => params,
        Err(message) => return json_problem(StatusCode::BAD_REQUEST, &message),
    };
    let work = Arc::clone(&app);
    match blocking(move || search::answer(&work.repo, work.search.as_deref(), &params)).await {
        Ok(answer) => json(&answer),
        Err(problem) => refuse(&app, &problem, true),
    }
}

// ---- the status view -------------------------------------------------------

/// How many changes the watcher announced, once it has listed the working
/// tree: the status view keeps the working tree's documents until it
/// changes.
fn watched(app: &App) -> Option<u64> {
    let (changes, _, listed) = app.watch.as_ref()?.state();
    listed.then_some(changes)
}

async fn lifecycle_page(State(app): Shared, uri: Uri) -> Response {
    let revision = match address::revision(after(&uri, "/lifecycle/")) {
        Ok(revision) => revision,
        Err(bad) => return problem_page(&app, &bad.into()),
    };
    let params = match StatusParams::read(uri.query()) {
        Ok(params) => params,
        Err(message) => {
            return html(
                StatusCode::BAD_REQUEST,
                full_page(
                    &app,
                    &Frame::plain("Bad address", app.watch.is_some()),
                    &page::problem(false, &message),
                ),
            );
        }
    };
    let now = page::now();
    let work = Arc::clone(&app);
    let asked = revision.clone();
    let watched = watched(&app);
    match blocking(move || work.repo.status(&asked, now, watched)).await {
        Ok(status) => {
            let content = page::lifecycle::render(&page::lifecycle::StatusPage {
                revision: revision.label(),
                status: &status,
                params: &params,
            });
            let frame = Frame {
                title: "Which documents are current",
                live: if status.commit.is_some() {
                    Live::No
                } else {
                    Live::Worktree
                },
                tab: Tab::None,
                worktree: app.watch.is_some(),
                revision: Some(revision.label()),
            };
            html(StatusCode::OK, full_page(&app, &frame, &content))
        }
        Err(problem) => problem_page(&app, &problem),
    }
}

async fn lifecycle_json(State(app): Shared, uri: Uri) -> Response {
    let revision = match address::revision(after(&uri, "/api/lifecycle/")) {
        Ok(revision) => revision,
        Err(bad) => return json_problem(StatusCode::NOT_FOUND, &bad.0),
    };
    let params = match StatusParams::read(uri.query()) {
        Ok(params) => params,
        Err(message) => return json_problem(StatusCode::BAD_REQUEST, &message),
    };
    let now = page::now();
    let work = Arc::clone(&app);
    let asked = revision.clone();
    let watched = watched(&app);
    match blocking(move || work.repo.status(&asked, now, watched)).await {
        Ok(status) => json(&ViewJson::of(
            &status,
            revision.label(),
            &params.filter,
            now,
        )),
        Err(problem) => refuse(&app, &problem, true),
    }
}

// ---- events and files ----------------------------------------------------

/// Counts an open event stream while it lasts.
struct Listening(Option<Arc<Watch>>);

impl Listening {
    fn new(watch: Option<Arc<Watch>>) -> Listening {
        if let Some(watch) = &watch {
            watch.clients.fetch_add(1, Ordering::AcqRel);
        }
        Listening(watch)
    }
}

impl Drop for Listening {
    fn drop(&mut self) {
        if let Some(watch) = &self.0 {
            watch.clients.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

async fn events(State(app): Shared) -> Response {
    #[derive(Serialize)]
    struct Changed<'a> {
        paths: &'a [String],
    }
    let receiver = app.events.subscribe();
    let stopping = app.stopping.clone();
    let listening = Listening::new(app.watch.clone());
    let ready = Event::default().event("ready").data("{}");
    let first = futures_util::stream::once(async move { Ok::<_, Infallible>(ready) });
    let rest = futures_util::stream::unfold(
        (receiver, stopping, listening),
        |(mut receiver, mut stopping, listening)| async move {
            let event = tokio::select! {
                message = receiver.recv() => match message {
                    Ok(Message::Changed(paths)) => Event::default()
                        .event("change")
                        .data(serde_json::to_string(&Changed { paths: &paths }).unwrap_or_default()),
                    Ok(Message::Counted) => Event::default().event("counted").data("{}"),
                    // Events were missed; the page reloads to be safe.
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        Event::default().event("change").data("{\"paths\":[]}")
                    }
                    Err(broadcast::error::RecvError::Closed) => return None,
                },
                _ = stopping.wait_for(|stopping| *stopping) => return None,
            };
            Some((Ok::<_, Infallible>(event), (receiver, stopping, listening)))
        },
    );
    Sse::new(futures_util::StreamExt::chain(first, rest))
        .keep_alive(KeepAlive::new().interval(KEEP_ALIVE))
        .into_response()
}

async fn asset(
    State(app): Shared,
    Path((hash, name)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let Some(file) = app.assets.get(&name) else {
        return refuse(
            &app,
            &Problem::missing(format!("there is no file `{name}`")),
            false,
        );
    };
    // An old address still finds the file, but the browser must ask again.
    let lasting = if file.hash == hash {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    let accepts_gzip = headers
        .get(header::ACCEPT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value.split(',').any(|coding| {
                let mut parts = coding.split(';');
                let name = parts.next().unwrap_or_default().trim();
                let refused = parts.any(|part| part.trim().replace(' ', "") == "q=0");
                (name.eq_ignore_ascii_case("gzip") || name == "*") && !refused
            })
        });
    let body = file.body(accepts_gzip);
    let mut response = (
        [
            (header::CONTENT_TYPE, file.content_type),
            (header::CACHE_CONTROL, lasting),
        ],
        body,
    )
        .into_response();
    if file.gzip {
        let headers = response.headers_mut();
        headers.insert(
            header::VARY,
            header::HeaderValue::from_static("accept-encoding"),
        );
        if accepts_gzip {
            headers.insert(
                header::CONTENT_ENCODING,
                header::HeaderValue::from_static("gzip"),
            );
        }
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_lead_to_markdown_at_the_same_revision() {
        let path = RepoPath::new("docs/features/a.md").unwrap();
        let links = Links::for_path(&path);
        let url = |address: &str| links.resolve(address, LinkKind::Link);
        assert_eq!(url("b.md#install"), Resolved::Url("b.md#install".into()));
        assert_eq!(url("../B.MARKDOWN"), Resolved::Url("../B.MARKDOWN".into()));
        assert_eq!(url("/docs/x.md"), Resolved::Url("../../docs/x.md".into()));
        assert_eq!(
            url("/x.md?plain=1"),
            Resolved::Url("../../x.md?plain=1".into())
        );
        assert_eq!(url("../src/main.rs"), Resolved::Text);
        assert_eq!(url("folder/"), Resolved::Text);
        assert_eq!(url(""), Resolved::Text);
        assert_eq!(links.resolve("cat.md", LinkKind::Image), Resolved::Text);
        let top = Links::for_path(&RepoPath::new("README.md").unwrap());
        assert_eq!(
            top.resolve("/docs/x.md", LinkKind::Link),
            Resolved::Url("docs/x.md".into())
        );
    }

    #[test]
    fn a_range_page_is_named_after_its_branch() {
        assert_eq!(range_heading("origin/main...feature/x"), "feature/x");
        assert_eq!(range_heading("main..feature"), "main..feature");
        assert_eq!(range_heading("main..."), "main...");
        assert_eq!(range_heading("auto"), "This checkout");
        assert_eq!(range_heading("since/2026-10-01"), "since/2026-10-01");
    }

    #[test]
    fn documents_are_read_at_the_newer_side() {
        let id = ObjectId::from_hex(&"a".repeat(40)).unwrap();
        let range = ResolvedRange {
            base: Some(id.clone()),
            head: Snapshot::Worktree,
            notes: Vec::new(),
        };
        assert_eq!(sides(&range), ("worktree".to_owned(), Some("a".repeat(40))));
        let range = ResolvedRange {
            base: None,
            head: Snapshot::Commit(id),
            notes: Vec::new(),
        };
        assert_eq!(sides(&range), ("a".repeat(40), None));
    }
}
