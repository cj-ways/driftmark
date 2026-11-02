//! The status view of `driftmark serve`: `/lifecycle/<revision>` and
//! `/api/lifecycle/<revision>`, run as the real program on a free port.
//!
//! Each tree of `/fixtures/status/<behaviour>/` is committed to a git
//! repository, every file at the day its `touched.toml` gives, and the
//! server's answer must give the view of the tree's `expected.txt`, which
//! the core's own test (`crates/driftmark-core/tests/status_fixtures.rs`)
//! checks too.

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use support::{DAY, T0, TestRepo};

const PROGRAM: &str = env!("CARGO_BIN_EXE_driftmark");

/// A running `driftmark serve`, stopped when dropped.
struct Server {
    _child: support::Running,
    addr: SocketAddr,
}

impl Server {
    fn start(repo: &Path) -> Server {
        let mut child = support::Running(
            Command::new(PROGRAM)
                .arg("serve")
                .arg("--repo")
                .arg(repo)
                .args(["--addr", "127.0.0.1:0"])
                .stdout(Stdio::piped())
                .env_remove("DRIFTMARK_TOKEN")
                .stderr(Stdio::null())
                .spawn()
                .expect("start driftmark serve"),
        );
        let mut reader = BufReader::new(child.stdout.take().expect("standard output"));
        let mut port = None;
        loop {
            let mut line = String::new();
            assert!(
                reader.read_line(&mut line).expect("read") > 0,
                "stopped early"
            );
            if let Some(rest) = line.strip_prefix("driftmark serve: open http://127.0.0.1:") {
                port = rest.split('/').next().map(str::to_owned);
            }
            if line.contains("press Ctrl-C to stop") {
                break;
            }
        }
        std::thread::spawn(move || {
            let _ = std::io::copy(&mut reader, &mut std::io::sink());
        });
        let addr = format!("127.0.0.1:{}", port.expect("a port"));
        Server {
            _child: child,
            addr: addr.parse().expect("an address"),
        }
    }

