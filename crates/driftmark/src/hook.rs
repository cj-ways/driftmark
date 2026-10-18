//! `driftmark hook install`: adds the lint to Claude Code as a `PostToolUse`
//! hook in the repository's `.claude/settings.json`.
//!
//! Claude Code runs the hook after every `Edit` and `Write`. The hook runs
//! `driftmark lint --hook claude-code`, which reads the event on standard
//! input and reports findings on standard error with exit code 2; Claude Code
//! shows that text to Claude next to the tool's result. See
//! `docs/features/lint.md` for the contract and its sources.

use std::ffi::OsString;
use std::io::Write;
use std::path::Path;

use serde_json::{Map, Value, json};

use crate::lint::find_root;

/// The command the hook runs.
pub const HOOK_COMMAND: &str = "driftmark lint --hook claude-code";

/// The tools whose edits the hook checks. `MultiEdit` is not in Claude Code's
/// current tool list; it stays for older versions, and a name that matches no
/// tool matches nothing.
pub const HOOK_MATCHER: &str = "Edit|Write|MultiEdit";

/// Seconds Claude Code waits for the hook before it cancels it. Its default
/// for a command hook is 600; a lint of one file takes milliseconds.
pub const HOOK_TIMEOUT: u64 = 30;

/// Where the settings live, relative to the repository root.
pub const SETTINGS_FILE: &str = ".claude/settings.json";

/// Exit code when the entry was added, or would be, or is already there.
const EXIT_DONE: u8 = 0;
/// Exit code when the settings file cannot take the entry.
const EXIT_REFUSED: u8 = 1;
/// Exit code for bad arguments.
const EXIT_USAGE: u8 = 2;

const USAGE: &str = "\
usage: driftmark hook install [--write]

Adds `driftmark lint` to Claude Code as a PostToolUse hook in this
repository's .claude/settings.json, so Claude sees the findings after each
edit of a Markdown file. Without --write it only prints the entry it would
add. It never removes or changes other settings.";

/// The matcher group the installer adds under `hooks.PostToolUse`.
#[must_use]
pub fn entry() -> Value {
    json!({
        "matcher": HOOK_MATCHER,
        "hooks": [
            {
                "type": "command",
                "command": HOOK_COMMAND,
                "timeout": HOOK_TIMEOUT,
            }
        ]
    })
}

/// Runs `driftmark hook` with the arguments after `hook`, and returns the
/// exit code.
pub fn run(args: &[OsString], stdout: &mut dyn Write, stderr: &mut dyn Write) -> u8 {
    let words: Vec<Option<&str>> = args.iter().map(|arg| arg.to_str()).collect();
    let write = match words.as_slice() {
        [Some("install")] => false,
        [Some("install"), Some("--write")] => true,
        [Some("--help" | "-h")] | [Some("install"), Some("--help" | "-h")] => {
            let _ = writeln!(stdout, "{USAGE}");
            return EXIT_DONE;
        }
        _ => {
            let _ = writeln!(stderr, "driftmark hook: unknown arguments\n\n{USAGE}");
            return EXIT_USAGE;
        }
    };
    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(error) => {
            let _ = writeln!(
                stderr,
                "driftmark hook install: the current folder could not be read: {error}"
            );
            return EXIT_REFUSED;
        }
    };
    let root = match find_root(&cwd) {
        Ok(root) => root,
        Err(message) => {
            let _ = writeln!(stderr, "driftmark hook install: {message}");
            return EXIT_REFUSED;
        }
    };
    match install(&root, write) {
        Ok(message) => {
            let _ = writeln!(stdout, "{message}");
            EXIT_DONE
        }
        Err(message) => {
            let _ = writeln!(stderr, "driftmark hook install: {message}");
            EXIT_REFUSED
        }
    }
}

