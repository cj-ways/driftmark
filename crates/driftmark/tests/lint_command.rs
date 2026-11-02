//! `driftmark lint` run as a program on small test repos: which files it
//! checks, what it prints, and its exit codes.

mod support;

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use support::{TempDir, TestRepo};

/// Runs the program in `dir` and returns its exit code, standard output and
/// standard error.
fn driftmark(dir: &Path, args: &[&str], stdin: &str) -> (i32, String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_driftmark"))
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the program starts");
    child
        .stdin
        .take()
        .expect("a stdin pipe")
        .write_all(stdin.as_bytes())
        .expect("stdin is written");
    let output = child.wait_with_output().expect("the program finishes");
    (
        output.status.code().expect("an exit code"),
        String::from_utf8(output.stdout).expect("UTF-8 output"),
        String::from_utf8(output.stderr).expect("UTF-8 errors"),
    )
}

const GOOD: &str = "# Good\n\n> Short version: a made-up document that follows every writing rule that driftmark checks, so it passes.\n\n## Part\n\nSee [the other part](#part).\n";
const BAD: &str = "Intro first.\n\n# Bad\n\nSummary.\n\n```\ncode\n```\n\n[gone](missing.md)\n";
const WARNS: &str = "# Warns\n\nSummary.\n\n```\nno language here\n```\n";

fn repo() -> TestRepo {
    let repo = TestRepo::new("lint");
    repo.write("docs/good.md", GOOD);
    repo.write("docs/bad.md", BAD);
    repo.write("README.md", GOOD.replace("# Good", "# Read me"));
    repo.write(".gitignore", "ignored.md\n");
    repo.write("ignored.md", BAD);
    repo.commit("docs", support::T0);
    repo.write("notes/new.md", WARNS);
    repo
}

#[test]
fn with_no_path_it_checks_every_markdown_file_git_would_track() {
    let repo = repo();
    let (code, out, err) = driftmark(&repo.dir, &["lint"], "");
    assert_eq!(err, "");
    assert_eq!(
        out,
        "\
docs/bad.md:1:1: error title: this comes before the `#` title, and only front matter may; move it below the title
docs/bad.md:7:1: warning code-language: this code block names no language; write it after the opening fence, such as ```sh, or ```text for plain text
docs/bad.md:11:1: error link: the link `missing.md` leads to `docs/missing.md`, which does not exist; fix the path, or create the file
notes/new.md:5:1: warning code-language: this code block names no language; write it after the opening fence, such as ```sh, or ```text for plain text
driftmark lint: 4 files checked, 2 errors and 2 warnings in 2 files
"
    );
    assert_eq!(code, 1);
}

#[test]
fn exclude_skips_files_in_a_walk_but_not_files_named_on_their_own() {
    let repo = repo();
    repo.write(
        "driftmark.toml",
        "[lint]\nexclude = [\"docs/bad.md\", \"notes/**\"]\n",
    );
    let (code, out, err) = driftmark(&repo.dir, &["lint"], "");
    assert_eq!(err, "");
    assert_eq!(out, "driftmark lint: 2 files checked, no problems found\n");
    assert_eq!(code, 0);
    let (_, out, _) = driftmark(&repo.dir, &["lint", "docs"], "");
    assert_eq!(out, "driftmark lint: 1 file checked, no problems found\n");
    let (code, out, _) = driftmark(&repo.dir, &["lint", "docs/bad.md"], "");
    assert!(
        out.ends_with("1 file checked, 2 errors and 1 warning in 1 file\n"),
        "a file named on its own is checked: {out}"
    );
    assert_eq!(code, 1);
    repo.write("driftmark.toml", "[lint]\nexclude = [\"a/**b\"]\n");
    let (code, out, err) = driftmark(&repo.dir, &["lint"], "");
    assert_eq!((code, out.as_str()), (2, ""));
    assert!(err.contains("`exclude` holds \"a/**b\""), "{err}");
}

