//! The Claude Code hook on small test repos: `driftmark hook install` and
//! the command it installs, `driftmark lint --hook claude-code`, fed the
//! event Claude Code sends.

mod support;

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::{Value, json};
use support::{TempDir, TestRepo};

/// Runs the program in `dir`; returns its exit code, output and errors.
fn driftmark(dir: &Path, args: &[&str], stdin: &str) -> (i32, String, String) {
    run(
        Command::new(env!("CARGO_BIN_EXE_driftmark")).args(args),
        dir,
        stdin,
    )
}

fn run(command: &mut Command, dir: &Path, stdin: &str) -> (i32, String, String) {
    let mut child = command
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

/// The `PostToolUse` event Claude Code sends after an edit, in the shape its
/// hooks reference documents.
fn event(file: &Path, cwd: &Path) -> String {
    json!({
        "session_id": "abc123",
        "transcript_path": "/tmp/transcript.jsonl",
        "cwd": cwd,
        "permission_mode": "default",
        "hook_event_name": "PostToolUse",
        "tool_name": "Edit",
        "tool_input": {
            "file_path": file,
            "old_string": "a",
            "new_string": "b",
            "replace_all": false
        },
        "tool_response": { "filePath": file, "success": true },
        "tool_use_id": "toolu_01ABC123"
    })
    .to_string()
}

const GOOD: &str = "# Good\n\n> Short version: a made-up document that follows every writing rule that driftmark checks, so it passes.\n\n## Part\n\nText.\n";

fn repo() -> TestRepo {
    let repo = TestRepo::new("hook");
    repo.write("docs/good.md", GOOD);
    repo.write(
        "docs/bad.md",
        "# Bad\n\nSummary.\n\n### Skipped\n\n```sh\nls\n",
    );
    repo.write("code.rs", "fn main() {}\n");
    repo
}

#[test]
fn the_hook_reports_findings_to_claude_with_exit_code_two() {
    let repo = repo();
    let file = repo.dir.join("docs/bad.md");
    let (code, out, err) = driftmark(
        &repo.dir,
        &["lint", "--hook", "claude-code"],
        &event(&file, &repo.dir),
    );
    assert_eq!(out, "");
    assert_eq!(
        err,
        "\
driftmark lint found 2 problems in docs/bad.md. Fix them in the file:
docs/bad.md:5:1: warning heading-levels: this `###` heading skips a level after the `#` heading above it; make it `##`
docs/bad.md:7:1: error code-closed: this code block is never closed, so the rest of the file shows as code; add a closing ``` line where the code ends
"
    );
    assert_eq!(code, 2);
}

#[test]
fn the_hook_is_silent_when_it_has_nothing_to_say() {
    let repo = repo();
    let outside = TempDir::new("outside");
    std::fs::write(outside.path().join("loose.md"), "no title\n").expect("a file outside any repo");
    let quiet = |input: String| {
        let (code, out, err) = driftmark(&repo.dir, &["lint", "--hook", "claude-code"], &input);
        assert_eq!((code, out.as_str(), err.as_str()), (0, "", ""), "{input}");
    };
    quiet(event(&repo.dir.join("docs/good.md"), &repo.dir));
    quiet(event(&repo.dir.join("code.rs"), &repo.dir));
    quiet(event(&repo.dir.join("docs/deleted.md"), &repo.dir));
    quiet(event(&outside.path().join("loose.md"), outside.path()));
    quiet(json!({"hook_event_name": "PostToolUse", "tool_input": {}}).to_string());
}

#[test]
fn the_hook_skips_a_file_the_rules_file_excludes() {
    let repo = repo();
    repo.write("driftmark.toml", "[lint]\nexclude = [\"docs/bad.md\"]\n");
    let input = event(&repo.dir.join("docs/bad.md"), &repo.dir);
    let (code, out, err) = driftmark(&repo.dir, &["lint", "--hook", "claude-code"], &input);
    assert_eq!((code, out.as_str(), err.as_str()), (0, "", ""));
}

#[test]
fn a_relative_path_is_read_from_the_event_folder() {
    let repo = repo();
    let input = event(Path::new("docs/bad.md"), &repo.dir);
    let outside = TempDir::new("elsewhere");
    let (code, _, err) = driftmark(outside.path(), &["lint", "--hook", "claude-code"], &input);
    assert_eq!(code, 2);
    assert!(
        err.starts_with("driftmark lint found 2 problems in docs/bad.md."),
        "{err}"
    );
}

#[test]
fn a_long_report_is_cut_to_fifty_findings() {
    let repo = repo();
    let mut text = String::from("# Many\n\nSummary.\n\n");
    for _ in 0..60 {
        text.push_str("<div>x</div>\n\n");
    }
    repo.write("docs/many.md", &text);
    let (code, _, err) = driftmark(
        &repo.dir,
        &["lint", "--hook", "claude-code"],
        &event(&repo.dir.join("docs/many.md"), &repo.dir),
    );
    assert_eq!(code, 2);
    assert_eq!(err.lines().count(), 1 + 50 + 1, "{err}");
    assert!(
        err.ends_with("…and 10 more; run `driftmark lint docs/many.md` to see them all.\n"),
        "{err}"
    );
}

#[test]
fn a_broken_rules_file_or_input_is_reported() {
    let repo = repo();
    repo.write("driftmark.toml", "[lint]\nmax-file-size = 0\n");
    let (code, _, err) = driftmark(
        &repo.dir,
        &["lint", "--hook", "claude-code"],
        &event(&repo.dir.join("docs/good.md"), &repo.dir),
    );
    assert_eq!(code, 2, "Claude sees it and can fix the rules file");
    assert!(err.starts_with("driftmark lint could not check docs/good.md: driftmark.toml: the rules file could not be read"), "{err}");
    let (code, _, err) = driftmark(&repo.dir, &["lint", "--hook", "claude-code"], "not json");
    assert_eq!(code, 1, "a non-blocking error the user sees");
    assert!(err.contains("the hook input is not JSON"), "{err}");
}

fn settings(repo: &TestRepo) -> String {
    std::fs::read_to_string(repo.dir.join(".claude/settings.json"))
        .expect("the settings file exists")
}

const INSTALLED: &str = r#"{
  "hooks": {
    "PostToolUse": [
      {
        "matcher": "Edit|Write|MultiEdit",
        "hooks": [
          {
            "type": "command",
            "command": "driftmark lint --hook claude-code",
            "timeout": 30
          }
        ]
      }
    ]
  }
}
"#;

#[test]
fn install_prints_the_entry_and_writes_it_only_when_asked() {
    let repo = repo();
    let (code, out, _) = driftmark(&repo.dir, &["hook", "install"], "");
    assert_eq!(code, 0);
    assert!(out.starts_with("driftmark hook install would add this entry to `hooks.PostToolUse` in .claude/settings.json:"), "{out}");
    assert!(
        out.contains("\"command\": \"driftmark lint --hook claude-code\""),
        "{out}"
    );
    assert!(
        !repo.dir.join(".claude").exists(),
        "a dry run writes nothing"
    );
    let (code, out, _) = driftmark(&repo.dir.join("docs"), &["hook", "install", "--write"], "");
    assert_eq!(code, 0);
    assert!(
        out.starts_with("Added `driftmark lint --hook claude-code`"),
        "{out}"
    );
    assert_eq!(settings(&repo), INSTALLED);
}

#[test]
fn installing_twice_adds_nothing() {
    let repo = repo();
    assert_eq!(
        driftmark(&repo.dir, &["hook", "install", "--write"], "").0,
        0
    );
    let before = settings(&repo);
    for args in [
        &["hook", "install", "--write"][..],
        &["hook", "install"][..],
    ] {
        let (code, out, _) = driftmark(&repo.dir, args, "");
        assert_eq!(code, 0);
        assert_eq!(
            out,
            ".claude/settings.json already runs `driftmark lint --hook claude-code` after edits; nothing to add\n"
        );
    }
    assert_eq!(settings(&repo), before);
    repo.write(
        ".claude/settings.json",
        r#"{"hooks": {"PostToolUse": [{"matcher": "Write", "hooks": [{"type": "command", "command": "\"/opt/bin/driftmark\" lint --hook claude-code"}]}]}}"#,
    );
    let (_, out, _) = driftmark(&repo.dir, &["hook", "install", "--write"], "");
    assert!(
        out.contains("already runs"),
        "a command with a path counts too: {out}"
    );
}

#[test]
fn install_keeps_every_other_setting_and_the_file_style() {
    let repo = repo();
    let original = "{\r\n    \"model\": \"opus\",\r\n    \"permissions\": {\r\n        \"allow\": [\"Bash(git status)\"]\r\n    },\r\n    \"hooks\": {\r\n        \"PostToolUse\": [\r\n            {\r\n                \"matcher\": \"Bash\",\r\n                \"hooks\": [{\"type\": \"command\", \"command\": \"echo ran\"}]\r\n            }\r\n        ],\r\n        \"Stop\": []\r\n    },\r\n    \"env\": {\"Z\": \"1\", \"A\": \"2\"}\r\n}";
    repo.write(".claude/settings.json", original);
    let (code, _, err) = driftmark(&repo.dir, &["hook", "install", "--write"], "");
    assert_eq!(code, 0, "{err}");
    let written = settings(&repo);
    assert!(
        written.contains("\r\n    \"model\": \"opus\",\r\n"),
        "indentation and line endings kept: {written}"
    );
    assert!(!written.ends_with('\n'), "no final line ending was added");
    let before: Value = serde_json::from_str(original).expect("JSON");
    let after: Value = serde_json::from_str(&written).expect("JSON");
    let keys = |value: &Value| {
        value
            .as_object()
            .map(|map| map.keys().cloned().collect::<Vec<_>>())
    };
    assert_eq!(
        keys(&after),
        keys(&before),
        "the same keys in the same order"
    );
    assert_eq!(
        keys(&after["env"]),
        Some(vec!["Z".to_owned(), "A".to_owned()])
    );
    for key in ["model", "permissions", "env"] {
        assert_eq!(after[key], before[key], "{key}");
    }
    assert_eq!(after["hooks"]["Stop"], json!([]));
    let groups = after["hooks"]["PostToolUse"].as_array().expect("a list");
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0], before["hooks"]["PostToolUse"][0]);
    assert_eq!(
        groups[1]["hooks"][0]["command"],
        "driftmark lint --hook claude-code"
    );
}

#[test]
fn install_refuses_a_file_it_cannot_add_to() {
    let repo = repo();
    for text in [
        "{ \"hooks\": { ",
        "[1, 2]",
        "{\"hooks\": \"none\"}",
        "{\"hooks\": {\"PostToolUse\": {}}}",
    ] {
        repo.write(".claude/settings.json", text);
        for args in [
            &["hook", "install", "--write"][..],
            &["hook", "install"][..],
        ] {
            let (code, out, err) = driftmark(&repo.dir, args, "");
            assert_eq!(code, 1, "{text}");
            assert_eq!(out, "");
            assert!(err.contains("nothing was added"), "{text}: {err}");
        }
        assert_eq!(settings(&repo), text, "the file is left as it was");
    }
    repo.write(".claude/settings.json", "{ \"hooks\": { ");
    let (_, _, err) = driftmark(&repo.dir, &["hook", "install", "--write"], "");
    assert!(
        err.contains(".claude/settings.json is not valid JSON"),
        "{err}"
    );
}

#[test]
fn install_needs_a_repo_and_known_arguments() {
    let outside = TempDir::new("no-repo");
    let (code, _, err) = driftmark(outside.path(), &["hook", "install"], "");
    assert_eq!(code, 1);
    assert!(err.contains("run it inside a git repository"), "{err}");
    let repo = repo();
    assert_eq!(driftmark(&repo.dir, &["hook", "install", "--now"], "").0, 2);
    assert_eq!(driftmark(&repo.dir, &["hook"], "").0, 2);
    let (code, out, _) = driftmark(&repo.dir, &["hook", "install", "--help"], "");
    assert_eq!(code, 0);
    assert!(out.starts_with("usage: driftmark hook install"));
}

/// Runs the installed hook the way Claude Code does: its command through
/// `sh -c`, with the event on standard input and the program on `PATH`.
#[cfg(unix)]
#[test]
fn the_installed_command_works_as_claude_code_runs_it() {
    let repo = repo();
    assert_eq!(
        driftmark(&repo.dir, &["hook", "install", "--write"], "").0,
        0
    );
    let installed: Value = serde_json::from_str(&settings(&repo)).expect("JSON");
    let handler = &installed["hooks"]["PostToolUse"][0]["hooks"][0];
    let command = handler["command"].as_str().expect("a command");
    let program = Path::new(env!("CARGO_BIN_EXE_driftmark"));
    let path = format!(
        "{}:{}",
        program.parent().expect("a folder").display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let input = event(&repo.dir.join("docs/bad.md"), &repo.dir);
    let (code, _, err) = run(
        Command::new("sh").args(["-c", command]).env("PATH", &path),
        &repo.dir,
        &input,
    );
    assert_eq!(code, 2);
    assert!(err.contains("docs/bad.md:7:1: error code-closed"), "{err}");
    let input = event(&repo.dir.join("docs/good.md"), &repo.dir);
    let (code, _, err) = run(
        Command::new("sh").args(["-c", command]).env("PATH", &path),
        &repo.dir,
        &input,
    );
    assert_eq!((code, err.as_str()), (0, ""));
}
