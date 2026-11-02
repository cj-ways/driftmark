//! The CI parts run as they run on a host, against a made-up repository:
//! the GitHub Action's steps (`.github/actions/driftmark/driftmark.sh`) and
//! the GitLab template's script (`ci/gitlab/driftmark.gitlab-ci.yml`).
//!
//! A small HTTP server on 127.0.0.1 plays GitHub's comments API, GitLab's
//! notes API and a release download host, and records every request, so the
//! tests can check what the scripts asked for: one comment posted, the same
//! comment updated on the next push, other people's comments left alone.
//!
//! The scripts need bash, git, curl, jq and tar, as every hosted runner has.
//! They run on Linux and macOS, so these tests do too.

#![cfg(unix)]

mod support;

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use support::{TempDir, TestRepo};

/// The first line of every summary comment.
const MARKER: &str = "<!-- driftmark:changes -->";

/// The repository root of this checkout.
fn checkout() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn action_path() -> PathBuf {
    checkout().join(".github/actions/driftmark")
}

/// Whether the tools the scripts call are here. Every hosted runner has
/// them; a machine without them skips these tests and says why.
fn tools_present() -> bool {
    let missing: Vec<&str> = ["bash", "git", "curl", "jq", "tar"]
        .into_iter()
        .filter(|tool| {
            Command::new("sh")
                .args(["-c", &format!("command -v {tool}")])
                .output()
                .map_or(true, |output| !output.status.success())
        })
        .collect();
    if !missing.is_empty() {
        eprintln!("skipped: the CI scripts need {missing:?}, which are not installed");
    }
    missing.is_empty()
}

// ---------------------------------------------------------------------------
// The made-up repository.

