//! `driftmark lint`: the writing check, run on files in a working tree.
//!
//! The rules live in `driftmark_core::lint`. This module finds the files,
//! reads them and the rules file, answers the core's questions about link
//! targets and diagrams, and prints the findings.

mod mermaid;
mod targets;

use std::ffi::OsString;
use std::fmt::Write as _;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use driftmark_core::git::{GitRead, Roots, Snapshot};
use driftmark_core::lint::{Finding, LintSettings, Linter, Severity};
use driftmark_core::rules::Rules;
use serde_json::json;

use crate::git::Git;

pub use mermaid::MermaidCheck;
pub use targets::FileTargets;

/// Where the rules file lives, relative to the repository root.
pub const RULES_FILE: &str = "driftmark.toml";

/// Exit code when no error-level finding exists.
pub const EXIT_CLEAN: u8 = 0;
/// Exit code when at least one error-level finding exists.
pub const EXIT_ERRORS: u8 = 1;
/// Exit code when the command could not run: bad arguments, no repository,
/// or a rules file that does not parse.
pub const EXIT_USAGE: u8 = 2;

/// The most findings the Claude Code hook reports at once. Claude reads them
/// all, so a file with hundreds would fill its context; the rest are named by
/// count.
const HOOK_FINDINGS: usize = 50;

/// How `driftmark lint` is called.
const USAGE: &str = "\
usage: driftmark lint [--json] [<path>...]
       driftmark lint --hook claude-code

Checks Markdown files against driftmark's writing rules. With no path, it
checks every Markdown file in the repository; a folder means every Markdown
file in it. Both skip the files that `exclude` in the rules file's [lint]
table names; a file named on its own is always checked. The rules file is
driftmark.toml at the repository root.

  --json               print the findings as JSON
  --hook claude-code   read a Claude Code PostToolUse event on standard input
                       and check the file it edited

Exit codes: 0 no errors, 1 errors found, 2 the check could not run.";

/// One file's findings, with its path from the repository root.
pub struct FileReport {
    /// The file, relative to the repository root with `/` between folders.
    pub path: String,
    /// What the lint found, in order of line.
    pub findings: Vec<Finding>,
}

/// A repository ready to lint: its root, its rules, and the helpers the core
/// asks.
pub struct Workspace {
    /// The working tree's root folder.
    pub root: PathBuf,
    /// The rules file, or the defaults when there is none.
    pub rules: Rules,
    targets: FileTargets,
    diagrams: MermaidCheck,
}

impl Workspace {
    /// Opens the working tree at `root` and reads its rules file, with the
    /// presets it names, or, without one, the presets whose layout is there.
    ///
    /// # Errors
    ///
    /// A sentence for the user when the rules file cannot be read or is not
    /// valid.
    pub fn open(root: &Path) -> Result<Workspace, String> {
        let rules = crate::presets::rules_on_disk(root)?;
        Ok(Workspace {
            root: root.to_path_buf(),
            rules,
            targets: FileTargets::new(root),
            diagrams: MermaidCheck::new(),
        })
    }

    /// Lints the files at these paths, each relative to the root. Files run
    /// on every core at once; the reports come back in the order given.
    #[must_use]
    pub fn lint(&self, paths: &[String]) -> Vec<FileReport> {
        let linter = Linter {
            rules: &self.rules,
            targets: &self.targets,
            diagrams: &self.diagrams,
        };
        let next = AtomicUsize::new(0);
        let done: Mutex<Vec<(usize, FileReport)>> = Mutex::new(Vec::with_capacity(paths.len()));
        let threads = std::thread::available_parallelism()
            .map_or(1, std::num::NonZeroUsize::get)
            .min(paths.len().max(1));
        std::thread::scope(|scope| {
            for _ in 0..threads {
                scope.spawn(|| {
                    loop {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some(path) = paths.get(index) else { break };
                        let findings = match std::fs::read(self.root.join(path)) {
                            Ok(bytes) => linter.lint(path, bytes),
                            Err(error) => vec![unreadable(&error)],
                        };
                        let report = FileReport {
                            path: path.clone(),
                            findings,
                        };
                        done.lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .push((index, report));
                    }
                });
            }
        });
        let mut done = done
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        done.sort_by_key(|(index, _)| *index);
        done.into_iter().map(|(_, report)| report).collect()
    }
}

