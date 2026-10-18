//! `driftmark init` and `driftmark new` on small test repos, and the examples
//! of `docs/features/starter-layout.md` compared with what they write.

mod support;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use driftmark_core::starter::{AGENT_BLOCK, Kind, LAYOUT, Part, new_document, preset};
use support::{TempDir, TestRepo};

/// Runs the program in `dir`; returns its exit code, output and errors.
fn driftmark(dir: &Path, args: &[&str]) -> (i32, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_driftmark"))
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

/// No files, to compare a folder's files with.
const NONE: [&str; 0] = [];

/// Every file under `dir`, outside `.git`, from `dir` with `/` between
/// folders, in order.
fn files(dir: &Path) -> Vec<String> {
    fn walk(root: &Path, folder: &Path, found: &mut Vec<String>) {
        for entry in std::fs::read_dir(folder).expect("the folder is readable") {
            let path = entry.expect("an entry").path();
            if path.file_name().is_some_and(|name| name == ".git") {
                continue;
            }
            if path.is_dir() {
                walk(root, &path, found);
            } else {
                let inside = path.strip_prefix(root).expect("inside the root");
                let parts: Vec<String> = inside
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy().into_owned())
                    .collect();
                found.push(parts.join("/"));
            }
        }
    }
    let mut found = Vec::new();
    walk(dir, dir, &mut found);
    found.sort();
    found
}

fn read(dir: &Path, path: &str) -> String {
    std::fs::read_to_string(dir.join(path)).expect("the file is readable")
}

/// The files of the layout, with their texts.
fn layout_files() -> Vec<(&'static str, &'static str)> {
    LAYOUT
        .into_iter()
        .filter_map(|part| match part {
            Part::File { path, text } => Some((path, text)),
            Part::Folder(_) => None,
        })
        .collect()
}

/// The repository root as `init` prints it.
fn shown_root(repo: &TestRepo) -> String {
    repo.dir
        .canonicalize()
        .expect("the repo exists")
        .display()
        .to_string()
}

/// The repository root of this checkout.
fn this_repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn the_document() -> String {
    std::fs::read_to_string(this_repository().join("docs/features/starter-layout.md"))
        .expect("the feature document exists")
        .replace("\r\n", "\n")
}

/// The text of the fenced block that follows `after` in the document, with
/// the indentation of its fence taken off each line.
fn block_after(document: &str, after: &str) -> String {
    let start = document
        .find(after)
        .expect("the lead-in is in the document")
        + after.len();
    let rest = &document[start..];
    let open = rest.find("```").expect("a fenced block follows");
    let indent = &rest[rest[..open].rfind('\n').map_or(0, |at| at + 1)..open];
    let body = &rest[open..];
    let first_line = body.find('\n').expect("the fence line ends") + 1;
    let close = body[first_line..]
        .find(&format!("\n{indent}```\n"))
        .expect("the block is closed");
    let lines: Vec<&str> = body[first_line..first_line + close]
        .split('\n')
        .map(|line| line.strip_prefix(indent).unwrap_or(line))
        .collect();
    format!("{}\n", lines.join("\n"))
}

#[test]
fn init_without_write_writes_nothing() {
    let repo = TestRepo::new("init-dry");
    let (code, out, err) = driftmark(&repo.dir, &["init"]);
    assert_eq!((code, err.as_str()), (0, ""));
    assert_eq!(files(&repo.dir), NONE);
    assert!(out.starts_with(&format!(
        "driftmark init: the starter layout in {}. Nothing is written without --write.\n\n  would create  docs/decisions/\n",
        shown_root(&repo)
    )));
    assert_eq!(out.matches("would create").count(), 12);
    assert!(out.contains("\nRun `driftmark init --write` to create what is missing."));
    assert!(out.ends_with(AGENT_BLOCK));
}

