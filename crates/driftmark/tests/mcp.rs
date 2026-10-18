//! `driftmark mcp`, run as the real program and driven by a small MCP
//! client over standard input and output, on small repos built for each
//! test. The client sends exactly the JSON-RPC lines a client such as Claude
//! Code sends, so these tests also pin the wire format, whatever library the
//! program uses for it.

mod support;

use std::fmt::Write as _;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use support::{DAY, Expected, T0, TestRepo, documented_repo, examples};

const PROGRAM: &str = env!("CARGO_BIN_EXE_driftmark");

/// The size limit of an answer when the call names none.
const DEFAULT_CHARS: usize = 16_000;

/// Rules for the test repos: files under `docs/decisions/` are decision
/// records with a status in front matter.
const RULES: &str = "\
[[type]]
name = \"decision\"
paths = [\"docs/decisions/*.md\"]
status = { front-matter = \"status\" }
current = [\"accepted\"]
other-statuses = [\"proposed\"]
decision-headings = [\"Decision\"]

[lint]
exclude = [\"docs/broken/**\"]
";

const DECISION_BEFORE: &str = "---\nstatus: proposed\n---\n\n# Cache pages\n\n> Short version: the server keeps pages.\n\n## Decision\n\nKeep each page for 60 minutes.\n\n## Notes\n\nThe cache lives in memory.\n";

const DECISION_AFTER: &str = "---\nstatus: accepted\n---\n\n# Cache pages\n\n> Short version: the server keeps pages.\n\n## Decision\n\nKeep each page for 90 minutes.\n\nNever cache a page with open notes.\n\n## Notes\n\nThe cache lives in memory.\n";

const GUIDE: &str = "# Guide\n\n> Short version: how to use the cache.\n\n## Install\n\nRun the installer.\n\n## Configure\n\nSet the lantern size.\n\n> ### Quoted part\n>\n> A quoted heading.\n";

const NEW_PAGE: &str = "# New page\n\n> Short version: a new page.\n\nIt holds 500 pages.\n";

/// A repo on `main` with a `feature` branch that edits a decision record
/// and adds a page. `origin/main` points at `main`, so it is the default
/// branch. `HEAD` stays on `main`.
fn branch_repo() -> TestRepo {
    let repo = TestRepo::new("mcp");
    repo.write("driftmark.toml", RULES);
    repo.write("docs/decisions/0001-cache.md", DECISION_BEFORE);
    repo.write("docs/guide.md", GUIDE);
    repo.write("README.md", "# Readme\n\n> Short version: the readme.\n");
    let base = repo.commit("first", T0);
    repo.git(&["update-ref", "refs/remotes/origin/main", &base]);
    repo.git(&["checkout", "-q", "-b", "feature"]);
    repo.write("docs/decisions/0001-cache.md", DECISION_AFTER);
    repo.write("docs/new.md", NEW_PAGE);
    repo.commit("feature", T0 + DAY);
    repo.git(&["checkout", "-q", "main"]);
    repo
}

/// An answer to a tool call.
#[derive(Debug)]
struct Answer {
    is_error: bool,
    text: String,
}

/// A running `driftmark mcp` with a client attached.
struct Client {
    child: support::Running,
    stdin: Option<ChildStdin>,
    lines: Receiver<String>,
    /// Lines on standard output that are not JSON: there must be none.
    stray: Arc<Mutex<Vec<String>>>,
    next: u64,
}

impl Client {
    /// Starts `driftmark mcp` in `dir` with `args`, and opens the session.
    fn start(dir: &Path, args: &[&str]) -> Client {
        let mut child = support::Running(
            Command::new(PROGRAM)
                .arg("mcp")
                .args(args)
                .current_dir(dir)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .expect("start driftmark mcp"),
        );
        let stdout = child.stdout.take().expect("standard output");
        let (send, lines) = channel();
        let stray = Arc::new(Mutex::new(Vec::new()));
        let strays = Arc::clone(&stray);
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { return };
                if serde_json::from_str::<Value>(&line).is_err() {
                    strays.lock().expect("lock").push(line.clone());
                }
                if send.send(line).is_err() {
                    return;
                }
            }
        });
        let stdin = child.stdin.take();
        let mut client = Client {
            child,
            stdin,
            lines,
            stray,
            next: 0,
        };
        let opened = client.request(
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "driftmark-test", "version": "0"}
            }),
        );
        assert_eq!(opened["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(opened["result"]["serverInfo"]["name"], "driftmark");
        assert!(opened["result"]["capabilities"]["tools"].is_object());
        client.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        client
    }

    fn send(&mut self, message: &Value) {
        let stdin = self.stdin.as_mut().expect("the session is open");
        writeln!(stdin, "{message}").expect("write a message");
        stdin.flush().expect("flush");
    }

    /// Sends one request and waits for its answer.
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let id = self.next;
        let mut message = json!({"jsonrpc": "2.0", "id": id, "method": method});
        message["params"] = params;
        self.send(&message);
        loop {
            let line = self
                .lines
                .recv_timeout(Duration::from_secs(120))
                .expect("an answer in time");
            let message: Value = serde_json::from_str(&line).expect("a JSON answer");
            if message["id"] == id {
                return message;
            }
        }
    }

    /// Calls a tool.
    fn call(&mut self, name: &str, arguments: Value) -> Answer {
        let limit = arguments["max_chars"]
            .as_u64()
            .map_or(DEFAULT_CHARS, |limit| {
                usize::try_from(limit).expect("a size")
            });
        let mut params = json!({"name": name});
        params["arguments"] = arguments;
        let message = self.request("tools/call", params);
        let result = &message["result"];
        assert!(result.is_object(), "{name}: {message}");
        let content = result["content"].as_array().expect("content");
        assert_eq!(content.len(), 1, "one text item");
        assert_eq!(content[0]["type"], "text");
        let text = content[0]["text"].as_str().expect("text").to_owned();
        let size = text.encode_utf16().count();
        assert!(size <= limit, "{name}: {size} characters, limit {limit}");
        Answer {
            is_error: result["isError"] == true,
            text,
        }
    }

    /// Calls a tool that must answer, and reads its JSON.
    fn json(&mut self, name: &str, arguments: Value) -> Value {
        let answer = self.call(name, arguments);
        assert!(!answer.is_error, "{name}: {}", answer.text);
        serde_json::from_str(&answer.text).expect("the answer is JSON")
    }

    /// Calls a tool that must refuse, and returns why.
    fn refused(&mut self, name: &str, arguments: Value) -> String {
        let answer = self.call(name, arguments);
        assert!(answer.is_error, "{name} answered: {}", answer.text);
        answer.text
    }

    /// Closes the session and checks the program ended cleanly, having
    /// written nothing but protocol messages.
    fn finish(mut self) {
        drop(self.stdin.take());
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        let status = loop {
            if let Some(status) = self.child.try_wait().expect("wait") {
                break status;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the program did not stop when standard input closed"
            );
            std::thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(status.code(), Some(0));
        assert!(
            self.stray.lock().expect("lock").is_empty(),
            "standard output held more than protocol messages"
        );
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The texts `line(0)` to `line(count - 1)`, joined.
fn numbered(count: usize, line: impl Fn(usize) -> String) -> String {
    (0..count).fold(String::new(), |mut text, n| {
        text.push_str(&line(n));
        text
    })
}

/// Runs the program in `dir`; returns its exit code, output and errors.
fn driftmark(dir: &Path, args: &[&str]) -> (i32, String, String) {
    let output = Command::new(PROGRAM)
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .expect("the program runs");
    (
        output.status.code().expect("an exit code"),
        String::from_utf8(output.stdout).expect("UTF-8 output"),
        String::from_utf8(output.stderr).expect("UTF-8 errors"),
    )
}

// ---- the session ---------------------------------------------------------

#[test]
fn the_tools_are_listed_with_schemas_and_marked_read_only_or_not() {
    let repo = branch_repo();
    let mut client = Client::start(&repo.dir, &[]);
    let listed = client.request("tools/list", json!({}));
    let tools = listed["result"]["tools"].as_array().expect("tools");
    let names: Vec<&str> = tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
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
    let writers = ["note_reply", "note_propose_decision", "note_mark_applied"];
    for tool in tools {
        let name = tool["name"].as_str().unwrap();
        assert!(tool["description"].as_str().unwrap().len() > 100, "{name}");
        let schema = &tool["inputSchema"];
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["properties"]["max_chars"]["maximum"], 40_000);
        let reads = !writers.contains(&name);
        assert_eq!(tool["annotations"]["readOnlyHint"], reads, "{name}");
        assert_eq!(tool["annotations"]["idempotentHint"], reads, "{name}");
        assert_eq!(tool["annotations"]["destructiveHint"], false);
        assert_eq!(tool["annotations"]["openWorldHint"], false);
    }
    assert_eq!(tools[1]["inputSchema"]["required"], json!(["path"]));
    assert_eq!(tools[3]["inputSchema"]["required"], json!(["query"]));
    assert_eq!(
        tools[8]["inputSchema"]["required"],
        json!(["note", "body", "stance"])
    );
    let unknown = client.request("tools/call", json!({"name": "nope", "arguments": {}}));
    assert_eq!(unknown["error"]["code"], -32602);
    assert_eq!(unknown["error"]["message"], "driftmark has no tool `nope`");
    client.finish();
}

#[test]
fn wrong_arguments_are_refused_with_a_reason_the_agent_can_act_on() {
    let repo = branch_repo();
    let mut client = Client::start(&repo.dir, &[]);
    assert_eq!(
        client.refused("search", json!({"query": "x", "q": "y"})),
        "`search` takes no argument `q`; its arguments are `query`, `revision`, `path`, `types`, `offset`, `limit`, `max_chars`"
    );
    assert_eq!(
        client.refused("changes_document", json!({})),
        "`changes_document` needs the argument `path`"
    );
    assert_eq!(
        client.refused("search", json!({"query": 5})),
        "`query` must be text"
    );
    assert_eq!(
        client.refused("lint", json!({"max_chars": 100})),
        "`max_chars` must be a whole number from 2000 to 40000"
    );
    assert_eq!(
        client.refused("changes_summary", json!({"limit": 0})),
        "`limit` must be a whole number from 1 to 500"
    );
    // Numbers given as text are read as numbers.
    let summary = client.json(
        "changes_summary",
        json!({"range": "main...feature", "limit": "1"}),
    );
    assert_eq!(summary["page"]["shown"], 1);
    client.finish();
}

// ---- changes_summary -----------------------------------------------------

#[test]
fn the_summary_is_the_diff_commands_summary_json() {
    let repo = branch_repo();
    let (code, diff, _) = driftmark(&repo.dir, &["diff", "main...feature", "--json=summary"]);
    assert_eq!(code, 0);
    let diff: Value = serde_json::from_str(&diff).expect("JSON");
    let mut client = Client::start(&repo.dir, &[]);
    let summary = client.json("changes_summary", json!({"range": "main...feature"}));
    for field in ["schema_version", "range", "totals", "documents"] {
        assert_eq!(summary[field], diff[field], "{field}");
    }
    assert_eq!(summary["schema_version"], 2);
    assert_eq!(
        summary["documents"][0]["path"],
        "docs/decisions/0001-cache.md"
    );
    assert_eq!(summary["documents"][0]["summary"]["highest"], "decision");
    assert_eq!(summary["skipped"], json!([]));
    assert_eq!(
        summary["page"],
        json!({"offset": 0, "shown": 2, "total": 2})
    );
    assert!(summary.get("cut").is_none());
    client.finish();
}

#[test]
fn auto_is_the_default_range_and_sees_work_not_committed() {
    let repo = branch_repo();
    repo.git(&["checkout", "-q", "feature"]);
    repo.write("docs/guide.md", GUIDE.replace("installer", "new installer"));
    let mut client = Client::start(&repo.dir, &[]);
    let summary = client.json("changes_summary", json!({}));
    assert_eq!(summary["range"]["head"], "worktree");
    assert_eq!(
        summary["range"]["notes"],
        json!(["compared with `origin/main`"])
    );
    let paths: Vec<&str> = summary["documents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|document| document["path"].as_str().unwrap())
        .collect();
    assert_eq!(
        paths,
        [
            "docs/decisions/0001-cache.md",
            "docs/new.md",
            "docs/guide.md"
        ]
    );
    // `base` names another default branch; `main` is where `origin/main` is.
    let main = client.json("changes_summary", json!({"base": "main"}));
    assert_eq!(main["range"]["notes"], json!(["compared with `main`"]));
    assert_eq!(
        client.refused(
            "changes_summary",
            json!({"range": "main..feature", "base": "main"})
        ),
        "`base` names the branch that `auto` compares with, so it goes only with the range \"auto\""
    );
    let missing = client.refused("changes_summary", json!({"base": "nope"}));
    assert!(missing.contains("nope"), "{missing}");
    client.finish();
}

#[test]
fn every_range_form_is_read_and_a_wrong_one_is_refused() {
    let repo = branch_repo();
    let mut client = Client::start(&repo.dir, &[]);
    let two = client.json("changes_summary", json!({"range": "main..feature"}));
    assert_eq!(two["page"]["total"], 2);
    let since = client.json(
        "changes_summary",
        json!({"range": "since/2026-01-02/feature"}),
    );
    assert_eq!(since["page"]["total"], 2);
    let everything = client.json(
        "changes_summary",
        json!({"range": "since/2025-01-01/feature"}),
    );
    assert_eq!(everything["range"]["base"], Value::Null);
    assert_eq!(everything["page"]["total"], 4);
    let not_a_range = client.refused("changes_summary", json!({"range": "main"}));
    assert!(
        not_a_range.starts_with("`main` is not a range; write"),
        "{not_a_range}"
    );
    let unknown = client.refused("changes_summary", json!({"range": "main..nope"}));
    assert_eq!(unknown, "`nope` does not name a commit");
    let date = client.refused("changes_summary", json!({"range": "since/last-week"}));
    assert!(date.starts_with("`last-week` is not a date"), "{date}");
    client.finish();
}

#[test]
fn a_long_summary_is_cut_into_pages() {
    let repo = TestRepo::new("mcp-many");
    repo.write("README.md", "# Readme\n");
    repo.commit("first", T0);
    for n in 0..60 {
        repo.write(
            &format!("docs/a-long-folder-name/document-number-{n:02}.md"),
            format!("# Document {n}\n\nIt holds {n} pages.\n"),
        );
    }
    let mut client = Client::start(&repo.dir, &[]);
    let first = client.json("changes_summary", json!({"range": "HEAD..", "limit": 10}));
    assert!(first["range"].is_object());
    assert_eq!(first["page"], json!({"offset": 0, "shown": 0, "total": 0}));
    // `HEAD..` is the same commit; `auto` holds the new files.
    let auto = client.json("changes_summary", json!({"base": "main", "limit": 25}));
    assert_eq!(auto["page"], json!({"offset": 0, "shown": 25, "total": 60}));
    let cut = &auto["cut"];
    assert_eq!(
        cut["reason"],
        "25 of 60 documents are listed, from number 0; ask with `offset` 25 for the rest"
    );
    assert_eq!(
        cut["next"],
        json!({"base": "main", "limit": 25, "offset": 25})
    );
    let next = client.json("changes_summary", cut["next"].clone());
    assert_eq!(
        next["page"],
        json!({"offset": 25, "shown": 25, "total": 60})
    );
    assert_eq!(next["totals"], auto["totals"]);
    let last = client.json(
        "changes_summary",
        json!({"base": "main", "offset": 50, "limit": 25}),
    );
    assert_eq!(
        last["page"],
        json!({"offset": 50, "shown": 10, "total": 60})
    );
    assert!(last.get("cut").is_none());
    let past = client.json("changes_summary", json!({"base": "main", "offset": 99}));
    assert_eq!(past["page"], json!({"offset": 99, "shown": 0, "total": 60}));
    // The size limit cuts before the count limit does.
    let small = client.json(
        "changes_summary",
        json!({"base": "main", "max_chars": 2000}),
    );
    let shown = small["page"]["shown"].as_u64().unwrap();
    assert!(shown > 0 && shown < 50, "{shown}");
    assert_eq!(small["cut"]["next"]["offset"], shown);
    let all = client.json(
        "changes_summary",
        json!({"base": "main", "limit": 500, "max_chars": 40000}),
    );
    assert_eq!(all["page"]["shown"], 60);
    client.finish();
}

#[test]
fn only_twenty_skipped_files_are_listed() {
    let repo = TestRepo::new("mcp-skipped");
    repo.write("README.md", "# Readme\n");
    repo.commit("first", T0);
    for n in 0..25 {
        repo.write(&format!("docs/latin1-{n:02}.md"), b"# Caf\xe9\n");
    }
    let mut client = Client::start(&repo.dir, &[]);
    let summary = client.json("changes_summary", json!({"base": "main"}));
    assert_eq!(summary["skipped"].as_array().unwrap().len(), 20);
    assert_eq!(summary["page"]["total"], 0);
    assert_eq!(
        summary["cut"],
        json!({"reason": "only the first 20 of 25 files that could not be compared are listed in `skipped`"})
    );
    client.finish();
}

// ---- changes_document ------------------------------------------------------

#[test]
fn one_documents_changes_are_the_diff_commands_json_without_unchanged() {
    let repo = branch_repo();
    let (code, diff, _) = driftmark(&repo.dir, &["diff", "main...feature", "--json"]);
    assert_eq!(code, 0);
    let diff: Value = serde_json::from_str(&diff).expect("JSON");
    let mut from_diff = diff["documents"][0].clone();
    assert_eq!(from_diff["path"], "docs/decisions/0001-cache.md");
    assert!(from_diff["unchanged"].is_array());
    from_diff.as_object_mut().unwrap().remove("unchanged");
    let mut client = Client::start(&repo.dir, &[]);
    let changes = client.json(
        "changes_document",
        json!({"range": "main...feature", "path": "docs/decisions/0001-cache.md"}),
    );
    assert_eq!(changes["schema_version"], 2);
    assert_eq!(changes["range"], diff["range"]);
    assert_eq!(changes["documents"], json!([from_diff]));
    assert_eq!(
        changes["documents"][0]["status"],
        json!({"old": "proposed", "new": "accepted"})
    );
    assert_eq!(
        changes["page"],
        json!({"offset": 0, "shown": 3, "total": 3})
    );
    assert_eq!(changes["skipped"], json!([]));
    client.finish();
}

#[test]
fn a_new_documents_full_text_can_be_left_out() {
    let repo = branch_repo();
    let mut client = Client::start(&repo.dir, &[]);
    let path = json!("docs/new.md");
    let full = client.json(
        "changes_document",
        json!({"range": "main...feature", "path": path}),
    );
    let change = &full["documents"][0]["changes"][0];
    assert_eq!(change["kind"], "added");
    assert_eq!(change["document"]["source"], NEW_PAGE);
    let short = client.json(
        "changes_document",
        json!({"range": "main...feature", "path": path, "full_text": false}),
    );
    let document = &short["documents"][0]["changes"][0]["document"];
    assert_eq!(document["title"], "New page");
    assert_eq!(document["bytes"], NEW_PAGE.len());
    assert!(document.get("source").is_none());
    client.finish();
}

#[test]
fn many_changes_are_cut_into_pages_and_one_too_large_is_trimmed() {
    let repo = TestRepo::new("mcp-changes");
    let before = numbered(40, |n| format!("Paragraph {n} says one thing.\n\n"));
    repo.write("docs/long.md", format!("# Long\n\n{before}"));
    repo.commit("first", T0);
    let after = numbered(40, |n| format!("Paragraph {n} says {n} things.\n\n"));
    repo.write("docs/long.md", format!("# Long\n\n{after}"));
    let huge = numbered(3000, |n| format!("Line {n} of a long new page.\n"));
    repo.write("docs/huge.md", format!("# Huge\n\n{huge}"));
    let mut client = Client::start(&repo.dir, &[]);
    let range = json!("HEAD..worktree");
    // `HEAD..worktree` names a branch `worktree`, which does not exist.
    assert!(
        client
            .refused(
                "changes_document",
                json!({"range": range, "path": "docs/long.md"})
            )
            .contains("worktree")
    );
    let first = client.json(
        "changes_document",
        json!({"base": "main", "path": "docs/long.md", "limit": 15}),
    );
    assert_eq!(
        first["page"],
        json!({"offset": 0, "shown": 15, "total": 40})
    );
    assert_eq!(
        first["cut"]["next"],
        json!({"base": "main", "path": "docs/long.md", "limit": 15, "offset": 15})
    );
    let rest = client.json(
        "changes_document",
        json!({"base": "main", "path": "docs/long.md", "offset": 30, "limit": 15}),
    );
    assert_eq!(
        rest["page"],
        json!({"offset": 30, "shown": 10, "total": 40})
    );
    assert_eq!(
        rest["documents"][0]["changes"][0]["new"]["block"]["text"],
        "Paragraph 30 says 30 things."
    );
    let small = client.json(
        "changes_document",
        json!({"base": "main", "path": "docs/long.md", "max_chars": 3000}),
    );
    let shown = small["page"]["shown"].as_u64().unwrap();
    assert!(shown > 0 && shown < 40, "{shown}");

    // The new page's text alone is larger than the limit: the change is
    // shown without it, and the cut says so.
    let huge = client.json(
        "changes_document",
        json!({"base": "main", "path": "docs/huge.md"}),
    );
    let change = &huge["documents"][0]["changes"][0];
    assert_eq!(change["document"]["title"], "Huge");
    assert!(change["document"].get("source").is_none());
    let reason = huge["cut"]["reason"].as_str().unwrap();
    assert!(
        reason.starts_with("change number 0 is too large to show whole"),
        "{reason}"
    );
    let whole = client.json(
        "changes_document",
        json!({"base": "main", "path": "docs/huge.md", "full_text": false}),
    );
    assert!(whole.get("cut").is_none());
    client.finish();
}

#[test]
fn a_document_outside_the_range_or_the_repository_is_refused() {
    let repo = branch_repo();
    let mut client = Client::start(&repo.dir, &[]);
    let range = json!("main...feature");
    assert_eq!(
        client.refused(
            "changes_document",
            json!({"range": range, "path": "docs/guide.md"})
        ),
        "`docs/guide.md` did not change in this range"
    );
    assert_eq!(
        client.refused(
            "changes_document",
            json!({"range": range, "path": "docs/../x.md"})
        ),
        "`docs/../x.md` is not a path inside the repository; it cannot be empty or hold `.` or `..` parts"
    );
    assert_eq!(
        client.refused(
            "changes_document",
            json!({"range": range, "path": "driftmark.toml"})
        ),
        "`driftmark.toml` is not a Markdown file; only `.md` and `.markdown` files are shown"
    );
    let outside = client.refused(
        "changes_document",
        json!({"range": range, "path": "/etc/hosts.md"}),
    );
    assert_eq!(
        outside,
        "`/etc/hosts.md` is outside the repository; give a path from the repository root, such as docs/guide.md"
    );
    // A path from the root of the file system works inside the repository,
    // and so does a leading `./`.
    let full = repo.dir.join("docs/new.md");
    let by_full_path = client.json(
        "changes_document",
        json!({"range": range, "path": full.to_str().unwrap()}),
    );
    assert_eq!(by_full_path["documents"][0]["path"], "docs/new.md");
    let dotted = client.json(
        "changes_document",
        json!({"range": range, "path": "./docs/new.md"}),
    );
    assert_eq!(dotted["documents"][0]["path"], "docs/new.md");
    client.finish();
}

// ---- document_read ---------------------------------------------------------

#[test]
fn a_document_comes_with_its_metadata_outline_and_text() {
    let repo = branch_repo();
    repo.git(&["checkout", "-q", "feature"]);
    let mut client = Client::start(&repo.dir, &[]);
    let read = client.json(
        "document_read",
        json!({"path": "docs/decisions/0001-cache.md"}),
    );
    assert_eq!(read["schema_version"], 1);
    assert_eq!(read["revision"], "worktree");
    assert_eq!(read["commit"], "worktree");
    assert!(read.get("blob").is_none());
    assert_eq!(read["type"], "decision");
    assert_eq!(read["status"], "accepted");
    assert_eq!(read["lifecycle"], "current");
    assert_eq!(read["title"], "Cache pages");
    assert_eq!(read["summary"], "Short version: the server keeps pages.");
    assert_eq!(read["bytes"], DECISION_AFTER.len());
    assert_eq!(read["lines"], DECISION_AFTER.lines().count());
    assert_eq!(read["text"], DECISION_AFTER);
    assert_eq!(
        read["text_range"],
        json!({"start": 0, "end": DECISION_AFTER.len(), "first_line": 1, "last_line": 17})
    );
    let from = |heading: &str| DECISION_AFTER.len() - DECISION_AFTER.find(heading).unwrap() - 1;
    let decision =
        DECISION_AFTER.find("## Notes").unwrap() - DECISION_AFTER.find("## Decision").unwrap() - 2;
    assert_eq!(
        read["outline"],
        json!([
            {"path": ["Cache pages"], "anchor": "cache-pages", "block": 1, "line": 5, "bytes": from("# Cache")},
            {"path": ["Cache pages", "Decision"], "anchor": "decision", "block": 3, "line": 9, "bytes": decision},
            {"path": ["Cache pages", "Notes"], "anchor": "notes", "block": 6, "line": 15, "bytes": from("## Notes")},
        ])
    );
    assert!(read.get("cut").is_none());
    let commit = repo.git(&["rev-parse", "main"]);
    let old = client.json(
        "document_read",
        json!({"path": "docs/decisions/0001-cache.md", "revision": "main", "outline": false}),
    );
    assert_eq!(old["commit"], commit);
    assert!(old["blob"].is_string());
    assert_eq!(old["status"], "proposed");
    assert_eq!(old["text"], DECISION_BEFORE);
    assert!(old.get("outline").is_none());
    client.finish();
}

#[test]
fn a_section_is_read_by_its_anchor() {
    let repo = branch_repo();
    let mut client = Client::start(&repo.dir, &[]);
    let section = client.json(
        "document_read",
        json!({"path": "docs/guide.md", "section": "configure"}),
    );
    let text = "## Configure\n\nSet the lantern size.\n\n> ### Quoted part\n>\n> A quoted heading.";
    assert_eq!(section["text"], text);
    assert_eq!(
        section["section"],
        json!({"path": ["Guide", "Configure"], "anchor": "configure", "block": 4, "bytes": text.len()})
    );
    assert_eq!(
        section["outline"],
        json!([{"path": ["Guide", "Configure"], "anchor": "configure", "block": 4, "line": 9, "bytes": text.len()}])
    );
    assert_eq!(section["text_range"]["first_line"], 9);
    // A heading inside a quote opens no section: its anchor reads the
    // section around it. `#` and another case are accepted.
    let quoted = client.json(
        "document_read",
        json!({"path": "docs/guide.md", "section": "#Quoted-Part"}),
    );
    assert_eq!(quoted["text"], text);
    let missing = client.refused(
        "document_read",
        json!({"path": "docs/guide.md", "section": "usage"}),
    );
    assert_eq!(
        missing,
        "`docs/guide.md` has no heading with the anchor `usage`; its sections' anchors are: guide, install, configure"
    );
    client.finish();
}

#[test]
fn a_long_document_is_read_in_parts_that_join_into_the_whole() {
    let repo = TestRepo::new("mcp-long");
    let mut source = String::from("# Long\n\n> Short version: a long page.\n\n");
    for n in 0..120 {
        let _ = write!(
            source,
            "## Part {n}\n\n### Detail {n}\n\nThe part number {n} says \"quoted\" things.\n\n"
        );
    }
    repo.write("docs/long.md", &source);
    let mut client = Client::start(&repo.dir, &[]);
    let first = client.json(
        "document_read",
        json!({"path": "docs/long.md", "max_chars": 4000}),
    );
    let outline = first["outline"].as_array().unwrap();
    assert!(outline.len() < 241, "{}", outline.len());
    let reason = first["cut"]["reason"].as_str().unwrap();
    assert!(reason.contains("the text was cut after line"), "{reason}");
    assert!(
        reason.contains("leaving out the deepest headings"),
        "{reason}"
    );
    let mut text = first["text"].as_str().unwrap().to_owned();
    assert!(text.ends_with('\n'), "a part ends after a whole line");
    let mut parts = 1;
    let mut answer = first;
    while let Some(next) = answer["cut"].get("next").cloned() {
        assert_eq!(next["outline"], false);
        answer = client.json("document_read", next);
        assert!(answer.get("outline").is_none());
        assert_eq!(
            usize::try_from(answer["text_range"]["start"].as_u64().unwrap()).unwrap(),
            text.len()
        );
        text.push_str(answer["text"].as_str().unwrap());
        parts += 1;
        assert!(parts < 100, "reading on never ends");
    }
    assert_eq!(text, source);
    assert!(parts >= 3, "{parts}");
    // A section of a long document is read the same way.
    let section = client.json(
        "document_read",
        json!({"path": "docs/long.md", "section": "part-7", "outline": false}),
    );
    assert_eq!(
        section["text"],
        "## Part 7\n\n### Detail 7\n\nThe part number 7 says \"quoted\" things."
    );
    let outside = client.refused(
        "document_read",
        json!({"path": "docs/long.md", "section": "part-7", "offset": 1}),
    );
    assert!(
        outside.starts_with("`offset` 1 is outside the text read, which runs from byte"),
        "{outside}"
    );
    client.finish();
}

#[test]
fn a_line_longer_than_the_limit_is_cut_inside_and_read_on() {
    let repo = TestRepo::new("mcp-one-line");
    let line = "word ".repeat(5_000);
    let source = format!("# One line\n\n{line}\n");
    repo.write("docs/line.md", &source);
    let mut client = Client::start(&repo.dir, &[]);
    let mut answer = client.json(
        "document_read",
        json!({"path": "docs/line.md", "max_chars": 4000}),
    );
    let mut text = answer["text"].as_str().unwrap().to_owned();
    assert_eq!(text, "# One line\n\n", "whole lines first");
    let mut parts = 1;
    while let Some(next) = answer["cut"].get("next").cloned() {
        answer = client.json("document_read", next);
        text.push_str(answer["text"].as_str().unwrap());
        parts += 1;
        assert!(parts < 50, "reading on never ends");
    }
    assert_eq!(text, source);
    assert!(parts > 5, "{parts}");
    client.finish();
}

#[test]
fn an_answer_that_cannot_fit_even_cut_is_refused() {
    let repo = TestRepo::new("mcp-too-large");
    let summary = "A summary that goes on. ".repeat(150);
    repo.write("docs/wide.md", format!("# Wide\n\n> {summary}\n\nText.\n"));
    let mut client = Client::start(&repo.dir, &[]);
    let refused = client.refused(
        "document_read",
        json!({"path": "docs/wide.md", "max_chars": 2000}),
    );
    assert!(
        refused.starts_with("the answer needs ")
            && refused.ends_with(
                "characters even after cutting, more than the limit of 2000; ask again with a larger `max_chars`, up to 40000"
            ),
        "{refused}"
    );
    let read = client.json(
        "document_read",
        json!({"path": "docs/wide.md", "max_chars": 8000}),
    );
    assert_eq!(read["title"], "Wide");
    client.finish();
}

#[test]
fn a_document_that_cannot_be_read_is_refused() {
    let repo = branch_repo();
    repo.write("docs/latin1.md", b"# Caf\xe9\n");
    let mut client = Client::start(&repo.dir, &[]);
    let not_utf8 = client.refused("document_read", json!({"path": "docs/latin1.md"}));
    assert!(
        not_utf8.starts_with(
            "`docs/latin1.md` at `worktree` cannot be read as Markdown: the file is not UTF-8 text"
        ),
        "{not_utf8}"
    );
    assert_eq!(
        client.refused("document_read", json!({"path": "docs/missing.md"})),
        "the working tree has no file `docs/missing.md`"
    );
    assert_eq!(
        client.refused(
            "document_read",
            json!({"path": "docs/new.md", "revision": "main"})
        ),
        "`main` has no Markdown document `docs/new.md`"
    );
    assert_eq!(
        client.refused(
            "document_read",
            json!({"path": "docs/guide.md", "revision": "nope"})
        ),
        "`nope` does not name a commit"
    );
    client.finish();
}

// ---- search ------------------------------------------------------------------

#[test]
fn search_finds_words_at_a_revision_and_in_the_files_on_disk() {
    let repo = branch_repo();
    let mut client = Client::start(&repo.dir, &[]);
    let found = client.json("search", json!({"query": "lantern"}));
    assert_eq!(found["schema_version"], 1);
    assert_eq!(found["revision"], "worktree");
    assert_eq!(found["documents"], 3);
    assert_eq!(found["total"], 1);
    assert_eq!(
        found["results"],
        json!([{
            "path": "docs/guide.md",
            "title": "Guide",
            "type": "document",
            "block": 5,
            "headings": ["Guide", "Configure"],
            "anchor": "configure",
            "snippet": "Set the **lantern** size."
        }])
    );
    // A word written after the first search is found: the index follows the
    // files on disk.
    repo.write("docs/guide.md", GUIDE.replace("lantern", "beacon"));
    let moved = client.json("search", json!({"query": "beacon"}));
    assert_eq!(moved["total"], 1);
    assert_eq!(
        client.json("search", json!({"query": "lantern"}))["total"],
        0
    );
    // At a commit, the committed text is searched.
    let committed = client.json("search", json!({"query": "lantern", "revision": "main"}));
    assert_eq!(committed["total"], 1);
    assert_eq!(committed["commit"], repo.git(&["rev-parse", "main"]));
    let feature = client.json(
        "search",
        json!({"query": "\"open notes\"", "revision": "feature", "types": ["decision"]}),
    );
    assert_eq!(feature["results"][0]["anchor"], "decision");
    assert_eq!(feature["results"][0]["type"], "decision");
    let filtered = client.json(
        "search",
        json!({"query": "cache", "revision": "feature", "path": "docs/decisions/"}),
    );
    assert_eq!(filtered["total"], 1);
    let none = client.json("search", json!({"query": ""}));
    assert_eq!(none["total"], 0);
    let long = vec!["word"; 33].join(" ");
    assert_eq!(
        client.refused("search", json!({"query": long})),
        "the query holds 33 words; a query holds at most 32"
    );
    assert_eq!(
        client.refused("search", json!({"query": "x", "revision": "nope"})),
        "`nope` does not name a commit"
    );
    client.finish();
}

#[test]
fn many_results_are_cut_into_pages() {
    let repo = TestRepo::new("mcp-search");
    for n in 0..30 {
        repo.write(
            &format!("docs/page-{n:02}.md"),
            format!(
                "# Page {n}\n\nThe lantern stands in room {n}, {}.\n",
                "far away ".repeat(20)
            ),
        );
    }
    let mut client = Client::start(&repo.dir, &[]);
    let first = client.json("search", json!({"query": "lantern"}));
    assert_eq!(first["total"], 30);
    assert_eq!(
        first["page"],
        json!({"offset": 0, "shown": 10, "total": 30})
    );
    assert_eq!(
        first["cut"]["next"],
        json!({"query": "lantern", "offset": 10})
    );
    let second = client.json("search", first["cut"]["next"].clone());
    assert_eq!(
        second["page"],
        json!({"offset": 10, "shown": 10, "total": 30})
    );
    assert_ne!(second["results"][0], first["results"][0]);
    let small = client.json(
        "search",
        json!({"query": "lantern", "limit": 50, "max_chars": 2000}),
    );
    let shown = small["page"]["shown"].as_u64().unwrap();
    assert!(shown > 0 && shown < 10, "{shown}");
    assert_eq!(
        client.refused("search", json!({"query": "lantern", "limit": 51})),
        "`limit` must be a whole number from 1 to 50"
    );
    client.finish();
}

// ---- lint --------------------------------------------------------------------

#[test]
fn lint_checks_the_files_changed_since_the_last_commit() {
    let repo = branch_repo();
    repo.write("docs/old-problem.md", "No title here.\n");
    repo.commit("a problem already committed", T0 + 2 * DAY);
    repo.write("docs/guide.md", format!("{GUIDE}\n```\ncode\n```\n"));
    repo.write("docs/fresh.md", "Text before the title.\n\n# Fresh\n");
    repo.write("docs/broken/skip.md", "no title\n");
    let mut client = Client::start(&repo.dir, &[]);
    let lint = client.json("lint", json!({}));
    assert_eq!(lint["schema_version"], 1);
    assert_eq!(lint["changed"], true);
    assert_eq!(lint["files"], 2);
    assert_eq!(lint["errors"], 1);
    assert_eq!(lint["warnings"], 2);
    assert_eq!(
        lint["findings"],
        json!([
            {"path": "docs/fresh.md", "line": 1, "column": 1, "severity": "error", "rule": "title",
             "message": "this comes before the `#` title, and only front matter may; move it below the title"},
            {"path": "docs/fresh.md", "line": 3, "column": 1, "severity": "warning", "rule": "summary",
             "message": "the document has no summary before its first section; add a paragraph or a `>` quote under the title that says in a sentence or two what the document is for"},
            {"path": "docs/guide.md", "line": 17, "column": 1, "severity": "warning", "rule": "code-language",
             "message": "this code block names no language; write it after the opening fence, such as ```sh, or ```text for plain text"},
        ])
    );
    // Files and folders by name: a folder skips excluded files, a file
    // named on its own is checked.
    let folder = client.json("lint", json!({"paths": ["docs/broken", "docs/decisions/"]}));
    assert_eq!(folder["files"], 1);
    assert!(folder.get("changed").is_none());
    let named = client.json("lint", json!({"paths": ["docs/broken/skip.md"]}));
    assert_eq!(named["files"], 1);
    assert_eq!(named["findings"][0]["rule"], "title");
    // The whole repository: five files, the excluded one left out.
    let everything = client.json("lint", json!({"paths": ["."]}));
    assert_eq!(everything["files"], 5);
    assert_eq!(
        client.refused("lint", json!({"paths": ["docs/nothing"]})),
        "`docs/nothing` is neither a Markdown file git lists in the working tree nor a folder that holds one"
    );
    assert_eq!(
        client.refused("lint", json!({"paths": ["../outside"]})),
        "`../outside`: `../outside` is not a folder inside the repo; write it like `docs` or `specs/api`"
    );
    client.finish();
}

#[test]
fn many_findings_are_cut_into_pages() {
    let repo = TestRepo::new("mcp-lint");
    let code = numbered(80, |n| format!("```\nblock {n}\n```\n\n"));
    repo.write(
        "docs/many.md",
        format!("# Many\n\n> Short version: many blocks.\n\n{code}"),
    );
    let mut client = Client::start(&repo.dir, &[]);
    let first = client.json("lint", json!({}));
    assert_eq!(first["warnings"], 80);
    assert_eq!(
        first["page"],
        json!({"offset": 0, "shown": 50, "total": 80})
    );
    assert_eq!(first["cut"]["next"], json!({"offset": 50}));
    let rest = client.json("lint", first["cut"]["next"].clone());
    assert_eq!(
        rest["page"],
        json!({"offset": 50, "shown": 30, "total": 80})
    );
    assert!(rest.get("cut").is_none());
    let small = client.json("lint", json!({"max_chars": 2000}));
    let shown = small["page"]["shown"].as_u64().unwrap();
    assert!(shown > 0 && shown < 50, "{shown}");
    client.finish();
}

#[cfg(unix)]
#[test]
fn nothing_is_read_through_a_symbolic_link() {
    let repo = branch_repo();
    let outside = support::TempDir::new("mcp-outside");
    std::fs::write(
        outside.path().join("secret.md"),
        "# Secret\n\n## Hidden heading\n",
    )
    .unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("secret.md"),
        repo.dir.join("docs/link.md"),
    )
    .unwrap();
    repo.write(
        "docs/points.md",
        "# Points\n\n> Short version: links out.\n\n[A](link.md#hidden-heading) and [B](link.md#nothing).\n",
    );
    let mut client = Client::start(&repo.dir, &[]);
    assert_eq!(
        client.refused("document_read", json!({"path": "docs/link.md"})),
        "the working tree has no file `docs/link.md`"
    );
    assert_eq!(
        client.refused("lint", json!({"paths": ["docs/link.md"]})),
        "`docs/link.md` is neither a Markdown file git lists in the working tree nor a folder that holds one"
    );
    // The link check sees the name, but never reads the headings behind it,
    // so neither anchor is judged.
    let lint = client.json("lint", json!({"paths": ["docs/points.md"]}));
    assert_eq!(lint["findings"], json!([]));
    let found = client.json("search", json!({"query": "hidden"}));
    assert_eq!(found["total"], 0);
    client.finish();
}

// ---- safety and the command line -------------------------------------------

#[test]
fn no_tool_changes_the_repository() {
    let repo = branch_repo();
    repo.write("docs/guide.md", GUIDE.replace("installer", "new installer"));
    let before = repo.git(&[
        "status",
        "--porcelain",
        "--ignored",
        "--untracked-files=all",
    ]);
    let mut client = Client::start(&repo.dir, &[]);
    client.json("changes_summary", json!({}));
    client.json("changes_document", json!({"path": "docs/guide.md"}));
    client.json("document_read", json!({"path": "docs/guide.md"}));
    client.json("search", json!({"query": "installer"}));
    client.json("lint", json!({}));
    client.finish();
    let after = repo.git(&[
        "status",
        "--porcelain",
        "--ignored",
        "--untracked-files=all",
    ]);
    assert_eq!(before, after);
}

#[test]
fn a_bare_repository_is_read_at_head() {
    let repo = branch_repo();
    let bare = support::TempDir::new("mcp-bare");
    let dir = bare.path().join("repo.git");
    let status = Command::new("git")
        .args(["clone", "-q", "--bare"])
        .arg(&repo.dir)
        .arg(&dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .status()
        .expect("git clone");
    assert!(status.success());
    let mut client = Client::start(&dir, &[]);
    let read = client.json("document_read", json!({"path": "docs/guide.md"}));
    assert_eq!(read["revision"], "HEAD");
    let found = client.json("search", json!({"query": "lantern"}));
    assert_eq!(found["revision"], "HEAD");
    assert_eq!(
        client.refused("lint", json!({})),
        "the repository is bare, so it has no files to check"
    );
    assert_eq!(
        client.refused("search", json!({"query": "x", "revision": "worktree"})),
        "the repo has no working tree"
    );
    client.finish();
}

#[test]
fn the_command_line_is_checked_before_anything_starts() {
    let repo = branch_repo();
    let (code, out, _) = driftmark(&repo.dir, &["mcp", "--help"]);
    assert_eq!(code, 0);
    assert!(out.starts_with("usage: driftmark mcp"), "{out}");
    let (code, _, err) = driftmark(&repo.dir, &["mcp", "--nope"]);
    assert_eq!(code, 2);
    assert!(
        err.starts_with("driftmark mcp: unknown argument `--nope`"),
        "{err}"
    );
    let nowhere = support::TempDir::new("mcp-nowhere");
    let (code, _, err) = driftmark(nowhere.path(), &["mcp"]);
    assert_eq!(code, 1);
    assert!(err.contains("is not inside a git repository"), "{err}");
    // `--repo` names the repository from anywhere.
    let mut client = Client::start(nowhere.path(), &["--repo", repo.dir.to_str().unwrap()]);
    assert_eq!(
        client.json("document_read", json!({"path": "README.md"}))["title"],
        "Readme"
    );
    client.finish();
}

// ---- install -------------------------------------------------------------------

#[test]
fn install_prints_the_entry_and_writes_it_only_when_asked() {
    let repo = branch_repo();
    let (code, out, _) = driftmark(&repo.dir.join("docs"), &["mcp", "install"]);
    assert_eq!(code, 0);
    assert!(out.contains("\"command\": \"driftmark\""), "{out}");
    assert!(
        out.contains("claude mcp add --transport stdio --scope project driftmark -- driftmark mcp"),
        "{out}"
    );
    assert!(!repo.dir.join(".mcp.json").exists());
    let (code, out, _) = driftmark(&repo.dir.join("docs"), &["mcp", "install", "--write"]);
    assert_eq!(code, 0, "{out}");
    let written = std::fs::read_to_string(repo.dir.join(".mcp.json")).unwrap();
    assert_eq!(
        written,
        "{\n  \"mcpServers\": {\n    \"driftmark\": {\n      \"type\": \"stdio\",\n      \"command\": \"driftmark\",\n      \"args\": [\n        \"mcp\"\n      ]\n    }\n  }\n}\n"
    );
    let (code, out, _) = driftmark(&repo.dir, &["mcp", "install", "--write"]);
    assert_eq!(code, 0);
    assert_eq!(
        out,
        ".mcp.json already starts `driftmark mcp` as the server `driftmark`; nothing to add\n\nAgents learn about the reader's notes from the `notes_list` tool. `driftmark mcp install --notes-line` shows a line that tells them to call it, for AGENTS.md or CLAUDE.md.\n"
    );
}

#[test]
fn install_adds_the_line_about_notes_only_when_asked() {
    let repo = branch_repo();
    // Without an instruction file, the line is shown to add by hand.
    let (code, out, _) = driftmark(&repo.dir, &["mcp", "install", "--notes-line"]);
    assert_eq!(code, 0);
    assert!(
        out.contains("There is no AGENTS.md or CLAUDE.md at the repository root."),
        "{out}"
    );
    // CLAUDE.md gets it when there is no AGENTS.md; nothing is written
    // without `--write`, and `--write` alone leaves it alone.
    repo.write("CLAUDE.md", "# Rules\r\n\r\nBe brief.");
    let (_, out, _) = driftmark(&repo.dir, &["mcp", "install", "--notes-line"]);
    assert!(
        out.contains("It would add this line at the end of CLAUDE.md"),
        "{out}"
    );
    driftmark(&repo.dir, &["mcp", "install", "--write"]);
    assert_eq!(
        std::fs::read_to_string(repo.dir.join("CLAUDE.md")).unwrap(),
        "# Rules\r\n\r\nBe brief."
    );
    // AGENTS.md comes first, and keeps its own line endings.
    repo.write("AGENTS.md", "# Agents\n\nBe kind.\n");
    let (code, out, _) = driftmark(&repo.dir, &["mcp", "install", "--notes-line", "--write"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.ends_with("Added a line about the reader's notes at the end of AGENTS.md.\n"),
        "{out}"
    );
    let agents = std::fs::read_to_string(repo.dir.join("AGENTS.md")).unwrap();
    assert!(
        agents.starts_with("# Agents\n\nBe kind.\n\nReader's notes: "),
        "{agents}"
    );
    assert!(agents.ends_with("`note_mark_applied`.\n"), "{agents}");
    assert!(agents.contains("call the driftmark tool `notes_list`"));
    // Running it again adds nothing.
    let (_, out, _) = driftmark(&repo.dir, &["mcp", "install", "--notes-line", "--write"]);
    assert!(
        out.ends_with("AGENTS.md already tells agents about `notes_list`; nothing to add there\n"),
        "{out}"
    );
    assert_eq!(
        std::fs::read_to_string(repo.dir.join("AGENTS.md")).unwrap(),
        agents
    );
    assert_eq!(
        std::fs::read_to_string(repo.dir.join("CLAUDE.md")).unwrap(),
        "# Rules\r\n\r\nBe brief."
    );
}

#[cfg(unix)]
#[test]
fn install_never_writes_through_a_linked_instruction_file() {
    let repo = branch_repo();
    let outside = support::TempDir::new("mcp-outside");
    let target = outside.path().join("AGENTS.md");
    std::fs::write(&target, "# Elsewhere\n").unwrap();
    std::os::unix::fs::symlink(&target, repo.dir.join("AGENTS.md")).unwrap();
    let (code, out, _) = driftmark(&repo.dir, &["mcp", "install", "--notes-line", "--write"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("AGENTS.md is a link, so driftmark leaves it alone."),
        "{out}"
    );
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "# Elsewhere\n");
}

#[test]
fn install_keeps_other_servers_and_the_file_style() {
    let repo = branch_repo();
    let original = "{\r\n\t\"mcpServers\": {\r\n\t\t\"other\": {\r\n\t\t\t\"command\": \"npx\",\r\n\t\t\t\"args\": [\"-y\", \"x\"]\r\n\t\t}\r\n\t},\r\n\t\"extra\": 1e3\r\n}";
    repo.write(".mcp.json", original);
    let (code, _, err) = driftmark(&repo.dir, &["mcp", "install", "--write"]);
    assert_eq!(code, 0, "{err}");
    let written = std::fs::read_to_string(repo.dir.join(".mcp.json")).unwrap();
    assert!(
        written.starts_with("{\r\n\t\"mcpServers\": {\r\n\t\t\"other\": {"),
        "{written}"
    );
    assert!(!written.ends_with('\n'));
    let parsed: Value = serde_json::from_str(&written).unwrap();
    assert_eq!(parsed["mcpServers"]["other"]["args"], json!(["-y", "x"]));
    assert_eq!(parsed["mcpServers"]["driftmark"]["args"], json!(["mcp"]));
    assert_eq!(parsed["extra"], 1000.0);
    let keys: Vec<&String> = parsed["mcpServers"].as_object().unwrap().keys().collect();
    assert_eq!(keys, ["other", "driftmark"]);
}

#[test]
fn install_refuses_a_file_it_cannot_add_to() {
    let repo = branch_repo();
    for (text, why) in [
        ("{ not json", ".mcp.json is not valid JSON"),
        ("[]", "the top level is not an object in .mcp.json"),
        (
            "{\"mcpServers\": []}",
            "`mcpServers` is not an object in .mcp.json",
        ),
        (
            "{\"mcpServers\": {\"driftmark\": {\"command\": \"other\"}}}",
            ".mcp.json already has a server named `driftmark` that runs `other`",
        ),
    ] {
        repo.write(".mcp.json", text);
        for args in [&["mcp", "install"][..], &["mcp", "install", "--write"]] {
            let (code, _, err) = driftmark(&repo.dir, args);
            assert_eq!(code, 1, "{text}");
            assert!(err.contains(why), "{text}: {err}");
            assert_eq!(
                std::fs::read_to_string(repo.dir.join(".mcp.json")).unwrap(),
                text
            );
        }
    }
    let (code, _, err) = driftmark(&repo.dir, &["mcp", "install", "--force"]);
    assert_eq!(code, 2);
    assert!(err.contains("`install` takes only `--write`"), "{err}");
    let nowhere = support::TempDir::new("mcp-install-nowhere");
    let (code, _, err) = driftmark(nowhere.path(), &["mcp", "install"]);
    assert_eq!(code, 1);
    assert!(err.contains("run it inside a git repository"), "{err}");
}

#[test]
fn the_installed_entry_starts_a_working_connector() {
    let repo = branch_repo();
    let (code, _, _) = driftmark(&repo.dir, &["mcp", "install", "--write"]);
    assert_eq!(code, 0);
    let file: Value =
        serde_json::from_str(&std::fs::read_to_string(repo.dir.join(".mcp.json")).unwrap())
            .unwrap();
    let entry = &file["mcpServers"]["driftmark"];
    assert_eq!(entry["command"], "driftmark");
    let args: Vec<&str> = entry["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|arg| arg.as_str().unwrap())
        .collect();
    // Started the way Claude Code starts it: the entry's arguments, in the
    // project folder. The test runs the program it built in place of the
    // `driftmark` on the PATH.
    assert_eq!(args[0], "mcp");
    let mut client = Client::start(&repo.dir, &args[1..]);
    let summary = client.json("changes_summary", json!({"range": "main...feature"}));
    assert_eq!(summary["page"]["total"], 2);
    client.finish();
}

// ---- the documented examples ---------------------------------------------------

#[test]
fn the_documented_examples_hold() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/features/commands/mcp.md");
    // Git for Windows may check the document out with CRLF line endings.
    let document = std::fs::read_to_string(path)
        .expect("the document")
        .replace("\r\n", "\n");
    // The note tools' examples need notes; `mcp_notes.rs` checks them.
    let examples: Vec<_> = examples(&document)
        .into_iter()
        .filter(|(tool, _, _)| !support::NOTE_TOOLS.contains(&tool.as_str()))
        .collect();
    assert!(examples.len() >= 9, "{} examples", examples.len());
    let repo = documented_repo();
    let mut client = Client::start(&repo.dir, &[]);
    let mut wrong = Vec::new();
    for (tool, arguments, expected) in examples {
        let answer = client.call(&tool, arguments.clone());
        match expected {
            Expected::Answer(value) => {
                assert!(!answer.is_error, "{tool} {arguments}: {}", answer.text);
                let got: Value = serde_json::from_str(&answer.text).expect("JSON");
                if got != value {
                    wrong.push(format!(
                        "{tool} {arguments} answers:\n{}",
                        serde_json::to_string_pretty(&got).expect("JSON")
                    ));
                }
            }
            Expected::Refusal(text) => {
                assert!(answer.is_error, "{tool} {arguments}: {}", answer.text);
                if answer.text != text {
                    wrong.push(format!("{tool} {arguments} refuses:\n{}", answer.text));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n\n"));
    client.finish();
}
