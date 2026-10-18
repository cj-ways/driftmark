//! The note tools of `driftmark mcp`, with `driftmark serve` running beside
//! them on the same notes file: one whole review round between a reader's
//! page and an agent, every action an agent may not take, the size limits,
//! and the documented examples of `docs/features/commands/mcp.md`.
//!
//! Both programs run as the real binary on small repos built for each test.
//! The reader's side speaks HTTP as the server's own pages do; the agent's
//! side speaks the JSON-RPC lines an MCP client such as Claude Code sends.

mod support;

use std::fmt::Write as _;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use serde_json::{Value, json};
use support::{DAY, Expected, NOTE_TOOLS, T0, TestRepo, documented_repo, examples};

const PROGRAM: &str = env!("CARGO_BIN_EXE_driftmark");

const CACHE: &str = "# Cache pages\n\n> Short version: how long the server keeps pages.\n\n## Decision\n\nKeep each page for one hour, then drop it from the cache.\n\nA page with open notes is never cached.\n\n## Limits\n\nThe cache holds at most 64 MB.\n";

const CACHE_FEATURE: &str = "# Cache pages\n\n> Short version: how long the server keeps pages.\n\n## Decision\n\nKeep each page for two hours, then drop it from the cache.\n\nA page with open notes is never cached.\n\n## Limits\n\nThe cache holds at most 128 MB.\n";

const GUIDE: &str = "# Guide\n\n> Short version: how to run the cache.\n\n## Install\n\nRun the installer once on each server.\n";

// ---- the reader: driftmark serve -------------------------------------------

/// The browser the reader's page runs in, as its headers name it.
const BROWSER: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:131.0) Gecko/20100101 Firefox/131.0";

struct Server {
    child: Child,
    addr: SocketAddr,
    /// The token the start message printed.
    token: String,
    /// The marker of a page this server sent, as the page script keeps it.
    marker: String,
}

impl Server {
    fn start(repo: &Path) -> Server {
        let mut child = Command::new(PROGRAM)
            .arg("serve")
            .arg("--repo")
            .arg(repo)
            .args(["--addr", "127.0.0.1:0", "--reader", "Ana"])
            .env_remove("DRIFTMARK_TOKEN")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("start driftmark serve");
        let mut reader = BufReader::new(child.stdout.take().expect("standard output"));
        let mut open = None;
        loop {
            let mut line = String::new();
            assert!(reader.read_line(&mut line).expect("read") > 0, "stopped");
            if let Some(rest) = line.strip_prefix("driftmark serve: open http://127.0.0.1:") {
                open = Some(rest.trim_end().to_owned());
            }
            if line.contains("press Ctrl-C") {
                break;
            }
        }
        std::thread::spawn(move || {
            let _ = std::io::copy(&mut reader, &mut std::io::sink());
        });
        let open = open.expect("an address");
        let (port, token) = open.split_once("/?token=").expect("the token link");
        let mut server = Server {
            child,
            addr: format!("127.0.0.1:{port}").parse().expect("an address"),
            token: token.to_owned(),
            marker: String::new(),
        };
        let home = server.page("/");
        home.split("<meta name=\"driftmark-page\" content=\"")
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .expect("a page marker")
            .clone_into(&mut server.marker);
        server
    }

