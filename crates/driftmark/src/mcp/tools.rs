//! The tool list, how a call's arguments are read, and how an answer is
//! checked against its size limit.
//!
//! Each tool is one entry of [`TOOLS`]: its name, its description, the JSON
//! Schema of its arguments, whether it only reads, and the function that
//! answers. The note tools (`notes_list`, `note_get`, `review_get`,
//! `note_reply`, `note_propose_decision`, `note_mark_applied`) read and
//! write the notes file in [`Context`].

use std::path::PathBuf;
use std::sync::Arc;

use driftmark_core::git::RepoPath;
use serde_json::{Map, Value, json};

use super::fit::{self, DEFAULT_CHARS, MAX_CHARS, MIN_CHARS};
use super::protocol::{Connector, Reply, ToolInfo, UNNAMED_CLIENT};
use super::search::WorktreeSearch;
use crate::serve::address;
use crate::serve::placing::NotesFile;
use crate::serve::repo::{Problem, Repo};

/// What every tool reads from.
pub(crate) struct Context {
    /// The repository, with the server's cache of parsed documents and
    /// counted ranges.
    pub repo: Arc<Repo>,
    /// The working tree's root folder; `None` in a bare repository.
    pub workdir: Option<PathBuf>,
    /// The working tree's search index, built again when its files change.
    pub worktree_search: WorktreeSearch,
    /// The notes file `driftmark serve` writes, opened on first use. A tool
    /// that only reads never makes it.
    pub notes: NotesFile,
}

/// One tool.
pub(crate) struct Tool {
    /// The name a client calls it by.
    pub name: &'static str,
    /// A short name for people.
    pub title: &'static str,
    /// What it does and when to use it, for the agent.
    pub description: &'static str,
    /// Its arguments, each with its JSON Schema, in the order a client
    /// shows them. `max_chars` is added to every tool.
    pub arguments: fn() -> Value,
    /// The arguments a call must give.
    pub required: &'static [&'static str],
    /// Whether it only reads. Only the note tools that reply, propose or
    /// record commits write, and only to the notes file.
    pub read_only: bool,
    /// The answer: a JSON object, or why there is none.
    pub run: fn(&Context, &Args) -> Result<Value, String>,
}

/// Every tool, in the order a client lists them.
pub(crate) const TOOLS: &[Tool] = &[
    super::changes::SUMMARY,
    super::changes::DOCUMENT,
    super::document::READ,
    super::search::SEARCH,
    super::lint::LINT,
    super::notes::LIST,
    super::notes::GET,
    super::notes::REVIEW,
    super::notes::REPLY,
    super::notes::PROPOSE,
    super::notes::MARK_APPLIED,
];

/// The text a client shows the agent about the whole server.
const INSTRUCTIONS: &str = "\
driftmark reads this git repository's Markdown documents. Use it to see what changed in the docs between two git points, ranked by meaning (decision, rule, fact, wording, formatting), to read a document or one section of it at any revision, to search every document, to check documents against the writing rules, and to answer the reader's notes on the documents.

