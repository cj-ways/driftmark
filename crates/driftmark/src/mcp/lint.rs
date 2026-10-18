//! `lint`: the writing check (`docs/features/lint.md`) on given files and
//! folders, or on the documents changed in the working tree.
//!
//! It reads the same files `driftmark lint` would, with one difference: it
//! reads every file through the git reader, which follows no symbolic link,
//! and its link check never reads a heading through one. An agent's
//! connector must not read anything outside the repository, even when a
//! link inside it points out.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use driftmark_core::git::{GitRead, RepoPath, ResolvedRange, Roots, Snapshot};
use driftmark_core::lint::{Anchors, Finding, LinkTargets, LintSettings, Linter, Rule, Severity};
use driftmark_core::rules::Rules;
use serde_json::{Value, json};

use super::fit;
use super::tools::{
    Args, Context, MAX_LIMIT, MAX_OFFSET, Tool, limit_schema, list_cut, offset_schema, page,
    repo_relative, said,
};
use crate::git::worktree::regular_file;
use crate::lint::{FileTargets, MermaidCheck, RULES_FILE};
use crate::serve::repo::{Problem, parallel};

/// How many findings an answer lists when the call names no limit; the
/// Claude Code hook reports as many.
const DEFAULT_LIMIT: usize = 50;

/// `lint`.
pub(crate) const LINT: Tool = Tool {
    name: "lint",
    title: "Check docs against the writing rules",
    description: "Checks Markdown files against driftmark's writing rules (one # title, a summary, no skipped heading levels, closed code blocks with a language, Mermaid that draws, valid front matter, links and anchors that exist, and more) and the rules file's [lint] table. Without `paths`, it checks the Markdown files changed in the working tree since the last commit, new files included. Each finding has the file, line, column, severity, rule and a message that names the fix.",
    arguments,
    required: &[],
    run,
};

fn arguments() -> Value {
    json!({
        "paths": {
            "type": "array",
            "items": {"type": "string"},
            "description": "Markdown files or folders, from the repository root, such as [\"docs/guide.md\", \"docs/decisions\"]. A folder means every Markdown file in it except those the rules file's [lint] exclude names; a file named on its own is always checked. Default: the files changed in the working tree since the last commit.",
        },
        "offset": offset_schema("findings"),
        "limit": limit_schema("findings", DEFAULT_LIMIT, MAX_LIMIT),
    })
}

/// Which files and headings exist, read without following symbolic links.
///
/// A name in a real folder of the working tree exists as `driftmark lint`
/// sees it, case included. A link that leads through a linked folder is
/// not checked, because the folder it names may be outside the repository,
/// and a heading is read only from a regular file reached through real
/// folders.
struct SafeTargets {
    root: PathBuf,
    files: FileTargets,
}

impl SafeTargets {
    fn new(root: &Path) -> SafeTargets {
        SafeTargets {
            root: root.to_path_buf(),
            files: FileTargets::new(root),
        }
    }

    /// Whether every folder on the way to `path` is a real folder.
    fn through_real_folders(&self, path: &str) -> bool {
        let mut folder = self.root.clone();
        let parts: Vec<&str> = path.split('/').collect();
        parts[..parts.len() - 1].iter().all(|part| {
            folder.push(part);
            std::fs::symlink_metadata(&folder).is_ok_and(|meta| meta.file_type().is_dir())
        })
    }
}

impl LinkTargets for SafeTargets {
    fn exists(&self, path: &str) -> bool {
        // A path through a linked folder counts as there: not checked.
        !self.through_real_folders(path) || self.files.exists(path)
    }

    fn anchors(&self, path: &str) -> Option<Arc<Anchors>> {
        let repo_path = RepoPath::new(path)?;
        regular_file(&self.root, &repo_path)?;
        self.files.anchors(path)
    }
}

/// A finding with its file.
struct Found {
    path: String,
    finding: Finding,
}

fn run(context: &Context, args: &Args) -> Result<Value, String> {
    let limit = args.max_chars()?;
    let offset = args.number("offset", 0, MAX_OFFSET, 0)?;
    let most = args.number("limit", 1, MAX_LIMIT, DEFAULT_LIMIT)?;
    let root = context
        .workdir
        .clone()
        .ok_or("the repository is bare, so it has no files to check")?;
    let rules = context
        .repo
        .rules_at(&Snapshot::Worktree)
        .map_err(|problem| format!("{RULES_FILE}: {}", problem.message))?;
    let asked = args.texts("paths")?;
    let files = files_to_check(context, rules.rules.lint(), &asked)?;
    let found = check(context, &root, &rules.rules, &files);
    let errors = found
        .iter()
        .filter(|found| found.finding.severity == Severity::Error)
        .count();
    let total = found.len();
    let start = offset.min(total);
    let findings: Vec<Value> = found[start..total.min(start.saturating_add(most))]
        .iter()
        .map(|found| {
            json!({
                "path": found.path,
                "line": found.finding.line,
                "column": found.finding.column,
                "severity": found.finding.severity.name(),
                "rule": found.finding.rule.id(),
                "message": found.finding.message,
            })
        })
        .collect();
    let build = |shown: usize| {
        let mut answer = json!({
            "schema_version": 1,
            "files": files.len(),
            "errors": errors,
            "warnings": total - errors,
            "findings": &findings[..shown],
            "page": page(offset, shown, total),
        });
        if asked.is_empty() {
            answer["changed"] = json!(true);
        }
        if let Some(cut) = list_cut(args, "findings", offset, shown, total) {
            answer["cut"] = cut;
        }
        answer
    };
    let sizes: Vec<usize> = findings.iter().map(fit::size).collect();
    Ok(fit::most_that_fit(limit, &sizes, build).0)
}