/// The finding for a file that could not be read from disk.
fn unreadable(error: &std::io::Error) -> Finding {
    Finding {
        rule: driftmark_core::lint::Rule::Parse,
        severity: Severity::Error,
        line: 1,
        column: 1,
        message: format!("the file could not be read: {error}"),
    }
}

/// The repository root that holds `start`, as a canonical path.
///
/// # Errors
///
/// A sentence for the user when `start` is not inside a git repository with
/// a working tree.
pub fn find_root(start: &Path) -> Result<PathBuf, String> {
    let git =
        Git::open(start).map_err(|error| format!("run it inside a git repository ({error})"))?;
    let workdir = git
        .workdir()
        .ok_or("the repository is bare, so it has no files to check")?;
    workdir
        .canonicalize()
        .map_err(|error| format!("the repository folder could not be read: {error}"))
}

/// A path from the repository root with `/` between folders, for a file or
/// folder inside it; `None` when it is outside.
#[must_use]
pub fn relative(root: &Path, path: &Path) -> Option<String> {
    let inside = path.strip_prefix(root).ok()?;
    let parts: Option<Vec<&str>> = inside
        .components()
        .map(|part| part.as_os_str().to_str())
        .collect();
    Some(parts?.join("/"))
}

/// Whether a file name ends in `.md` or `.markdown`, in any case.
fn is_markdown(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("md") || extension.eq_ignore_ascii_case("markdown")
        })
}

/// Runs `driftmark lint` with the arguments after `lint`, and returns the
/// exit code.
pub fn run(
    args: &[OsString],
    stdin: &mut dyn Read,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> u8 {
    let mut json = false;
    let mut hook = None;
    let mut paths = Vec::new();
    let mut args = args.iter();
    let mut only_paths = false;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            _ if only_paths => paths.push(PathBuf::from(arg)),
            Some("--json") => json = true,
            Some("--hook") => hook = Some(args.next().cloned().unwrap_or_default()),
            Some("--help" | "-h") => {
                let _ = writeln!(stdout, "{USAGE}");
                return EXIT_CLEAN;
            }
            Some("--") => only_paths = true,
            Some(flag) if flag.starts_with('-') => {
                let _ = writeln!(stderr, "driftmark lint: unknown option `{flag}`\n\n{USAGE}");
                return EXIT_USAGE;
            }
            _ => paths.push(PathBuf::from(arg)),
        }
    }
    match hook {
        Some(agent) if agent == "claude-code" && !json && paths.is_empty() => {
            run_hook(stdin, stderr)
        }
        Some(agent) if agent == "claude-code" => {
            let _ = writeln!(
                stderr,
                "driftmark lint: `--hook` reads its file from standard input, so it takes no path and no `--json`\n\n{USAGE}"
            );
            EXIT_USAGE
        }
        Some(agent) => {
            let _ = writeln!(
                stderr,
                "driftmark lint: `--hook` knows only `claude-code`, not `{}`\n\n{USAGE}",
                agent.to_string_lossy()
            );
            EXIT_USAGE
        }
        None => run_paths(&paths, json, stdout, stderr),
    }
}

