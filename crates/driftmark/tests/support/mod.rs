//! Small git repos for tests, built with the `git` command in a temporary
//! folder that is removed afterwards.
//!
//! Every repo gets an empty global config and no system config, so the
//! machine's own git settings cannot change what is committed. The readers
//! under test still run with the machine's settings, as they would for a
//! user; each repo pins the settings that matter, such as line endings.

#![allow(dead_code, reason = "each test file uses part of this module")]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

use driftmark::git::{CommandReader, Git, GixReader};
use driftmark_core::git::{GitError, GitRead};
use serde_json::Value;

/// 2026-01-01 00:00 UTC. Test commits are dated from here, one day apart.
pub const T0: i64 = 1_767_225_600;
pub const DAY: i64 = 86_400;

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A folder under the system's temporary folder, removed when dropped.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(label: &str) -> Self {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let name = format!("driftmark-{label}-{}-{n}", std::process::id());
        let path = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create a temporary folder");
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A repo made for one test.
pub struct TestRepo {
    temp: TempDir,
    pub dir: PathBuf,
    config: PathBuf,
}

impl TestRepo {
    /// An empty repo on branch `main`, line endings kept as written.
    pub fn new(label: &str) -> Self {
        Self::init(label, &[])
    }

    /// An empty repo made with extra `git init` options.
    pub fn init(label: &str, options: &[&str]) -> Self {
        let temp = TempDir::new(label);
        let dir = temp.path().join("repo");
        let config = temp.path().join("gitconfig");
        std::fs::write(&config, "").expect("write an empty git config");
        let repo = Self { temp, dir, config };
        let mut init = repo.command_in(repo.temp.path());
        init.args(["init", "-q", "-b", "main"])
            .args(options)
            .arg(&repo.dir);
        run(&mut init, None);
        repo.git(&["config", "core.autocrlf", "false"]);
        repo
    }

    /// A `git` command in `folder` with this repo's settings.
    fn command_in(&self, folder: &Path) -> Command {
        let mut command = Command::new("git");
        command.arg("-C").arg(folder);
        for name in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_OBJECT_DIRECTORY",
        ] {
            command.env_remove(name);
        }
        command
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", &self.config)
            .env("GIT_AUTHOR_NAME", "Test Author")
            .env("GIT_AUTHOR_EMAIL", "author@example.com")
            .env("GIT_COMMITTER_NAME", "Test Committer")
            .env("GIT_COMMITTER_EMAIL", "committer@example.com");
        command
    }

    /// Runs `git` in the repo and returns its output without the last line
    /// ending. Fails the test when git fails.
    pub fn git(&self, args: &[&str]) -> String {
        run(self.command_in(&self.dir).args(args), None)
    }

    /// Runs `git` in the repo with `input` on its standard input.
    pub fn git_with_input(&self, args: &[&str], input: &[u8]) -> String {
        run(self.command_in(&self.dir).args(args), Some(input))
    }

    /// Runs `git` in the repo with the commit dates set to `time`.
    pub fn git_at(&self, time: i64, args: &[&str]) -> String {
        let date = format!("@{time} +0000");
        let mut command = self.command_in(&self.dir);
        command
            .env("GIT_AUTHOR_DATE", &date)
            .env("GIT_COMMITTER_DATE", &date);
        run(command.args(args), None)
    }

    /// Writes a file in the working tree, making its folders.
    pub fn write(&self, path: &str, bytes: impl AsRef<[u8]>) {
        let full = self.dir.join(path);
        std::fs::create_dir_all(full.parent().expect("a parent folder")).expect("make folders");
        std::fs::write(full, bytes).expect("write a file");
    }

    /// Removes a file from the working tree.
    pub fn remove(&self, path: &str) {
        std::fs::remove_file(self.dir.join(path)).expect("remove a file");
    }

    /// Stages everything and commits it at `time`. Returns the commit id.
    pub fn commit(&self, message: &str, time: i64) -> String {
        self.git(&["add", "-A"]);
        self.commit_index(message, time)
    }

    /// Commits what is staged, at `time`. Returns the commit id.
    pub fn commit_index(&self, message: &str, time: i64) -> String {
        self.git_at(time, &["commit", "-q", "--allow-empty", "-m", message]);
        self.git(&["rev-parse", "HEAD"])
    }

    /// Stores `bytes` as a blob and returns its id.
    pub fn blob(&self, bytes: &[u8]) -> String {
        self.git_with_input(&["hash-object", "-w", "--stdin"], bytes)
    }

    /// The three readers on this repo: gix, the `git` command, and the
    /// chooser with its messages collected.
    pub fn readers(&self) -> Readers {
        readers_at(&self.dir)
    }
}

