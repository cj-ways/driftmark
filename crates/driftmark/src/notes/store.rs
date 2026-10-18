//! The notes file: one SQLite database that `serve` and `mcp` share.
//!
//! The file is opened in WAL mode, so one process can read while another
//! writes, and a writer waits up to five seconds for another one. Its schema
//! carries a version in SQLite's `user_version`; each version adds one
//! migration, applied once, in order, inside a transaction. A file made by a
//! newer driftmark is refused rather than changed.
//!
//! Every action on a note, and every seen mark, records where its request
//! came from (decision 0012): the reader's page, the MCP connector, or any
//! other program that held the token. A note's history is read back with
//! it, and a reader's action that did not come from the page carries the
//! core's warning.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use driftmark_core::notes::{
    Action, Anchor, Applied, Ask, Decision, Event, EventAction, Note, Outcome, Quote, Refusal,
    Reply, ReplyStance, Role, Side, Source, Stance, Status, Target, next_status,
};
use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params};
use serde::Serialize;

/// The schema version this program writes.
pub const SCHEMA_VERSION: i64 = 2;

/// The migrations, by the version they lead to. Version 1 is the first
/// release's schema. A later version appends one; none is ever edited.
///
/// Version 2 adds each action's source (decision 0012): the `actions`
/// table, and a source and a `User-Agent` on each seen mark. The actions a
/// version 1 file can show are copied in with the source `unknown`: each
/// note's making, replies, the last proposal, the decision, each recorded
/// set of commits, and a withdrawal. Edits and sends left no trace in
/// version 1, so they are not in an old note's history. Who recorded old
/// commits is a guess: the reader when the name is the note's author, an
/// agent otherwise.
const MIGRATIONS: &[(i64, &str)] = &[
    (
        1,
        "
    CREATE TABLE reviews (
      id INTEGER PRIMARY KEY,
      kind TEXT NOT NULL CHECK (kind IN ('range', 'document')),
      scope TEXT NOT NULL,
      created_at INTEGER NOT NULL,
      UNIQUE (kind, scope)
    );
    CREATE TABLE notes (
      id INTEGER PRIMARY KEY,
      review INTEGER NOT NULL REFERENCES reviews (id),
      path TEXT NOT NULL,
      commit_id TEXT,
      blob TEXT NOT NULL,
      side TEXT NOT NULL,
      start_byte INTEGER NOT NULL,
      end_byte INTEGER NOT NULL,
      quote TEXT NOT NULL,
      prefix TEXT NOT NULL,
      suffix TEXT NOT NULL,
      heading_path TEXT NOT NULL,
      block INTEGER NOT NULL,
      block_hash TEXT NOT NULL,
      stance TEXT NOT NULL,
      ask TEXT NOT NULL,
      blocking INTEGER NOT NULL,
      body TEXT NOT NULL,
      status TEXT NOT NULL,
      proposal TEXT,
      decision TEXT,
      author TEXT NOT NULL,
      created_at INTEGER NOT NULL,
      updated_at INTEGER NOT NULL
    );
    CREATE INDEX notes_by_path ON notes (path);
    CREATE INDEX notes_by_review ON notes (review);
    CREATE TABLE replies (
      id INTEGER PRIMARY KEY,
      note INTEGER NOT NULL REFERENCES notes (id),
      role TEXT NOT NULL,
      by TEXT NOT NULL,
      at INTEGER NOT NULL,
      stance TEXT,
      body TEXT NOT NULL
    );
    CREATE INDEX replies_by_note ON replies (note);
    CREATE TABLE applied (
      note INTEGER NOT NULL REFERENCES notes (id),
      commit_id TEXT NOT NULL,
      by TEXT NOT NULL,
      at INTEGER NOT NULL,
      PRIMARY KEY (note, commit_id)
    );
    CREATE INDEX applied_by_commit ON applied (commit_id);
    CREATE TABLE versions (
      blob TEXT PRIMARY KEY,
      bytes BLOB NOT NULL
    );
    CREATE TABLE seen (
      reader TEXT NOT NULL,
      change TEXT NOT NULL,
      at INTEGER NOT NULL,
      PRIMARY KEY (reader, change)
    );
    ",
    ),
    (
        2,
        "
    CREATE TABLE actions (
      id INTEGER PRIMARY KEY,
      note INTEGER NOT NULL REFERENCES notes (id),
      action TEXT NOT NULL,
      role TEXT NOT NULL,
      by TEXT NOT NULL,
      at INTEGER NOT NULL,
      source TEXT NOT NULL,
      user_agent TEXT
    );
    CREATE INDEX actions_by_note ON actions (note);
    ALTER TABLE seen ADD COLUMN source TEXT NOT NULL DEFAULT 'unknown';
    ALTER TABLE seen ADD COLUMN user_agent TEXT;
    INSERT INTO actions (note, action, role, by, at, source)
      SELECT id, 'made', 'reader', author, created_at, 'unknown' FROM notes ORDER BY id;
    INSERT INTO actions (note, action, role, by, at, source)
      SELECT note, 'reply', role, by, at, 'unknown' FROM replies ORDER BY id;
    INSERT INTO actions (note, action, role, by, at, source)
      SELECT id, 'propose', 'agent',
        COALESCE(json_extract(proposal, '$.by'), ''),
        COALESCE(json_extract(proposal, '$.at'), updated_at), 'unknown'
      FROM notes WHERE json_valid(proposal) ORDER BY id;
    INSERT INTO actions (note, action, role, by, at, source)
      SELECT id, 'decide', 'reader',
        COALESCE(json_extract(decision, '$.by'), author),
        COALESCE(json_extract(decision, '$.at'), updated_at), 'unknown'
      FROM notes WHERE json_valid(decision) ORDER BY id;
    INSERT INTO actions (note, action, role, by, at, source)
      SELECT applied.note, 'applied',
        CASE WHEN applied.by = notes.author THEN 'reader' ELSE 'agent' END,
        applied.by, applied.at, 'unknown'
      FROM applied JOIN notes ON notes.id = applied.note
      GROUP BY applied.note, applied.by, applied.at
      ORDER BY applied.at, applied.note;
    INSERT INTO actions (note, action, role, by, at, source)
      SELECT id, 'withdraw', 'reader', author, updated_at, 'unknown'
      FROM notes WHERE status = 'withdrawn' ORDER BY id;
    ",
    ),
];