#[test]
fn an_empty_repository_gets_a_layout_that_passes_its_own_lint() {
    let repo = TestRepo::new("init-empty");
    let (code, out, err) = driftmark(&repo.dir, &["init", "--write", "--agent-file", "AGENTS.md"]);
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    assert!(out.contains("\n  created  AGENTS.md (with the agent block)\n"));
    assert!(!out.contains(AGENT_BLOCK), "the block went into the file");
    for (path, text) in layout_files() {
        assert_eq!(read(&repo.dir, path), text, "{path}");
    }
    assert!(read(&repo.dir, "AGENTS.md").ends_with(AGENT_BLOCK));

    let made: Vec<String> = Kind::ALL
        .into_iter()
        .map(|kind| {
            let (code, out, err) = driftmark(&repo.dir, &["new", kind.name(), "Faster search"]);
            assert_eq!((code, err.as_str()), (0, ""), "{}", kind.name());
            out
        })
        .collect();
    assert_eq!(
        made,
        [
            "docs/decisions/0001-faster-search.md\n",
            "docs/standards/faster-search.md\n",
            "docs/features/faster-search.md\n",
            "docs/working/faster-search/plan.md\n",
            "docs/working/faster-search/status.md\n",
        ]
    );
    let (code, out, err) = driftmark(&repo.dir, &["lint"]);
    assert_eq!((code, err.as_str()), (0, ""));
    assert_eq!(out, "driftmark lint: 12 files checked, no problems found\n");
    let (_, json, _) = driftmark(&repo.dir, &["lint", "--json"]);
    let report: serde_json::Value = serde_json::from_str(&json).expect("JSON");
    assert_eq!(
        (report["errors"].as_u64(), report["warnings"].as_u64()),
        (Some(0), Some(0))
    );
}

#[test]
fn init_never_changes_a_file_that_exists() {
    let repo = TestRepo::new("init-keep");
    repo.write("driftmark.toml", "[lint]\nexclude = [\"vendor/**\"]\n");
    repo.write("docs/standards/writing.md", "# Our own rules\n");
    repo.write("docs/archive/README.md", "");
    let (code, out, err) = driftmark(&repo.dir, &["init", "--write"]);
    assert_eq!((code, err.as_str()), (0, ""));
    assert_eq!(
        read(&repo.dir, "driftmark.toml"),
        "[lint]\nexclude = [\"vendor/**\"]\n"
    );
    assert_eq!(
        read(&repo.dir, "docs/standards/writing.md"),
        "# Our own rules\n"
    );
    assert_eq!(read(&repo.dir, "docs/archive/README.md"), "");
    for skipped in [
        "driftmark.toml",
        "docs/standards/writing.md",
        "docs/archive/README.md",
    ] {
        assert!(
            out.contains(&format!(
                "\n  skipped  {skipped} (it exists, so it was left as it is)\n"
            )),
            "{skipped}: {out}"
        );
    }
    assert!(out.contains("\n  exists   docs/standards/\n"));
    assert!(out.contains("\n  created  docs/standards/README.md\n"));
    let readme = layout_files()
        .into_iter()
        .find(|(path, _)| *path == "docs/standards/README.md")
        .expect("the layout has the README");
    assert_eq!(read(&repo.dir, readme.0), readme.1);
}

