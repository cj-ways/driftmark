//! `driftmark diff` run as a program on small test repos: the ranges it
//! reads, the rules file it uses, its summary, its JSON and its exit codes.

mod support;

use std::path::Path;
use std::process::Command;

use support::{DAY, T0, TempDir, TestRepo};

/// Runs the program in `dir` and returns its exit code, standard output and
/// standard error. Standard output is a pipe, not a terminal.
fn driftmark(dir: &Path, args: &[&str]) -> (i32, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_driftmark"))
        .args(args)
        .current_dir(dir)
        .env_remove("NO_COLOR")
        .output()
        .expect("the program runs");
    (
        output.status.code().expect("an exit code"),
        String::from_utf8(output.stdout).expect("UTF-8 output"),
        String::from_utf8(output.stderr).expect("UTF-8 errors"),
    )
}

/// The made-up decision record of `docs/features/engine/change-list.md`,
/// before and after.
const DECISION_BEFORE: &str = "---\nstatus: proposed\n---\n\n# Cache rendered pages\n\n## Decision\n\nKeep each rendered page in the cache for 60 minutes.\n\n## Notes\n\nThe cache lives in memory.\n";
const DECISION_AFTER: &str = "---\nstatus: accepted\n---\n\n# Cache rendered pages\n\n## Decision\n\nKeep each rendered page in the cache for 90 minutes.\n\nNever cache a page with open notes.\n";

/// Decision records keep their status in front matter, and `## Decision`
/// opens a decision section.
const RULES: &str = "[[type]]\nname = \"decision\"\npaths = [\"docs/decisions/*.md\"]\nstatus = { front-matter = \"status\" }\ncurrent = [\"accepted\"]\nother-statuses = [\"proposed\"]\ndecision-headings = [\"Decision\"]\n";

/// A repo with these commits:
///
/// - `main`: the first version, with no rules file;
/// - `feature`, one day later: the decision record changes, a guide is
///   reworded, a page is deleted, a page is added, `README.md` moves to
///   `docs/readme.md`, and the rules file appears;
/// - `broken`, on `feature`: a rules file with a typo;
/// - `latin`, on `main`: a Markdown file that is not UTF-8.
///
/// `feature` is checked out at the end.
struct Fixture {
    repo: TestRepo,
    main: String,
    feature: String,
    broken: String,
}

fn repo() -> Fixture {
    let repo = TestRepo::new("diff");
    repo.write("docs/decisions/0002-cache.md", DECISION_BEFORE);
    repo.write("docs/guide.md", "# Guide\n\nRead the setup first.\n");
    repo.write("docs/old.md", "# Old\n\nThis page goes away.\n");
    repo.write("README.md", "# Readme\n");
    repo.write("notes.txt", "not Markdown\n");
    let main = repo.commit("first", T0);
    repo.git(&["checkout", "-q", "-b", "feature"]);
    repo.write("docs/decisions/0002-cache.md", DECISION_AFTER);
    repo.write("docs/guide.md", "# Guide\n\nRead the whole setup first.\n");
    repo.remove("docs/old.md");
    repo.write("docs/new.md", "# New\n\nVersion 2 adds export.\n");
    repo.remove("README.md");
    repo.write("docs/readme.md", "# Readme\n");
    repo.write("driftmark.toml", RULES);
    let feature = repo.commit("second", T0 + DAY);
    repo.git(&["checkout", "-q", "-b", "broken"]);
    repo.write("driftmark.toml", "[[type]]\nname = \"x\"\nstauts = 1\n");
    let broken = repo.commit("broken rules", T0 + 2 * DAY);
    repo.git(&["checkout", "-q", "main"]);
    repo.git(&["checkout", "-q", "-b", "latin"]);
    repo.write("docs/latin1.md", b"# Caf\xe9\n");
    repo.commit("not UTF-8", T0 + 3 * DAY);
    repo.git(&["checkout", "-q", "feature"]);
    Fixture {
        repo,
        main,
        feature,
        broken,
    }
}

fn short(id: &str) -> &str {
    &id[..12]
}