/// The most characters of a `User-Agent` the file keeps.
pub const AGENT_CHARS: usize = 160;

/// Where a write to the notes file came from (decision 0012).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Via {
    /// The reader's page, the MCP connector, or any other request with the
    /// token.
    pub source: Source,
    /// What the sending program calls itself: a `User-Agent`, or the MCP
    /// client's name and version. At most [`AGENT_CHARS`] characters, with
    /// no control characters; `None` when it gave nothing.
    pub user_agent: Option<String>,
}

impl Via {
    /// A source with what the sending program called itself, cut short.
    #[must_use]
    pub fn new(source: Source, user_agent: Option<&str>) -> Via {
        let user_agent = user_agent
            .map(|text| {
                text.chars()
                    .filter(|c| !c.is_control())
                    .take(AGENT_CHARS)
                    .collect::<String>()
                    .trim()
                    .to_owned()
            })
            .filter(|text| !text.is_empty());
        Via { source, user_agent }
    }
}

/// A seen mark as the file keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeenMark {
    /// When it was set, in seconds since 1970 UTC.
    pub at: i64,
    /// Where it came from.
    pub via: Via,
}

/// How long a writer waits for another process that holds the file.
const BUSY_WAIT: Duration = Duration::from_secs(5);

/// Why the notes file could not do what was asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    /// The file cannot be opened, read or written.
    File(String),
    /// A newer driftmark made the file; this one does not know its schema.
    Newer {
        /// The file's schema version.
        found: i64,
    },
    /// No note has this number.
    NoNote(u64),
    /// No review has this number.
    NoReview(u64),
    /// The loop does not allow the action, for this reason.
    Refused(Refusal),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::File(message) => write!(f, "the notes file: {message}"),
            StoreError::Newer { found } => write!(
                f,
                "the notes file has schema version {found}, made by a newer driftmark; this one knows up to {SCHEMA_VERSION}"
            ),
            StoreError::NoNote(id) => write!(f, "there is no note n{id}"),
            StoreError::NoReview(id) => write!(f, "there is no review r{id}"),
            StoreError::Refused(refusal) => write!(f, "{refusal}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(error: rusqlite::Error) -> StoreError {
        StoreError::File(error.to_string())
    }
}