#[test]
fn a_second_run_adds_nothing() {
    let repo = TestRepo::new("init-twice");
    repo.write("AGENTS.md", "# Agents\n\nRun the tests before you push.\n");
    let first = driftmark(&repo.dir, &["init", "--write", "--agent-file", "AGENTS.md"]);
    assert_eq!(first.0, 0);
    assert!(
        first
            .1
            .contains("\n  appended  AGENTS.md (the agent block, at the end)\n")
    );
    let after_first = read(&repo.dir, "AGENTS.md");
    assert_eq!(
        after_first,
        format!("# Agents\n\nRun the tests before you push.\n\n{AGENT_BLOCK}")
    );
    let before = files(&repo.dir);
    let (code, out, err) = driftmark(&repo.dir, &["init", "--write", "--agent-file", "AGENTS.md"]);
    assert_eq!((code, err.as_str()), (0, ""));
    assert_eq!(files(&repo.dir), before);
    assert_eq!(read(&repo.dir, "AGENTS.md"), after_first);
    assert_eq!(out.matches("  skipped  ").count(), 8, "{out}");
    assert_eq!(out.matches("  exists   ").count(), 5, "{out}");
    assert!(out.contains("\n  skipped  AGENTS.md (it holds the agent block)\n"));
    let (code, out, _) = driftmark(&repo.dir, &["init", "--agent-file", "AGENTS.md"]);
    assert_eq!(code, 0);
    assert!(out.contains("\nEverything is in place, so there is nothing to create.\n"));
    assert!(
        out.contains("\n  exists  AGENTS.md (it holds the agent block)\n"),
        "{out}"
    );
    assert!(!out.contains(AGENT_BLOCK));
}

#[test]
fn a_dry_run_says_where_the_block_would_go() {
    let repo = TestRepo::new("init-block");
    repo.write("CLAUDE.md", "# Claude");
    let (code, out, _) = driftmark(&repo.dir, &["init", "--agent-file", "CLAUDE.md"]);
    assert_eq!(code, 0);
    assert!(out.contains("\n  would append  CLAUDE.md (the agent block, at the end)\n"));
    assert!(out.ends_with(&format!(
        "\nWith --write, this block goes at the end of CLAUDE.md:\n\n{AGENT_BLOCK}"
    )));
    assert_eq!(read(&repo.dir, "CLAUDE.md"), "# Claude");
    let (code, out, _) = driftmark(&repo.dir, &["init", "--agent-file", "docs/AGENTS.md"]);
    assert_eq!(code, 1, "{out}");
    let (code, out, _) = driftmark(&repo.dir, &["init", "--agent-file", "NEW.md"]);
    assert_eq!(code, 0);
    assert!(out.contains("\n  would create  NEW.md (with the agent block)\n"));
    assert!(!repo.dir.join("NEW.md").exists());
}

#[cfg(unix)]
#[test]
fn a_link_to_the_agent_file_stays_a_link() {
    let repo = TestRepo::new("init-link");
    repo.write("AGENTS.md", "# Agents\n");
    std::os::unix::fs::symlink("AGENTS.md", repo.dir.join("CLAUDE.md")).expect("a link");
    let (code, out, err) = driftmark(&repo.dir, &["init", "--write", "--agent-file", "CLAUDE.md"]);
    assert_eq!((code, err.as_str()), (0, ""));
    assert!(out.contains("\n  appended  AGENTS.md (the agent block, at the end)\n"));
    assert!(
        std::fs::symlink_metadata(repo.dir.join("CLAUDE.md"))
            .expect("the link")
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        read(&repo.dir, "AGENTS.md"),
        format!("# Agents\n\n{AGENT_BLOCK}")
    );
    std::os::unix::fs::symlink("gone.md", repo.dir.join("BROKEN.md")).expect("a link");
    let (code, _, err) = driftmark(&repo.dir, &["init", "--write", "--agent-file", "BROKEN.md"]);
    assert_eq!(code, 1);
    assert_eq!(
        err,
        "driftmark init: `BROKEN.md` is a link to a file that does not exist\n"
    );
    assert!(!repo.dir.join("gone.md").exists());
}

#[test]
fn init_refuses_outside_a_repository() {
    let temp = TempDir::new("init-outside");
    let (code, out, err) = driftmark(temp.path(), &["init", "--write"]);
    assert_eq!((code, out.as_str()), (1, ""));
    assert!(
        err.starts_with("driftmark init: run it inside a git repository"),
        "{err}"
    );
    assert_eq!(files(temp.path()), NONE);
    let (code, out, err) = driftmark(temp.path(), &["new", "decision", "Use one queue"]);
    assert_eq!((code, out.as_str()), (1, ""));
    assert!(
        err.starts_with("driftmark new: run it inside a git repository"),
        "{err}"
    );
    assert_eq!(files(temp.path()), NONE);
}