    /// The status, the headers in lower case and the body of a GET.
    fn get(&self, target: &str) -> (u16, String, String) {
        let mut stream = TcpStream::connect(self.addr).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(60)))
            .expect("a read timeout");
        let head = format!(
            "GET {target} HTTP/1.1\r\nHost: localhost:{}\r\nConnection: close\r\n\r\n",
            self.addr.port()
        );
        stream.write_all(head.as_bytes()).expect("send");
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).expect("read the answer");
        let end = raw
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .expect("an answer head");
        let head = String::from_utf8_lossy(&raw[..end]).to_ascii_lowercase();
        let status = head[9..12].parse().expect("a status");
        let mut body = raw[end + 4..].to_vec();
        if head.contains("transfer-encoding: chunked") {
            body = unchunk(&body);
        }
        (status, head, String::from_utf8(body).expect("UTF-8"))
    }

    /// The JSON of a request that must answer 200.
    fn json(&self, target: &str) -> Value {
        let (status, head, body) = self.get(target);
        assert_eq!(status, 200, "{target}: {body}");
        assert!(head.contains("content-type: application/json"), "{head}");
        serde_json::from_str(&body).expect("JSON")
    }

    /// Asks until `done` holds for the JSON, or fails after 30 seconds: the
    /// working tree's view follows the watcher, which looks once a second.
    fn json_until(&self, target: &str, done: impl Fn(&Value) -> bool) -> Value {
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            let answer = self.json(target);
            if done(&answer) {
                return answer;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "{target} never matched: {answer}"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// The HTML of a page that must answer 200.
    fn page(&self, target: &str) -> String {
        let (status, head, body) = self.get(target);
        assert_eq!(status, 200, "{target}: {body}");
        assert!(head.contains("content-type: text/html"), "{head}");
        body
    }
}

fn unchunk(mut raw: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    loop {
        let line_end = raw
            .windows(2)
            .position(|w| w == b"\r\n")
            .expect("a chunk size line");
        let size = std::str::from_utf8(&raw[..line_end])
            .ok()
            .and_then(|hex| usize::from_str_radix(hex, 16).ok())
            .expect("a chunk size");
        raw = &raw[line_end + 2..];
        if size == 0 {
            return body;
        }
        body.extend_from_slice(&raw[..size]);
        raw = &raw[size + 2..];
    }
}

/// The seconds since 1970 now.
fn now() -> i64 {
    let since = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock after 1970");
    i64::try_from(since.as_secs()).expect("a time that fits")
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/status")
        .join(name)
}

/// `touched.toml`: each file's whole days since its last commit, or `None`
/// for a file no commit holds. Its lines are `"<path>" = <days>` or
/// `"<path>" = "uncommitted"`.
fn touched(root: &Path) -> BTreeMap<String, Option<i64>> {
    let text = std::fs::read_to_string(root.join("touched.toml")).expect("touched.toml");
    text.lines()
        .filter_map(|line| line.strip_prefix('"'))
        .map(|line| {
            let (path, value) = line.split_once("\" = ").expect("a path and a value");
            (path.to_owned(), value.parse().ok())
        })
        .collect()
}

/// The tree of a fixture committed to a new repository: the rules file
/// first, then every file at the day it last changed, oldest first, as
/// `main`. Files no commit holds are written to the working tree only.
fn committed(name: &str) -> TestRepo {
    let root = fixture(name);
    let repo = TestRepo::new(&format!("status-{name}"));
    repo.write(
        "driftmark.toml",
        std::fs::read(root.join("driftmark.toml")).expect("the rules file"),
    );
    let days = touched(&root);
    let mut by_day: BTreeMap<i64, Vec<&String>> = BTreeMap::new();
    for (path, day) in &days {
        if let Some(day) = day {
            by_day.entry(-day).or_default().push(path);
        }
    }
    let now = now();
    for (day, paths) in by_day {
        for path in paths {
            repo.write(
                path,
                std::fs::read(root.join(path)).expect("a fixture file"),
            );
        }
        repo.commit("change", now + day * DAY - 3_600);
    }
    for (path, day) in &days {
        if day.is_none() {
            repo.write(
                path,
                std::fs::read(root.join(path)).expect("a fixture file"),
            );
        }
    }
    repo
}

/// One document of the JSON on one line, as `expected.txt` writes it.
fn line(document: &Value) -> String {
    let text = |value: &Value| value.as_str().map(str::to_owned);
    let by = match &document["superseded_by"] {
        Value::Null => "by -".to_owned(),
        by => {
            let named = text(&by["text"]).map_or_else(String::new, |text| format!(" \"{text}\""));
            let path = text(&by["path"]).map_or_else(String::new, |path| format!(" {path}"));
            format!(
                "by{named}: {}{path}",
                by["state"].as_str().expect("a state")
            )
        }
    };
    let age = match &document["last_change"] {
        Value::Null => "uncommitted".to_owned(),
        change => {
            let untouched = if document["untouched"] == true {
                ", untouched"
            } else {
                ""
            };
            format!("{} days{untouched}", change["days"])
        }
    };
    format!(
        "{} | {} | status {} | {by} | {age}",
        text(&document["path"]).expect("a path"),
        text(&document["lifecycle"]).expect("a lifecycle"),
        text(&document["status"]).unwrap_or_else(|| "-".to_owned())
    )
}

/// The counts of the JSON, as `expected.txt` writes them.
fn counts(answer: &Value) -> String {
    let mut out = format!("untouched-days {}\n\n", answer["untouched_days"]);
    let mut all = answer["totals"].clone();
    all["type"] = "all".into();
    for counts in answer["types"]
        .as_array()
        .expect("types")
        .iter()
        .chain([&all])
    {
        writeln!(
            out,
            "{}: {} documents, {} current, {} superseded, {} archived, {} none, {} untouched",
            counts["type"].as_str().expect("a type"),
            counts["documents"],
            counts["current"],
            counts["superseded"],
            counts["archived"],
            counts["none"],
            counts["untouched"]
        )
        .expect("a string");
    }
    out
}

/// The document lines of `expected.txt`, and which documents each one
/// replaced, from their indentation.
fn expected_documents(name: &str) -> (String, BTreeSet<String>, BTreeMap<String, Vec<String>>) {
    let text = std::fs::read_to_string(fixture(name).join("expected.txt")).expect("expected.txt");
    let (head, rows) = text.split_once("\n## ").expect("a type section");
    let mut lines = BTreeSet::new();
    let mut replaces: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut stack: Vec<(usize, String)> = Vec::new();
    for row in rows.lines().filter(|row| row.contains(" | ")) {
        let depth = (row.len() - row.trim_start().len()) / 2;
        let row = row.trim_start();
        let path = row.split(" | ").next().expect("a path").to_owned();
        stack.truncate(depth);
        if let Some((_, newer)) = stack.last() {
            replaces
                .entry(newer.clone())
                .or_default()
                .push(path.clone());
        }
        stack.push((depth, path));
        lines.insert(row.to_owned());
    }
    (head.to_owned(), lines, replaces)
}

/// The server's view of `revision` must be the fixture's: every document's
/// line and folding, and, for the working tree, the counts too.
fn check(name: &str) {
    let repo = committed(name);
    let server = Server::start(&repo.dir);
    let (head, lines, replaces) = expected_documents(name);
    for revision in ["worktree", "main"] {
        let answer = server.json(&format!("/api/lifecycle/{revision}"));
        let documents = answer["documents"].as_array().expect("documents");
        let found: BTreeSet<String> = documents.iter().map(line).collect();
        let wanted: BTreeSet<String> = lines
            .iter()
            .filter(|line| revision == "worktree" || !line.ends_with("uncommitted"))
            .cloned()
            .collect();
        assert_eq!(found, wanted, "{name} at {revision}");
        for document in documents {
            let path = document["path"].as_str().expect("a path");
            let own: Vec<String> = document["replaces"]
                .as_array()
                .map(|older| {
                    older
                        .iter()
                        .map(|path| path.as_str().expect("a path").to_owned())
                        .collect()
                })
                .unwrap_or_default();
            assert_eq!(
                own,
                replaces.get(path).cloned().unwrap_or_default(),
                "{name}: what {path} replaced"
            );
        }
        if revision == "worktree" {
            assert_eq!(counts(&answer), head, "{name}: the counts");
        }
        assert_eq!(answer["schema_version"], 1);
        assert_eq!(answer["revision"], revision);
        assert_eq!(answer["skipped"], Value::Array(Vec::new()));
    }
    let page = server.page("/lifecycle/worktree");
    assert!(
        page.contains("<h1>Which documents are current</h1>"),
        "{page}"
    );
    assert!(page.contains("data-live=\"worktree\""));
    for line in &lines {
        let path = line.split(" | ").next().expect("a path");
        let file = path.rsplit('/').next().expect("a file name");
        assert!(page.contains(file), "{name}: the page lists {path}");
    }
}

#[test]
fn every_lifecycle() {
    check("lifecycles");
}

#[test]
fn a_superseded_chain() {
    check("superseded-chain");
}

#[test]
fn a_broken_superseded_by() {
    check("broken-superseded-by");
}

#[test]
fn the_untouched_rule() {
    check("untouched");
}

const RULES: &str = "\
[[type]]
name = \"decision\"
paths = [\"docs/decisions/*.md\"]
status = { front-matter = \"status\" }
current = [\"accepted\"]
superseded = [\"superseded\"]
archived = [\"rejected\"]
superseded-by = { front-matter = \"superseded-by\" }
";

fn record(status: &str, extra: &str, title: &str) -> String {
    format!("---\nstatus: {status}\n{extra}---\n\n# {title}\n\nWords.\n")
}

/// A repository on `main` with three decision records and a note, and
/// `origin/main` at `main`.
fn small_repo() -> TestRepo {
    let repo = TestRepo::new("status");
    repo.write("driftmark.toml", RULES);
    repo.write(
        "docs/decisions/0001-old.md",
        record(
            "superseded",
            "superseded-by: 0002-new.md\n",
            "Old & slow way",
        ),
    );
    repo.write(
        "docs/decisions/0002-new.md",
        record("accepted", "", "New way"),
    );
    repo.write(
        "docs/decisions/0003-no.md",
        record("rejected", "", "No way"),
    );
    repo.write("notes/a.md", "# A note\n");
    let base = repo.commit("first", T0);
    repo.git(&["update-ref", "refs/remotes/origin/main", &base]);
    repo
}

#[test]
fn the_page_counts_filters_and_folds() {
    let repo = small_repo();
    let server = Server::start(&repo.dir);
    let page = server.page("/lifecycle/main");
    for wanted in [
        "<a href=\"/api/lifecycle/main\">JSON</a>",
        "<th scope=\"row\">Decision</th>",
        "<td data-label=\"Current\"><a href=\"/lifecycle/main?type=decision&amp;lifecycle=current\">1</a></td>",
        "<td data-label=\"Superseded\"><a href=\"/lifecycle/main?type=decision&amp;lifecycle=superseded\">1</a></td>",
        "<th scope=\"row\">Every type</th>",
        "<summary>Replaced 1 document</summary>",
        "Old &amp; slow way",
        "<summary>Archived · 1</summary>",
        "<details class=\"lc-plain\">",
        "All of the 4 documents these filters list, folded ones included.",
        "<span class=\"lc-old\">untouched</span>",
        "<form class=\"lc-filters\" method=\"get\" action=\"/lifecycle/main\"",
    ] {
        assert!(page.contains(wanted), "missing {wanted}: {page}");
    }
    assert!(!page.contains("data-live"));
    assert!(
        page.contains("<span class=\"group-n\">3 documents</span>"),
        "{page}"
    );
    let superseded = server.page("/lifecycle/main?type=decision&lifecycle=superseded");
    assert!(
        superseded.contains("<span class=\"group-n\">1 of 3 documents</span>"),
        "{superseded}"
    );
    assert!(
        superseded.contains(
            "Superseded by <a href=\"/doc/main/-/docs/decisions/0002-new.md\">New way</a>"
        ),
        "{superseded}"
    );
    assert!(superseded.contains("<option value=\"superseded\" selected>"));
    assert!(superseded.contains("<option value=\"decision\" selected>"));
    assert!(!superseded.contains("No way"));
    let archived = server.page("/lifecycle/main?lifecycle=archived");
    assert!(
        archived.contains("<details class=\"lc-archive\" open>"),
        "{archived}"
    );
    let folder = server.page("/lifecycle/main?folder=notes%2F&untouched=yes");
    assert!(folder.contains("A note"));
    assert!(!folder.contains("New way"));
    assert!(folder.contains("value=\"notes/\""));
    assert!(folder.contains("name=\"untouched\" value=\"yes\" checked"));
    let none = server.page("/lifecycle/main?type=nothing");
    assert!(none.contains("No document matches these filters."));
    let json = server.json("/api/lifecycle/main?type=decision&lifecycle=superseded");
    assert_eq!(json["documents"].as_array().expect("documents").len(), 1);
    assert_eq!(json["filter"]["types"], serde_json::json!(["decision"]));
    assert_eq!(json["filter"]["lifecycle"], "superseded");
    assert_eq!(json["totals"]["documents"], 4);
    let old = &json["documents"][0];
    assert_eq!(old["superseded_by"]["state"], "found");
    assert_eq!(old["superseded_by"]["path"], "docs/decisions/0002-new.md");
    assert_eq!(old["last_change"]["committed_at"], T0);
}

#[test]
fn the_inbox_and_the_reading_pills_lead_here() {
    let repo = small_repo();
    let server = Server::start(&repo.dir);
    let inbox = server.page("/");
    assert!(
        inbox.contains("<a href=\"/lifecycle/origin/main\">Which documents are current on <code>origin/main</code> →</a>"),
        "{inbox}"
    );
    let reading = server.page("/doc/main/-/docs/decisions/0002-new.md");
    assert!(
        reading.contains(
            "<a href=\"/lifecycle/main?type=decision&amp;lifecycle=current\">Current</a>"
        ),
        "{reading}"
    );
    assert!(reading.contains("<a href=\"/lifecycle/main?type=decision\">Decision</a>"));
}

#[test]
fn the_working_tree_reads_edits_and_its_own_rules() {
    let repo = small_repo();
    let server = Server::start(&repo.dir);
    let committed = server.json("/api/lifecycle/worktree");
    let new = |answer: &Value| {
        answer["documents"]
            .as_array()
            .expect("documents")
            .iter()
            .find(|document| document["path"] == "docs/decisions/0002-new.md")
            .cloned()
            .expect("the new record")
    };
    assert_eq!(new(&committed)["last_change"]["committed_at"], T0);
    repo.write(
        "docs/decisions/0002-new.md",
        record("rejected", "", "New way, edited"),
    );
    let edited = server.json_until("/api/lifecycle/worktree", |answer| {
        new(answer)["last_change"] == Value::Null
    });
    assert_eq!(new(&edited)["lifecycle"], "archived");
    assert_eq!(new(&edited)["title"], "New way, edited");
    let at_main = server.json("/api/lifecycle/main");
    assert_eq!(new(&at_main)["lifecycle"], "current");
    // Rules changed on disk: every file is read with them, and an unchanged
    // file keeps its last commit.
    repo.write(
        "driftmark.toml",
        RULES
            .replace("current = [\"accepted\"]\n", "current = [\"rejected\"]\n")
            .replace("archived = [\"rejected\"]\n", ""),
    );
    let own_rules = server.json("/api/lifecycle/worktree");
    assert_eq!(new(&own_rules)["lifecycle"], "current");
    let no = own_rules["documents"]
        .as_array()
        .expect("documents")
        .iter()
        .find(|document| document["path"] == "docs/decisions/0003-no.md")
        .expect("the third record");
    assert_eq!(no["lifecycle"], "current");
    assert_eq!(no["last_change"]["committed_at"], T0);
}

#[test]
fn a_repository_without_commits_has_only_uncommitted_documents() {
    let repo = TestRepo::new("status-empty");
    repo.write("docs/a.md", "# A\n");
    let server = Server::start(&repo.dir);
    let answer = server.json("/api/lifecycle/worktree");
    assert_eq!(answer["documents"][0]["last_change"], Value::Null);
    assert_eq!(answer["commit"], "worktree");
}

#[test]
fn wrong_addresses_are_refused() {
    let repo = small_repo();
    let server = Server::start(&repo.dir);
    let (status, _, body) = server.get("/api/lifecycle/nope");
    assert_eq!(
        (status, body.as_str()),
        (404, "{\"error\":\"`nope` does not name a commit\"}")
    );
    for (query, message) in [
        (
            "lifecycle=draft",
            "`lifecycle` is \\\"draft\\\"; write current, superseded, archived or none",
        ),
        (
            "untouched=maybe",
            "`untouched` is \\\"maybe\\\"; write yes or no",
        ),
        ("offset=-1", "`offset` is \\\"-1\\\"; write a whole number"),
        ("folder=%FF", "a parameter of the address is not valid text"),
    ] {
        let (status, _, body) = server.get(&format!("/api/lifecycle/main?{query}"));
        assert_eq!(status, 400, "{query}");
        assert_eq!(body, format!("{{\"error\":\"{message}\"}}"), "{query}");
        let (status, _, page) = server.get(&format!("/lifecycle/main?{query}"));
        assert_eq!(status, 400, "{query}");
        assert!(
            page.contains("<title>Bad address · driftmark</title>"),
            "{page}"
        );
    }
    let (status, _, page) = server.get("/lifecycle/nope");
    assert_eq!(status, 404);
    assert!(page.contains("does not name a commit"));
    let (status, _, _) = server.get("/lifecycle/%FF");
    assert_eq!(status, 404);
    let (status, _, body) = server.get("/api/lifecycle/%FF");
    assert_eq!(status, 404, "{body}");
    let bare = support::TempDir::new("status-bare");
    let target = bare.path().join("bare.git");
    repo.git(&["clone", "-q", "--bare", ".", &target.to_string_lossy()]);
    let server = Server::start(&target);
    let (status, _, body) = server.get("/api/lifecycle/worktree");
    assert_eq!(
        (status, body.as_str()),
        (404, "{\"error\":\"the repo has no working tree\"}")
    );
    assert_eq!(server.json("/api/lifecycle/main")["totals"]["documents"], 4);
}

#[test]
fn a_long_list_comes_in_pages() {
    let repo = TestRepo::new("status-pages");
    for n in 0..1_005 {
        repo.write(&format!("docs/note-{n:04}.md"), format!("# Note {n}\n"));
    }
    repo.commit("many", T0);
    let server = Server::start(&repo.dir);
    let first = server.page("/lifecycle/main");
    assert!(
        first.contains("Showing 1000 of the 1005 documents"),
        "{first}"
    );
    assert!(first.contains("note-0999.md"));
    assert!(!first.contains("note-1000.md"));
    assert!(
        first.contains("<a href=\"/lifecycle/main?offset=1000\">Show the next documents →</a>")
    );
    let next = server.page("/lifecycle/main?offset=1000");
    assert!(next.contains("note-1004.md"));
    assert!(!next.contains("note-0999.md"));
    assert!(next.contains("<a href=\"/lifecycle/main\">← Back to the first documents</a>"));
    assert!(!next.contains("Show the next documents"));
}