#[test]
fn the_summary_ranks_documents_by_their_highest_level() {
    let r = repo();
    let (code, out, err) = driftmark(&r.repo.dir, &["diff", "main..feature"]);
    assert_eq!(err, "");
    assert_eq!(
        out,
        format!(
            "\
main..feature: from {} to {}

level       changes  document
decision     3 of 5  docs/decisions/0002-cache.md (status proposed -> accepted)
fact         2 of 2  docs/new.md (new)
fact         1 of 2  docs/old.md (deleted)
fact         1 of 1  README.md (deleted)
fact         1 of 1  docs/readme.md (new)
wording      1 of 1  docs/guide.md

6 documents changed, 12 changes
By level: 3 decision, 6 fact, 3 wording
By kind: 3 edited, 4 added, 5 removed
",
            short(&r.main),
            short(&r.feature)
        )
    );
    assert_eq!(code, 0);
    assert!(!out.contains('\x1b'), "no colours when not a terminal");
}

#[test]
fn the_rules_file_comes_from_the_head_side() {
    let r = repo();
    // Backwards, the head is `main`, which has no rules file: the decision
    // record has no status and no decision section, so `never` going away
    // ranks it as a rule change.
    let (code, out, _) = driftmark(&r.repo.dir, &["diff", "feature..main"]);
    assert_eq!(code, 0);
    let line = out
        .lines()
        .find(|line| line.contains("0002-cache.md"))
        .expect("the decision record is listed");
    assert_eq!(line, "rule         1 of 5  docs/decisions/0002-cache.md");

    // A broken rules file in the working tree does not matter to a range
    // whose head is a commit, and is an error for `auto`.
    r.repo
        .write("driftmark.toml", "[[type]]\nname = \"x\"\npaths = []\n");
    let (code, _, _) = driftmark(&r.repo.dir, &["diff", "main..feature"]);
    assert_eq!(code, 0);
    let (code, out, err) = driftmark(&r.repo.dir, &["diff", "auto"]);
    assert_eq!(code, 1);
    assert_eq!(out, "");
    assert_eq!(
        err,
        "driftmark diff: driftmark.toml in the working tree, line 1: the rules file's type 1 (\"x\"): it lists no paths\n"
    );

    // A broken committed rules file is named with its commit and line.
    let (code, out, err) = driftmark(&r.repo.dir, &["diff", "main..broken"]);
    assert_eq!(code, 1);
    assert_eq!(out, "");
    let wanted = format!(
        "driftmark diff: driftmark.toml at commit {}: the rules file could not be read: TOML parse error at line 3, column 1",
        short(&r.broken)
    );
    assert!(err.starts_with(&wanted), "{err}");
}

#[test]
fn json_holds_the_whole_change_list() {
    let r = repo();
    let (code, out, err) = driftmark(&r.repo.dir, &["diff", "--json", "main...feature"]);
    assert_eq!((code, err.as_str()), (0, ""));
    assert!(out.ends_with("}\n") && out.lines().count() == 1, "one line");
    let list: serde_json::Value = serde_json::from_str(&out).expect("JSON");
    assert_eq!(list["schema_version"], 1);
    assert_eq!(list["range"]["base"], r.main.as_str());
    assert_eq!(list["range"]["head"], r.feature.as_str());
    assert_eq!(list["range"]["notes"], serde_json::json!([]));
    assert_eq!(list["totals"]["changes"], 12);
    assert_eq!(list["totals"]["highest"], "decision");
    let documents = list["documents"].as_array().expect("documents");
    let paths: Vec<&str> = documents
        .iter()
        .map(|d| d["path"].as_str().expect("a path"))
        .collect();
    assert_eq!(
        paths,
        [
            "docs/decisions/0002-cache.md",
            "docs/new.md",
            "docs/old.md",
            "README.md",
            "docs/readme.md",
            "docs/guide.md"
        ]
    );
    let decision = &documents[0];
    assert_eq!(decision["file"], "modified");
    assert_eq!(
        decision["status"],
        serde_json::json!({"old": "proposed", "new": "accepted"})
    );
    assert_eq!(decision["changes"][1]["edit"]["view"], "inline");
    assert_eq!(decision["unchanged"], serde_json::json!([[1, 1], [2, 2]]));

    // One thread gives the same bytes as many.
    let (_, single, _) = driftmark(
        &r.repo.dir,
        &["diff", "main...feature", "--json", "--threads", "1"],
    );
    assert_eq!(single, out);
}