#[test]
fn an_agent_file_outside_the_repository_stops_everything() {
    let repo = TestRepo::new("init-escape");
    let (code, out, err) = driftmark(
        &repo.dir,
        &["init", "--write", "--agent-file", "../AGENTS.md"],
    );
    assert_eq!((code, out.as_str()), (1, ""));
    assert!(
        err.starts_with("driftmark init: `../AGENTS.md` is outside the repository at "),
        "{err}"
    );
    assert_eq!(files(&repo.dir), NONE);
    assert!(!repo.dir.join("../AGENTS.md").exists());
    repo.write("binary.md", [0xff, 0xfe]);
    let (code, _, err) = driftmark(&repo.dir, &["init", "--write", "--agent-file", "binary.md"]);
    assert_eq!(code, 1);
    assert_eq!(
        err,
        "driftmark init: binary.md is not UTF-8 text, so nothing was added to it\n"
    );
    assert_eq!(files(&repo.dir), ["binary.md"]);
    let (code, _, err) = driftmark(&repo.dir, &["init", "--agent-file", ".."]);
    assert_eq!(
        (code, err.as_str()),
        (1, "driftmark init: `..` names no file\n")
    );
}

#[test]
fn a_folder_named_as_the_agent_file_is_refused() {
    let repo = TestRepo::new("init-folder");
    repo.write("notes/a.md", "# A\n\nB.\n");
    let (code, out, err) = driftmark(&repo.dir, &["init", "--write", "--agent-file", "notes"]);
    assert_eq!((code, out.as_str()), (1, ""));
    assert!(
        err.starts_with("driftmark init: notes could not be read: "),
        "{err}"
    );
    assert_eq!(files(&repo.dir), ["notes/a.md"]);
}

#[test]
fn a_part_that_cannot_be_made_stops_the_run() {
    let repo = TestRepo::new("init-blocked");
    repo.write("docs/standards", "a file where a folder should be\n");
    let (code, out, err) = driftmark(&repo.dir, &["init", "--write"]);
    assert_eq!(code, 1);
    assert!(
        out.contains("\n  created  docs/decisions/README.md\n"),
        "{out}"
    );
    assert!(!out.contains("docs/standards/\n"), "{out}");
    assert!(!out.contains("Next,"), "{out}");
    assert!(
        err.starts_with("driftmark init: docs/standards/ could not be made: "),
        "{err}"
    );
    assert!(!repo.dir.join("driftmark.toml").exists());
}

#[test]
fn init_arguments() {
    let repo = TestRepo::new("init-args");
    let (code, out, _) = driftmark(&repo.dir, &["init", "--help"]);
    assert_eq!(code, 0);
    assert!(out.starts_with("usage: driftmark init [--write] [--agent-file <file>]\n"));
    let (code, _, err) = driftmark(&repo.dir, &["init", "--force"]);
    assert_eq!(code, 2);
    assert!(err.starts_with("driftmark init: unknown argument `--force`\n\nusage:"));
    let (code, _, err) = driftmark(&repo.dir, &["init", "--agent-file"]);
    assert_eq!(code, 2);
    assert!(err.starts_with(
        "driftmark init: `--agent-file` needs a file, such as `--agent-file AGENTS.md`\n\nusage:"
    ));
    assert_eq!(files(&repo.dir), NONE);
    let (_, out, _) = driftmark(&repo.dir, &["--help"]);
    assert!(out.contains("\n  init  "));
    assert!(out.contains("\n  new  "));
}