/// The files to check, from the repository root, sorted: those changed in
/// the working tree since the last commit when `asked` is empty, otherwise
/// each file named and the Markdown files in each folder named. Only files
/// git lists in the working tree count, so a symbolic link is never read.
fn files_to_check(
    context: &Context,
    settings: &LintSettings,
    asked: &[String],
) -> Result<Vec<String>, String> {
    let git = &context.repo.git;
    let listed: BTreeSet<String> = git
        .markdown_files(&Snapshot::Worktree, &Roots::all())
        .map_err(|error| said(error.into()))?
        .into_iter()
        .filter_map(|file| String::from_utf8(file.path.as_bytes().to_vec()).ok())
        .collect();
    let mut files: Vec<String> = Vec::new();
    if asked.is_empty() {
        let range = ResolvedRange {
            base: git.resolve("HEAD").ok(),
            head: Snapshot::Worktree,
            notes: Vec::new(),
        };
        let changes = range
            .changes(git, &Roots::all())
            .map_err(|error| said(Problem::from(error)))?;
        files.extend(
            changes
                .iter()
                .filter(|change| change.new_blob().is_some())
                .filter_map(|change| std::str::from_utf8(change.path().as_bytes()).ok())
                .filter(|path| !settings.excludes(path))
                .map(str::to_owned),
        );
    }
    for text in asked {
        let relative = repo_relative(context, text)?;
        let relative = relative.trim_end_matches('/');
        if listed.contains(relative) {
            files.push(relative.to_owned());
            continue;
        }
        let whole = relative.is_empty() || relative == ".";
        if !whole {
            Roots::new([relative]).map_err(|error| format!("`{text}`: {error}"))?;
        }
        let inside: Vec<&String> = listed
            .iter()
            .filter(|path| {
                whole
                    || path
                        .strip_prefix(relative)
                        .is_some_and(|rest| rest.starts_with('/'))
            })
            .collect();
        if inside.is_empty() {
            return Err(format!(
                "`{text}` is neither a Markdown file git lists in the working tree nor a folder that holds one"
            ));
        }
        files.extend(
            inside
                .into_iter()
                .filter(|path| !settings.excludes(path))
                .cloned(),
        );
    }
    files.sort();
    files.dedup();
    Ok(files)
}

/// Lints `files` on every core, reading each through the git reader. The
/// findings come in the order of the files, each file's in its own order.
fn check(context: &Context, root: &Path, rules: &Rules, files: &[String]) -> Vec<Found> {
    let git = &context.repo.git;
    let targets = SafeTargets::new(root);
    let diagrams = MermaidCheck::new();
    let linter = Linter {
        rules,
        targets: &targets,
        diagrams: &diagrams,
    };
    let reports = parallel(files, context.repo.threads, |path| {
        let read = RepoPath::new(path.as_str())
            .ok_or_else(|| format!("`{path}` is not a path git can store"))
            .and_then(|repo_path| {
                git.read_worktree_file(&repo_path)
                    .map_err(|error| error.to_string())
            });
        match read {
            Ok(bytes) => linter.lint(path, bytes),
            Err(message) => vec![Finding {
                rule: Rule::Parse,
                severity: Severity::Error,
                line: 1,
                column: 1,
                message: format!("the file could not be read: {message}"),
            }],
        }
    });
    files
        .iter()
        .zip(reports)
        .flat_map(|(path, findings)| {
            findings.into_iter().map(|finding| Found {
                path: path.clone(),
                finding,
            })
        })
        .collect()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn links_through_a_linked_folder_are_not_followed() {
        let temp = std::env::temp_dir().join(format!("driftmark-mcp-safe-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        let outside = temp.join("outside");
        let root = temp.join("repo");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::create_dir_all(root.join("docs")).unwrap();
        std::fs::write(outside.join("secret.md"), "# Secret heading\n").unwrap();
        std::fs::write(root.join("docs/real.md"), "# Real heading\n").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("docs/out")).unwrap();
        std::os::unix::fs::symlink(outside.join("secret.md"), root.join("docs/linked.md")).unwrap();
        let targets = SafeTargets::new(&root);
        assert!(targets.exists("docs/real.md"));
        assert!(!targets.exists("docs/missing.md"));
        // Through the linked folder: counted as there, never looked at.
        assert!(targets.exists("docs/out/secret.md"));
        assert!(targets.exists("docs/out/nothing.md"));
        // A linked file exists by its name, but its headings are not read.
        assert!(targets.exists("docs/linked.md"));
        assert_eq!(targets.anchors("docs/linked.md"), None);
        assert_eq!(targets.anchors("docs/out/secret.md"), None);
        assert!(
            targets
                .anchors("docs/real.md")
                .is_some_and(|anchors| anchors.contains("real-heading"))
        );
        assert_eq!(targets.anchors("../x.md"), None);
        let _ = std::fs::remove_dir_all(&temp);
    }
}
