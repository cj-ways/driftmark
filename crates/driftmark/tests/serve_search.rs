//! `GET /api/search` of `driftmark serve`, run as the real program on a
//! free port against small repos built for each test.

mod support;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use support::{DAY, T0, TestRepo};

const PROGRAM: &str = env!("CARGO_BIN_EXE_driftmark");

const RULES: &str = "\
[[type]]
name = \"decision\"
paths = [\"docs/decisions/*.md\"]
";

/// The decision record of the search example in
/// `docs/features/commands/serve.md`.
const DECISION: &str = "---\nstatus: accepted\n---\n\n# Cache pages\n\n> Short version: pages are kept in a cache.\n\n## Decision\n\nKeep each page for one hour.\n";

/// A repo on `main` with three documents and a rules file.
fn repo() -> TestRepo {
    let repo = TestRepo::new("search");
    repo.write("driftmark.toml", RULES);
    repo.write("docs/decisions/0001-cache-pages.md", DECISION);
    repo.write(
        "docs/guide.md",
        "# Guide\n\nThe cache is cleared on deploy.\n",
    );
    repo.write("README.md", "# Readme\n\nHow to read these docs.\n");
    repo.commit("first", T0);
    repo
}

/// A running `driftmark serve`, stopped when dropped.
struct Server {
    child: support::Running,
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
                // The address ends in `/?token=`; a search only reads, so it
                // needs no token on this machine.
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
            child,
            addr: addr.parse().expect("an address"),
        }
    }

    /// The status and the JSON body of a GET request.
    fn get(&self, target: &str) -> (u16, Value) {
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
        assert!(head.contains("content-type: application/json"), "{head}");
        let mut body = raw[end + 4..].to_vec();
        if head.contains("transfer-encoding: chunked") {
            body = unchunk(&body);
        }
        (status, serde_json::from_slice(&body).expect("JSON"))
    }

    /// The JSON of a request that must answer 200.
    fn json(&self, target: &str) -> Value {
        let (status, body) = self.get(target);
        assert_eq!(status, 200, "{target}: {body}");
        body
    }

    /// Searches until `done` holds for the answer, or fails after 30 seconds.
    fn search_until(&self, target: &str, done: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let answer = self.json(target);
            if done(&answer) {
                return answer;
            }
            assert!(
                Instant::now() < deadline,
                "{target} never matched: {answer}"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
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

fn paths(answer: &Value) -> Vec<String> {
    answer["results"]
        .as_array()
        .expect("results")
        .iter()
        .map(|result| result["path"].as_str().expect("a path").to_owned())
        .collect()
}

#[test]
fn a_revision_is_searched_as_the_guide_shows() {
    let repo = repo();
    let commit = repo.git(&["rev-parse", "HEAD"]);
    let server = Server::start(&repo.dir);
    let answer = server.json("/api/search?q=cache+%22one+hour%22&rev=main");
    assert_eq!(
        answer,
        json!({
            "schema_version": 1,
            "query": "cache \"one hour\"",
            "revision": "main",
            "commit": commit,
            "documents": 3,
            "total": 1,
            "results": [{
                "path": "docs/decisions/0001-cache-pages.md",
                "url": "/doc/main/-/docs/decisions/0001-cache-pages.md#decision",
                "title": "Cache pages",
                "type": "decision",
                "block": 4,
                "headings": ["Cache pages", "Decision"],
                "anchor": "decision",
                "snippet": [
                    {"text": "Keep each page for ", "hit": false},
                    {"text": "one hour", "hit": true},
                    {"text": ".", "hit": false}
                ]
            }],
            "skipped": []
        })
    );
    // The revision's index is kept by its tree, which `HEAD` shares.
    let again = server.json("/api/search?q=cache&rev=HEAD");
    assert_eq!(
        paths(&again),
        ["docs/decisions/0001-cache-pages.md", "docs/guide.md"]
    );
    let cache = &server.json("/api/status")["cache"];
    assert_eq!(cache["kinds"]["search"]["entries"], 1, "{cache}");
    assert_eq!(cache["kinds"]["search"]["misses"], 1, "{cache}");
    assert_eq!(cache["kinds"]["search"]["hits"], 1, "{cache}");
}

#[test]
fn parameters_filter_and_limit_the_results() {
    let repo = repo();
    let server = Server::start(&repo.dir);
    let all = server.json("/api/search?q=cache&rev=main");
    assert_eq!(all["total"], 2);
    let decisions = server.json("/api/search?q=cache&rev=main&type=decision");
    assert_eq!(paths(&decisions), ["docs/decisions/0001-cache-pages.md"]);
    let documents = server.json("/api/search?q=cache&rev=main&type=document&type=other");
    assert_eq!(paths(&documents), ["docs/guide.md"]);
    let folder = server.json("/api/search?q=cache&rev=main&path=docs%2Fg");
    assert_eq!(paths(&folder), ["docs/guide.md"]);
    let one = server.json("/api/search?q=cache&rev=main&limit=1");
    assert_eq!((one["total"].as_u64(), paths(&one).len()), (Some(2), 1));
    let none = server.json("/api/search?rev=main");
    assert_eq!(
        (none["total"].as_u64(), none["documents"].as_u64()),
        (Some(0), Some(3))
    );
}

#[test]
fn bad_requests_are_refused_with_a_reason() {
    let repo = repo();
    let server = Server::start(&repo.dir);
    let words = vec!["w"; 33].join("+");
    for (target, status, message) in [
        (
            "/api/search?q=a&limit=0".to_owned(),
            400,
            "`limit` must be a number from 1 to 100",
        ),
        (
            format!("/api/search?q={words}"),
            400,
            "the query holds 33 words; a query holds at most 32",
        ),
        (
            "/api/search?q=%FF".to_owned(),
            400,
            "a search parameter is not valid text",
        ),
        (
            "/api/search?q=a&rev=nope".to_owned(),
            404,
            "`nope` does not name a commit",
        ),
    ] {
        let (got, body) = server.get(&target);
        assert_eq!(got, status, "{target}: {body}");
        assert_eq!(body, json!({"error": message}), "{target}");
    }
}

#[test]
fn a_bare_repo_searches_head_and_has_no_working_tree() {
    let repo = repo();
    let bare = support::TempDir::new("search-bare");
    let dir = bare.path().join("bare.git");
    let status = Command::new("git")
        .args(["clone", "-q", "--bare"])
        .arg(&repo.dir)
        .arg(&dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .status()
        .expect("clone");
    assert!(status.success());
    let server = Server::start(&dir);
    let answer = server.json("/api/search?q=guide");
    assert_eq!(answer["revision"], "HEAD");
    assert_eq!(paths(&answer), ["docs/guide.md"]);
    let (status, body) = server.get("/api/search?q=guide&rev=worktree");
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["error"], "the repo has no working tree");
    assert_eq!(
        server.json("/api/status")["search"]["worktree"],
        Value::Null
    );
}

#[test]
fn the_working_tree_index_follows_the_files() {
    let repo = repo();
    repo.write(
        "docs/draft.md",
        "# Draft\n\nNot committed yet, about a lantern.\n",
    );
    let server = Server::start(&repo.dir);
    assert_eq!(
        server.json("/api/status")["search"]["worktree"],
        Value::Null
    );
    let first = server.json("/api/search?q=lantern");
    assert_eq!(first["revision"], "worktree");
    assert_eq!(first["commit"], "worktree");
    assert_eq!(first["documents"], 4);
    assert_eq!(paths(&first), ["docs/draft.md"]);
    assert_eq!(
        first["results"][0]["url"],
        "/doc/worktree/-/docs/draft.md#draft"
    );

    // An edited file.
    repo.write("README.md", "# Readme\n\nA lantern by the door.\n");
    server.search_until("/api/search?q=lantern", |answer| answer["total"] == 2);
    // A new file in a new folder, found when the watcher lists again.
    repo.write("notes/new.md", "# New\n\nA lantern at sea.\n");
    server.search_until("/api/search?q=lantern", |answer| answer["total"] == 3);
    // A removed file.
    repo.remove("docs/draft.md");
    let after = server.search_until("/api/search?q=lantern", |answer| answer["total"] == 2);
    assert_eq!(paths(&after), ["README.md", "notes/new.md"]);
    let status = server.json("/api/status")["search"]["worktree"].clone();
    assert_eq!(status["documents"], 4, "{status}");
    assert!(status["changed"].as_u64().unwrap() >= 2, "{status}");
    assert!(status["bytes"].as_u64().unwrap() > 0, "{status}");

    // A file that cannot be parsed is listed as skipped.
    repo.write("README.md", b"# Readme\n\n\xff lantern\n");
    let broken = server.search_until("/api/search?q=lantern", |answer| answer["total"] == 1);
    assert_eq!(broken["skipped"][0]["path"], "README.md", "{broken}");

    // A new rules file type indexes everything again.
    repo.write(
        "driftmark.toml",
        format!("{RULES}\n[[type]]\nname = \"note\"\npaths = [\"notes/*.md\"]\n"),
    );
    let typed = server.search_until("/api/search?q=lantern&type=note", |answer| {
        answer["total"] == 1
    });
    assert_eq!(paths(&typed), ["notes/new.md"]);
    let status = server.json("/api/status")["search"]["worktree"].clone();
    assert_eq!(status["changed"], 0, "{status}");

    // A committed revision is not affected by the working tree.
    let main = server.json("/api/search?q=lantern&rev=main");
    assert_eq!(main["total"], 0);
    repo.commit("second", T0 + DAY);
    let committed = server.json("/api/search?q=lantern&rev=main");
    assert_eq!(paths(&committed), ["notes/new.md"]);
}