#[test]
fn auto_compares_the_working_tree_with_where_the_branch_started() {
    let r = repo();
    r.repo.write(
        "docs/guide.md",
        "# Guide\n\nRead the whole setup first, then build.\n",
    );
    r.repo
        .write("docs/draft.md", "# Draft\n\nNot added to git yet.\n");
    let (code, out, err) = driftmark(&r.repo.dir, &["diff", "auto"]);
    assert_eq!((code, err.as_str()), (0, ""));
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines[0],
        format!("auto: from {} to the working tree", short(&r.main))
    );
    assert_eq!(lines[1], "Note: compared with `main`");
    assert!(
        lines.contains(&"fact         1 of 2  docs/draft.md (new)"),
        "{out}"
    );
    assert!(
        lines.contains(&"wording      1 of 1  docs/guide.md"),
        "{out}"
    );
    assert!(lines.contains(&"7 documents changed, 14 changes"), "{out}");
    let (_, json, _) = driftmark(&r.repo.dir, &["diff", "auto", "--json"]);
    let list: serde_json::Value = serde_json::from_str(&json).expect("JSON");
    assert_eq!(list["range"]["head"], "worktree");
    assert_eq!(
        list["range"]["notes"],
        serde_json::json!(["compared with `main`"])
    );
}

#[test]
fn since_and_folders_narrow_the_range() {
    let r = repo();
    let (code, out, err) = driftmark(
        &r.repo.dir,
        &["diff", "--since", "2026-01-02", "feature", "--", "docs"],
    );
    assert_eq!((code, err.as_str()), (0, ""));
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines[..2],
        [
            format!(
                "--since 2026-01-02 feature: from {} to {}",
                short(&r.main),
                short(&r.feature)
            )
            .as_str(),
            "Only in: docs"
        ]
    );
    assert!(!out.contains("README.md"), "outside `docs`: {out}");
    assert!(out.contains("5 documents changed, 11 changes"), "{out}");

    // Before the first commit, every file is new.
    let (code, out, _) = driftmark(&r.repo.dir, &["diff", "--since=2025-12-31"]);
    assert_eq!(code, 0);
    assert!(
        out.starts_with(&format!(
            "--since 2025-12-31: from before the first commit to {}\n",
            short(&r.feature)
        )),
        "{out}"
    );
    assert!(out.contains("4 documents changed"), "{out}");
}

#[test]
fn timings_go_to_standard_error_and_count_the_shared_blob_once() {
    let r = repo();
    let (code, plain, _) = driftmark(&r.repo.dir, &["diff", "main..feature"]);
    assert_eq!(code, 0);
    let (code, out, err) = driftmark(
        &r.repo.dir,
        &["diff", "main..feature", "--timings", "--threads=2"],
    );
    assert_eq!(code, 0);
    assert_eq!(out, plain, "the summary does not change");
    let lines: Vec<&str> = err.lines().collect();
    assert_eq!(
        lines[0],
        "driftmark diff: timings in milliseconds, on 2 threads"
    );
    for (line, step) in lines[1..].iter().zip([
        "  open the repository",
        "  resolve the range",
        "  read the rules file",
        "  list the changed files",
        "  read, parse and compare",
        "    reading, all threads",
        "    parsing, all threads",
        "    comparing, all threads",
        "  sort the documents",
        "  write the output",
        "  total",
    ]) {
        assert!(line.starts_with(step), "{line:?} is not {step:?}");
        let number = line[step.len()..].trim();
        assert!(number.parse::<f64>().is_ok(), "{line:?}");
    }
    // `README.md` moved to `docs/readme.md` with the same bytes, so its blob
    // is parsed once for both files.
    assert_eq!(
        lines[12],
        "driftmark diff: 6 changed files, 8 versions, 7 parsed"
    );
}

#[test]
fn a_file_that_is_not_utf8_is_skipped_and_named() {
    let r = repo();
    let (code, out, err) = driftmark(&r.repo.dir, &["diff", "main..latin"]);
    assert_eq!(code, 0);
    assert_eq!(
        err,
        "driftmark diff: skipped docs/latin1.md: the new version cannot be compared: the file is not UTF-8 text: byte 5, on line 1, is not valid UTF-8\n"
    );
    assert!(
        out.ends_with(
            "\n\n0 documents changed, 0 changes; 1 file skipped, named on standard error\n"
        ),
        "{out}"
    );
}

#[test]
fn nothing_changed_is_not_an_error() {
    let r = repo();
    let (code, out, err) = driftmark(&r.repo.dir, &["diff", "main..main"]);
    assert_eq!((code, err.as_str()), (0, ""));
    let base = short(&r.main);
    assert_eq!(
        out,
        format!("main..main: from {base} to {base}\n\nNo Markdown files changed.\n")
    );
    let (code, out, _) = driftmark(&r.repo.dir, &["diff", "main..main", "--json"]);
    assert_eq!(code, 0);
    let list: serde_json::Value = serde_json::from_str(&out).expect("JSON");
    assert_eq!(list["documents"], serde_json::json!([]));
}

