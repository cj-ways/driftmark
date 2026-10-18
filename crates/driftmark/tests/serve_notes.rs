//! The notes routes of `driftmark serve`, run as the real program on a free
//! port against small repos built for each test: making, listing, placing,
//! changing and handing over notes, the seen marks, the same-origin rule,
//! and the notes file's place in the clone's shared git folder.

mod support;

use std::fmt::Write as _;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use serde_json::{Value, json};
use support::{DAY, T0, TestRepo};

const PROGRAM: &str = env!("CARGO_BIN_EXE_driftmark");

const CACHE_V1: &str = "# Cache pages\n\n## Decision\n\nKeep each page for one hour, then drop it from the cache.\n\nA page with open notes is never cached.\n\n## Limits\n\nThe cache holds at most 64 MB.\n";

const CACHE_V2: &str = "# Cache pages\n\n## Decision\n\nA short intro comes first now.\n\nKeep each page for one hour, then drop it from the cache.\n\n## Limits\n\nThe cache holds at most 128 MB.\n";

/// A repo whose `main` has `docs/cache.md`, and a `feature` branch that
/// rewrites it: one paragraph removed, one added, one number changed.
/// `origin/main` points at `main`.
fn repo() -> TestRepo {
    let repo = TestRepo::new("notes");
    repo.write("docs/cache.md", CACHE_V1);
    let base = repo.commit("first", T0);
    repo.git(&["update-ref", "refs/remotes/origin/main", &base]);
    repo.git(&["checkout", "-q", "-b", "feature"]);
    repo.write("docs/cache.md", CACHE_V2);
    repo.commit("feature", T0 + DAY);
    repo.git(&["checkout", "-q", "main"]);
    repo
}

struct Server {
    child: Child,
    addr: SocketAddr,
}

impl Server {
    fn start(repo: &Path, options: &[&str]) -> Server {
        let mut child = Command::new(PROGRAM)
            .arg("serve")
            .arg("--repo")
            .arg(repo)
            .args(["--addr", "127.0.0.1:0"])
            .args(options)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("start driftmark serve");
        let mut reader = BufReader::new(child.stdout.take().expect("standard output"));
        let mut port = None;
        loop {
            let mut line = String::new();
            assert!(
                reader.read_line(&mut line).expect("read") > 0,
                "stopped early"
            );
            if let Some(rest) = line.strip_prefix("driftmark serve: open http://127.0.0.1:") {
                port = Some(rest.trim_end().trim_end_matches('/').to_owned());
            }
            if line.contains("press Ctrl-C") {
                break;
            }
        }
        std::thread::spawn(move || {
            let _ = std::io::copy(&mut reader, &mut std::io::sink());
        });
        Server {
            child,
            addr: format!("127.0.0.1:{}", port.expect("a port"))
                .parse()
                .expect("an address"),
        }
    }

    fn host(&self) -> String {
        format!("localhost:{}", self.addr.port())
    }

    /// A request, as this server's own page would send it.
    fn send(&self, method: &str, path: &str, body: Option<&Value>) -> (u16, Value) {
        let host = self.host();
        let origin = format!("http://{host}");
        let text = body.map(Value::to_string).unwrap_or_default();
        let mut headers = vec![("Host", host.as_str())];
        if body.is_some() {
            headers.push(("Content-Type", "application/json"));
            headers.push(("Origin", origin.as_str()));
            headers.push(("Sec-Fetch-Site", "same-origin"));
        }
        let (status, raw) = request(self.addr, method, path, &headers, text.as_bytes());
        let value = serde_json::from_slice(&raw).unwrap_or(Value::Null);
        (status, value)
    }

    fn get(&self, path: &str) -> Value {
        let (status, value) = self.send("GET", path, None);
        assert_eq!(status, 200, "{path}: {value}");
        value
    }

    fn page(&self, path: &str) -> String {
        let host = self.host();
        let (status, raw) = request(self.addr, "GET", path, &[("Host", host.as_str())], b"");
        assert_eq!(status, 200, "{path}");
        String::from_utf8_lossy(&raw).into_owned()
    }

