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
use axum::http::{StatusCode, Uri, header};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use driftmark_core::diff::{ChangeList, Summary, SummaryList};
use driftmark_core::git::{ObjectId, RepoPath, ResolvedRange, Snapshot};
use serde::Serialize;
use tokio::sync::{broadcast, watch};
use tower_http::compression::CompressionLayer;

use super::address::{self, BadAddress, RangeAddress, Revision};
use super::assets::Assets;
use super::cache::{Key, Sized, Stats};
use super::compare::Skipped;
use super::guard::{self, Access, Verdict};
use super::page::{self, Live, WorktreeLine};
use super::repo::{Problem, Repo};
use super::watch::{Message, Watch};

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
    /// Who may ask.
    pub access: Access,
    /// The page script and style sheet.
    pub assets: Assets,
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
        .route("/events", get(events))
        .route("/static/{hash}/{name}", get(asset))
        .fallback(not_found)
        .layer(CompressionLayer::new())
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
        Verdict::NeedToken { wrong } => html(
            StatusCode::UNAUTHORIZED,
            page::page(
                &app.assets,
                "Token needed",
                Live::No,
                &page::token_needed(wrong),
            ),
        ),
        Verdict::OtherHost => html(
            StatusCode::FORBIDDEN,
            page::page(
                &app.assets,
                "Not this host",
                Live::No,
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
        page::page(
            &app.assets,
            title,
            Live::No,
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
        "nothing is at `{}`; the pages are /, /doc/<revision>/-/<path>, /changes/<range> and /changes/<range>/-/<path>",
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
            let body = page::home(&home, &line);
            html(
                StatusCode::OK,
                page::page(&app.assets, "Branches", Live::Home, &body),
            )
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
    let default_name = home.default.as_ref().map_or("", |default| default.name);
    json(&HomeJson {
        schema_version: SCHEMA_VERSION,
        default_branch: home.default.as_ref().map(|default| DefaultJson {
            name: default.name,
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

async fn document_page(State(app): Shared, uri: Uri) -> Response {
    let (revision, path) = match document_address(after(&uri, "/doc/")) {
        Ok(found) => found,
        Err(problem) => return problem_page(&app, &problem),
    };
    let work = Arc::clone(&app);
    let (asked, wanted) = (revision.clone(), path.clone());
    let made = blocking(move || {
        let document = work.repo.document_at(&asked, &wanted)?;
        let body = match &document.blob {
            Some(blob) => {
                let key = Key::Reading(blob.clone(), document.rules.id.clone(), wanted.clone());
                work.repo.cache.get_or_load(&key, || {
                    let html = page::document_body(&document.parsed);
                    let bytes = html.capacity();
                    Ok::<_, Problem>(Sized { value: html, bytes })
                })?
            }
            None => Arc::new(page::document_body(&document.parsed)),
        };
        Ok((document, body))
    })
    .await;
    match made {
        Ok((document, body)) => {
            let title = match document.parsed.as_ref() {
                Ok(parsed) => parsed.meta.title.clone(),
                Err(_) => None,
            }
            .unwrap_or_else(|| path.to_lossy().into_owned());
            let commit = document.commit.as_ref().map(ObjectId::as_hex);
            let live = if commit.is_some() {
                Live::No
            } else {
                Live::Worktree
            };
            let content = page::document(&title, revision.label(), commit, &path, &body);
            html(
                StatusCode::OK,
                page::page(&app.assets, &title, live, &content),
            )
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
    let text_path = path.to_lossy();
    let parsed = document.parsed.as_ref().as_ref();
    let classification = parsed
        .ok()
        .map(|parsed| document.rules.rules.classify(&text_path, parsed));
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

async fn changes_page(State(app): Shared, uri: Uri) -> Response {
    let (range, path) = match changes_address(after(&uri, "/changes/")) {
        Ok(found) => found,
        Err(problem) => return problem_page(&app, &problem),
    };
    let work = Arc::clone(&app);
    let label = range.label.clone();
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
                    let list = with_range(&count.list, &resolved);
                    let body = page::range(&label, &list, &count.skipped);
                    let live = live_for(&resolved);
                    html(StatusCode::OK, page::page(&app.assets, &label, live, &body))
                }
                Err(problem) => problem_page(&app, &problem),
            }
        }
        Some(path) => {
            let at = path.clone();
            let made = blocking(move || {
                let resolved = work.repo.resolve(&range)?;
                let located = work.repo.locate(&resolved, &path)?;
                let make = || {
                    let changes = work.repo.document_changes(&resolved, located.clone())?;
                    let html = page::changes_body(&changes.list, &changes.skipped);
                    let bytes = html.capacity();
                    Ok::<_, Problem>(Sized { value: html, bytes })
                };
                let body = match located.page_key() {
                    Some(key) => work.repo.cache.get_or_load(&key, make)?,
                    None => Arc::new(make()?.value),
                };
                Ok((resolved, body))
            })
            .await;
            match made {
                Ok((resolved, body)) => {
                    let content = page::document_changes(&label, &at, &body);
                    let title = at.to_lossy().into_owned();
                    let live = live_for(&resolved);
                    html(
                        StatusCode::OK,
                        page::page(&app.assets, &title, live, &content),
                    )
                }
                Err(problem) => problem_page(&app, &problem),
            }
        }
    }
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

async fn asset(State(app): Shared, Path((hash, name)): Path<(String, String)>) -> Response {
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
    (
        [
            (header::CONTENT_TYPE, file.content_type),
            (header::CACHE_CONTROL, lasting),
        ],
        file.bytes,
    )
        .into_response()
}