fn run_paths(paths: &[PathBuf], json: bool, stdout: &mut dyn Write, stderr: &mut dyn Write) -> u8 {
    let fail = |stderr: &mut dyn Write, message: &str| {
        let _ = writeln!(stderr, "driftmark lint: {message}");
        EXIT_USAGE
    };
    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(error) => {
            return fail(
                stderr,
                &format!("the current folder could not be read: {error}"),
            );
        }
    };
    let root = match find_root(&cwd) {
        Ok(root) => root,
        Err(message) => return fail(stderr, &message),
    };
    let workspace = match Workspace::open(&root) {
        Ok(workspace) => workspace,
        Err(message) => return fail(stderr, &message),
    };
    let files = match select(&root, &cwd, paths, workspace.rules.lint()) {
        Ok(files) => files,
        Err(message) => return fail(stderr, &message),
    };
    let reports = workspace.lint(&files);
    if json {
        let _ = writeln!(stdout, "{}", to_json(&reports));
    } else {
        let _ = write!(stdout, "{}", to_text(&reports));
    }
    let errors = reports
        .iter()
        .flat_map(|report| &report.findings)
        .any(|finding| finding.severity == Severity::Error);
    if errors { EXIT_ERRORS } else { EXIT_CLEAN }
}

/// The files to check, relative to the root: every Markdown file in the
/// repository when no path is given, the Markdown files inside each folder
/// given, and each file given as it is. Files found inside a folder are left
/// out when the `[lint]` table's `exclude` names them; a file given by name
/// is kept, because the person asked for it.
fn select(
    root: &Path,
    cwd: &Path,
    paths: &[PathBuf],
    settings: &LintSettings,
) -> Result<Vec<String>, String> {
    let mut wanted = Vec::new();
    let mut folders = Vec::new();
    for path in paths {
        let full = cwd
            .join(path)
            .canonicalize()
            .map_err(|_| format!("there is no file or folder `{}`", path.display()))?;
        let inside = relative(root, &full).ok_or_else(|| {
            format!(
                "`{}` is outside the repository at {}",
                path.display(),
                root.display()
            )
        })?;
        if full.is_dir() {
            folders.push(inside);
        } else {
            wanted.push(inside);
        }
    }
    if paths.is_empty() {
        folders.push(String::new());
    }
    if !folders.is_empty() {
        let git = Git::open(root)
            .map_err(|error| format!("the repository could not be read: {error}"))?;
        let roots = Roots::new(Vec::<&str>::new()).map_err(|error| error.to_string())?;
        let all = git
            .markdown_files(&Snapshot::Worktree, &roots)
            .map_err(|error| format!("the repository's files could not be listed: {error}"))?;
        for entry in all {
            let Ok(path) = std::str::from_utf8(entry.path.as_bytes()) else {
                continue;
            };
            let in_folder = folders.iter().any(|folder| {
                folder.is_empty()
                    || path
                        .strip_prefix(folder.as_str())
                        .is_some_and(|rest| rest.starts_with('/'))
            });
            if in_folder && !settings.excludes(path) {
                wanted.push(path.to_owned());
            }
        }
    }
    wanted.sort();
    wanted.dedup();
    Ok(wanted)
}

/// `n thing` or `n things`.
fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The findings as text: one `path:line:column: severity rule: message`
/// line each, then a summary line.
#[must_use]
pub fn to_text(reports: &[FileReport]) -> String {
    let mut out = String::new();
    for report in reports {
        for finding in &report.findings {
            out.push_str(&finding_line(&report.path, finding));
            out.push('\n');
        }
    }
    out.push_str(&summary(reports));
    out.push('\n');
    out
}

/// One finding as `path:line:column: severity rule: message`.
#[must_use]
pub fn finding_line(path: &str, finding: &Finding) -> String {
    format!(
        "{path}:{}:{}: {} {}: {}",
        finding.line, finding.column, finding.severity, finding.rule, finding.message
    )
}

/// The last line of the text output.
fn summary(reports: &[FileReport]) -> String {
    let all = || reports.iter().flat_map(|report| &report.findings);
    let errors = all()
        .filter(|finding| finding.severity == Severity::Error)
        .count();
    let warnings = all().count() - errors;
    let with_findings = reports
        .iter()
        .filter(|report| !report.findings.is_empty())
        .count();
    let checked = count(reports.len(), "file", "files");
    if with_findings == 0 {
        format!("driftmark lint: {checked} checked, no problems found")
    } else {
        format!(
            "driftmark lint: {checked} checked, {} and {} in {}",
            count(errors, "error", "errors"),
            count(warnings, "warning", "warnings"),
            count(with_findings, "file", "files")
        )
    }
}