#[test]
fn new_numbers_decisions_after_the_highest() {
    let repo = TestRepo::new("new-number");
    repo.write(
        "docs/decisions/0007-use-one-queue.md",
        "# 0007 Use one queue\n",
    );
    repo.write("docs/decisions/0003-old.md", "# 0003 Old\n");
    repo.write("docs/decisions/README.md", "# Decisions\n");
    let before = jiff::Zoned::now().date().to_string();
    let (code, out, err) = driftmark(&repo.dir, &["new", "decision", "Use", "two", "queues"]);
    let after = jiff::Zoned::now().date().to_string();
    assert_eq!(
        (code, out.as_str(), err.as_str()),
        (0, "docs/decisions/0008-use-two-queues.md\n", "")
    );
    let text = read(&repo.dir, "docs/decisions/0008-use-two-queues.md");
    assert!(text.contains("\n# 0008 Use two queues\n"), "{text}");
    let date = text.lines().nth(2).expect("a date line");
    assert!(
        date == format!("date: {before}") || date == format!("date: {after}"),
        "{date}"
    );
    let (_, out, _) = driftmark(&repo.dir, &["new", "decision", "Again"]);
    assert_eq!(out, "docs/decisions/0009-again.md\n");
}

#[test]
fn new_works_from_a_subfolder_and_without_the_layout() {
    let repo = TestRepo::new("new-sub");
    repo.write("src/main.txt", "x\n");
    let (code, out, _) = driftmark(&repo.dir.join("src"), &["new", "feature", "Export to PDF"]);
    assert_eq!(
        (code, out.as_str()),
        (0, "docs/features/export-to-pdf.md\n")
    );
    assert!(repo.dir.join("docs/features/export-to-pdf.md").is_file());
    let (code, out, _) = driftmark(&repo.dir, &["new", "plan", "--", "--quiet", "mode"]);
    assert_eq!(
        (code, out.as_str()),
        (0, "docs/working/quiet-mode/plan.md\n")
    );
    assert!(read(&repo.dir, "docs/working/quiet-mode/plan.md").contains("\n# --quiet mode plan\n"));
}

#[test]
fn new_never_changes_a_file_that_exists() {
    let repo = TestRepo::new("new-keep");
    repo.write("docs/standards/error-messages.md", "# Ours\n");
    let (code, out, err) = driftmark(&repo.dir, &["new", "standard", "Error messages"]);
    assert_eq!((code, out.as_str()), (1, ""));
    assert_eq!(
        err,
        "driftmark new: docs/standards/error-messages.md already exists, so nothing was written; choose another title, or edit that file\n"
    );
    assert_eq!(
        read(&repo.dir, "docs/standards/error-messages.md"),
        "# Ours\n"
    );
}

#[test]
fn new_refuses_bad_arguments() {
    let repo = TestRepo::new("new-args");
    let cases: [(&[&str], &str); 6] = [
        (&["new"], "name a kind and a title"),
        (
            &["new", "memo", "X"],
            "there is no kind `memo`; use decision, standard, feature, plan or status",
        ),
        (
            &["new", "decision", "--force", "X"],
            "unknown option `--force`; put `--` before a title that starts with `--`",
        ),
        (
            &["new", "decision"],
            "the title is empty; write it after the kind, such as `driftmark new decision \"Keep one database per service\"`",
        ),
        (
            &["new", "standard", "Two\nlines"],
            "the title holds a line break or a tab; write it on one line, with spaces between the words",
        ),
        (
            &["new", "standard", "?!"],
            "the title has no letter or digit, so no file name can be made from it; use words in the title",
        ),
    ];
    let document = the_document();
    for (args, message) in cases {
        let (code, out, err) = driftmark(&repo.dir, args);
        assert_eq!((code, out.as_str()), (2, ""), "{args:?}");
        assert!(
            err.starts_with(&format!(
                "driftmark new: {message}\n\nusage: driftmark new <kind> <title>\n"
            )),
            "{err}"
        );
        assert!(document.contains(message), "the document lists: {message}");
    }
    let (code, out, _) = driftmark(&repo.dir, &["new", "--help"]);
    assert_eq!(code, 0);
    assert!(out.starts_with("usage: driftmark new <kind> <title>\n"));
    assert_eq!(files(&repo.dir), NONE);
}