#[test]
fn paths_name_files_or_folders_from_the_current_folder() {
    let repo = repo();
    let (code, out, _) = driftmark(&repo.dir, &["lint", "docs/good.md"], "");
    assert_eq!(out, "driftmark lint: 1 file checked, no problems found\n");
    assert_eq!(code, 0);
    let (code, out, _) = driftmark(&repo.dir, &["lint", "notes"], "");
    assert!(
        out.ends_with("1 file checked, 0 errors and 1 warning in 1 file\n"),
        "{out}"
    );
    assert_eq!(code, 0, "warnings alone do not fail");
    let (code, out, _) = driftmark(
        &repo.dir.join("docs"),
        &["lint", "bad.md", "good.md", "./bad.md"],
        "",
    );
    assert!(out.starts_with("docs/bad.md:1:1: error title"), "{out}");
    assert!(
        out.ends_with("2 files checked, 2 errors and 1 warning in 1 file\n"),
        "{out}"
    );
    assert_eq!(code, 1);
    let (_, out, _) = driftmark(&repo.dir, &["lint", "--", "docs"], "");
    assert!(
        out.ends_with("2 files checked, 2 errors and 1 warning in 1 file\n"),
        "{out}"
    );
    let (_, out, _) = driftmark(&repo.dir, &["lint", "ignored.md"], "");
    assert!(
        out.ends_with("1 file checked, 2 errors and 1 warning in 1 file\n"),
        "a named file is checked even when git ignores it: {out}"
    );
}

#[test]
fn json_output_lists_every_finding_with_its_file() {
    let repo = repo();
    let (code, out, _) = driftmark(&repo.dir, &["lint", "--json", "docs"], "");
    assert_eq!(code, 1);
    let value: serde_json::Value = serde_json::from_str(&out).expect("JSON");
    assert_eq!(value["files"], 2);
    assert_eq!(value["errors"], 2);
    assert_eq!(value["warnings"], 1);
    let first = &value["findings"][0];
    assert_eq!(first["path"], "docs/bad.md");
    assert_eq!(first["line"], 1);
    assert_eq!(first["column"], 1);
    assert_eq!(first["severity"], "error");
    assert_eq!(first["rule"], "title");
    assert!(
        first["message"]
            .as_str()
            .is_some_and(|m| m.contains("before the `#` title"))
    );
    let (code, out, _) = driftmark(&repo.dir, &["lint", "--json", "docs/good.md"], "");
    assert_eq!(code, 0);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&out).expect("JSON"),
        serde_json::json!({"files": 1, "errors": 0, "warnings": 0, "findings": []})
    );
}

#[test]
fn the_rules_file_changes_severities() {
    let repo = repo();
    repo.write(
        "driftmark.toml",
        "[lint.rules]\ntitle = \"warning\"\nlink = \"off\"\ncode-language = \"error\"\n",
    );
    let (code, out, _) = driftmark(&repo.dir, &["lint", "docs/bad.md"], "");
    assert_eq!(
        out,
        "\
docs/bad.md:1:1: warning title: this comes before the `#` title, and only front matter may; move it below the title
docs/bad.md:7:1: error code-language: this code block names no language; write it after the opening fence, such as ```sh, or ```text for plain text
driftmark lint: 1 file checked, 1 error and 1 warning in 1 file
"
    );
    assert_eq!(code, 1);
}