    fn post(&self, path: &str, body: &Value) -> Value {
        let (status, value) = self.send("POST", path, Some(body));
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

/// One HTTP/1.1 request; the answer's status and body, unchunked.
fn request(
    addr: SocketAddr,
    method: &str,
    target: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> (u16, Vec<u8>) {
    let mut stream = TcpStream::connect(addr).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(60)))
        .expect("a timeout");
    let mut head = format!(
        "{method} {target} HTTP/1.1\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (name, value) in headers {
        let _ = write!(head, "{name}: {value}\r\n");
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).expect("send");
    stream.write_all(body).expect("send the body");
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
            let size = usize::from_str_radix(std::str::from_utf8(&at[..line]).expect("hex"), 16)
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

/// The byte range of `quote` in `text`.
fn bytes_of(text: &str, quote: &str) -> (usize, usize) {
    let start = text.find(quote).expect("the quote is there");
    (start, start + quote.len())
}

fn note_on(server: &Server, revision: &str, quote: &str, body: &str) -> Value {
    let (start, end) = bytes_of(
        if revision == "feature" {
            CACHE_V2
        } else {
            CACHE_V1
        },
        quote,
    );
    server.post(
        "/api/notes",
        &json!({
            "path": "docs/cache.md", "revision": revision, "start": start, "end": end,
            "stance": "disagree", "ask": "change", "blocking": true, "body": body,
        }),
    )["note"]
        .clone()
}

fn notes_file(repo: &TestRepo) -> PathBuf {
    repo.dir.join(".git").join("driftmark").join("notes.db")
}

#[test]
fn a_note_is_kept_in_the_git_folder_and_found_again_on_a_branch() {
    let repo = repo();
    let server = Server::start(&repo.dir, &["--reader", "Ana"]);
    // Reading a page makes no file.
    let empty = server.get("/api/notes?path=docs/cache.md&rev=main");
    assert_eq!(empty["notes"], json!([]));
    assert!(!notes_file(&repo).exists());
    let page = server.page("/doc/main/-/docs/cache.md");
    assert!(page.contains("data-notes=\"0\" data-path=\"docs/cache.md\" data-rev=\"main\""));

    let first = note_on(
        &server,
        "main",
        "Keep each page for one hour",
        "Too long for phones.",
    );
    assert_eq!(first["id"], 1);
    assert_eq!(first["status"], "open");
    assert_eq!(first["author"], "Ana");
    assert_eq!(first["target"]["side"], "reading");
    assert_eq!(
        first["target"]["quote"]["exact"],
        "Keep each page for one hour"
    );
    assert_eq!(
        first["target"]["heading_path"],
        json!(["Cache pages", "Decision"])
    );
    assert_eq!(first["placement"]["state"], "exact");
    assert_eq!(first["placement"]["lines"], json!([5, 5]));
    assert!(notes_file(&repo).exists());
    let gone = note_on(
        &server,
        "main",
        "A page with open notes is never cached.",
        "Why?",
    );
    let number = note_on(&server, "main", "64 MB", "Measure it.");

    // On the branch, each note is found again with its own state.
    let on_feature = server.get("/api/notes?path=docs/cache.md&rev=feature");
    let states: Vec<(u64, String, Option<String>)> = on_feature["notes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|note| {
            (
                note["id"].as_u64().unwrap(),
                note["placement"]["state"].as_str().unwrap().to_owned(),
                note["placement"]["text"].as_str().map(str::to_owned),
            )
        })
        .collect();
    assert_eq!(
        states,
        [
            (1, "exact".to_owned(), None),
            (2, "orphaned".to_owned(), None),
            (
                3,
                "outdated".to_owned(),
                Some("The cache holds at most 128 MB.".to_owned())
            ),
        ]
    );
    assert_eq!(on_feature["review"]["kind"], "document");
    assert_eq!(on_feature["review"]["scope"], "docs/cache.md");
    assert_eq!(on_feature["reviews"].as_array().unwrap().len(), 1);
    let page = server.page("/doc/feature/-/docs/cache.md");
    assert!(page.contains("data-notes=\"3\""), "{page}");

    // The notes file is shared by every worktree of the clone.
    let linked = repo.dir.parent().unwrap().join("linked");
    repo.git(&["worktree", "add", "-q", linked.to_str().unwrap(), "feature"]);
    drop(server);
    let other = Server::start(&linked, &[]);
    let seen_there = other.get("/api/notes?path=docs/cache.md&rev=worktree");
    assert_eq!(seen_there["notes"].as_array().unwrap().len(), 3);
    assert_eq!(
        seen_there["file"],
        notes_file(&repo)
            .canonicalize()
            .unwrap()
            .display()
            .to_string()
    );
    let _ = (gone, number);
}

#[test]
fn the_loop_runs_through_the_routes() {
    let repo = repo();
    let server = Server::start(&repo.dir, &[]);
    let note = note_on(&server, "main", "Keep each page for one hour", "Too long.");
    let id = note["id"].as_u64().unwrap();
    let edited = server.send(
        "PATCH",
        &format!("/api/notes/{id}"),
        Some(&json!({"stance": "question", "body": "Why an hour?"})),
    );
    assert_eq!(edited.0, 200, "{}", edited.1);
    assert_eq!(edited.1["note"]["stance"], "question");
    assert_eq!(edited.1["note"]["ask"], "change");
    let reply = server.post(
        &format!("/api/notes/n{id}/reply"),
        &json!({"body": "Because phones."}),
    );
    assert_eq!(reply["note"]["status"], "open");
    assert_eq!(reply["note"]["thread"][0]["role"], "reader");
    assert_eq!(reply["note"]["thread"][0]["by"], "reader");

    // Round 1 as a prompt: the open note is sent.
    let review = server.get(&format!("/api/notes/{id}"))["note"]["review"]
        .as_u64()
        .unwrap();
    let prompt = server.post(
        &format!("/api/reviews/r{review}/prompt"),
        &json!({"round": 1}),
    );
    let text = prompt["text"].as_str().unwrap();
    assert!(
        text.starts_with(&format!(
            "Review r{review} · docs/cache.md · 1 note · at worktree\n"
        )),
        "{text}"
    );
    assert!(
        text.contains("## 1 · n1 · question · blocking · ask: change\n"),
        "{text}"
    );
    assert!(
        text.contains("File: docs/cache.md, line 5, under Cache pages › Decision\n"),
        "{text}"
    );
    assert_eq!(prompt["sent"], json!([id]));
    assert_eq!(
        server.get(&format!("/api/notes/{id}"))["note"]["status"],
        "sent"
    );

    // The reader decides; round 2 is a review file next to the notes file.
    let decided = server.post(
        &format!("/api/notes/{id}/decide"),
        &json!({"outcome": "modify", "summary": "Ten minutes."}),
    );
    assert_eq!(decided["note"]["status"], "decided");
    let file = server.post(&format!("/api/reviews/{review}/file"), &json!({"round": 2}));
    let path = PathBuf::from(file["file"].as_str().unwrap());
    assert_eq!(
        path,
        notes_file(&repo)
            .canonicalize()
            .unwrap()
            .parent()
            .unwrap()
            .join("reviews")
            .join(format!("r{review}.md"))
    );
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.starts_with(&format!("# Review r{review} of docs/cache.md\n")));
    assert!(
        written.contains("Decision: Ten minutes.\nCommit:\n"),
        "{written}"
    );
    assert_eq!(file["sent"], json!([]));