#[test]
fn init_on_this_repository_reports_what_exists_and_writes_nothing() {
    let root = this_repository();
    let agents = read(&root, "AGENTS.md");
    let (code, out, err) = driftmark(&root, &["init", "--agent-file", "AGENTS.md"]);
    assert_eq!((code, err.as_str()), (0, ""));
    assert!(out.contains("\n  exists        docs/decisions/\n"), "{out}");
    assert!(out.contains("\n  exists        docs/features/\n"), "{out}");
    assert!(
        out.contains("\n  exists        driftmark.toml (it stays as it is)\n"),
        "{out}"
    );
    for line in out.lines() {
        if let Some(path) = line.strip_prefix("  would create  ") {
            assert!(!root.join(path).exists(), "{path} was created");
        }
    }
    assert_eq!(read(&root, "AGENTS.md"), agents);
}

#[test]
fn the_documents_rules_file_is_the_preset() {
    let document = the_document();
    assert_eq!(
        block_after(&document, "It is an ordinary rules file in the format of"),
        preset::RULES
    );
}

#[test]
fn the_documents_agent_block_is_the_one_init_writes() {
    let document = the_document();
    assert_eq!(
        block_after(
            &document,
            "`init` prints this block, and `--agent-file` adds it to a file:"
        ),
        AGENT_BLOCK
    );
    let new_file = driftmark_core::starter::agent_addition(None).expect("a new file");
    assert_eq!(
        block_after(&document, "so it passes the writing check:"),
        new_file.replace(AGENT_BLOCK, "")
    );
}

#[test]
fn the_documents_templates_are_what_new_writes() {
    let document = the_document();
    let examples = [
        (
            Kind::Decision,
            "Keep one database per service",
            "docs/decisions/0004-keep-one-database-per-service.md",
        ),
        (
            Kind::Standard,
            "Error messages",
            "docs/standards/error-messages.md",
        ),
        (
            Kind::Feature,
            "Export to PDF",
            "docs/features/export-to-pdf.md",
        ),
        (
            Kind::Plan,
            "Faster search",
            "docs/working/faster-search/plan.md",
        ),
        (
            Kind::Status,
            "Faster search",
            "docs/working/faster-search/status.md",
        ),
    ];
    for (kind, title, path) in examples {
        let made = new_document(kind, title, "2026-03-02", 4).expect("a good title");
        assert_eq!(made.path, path);
        let lead_in = format!(
            "`driftmark new {} \"{title}\"` writes `{path}`:",
            kind.name()
        );
        assert_eq!(
            block_after(&document, &lead_in),
            made.text,
            "{}",
            kind.name()
        );
    }
}

#[test]
fn the_documents_init_output_is_what_init_prints() {
    let document = the_document();
    let repo = TestRepo::new("init-doc");
    repo.write("driftmark.toml", "[lint]\nexclude = [\"vendor/**\"]\n");
    repo.write(
        "docs/decisions/0003-use-one-queue.md",
        "---\nstatus: accepted\ndate: 2026-01-05\n---\n\n# 0003 Use one queue\n\n> Short version: one queue for every job.\n",
    );
    let root = shown_root(&repo);
    let dry = block_after(
        &document,
        "and a `driftmark.toml`, `driftmark init` prints:",
    );
    let (code, out, _) = driftmark(&repo.dir, &["init"]);
    assert_eq!(code, 0);
    let out = out.replace(&root, "/home/sam/shop");
    assert!(out.starts_with(&dry), "{out}");
    assert_eq!(&out[dry.len()..], format!("\n{AGENT_BLOCK}"));
    let written = block_after(&document, "### With `--write`");
    let (code, out, _) = driftmark(&repo.dir, &["init", "--write"]);
    assert_eq!(code, 0);
    let out = out.replace(&root, "/home/sam/shop");
    assert!(out.starts_with(&written), "{out}");
    assert!(out[written.len()..].starts_with("\nAdd this block to AGENTS.md or CLAUDE.md"));
}