/// What a review covers.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(tag = "kind", content = "scope", rename_all = "snake_case")]
pub enum Scope {
    /// The notes made on a range's change pages, by the range as written,
    /// such as `main...feature`.
    Range(String),
    /// The notes made on a document's reading pages, by its path.
    Document(String),
}

impl Scope {
    fn kind(&self) -> &'static str {
        match self {
            Scope::Range(_) => "range",
            Scope::Document(_) => "document",
        }
    }

    /// The range or the path.
    #[must_use]
    pub fn text(&self) -> &str {
        match self {
            Scope::Range(text) | Scope::Document(text) => text,
        }
    }
}

/// A review: the notes of one scope, handed to agents together.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Review {
    /// Its number, shown as `r3`.
    pub id: u64,
    /// What it covers.
    #[serde(flatten)]
    pub scope: Scope,
    /// When it was made, in seconds since 1970 UTC.
    pub created_at: i64,
}

/// A new note, before the file numbers it.
#[derive(Debug, Clone)]
pub struct NewNote {
    /// What it is made on.
    pub target: Target,
    /// What the reader thinks.
    pub stance: Stance,
    /// What the reader wants.
    pub ask: Ask,
    /// Whether it holds the work.
    pub blocking: bool,
    /// The reader's point.
    pub body: String,
    /// Who wrote it.
    pub author: String,
}

/// A change to a note's own fields; `None` keeps a field.
#[derive(Debug, Clone, Default)]
pub struct NoteEdit {
    /// A new stance.
    pub stance: Option<Stance>,
    /// A new ask.
    pub ask: Option<Ask>,
    /// A new blocking flag.
    pub blocking: Option<bool>,
    /// A new text.
    pub body: Option<String>,
}

/// Something done to a note by the reader or an agent.
#[derive(Debug, Clone)]
pub enum Act {
    /// Change its own fields (reader).
    Edit(NoteEdit),
    /// Hand it to an agent (reader).
    Send,
    /// Add a reply.
    Reply {
        /// The text.
        body: String,
        /// What it says about the note.
        stance: Option<ReplyStance>,
    },
    /// Propose a decision (agent).
    Propose {
        /// The proposed outcome.
        outcome: Outcome,
        /// What exactly.
        summary: String,
    },
    /// Decide (reader).
    Decide {
        /// The outcome.
        outcome: Outcome,
        /// What exactly.
        summary: String,
    },
    /// Record the commits that applied the decision.
    Applied {
        /// The commit ids.
        commits: Vec<String>,
    },
    /// Take it back (reader).
    Withdraw,
}

impl Act {
    fn action(&self) -> Action {
        match self {
            Act::Edit(_) => Action::Edit,
            Act::Send => Action::Send,
            Act::Reply { .. } => Action::Reply,
            Act::Propose { .. } => Action::Propose,
            Act::Decide { outcome, .. } => Action::Decide(*outcome),
            Act::Applied { .. } => Action::MarkApplied,
            Act::Withdraw => Action::Withdraw,
        }
    }
}

/// The notes file, open.
pub struct Store {
    path: PathBuf,
    connection: Mutex<Connection>,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

fn text_of<T: Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(text)) => text,
        other => format!("{other:?}"),
    }
}

fn value_of<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, StoreError> {
    serde_json::from_value(serde_json::Value::String(text.to_owned()))
        .map_err(|error| StoreError::File(format!("`{text}` is not a known value: {error}")))
}

fn json_of<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

fn from_json<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, StoreError> {
    serde_json::from_str(text).map_err(|error| StoreError::File(format!("a stored value: {error}")))
}

fn to_i64(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn to_u64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or_default()
}