/// Adds the hook entry to the settings file of the repository at `root`, or
/// with `write` false only says what it would add. Returns what happened, as
/// a sentence for the user.
///
/// # Errors
///
/// A sentence for the user when the settings file is not valid JSON, has a
/// shape the entry cannot go into, or cannot be read or written. The file is
/// then left as it was.
pub fn install(root: &Path, write: bool) -> Result<String, String> {
    let path = root.join(SETTINGS_FILE);
    let original = match std::fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(format!("{SETTINGS_FILE} could not be read: {error}")),
    };
    let mut settings: Value = match &original {
        Some(text) => serde_json::from_str(text).map_err(|error| {
            format!("{SETTINGS_FILE} is not valid JSON ({error}), so nothing was added; fix the file, then run this again")
        })?,
        None => Value::Object(Map::new()),
    };
    let groups = post_tool_use(&mut settings)?;
    if groups.iter().any(holds_the_hook) {
        return Ok(format!(
            "{SETTINGS_FILE} already runs `{HOOK_COMMAND}` after edits; nothing to add"
        ));
    }
    let shown = serde_json::to_string_pretty(&entry()).unwrap_or_default();
    if !write {
        return Ok(format!(
            "driftmark hook install would add this entry to `hooks.PostToolUse` in {SETTINGS_FILE}:\n\n{shown}\n\nRun `driftmark hook install --write` to add it."
        ));
    }
    groups.push(entry());
    save(&path, &settings, original.as_deref(), SETTINGS_FILE)?;
    Ok(format!(
        "Added `{HOOK_COMMAND}` as a PostToolUse hook in {SETTINGS_FILE}. Claude Code picks up the change by itself."
    ))
}

/// Writes a JSON settings file the way its original was written (see
/// [`render`]), making its folder when missing. The new file is written
/// beside the old one and then moved over it, so a crash never leaves half
/// a file. `name` is the file as the user knows it, for messages.
///
/// # Errors
///
/// A sentence when the folder or the file cannot be written.
pub(crate) fn save(
    path: &Path,
    settings: &Value,
    original: Option<&str>,
    name: &str,
) -> Result<(), String> {
    let text = render(settings, original)?;
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)
            .map_err(|error| format!("{} could not be made: {error}", folder.display()))?;
    }
    let temporary = path.with_extension("json.driftmark-new");
    std::fs::write(&temporary, text)
        .and_then(|()| std::fs::rename(&temporary, path))
        .map_err(|error| {
            let _ = std::fs::remove_file(&temporary);
            format!("{name} could not be written: {error}")
        })
}

/// The `hooks.PostToolUse` list of a settings object, made when missing.
fn post_tool_use(settings: &mut Value) -> Result<&mut Vec<Value>, String> {
    let shape = |what: &str| {
        format!("{what} in {SETTINGS_FILE}, so the hook has nowhere to go; nothing was added")
    };
    let settings = settings
        .as_object_mut()
        .ok_or_else(|| shape("the top level is not an object"))?;
    let hooks = settings
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| shape("`hooks` is not an object"))?;
    hooks
        .entry("PostToolUse")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(|| shape("`hooks.PostToolUse` is not a list"))
}

/// Whether a matcher group already runs the lint hook, whatever its matcher
/// or timeout. The command counts when it is `driftmark lint --hook
/// claude-code`, with the program named by any path.
fn holds_the_hook(group: &Value) -> bool {
    group["hooks"].as_array().is_some_and(|handlers| {
        handlers.iter().any(|handler| {
            handler["command"].as_str().is_some_and(|command| {
                let words: Vec<&str> = command.split_whitespace().collect();
                matches!(words.as_slice(), [program, "lint", "--hook", "claude-code"]
                    if Path::new(program.trim_matches(['"', '\''])).file_stem().is_some_and(|stem| stem == "driftmark"))
            })
        })
    })
}

/// The settings as text, written the way the original was: its indentation,
/// its line endings, and a final line ending only if it had one.
fn render(settings: &Value, original: Option<&str>) -> Result<String, String> {
    let indent = original
        .and_then(|text| {
            text.lines().skip(1).find_map(|line| {
                let trimmed = line.trim_start_matches([' ', '\t']);
                (trimmed.len() < line.len() && trimmed.starts_with(['"', '}', ']', '{', '[']))
                    .then(|| &line[..line.len() - trimmed.len()])
            })
        })
        .unwrap_or("  ");
    // JSON strings hold no raw line breaks, so the spaces that start a line
    // of pretty output are all indentation, two per level.
    let pretty = serde_json::to_string_pretty(settings)
        .map_err(|error| format!("the settings could not be written as JSON: {error}"))?;
    let mut text = String::with_capacity(pretty.len());
    for line in pretty.lines() {
        let content = line.trim_start_matches(' ');
        let levels = (line.len() - content.len()) / 2;
        text.push_str(&indent.repeat(levels));
        text.push_str(content);
        text.push('\n');
    }
    text.pop();
    if original.is_none_or(|text| text.ends_with('\n')) {
        text.push('\n');
    }
    if original.is_some_and(|text| text.contains("\r\n")) {
        text = text.replace('\n', "\r\n");
    }
    Ok(text)
}