#[test]
fn the_repository_is_found_from_a_subfolder_or_named() {
    let r = repo();
    let (code, inside, _) = driftmark(&r.repo.dir.join("docs"), &["diff", "main..feature"]);
    assert_eq!(code, 0);
    let elsewhere = TempDir::new("elsewhere");
    let repo_path = r.repo.dir.to_string_lossy().into_owned();
    let (code, named, _) = driftmark(
        elsewhere.path(),
        &["diff", "--repo", &repo_path, "main..feature"],
    );
    assert_eq!(code, 0);
    assert_eq!(inside, named);
    assert!(
        named.contains("README.md (deleted)"),
        "paths start at the root"
    );
}

#[test]
fn exit_code_2_means_the_command_line_is_wrong() {
    let r = repo();
    for (args, wanted) in [
        (&["diff"][..], "give a range"),
        (&["diff", "main"], "`main` is not a range"),
        (&["diff", "main..feature", "docs"], "put folders after `--`"),
        (
            &["diff", "--nope", "main..feature"],
            "unknown option `--nope`",
        ),
        (
            &["diff", "main..feature", "--threads", "0"],
            "a whole number from 1",
        ),
        (
            &["diff", "--since", "yesterday"],
            "`yesterday` is not a date",
        ),
        (
            &["diff", "main..feature", "--repo"],
            "`--repo` needs a value",
        ),
        (
            &["diff", "main..feature", "--", "../x"],
            "is not a folder inside the repo",
        ),
    ] {
        let (code, out, err) = driftmark(&r.repo.dir, args);
        assert_eq!(code, 2, "{args:?}: {err}");
        assert_eq!(out, "", "{args:?}");
        assert!(err.starts_with("driftmark diff: "), "{args:?}: {err}");
        assert!(err.contains(wanted), "{args:?}: {err}");
        assert!(err.contains("\n\nusage: driftmark diff"), "{args:?}: {err}");
    }
    let (code, out, _) = driftmark(&r.repo.dir, &["diff", "--help"]);
    assert_eq!(code, 0);
    assert!(out.starts_with("usage: driftmark diff"), "{out}");
    assert!(
        out.contains("Exit codes: 0 it ran, 1 it could not finish, 2"),
        "{out}"
    );
}

#[test]
fn exit_code_1_means_it_could_not_finish() {
    let r = repo();
    let fails = |dir: &Path, args: &[&str], wanted: &str| {
        let (code, out, err) = driftmark(dir, args);
        assert_eq!(code, 1, "{args:?}: {err}");
        assert_eq!(out, "", "{args:?}");
        assert!(err.starts_with("driftmark diff: "), "{args:?}: {err}");
        assert!(err.contains(wanted), "{args:?}: {err}");
        assert!(!err.contains("usage:"), "{args:?}: {err}");
    };
    fails(
        &r.repo.dir,
        &["diff", "main..nope"],
        "`nope` does not name a commit",
    );
    let other = TempDir::new("no-repo");
    fails(
        other.path(),
        &["diff", "main..feature"],
        "is not inside a git repository",
    );
    let missing = other.path().join("missing").to_string_lossy().into_owned();
    fails(
        &r.repo.dir,
        &["diff", "--repo", &missing, "main..feature"],
        "is not inside a git repository",
    );
    let unrelated = TestRepo::new("no-default");
    unrelated.write("a.md", "# A\n");
    unrelated.commit("one", T0);
    unrelated.git(&["branch", "-m", "trunk"]);
    fails(
        &unrelated.dir,
        &["diff", "auto"],
        "`auto` found no default branch",
    );
}

#[cfg(unix)]
#[test]
fn control_characters_in_names_cannot_reach_the_terminal() {
    let repo = TestRepo::new("names");
    repo.write("a.md", "# A\n");
    repo.commit("one", T0);
    repo.write("docs/new\nline\u{1b}[2J.md", "# Odd\n");
    let (code, out, _) = driftmark(&repo.dir, &["diff", "auto"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("  docs/new\\nline\\u{1b}[2J.md (new)\n"),
        "{out}"
    );
    assert!(!out.contains('\x1b'), "{out}");
}