fn to_usize(value: i64) -> usize {
    usize::try_from(value).unwrap_or_default()
}

/// The note columns, in the order [`note_row`] reads them.
const NOTE_COLUMNS: &str = "id, review, path, commit_id, blob, side, start_byte, end_byte, quote, prefix, suffix, heading_path, block, block_hash, stance, ask, blocking, body, status, proposal, decision, author, created_at, updated_at";

/// The raw values of one `notes` row, in the order of [`NOTE_COLUMNS`].
fn note_row(row: &Row<'_>) -> rusqlite::Result<Vec<rusqlite::types::Value>> {
    (0..24).map(|index| row.get(index)).collect()
}

/// A note from the raw values of its row, without its thread and commits.
fn read_note(values: Vec<rusqlite::types::Value>) -> Result<Note, StoreError> {
    {
        use rusqlite::types::Value;
        let mut values = values.into_iter();
        let mut next = || values.next().unwrap_or(Value::Null);
        let int = |value: Value| match value {
            Value::Integer(number) => Ok(number),
            other => Err(StoreError::File(format!(
                "expected a number, found {other:?}"
            ))),
        };
        let text = |value: Value| match value {
            Value::Text(text) => Ok(text),
            other => Err(StoreError::File(format!("expected text, found {other:?}"))),
        };
        let maybe = |value: Value| match value {
            Value::Null => Ok(None),
            Value::Text(text) => Ok(Some(text)),
            other => Err(StoreError::File(format!("expected text, found {other:?}"))),
        };
        let id = to_u64(int(next())?);
        let review = to_u64(int(next())?);
        let path = text(next())?;
        let commit = maybe(next())?;
        let blob = text(next())?;
        let side: Side = value_of(&text(next())?)?;
        let start = to_usize(int(next())?);
        let end = to_usize(int(next())?);
        let exact = text(next())?;
        let prefix = text(next())?;
        let suffix = text(next())?;
        let heading_path: Vec<String> = from_json(&text(next())?)?;
        let block = to_usize(int(next())?);
        let block_hash = text(next())?;
        let stance: Stance = value_of(&text(next())?)?;
        let ask: Ask = value_of(&text(next())?)?;
        let blocking = int(next())? != 0;
        let body = text(next())?;
        let status_text = text(next())?;
        let status = Status::from_name(&status_text)
            .ok_or_else(|| StoreError::File(format!("unknown status `{status_text}`")))?;
        let proposal = maybe(next())?
            .map(|text| from_json::<Decision>(&text))
            .transpose()?;
        let decision = maybe(next())?
            .map(|text| from_json::<Decision>(&text))
            .transpose()?;
        let author = text(next())?;
        let created_at = int(next())?;
        let updated_at = int(next())?;
        Ok(Note {
            id,
            review,
            target: Target {
                path,
                commit,
                blob,
                side,
                anchor: Anchor {
                    start,
                    end,
                    quote: Quote {
                        exact,
                        prefix,
                        suffix,
                    },
                    heading_path,
                    block,
                    block_hash,
                },
            },
            stance,
            ask,
            blocking,
            body,
            status,
            thread: Vec::new(),
            proposal,
            decision,
            applied: Vec::new(),
            author,
            created_at,
            updated_at,
            history: Vec::new(),
        })
    }
}