Start with changes_summary (range \"auto\" is this branch's work, committed or not), then changes_document for one document, document_read for the text, search to find a topic, and lint after editing. Paths run from the repository root, such as docs/guide.md. A revision is a branch, a tag, a commit or \"worktree\" for the files on disk. Every answer is JSON. When an answer is cut to fit, its \"cut\" object says what was left out, and \"cut.next\" holds the arguments of the call that continues.

The reader writes notes on sentences of these documents, on driftmark's pages. notes_list shows the notes still open; note_get reads one with where its words are now and the section around them; review_get gives a review's notes as one prompt. Discuss first: answer each note with note_reply, and suggest what to do with note_propose_decision. Edit a document only for a note the reader decided (status \"decided\", review_get round 2); commit the edit, then record each commit with note_mark_applied. Only the reader decides, edits or withdraws a note.";

/// The schema of `max_chars`, which every tool takes.
fn max_chars_schema() -> Value {
    json!({
        "type": "integer",
        "minimum": MIN_CHARS,
        "maximum": MAX_CHARS,
        "description": format!("The most characters the answer may hold, from {MIN_CHARS} to {MAX_CHARS}. Default {DEFAULT_CHARS}. A longer answer is cut, and its `cut` object says how to ask for the rest."),
    })
}

impl Tool {
    /// The tool as a client lists it.
    fn info(&self) -> ToolInfo {
        let mut properties = match (self.arguments)() {
            Value::Object(properties) => properties,
            _ => Map::new(),
        };
        properties.insert("max_chars".to_owned(), max_chars_schema());
        let mut schema = Map::new();
        schema.insert("type".to_owned(), json!("object"));
        schema.insert("properties".to_owned(), Value::Object(properties));
        if !self.required.is_empty() {
            schema.insert("required".to_owned(), json!(self.required));
        }
        schema.insert("additionalProperties".to_owned(), json!(false));
        ToolInfo {
            name: self.name,
            title: self.title,
            description: self.description,
            schema,
            read_only: self.read_only,
        }
    }

    /// The names of the arguments it takes.
    fn takes(&self) -> Vec<String> {
        let mut names: Vec<String> = match (self.arguments)() {
            Value::Object(properties) => properties.keys().cloned().collect(),
            _ => Vec::new(),
        };
        names.push("max_chars".to_owned());
        names
    }
}

/// The connector the protocol layer serves.
pub(crate) struct Driftmark {
    context: Context,
}

impl Driftmark {
    /// The tools over `context`.
    pub(crate) fn new(context: Context) -> Driftmark {
        Driftmark { context }
    }

    /// Runs one tool for the client named `client`: reads its arguments,
    /// answers, and checks the answer's size. `None` when no tool has this
    /// name.
    pub(crate) fn answer(
        &self,
        name: &str,
        arguments: Map<String, Value>,
        client: &str,
    ) -> Option<Reply> {
        let tool = TOOLS.iter().find(|tool| tool.name == name)?;
        let reply = Args::read(tool, arguments)
            .map(|args| args.called_by(client))
            .and_then(|args| {
                let limit = args.max_chars()?;
                let answer = (tool.run)(&self.context, &args)?;
                let text = answer.to_string();
                let length = fit::chars(&text);
                if length > limit {
                    return Err(format!(
                        "the answer needs {length} characters even after cutting, more than the limit of {limit}; ask again with a larger `max_chars`, up to {MAX_CHARS}"
                    ));
                }
                Ok(text)
            })
            .map_or_else(Reply::Refused, Reply::Answer);
        Some(reply)
    }
}

impl Connector for Driftmark {
    fn instructions(&self) -> String {
        INSTRUCTIONS.to_owned()
    }

    fn tools(&self) -> Vec<ToolInfo> {
        TOOLS.iter().map(Tool::info).collect()
    }

    fn call(&self, name: &str, arguments: Map<String, Value>, client: &str) -> Option<Reply> {
        self.answer(name, arguments, client)
    }

    fn missing(&self, name: &str) -> String {
        missing(name)
    }
}

/// Why no tool has this name. A name about notes gets the reason an agent
/// most likely needs: the note tools there are, and what only the reader
/// does.
pub(crate) fn missing(name: &str) -> String {
    if name.starts_with("note") || name.starts_with("review") {
        format!(
            "driftmark has no tool `{name}`. An agent lists and reads notes (notes_list, note_get, review_get), answers them (note_reply), suggests a decision (note_propose_decision) and records the commits that applied one (note_mark_applied). Deciding, editing and withdrawing a note are the reader's, on the note's page, because the notes are the reader's own words and judgement"
        )
    } else {
        format!("driftmark has no tool `{name}`")
    }
}

/// A call's arguments, read with the checks every tool shares.
///
/// Agents sometimes send a number or a true-or-false value as text, such as
/// `"20"`, so those are accepted as text too.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Args {
    map: Map<String, Value>,
    /// The client that called, as its handshake names itself.
    client: String,
}

impl Args {
    /// The arguments of a call to `tool`, refused when one is unknown or a
    /// required one is missing.
    pub(crate) fn read(tool: &Tool, map: Map<String, Value>) -> Result<Args, String> {
        let takes = tool.takes();
        if let Some(unknown) = map.keys().find(|key| !takes.contains(key)) {
            return Err(format!(
                "`{}` takes no argument `{unknown}`; its arguments are {}",
                tool.name,
                takes
                    .iter()
                    .map(|name| format!("`{name}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if let Some(missing) = tool
            .required
            .iter()
            .find(|name| map.get(**name).is_none_or(Value::is_null))
        {
            return Err(format!("`{}` needs the argument `{missing}`", tool.name));
        }
        Ok(Args {
            map,
            client: UNNAMED_CLIENT.to_owned(),
        })
    }

    /// These arguments, called by the client named `client`.
    #[must_use]
    pub(crate) fn called_by(mut self, client: &str) -> Args {
        client.clone_into(&mut self.client);
        self
    }

    /// The client that called, such as `claude-code`: the name an agent's
    /// replies and records carry.
    pub(crate) fn client(&self) -> &str {
        &self.client
    }

    /// Arguments read as they were given, for tests.
    #[cfg(test)]
    pub(crate) fn of(value: Value) -> Args {
        let map = match value {
            Value::Object(map) => map,
            _ => Map::new(),
        };
        Args {
            map,
            client: UNNAMED_CLIENT.to_owned(),
        }
    }

    fn given(&self, name: &str) -> Option<&Value> {
        self.map.get(name).filter(|value| !value.is_null())
    }

    /// A text argument, or `None` when it is not given.
    pub(crate) fn text(&self, name: &str) -> Result<Option<String>, String> {
        match self.given(name) {
            None => Ok(None),
            Some(Value::String(text)) => Ok(Some(text.clone())),
            Some(_) => Err(format!("`{name}` must be text")),
        }
    }

    /// A list of texts; one text alone counts as a list of one.
    pub(crate) fn texts(&self, name: &str) -> Result<Vec<String>, String> {
        let wrong = || format!("`{name}` must be a list of texts");
        match self.given(name) {
            None => Ok(Vec::new()),
            Some(Value::String(text)) => Ok(vec![text.clone()]),
            Some(Value::Array(items)) => items
                .iter()
                .map(|item| item.as_str().map(str::to_owned).ok_or_else(wrong))
                .collect(),
            Some(_) => Err(wrong()),
        }
    }

    /// A true-or-false argument, or `default` when it is not given.
    pub(crate) fn flag(&self, name: &str, default: bool) -> Result<bool, String> {
        match self.given(name) {
            None => Ok(default),
            Some(Value::Bool(flag)) => Ok(*flag),
            Some(Value::String(text)) if text == "true" => Ok(true),
            Some(Value::String(text)) if text == "false" => Ok(false),
            Some(_) => Err(format!("`{name}` must be true or false")),
        }
    }

    /// A true-or-false argument, or `None` when it is not given.
    pub(crate) fn given_flag(&self, name: &str) -> Result<Option<bool>, String> {
        self.given(name).map(|_| self.flag(name, false)).transpose()
    }

    /// A note or review number, written `12`, `"12"` or with its letter, as
    /// `"n12"` for a note; `None` when it is not given.
    pub(crate) fn number_of(&self, name: &str, letter: char) -> Result<Option<u64>, String> {
        let wrong = || format!("`{name}` must be a number such as {letter}12 or 12");
        let number = match self.given(name) {
            None => return Ok(None),
            Some(Value::Number(number)) => number.as_u64(),
            Some(Value::String(text)) => {
                let text = text.trim();
                text.strip_prefix(letter)
                    .unwrap_or(text)
                    .parse::<u64>()
                    .ok()
            }
            Some(_) => None,
        };
        number
            .filter(|&number| number > 0)
            .map(Some)
            .ok_or_else(wrong)
    }

    /// A whole number from `min` to `max`, or `default` when it is not
    /// given.
    pub(crate) fn number(
        &self,
        name: &str,
        min: usize,
        max: usize,
        default: usize,
    ) -> Result<usize, String> {
        let value = match self.given(name) {
            None => return Ok(default),
            Some(Value::Number(number)) => number.as_u64(),
            Some(Value::String(text)) => text.trim().parse::<u64>().ok(),
            Some(_) => None,
        };
        value
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| (min..=max).contains(value))
            .ok_or_else(|| format!("`{name}` must be a whole number from {min} to {max}"))
    }

    /// A whole number from 0 to `max`, or `None` when it is not given.
    pub(crate) fn given_number(&self, name: &str, max: usize) -> Result<Option<usize>, String> {
        self.given(name)
            .map(|_| self.number(name, 0, max, 0))
            .transpose()
    }

    /// The answer's size limit, in characters.
    pub(crate) fn max_chars(&self) -> Result<usize, String> {
        self.number("max_chars", MIN_CHARS, MAX_CHARS, DEFAULT_CHARS)
    }

    /// These arguments with `changes` made, for a `cut`'s next call.
    pub(crate) fn next(&self, changes: &[(&str, Value)]) -> Map<String, Value> {
        let mut map = self.map.clone();
        for (name, value) in changes {
            map.insert((*name).to_owned(), value.clone());
        }
        map
    }
}

/// The most items a list answer may ask for at once.
pub(crate) const MAX_LIMIT: usize = 500;

/// The largest `offset` a call may give, far past any list or document.
pub(crate) const MAX_OFFSET: usize = 1_000_000_000;

/// The schema of `offset`, for a tool whose answer is a list.
pub(crate) fn offset_schema(what: &str) -> Value {
    json!({
        "type": "integer",
        "minimum": 0,
        "description": format!("How many {what} to skip, to read on after a cut answer. Default 0."),
    })
}

/// The schema of `limit`, for a tool whose answer is a list.
pub(crate) fn limit_schema(what: &str, default: usize, max: usize) -> Value {
    json!({
        "type": "integer",
        "minimum": 1,
        "maximum": max,
        "description": format!("The most {what} to list. Default {default}; fewer when they do not fit in `max_chars`."),
    })
}

/// The `page` object of a list answer: where it starts, how many it shows,
/// and how many there are in all.
pub(crate) fn page(offset: usize, shown: usize, total: usize) -> Value {
    json!({"offset": offset, "shown": shown, "total": total})
}

/// The `cut` of a list answer that shows `shown` of `total` items from
/// `offset`, or `None` when it shows the rest of the list.
pub(crate) fn list_cut(
    args: &Args,
    what: &str,
    offset: usize,
    shown: usize,
    total: usize,
) -> Option<Value> {
    let next = offset + shown;
    (next < total).then(|| {
        fit::cut(
            &format!(
                "{shown} of {total} {what} are listed, from number {offset}; ask with `offset` {next} for the rest"
            ),
            Some(args.next(&[("offset", json!(next))])),
        )
    })
}

/// A document's path as a tool takes it: from the repository root, such as
/// `docs/guide.md`. A path written from the root of the file system is
/// accepted when it lies inside the working tree, because agents often
/// hold such paths; any other path is refused, so no argument can make the
/// connector read outside the repository.
///
/// # Errors
///
/// A sentence when the path is outside the repository, holds `.` or `..`
/// parts, or does not name a Markdown file.
pub(crate) fn document_path(context: &Context, text: &str) -> Result<RepoPath, String> {
    let inside = repo_relative(context, text)?;
    address::markdown_path(inside.into_bytes()).map_err(|bad| bad.0)
}

/// `text` as a path from the repository root: a path from the root of the
/// file system loses the working tree's folder, and a leading `./` goes.
///
/// # Errors
///
/// A sentence when the path starts at the root of the file system and is
/// not inside the working tree.
pub(crate) fn repo_relative(context: &Context, text: &str) -> Result<String, String> {
    let path = std::path::Path::new(text);
    // `has_root`, not `is_absolute`: on Windows `/etc/x.md` has a root but no
    // drive, and it must not be read as a path from the repository root.
    if !path.has_root() {
        return Ok(text.strip_prefix("./").unwrap_or(text).to_owned());
    }
    // Both sides are compared as written and with links resolved: on macOS
    // `/var` is a link to `/private/var`, so a path through the link and git's
    // working tree can name one folder two ways.
    let given = [Some(path.to_path_buf()), resolved(path)];
    let inside = context.workdir.as_ref().and_then(|root| {
        let roots = [Some(root.clone()), root.canonicalize().ok()];
        roots.iter().flatten().find_map(|root| {
            given.iter().flatten().find_map(|path| {
                path.strip_prefix(root)
                    .ok()
                    .map(std::path::Path::to_path_buf)
            })
        })
    });
    let relative = inside.ok_or_else(|| {
        format!("`{text}` is outside the repository; give a path from the repository root, such as docs/guide.md")
    })?;
    let parts: Option<Vec<&str>> = relative
        .components()
        .map(|part| part.as_os_str().to_str())
        .collect();
    parts
        .map(|parts| parts.join("/"))
        .ok_or_else(|| format!("`{text}` is not valid text"))
}

/// `path` with links resolved. A document deleted in a range no longer
/// exists, so the deepest folder that exists is resolved and the rest is
/// added back as written.
fn resolved(path: &std::path::Path) -> Option<std::path::PathBuf> {
    path.ancestors().find_map(|existing| {
        let rest = path.strip_prefix(existing).ok()?;
        existing.canonicalize().ok().map(|real| real.join(rest))
    })
}

/// A problem reading git, as the sentence a tool gives back.
pub(crate) fn said(problem: Problem) -> String {
    problem.message
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str) -> &'static Tool {
        TOOLS.iter().find(|tool| tool.name == name).unwrap()
    }

    #[test]
    fn every_tool_lists_an_object_schema_that_refuses_unknown_arguments() {
        let names: Vec<&str> = TOOLS.iter().map(|tool| tool.name).collect();
        assert_eq!(
            names,
            [
                "changes_summary",
                "changes_document",
                "document_read",
                "search",
                "lint",
                "notes_list",
                "note_get",
                "review_get",
                "note_reply",
                "note_propose_decision",
                "note_mark_applied"
            ]
        );
        for tool in TOOLS {
            let info = tool.info();
            assert_eq!(info.schema["type"], "object");
            assert_eq!(info.schema["additionalProperties"], false);
            assert!(info.schema["properties"]["max_chars"].is_object());
            for required in tool.required {
                assert!(
                    info.schema["properties"][*required].is_object(),
                    "{required}"
                );
            }
            assert!(info.description.len() < 4_096, "{}", tool.name);
            for (name, property) in info.schema["properties"].as_object().unwrap() {
                assert!(
                    property["description"].is_string(),
                    "{}: {name} has no description",
                    tool.name
                );
            }
        }
        assert!(INSTRUCTIONS.len() < 4_096);
    }

    #[test]
    fn arguments_are_checked_by_name_and_kind() {
        let search = tool("search");
        let unknown = Args::read(
            search,
            json!({"query": "a", "q": "b"}).as_object().unwrap().clone(),
        );
        assert_eq!(
            unknown.unwrap_err(),
            "`search` takes no argument `q`; its arguments are `query`, `revision`, `path`, `types`, `offset`, `limit`, `max_chars`"
        );
        let missing = Args::read(search, Map::new());
        assert_eq!(missing.unwrap_err(), "`search` needs the argument `query`");
        let null = Args::read(search, json!({"query": null}).as_object().unwrap().clone());
        assert_eq!(null.unwrap_err(), "`search` needs the argument `query`");

        let args = Args::of(json!({
            "t": "x", "n": 5, "s": " 7 ", "b": true, "bs": "false", "list": ["a", "b"], "one": "c", "null": null, "bad": [1]
        }));
        assert_eq!(args.text("t"), Ok(Some("x".into())));
        assert_eq!(args.text("null"), Ok(None));
        assert_eq!(args.text("n").unwrap_err(), "`n` must be text");
        assert_eq!(args.number("n", 1, 10, 3), Ok(5));
        assert_eq!(args.number("s", 1, 10, 3), Ok(7));
        assert_eq!(args.number("missing", 1, 10, 3), Ok(3));
        assert_eq!(
            args.number("n", 6, 10, 6).unwrap_err(),
            "`n` must be a whole number from 6 to 10"
        );
        assert!(args.number("t", 0, 10, 0).is_err());
        assert!(args.number("b", 0, 10, 0).is_err());
        assert_eq!(args.flag("b", false), Ok(true));
        assert_eq!(args.flag("bs", true), Ok(false));
        assert_eq!(args.flag("missing", true), Ok(true));
        assert_eq!(
            args.flag("t", true).unwrap_err(),
            "`t` must be true or false"
        );
        assert_eq!(args.texts("list"), Ok(vec!["a".into(), "b".into()]));
        assert_eq!(args.texts("one"), Ok(vec!["c".into()]));
        assert_eq!(args.texts("missing"), Ok(vec![]));
        assert_eq!(
            args.texts("bad").unwrap_err(),
            "`bad` must be a list of texts"
        );
        assert_eq!(args.texts("n").unwrap_err(), "`n` must be a list of texts");
        assert_eq!(args.given_number("n", 9), Ok(Some(5)));
        assert_eq!(args.given_number("missing", 9), Ok(None));
        assert!(args.given_number("t", 9).is_err());
        assert_eq!(args.max_chars(), Ok(DEFAULT_CHARS));
        assert_eq!(
            Args::of(json!({"max_chars": 100})).max_chars().unwrap_err(),
            "`max_chars` must be a whole number from 2000 to 40000"
        );
        let next = Args::of(json!({"query": "a", "offset": 0})).next(&[("offset", json!(20))]);
        assert_eq!(Value::Object(next), json!({"query": "a", "offset": 20}));
    }

    #[test]
    fn a_list_cut_names_the_next_offset() {
        let args = Args::of(json!({"range": "auto"}));
        assert_eq!(list_cut(&args, "documents", 0, 5, 5), None);
        let cut = list_cut(&args, "documents", 10, 5, 30).unwrap();
        assert_eq!(
            cut["reason"],
            "5 of 30 documents are listed, from number 10; ask with `offset` 15 for the rest"
        );
        assert_eq!(cut["next"], json!({"range": "auto", "offset": 15}));
        assert_eq!(page(1, 2, 3), json!({"offset": 1, "shown": 2, "total": 3}));
    }
}