    let applied = server.post(
        &format!("/api/notes/{id}/applied"),
        &json!({"commits": ["0123456789abcdef0123456789abcdef01234567"]}),
    );
    assert_eq!(applied["note"]["status"], "applied");
    let late = server.send(
        "POST",
        &format!("/api/notes/{id}/withdraw"),
        Some(&json!({})),
    );
    assert_eq!(late.0, 409);
    assert_eq!(late.1["error"], "the note is applied, so it cannot change");

    // Listing by review.
    let listed = server.get(&format!("/api/notes?review=r{review}"));
    assert_eq!(listed["notes"][0]["id"], id);
    assert_eq!(listed["notes"][0]["placement"], Value::Null);
}

#[test]
fn notes_on_a_change_page_take_a_side() {
    let repo = repo();
    let server = Server::start(&repo.dir, &[]);
    // The new side: the selected text found in its block's source.
    let (_, page_new) = {
        let page = server.page("/changes/origin/main...feature/-/docs/cache.md");
        (0, page)
    };
    assert!(page_new.contains(
        "data-notes=\"0\" data-range=\"origin/main...feature\" data-path=\"docs/cache.md\""
    ));
    assert!(page_new.contains("data-keys=\""), "{page_new}");
    let intro = server.post(
        "/api/notes",
        &json!({
            "path": "docs/cache.md", "range": "origin/main...feature", "side": "new",
            "block": 2, "text": "short intro", "hint": 2,
            "stance": "question", "ask": "explain", "body": "What is it for?",
        }),
    )["note"]
        .clone();
    assert_eq!(intro["target"]["side"], "new");
    assert_eq!(intro["target"]["quote"]["exact"], "short intro");
    // The old side: the whole removed paragraph.
    let removed = server.post(
        "/api/notes",
        &json!({
            "path": "docs/cache.md", "range": "origin/main...feature", "side": "old",
            "block": 3, "stance": "disagree", "ask": "keep", "body": "Keep this rule.",
        }),
    )["note"]
        .clone();
    assert_eq!(removed["target"]["side"], "old");
    assert_eq!(
        removed["target"]["quote"]["exact"],
        "A page with open notes is never cached."
    );
    // A reading note at the base whose text the branch deleted shows on the
    // old side of the change page.
    let deleted = note_on(&server, "main", "never cached", "Hmm.");

    let listed = server.get("/api/notes?range=origin/main...feature");
    let sides: Vec<(u64, String, String)> = listed["notes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|note| {
            (
                note["id"].as_u64().unwrap(),
                note["placement"]["side"].as_str().unwrap().to_owned(),
                note["placement"]["state"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        sides,
        [
            (1, "new".to_owned(), "exact".to_owned()),
            (2, "old".to_owned(), "exact".to_owned()),
            (3, "old".to_owned(), "exact".to_owned()),
        ]
    );
    assert_eq!(listed["review"]["kind"], "range");
    assert_eq!(listed["reviews"].as_array().unwrap().len(), 2);
    // On the reading page of the branch, the old-side note has no place,
    // and the deleted reading note is orphaned.
    let reading = server.get("/api/notes?path=docs/cache.md&rev=feature");
    assert_eq!(reading["notes"][1]["placement"], Value::Null);
    assert_eq!(reading["notes"][2]["placement"]["state"], "orphaned");
    let _ = (removed, deleted);

    // Seen marks are kept per change and shown on the page.
    let key = page_new
        .split("data-keys=\"")
        .nth(1)
        .unwrap()
        .split(['"', ' '])
        .next()
        .unwrap()
        .to_owned();
    let marked = server.post("/api/seen", &json!({"key": key, "seen": true}));
    assert_eq!(marked["seen"], true);
    let page = server.page("/changes/origin/main...feature/-/docs/cache.md");
    assert!(page.contains(&format!("data-seen=\"{key}\"")), "{page}");
    server.post("/api/seen", &json!({"key": key, "seen": false}));
    let page = server.page("/changes/origin/main...feature/-/docs/cache.md");
    assert!(page.contains("data-seen=\"\""), "{page}");
    assert!(page.contains("data-notes=\"3\""));
}

#[test]
fn a_working_tree_note_keeps_its_version_and_follows_edits() {
    let repo = repo();
    repo.write("docs/cache.md", CACHE_V1.replace("64 MB", "96 MB"));
    let server = Server::start(&repo.dir, &[]);
    let text = CACHE_V1.replace("64 MB", "96 MB");
    let (start, end) = bytes_of(&text, "Keep each page for one hour");
    let note = server.post(
        "/api/notes",
        &json!({
            "path": "docs/cache.md", "revision": "worktree", "start": start, "end": end,
            "stance": "agree", "ask": "keep", "body": "Good.",
        }),
    )["note"]
        .clone();
    assert_eq!(note["target"]["commit"], Value::Null);
    // The file changes on disk: the note follows from its kept version.
    repo.write(
        "docs/cache.md",
        format!("# Cache pages\n\nNew intro.\n\n{}", &text[15..]),
    );
    let now = server.get("/api/notes?path=docs/cache.md&rev=worktree");
    assert_eq!(now["notes"][0]["placement"]["state"], "exact");
    assert_eq!(now["notes"][0]["placement"]["lines"], json!([7, 7]));
}

#[test]
fn requests_are_checked() {
    let repo = repo();
    let server = Server::start(&repo.dir, &[]);
    let base = json!({"path": "docs/cache.md", "stance": "agree", "ask": "keep", "body": "x"});
    let with = |extra: Value| {
        let mut body = base.clone();
        for (key, value) in extra.as_object().unwrap() {
            body[key] = value.clone();
        }
        body
    };
    for (body, status, start) in [
        (with(json!({})), 400, "name either the `revision`"),
        (
            with(json!({"revision": "main", "range": "x"})),
            400,
            "name either",
        ),
        (
            with(json!({"revision": "main"})),
            400,
            "give `start` and `end`",
        ),
        (
            with(json!({"revision": "main", "start": 0, "end": 0})),
            400,
            "the selection is empty",
        ),
        (
            with(json!({"revision": "main", "start": 0, "end": 500})),
            400,
            "the selection is empty",
        ),
        (
            with(json!({"revision": "main", "start": 0, "end": 30000})),
            400,
            "a note can quote at most",
        ),
        (
            with(json!({"revision": "main", "side": "old", "start": 0, "end": 3})),
            400,
            "a reading page has no",
        ),
        (
            with(json!({"revision": "nope", "start": 0, "end": 3})),
            404,
            "`nope` does not name",
        ),
        (
            with(json!({"range": "origin/main...feature", "block": 2})),
            400,
            "a change page names the `side`",
        ),
        (
            with(json!({"range": "origin/main...feature", "side": "reading", "block": 2})),
            400,
            "a change page's `side`",
        ),
        (
            with(json!({"range": "origin/main...feature", "side": "new", "block": 99})),
            400,
            "this version has no block 99",
        ),
        (
            with(
                json!({"range": "origin/main...feature", "side": "new", "block": 2, "text": "zzzzzzzzzzzzzzzzzz"}),
            ),
            400,
            "the selected text was not found",
        ),
        (
            with(json!({"revision": "main", "start": 0, "end": 3, "loud": true})),
            400,
            "the request: unknown field",
        ),
        (
            with(json!({"revision": "main", "start": 0, "end": 3, "stance": "maybe"})),
            400,
            "the request: unknown variant",
        ),
        (
            with(json!({"path": "../x.md", "revision": "main", "start": 0, "end": 3})),
            400,
            "`../x.md` is not a path",
        ),
    ] {
        let (got, answer) = server.send("POST", "/api/notes", Some(&body));
        assert_eq!(got, status, "{body}: {answer}");
        let error = answer["error"].as_str().unwrap_or_default();
        assert!(error.starts_with(start), "{body}: {error}");
    }
}

#[test]
fn strangers_unknown_names_and_bad_actions_are_refused() {
    let repo = repo();
    let server = Server::start(&repo.dir, &[]);
    let body = json!({
        "path": "docs/cache.md", "revision": "main", "start": 0, "end": 3,
        "stance": "agree", "ask": "keep", "body": "x",
    })
    .to_string();
    // A page from elsewhere, or a body that is not JSON, never reaches the
    // routes.
    let host = server.host();
    let (status, _) = request(
        server.addr,
        "POST",
        "/api/notes",
        &[
            ("Host", &host),
            ("Content-Type", "application/json"),
            ("Origin", "http://evil.example"),
        ],
        body.as_bytes(),
    );
    assert_eq!(status, 403);
    let (status, _) = request(
        server.addr,
        "POST",
        "/api/notes",
        &[("Host", &host)],
        body.as_bytes(),
    );
    assert_eq!(status, 415);
    // Unknown notes, reviews and actions.
    for (method, path, body, status) in [
        ("GET", "/api/notes/7", None, 404),
        ("GET", "/api/notes/x", None, 404),
        ("POST", "/api/notes/7/withdraw", Some(json!({})), 404),
        ("POST", "/api/notes/1/shout", Some(json!({})), 404),
        (
            "POST",
            "/api/reviews/3/prompt",
            Some(json!({"round": 1})),
            404,
        ),
        ("POST", "/api/reviews/3/fax", Some(json!({"round": 1})), 404),
        ("GET", "/api/notes", None, 400),
        ("GET", "/api/notes?review=x", None, 400),
        (
            "POST",
            "/api/seen",
            Some(json!({"key": "short", "seen": true})),
            400,
        ),
    ] {
        let (got, answer) = server.send(method, path, body.as_ref());
        assert_eq!(got, status, "{method} {path}: {answer}");
    }
    let note = note_on(&server, "main", "64 MB", "x");
    let id = note["id"].as_u64().unwrap();
    for (action, body) in [
        ("reply", json!({"body": "  "})),
        ("applied", json!({"commits": []})),
        ("applied", json!({"commits": ["xyz"]})),
        ("decide", json!({"outcome": "maybe"})),
    ] {
        let (got, answer) = server.send("POST", &format!("/api/notes/{id}/{action}"), Some(&body));
        assert_eq!(got, 400, "{action}: {answer}");
    }
    let review = note["review"].as_u64().unwrap();
    let (got, _) = server.send(
        "POST",
        &format!("/api/reviews/{review}/prompt"),
        Some(&json!({"round": 3})),
    );
    assert_eq!(got, 400);
    let (got, _) = server.send(
        "POST",
        &format!("/api/notes/{id}/applied"),
        Some(&json!({"commits": ["abcdef1"]})),
    );
    assert_eq!(got, 409);
}

#[test]
fn position_tables_and_scripts_are_served() {
    let repo = repo();
    let server = Server::start(&repo.dir, &[]);
    let tables = server.get("/api/positions/main/-/docs/cache.md");
    let blocks = tables.as_array().unwrap();
    assert!(blocks.len() >= 5);
    // Block 2 starts where its paragraph does.
    assert_eq!(blocks[2][0], CACHE_V1.find("Keep each").unwrap());
    let page = server.page("/doc/main/-/docs/cache.md");
    let script = page
        .split("<script src=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap()
        .to_owned();
    let page_js = server.page(&script);
    for name in ["notes.js", "anchor.js"] {
        let at = page_js.find(&format!("\"{name}\":\"/static/")).expect(name);
        let address = page_js[at + name.len() + 4..].split('"').next().unwrap();
        let file = server.page(address);
        assert!(file.len() > 1000, "{name}");
    }
    let (status, _) = server.send("GET", "/api/positions/main/-/docs/none.md", None);
    assert_eq!(status, 404);
}

/// Prints one answer of every notes route on the made-up repo, for the
/// examples in `docs/features/commands/serve.md`.
#[test]
#[ignore = "prints examples for the guide; run with --ignored --nocapture"]
fn examples_for_the_guide() {
    let repo = repo();
    let server = Server::start(&repo.dir, &["--reader", "Ana"]);
    let made = note_on(
        &server,
        "main",
        "Keep each page for one hour",
        "Phones keep pages longer.",
    );
    println!(
        "== POST /api/notes\n{}",
        serde_json::to_string_pretty(&made).unwrap()
    );
    let listed = server.get("/api/notes?path=docs/cache.md&rev=feature");
    println!(
        "== GET /api/notes?path&rev\n{}",
        serde_json::to_string_pretty(&listed).unwrap()
    );
    let replied = server.post("/api/notes/1/reply", &json!({"body": "A day, on phones."}));
    println!(
        "== reply\n{}",
        serde_json::to_string_pretty(&replied["note"]["thread"]).unwrap()
    );
    let prompt = server.post("/api/reviews/1/prompt", &json!({"round": 1}));
    println!(
        "== prompt\n{}",
        serde_json::to_string_pretty(&prompt).unwrap()
    );
    let ranged = server.get("/api/notes?range=origin/main...feature");
    println!(
        "== range\n{}",
        serde_json::to_string_pretty(&ranged).unwrap()
    );
    let seen = server.post(
        "/api/seen",
        &json!({"key": "0123456789abcdef", "seen": true}),
    );
    println!("== seen\n{seen}");
    let positions = server.get("/api/positions/main/-/docs/cache.md");
    println!("== positions\n{positions}");
}