impl Store {
    /// Opens the notes file at `path`, making it and its folder when they
    /// do not exist, and brings its schema up to [`SCHEMA_VERSION`].
    ///
    /// # Errors
    ///
    /// When the file cannot be made or read, or a newer driftmark made it.
    pub fn open(path: &Path) -> Result<Store, StoreError> {
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder).map_err(|error| {
                StoreError::File(format!("cannot make `{}`: {error}", folder.display()))
            })?;
        }
        let mut connection = Connection::open(path)?;
        connection.busy_timeout(BUSY_WAIT)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        migrate(&mut connection)?;
        Ok(Store {
            path: path.to_path_buf(),
            connection: Mutex::new(connection),
        })
    }

    /// Where the file is.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn with<T>(
        &self,
        work: impl FnOnce(&mut Connection) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        let mut connection = self
            .connection
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        work(&mut connection)
    }

    /// The review of a scope, made when it does not exist yet.
    ///
    /// # Errors
    ///
    /// When the file cannot be written.
    pub fn review_for(&self, scope: &Scope, now: i64) -> Result<Review, StoreError> {
        self.with(|connection| {
            connection.execute(
                "INSERT INTO reviews (kind, scope, created_at) VALUES (?1, ?2, ?3) ON CONFLICT (kind, scope) DO NOTHING",
                params![scope.kind(), scope.text(), now],
            )?;
            let (id, created_at) = connection.query_row(
                "SELECT id, created_at FROM reviews WHERE kind = ?1 AND scope = ?2",
                params![scope.kind(), scope.text()],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )?;
            Ok(Review {
                id: to_u64(id),
                scope: scope.clone(),
                created_at,
            })
        })
    }

    /// The review of a scope, if it has one.
    ///
    /// # Errors
    ///
    /// When the file cannot be read.
    pub fn find_review(&self, scope: &Scope) -> Result<Option<Review>, StoreError> {
        self.with(|connection| {
            let found = connection
                .query_row(
                    "SELECT id, created_at FROM reviews WHERE kind = ?1 AND scope = ?2",
                    params![scope.kind(), scope.text()],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
                )
                .optional()?;
            Ok(found.map(|(id, created_at)| Review {
                id: to_u64(id),
                scope: scope.clone(),
                created_at,
            }))
        })
    }

    /// The review with this number.
    ///
    /// # Errors
    ///
    /// [`StoreError::NoReview`] when there is none.
    pub fn review(&self, id: u64) -> Result<Review, StoreError> {
        self.with(|connection| {
            let found = connection
                .query_row(
                    "SELECT kind, scope, created_at FROM reviews WHERE id = ?1",
                    params![number(id)],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, i64>(2)?,
                        ))
                    },
                )
                .optional()?;
            let (kind, scope, created_at) = found.ok_or(StoreError::NoReview(id))?;
            let scope = if kind == "range" {
                Scope::Range(scope)
            } else {
                Scope::Document(scope)
            };
            Ok(Review {
                id,
                scope,
                created_at,
            })
        })
    }

    /// Adds a note to the review of `scope`, open, and returns it with its
    /// number. Its history starts with its making, from `via`.
    ///
    /// # Errors
    ///
    /// When the file cannot be written.
    pub fn add(
        &self,
        scope: &Scope,
        new: &NewNote,
        now: i64,
        via: &Via,
    ) -> Result<Note, StoreError> {
        let review = self.review_for(scope, now)?;
        let id = self.with(|connection| {
            let target = &new.target;
            let anchor = &target.anchor;
            let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute(
                &format!(
                    "INSERT INTO notes ({NOTE_COLUMNS}) VALUES (NULL, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, NULL, NULL, ?19, ?20, ?20)"
                ),
                params![
                    number(review.id),
                    target.path,
                    target.commit,
                    target.blob,
                    text_of(&target.side),
                    to_i64(anchor.start),
                    to_i64(anchor.end),
                    anchor.quote.exact,
                    anchor.quote.prefix,
                    anchor.quote.suffix,
                    json_of(&anchor.heading_path),
                    to_i64(anchor.block),
                    anchor.block_hash,
                    text_of(&new.stance),
                    text_of(&new.ask),
                    i64::from(new.blocking),
                    &new.body,
                    Status::Open.name(),
                    &new.author,
                    now,
                ],
            )?;
            let id = transaction.last_insert_rowid();
            record(
                &transaction,
                id,
                EventAction::Made,
                Role::Reader,
                &new.author,
                now,
                via,
            )?;
            transaction.commit()?;
            Ok(to_u64(id))
        })?;
        self.note(id)
    }

    /// The note with this number, with its thread and commits.
    ///
    /// # Errors
    ///
    /// [`StoreError::NoNote`] when there is none.
    pub fn note(&self, id: u64) -> Result<Note, StoreError> {
        let mut found = self.select("WHERE id = ?1", &[&number(id)])?;
        found.pop().ok_or(StoreError::NoNote(id))
    }

    /// Every note, oldest first.
    ///
    /// # Errors
    ///
    /// When the file cannot be read.
    pub fn notes(&self) -> Result<Vec<Note>, StoreError> {
        self.select("ORDER BY id", &[])
    }

    /// The notes on one document, oldest first.
    ///
    /// # Errors
    ///
    /// When the file cannot be read.
    pub fn notes_on(&self, path: &str) -> Result<Vec<Note>, StoreError> {
        self.select("WHERE path = ?1 ORDER BY id", &[&path])
    }

    /// The notes on any of these documents, oldest first.
    ///
    /// # Errors
    ///
    /// When the file cannot be read.
    pub fn notes_on_any(&self, paths: &HashSet<String>) -> Result<Vec<Note>, StoreError> {
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        let all = self.notes()?;
        Ok(all
            .into_iter()
            .filter(|note| paths.contains(&note.target.path))
            .collect())
    }

    /// How many notes are on one document.
    ///
    /// # Errors
    ///
    /// When the file cannot be read.
    pub fn count_on(&self, path: &str) -> Result<usize, StoreError> {
        self.with(|connection| {
            let count: i64 = connection.query_row(
                "SELECT COUNT(*) FROM notes WHERE path = ?1",
                params![path],
                |row| row.get(0),
            )?;
            Ok(to_usize(count))
        })
    }

    /// The notes of one review, oldest first.
    ///
    /// # Errors
    ///
    /// When the file cannot be read.
    pub fn notes_in(&self, review: u64) -> Result<Vec<Note>, StoreError> {
        self.select("WHERE review = ?1 ORDER BY id", &[&number(review)])
    }

    fn select(
        &self,
        clause: &str,
        values: &[&dyn rusqlite::ToSql],
    ) -> Result<Vec<Note>, StoreError> {
        self.with(|connection| {
            let mut statement =
                connection.prepare(&format!("SELECT {NOTE_COLUMNS} FROM notes {clause}"))?;
            let rows = statement
                .query_map(values, note_row)?
                .collect::<Result<Vec<_>, _>>()?;
            let mut notes = Vec::with_capacity(rows.len());
            for values in rows {
                notes.push(read_note(values)?);
            }
            if notes.is_empty() {
                return Ok(notes);
            }
            let wanted: HashMap<u64, usize> = notes
                .iter()
                .enumerate()
                .map(|(at, note)| (note.id, at))
                .collect();
            let mut replies = connection
                .prepare("SELECT note, role, by, at, stance, body FROM replies ORDER BY id")?;
            let rows = replies.query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, String>(5)?,
                ))
            })?;
            for row in rows {
                let (note, role, by, at, stance, body) = row?;
                if let Some(&index) = wanted.get(&to_u64(note)) {
                    notes[index].thread.push(Reply {
                        role: value_of(&role)?,
                        by,
                        at,
                        stance: stance.map(|stance| value_of(&stance)).transpose()?,
                        body,
                    });
                }
            }
            let mut applied = connection
                .prepare("SELECT note, commit_id, by, at FROM applied ORDER BY at, commit_id")?;
            let rows = applied.query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })?;
            for row in rows {
                let (note, commit, by, at) = row?;
                if let Some(&index) = wanted.get(&to_u64(note)) {
                    notes[index].applied.push(Applied { commit, by, at });
                }
            }
            let mut actions = connection.prepare(
                "SELECT note, action, role, by, at, source, user_agent FROM actions ORDER BY at, id",
            )?;
            let rows = actions.query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                ))
            })?;
            for row in rows {
                let (note, action, role, by, at, source, user_agent) = row?;
                if let Some(&index) = wanted.get(&to_u64(note)) {
                    let action = EventAction::from_name(&action).ok_or_else(|| {
                        StoreError::File(format!("unknown action `{action}`"))
                    })?;
                    notes[index].history.push(Event::new(
                        action,
                        value_of(&role)?,
                        by,
                        at,
                        source_of(&source)?,
                        user_agent,
                    ));
                }
            }
            Ok(notes)
        })
    }

    /// Does `act` to note `id` as `who`, named `by`, and returns the note
    /// after it. The loop's rules decide whether it may
    /// (`driftmark_core::notes::next_status`). The note's history records
    /// the action with `via`, in the same transaction.
    ///
    /// The status is read and written in one transaction that holds the
    /// file's write lock from its start. `serve` and `mcp` write the same
    /// file, so a reader's decision and an agent's proposal can arrive at
    /// once; each then sees the status the other left, and neither writes
    /// over it.
    ///
    /// # Errors
    ///
    /// [`StoreError::NoNote`], or [`StoreError::Refused`] with the reason.
    pub fn act(
        &self,
        id: u64,
        who: Role,
        by: &str,
        act: &Act,
        now: i64,
        via: &Via,
    ) -> Result<Note, StoreError> {
        self.with(|connection| {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let key = number(id);
            let current = transaction
                .query_row(
                    "SELECT status FROM notes WHERE id = ?1",
                    params![key],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
                .ok_or(StoreError::NoNote(id))?;
            let current = Status::from_name(&current)
                .ok_or_else(|| StoreError::File(format!("unknown status `{current}`")))?;
            let status = next_status(current, who, act.action()).map_err(StoreError::Refused)?;
            match act {
                Act::Edit(edit) => {
                    transaction.execute(
                        "UPDATE notes SET stance = COALESCE(?2, stance), ask = COALESCE(?3, ask), blocking = COALESCE(?4, blocking), body = COALESCE(?5, body) WHERE id = ?1",
                        params![
                            key,
                            edit.stance.as_ref().map(text_of),
                            edit.ask.as_ref().map(text_of),
                            edit.blocking.map(i64::from),
                            edit.body.as_deref(),
                        ],
                    )?;
                }
                Act::Send | Act::Withdraw => {}
                Act::Reply { body, stance } => {
                    transaction.execute(
                        "INSERT INTO replies (note, role, by, at, stance, body) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        params![
                            key,
                            text_of(&who),
                            by,
                            now,
                            stance.as_ref().map(text_of),
                            body
                        ],
                    )?;
                }
                Act::Propose { outcome, summary } | Act::Decide { outcome, summary } => {
                    let decision = Decision {
                        outcome: *outcome,
                        summary: summary.clone(),
                        by: by.to_owned(),
                        at: now,
                    };
                    let column = if matches!(act, Act::Propose { .. }) {
                        "proposal"
                    } else {
                        "decision"
                    };
                    transaction.execute(
                        &format!("UPDATE notes SET {column} = ?2 WHERE id = ?1"),
                        params![key, json_of(&decision)],
                    )?;
                }
                Act::Applied { commits } => {
                    for commit in commits {
                        transaction.execute(
                            "INSERT INTO applied (note, commit_id, by, at) VALUES (?1, ?2, ?3, ?4) ON CONFLICT (note, commit_id) DO NOTHING",
                            params![key, commit, by, now],
                        )?;
                    }
                }
            }
            transaction.execute(
                "UPDATE notes SET status = ?2, updated_at = ?3 WHERE id = ?1",
                params![key, status.name(), now],
            )?;
            record(
                &transaction,
                key,
                EventAction::of(act.action()),
                who,
                by,
                now,
                via,
            )?;
            transaction.commit()?;
            Ok(())
        })?;
        self.note(id)
    }

    /// Keeps the bytes of a version that git does not hold, such as a
    /// working-tree file a note was made on, so the note can be followed
    /// from it later. A version kept before is kept once.
    ///
    /// # Errors
    ///
    /// When the file cannot be written.
    pub fn keep_version(&self, blob: &str, bytes: &[u8]) -> Result<(), StoreError> {
        self.with(|connection| {
            connection.execute(
                "INSERT INTO versions (blob, bytes) VALUES (?1, ?2) ON CONFLICT (blob) DO NOTHING",
                params![blob, bytes],
            )?;
            Ok(())
        })
    }

    /// The bytes of a kept version.
    ///
    /// # Errors
    ///
    /// When the file cannot be read.
    pub fn version(&self, blob: &str) -> Result<Option<Vec<u8>>, StoreError> {
        self.with(|connection| {
            Ok(connection
                .query_row(
                    "SELECT bytes FROM versions WHERE blob = ?1",
                    params![blob],
                    |row| row.get::<_, Vec<u8>>(0),
                )
                .optional()?)
        })
    }

    /// Marks a change seen or not seen by a reader. A mark keeps where its
    /// request came from; taking it back removes it.
    ///
    /// # Errors
    ///
    /// When the file cannot be written.
    pub fn set_seen(
        &self,
        reader: &str,
        key: &str,
        seen: bool,
        now: i64,
        via: &Via,
    ) -> Result<(), StoreError> {
        self.with(|connection| {
            if seen {
                connection.execute(
                    "INSERT INTO seen (reader, change, at, source, user_agent) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT (reader, change) DO UPDATE SET at = ?3, source = ?4, user_agent = ?5",
                    params![reader, key, now, via.source.name(), via.user_agent],
                )?;
            } else {
                connection.execute(
                    "DELETE FROM seen WHERE reader = ?1 AND change = ?2",
                    params![reader, key],
                )?;
            }
            Ok(())
        })
    }

    /// Which of these changes the reader marked seen.
    ///
    /// # Errors
    ///
    /// When the file cannot be read.
    pub fn seen(&self, reader: &str, keys: &[String]) -> Result<HashSet<String>, StoreError> {
        if keys.is_empty() {
            return Ok(HashSet::new());
        }
        self.with(|connection| {
            let mut statement =
                connection.prepare("SELECT 1 FROM seen WHERE reader = ?1 AND change = ?2")?;
            let mut found = HashSet::new();
            for key in keys {
                if statement.exists(params![reader, key])? {
                    found.insert(key.clone());
                }
            }
            Ok(found)
        })
    }

    /// A reader's seen mark on one change, with where it came from; `None`
    /// when the change is not marked seen.
    ///
    /// # Errors
    ///
    /// When the file cannot be read.
    pub fn seen_mark(&self, reader: &str, key: &str) -> Result<Option<SeenMark>, StoreError> {
        self.with(|connection| {
            let found = connection
                .query_row(
                    "SELECT at, source, user_agent FROM seen WHERE reader = ?1 AND change = ?2",
                    params![reader, key],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, Option<String>>(2)?,
                        ))
                    },
                )
                .optional()?;
            found
                .map(|(at, source, user_agent)| {
                    Ok(SeenMark {
                        at,
                        via: Via {
                            source: source_of(&source)?,
                            user_agent,
                        },
                    })
                })
                .transpose()
        })
    }
}