#[test]
fn it_cannot_run_without_a_repo_good_arguments_and_a_valid_rules_file() {
    let repo = repo();
    let usage = |args: &[&str], wanted: &str| {
        let (code, out, err) = driftmark(&repo.dir, args, "");
        assert_eq!(code, 2, "{args:?}: {out}{err}");
        assert!(err.contains(wanted), "{args:?}: {err}");
        assert_eq!(out, "");
    };
    usage(&["lint", "--nope"], "unknown option `--nope`");
    usage(
        &["lint", "missing.md"],
        "there is no file or folder `missing.md`",
    );
    usage(&["lint", ".."], "is outside the repository");
    usage(
        &["lint", "--hook", "other"],
        "knows only `claude-code`, not `other`",
    );
    usage(&["lint", "--hook"], "knows only `claude-code`, not ``");
    usage(
        &["lint", "--hook", "claude-code", "docs"],
        "takes no path and no `--json`",
    );
    repo.write("driftmark.toml", "[lint.rules]\ntitel = \"off\"\n");
    usage(
        &["lint"],
        "driftmark.toml: the rules file could not be read",
    );
    let outside = TempDir::new("no-repo");
    let (code, _, err) = driftmark(outside.path(), &["lint"], "");
    assert_eq!(code, 2);
    assert!(err.contains("run it inside a git repository"), "{err}");
    let (code, out, _) = driftmark(&repo.dir, &["lint", "--help"], "");
    assert_eq!(code, 0);
    assert!(out.starts_with("usage: driftmark lint"));
}

#[test]
fn the_program_names_its_commands() {
    let repo = repo();
    let (code, _, err) = driftmark(&repo.dir, &[], "");
    assert_eq!(code, 2);
    assert!(err.contains("lint "), "{err}");
    let (code, _, _) = driftmark(&repo.dir, &["nope"], "");
    assert_eq!(code, 2);
    let (code, out, _) = driftmark(&repo.dir, &["--help"], "");
    assert_eq!(code, 0);
    assert!(out.contains("hook install"), "{out}");
    let (code, out, _) = driftmark(&repo.dir, &["--version"], "");
    assert_eq!(code, 0);
    assert!(out.starts_with("driftmark "), "{out}");
}

/// The repo of [`repo`] with its new file committed on a branch `feature`
/// that also edits `docs/good.md`, removes `README.md`, and changes
/// `docs/bad.md`, which the rules file then excludes. Returns the repo and
/// the commit of `main`.
fn branched() -> (TestRepo, String) {
    let repo = repo();
    repo.write("driftmark.toml", "[lint]\nexclude = [\"docs/bad.md\"]\n");
    let main = repo.commit("rules", support::T0 + support::DAY);
    repo.git(&["checkout", "-q", "-b", "feature"]);
    repo.write("docs/good.md", format!("{GOOD}\nOne more line.\n"));
    repo.write("docs/bad.md", format!("{BAD}\nStill bad.\n"));
    repo.remove("README.md");
    repo.commit("feature", support::T0 + 2 * support::DAY);
    (repo, main)
}

#[test]
fn changed_checks_only_what_the_range_added_or_changed() {
    let (repo, _) = branched();
    let (code, out, err) = driftmark(&repo.dir, &["lint", "--changed", "main...feature"], "");
    assert_eq!(err, "");
    assert_eq!(
        out, "driftmark lint: 1 file checked, no problems found\n",
        "docs/good.md only: notes/new.md came before the range, README.md is gone, docs/bad.md is excluded"
    );
    assert_eq!(code, 0);
    let (code, out, _) = driftmark(&repo.dir, &["lint", "--changed=main~1...feature"], "");
    assert_eq!(
        out,
        "\
notes/new.md:5:1: warning code-language: this code block names no language; write it after the opening fence, such as ```sh, or ```text for plain text
driftmark lint: 2 files checked, 0 errors and 1 warning in 1 file
"
    );
    assert_eq!(code, 0);
}