    /// A request as the server's own page sends it, with the token cookie
    /// and the page's marker; the status and the body.
    fn send(&self, method: &str, path: &str, body: Option<&Value>) -> (u16, Vec<u8>) {
        let host = format!("localhost:{}", self.addr.port());
        let origin = format!("http://{host}");
        let text = body.map(Value::to_string).unwrap_or_default();
        let mut head = format!(
            "{method} {path} HTTP/1.1\r\nConnection: close\r\nHost: {host}\r\nContent-Length: {}\r\nCookie: driftmark-{}={}\r\n",
            text.len(),
            self.addr.port(),
            self.token
        );
        if body.is_some() {
            let _ = write!(
                head,
                "Content-Type: application/json\r\nOrigin: {origin}\r\nSec-Fetch-Site: same-origin\r\nDriftmark-Page: {}\r\nUser-Agent: {BROWSER}\r\n",
                self.marker
            );
        }
        head.push_str("\r\n");
        let mut stream = TcpStream::connect(self.addr).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(60)))
            .expect("a timeout");
        stream.write_all(head.as_bytes()).expect("send");
        stream.write_all(text.as_bytes()).expect("send the body");
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).expect("read");
        let end = raw
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .expect("a head");
        let head = String::from_utf8_lossy(&raw[..end]).to_ascii_lowercase();
        let status = head[9..12].parse().expect("a status");
        let mut rest = raw[end + 4..].to_vec();
        if head.contains("transfer-encoding: chunked") {
            let mut out = Vec::new();
            let mut at = rest.as_slice();
            loop {
                let line = at.windows(2).position(|w| w == b"\r\n").expect("a size");
                let size =
                    usize::from_str_radix(std::str::from_utf8(&at[..line]).expect("hex"), 16)
                        .expect("a size");
                at = &at[line + 2..];
                if size == 0 {
                    break;
                }
                out.extend_from_slice(&at[..size]);
                at = &at[size + 2..];
            }
            rest = out;
        }
        (status, rest)
    }

    fn get(&self, path: &str) -> Value {
        let (status, body) = self.send("GET", path, None);
        assert_eq!(status, 200, "{path}");
        serde_json::from_slice(&body).expect("JSON")
    }

    fn page(&self, path: &str) -> String {
        let (status, body) = self.send("GET", path, None);
        assert_eq!(status, 200, "{path}");
        String::from_utf8(body).expect("UTF-8")
    }

    fn post(&self, path: &str, body: &Value) -> Value {
        let (status, answer) = self.send("POST", path, Some(body));
        let value: Value = serde_json::from_slice(&answer).unwrap_or(Value::Null);
        assert!(status < 300, "{path}: {status} {value}");
        value
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ---- the agent: driftmark mcp ------------------------------------------------

/// A tool's answer.
struct Answer {
    is_error: bool,
    text: String,
}

/// A running `driftmark mcp` with a client attached.
struct Client {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<String>,
    next: u64,
}

impl Client {
    /// Starts `driftmark mcp` in `dir` for a client named `name`.
    fn start(dir: &Path, name: &str) -> Client {
        let mut child = Command::new(PROGRAM)
            .arg("mcp")
            .current_dir(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("start driftmark mcp");
        let stdout = child.stdout.take().expect("standard output");
        let (send, lines) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { return };
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
            next: 0,
        };
        let opened = client.request(
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": name, "version": "1"}
            }),
        );
        assert_eq!(opened["result"]["serverInfo"]["name"], "driftmark");
        client.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        client
    }

    fn send(&mut self, message: &Value) {
        let stdin = self.stdin.as_mut().expect("open");
        writeln!(stdin, "{message}").expect("write");
        stdin.flush().expect("flush");
    }

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
            let message: Value = serde_json::from_str(&line).expect("JSON");
            if message["id"] == id {
                return message;
            }
        }
    }

    fn call(&mut self, name: &str, arguments: Value) -> Answer {
        let limit = arguments["max_chars"]
            .as_u64()
            .map_or(16_000, |limit| usize::try_from(limit).expect("a size"));
        let mut params = json!({"name": name});
        params["arguments"] = arguments;
        let message = self.request("tools/call", params);
        let result = &message["result"];
        assert!(result.is_object(), "{name}: {message}");
        let text = result["content"][0]["text"]
            .as_str()
            .expect("text")
            .to_owned();
        let size = text.encode_utf16().count();
        assert!(size <= limit, "{name}: {size} characters, limit {limit}");
        Answer {
            is_error: result["isError"] == true,
            text,
        }
    }

    fn json(&mut self, name: &str, arguments: Value) -> Value {
        let answer = self.call(name, arguments);
        assert!(!answer.is_error, "{name}: {}", answer.text);
        serde_json::from_str(&answer.text).expect("JSON")
    }

    fn refused(&mut self, name: &str, arguments: Value) -> String {
        let answer = self.call(name, arguments);
        assert!(answer.is_error, "{name} answered: {}", answer.text);
        answer.text
    }

    fn finish(mut self) {
        drop(self.stdin.take());
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(status) = self.child.try_wait().expect("wait") {
                assert_eq!(status.code(), Some(0));
                return;
            }
            assert!(std::time::Instant::now() < deadline, "did not stop");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ---- the repos -----------------------------------------------------------------

/// A repo whose `main` has `docs/cache.md` and `docs/guide.md`, and whose
/// `feature` branch changes two paragraphs of the first. `HEAD` stays on
/// `main`; an agent works on `feature` in a linked worktree.
fn repo() -> (TestRepo, PathBuf) {
    let repo = TestRepo::new("mcp-notes");
    repo.write("docs/cache.md", CACHE);
    repo.write("docs/guide.md", GUIDE);
    let base = repo.commit("first", T0);
    repo.git(&["update-ref", "refs/remotes/origin/main", &base]);
    repo.git(&["checkout", "-q", "-b", "feature"]);
    repo.write("docs/cache.md", CACHE_FEATURE);
    repo.commit("feature", T0 + DAY);
    repo.git(&["checkout", "-q", "main"]);
    let worktree = repo.dir.with_file_name("agent");
    repo.git(&[
        "worktree",
        "add",
        "-q",
        worktree.to_str().expect("text"),
        "feature",
    ]);
    (repo, worktree)
}

fn notes_file(repo: &TestRepo) -> PathBuf {
    repo.dir.join(".git").join("driftmark").join("notes.db")
}

/// The block index of the change card on the new side of `range` whose text
/// holds `words`.
fn card_block(server: &Server, range: &str, path: &str, words: &str) -> u64 {
    let changes = server.get(&format!("/api/changes/{range}/-/{path}"));
    changes["documents"][0]["changes"]
        .as_array()
        .expect("changes")
        .iter()
        .find(|change| {
            change["new"]["block"]["text"]
                .as_str()
                .is_some_and(|text| text.contains(words))
        })
        .and_then(|change| change["new"]["at"].as_u64())
        .expect("a card with the words")
}

/// The reader's notes for the round: two on the branch's change page, one on
/// a reading page.
fn reader_notes(server: &Server) {
    let range = "main...feature";
    let decision = card_block(server, range, "docs/cache.md", "two hours");
    let limits = card_block(server, range, "docs/cache.md", "128 MB");
    server.post(
        "/api/notes",
        &json!({
            "path": "docs/cache.md", "range": range, "side": "new", "block": decision,
            "text": "Keep each page for two hours", "hint": 0,
            "stance": "disagree", "ask": "change", "blocking": true,
            "body": "Phones keep pages for days; two hours is far too long for a cache.",
        }),
    );
    server.post(
        "/api/notes",
        &json!({
            "path": "docs/cache.md", "range": range, "side": "new", "block": limits,
            "stance": "question", "ask": "explain",
            "body": "Why double the limit?",
        }),
    );
    let start = GUIDE.find("Run the installer").expect("the words");
    server.post(
        "/api/notes",
        &json!({
            "path": "docs/guide.md", "revision": "main", "start": start,
            "end": start + "Run the installer once on each server.".len(),
            "stance": "suggestion", "ask": "change",
            "body": "Say which installer.",
        }),
    );
}

// ---- one whole round -------------------------------------------------------------

#[test]
fn a_whole_round_runs_between_the_reader_and_the_agent() {
    let (repo, worktree) = repo();
    let server = Server::start(&repo.dir);
    // 1. The reader writes notes through the HTTP routes.
    reader_notes(&server);
    // 2. The agent, in its own worktree, lists and reads them, replies and
    //    proposes through MCP.
    let mut agent = Client::start(&worktree, "claude-code");
    agent_reads(&mut agent, &server, &repo);
    agent_answers(&mut agent);
    // 3. The reader sees the answers and decides through HTTP.
    reader_decides(&server);
    // 4. The agent applies the decided note, commits, and records the commit.
    let commit = agent_applies(&mut agent, &repo, &worktree);
    // 5. The change page marks the change n1 caused, and n1 lists it.
    page_shows(&server, &commit);
    agent.finish();
}

/// Step 2: the agent lists the notes, reads one with its place and
/// section, and gets the review as the prompt "Copy as prompt" gives.
fn agent_reads(agent: &mut Client, server: &Server, repo: &TestRepo) {
    let listed = agent.json("notes_list", json!({}));
    assert_eq!(listed["total"], 3);
    let ids: Vec<u64> = listed["notes"]
        .as_array()
        .expect("in the answer")
        .iter()
        .map(|note| note["id"].as_u64().expect("in the answer"))
        .collect();
    assert_eq!(ids, [1, 2, 3]);
    assert_eq!(listed["notes"][0]["quote"], "Keep each page for two hours");
    assert_eq!(
        listed["notes"][0]["under"],
        json!(["Cache pages", "Decision"])
    );
    assert_eq!(listed["notes"][0]["status"], "open");
    assert_eq!(
        listed["reviews"],
        json!([
            {"id": 1, "kind": "range", "scope": "main...feature"},
            {"id": 2, "kind": "document", "scope": "docs/guide.md"}
        ])
    );
    let blocking = agent.json("notes_list", json!({"blocking": true}));
    assert_eq!(blocking["total"], 1);
    let on_guide = agent.json("notes_list", json!({"path": "docs/guide.md"}));
    assert_eq!(on_guide["notes"][0]["id"], 3);
    let questions = agent.json(
        "notes_list",
        json!({"stance": ["question"], "review": "r1"}),
    );
    assert_eq!(questions["notes"][0]["id"], 2);

    let feature_tip = repo.git(&["rev-parse", "feature"]);
    let first = agent.json("note_get", json!({"note": "n1"}));
    assert_eq!(
        first["note"]["body"],
        "Phones keep pages for days; two hours is far too long for a cache."
    );
    assert_eq!(
        first["note"]["target"]["quote"]["exact"],
        "Keep each page for two hours"
    );
    assert_eq!(
        first["made_on"],
        json!({"revision": feature_tip, "lines": [7, 7]})
    );
    assert_eq!(first["now"]["revision"], feature_tip.as_str());
    assert_eq!(first["now"]["state"], "exact");
    assert_eq!(first["now"]["lines"], json!([7, 7]));
    assert_eq!(first["section"]["path"], json!(["Cache pages", "Decision"]));
    assert_eq!(first["section"]["anchor"], "decision");
    assert_eq!(
        first["section"]["text"],
        "## Decision\n\nKeep each page for two hours, then drop it from the cache.\n\nA page with open notes is never cached."
    );
    assert_eq!(first["review"]["scope"], "main...feature");

    let packet = agent.json("review_get", json!({"review": "r1"}));
    assert_eq!(packet["notes"], json!([1, 2]));
    let copied = server.post("/api/reviews/1/prompt", &json!({"round": 1}));
    assert_eq!(packet["text"], copied["text"]);
    assert!(
        packet["text"]
            .as_str()
            .expect("in the answer")
            .starts_with("Review r1 · main...feature · 2 notes · at ")
    );
}

/// Step 2, continued: the agent replies to two notes and proposes a
/// decision on one.
fn agent_answers(agent: &mut Client) {
    let replied = agent.json(
        "note_reply",
        json!({"note": 1, "stance": "counter", "body": "Two hours matches how long a reader keeps one tab open."}),
    );
    assert_eq!(replied["note"]["status"], "answered");
    assert_eq!(replied["reply"]["role"], "agent");
    assert_eq!(replied["reply"]["by"], "claude-code");
    assert_eq!(replied["reply"]["stance"], "counter");
    let proposed = agent.json(
        "note_propose_decision",
        json!({"note": "n1", "outcome": "modify", "summary": "Keep each page for ten minutes."}),
    );
    assert_eq!(proposed["note"]["status"], "answered");
    assert_eq!(proposed["proposal"]["outcome"], "modify");
    assert_eq!(proposed["proposal"]["by"], "claude-code");
    agent.json(
        "note_reply",
        json!({"note": "n2", "stance": "question", "body": "The pages grew; should the reason go in the document?"}),
    );
    // Each agent action is recorded as the connector's, with the client's
    // name and version, and carries no warning.
    let read = agent.json("note_get", json!({"note": "n1"}));
    let history: Vec<(String, String, String, String, bool)> = read["note"]["history"]
        .as_array()
        .expect("a history")
        .iter()
        .map(|event| {
            (
                event["action"].as_str().unwrap_or_default().to_owned(),
                event["role"].as_str().unwrap_or_default().to_owned(),
                event["source"].as_str().unwrap_or_default().to_owned(),
                event["user_agent"].as_str().unwrap_or_default().to_owned(),
                event.get("warning").is_some(),
            )
        })
        .collect();
    let row = |action: &str, role: &str, source: &str, agent: &str| {
        (
            action.to_owned(),
            role.to_owned(),
            source.to_owned(),
            agent.to_owned(),
            false,
        )
    };
    assert_eq!(
        history,
        [
            row("made", "reader", "page", BROWSER),
            row("send", "reader", "page", BROWSER),
            row("reply", "agent", "mcp", "claude-code 1"),
            row("propose", "agent", "mcp", "claude-code 1"),
        ]
    );
    assert_eq!(read["note"]["history"][2]["by"], "claude-code");
    let listed = agent.json("notes_list", json!({}));
    assert_eq!(listed["notes"][0]["warnings"], json!([]));
}

/// Step 3: the reader reads the answers on the page and decides.
fn reader_decides(server: &Server) {
    let seen = server.get("/api/notes/1");
    assert_eq!(seen["note"]["status"], "answered");
    assert_eq!(seen["note"]["thread"][0]["by"], "claude-code");
    assert_eq!(
        seen["note"]["proposal"]["summary"],
        "Keep each page for ten minutes."
    );
    server.post(
        "/api/notes/1/decide",
        &json!({"outcome": "modify", "summary": "Keep each page for ten minutes, then drop it from the cache."}),
    );
    server.post(
        "/api/notes/2/decide",
        &json!({"outcome": "keep", "summary": "The reason is in the merge request."}),
    );
    server.post("/api/notes/3/withdraw", &json!({}));
}

/// Step 4: the agent reads round 2, edits and commits in its worktree, and
/// records the commit by its short id. Returns the commit.
fn agent_applies(agent: &mut Client, repo: &TestRepo, worktree: &Path) -> String {
    let apply = agent.json("review_get", json!({"review": 1, "round": 2}));
    assert_eq!(apply["notes"], json!([1]));
    let text = apply["text"].as_str().expect("in the answer");
    assert!(text.contains("## 1 · n1 · decided: modify"), "{text}");
    assert!(
        text.contains("Decision: Keep each page for ten minutes, then drop it from the cache."),
        "{text}"
    );
    std::fs::write(
        worktree.join("docs/cache.md"),
        CACHE_FEATURE.replace("for two hours", "for ten minutes"),
    )
    .expect("the agent edits the document");
    let wt = worktree.to_str().expect("in the answer");
    repo.git(&["-C", wt, "add", "-A"]);
    repo.git_at(T0 + 2 * DAY, &["-C", wt, "commit", "-q", "-m", "apply n1"]);
    let commit = repo.git(&["-C", wt, "rev-parse", "HEAD"]);
    let marked = agent.json(
        "note_mark_applied",
        json!({"note": "n1", "commits": [&commit[..7]]}),
    );
    assert_eq!(marked["note"]["status"], "applied");
    assert_eq!(marked["applied"][0]["commit"], commit.as_str());
    assert_eq!(marked["applied"][0]["by"], "claude-code");
    commit
}

/// Step 5: the range's notes name the change the commit made, the page's
/// cards carry its key, and a range without the commit names none.
fn page_shows(server: &Server, commit: &str) {
    let range = server.get("/api/notes?range=main...feature");
    let notes = range["notes"].as_array().expect("in the answer");
    let one = notes
        .iter()
        .find(|note| note["id"] == 1)
        .expect("in the answer");
    let caused = one["caused"].as_array().expect("caused");
    assert_eq!(caused.len(), 1, "{caused:?}");
    assert_eq!(caused[0]["path"], "docs/cache.md");
    assert_eq!(caused[0]["commit"], commit);
    let two = notes
        .iter()
        .find(|note| note["id"] == 2)
        .expect("in the answer");
    assert_eq!(two["caused"], json!([]));
    let changes = server.get("/api/changes/main...feature/-/docs/cache.md");
    let index = usize::try_from(caused[0]["change"].as_u64().expect("in the answer"))
        .expect("in the answer");
    let change = &changes["documents"][0]["changes"][index];
    assert_eq!(change["kind"], "edited");
    assert!(
        change["new"]["block"]["text"]
            .as_str()
            .expect("in the answer")
            .contains("ten minutes"),
        "{change}"
    );
    let page = server.page("/changes/main...feature");
    let key = caused[0]["key"].as_str().expect("in the answer");
    let keys = page
        .split("data-keys=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("data-keys");
    assert_eq!(keys.split(' ').nth(index), Some(key), "{keys}");
    // The page loads the notes script, which draws the mark.
    assert!(page.contains("data-notes=\"2\" data-range=\"main...feature\""));

    // A range that ends before the commit holds no change of it.
    let before = server.get("/api/notes?range=main...feature~1");
    let one = before["notes"]
        .as_array()
        .expect("in the answer")
        .iter()
        .find(|note| note["id"] == 1)
        .expect("in the answer")
        .clone();
    assert_eq!(one["caused"], json!([]));
    // A document's notes carry no changes.
    let reading = server.get("/api/notes?path=docs/cache.md&rev=feature");
    assert!(reading["notes"][0].get("caused").is_none());
}

// ---- what an agent may not do ----------------------------------------------------

#[test]
fn no_tool_or_argument_lets_an_agent_decide_withdraw_or_edit() {
    let (repo, worktree) = repo();
    let server = Server::start(&repo.dir);
    reader_notes(&server);
    let mut agent = Client::start(&worktree, "claude-code");

    // Deciding, withdrawing and editing have no tool, and the error says why.
    for name in [
        "note_decide",
        "note_withdraw",
        "note_edit",
        "notes_update",
        "review_send",
    ] {
        let error = agent.request(
            "tools/call",
            json!({"name": name, "arguments": {"note": 1}}),
        );
        assert_eq!(error["error"]["code"], -32602, "{name}");
        let message = error["error"]["message"].as_str().unwrap();
        assert!(
            message.starts_with(&format!("driftmark has no tool `{name}`. An agent lists")),
            "{message}"
        );
        assert!(
            message.contains("Deciding, editing and withdrawing a note are the reader's"),
            "{message}"
        );
    }

    // No tool takes the reader's fields or a status.
    assert_eq!(
        agent.refused(
            "note_reply",
            json!({"note": 1, "stance": "agree", "body": "Yes.", "status": "decided"})
        ),
        "`note_reply` takes no argument `status`; its arguments are `note`, `body`, `stance`, `max_chars`"
    );
    assert!(
        agent
            .refused(
                "note_propose_decision",
                json!({"note": 1, "outcome": "change", "summary": "x", "decision": "change"})
            )
            .starts_with("`note_propose_decision` takes no argument `decision`")
    );

    // A proposal never decides.
    assert_eq!(
        agent.refused(
            "note_propose_decision",
            json!({"note": 1, "outcome": "decided", "summary": "Ten minutes."})
        ),
        "`outcome` must be change, modify or keep; a proposal never decides, the reader does"
    );
    let proposed = agent.json(
        "note_propose_decision",
        json!({"note": 1, "outcome": "change", "summary": "Ten minutes."}),
    );
    assert_eq!(proposed["note"]["status"], "answered");
    assert_eq!(proposed["note"]["decision"], Value::Null);
    let after = server.get("/api/notes/1");
    assert_eq!(after["note"]["status"], "answered");
    assert_eq!(after["note"]["decision"], Value::Null);
    // What the reader wrote is as it was.
    assert_eq!(
        after["note"]["body"],
        "Phones keep pages for days; two hours is far too long for a cache."
    );
    assert_eq!(after["note"]["stance"], "disagree");
    assert_eq!(after["note"]["blocking"], true);
    agent.finish();
}

#[test]
fn the_loop_and_the_commit_checks_refuse_with_their_reasons() {
    let (repo, worktree) = repo();
    let server = Server::start(&repo.dir);
    reader_notes(&server);
    let mut agent = Client::start(&worktree, "claude-code");
    agent.json(
        "note_reply",
        json!({"note": 1, "stance": "agree", "body": "Yes."}),
    );

    // No commit is recorded before the reader decides.
    assert_eq!(
        agent.refused(
            "note_mark_applied",
            json!({"note": 1, "commits": ["abc1234"]})
        ),
        "n1 is answered, not decided, so no commit can be recorded on it yet; only the reader decides, on the note's page. Suggest a decision with note_propose_decision and wait for the reader"
    );

    // Once decided, no new proposal; a reply keeps it decided.
    server.post(
        "/api/notes/1/decide",
        &json!({"outcome": "change", "summary": "Ten minutes."}),
    );
    assert_eq!(
        agent.refused(
            "note_propose_decision",
            json!({"note": 1, "outcome": "keep", "summary": "Keep it."})
        ),
        "n1 is decided already, so it takes no new proposal; apply the decision (review_get round 2), commit, then record the commits with note_mark_applied"
    );
    let replied = agent.json(
        "note_reply",
        json!({"note": 1, "stance": "agree", "body": "Applying it."}),
    );
    assert_eq!(replied["note"]["status"], "decided");

    // Each commit must exist and change the note's document.
    assert_eq!(
        agent.refused("note_mark_applied", json!({"note": 1, "commits": ["HEAD"]})),
        "`HEAD` is not a commit id; give 7 to 64 hex digits, as `git log` prints them"
    );
    assert_eq!(
        agent.refused(
            "note_mark_applied",
            json!({"note": 1, "commits": ["1234567"]})
        ),
        "`1234567` does not name a commit in this repository"
    );
    let wt = worktree.to_str().unwrap();
    std::fs::write(
        worktree.join("docs/guide.md"),
        GUIDE.replace("once", "one time"),
    )
    .unwrap();
    repo.git(&["-C", wt, "add", "-A"]);
    repo.git_at(
        T0 + 2 * DAY,
        &["-C", wt, "commit", "-q", "-m", "guide only"],
    );
    let elsewhere = repo.git(&["-C", wt, "rev-parse", "HEAD"]);
    let parent = repo.git(&["-C", wt, "rev-parse", "--short=7", "HEAD~1"]);
    assert_eq!(
        agent.refused(
            "note_mark_applied",
            json!({"note": 1, "commits": [&elsewhere]})
        ),
        format!(
            "`{elsewhere}` did not change `docs/cache.md`, the document of the note: the file is the same as in its parent {parent}. Record only the commits that edited the document as the decision says"
        )
    );
    assert_eq!(server.get("/api/notes/1")["note"]["status"], "decided");
    // The first commit made the document, so it changed it; a commit named
    // twice is recorded once.
    let first = repo.git(&["rev-list", "--max-parents=0", "main"]);
    let applied = agent.json(
        "note_mark_applied",
        json!({"note": 1, "commits": [first.clone(), first]}),
    );
    assert_eq!(applied["applied"].as_array().unwrap().len(), 1);

    finished_notes_take_nothing(&mut agent, &server);
    agent.finish();
}

/// A note whose loop is over, applied, closed or withdrawn, takes nothing
/// more from an agent, and the reader's words stay as written.
fn finished_notes_take_nothing(agent: &mut Client, server: &Server) {
    assert_eq!(
        agent.refused(
            "note_reply",
            json!({"note": 1, "stance": "agree", "body": "Done."})
        ),
        "n1 is applied: its commits are recorded and its loop is over, so it cannot change"
    );
    server.post(
        "/api/notes/2/decide",
        &json!({"outcome": "keep", "summary": ""}),
    );
    assert_eq!(
        agent.refused(
            "note_reply",
            json!({"note": 2, "stance": "question", "body": "Sure?"})
        ),
        "n2 is closed: the reader decided to keep the text as it is, so nothing more happens to it"
    );
    server.post("/api/notes/3/withdraw", &json!({}));
    assert_eq!(
        agent.refused(
            "note_propose_decision",
            json!({"note": 3, "outcome": "change", "summary": "x"})
        ),
        "n3 is withdrawn: the reader took it back, so it takes no reply, proposal or commit"
    );
    assert_eq!(
        server.get("/api/notes/3")["note"]["body"],
        "Say which installer."
    );
    let every = agent.json("notes_list", json!({"status": "all"}));
    assert_eq!(every["total"], 3);
    assert_eq!(agent.json("notes_list", json!({}))["total"], 0);
}

#[test]
fn wrong_numbers_texts_and_values_are_refused() {
    let (repo, worktree) = repo();
    let server = Server::start(&repo.dir);
    reader_notes(&server);
    let mut agent = Client::start(&worktree, "claude-code");
    for (tool, arguments, said) in [
        ("note_get", json!({"note": 99}), "there is no note n99"),
        (
            "note_get",
            json!({"note": "first"}),
            "`note` must be a number such as n12 or 12",
        ),
        (
            "note_reply",
            json!({"note": 1, "stance": "agree", "body": "  "}),
            "`body` is empty; write it in plain words",
        ),
        (
            "note_reply",
            json!({"note": 1, "stance": "maybe", "body": "x"}),
            "`stance` must be agree, counter, question or proposal",
        ),
        (
            "note_reply",
            json!({"note": 1, "stance": "agree", "body": "x".repeat(20_001)}),
            "`body` is longer than 20000 bytes; say it in fewer words",
        ),
        (
            "note_propose_decision",
            json!({"note": 1, "outcome": "keep", "summary": ""}),
            "`summary` is empty; write it in plain words",
        ),
        (
            "note_mark_applied",
            json!({"note": 1, "commits": []}),
            "name at least one commit in `commits`",
        ),
        (
            "notes_list",
            json!({"status": ["open", "done"]}),
            "`status` has no value `done`; the values are open, sent, answered, discussing, decided, applied, closed and withdrawn, or \"all\"",
        ),
        (
            "notes_list",
            json!({"stance": "angry"}),
            "`stance` has no value `angry`; the values are agree, disagree, question and suggestion",
        ),
        (
            "notes_list",
            json!({"path": "/etc/hosts.md"}),
            "`/etc/hosts.md` is outside the repository; give a path from the repository root, such as docs/guide.md",
        ),
        (
            "review_get",
            json!({"review": "r9"}),
            "there is no review r9",
        ),
        (
            "review_get",
            json!({"review": 1, "round": 3}),
            "`round` must be a whole number from 1 to 2",
        ),
        (
            "review_get",
            json!({"review": 1, "offset": 100_000}),
            "`offset` 100000 is past the prompt's end, at byte ",
        ),
    ] {
        let refused = agent.refused(tool, arguments.clone());
        assert!(refused.starts_with(said), "{tool} {arguments}: {refused}");
    }
    agent.finish();
}

// ---- reading makes nothing -------------------------------------------------------

#[test]
fn reading_tools_never_make_the_notes_file() {
    let (repo, worktree) = repo();
    let mut agent = Client::start(&worktree, "claude-code");
    let empty = agent.json("notes_list", json!({}));
    assert_eq!(empty["total"], 0);
    assert_eq!(empty["notes"], json!([]));
    assert_eq!(
        empty["statuses"],
        json!(["open", "sent", "answered", "discussing", "decided"])
    );
    assert_eq!(
        agent.refused("note_get", json!({"note": 1})),
        "there is no note n1"
    );
    assert_eq!(
        agent.refused("review_get", json!({"review": 1})),
        "there is no review r1"
    );
    assert_eq!(
        agent.refused(
            "note_reply",
            json!({"note": 1, "stance": "agree", "body": "x"})
        ),
        "there is no note n1"
    );
    assert_eq!(
        agent.refused("notes_list", json!({"review": 4})),
        "there is no review r4"
    );
    agent.finish();
    assert!(!notes_file(&repo).exists());
    assert!(!repo.dir.join(".git").join("driftmark").exists());
}

// ---- size limits ----------------------------------------------------------------------

#[test]
fn long_answers_are_cut_and_read_on() {
    let (repo, worktree) = repo();
    let server = Server::start(&repo.dir);
    reader_notes(&server);
    // Many notes: the list is cut into pages.
    let start = GUIDE.find("Run").unwrap();
    for n in 0..40 {
        server.post(
            "/api/notes",
            &json!({
                "path": "docs/guide.md", "revision": "main", "start": start, "end": start + 3,
                "stance": "question", "ask": "explain", "body": format!("Question {n}: {}", "why ".repeat(60)),
            }),
        );
    }
    let mut agent = Client::start(&worktree, "claude-code");
    let first = agent.json("notes_list", json!({"max_chars": 4000}));
    assert_eq!(first["total"], 43);
    let shown = first["page"]["shown"].as_u64().unwrap();
    assert!(shown > 0 && shown < 43);
    assert_eq!(
        first["cut"]["next"],
        json!({"max_chars": 4000, "offset": shown})
    );
    let second = agent.json("notes_list", first["cut"]["next"].clone());
    assert_eq!(second["notes"][0]["id"], shown + 1);
    // A long body is shortened in the list, and whole in note_get.
    assert!(first["notes"][3]["body"].as_str().unwrap().ends_with('…'));
    assert_eq!(
        first["notes"][3]["body"].as_str().unwrap().chars().count(),
        240
    );

    // A long thread loses its oldest replies first.
    for n in 0..12 {
        agent.json(
            "note_reply",
            json!({"note": 1, "stance": "counter", "body": format!("Reply {n}: {}", "because ".repeat(150))}),
        );
    }
    let cut = agent.json("note_get", json!({"note": 1}));
    let thread = cut["note"]["thread"].as_array().unwrap();
    assert!(thread.len() < 12);
    assert!(
        thread.last().unwrap()["body"]
            .as_str()
            .unwrap()
            .starts_with("Reply 11:")
    );
    let reason = cut["cut"]["reason"].as_str().unwrap();
    assert!(
        reason.starts_with(&format!(
            "the {} oldest of the 12 replies are left out",
            12 - thread.len()
        )),
        "{reason}"
    );
    let whole = agent.json("note_get", json!({"note": 1, "max_chars": 40000}));
    assert_eq!(whole["note"]["thread"].as_array().unwrap().len(), 12);
    assert!(whole.get("cut").is_none());

    // A long prompt is read in parts that join into the whole.
    let full = server.post("/api/reviews/2/prompt", &json!({"round": 1}))["text"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(full.len() > 2_000);
    let mut joined = String::new();
    let mut arguments = json!({"review": "r2", "max_chars": 2000});
    loop {
        let part = agent.json("review_get", arguments.clone());
        joined.push_str(part["text"].as_str().unwrap());
        match part.get("cut") {
            Some(cut) => arguments = cut["next"].clone(),
            None => break,
        }
    }
    assert_eq!(joined, full);
    agent.finish();
}

// ---- the documented examples --------------------------------------------------------

/// Sets every time in `got` to the time at the same place in `expected`:
/// the times a test run writes are its own.
fn same_times(got: &mut Value, expected: &Value) {
    match (got, expected) {
        (Value::Object(got), Value::Object(expected)) => {
            for (key, value) in got.iter_mut() {
                let Some(wanted) = expected.get(key) else {
                    continue;
                };
                if ["at", "created_at", "updated_at"].contains(&key.as_str())
                    && value.is_number()
                    && wanted.is_number()
                {
                    *value = wanted.clone();
                } else {
                    same_times(value, wanted);
                }
            }
        }
        (Value::Array(got), Value::Array(expected)) => {
            for (value, wanted) in got.iter_mut().zip(expected) {
                same_times(value, wanted);
            }
        }
        _ => {}
    }
}

#[test]
fn the_documented_note_examples_hold() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/features/commands/mcp.md");
    let document = std::fs::read_to_string(path)
        .expect("the document")
        .replace("\r\n", "\n");
    let examples: Vec<_> = examples(&document)
        .into_iter()
        .filter(|(tool, _, _)| NOTE_TOOLS.contains(&tool.as_str()))
        .collect();
    assert!(examples.len() >= 6, "{} examples", examples.len());
    // The notes of the examples, as the document tells them: two on the
    // branch's change page, one on the guide in the working tree. The reader
    // decided the third, and the agent applied it in one more commit on
    // `feature`.
    let repo = documented_repo();
    let server = Server::start(&repo.dir);
    let range = "main...feature";
    let record = "docs/decisions/0002-cache-rendered-pages.md";
    let block = card_block(&server, range, record, "90 minutes");
    server.post(
        "/api/notes",
        &json!({
            "path": record, "range": range, "side": "new",
            "block": block, "text": "for 90 minutes", "hint": 0,
            "stance": "disagree", "ask": "change", "blocking": true,
            "body": "Phones keep pages longer than that; ten minutes is enough.",
        }),
    );
    let guide = std::fs::read_to_string(repo.dir.join("docs/guide.md")).expect("the guide");
    let start = guide
        .find("The default keeps 500 pages.")
        .expect("the words");
    server.post(
        "/api/notes",
        &json!({
            "path": "docs/guide.md", "revision": "worktree", "start": start,
            "end": start + "The default keeps 500 pages.".len(),
            "stance": "question", "ask": "explain", "body": "Is 500 enough for a phone?",
        }),
    );
    let block = card_block(&server, range, record, "open notes");
    server.post(
        "/api/notes",
        &json!({
            "path": record, "range": range, "side": "new", "block": block,
            "stance": "suggestion", "ask": "change",
            "body": "Say when such a page may be cached again.",
        }),
    );
    server.post(
        "/api/notes/3/decide",
        &json!({"outcome": "change", "summary": "Cache the page again once all its notes are closed."}),
    );
    let text = std::fs::read_to_string(repo.dir.join(record)).expect("the record");
    repo.write(
        record,
        text.replace(
            "Never cache a page with open notes.",
            "Never cache a page with open notes; cache it again once they are all closed.",
        ),
    );
    repo.git(&["add", record]);
    let applied = repo.commit_index("apply n3", T0 + 2 * DAY);
    assert!(
        document.contains(&format!("`{}`", &applied[..7])),
        "the document names the commit that applied note 3, {applied}"
    );
    let mut agent = Client::start(&repo.dir, "claude-code");
    let mut wrong = Vec::new();
    for (tool, arguments, expected) in examples {
        let answer = agent.call(&tool, arguments.clone());
        match expected {
            Expected::Answer(value) => {
                assert!(!answer.is_error, "{tool} {arguments}: {}", answer.text);
                let mut got: Value = serde_json::from_str(&answer.text).expect("JSON");
                same_times(&mut got, &value);
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
    agent.finish();
}