/// A source as the file names it.
fn source_of(name: &str) -> Result<Source, StoreError> {
    Source::from_name(name).ok_or_else(|| StoreError::File(format!("unknown source `{name}`")))
}

/// Adds one entry to note `note`'s history, inside the transaction that
/// does the action.
fn record(
    transaction: &rusqlite::Transaction<'_>,
    note: i64,
    action: EventAction,
    who: Role,
    by: &str,
    now: i64,
    via: &Via,
) -> Result<(), StoreError> {
    transaction.execute(
        "INSERT INTO actions (note, action, role, by, at, source, user_agent) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            note,
            action.name(),
            text_of(&who),
            by,
            now,
            via.source.name(),
            via.user_agent
        ],
    )?;
    Ok(())
}

/// A note or review number as SQLite stores it.
fn number(id: u64) -> i64 {
    i64::try_from(id).unwrap_or(i64::MAX)
}

/// Brings the schema up to [`SCHEMA_VERSION`], one migration at a time.
fn migrate(connection: &mut Connection) -> Result<(), StoreError> {
    let found: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if found > SCHEMA_VERSION {
        return Err(StoreError::Newer { found });
    }
    for (version, sql) in MIGRATIONS {
        if *version <= found {
            continue;
        }
        let transaction = connection.transaction()?;
        transaction.execute_batch(sql)?;
        transaction.pragma_update(None, "user_version", version)?;
        transaction.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