#[test]
fn with_changed_paths_narrow_and_exclude_still_applies() {
    let (repo, _) = branched();
    let lint = |args: &[&str]| driftmark(&repo.dir, args, "").1;
    let range = "main~1...feature";
    assert_eq!(
        lint(&["lint", "--changed", range, "docs"]),
        "driftmark lint: 1 file checked, no problems found\n"
    );
    assert_eq!(
        lint(&["lint", "--changed", range, "notes/new.md", "docs/bad.md"]),
        "\
notes/new.md:5:1: warning code-language: this code block names no language; write it after the opening fence, such as ```sh, or ```text for plain text
driftmark lint: 1 file checked, 0 errors and 1 warning in 1 file
",
        "a file named on its own is still excluded with --changed"
    );
    assert_eq!(
        driftmark(
            &repo.dir.join("docs"),
            &["lint", "--changed", range, "."],
            ""
        )
        .1,
        "driftmark lint: 1 file checked, no problems found\n",
        "paths are read from the current folder"
    );
    assert_eq!(
        lint(&["lint", "--changed", "feature...feature"]),
        "driftmark lint: 0 files checked, no problems found\n"
    );
}

#[test]
fn changed_auto_reads_the_working_tree_against_the_default_branch() {
    let (repo, _) = branched();
    repo.write("docs/extra.md", WARNS);
    let (code, out, err) = driftmark(&repo.dir, &["lint", "--changed", "auto"], "");
    assert_eq!(err, "");
    assert_eq!(
        out,
        "\
docs/extra.md:5:1: warning code-language: this code block names no language; write it after the opening fence, such as ```sh, or ```text for plain text
driftmark lint: 2 files checked, 0 errors and 1 warning in 1 file
"
    );
    assert_eq!(code, 0);
}

#[test]
fn ci_formats_print_annotations_or_a_code_quality_report() {
    let (repo, _) = branched();
    let range = "main~1...feature";
    let (code, out, err) = driftmark(
        &repo.dir,
        &["lint", "--format", "github", "--changed", range, "notes"],
        "",
    );
    assert_eq!(err, "");
    assert_eq!(
        out,
        "\
::warning file=notes/new.md,line=5,col=1,title=driftmark lint%3A code-language::this code block names no language; write it after the opening fence, such as ```sh, or ```text for plain text
driftmark lint: 1 file checked, 0 errors and 1 warning in 1 file
"
    );
    assert_eq!(code, 0);
    let (code, out, _) = driftmark(&repo.dir, &["lint", "--format=gitlab", "docs/bad.md"], "");
    assert_eq!(code, 1, "an error still exits 1");
    let report: serde_json::Value = serde_json::from_str(&out).unwrap();
    let issues = report.as_array().unwrap();
    assert_eq!(issues.len(), 3);
    assert_eq!(issues[0]["check_name"], "title");
    assert_eq!(issues[0]["severity"], "major");
    assert_eq!(issues[0]["location"]["path"], "docs/bad.md");
    assert_eq!(issues[0]["location"]["lines"]["begin"], 1);
    assert_eq!(issues[1]["severity"], "minor");
    let (_, out, _) = driftmark(&repo.dir, &["lint", "--format", "json", "docs/good.md"], "");
    assert!(out.starts_with("{\"files\":1,"), "{out}");
}

#[test]
fn changed_and_format_mistakes_exit_2() {
    let (repo, _) = branched();
    let usage = |args: &[&str], wanted: &str| {
        let (code, out, err) = driftmark(&repo.dir, args, "");
        assert_eq!(code, 2, "{args:?}: {out}{err}");
        assert!(err.contains(wanted), "{args:?}: {err}");
        assert_eq!(out, "");
    };
    usage(
        &["lint", "--format", "sarif"],
        "`--format` takes `text`, `json`, `github` or `gitlab`, not `sarif`",
    );
    usage(
        &["lint", "--changed", "main"],
        "`--changed` takes a range such as `main...feature`",
    );
    usage(&["lint", "--changed", "main...nope"], "`nope`");
    usage(
        &["lint", "--changed", "main...feature", "missing"],
        "there is no file or folder `missing`",
    );
    usage(
        &["lint", "--hook", "claude-code", "--format", "github"],
        "takes no path and no `--json`, `--format` or `--changed`",
    );
}
