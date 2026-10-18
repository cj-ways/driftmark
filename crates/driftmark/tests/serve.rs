//! `driftmark serve`, run as the real program on a free port against small
//! repos built for each test. Requests go over a plain TCP connection, so a
//! test can send exactly the bytes it means, crafted paths included.

mod support;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;
use support::{DAY, T0, TestRepo};

const PROGRAM: &str = env!("CARGO_BIN_EXE_driftmark");

/// Rules for the test repo: `docs/a.md` is a decision record.
const RULES: &str = "\
[[type]]
name = \"decision\"
paths = [\"docs/a.md\"]
status = { front-matter = \"status\" }
current = [\"accepted\"]
other-statuses = [\"proposed\"]
decision-headings = [\"Decision\"]
";

const README: &str = "# Readme\n\nThe same words.\n";

/// A repo on `main` with a `feature` branch that edits a decision record and
/// adds a document. `origin/main` points at `main`, so it is the default
/// branch. `README.md` and `docs/copy.md` hold the same bytes.
fn branch_repo() -> TestRepo {
    let repo = TestRepo::new("serve");
    repo.write("README.md", README);
    repo.write("docs/copy.md", README);
    repo.write(
        "docs/a.md",
        "---\nstatus: proposed\n---\n\n# Cache pages\n\n## Decision\n\nKeep each page for 60 minutes.\n",
    );
    repo.write("driftmark.toml", RULES);
    let base = repo.commit("first", T0);
    repo.git(&["update-ref", "refs/remotes/origin/main", &base]);
    repo.git(&["checkout", "-q", "-b", "feature"]);
    repo.write(
        "docs/a.md",
        "---\nstatus: accepted\n---\n\n# Cache pages\n\n## Decision\n\nKeep each page for 90 minutes.\n\nNever cache a page with open notes.\n",
    );
    repo.write(
        "docs/new.md",
        "# New & old\n\n> Short version: a new page.\n\nIt holds 500 <b>pages</b>.\n",
    );
    repo.commit("feature", T0 + DAY);
    repo.git(&["checkout", "-q", "main"]);
    repo
}

/// A running `driftmark serve`, stopped when dropped.
struct Server {
    child: Child,
    addr: SocketAddr,
    token: Option<String>,
    lines: Vec<String>,
}

impl Server {
    fn start(repo: &Path, options: &[&str]) -> Server {
        let mut command = Command::new(PROGRAM);
        command.arg("serve").arg("--repo").arg(repo);
        if !options.contains(&"--addr") {
            command.args(["--addr", "127.0.0.1:0"]);
        }
        command
            .args(options)
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command.spawn().expect("start driftmark serve");
        let stdout = child.stdout.take().expect("standard output");
        let mut reader = BufReader::new(stdout);
        let mut lines = Vec::new();
        let mut open = None;
        loop {
            let mut line = String::new();
            let read = reader.read_line(&mut line).expect("read the greeting");
            assert!(read > 0, "the server stopped early: {lines:?}");
            if let Some(rest) = line.strip_prefix("driftmark serve: open http://") {
                open = Some(rest.trim_end().to_owned());
            }
            let last = line.contains("press Ctrl-C to stop");
            lines.push(line);
            if last {
                break;
            }
        }
        // Keep reading standard output so the server never blocks on it.
        std::thread::spawn(move || {
            let _ = std::io::copy(&mut reader, &mut std::io::sink());
        });
        let open = open.expect("an address to open");
        let (host, rest) = open.split_once('/').expect("an address with a path");
        let token = rest.strip_prefix("?token=").map(str::to_owned);
        let port = host.rsplit_once(':').expect("a port").1;
        Server {
            child,
            addr: format!("127.0.0.1:{port}").parse().expect("an address"),
            token,
            lines,
        }
    }

    fn host(&self) -> String {
        format!("localhost:{}", self.addr.port())
    }

    /// A GET request with the local host name.
    fn get(&self, path: &str) -> Reply {
        self.send("GET", path, &[])
    }

    fn send(&self, method: &str, path: &str, headers: &[(&str, &str)]) -> Reply {
        let host = self.host();
        let mut all = vec![("Host", host.as_str())];
        all.extend_from_slice(headers);
        request(self.addr, method, path, &all)
    }

    fn json(&self, path: &str) -> Value {
        let reply = self.get(path);
        assert_eq!(reply.status, 200, "{path}: {}", reply.text());
        assert_eq!(reply.header("content-type"), Some("application/json"));
        serde_json::from_slice(&reply.body).expect("JSON")
    }

    /// Waits until the watcher has listed and counted the working tree.
    fn wait_for_count(&self) {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let status = self.json("/api/status");
            let worktree = &status["worktree"];
            if worktree["watching"] == true && worktree["counting"] == false {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "the working tree was never counted"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// One answer.
struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// Sends one HTTP/1.1 request exactly as written and reads the whole answer.
fn request(addr: SocketAddr, method: &str, target: &str, headers: &[(&str, &str)]) -> Reply {
    let mut stream = TcpStream::connect(addr).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(60)))
        .expect("a read timeout");
    let mut head = format!("{method} {target} HTTP/1.1\r\nConnection: close\r\n");
    for (name, value) in headers {
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).expect("send");
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).expect("read the answer");
    parse(&raw)
}

fn parse(raw: &[u8]) -> Reply {
    let end = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("an answer head");
    let head = String::from_utf8_lossy(&raw[..end]).into_owned();
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|line| line.split(' ').nth(1))
        .and_then(|code| code.parse().ok())
        .expect("a status line");
    let headers: Vec<(String, String)> = lines
        .map(|line| {
            let (name, value) = line.split_once(':').expect("a header line");
            (name.to_owned(), value.trim().to_owned())
        })
        .collect();
    let mut body = raw[end + 4..].to_vec();
    let chunked = headers
        .iter()
        .any(|(n, v)| n.eq_ignore_ascii_case("transfer-encoding") && v == "chunked");
    if chunked {
        body = unchunk(&body);
    }
    Reply {
        status,
        headers,
        body,
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

/// `driftmark diff` on the same repo, as JSON.
fn diff_json(repo: &Path, args: &[&str]) -> Value {
    let output = Command::new(PROGRAM)
        .arg("diff")
        .arg("--repo")
        .arg(repo)
        .args(args)
        .output()
        .expect("run driftmark diff");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("JSON")
}

const SECURITY_HEADERS: [(&str, &str); 6] = [
    (
        "content-security-policy",
        "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; font-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'",
    ),
    ("x-content-type-options", "nosniff"),
    ("referrer-policy", "no-referrer"),
    ("x-frame-options", "DENY"),
    ("cross-origin-resource-policy", "same-origin"),
    ("cross-origin-opener-policy", "same-origin"),
];

fn assert_safe(reply: &Reply, what: &str) {
    for (name, value) in SECURITY_HEADERS {
        assert_eq!(reply.header(name), Some(value), "{what}: {name}");
    }
    for (name, _) in &reply.headers {
        assert!(
            !name.to_ascii_lowercase().starts_with("access-control-"),
            "{what}: no CORS header, found {name}"
        );
    }
}

#[test]
fn pages_and_json_answer_on_every_route() {
    let repo = branch_repo();
    let server = Server::start(&repo.dir, &[]);
    assert_eq!(server.token, None, "this machine only needs no token");
    assert!(server.lines[0].starts_with("driftmark serve: reading "));

    // The home page: the one branch with document changes.
    let home = server.get("/");
    assert_eq!(home.status, 200);
    assert_eq!(
        home.header("content-type"),
        Some("text/html; charset=utf-8")
    );
    let text = home.text();
    assert!(
        text.contains("<a href=\"/changes/origin/main...feature\">feature</a>"),
        "{text}"
    );
    assert!(text.contains("data-level=\"decision\""), "{text}");
    let home = server.json("/api/home");
    assert_eq!(home["schema_version"], 1);
    assert_eq!(home["default_branch"]["name"], "origin/main");
    assert_eq!(home["current_branch"], "main");
    let branches = home["branches"].as_array().unwrap();
    assert_eq!(branches.len(), 1, "main is the default's tip: {home}");
    let feature = &branches[0];
    assert_eq!(feature["name"], "feature");
    assert_eq!(feature["range"], "origin/main...feature");
    assert_eq!(feature["documents"], 2);
    assert_eq!(feature["committed_at"], T0 + DAY);
    let summary = diff_json(&repo.dir, &["--json=summary", "origin/main...feature"]);
    assert_eq!(
        feature["totals"], summary["totals"],
        "the same counts as diff"
    );
    assert_eq!(home["left_out"]["unchanged"], 0);

    // A document at a revision.
    let page = server.get("/doc/feature/-/docs/new.md");
    assert_eq!(page.status, 200);
    let text = page.text();
    assert!(
        text.contains("<title>New &amp; old · driftmark</title>"),
        "{text}"
    );
    assert!(
        text.contains("500 &lt;b&gt;pages"),
        "the source, escaped: {text}"
    );
    let document = server.json("/api/doc/feature/-/docs/a.md");
    assert_eq!(document["path"], "docs/a.md");
    assert_eq!(document["revision"], "feature");
    assert_eq!(document["type"], "decision");
    assert_eq!(document["status"], "accepted");
    assert_eq!(document["title"], "Cache pages");
    assert_eq!(document["commit"], repo.git(&["rev-parse", "feature"]));
    assert_eq!(
        document["blob"],
        repo.git(&["rev-parse", "feature:docs/a.md"])
    );
    assert!(document["source"].as_str().unwrap().contains("90 minutes"));
    let worktree = server.json("/api/doc/worktree/-/README.md");
    assert_eq!(worktree["commit"], "worktree");
    assert!(worktree.get("blob").is_none());

    let head = server.send("HEAD", "/", &[]);
    assert_eq!(head.status, 200);
    assert_eq!(head.body, b"");
}

#[test]
fn change_lists_match_the_diff_command() {
    let repo = branch_repo();
    let server = Server::start(&repo.dir, &[]);
    let summary = diff_json(&repo.dir, &["--json=summary", "origin/main...feature"]);
    // A range's change list, page and JSON; the JSON is diff's summary.
    let page = server.get("/changes/origin/main...feature");
    assert_eq!(page.status, 200);
    let text = page.text();
    assert!(text.contains("2 documents changed"), "{text}");
    assert!(
        text.contains("href=\"/changes/origin/main...feature/-/docs/a.md\""),
        "{text}"
    );
    let mut list = server.json("/api/changes/origin/main...feature");
    assert_eq!(list["skipped"], Value::Array(Vec::new()));
    list.as_object_mut().unwrap().remove("skipped");
    assert_eq!(list, summary);

    // One document's changes; the JSON is diff's entry for it.
    let page = server.get("/changes/origin/main...feature/-/docs/a.md");
    assert_eq!(page.status, 200);
    assert!(
        page.text().contains("decision, edited: status"),
        "{}",
        page.text()
    );
    let one = server.json("/api/changes/origin/main...feature/-/docs/a.md");
    let full = diff_json(&repo.dir, &["--json", "origin/main...feature"]);
    let entry = full["documents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["path"] == "docs/a.md")
        .unwrap();
    assert_eq!(one["documents"].as_array().unwrap(), &vec![entry.clone()]);
    assert_eq!(one["range"], full["range"]);
    assert_eq!(one["schema_version"], 2);

    // The other range forms.
    for range in [
        "main..feature",
        "since/2026-01-02/feature",
        "auto",
        "since/2026-01-01",
    ] {
        let list = server.json(&format!("/api/changes/{range}"));
        assert_eq!(list["schema_version"], 2, "{range}");
    }
    let since = server.json("/api/changes/since/2026-01-02/feature");
    assert_eq!(
        since["totals"], summary["totals"],
        "the feature commit only"
    );
}

#[test]
fn the_script_and_styles_are_built_in_and_kept_long() {
    let repo = branch_repo();
    let server = Server::start(&repo.dir, &[]);
    let page = server.get("/").text();
    for (name, kind) in [("page.js", "text/javascript"), ("page.css", "text/css")] {
        let address = page
            .split('"')
            .find(|part| part.starts_with("/static/") && part.ends_with(name))
            .unwrap()
            .to_owned();
        let file = server.get(&address);
        assert_eq!(file.status, 200, "{address}");
        assert!(file.header("content-type").unwrap().starts_with(kind));
        assert_eq!(
            file.header("cache-control"),
            Some("public, max-age=31536000, immutable")
        );
        let old = server.get(&format!("/static/0000000000000000/{name}"));
        assert_eq!(old.status, 200);
        assert_eq!(
            old.header("cache-control"),
            Some("no-cache"),
            "an old address"
        );
        assert_eq!(old.body, file.body);
    }
}

#[test]
fn answers_are_gzipped_when_the_browser_accepts_it() {
    let repo = branch_repo();
    let server = Server::start(&repo.dir, &[]);
    let plain = server.get("/api/changes/origin/main...feature");
    let zipped = server.send(
        "GET",
        "/api/changes/origin/main...feature",
        &[("Accept-Encoding", "gzip")],
    );
    assert_eq!(zipped.header("content-encoding"), Some("gzip"));
    let mut unzipped = Vec::new();
    flate2::read::GzDecoder::new(zipped.body.as_slice())
        .read_to_end(&mut unzipped)
        .unwrap();
    assert_eq!(unzipped, plain.body);
    let address = server
        .get("/")
        .text()
        .split('"')
        .find(|part| part.ends_with("page.css"))
        .unwrap()
        .to_owned();
    let css = server.send("GET", &address, &[("Accept-Encoding", "gzip")]);
    assert_eq!(css.header("content-encoding"), Some("gzip"));
}

#[test]
fn unknown_names_give_404_and_git_errors_give_500() {
    let repo = branch_repo();
    let server = Server::start(&repo.dir, &[]);
    for (path, message) in [
        ("/doc/nope/-/docs/a.md", "`nope` does not name a commit"),
        (
            "/doc/main/-/docs/new.md",
            "`main` has no Markdown document `docs/new.md`",
        ),
        ("/doc/main/-/Cargo.toml", "is not a Markdown file"),
        (
            "/doc/main",
            "a document&#39;s address is /doc/&lt;revision&gt;/-/&lt;path&gt;",
        ),
        ("/changes/main", "`main` is not a range"),
        ("/changes/nope...main", "`nope` does not name a commit"),
        ("/changes/since/yesterday", "`yesterday` is not a date"),
        (
            "/changes/main...feature/-/README.md",
            "`README.md` did not change in this range",
        ),
        ("/elsewhere", "nothing is at `/elsewhere`"),
    ] {
        let reply = server.get(path);
        assert_eq!(reply.status, 404, "{path}");
        assert!(reply.text().contains(message), "{path}: {}", reply.text());
        assert_eq!(
            reply.header("content-type"),
            Some("text/html; charset=utf-8")
        );
    }
    let reply = server.get("/api/doc/nope/-/a.md");
    assert_eq!(reply.status, 404);
    let error: Value = serde_json::from_slice(&reply.body).unwrap();
    assert_eq!(error["error"], "`nope` does not name a commit");

    // Delete the loose object of a blob the branch needs.
    let blob = repo.git(&["rev-parse", "feature:docs/new.md"]);
    let object = repo
        .dir
        .join(".git")
        .join("objects")
        .join(&blob[..2])
        .join(&blob[2..]);
    let mut permissions = std::fs::metadata(&object)
        .expect("a loose object")
        .permissions();
    #[expect(
        clippy::permissions_set_readonly_false,
        reason = "a test removes the file next"
    )]
    permissions.set_readonly(false);
    std::fs::set_permissions(&object, permissions).expect("make it writable");
    std::fs::remove_file(&object).expect("lose the object");
    let reply = server.get("/doc/feature/-/docs/new.md");
    assert_eq!(reply.status, 500);
    let text = reply.text();
    assert!(
        text.contains(&format!(
            "git could not be read: object {blob} is not in the repo"
        )),
        "{text}"
    );
}

#[test]
fn crafted_paths_do_not_escape_the_repository() {
    let repo = branch_repo();
    let secret = repo.dir.parent().unwrap().join("secret.md");
    std::fs::write(&secret, "# Secret\n\nnot for pages\n").unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&secret, repo.dir.join("docs/link.md")).unwrap();
        std::os::unix::fs::symlink(repo.dir.parent().unwrap(), repo.dir.join("up")).unwrap();
    }
    let server = Server::start(&repo.dir, &[]);
    for path in [
        "/doc/worktree/-/../secret.md",
        "/doc/worktree/-/docs/../../secret.md",
        "/doc/worktree/-/%2E%2E/secret.md",
        "/doc/worktree/-/docs%2F..%2F..%2Fsecret.md",
        "/doc/worktree/-//secret.md",
        "/doc/worktree/-/docs/link.md",
        "/doc/worktree/-/up/secret.md",
        "/doc/main/-/../secret.md",
        "/doc/worktree%2F..%2F../-/secret.md",
        "/changes/auto/-/../secret.md",
        "/api/doc/worktree/-/..%5Csecret.md",
        "/static/x/..%2F..%2Fsecret.md",
        "/../secret.md",
    ] {
        let reply = server.get(path);
        assert_eq!(reply.status, 404, "{path}: {}", reply.text());
        assert!(!reply.text().contains("not for pages"), "{path} leaked");
    }
}

#[test]
fn every_answer_carries_the_safety_headers() {
    let repo = branch_repo();
    let server = Server::start(&repo.dir, &[]);
    let origin = [("Origin", "http://evil.example")];
    for path in [
        "/",
        "/api/home",
        "/doc/main/-/README.md",
        "/nothing",
        "/api/status",
    ] {
        let reply = server.send("GET", path, &origin);
        assert_safe(&reply, path);
    }
    let address = server
        .get("/")
        .text()
        .split('"')
        .find(|part| part.ends_with("page.js"))
        .unwrap()
        .to_owned();
    assert_safe(&server.get(&address), "the script");

    // Requests with a body: JSON from this server's own pages only.
    let json = ("Content-Type", "application/json");
    let foreign = server.send(
        "POST",
        "/api/home",
        &[json, ("Origin", "http://evil.example")],
    );
    assert_eq!(foreign.status, 403);
    assert_safe(&foreign, "a foreign post");
    let site = server.send(
        "POST",
        "/api/home",
        &[json, ("Sec-Fetch-Site", "cross-site")],
    );
    assert_eq!(site.status, 403);
    let form = server.send("POST", "/api/home", &[("Content-Type", "text/plain")]);
    assert_eq!(form.status, 415);
    let host = server.host();
    let origin = format!("http://{host}");
    let own = server.send("POST", "/api/home", &[json, ("Origin", origin.as_str())]);
    assert_eq!(
        own.status, 405,
        "allowed through to a route that takes no body yet"
    );
}

#[test]
fn a_local_server_answers_only_to_local_host_names() {
    let repo = branch_repo();
    let server = Server::start(&repo.dir, &[]);
    let rebound = request(server.addr, "GET", "/api/home", &[("Host", "evil.example")]);
    assert_eq!(rebound.status, 403);
    assert!(
        rebound
            .text()
            .contains("answers only to requests for localhost")
    );
    for host in ["127.0.0.1:1", "[::1]:4170", "LocalHost"] {
        let reply = request(server.addr, "GET", "/api/status", &[("Host", host)]);
        assert_eq!(reply.status, 200, "{host}");
    }
}

#[test]
fn the_token_is_asked_once_and_kept_in_a_cookie() {
    let repo = branch_repo();
    let server = Server::start(&repo.dir, &["--addr", "0.0.0.0:0"]);
    let token = server.token.clone().expect("a random token is printed");
    assert_eq!(token.len(), 32);
    assert!(
        server
            .lines
            .iter()
            .any(|l| l.contains("in place of 0.0.0.0"))
    );
    let port = server.addr.port();
    let host = format!("100.64.0.1:{port}");
    let ask = |path: &str, cookie: Option<&str>| {
        let mut headers = vec![("Host", host.as_str())];
        if let Some(cookie) = cookie {
            headers.push(("Cookie", cookie));
        }
        request(server.addr, "GET", path, &headers)
    };
    for path in [
        "/",
        "/api/home",
        "/events",
        "/doc/main/-/README.md",
        "/static/x/page.js",
    ] {
        let refused = ask(path, None);
        assert_eq!(refused.status, 401, "{path}");
        assert!(refused.text().contains("needs its token"), "{path}");
        assert_safe(&refused, path);
    }
    let wrong = ask("/?token=0123456789abcdef0123456789abcdef", None);
    assert_eq!(wrong.status, 401);
    assert!(wrong.text().contains("is not this server's token"));

    let first = ask(&format!("/doc/main/-/README.md?a=1&token={token}"), None);
    assert_eq!(first.status, 303);
    assert_eq!(first.header("location"), Some("/doc/main/-/README.md?a=1"));
    let cookie = first.header("set-cookie").unwrap().to_owned();
    assert_eq!(
        cookie,
        format!("driftmark-{port}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age=2592000")
    );
    let pair = cookie.split(';').next().unwrap();
    assert_eq!(ask("/doc/main/-/README.md?a=1", Some(pair)).status, 200);
    assert_eq!(ask("/api/home", Some(pair)).status, 200);
    let stale = format!("driftmark-{port}=0123456789abcdef0123456789abcdef");
    assert_eq!(ask("/", Some(&stale)).status, 401);

    let chosen = "my-own-token-0123";
    let asked = Server::start(&repo.dir, &["--token", chosen]);
    assert_eq!(asked.token.as_deref(), Some(chosen));
    assert_eq!(
        asked.get("/").status,
        401,
        "a given token holds on this machine too"
    );
}

#[test]
fn the_cache_parses_a_blob_once_and_keeps_finished_pages() {
    let repo = branch_repo();
    let server = Server::start(&repo.dir, &[]);
    server.wait_for_count();
    let counts = |kind: &str| server.json("/api/status")["cache"]["kinds"][kind].clone();
    let before = counts("documents");
    let number = |value: &Value, field: &str| value[field].as_u64().unwrap_or(0);

    assert_eq!(server.get("/doc/main/-/README.md").status, 200);
    assert_eq!(server.get("/doc/main/-/docs/copy.md").status, 200);
    let documents = counts("documents");
    assert_eq!(
        number(&documents, "misses") - number(&before, "misses"),
        1,
        "two paths with one blob are parsed once: {documents}"
    );
    let pages = counts("pages");
    assert_eq!(number(&pages, "misses"), 2, "a page per path: {pages}");
    assert_eq!(server.get("/doc/main/-/README.md").status, 200);
    let pages = counts("pages");
    assert_eq!(number(&pages, "hits"), 1, "{pages}");

    for _ in 0..2 {
        server.get("/changes/origin/main...feature");
    }
    let ranges = counts("ranges");
    assert_eq!((number(&ranges, "misses"), number(&ranges, "hits")), (1, 1));
    server.get("/");
    server.get("/");
    let branches = counts("branches");
    assert_eq!(
        (number(&branches, "misses"), number(&branches, "hits")),
        (1, 1)
    );

    // A moved branch leads to a new entry; the old one stays valid.
    repo.git(&["checkout", "-q", "feature"]);
    repo.write("docs/new.md", "# New\n\nIt holds 600 pages.\n");
    repo.commit("more", T0 + 2 * DAY);
    repo.git(&["checkout", "-q", "main"]);
    let home = server.json("/api/home");
    assert_eq!(
        home["branches"][0]["tip"],
        repo.git(&["rev-parse", "feature"])
    );
    let status = server.json("/api/status");
    assert!(
        status["cache"]["bytes"].as_u64().unwrap()
            <= status["cache"]["cap_bytes"].as_u64().unwrap()
    );
}

/// Reads an event stream until `wanted` appears, or fails after 20 seconds.
fn read_until(stream: &mut TcpStream, seen: &mut String, wanted: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut buffer = [0; 4096];
    while !seen.contains(wanted) {
        assert!(Instant::now() < deadline, "no {wanted:?} in {seen:?}");
        match stream.read(&mut buffer) {
            Ok(0) => panic!("the stream ended before {wanted:?}: {seen:?}"),
            Ok(n) => seen.push_str(&String::from_utf8_lossy(&buffer[..n])),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {}
            Err(error) => panic!("{error}"),
        }
    }
}

fn open_events(server: &Server) -> (TcpStream, String) {
    let mut stream = TcpStream::connect(server.addr).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_millis(200)))
        .expect("a read timeout");
    let head = format!(
        "GET /events HTTP/1.1\r\nHost: {}\r\nAccept: text/event-stream\r\n\r\n",
        server.host()
    );
    stream.write_all(head.as_bytes()).expect("send");
    let mut seen = String::new();
    read_until(&mut stream, &mut seen, "event: ready");
    assert!(seen.starts_with("HTTP/1.1 200"), "{seen}");
    assert!(seen.contains("content-type: text/event-stream"), "{seen}");
    assert!(seen.contains("content-security-policy:"), "{seen}");
    (stream, seen)
}

#[test]
fn working_tree_changes_reach_open_pages() {
    let repo = branch_repo();
    let server = Server::start(&repo.dir, &[]);
    server.wait_for_count();
    let home = server.json("/api/home");
    assert_eq!(home["worktree"]["state"], "counted");
    assert_eq!(home["worktree"]["documents"], 0);

    let (mut stream, mut seen) = open_events(&server);
    assert_eq!(server.json("/api/status")["worktree"]["listeners"], 1);
    repo.write("README.md", "# Readme\n\nNew words, not committed.\n");
    read_until(
        &mut stream,
        &mut seen,
        "event: change\ndata: {\"paths\":[\"README.md\"]}",
    );
    read_until(&mut stream, &mut seen, "event: counted");

    // A new file in a folder that already holds one.
    seen.clear();
    repo.write("docs/fresh.md", "# Fresh\n");
    read_until(&mut stream, &mut seen, "docs/fresh.md");

    // The changes show in the auto range, on the home page and as pages.
    let auto = server.json("/api/changes/auto");
    let paths: Vec<&str> = auto["documents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["path"].as_str().unwrap())
        .collect();
    assert_eq!(
        paths,
        ["docs/fresh.md", "README.md"],
        "the new file ranks higher"
    );
    assert_eq!(auto["range"]["head"], "worktree");
    server.wait_for_count();
    let home = server.json("/api/home");
    assert_eq!(home["worktree"]["documents"], 2, "{home}");
    let page = server.get("/doc/worktree/-/docs/fresh.md");
    assert_eq!(page.status, 200);
    assert!(page.text().contains("data-live=\"worktree\""));
    let one = server.json("/api/changes/auto/-/README.md");
    assert_eq!(one["documents"][0]["file"], "modified");
    assert_eq!(server.get("/changes/auto/-/README.md").status, 200);
    drop(stream);
}

#[cfg(unix)]
#[test]
fn ctrl_c_stops_the_server_with_open_streams() {
    let repo = branch_repo();
    let mut server = Server::start(&repo.dir, &[]);
    let (_stream, _) = open_events(&server);
    let status = Command::new("kill")
        .args(["-INT", &server.child.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(exit) = server.child.try_wait().unwrap() {
            assert_eq!(exit.code(), Some(0));
            break;
        }
        assert!(Instant::now() < deadline, "the server did not stop");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_bare_repo_has_pages_but_no_working_tree() {
    let repo = branch_repo();
    let bare = repo.dir.parent().unwrap().join("bare.git");
    repo.git(&["clone", "-q", "--bare", ".", bare.to_str().unwrap()]);
    let server = Server::start(&bare, &[]);
    let home = server.json("/api/home");
    assert_eq!(home["default_branch"]["name"], "main");
    assert!(home["worktree"].is_null());
    let auto = server.get("/changes/auto");
    assert_eq!(auto.status, 404);
    assert!(auto.text().contains("the repo has no working tree"));
    assert_eq!(server.get("/doc/feature/-/docs/new.md").status, 200);
}

#[test]
fn the_command_line_is_checked_before_anything_starts() {
    let run = |args: &[&str]| {
        Command::new(PROGRAM)
            .arg("serve")
            .args(args)
            .output()
            .unwrap()
    };
    let help = run(&["--help"]);
    assert_eq!(help.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&help.stdout).starts_with("usage: driftmark serve"));
    let wrong = run(&["--addr", "nowhere"]);
    assert_eq!(wrong.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&wrong.stderr).starts_with("driftmark serve: `--addr` takes"));
    let temp = support::TempDir::new("serve-not-a-repo");
    let not_a_repo = run(&["--repo", temp.path().to_str().unwrap()]);
    assert_eq!(not_a_repo.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&not_a_repo.stderr).contains("is not inside a git repository"));

    let repo = branch_repo();
    let server = Server::start(&repo.dir, &[]);
    let taken = run(&[
        "--repo",
        repo.dir.to_str().unwrap(),
        "--addr",
        &server.addr.to_string(),
    ]);
    assert_eq!(taken.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&taken.stderr).contains("could not listen on"));
}

#[test]
fn without_a_default_branch_nothing_is_compared() {
    let repo = TestRepo::new("serve-trunk");
    repo.write("README.md", README);
    repo.commit("first", T0);
    repo.git(&["branch", "-m", "trunk"]);
    let server = Server::start(&repo.dir, &[]);
    let home = server.json("/api/home");
    assert!(home["default_branch"].is_null(), "{home}");
    assert_eq!(home["current_branch"], "trunk");
    assert_eq!(home["branches"], Value::Array(Vec::new()));
    let page = server.get("/").text();
    assert!(
        page.contains("No default branch: none of origin/main, main, origin/master, master, origin/dev, dev exists"),
        "{page}"
    );
    server.wait_for_count();
    let home = server.json("/api/home");
    assert_eq!(home["worktree"]["state"], "failed");
    assert!(
        home["worktree"]["error"]
            .as_str()
            .unwrap()
            .starts_with("`auto` found no default branch")
    );
}