fn run(command: &mut Command, input: Option<&[u8]>) -> String {
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("start git");
    let mut stdin = child.stdin.take().expect("an input pipe");
    stdin
        .write_all(input.unwrap_or_default())
        .expect("write to git");
    drop(stdin);
    let output = child.wait_with_output().expect("wait for git");
    assert!(
        output.status.success(),
        "{command:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8_lossy(&output.stdout);
    text.strip_suffix('\n').unwrap_or(&text).to_owned()
}

/// Messages the chooser printed, shared with the test.
pub type Told = std::sync::Arc<std::sync::Mutex<Vec<String>>>;

pub struct Readers {
    pub gix: GixReader,
    pub command: CommandReader,
    pub chosen: Git,
    pub told: Told,
}

pub fn readers_at(dir: &Path) -> Readers {
    let told = Told::default();
    let sink = told.clone();
    let tell = Box::new(move |message: &str| sink.lock().expect("lock").push(message.to_owned()));
    Readers {
        gix: GixReader::open(dir).expect("gix opens the repo"),
        command: CommandReader::open(dir).expect("git opens the repo"),
        chosen: Git::open_telling(dir, tell).expect("the chooser opens the repo"),
        told,
    }
}

impl Readers {
    /// The three readers, to ask each the same question.
    pub fn all(&self) -> [&dyn GitRead; 3] {
        [&self.gix, &self.command, &self.chosen]
    }

    /// Asks all three readers the same question and checks they agree.
    /// Errors agree when they are the same case; the wording of
    /// [`GitError::Other`] is each reader's own.
    pub fn same<T>(
        &self,
        what: &str,
        ask: impl Fn(&dyn GitRead) -> Result<T, GitError>,
    ) -> Result<T, GitError>
    where
        T: PartialEq + std::fmt::Debug,
    {
        let from_gix = ask(&self.gix);
        let from_command = ask(&self.command);
        let from_chosen = ask(&self.chosen);
        assert_eq!(
            comparable(&from_gix),
            comparable(&from_command),
            "{what}: gix and git differ"
        );
        assert_eq!(
            comparable(&from_gix),
            comparable(&from_chosen),
            "{what}: gix and the chooser differ"
        );
        assert!(
            self.told.lock().expect("lock").is_empty(),
            "{what}: the chooser fell back"
        );
        from_command
    }
}

fn comparable<T: PartialEq + std::fmt::Debug>(
    answer: &Result<T, GitError>,
) -> Result<&T, GitError> {
    match answer {
        Ok(value) => Ok(value),
        Err(GitError::Other { .. }) => Err(GitError::Other {
            message: String::new(),
        }),
        Err(error) => Err(error.clone()),
    }
}

// ---- the documented examples of `docs/features/commands/mcp.md` ------------

/// The decision record of `docs/features/engine/change-list.md`, before and
/// after its change.
const RECORD_BEFORE: &str = "---\nstatus: proposed\n---\n\n# Cache rendered pages\n\n## Decision\n\nKeep each rendered page in the cache for 60 minutes.\n\n## Notes\n\nThe cache lives in memory.\n";
const RECORD_AFTER: &str = "---\nstatus: accepted\n---\n\n# Cache rendered pages\n\n## Decision\n\nKeep each rendered page in the cache for 90 minutes.\n\nNever cache a page with open notes.\n";
const DOC_GUIDE: &str = "# Cache guide\n\n> Short version: how to turn the page cache on and size it.\n\n## Install\n\nRun `cache install` once on each server.\n\n## Configure\n\nSet `cache.size` in megabytes. The default keeps 500 pages.\n";

/// The repository of the examples in `docs/features/commands/mcp.md`.
pub fn documented_repo() -> TestRepo {
    let repo = TestRepo::new("mcp-doc");
    repo.write(
        "driftmark.toml",
        "[[type]]\nname = \"decision\"\npaths = [\"docs/decisions/*.md\"]\nstatus = { front-matter = \"status\" }\ncurrent = [\"accepted\"]\nother-statuses = [\"proposed\"]\ndecision-headings = [\"Decision\"]\n",
    );
    repo.write("docs/decisions/0002-cache-rendered-pages.md", RECORD_BEFORE);
    repo.write("docs/guide.md", DOC_GUIDE);
    let base = repo.commit("first", T0);
    repo.git(&["update-ref", "refs/remotes/origin/main", &base]);
    repo.git(&["checkout", "-q", "-b", "feature"]);
    repo.write("docs/decisions/0002-cache-rendered-pages.md", RECORD_AFTER);
    repo.write(
        "docs/notes/cache-sizes.md",
        "# Cache sizes\n\n> Short version: the size limit of the page cache.\n\nIt holds 500 pages.\n",
    );
    repo.commit("feature", T0 + DAY);
    repo.write(
        "docs/guide.md",
        format!("{DOC_GUIDE}\n```\ncache status\n```\n"),
    );
    repo
}

/// The connector's note tools, whose examples need a notes file.
pub const NOTE_TOOLS: [&str; 6] = [
    "notes_list",
    "note_get",
    "review_get",
    "note_reply",
    "note_propose_decision",
    "note_mark_applied",
];

/// What a documented call must give back: an answer, or a refusal.
pub enum Expected {
    Answer(Value),
    Refusal(String),
}

/// The fenced block that starts on the first line after `at` that is not
/// blank, with its language and its text.
fn block_after(lines: &[&str], at: usize) -> (String, String, usize) {
    let mut line = at + 1;
    while lines[line].trim().is_empty() {
        line += 1;
    }
    let language = lines[line]
        .strip_prefix("```")
        .expect("a fenced block")
        .to_owned();
    let mut text = String::new();
    line += 1;
    while lines[line] != "```" {
        text.push_str(lines[line]);
        text.push('\n');
        line += 1;
    }
    (language, text, line)
}

/// Every example of a document: each "Call `tool` with:" block, and the
/// "It answers:" or "It refuses:" block after it.
pub fn examples(document: &str) -> Vec<(String, Value, Expected)> {
    let lines: Vec<&str> = document.lines().collect();
    let mut found = Vec::new();
    let mut at = 0;
    while at < lines.len() {
        let Some(tool) = lines[at]
            .strip_prefix("Call `")
            .and_then(|rest| rest.strip_suffix("` with:"))
        else {
            at += 1;
            continue;
        };
        let (language, arguments, end) = block_after(&lines, at);
        assert_eq!(language, "json", "{tool}");
        let mut next = end + 1;
        while !lines[next].starts_with("It answers:") && !lines[next].starts_with("It refuses:") {
            next += 1;
        }
        let (language, text, end) = block_after(&lines, next);
        let expected = if lines[next].starts_with("It answers:") {
            assert_eq!(language, "json", "{tool}");
            Expected::Answer(serde_json::from_str(&text).expect("an answer in JSON"))
        } else {
            assert_eq!(language, "text", "{tool}");
            Expected::Refusal(text.trim_end().to_owned())
        };
        found.push((
            tool.to_owned(),
            serde_json::from_str(&arguments).expect("arguments in JSON"),
            expected,
        ));
        at = end + 1;
    }
    found
}

/// A child process that is stopped when dropped. A test holds its server in
/// one of these from the moment it starts, so a test that fails while the
/// server is still starting leaves nothing running behind it.
pub struct Running(pub std::process::Child);

impl std::ops::Deref for Running {
    type Target = std::process::Child;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for Running {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