const RULES: &str = "\
[[type]]
name = \"decision\"
paths = [\"docs/decisions/*.md\"]
status = { front-matter = \"status\" }
current = [\"accepted\"]
other-statuses = [\"proposed\"]
decision-headings = [\"Decision\"]

[lint]
exclude = [\"fixtures/**\"]
";

fn decision(status: &str, minutes: u32) -> String {
    format!(
        "---\nstatus: {status}\ndate: 2026-01-05\n---\n\n# Cache rendered pages\n\n> Short version: how long the server keeps a rendered page.\n\n## Decision\n\nKeep each rendered page in the cache for {minutes} minutes.\n"
    )
}

const GUIDE: &str = "# Guide\n\n> Short version: how to build the made-up lantern tool.\n\n## Build\n\nRun the build first.\n";
const BROKEN: &str = "Notes first.\n\n# Broken on purpose\n\nSummary.\n";
const NEW: &str = "# New page\n\n> Short version: a page the branch adds.\n\nSee [the missing page](missing.md).\n";

/// A repository whose branch `feature` edits a decision record, adds a page
/// with a broken link, touches a fixture the rules file excludes, and a file
/// outside `docs`. `docs/old.md` breaks the rules too, but the branch does
/// not touch it. Returns the repository, the base commit and the head.
fn sample() -> (TestRepo, String, String) {
    let repo = TestRepo::new("ci");
    repo.write("driftmark.toml", RULES);
    repo.write("docs/decisions/0001-cache.md", decision("proposed", 60));
    repo.write("docs/guide.md", GUIDE);
    repo.write("docs/old.md", BROKEN);
    repo.write("fixtures/broken.md", BROKEN);
    repo.write("README.md", GUIDE.replace("# Guide", "# Read me"));
    let base = repo.commit("base", support::T0);
    repo.git(&["checkout", "-q", "-b", "feature"]);
    repo.write("docs/decisions/0001-cache.md", decision("accepted", 90));
    repo.write("docs/new.md", NEW);
    repo.write("fixtures/broken.md", format!("{BROKEN}\nMore.\n"));
    repo.write("README.md", GUIDE.replace("# Guide", "# Read me again"));
    let head = repo.commit("feature", support::T0 + support::DAY);
    (repo, base, head)
}

// ---------------------------------------------------------------------------
// The fake host.

/// One request the fake host received.
#[derive(Debug, Clone)]
struct Seen {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: String,
}

/// What the fake host knows and answers.
#[derive(Default)]
struct Host {
    /// GitHub issue comments on pull request 7, oldest first.
    comments: Vec<Value>,
    /// GitLab notes on merge request 3, oldest first.
    notes: Vec<Value>,
    /// Files served as they are, by path.
    files: HashMap<String, Vec<u8>>,
    /// Refuse every write with 403, as for a fork's read-only token.
    read_only: bool,
    next_id: u64,
    seen: Vec<Seen>,
}

impl Host {
    fn writes(&self) -> Vec<Seen> {
        self.seen
            .iter()
            .filter(|seen| seen.method != "GET")
            .cloned()
            .collect()
    }
}

/// Starts the fake host on a free port and returns its address.
fn start(host: &Arc<Mutex<Host>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a free port");
    let address = format!("http://{}", listener.local_addr().expect("an address"));
    let host = Arc::clone(host);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let _ = answer(&host, stream);
        }
    });
    address
}

fn answer(host: &Mutex<Host>, stream: TcpStream) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let target = parts.next().unwrap_or_default().to_owned();
    let mut headers = HashMap::new();
    loop {
        let mut header = String::new();
        reader.read_line(&mut header)?;
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }
    let length: usize = headers
        .get("content-length")
        .and_then(|length| length.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    let seen = Seen {
        method,
        path: target,
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
    };
    let (status, reply) = {
        let mut host = host.lock().expect("the host lock");
        host.seen.push(seen.clone());
        route(&mut host, &seen)
    };
    let reason = match status {
        200 => "OK",
        201 => "Created",
        403 => "Forbidden",
        _ => "Not Found",
    };
    let mut stream = stream;
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",
        reply.len()
    )?;
    stream.write_all(&reply)?;
    stream.flush()
}

/// One page of 100, as both APIs page.
fn page(items: &[Value], query: &str) -> Vec<u8> {
    let page: usize = query
        .split('&')
        .find_map(|pair| pair.strip_prefix("page="))
        .and_then(|page| page.parse().ok())
        .unwrap_or(1);
    let items: Vec<&Value> = items.iter().skip((page - 1) * 100).take(100).collect();
    serde_json::to_vec(&items).expect("JSON")
}

/// The `body` field of a form, as `curl --data-urlencode body@file` sends it:
/// a space is `+`, and a `+` is `%2B`.
fn form_body(form: &str) -> String {
    let value = form
        .strip_prefix("body=")
        .expect("a body field")
        .replace('+', " ");
    percent_encoding::percent_decode_str(&value)
        .decode_utf8()
        .expect("UTF-8")
        .into_owned()
}

fn route(host: &mut Host, seen: &Seen) -> (u16, Vec<u8>) {
    let (path, query) = seen.path.split_once('?').unwrap_or((&seen.path, ""));
    let refused = || {
        (
            403,
            br#"{"message":"Resource not accessible by integration"}"#.to_vec(),
        )
    };
    let comments = "/repos/cj-ways/sample/issues/7/comments";
    let notes = "/api/v4/projects/42/merge_requests/3/notes";
    match (seen.method.as_str(), path) {
        ("GET", p) if p == comments => (200, page(&host.comments, query)),
        ("POST", p) if p == comments => {
            if host.read_only {
                return refused();
            }
            host.next_id += 1;
            let body: Value = serde_json::from_str(&seen.body).expect("a JSON body");
            let comment = json!({
                "id": 9000 + host.next_id,
                "user": {"login": "github-actions[bot]", "type": "Bot"},
                "body": body["body"],
                "html_url": format!("https://github.example/cj-ways/sample/pull/7#issuecomment-{}", 9000 + host.next_id),
            });
            host.comments.push(comment.clone());
            (201, comment.to_string().into_bytes())
        }
        ("PATCH", p) if p.starts_with("/repos/cj-ways/sample/issues/comments/") => {
            if host.read_only {
                return refused();
            }
            let id: u64 = p
                .rsplit('/')
                .next()
                .and_then(|id| id.parse().ok())
                .unwrap_or(0);
            let body: Value = serde_json::from_str(&seen.body).expect("a JSON body");
            match host.comments.iter_mut().find(|comment| comment["id"] == id) {
                Some(comment) => {
                    comment["body"] = body["body"].clone();
                    (200, comment.to_string().into_bytes())
                }
                None => (404, b"{}".to_vec()),
            }
        }
        ("GET", "/api/v4/user") => (200, br#"{"id": 5, "username": "project_42_bot"}"#.to_vec()),
        ("GET", p) if p == notes => (200, page(&host.notes, query)),
        ("POST", p) if p == notes => {
            host.next_id += 1;
            let note = json!({
                "id": 300 + host.next_id,
                "body": form_body(&seen.body),
                "author": {"id": 5},
                "system": false,
            });
            host.notes.push(note.clone());
            (201, note.to_string().into_bytes())
        }
        ("PUT", p) if p.starts_with(&format!("{notes}/")) => {
            let id: u64 = p
                .rsplit('/')
                .next()
                .and_then(|id| id.parse().ok())
                .unwrap_or(0);
            let body = form_body(&seen.body);
            match host.notes.iter_mut().find(|note| note["id"] == id) {
                Some(note) => {
                    note["body"] = json!(body);
                    (200, note.to_string().into_bytes())
                }
                None => (404, b"{}".to_vec()),
            }
        }
        ("GET", p) => match host.files.get(p) {
            Some(bytes) => (200, bytes.clone()),
            None => (404, b"{}".to_vec()),
        },
        _ => (404, b"{}".to_vec()),
    }
}

// ---------------------------------------------------------------------------
// Running the scripts.

/// What one script run left behind.
struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

/// Runs a bash script with only these variables, plus a `PATH` that finds
/// the driftmark built for this test first, and `HOME`.
fn bash(script: &Path, args: &[&str], cwd: &Path, env: &[(&str, &str)]) -> Run {
    let bin = Path::new(env!("CARGO_BIN_EXE_driftmark"))
        .parent()
        .expect("the program's folder")
        .to_owned();
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let output = Command::new("bash")
        .arg(script)
        .args(args)
        .current_dir(cwd)
        .env_clear()
        .env("PATH", path)
        .env("HOME", std::env::var("HOME").unwrap_or_default())
        .envs(env.iter().copied())
        .output()
        .expect("bash runs");
    Run {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// The `name=value` lines a step wrote to `GITHUB_OUTPUT`.
fn outputs(file: &Path) -> HashMap<String, String> {
    std::fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect()
}

/// A runner's folders for one GitHub job: its temporary folder, its tool
/// cache, and the files the steps write to.
struct Runner {
    _temp: TempDir,
    temp: PathBuf,
    tools: PathBuf,
    output: PathBuf,
    path: PathBuf,
    summary: PathBuf,
}

impl Runner {
    fn new() -> Runner {
        let temp = TempDir::new("runner");
        let root = temp.path().to_owned();
        let runner = Runner {
            temp: root.join("temp"),
            tools: root.join("tools"),
            output: root.join("output"),
            path: root.join("path"),
            summary: root.join("summary.md"),
            _temp: temp,
        };
        std::fs::create_dir_all(&runner.temp).expect("the runner's temporary folder");
        runner
    }

    /// Runs one step of the action, with the runner's own variables and
    /// `env`, after emptying the step's output file.
    fn step(&self, name: &str, workspace: &Path, env: &[(&str, &str)]) -> Run {
        let _ = std::fs::remove_file(&self.output);
        let action = action_path();
        let mut all: Vec<(&str, String)> = vec![
            ("GITHUB_ACTION_PATH", action.display().to_string()),
            ("GITHUB_WORKSPACE", workspace.display().to_string()),
            ("GITHUB_OUTPUT", self.output.display().to_string()),
            ("GITHUB_PATH", self.path.display().to_string()),
            ("GITHUB_STEP_SUMMARY", self.summary.display().to_string()),
            ("RUNNER_TEMP", self.temp.display().to_string()),
            ("RUNNER_TOOL_CACHE", self.tools.display().to_string()),
            ("RUNNER_OS", "Linux".to_owned()),
            ("RUNNER_ARCH", "X64".to_owned()),
        ];
        all.extend(env.iter().map(|(name, value)| (*name, (*value).to_owned())));
        let all: Vec<(&str, &str)> = all.iter().map(|(n, v)| (*n, v.as_str())).collect();
        bash(&action.join("driftmark.sh"), &[name], workspace, &all)
    }

    fn outputs(&self) -> HashMap<String, String> {
        outputs(&self.output)
    }
}

// ---------------------------------------------------------------------------
// The GitHub Action.

#[test]
fn check_annotates_only_the_changed_files_and_exports_the_range() {
    if !tools_present() {
        return;
    }
    let (repo, base, head) = sample();
    let runner = Runner::new();
    let env = [
        ("DRIFTMARK_BASE", base.as_str()),
        ("DRIFTMARK_HEAD", head.as_str()),
        ("DRIFTMARK_FOLDERS", "docs\nfixtures"),
        ("DRIFTMARK_LINT", "report"),
    ];
    let run = runner.step("check", &repo.dir, &env);
    assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
    let annotations: Vec<&str> = run
        .stdout
        .lines()
        .filter(|line| line.starts_with("::"))
        .collect();
    assert_eq!(
        annotations,
        [
            "::error file=docs/new.md,line=5,col=5,title=driftmark lint%3A link::the link `missing.md` leads to `docs/missing.md`, which does not exist; fix the path, or create the file"
        ],
        "only the changed files, without the excluded fixture: {}",
        run.stdout
    );
    assert!(
        run.stdout.contains(&format!(
            "driftmark: Markdown documents changed in {base}...{head}: 3.\n"
        )),
        "{}",
        run.stdout
    );
    let outputs = runner.outputs();
    assert_eq!(outputs["documents"], "3", "two in docs, one in fixtures");
    assert_eq!(outputs["lint-errors"], "true");
    let export = PathBuf::from(&outputs["export"]);
    assert_eq!(
        export,
        runner
            .temp
            .join(format!("driftmark/driftmark-changes-{}.html", &head[..12]))
    );
    let html = std::fs::read_to_string(&export).expect("the export");
    assert!(html.starts_with("<!doctype html>"), "{}", &html[..80]);
    assert!(html.contains("Cache rendered pages"));
    assert!(
        !html.contains("Read me again"),
        "README.md is outside the folders"
    );

    let finish = runner.step(
        "finish",
        &repo.dir,
        &[
            ("DRIFTMARK_LINT", "fail"),
            ("DRIFTMARK_LINT_ERRORS", "true"),
        ],
    );
    assert_eq!(finish.code, 1);
    assert!(
        finish
            .stdout
            .starts_with("::error title=driftmark::driftmark lint found errors")
    );
    let report = runner.step(
        "finish",
        &repo.dir,
        &[
            ("DRIFTMARK_LINT", "report"),
            ("DRIFTMARK_LINT_ERRORS", "true"),
        ],
    );
    assert_eq!(report.code, 0, "report never fails the job");
}

#[test]
fn check_without_lint_or_changes_writes_no_file() {
    if !tools_present() {
        return;
    }
    let (repo, base, _) = sample();
    let runner = Runner::new();
    let run = runner.step(
        "check",
        &repo.dir,
        &[
            ("DRIFTMARK_BASE", base.as_str()),
            ("DRIFTMARK_HEAD", base.as_str()),
            ("DRIFTMARK_LINT", "off"),
        ],
    );
    assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
    assert!(!run.stdout.contains("::"), "{}", run.stdout);
    let outputs = runner.outputs();
    assert_eq!(outputs["documents"], "0");
    assert_eq!(outputs["export"], "");
    assert!(!outputs.contains_key("lint-errors"));
}

#[test]
fn check_names_what_is_missing() {
    if !tools_present() {
        return;
    }
    let (repo, base, head) = sample();
    let runner = Runner::new();
    let cases: [(&[(&str, &str)], &str); 3] = [
        (
            &[("DRIFTMARK_HEAD", head.as_str())],
            "no base or head commit: run on a pull_request event, or give the base and head inputs",
        ),
        (
            &[
                ("DRIFTMARK_BASE", "0123456789012345678901234567890123456789"),
                ("DRIFTMARK_HEAD", head.as_str()),
            ],
            "commit 0123456789012345678901234567890123456789 is not in this clone; check out with fetch-depth: 0",
        ),
        (
            &[
                ("DRIFTMARK_BASE", base.as_str()),
                ("DRIFTMARK_HEAD", head.as_str()),
                ("DRIFTMARK_LINT", "loud"),
            ],
            "lint takes report, fail or off, not loud",
        ),
    ];
    for (env, wanted) in cases {
        let run = runner.step("check", &repo.dir, env);
        assert_eq!(run.code, 1, "{env:?}");
        assert!(
            run.stdout
                .contains(&format!("::error title=driftmark::{wanted}")),
            "{env:?}: {}",
            run.stdout
        );
    }
}

/// Runs the comment step for the sample's pull request 7 against the host.
fn comment(
    runner: &Runner,
    repo: &TestRepo,
    api: &str,
    range: (&str, &str),
    extra: &[(&str, &str)],
) -> Run {
    let mut env = vec![
        ("DRIFTMARK_BASE", range.0),
        ("DRIFTMARK_HEAD", range.1),
        ("DRIFTMARK_FOLDERS", "docs"),
        ("DRIFTMARK_COMMENT", "true"),
        ("DRIFTMARK_DOCUMENTS", "2"),
        (
            "DRIFTMARK_LINK",
            "https://github.example/cj-ways/sample/actions/runs/1/artifacts/2",
        ),
        ("DRIFTMARK_PR", "7"),
        ("GITHUB_API_URL", api),
        ("GITHUB_REPOSITORY", "cj-ways/sample"),
        ("GITHUB_TOKEN", "test-token"),
    ];
    env.extend_from_slice(extra);
    runner.step("comment", &repo.dir, &env)
}

#[test]
fn comment_posts_once_then_updates_the_same_comment() {
    if !tools_present() {
        return;
    }
    let (repo, base, head) = sample();
    let host = Arc::new(Mutex::new(Host::default()));
    {
        // 120 older comments, so the action's own lands on the second page,
        // and one by someone else that starts with the marker too.
        let mut host = host.lock().unwrap();
        for n in 0..120 {
            host.comments.push(json!({
                "id": n,
                "user": {"login": if n % 2 == 0 { "github-actions[bot]" } else { "a-reviewer" }},
                "body": format!("comment {n}"),
            }));
        }
        host.comments.push(json!({
            "id": 500,
            "user": {"login": "a-reviewer"},
            "body": format!("{MARKER}\nnot the action's"),
        }));
    }
    let api = start(&host);
    let runner = Runner::new();

    let first = comment(&runner, &repo, &api, (&base, &head), &[]);
    assert_eq!(first.code, 0, "{}\n{}", first.stdout, first.stderr);
    assert!(
        first.stdout.contains("Posted comment 9001."),
        "{}",
        first.stdout
    );
    let posted = host.lock().unwrap().writes();
    assert_eq!(posted.len(), 1, "{posted:?}");
    assert_eq!(posted[0].method, "POST");
    assert_eq!(posted[0].path, "/repos/cj-ways/sample/issues/7/comments");
    assert_eq!(posted[0].headers["authorization"], "Bearer test-token");
    assert_eq!(posted[0].headers["accept"], "application/vnd.github+json");
    let body: Value = serde_json::from_str(&posted[0].body).unwrap();
    let body = body["body"].as_str().unwrap();
    assert!(body.starts_with(MARKER), "{body}");
    assert!(body.contains("**2 documents changed, "), "{body}");
    assert!(
        body.contains("1. **decision**, edited · `docs/decisions/0001-cache.md`"),
        "{body}"
    );
    assert!(
        body.contains("(<https://github.example/cj-ways/sample/actions/runs/1/artifacts/2>)"),
        "{body}"
    );
    assert_eq!(
        std::fs::read_to_string(&runner.summary).unwrap(),
        body,
        "the job summary holds the same comment"
    );
    assert_eq!(
        runner.outputs()["comment-url"],
        "https://github.example/cj-ways/sample/pull/7#issuecomment-9001"
    );
    let pages: Vec<String> = host
        .lock()
        .unwrap()
        .seen
        .iter()
        .filter(|seen| seen.method == "GET")
        .map(|seen| seen.path.clone())
        .collect();
    assert_eq!(
        pages,
        [
            "/repos/cj-ways/sample/issues/7/comments?per_page=100&page=1",
            "/repos/cj-ways/sample/issues/7/comments?per_page=100&page=2",
        ]
    );

    // A second push: the same comment changes, no new one appears.
    repo.write("docs/guide.md", GUIDE.replace("first", "first, then test"));
    let head2 = repo.commit("second push", support::T0 + 2 * support::DAY);
    let second = comment(&runner, &repo, &api, (&base, &head2), &[]);
    assert_eq!(second.code, 0, "{}\n{}", second.stdout, second.stderr);
    assert!(
        second.stdout.contains("Updated comment 9001."),
        "{}",
        second.stdout
    );
    let host = host.lock().unwrap();
    let writes = host.writes();
    assert_eq!(writes.len(), 2, "{writes:?}");
    assert_eq!(writes[1].method, "PATCH");
    assert_eq!(writes[1].path, "/repos/cj-ways/sample/issues/comments/9001");
    let ours = host.comments.iter().find(|c| c["id"] == 9001).unwrap();
    assert!(
        ours["body"]
            .as_str()
            .unwrap()
            .contains("**3 documents changed, "),
        "{ours}"
    );
    let theirs = host.comments.iter().find(|c| c["id"] == 500).unwrap();
    assert_eq!(theirs["body"], format!("{MARKER}\nnot the action's"));
}

#[test]
fn a_read_only_token_warns_and_the_job_goes_on() {
    if !tools_present() {
        return;
    }
    let (repo, base, head) = sample();
    let host = Arc::new(Mutex::new(Host {
        read_only: true,
        ..Host::default()
    }));
    let api = start(&host);
    let runner = Runner::new();
    let run = comment(&runner, &repo, &api, (&base, &head), &[]);
    assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
    assert!(
        run.stdout.contains("::warning title=driftmark::could not comment (HTTP 403: Resource not accessible by integration); a pull request from a fork gets a read-only token"),
        "{}",
        run.stdout
    );
    assert!(
        std::fs::read_to_string(&runner.summary)
            .unwrap()
            .starts_with(MARKER)
    );
}

#[test]
fn no_comment_is_posted_when_nothing_changed_or_comments_are_off() {
    if !tools_present() {
        return;
    }
    let (repo, base, head) = sample();
    let host = Arc::new(Mutex::new(Host::default()));
    let api = start(&host);
    let runner = Runner::new();
    let unchanged = comment(
        &runner,
        &repo,
        &api,
        (&base, &base),
        &[("DRIFTMARK_DOCUMENTS", "0")],
    );
    assert_eq!(unchanged.code, 0, "{}", unchanged.stderr);
    assert!(
        unchanged
            .stdout
            .contains("No Markdown document changed and no comment to update."),
        "{}",
        unchanged.stdout
    );
    let off = comment(
        &runner,
        &repo,
        &api,
        (&base, &head),
        &[("DRIFTMARK_COMMENT", "false")],
    );
    assert_eq!(off.code, 0);
    let outside = comment(
        &runner,
        &repo,
        &api,
        (&base, &head),
        &[("DRIFTMARK_PR", "")],
    );
    assert_eq!(outside.code, 0);
    assert!(
        outside
            .stdout
            .contains("::notice title=driftmark::not a pull request, so no comment"),
        "{}",
        outside.stdout
    );
    assert!(host.lock().unwrap().writes().is_empty());
    let summary = std::fs::read_to_string(&runner.summary).unwrap();
    assert_eq!(
        summary.matches(MARKER).count(),
        3,
        "each run wrote its summary"
    );
}

#[test]
fn key_names_the_sources_and_install_uses_a_cached_build() {
    if !tools_present() {
        return;
    }
    let runner = Runner::new();
    let here = checkout();
    let key = runner.step("key", &here, &[("DRIFTMARK_VERSION", "source")]);
    assert_eq!(key.code, 0, "{}", key.stdout);
    let first = runner.outputs();
    let key = &first["key"];
    assert!(key.starts_with("driftmark-Linux-X64-source-"), "{key}");
    assert_eq!(key.len(), "driftmark-Linux-X64-source-".len() + 20);
    let dir = PathBuf::from(&first["dir"]);
    assert!(dir.starts_with(&runner.tools), "{}", dir.display());
    runner.step("key", &here, &[("DRIFTMARK_VERSION", "source")]);
    assert_eq!(
        runner.outputs(),
        first,
        "the same sources give the same key"
    );
    runner.step("key", &here, &[("DRIFTMARK_VERSION", "0.1.0")]);
    assert_eq!(runner.outputs()["key"], "");

    // A cached build is used as it is: this one only prints a version.
    std::fs::create_dir_all(&dir).unwrap();
    let fake = dir.join("driftmark");
    std::fs::write(&fake, "#!/bin/sh\necho 'driftmark 9.9.9'\n").unwrap();
    make_executable(&fake);
    let install = runner.step(
        "install",
        &here,
        &[
            ("DRIFTMARK_VERSION", "source"),
            ("DRIFTMARK_DIR", &first["dir"]),
        ],
    );
    assert_eq!(install.code, 0, "{}\n{}", install.stdout, install.stderr);
    assert_eq!(
        install.stdout,
        "driftmark is cached for these sources.\ndriftmark 9.9.9\n"
    );
    assert_eq!(
        std::fs::read_to_string(&runner.path).unwrap(),
        format!("{}\n", dir.display())
    );

    let installed = runner.step("install", &here, &[("DRIFTMARK_VERSION", "installed")]);
    assert_eq!(installed.code, 0, "{}", installed.stdout);
    assert!(
        installed.stdout.starts_with("driftmark "),
        "{}",
        installed.stdout
    );
}

fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .expect("the file is made executable");
}

/// A release archive as the release workflow will make it: the program at
/// the archive's root. The program here runs the driftmark built for the
/// tests, so a job can go on with it. Returns the archive and its
/// SHA256SUMS line.
fn release(name: &str) -> (Vec<u8>, String) {
    let temp = TempDir::new("release");
    let program = temp.path().join("driftmark");
    std::fs::write(
        &program,
        format!(
            "#!/bin/sh\nexec '{}' \"$@\"\n",
            env!("CARGO_BIN_EXE_driftmark")
        ),
    )
    .expect("the release archive is made");
    make_executable(&program);
    let archive = temp.path().join(name);
    let status = Command::new("tar")
        .arg("-czf")
        .arg(&archive)
        .arg("-C")
        .arg(temp.path())
        .arg("driftmark")
        .status()
        .expect("the release archive is made");
    assert!(status.success());
    let bytes = std::fs::read(&archive).expect("the release archive is made");
    let sum = Command::new("sh")
        .arg("-c")
        .arg("if command -v sha256sum >/dev/null; then sha256sum \"$1\"; else shasum -a 256 \"$1\"; fi")
        .arg("sh")
        .arg(&archive)
        .output()
        .expect("the release archive is made");
    let sum = String::from_utf8(sum.stdout).expect("the release archive is made")[..64].to_owned();
    (bytes, format!("{sum}  {name}\n"))
}

#[test]
fn install_downloads_a_release_and_checks_its_sum() {
    if !tools_present() {
        return;
    }
    let file = "driftmark-x86_64-unknown-linux-gnu.tar.gz";
    let (archive, sums) = release(file);
    let host = Arc::new(Mutex::new(Host::default()));
    {
        let mut host = host.lock().unwrap();
        let base = "/cj-ways/driftmark/releases/download/v0.1.0";
        host.files.insert(format!("{base}/{file}"), archive.clone());
        host.files.insert(
            format!("{base}/SHA256SUMS"),
            format!(
                "{}  driftmark-aarch64-apple-darwin.tar.gz\n{sums}",
                "0".repeat(64)
            )
            .into_bytes(),
        );
        let latest = "/cj-ways/driftmark/releases/latest/download";
        host.files.insert(format!("{latest}/{file}"), archive);
        host.files.insert(
            format!("{latest}/SHA256SUMS"),
            format!("{}{}", "0".repeat(64), &sums[64..]).into_bytes(),
        );
    }
    let server = start(&host);
    let runner = Runner::new();
    let here = checkout();
    let env = |version| {
        vec![
            ("DRIFTMARK_VERSION", version),
            ("GITHUB_SERVER_URL", server.as_str()),
            ("DRIFTMARK_REPOSITORY", "cj-ways/driftmark"),
        ]
    };
    let run = runner.step("install", &here, &env("0.1.0"));
    assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
    assert!(
        run.stdout
            .contains(&format!("driftmark {}\n", driftmark_core::version())),
        "{}",
        run.stdout
    );
    let dir = runner.temp.join("driftmark-0.1.0");
    assert_eq!(
        std::fs::read_to_string(&runner.path).unwrap(),
        format!("{}\n", dir.display())
    );
    assert!(!dir.join(file).exists(), "the archive is removed");

    let tampered = runner.step("install", &here, &env("latest"));
    assert_eq!(tampered.code, 1);
    assert!(
        tampered.stdout.contains(&format!(
            "::error title=driftmark::{file} does not match its SHA256SUMS line"
        )),
        "{}",
        tampered.stdout
    );
    let missing = runner.step("install", &here, &env("0.2.0"));
    assert_eq!(missing.code, 1);
    assert!(
        missing.stdout.contains("could not download"),
        "{}",
        missing.stdout
    );
}

// ---------------------------------------------------------------------------
// The GitLab template.

/// The template's script, between its two marker lines, without the YAML
/// indentation.
fn gitlab_script() -> String {
    let template = std::fs::read_to_string(checkout().join("ci/gitlab/driftmark.gitlab-ci.yml"))
        .expect("the template");
    let begin = template
        .find("      # --- driftmark script begins ---\n")
        .expect("the begin marker");
    let end = template
        .find("      # --- driftmark script ends ---\n")
        .expect("the end marker");
    let mut script = String::new();
    for line in template[begin..end].lines() {
        let line = line.strip_prefix("      ").unwrap_or_else(|| {
            assert!(
                line.is_empty(),
                "a script line without its indent: {line:?}"
            );
            line
        });
        script.push_str(line);
        script.push('\n');
    }
    script
}

/// Runs the template's script in the repository, as GitLab runs it for
/// merge request 3 of project 42.
fn gitlab(repo: &TestRepo, api: &str, base: &str, head: &str, extra: &[(&str, &str)]) -> Run {
    let temp = TempDir::new("gitlab");
    let script = temp.path().join("script.sh");
    std::fs::write(&script, gitlab_script()).expect("the script is written");
    let api = format!("{api}/api/v4");
    let dir = repo.dir.display().to_string();
    let mut env = vec![
        ("CI_PROJECT_DIR", dir.as_str()),
        ("CI_PROJECT_ID", "42"),
        ("CI_MERGE_REQUEST_IID", "3"),
        ("CI_MERGE_REQUEST_DIFF_BASE_SHA", base),
        ("CI_COMMIT_SHA", head),
        ("CI_API_V4_URL", api.as_str()),
        ("CI_JOB_URL", "https://gitlab.example/g/p/-/jobs/9"),
        ("DRIFTMARK_FOLDERS", "docs"),
        ("DRIFTMARK_COMMENT", "true"),
        ("DRIFTMARK_LINT", "report"),
        ("DRIFTMARK_VERSION", "installed"),
        (
            "DRIFTMARK_SOURCE",
            "https://github.com/cj-ways/driftmark.git",
        ),
        ("DRIFTMARK_REF", "main"),
        (
            "DRIFTMARK_RELEASES",
            "https://github.com/cj-ways/driftmark/releases",
        ),
        ("DRIFTMARK_GITLAB_TOKEN", "glpat-test"),
    ];
    env.extend_from_slice(extra);
    bash(&script, &[], &repo.dir, &env)
}

#[test]
fn the_gitlab_job_reports_exports_and_keeps_one_note() {
    if !tools_present() {
        return;
    }
    let (repo, base, head) = sample();
    let host = Arc::new(Mutex::new(Host::default()));
    {
        let mut host = host.lock().unwrap();
        host.notes.push(json!({"id": 1, "body": format!("{MARKER}\nsystem"), "author": {"id": 5}, "system": true}));
        host.notes.push(json!({"id": 2, "body": format!("{MARKER}\nsomeone"), "author": {"id": 6}, "system": false}));
    }
    let api = start(&host);
    let first = gitlab(&repo, &api, &base, &head, &[]);
    assert_eq!(first.code, 0, "{}\n{}", first.stdout, first.stderr);
    assert!(
        first.stdout.contains("driftmark: posted note 301."),
        "{}",
        first.stdout
    );
    assert!(
        first
            .stdout
            .contains("driftmark lint: findings in the changed documents: 1."),
        "{}",
        first.stdout
    );
    let report: Value = serde_json::from_str(
        &std::fs::read_to_string(repo.dir.join("driftmark-code-quality.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(report.as_array().unwrap().len(), 1, "{report}");
    assert_eq!(report[0]["location"]["path"], "docs/new.md");
    assert_eq!(report[0]["severity"], "major");
    let html = std::fs::read_to_string(repo.dir.join("driftmark-changes.html")).unwrap();
    assert!(html.starts_with("<!doctype html>"));
    {
        let host = host.lock().unwrap();
        let writes = host.writes();
        assert_eq!(writes.len(), 1, "{writes:?}");
        assert_eq!(writes[0].path, "/api/v4/projects/42/merge_requests/3/notes");
        assert_eq!(writes[0].headers["private-token"], "glpat-test");
        let note = form_body(&writes[0].body);
        assert!(note.starts_with(MARKER), "{note}");
        assert!(
            note.contains(
                "(<https://gitlab.example/g/p/-/jobs/9/artifacts/file/driftmark-changes.html>)"
            ),
            "{note}"
        );
        assert_eq!(host.notes[0]["body"], format!("{MARKER}\nsystem"));
        assert_eq!(host.notes[1]["body"], format!("{MARKER}\nsomeone"));
    }

    repo.write("docs/guide.md", GUIDE.replace("first", "first, then test"));
    let head2 = repo.commit("second push", support::T0 + 2 * support::DAY);
    let second = gitlab(&repo, &api, &base, &head2, &[]);
    assert_eq!(second.code, 0, "{}\n{}", second.stdout, second.stderr);
    assert!(
        second.stdout.contains("driftmark: updated note 301."),
        "{}",
        second.stdout
    );
    let host = host.lock().unwrap();
    let writes = host.writes();
    assert_eq!(writes.len(), 2);
    assert_eq!(writes[1].method, "PUT");
    assert_eq!(
        writes[1].path,
        "/api/v4/projects/42/merge_requests/3/notes/301"
    );
    assert!(
        host.notes[2]["body"]
            .as_str()
            .unwrap()
            .contains("**3 documents changed, "),
        "{}",
        host.notes[2]
    );
}

#[test]
fn the_gitlab_job_downloads_a_release_and_fails_on_lint_errors_when_asked() {
    if !tools_present() || std::env::consts::ARCH != "x86_64" || std::env::consts::OS != "linux" {
        return;
    }
    let file = "driftmark-x86_64-unknown-linux-gnu.tar.gz";
    let (archive, sums) = release(file);
    let host = Arc::new(Mutex::new(Host::default()));
    {
        let mut host = host.lock().unwrap();
        host.files
            .insert(format!("/releases/download/v0.1.0/{file}"), archive);
        host.files.insert(
            "/releases/download/v0.1.0/SHA256SUMS".to_owned(),
            sums.into_bytes(),
        );
    }
    let server = start(&host);
    let (repo, base, head) = sample();
    let releases = format!("{server}/releases");
    let run = gitlab(
        &repo,
        &server,
        &base,
        &head,
        &[
            ("DRIFTMARK_VERSION", "0.1.0"),
            ("DRIFTMARK_RELEASES", releases.as_str()),
            ("DRIFTMARK_LINT", "fail"),
            ("DRIFTMARK_COMMENT", "false"),
        ],
    );
    assert_eq!(run.code, 1, "{}\n{}", run.stdout, run.stderr);
    assert!(
        run.stdout
            .contains(&format!("driftmark {}\n", driftmark_core::version())),
        "{}",
        run.stdout
    );
    assert!(
        run.stdout
            .contains("driftmark: DRIFTMARK_COMMENT is false, so no note."),
        "{}",
        run.stdout
    );
    assert!(
        run.stderr
            .contains("driftmark: driftmark lint found errors in the changed documents"),
        "{}",
        run.stderr
    );
    assert!(host.lock().unwrap().writes().is_empty());
}

#[test]
fn the_gitlab_job_without_a_token_still_exports() {
    if !tools_present() {
        return;
    }
    let (repo, base, head) = sample();
    let run = gitlab(
        &repo,
        "http://127.0.0.1:9",
        &base,
        &head,
        &[("DRIFTMARK_GITLAB_TOKEN", "")],
    );
    assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
    assert!(
        run.stdout
            .contains("driftmark: no DRIFTMARK_GITLAB_TOKEN variable, so no note"),
        "{}",
        run.stdout
    );
    assert!(repo.dir.join("driftmark-changes.html").exists());
}

#[test]
fn the_template_runs_only_in_merge_request_pipelines_and_keeps_its_files() {
    let template =
        std::fs::read_to_string(checkout().join("ci/gitlab/driftmark.gitlab-ci.yml")).unwrap();
    for line in [
        "    - if: $CI_PIPELINE_SOURCE == \"merge_request_event\"\n",
        "    GIT_DEPTH: \"0\"\n",
        "    expose_as: driftmark changes\n",
        "      - driftmark-changes.html\n",
        "      codequality: driftmark-code-quality.json\n",
    ] {
        assert!(template.contains(line), "{line:?}");
    }
    assert!(gitlab_script().starts_with("# --- driftmark script begins ---\nset -eo pipefail\n"));
}