/// The findings as one JSON object: the counts, and every finding with its
/// file.
#[must_use]
pub fn to_json(reports: &[FileReport]) -> serde_json::Value {
    let findings: Vec<serde_json::Value> = reports
        .iter()
        .flat_map(|report| {
            report.findings.iter().map(|finding| {
                json!({
                    "path": report.path,
                    "line": finding.line,
                    "column": finding.column,
                    "severity": finding.severity.name(),
                    "rule": finding.rule.id(),
                    "message": finding.message,
                })
            })
        })
        .collect();
    let errors = findings
        .iter()
        .filter(|finding| finding["severity"] == "error")
        .count();
    json!({
        "files": reports.len(),
        "errors": errors,
        "warnings": findings.len() - errors,
        "findings": findings,
    })
}

/// Runs as a Claude Code `PostToolUse` hook: reads the event on standard
/// input, lints the Markdown file the tool edited, and reports findings on
/// standard error with exit code 2, which Claude Code shows to Claude.
/// Anything else exits 0 in silence, so the hook never gets in the way of
/// edits it has nothing to say about.
fn run_hook(stdin: &mut dyn Read, stderr: &mut dyn Write) -> u8 {
    let mut input = String::new();
    if let Err(error) = stdin.read_to_string(&mut input) {
        let _ = writeln!(
            stderr,
            "driftmark lint: the hook input could not be read: {error}"
        );
        return EXIT_ERRORS;
    }
    let event: serde_json::Value = match serde_json::from_str(&input) {
        Ok(event) => event,
        Err(error) => {
            let _ = writeln!(
                stderr,
                "driftmark lint: the hook input is not JSON: {error}"
            );
            return EXIT_ERRORS;
        }
    };
    let Some(file) = event["tool_input"]["file_path"].as_str() else {
        return EXIT_CLEAN;
    };
    let mut path = PathBuf::from(file);
    if path.is_relative()
        && let Some(cwd) = event["cwd"].as_str()
    {
        path = Path::new(cwd).join(path);
    }
    if !is_markdown(&path) {
        return EXIT_CLEAN;
    }
    let Ok(path) = path.canonicalize() else {
        return EXIT_CLEAN;
    };
    let Some(folder) = path.parent() else {
        return EXIT_CLEAN;
    };
    let Ok(root) = find_root(folder) else {
        return EXIT_CLEAN;
    };
    let Some(inside) = relative(&root, &path) else {
        return EXIT_CLEAN;
    };
    let workspace = match Workspace::open(&root) {
        Ok(workspace) => workspace,
        Err(message) => {
            let _ = writeln!(
                stderr,
                "driftmark lint could not check {inside}: {message}; fix {RULES_FILE}"
            );
            return 2;
        }
    };
    // The hook runs after every edit anywhere in the repository, as a walk
    // does. A file the rules file excludes, such as a fixture that breaks the
    // rules on purpose, must not send the agent off to "fix" it.
    if workspace.rules.lint().excludes(&inside) {
        return EXIT_CLEAN;
    }
    let reports = workspace.lint(std::slice::from_ref(&inside));
    let findings: Vec<&Finding> = reports.iter().flat_map(|report| &report.findings).collect();
    if findings.is_empty() {
        return EXIT_CLEAN;
    }
    let mut text = format!(
        "driftmark lint found {} in {inside}. Fix them in the file:\n",
        count(findings.len(), "problem", "problems")
    );
    for finding in findings.iter().take(HOOK_FINDINGS) {
        text.push_str(&finding_line(&inside, finding));
        text.push('\n');
    }
    if findings.len() > HOOK_FINDINGS {
        let more = findings.len() - HOOK_FINDINGS;
        let _ = writeln!(
            text,
            "…and {more} more; run `driftmark lint {inside}` to see them all."
        );
    }
    let _ = write!(stderr, "{text}");
    2
}
