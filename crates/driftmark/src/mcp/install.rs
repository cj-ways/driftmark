//! `driftmark mcp install`: the entry that makes Claude Code start the
//! connector, in the repository's `.mcp.json`, and on request a line in the
//! agent instruction file that tells agents about the reader's notes.
//!
//! Claude Code reads a project's servers from `.mcp.json` at its root and
//! asks once before it starts them. Without `--write` this prints the entry
//! and the `claude mcp add` command that adds the same; with it, the entry
//! is added and every other entry, key and the file's own indentation stay,
//! the way `driftmark hook install` treats `.claude/settings.json`.
//!
//! An agent learns that notes wait for it from the `notes_list` tool, but
//! only when it thinks to call it. `--notes-line` adds one line to
//! `AGENTS.md`, or `CLAUDE.md` when there is no `AGENTS.md`, that tells it
//! to. The file is a shared, committed file, so the line is added only when
//! asked for, and only at the end, with every other byte kept.

use std::io::Write;
use std::path::Path;

use serde_json::{Map, Value, json};

use super::{EXIT_ERROR, EXIT_OK};
use crate::hook::save;
use crate::lint::find_root;

/// Where the project's MCP servers live, relative to the repository root.
pub const MCP_FILE: &str = ".mcp.json";

/// The name the entry gets in `mcpServers`.
pub const SERVER_NAME: &str = "driftmark";

/// The command Claude Code's own CLI runs to add the same entry.
pub const CLAUDE_COMMAND: &str =
    "claude mcp add --transport stdio --scope project driftmark -- driftmark mcp";

/// The line `--notes-line` adds to the agent instruction file. Its words
/// name the tools, so a file that already names `notes_list` gets nothing.
pub const NOTES_LINE: &str = "Reader's notes: the reader leaves notes on these documents with driftmark. Before you edit a document, call the driftmark tool `notes_list`; answer open notes with `note_reply`, change a document only for a note the reader decided, and record each commit with `note_mark_applied`.";

/// The agent instruction files the line goes to, in order: the first that
/// exists at the repository root. `AGENTS.md` comes first because every
/// agent reads it, and Claude Code's `CLAUDE.md` often imports it.
pub const INSTRUCTION_FILES: [&str; 2] = ["AGENTS.md", "CLAUDE.md"];

/// The server entry: `driftmark mcp`, started in the folder Claude Code
/// runs in, found on the `PATH` Claude Code sees.
#[must_use]
pub fn entry() -> Value {
    json!({
        "type": "stdio",
        "command": "driftmark",
        "args": ["mcp"],
    })
}

/// Runs `driftmark mcp install`, and returns the exit code. `line` adds
/// the line about notes too.
pub(super) fn run(write: bool, line: bool, stdout: &mut dyn Write, stderr: &mut dyn Write) -> u8 {
    let outcome = std::env::current_dir()
        .map_err(|error| format!("the current folder could not be read: {error}"))
        .and_then(|cwd| find_root(&cwd))
        .and_then(|root| {
            let entry = install(&root, write)?;
            if line {
                Ok(format!("{entry}\n\n{}", notes_line(&root, write)?))
            } else {
                Ok(format!(
                    "{entry}\n\nAgents learn about the reader's notes from the `notes_list` tool. `driftmark mcp install --notes-line` shows a line that tells them to call it, for AGENTS.md or CLAUDE.md."
                ))
            }
        });
    match outcome {
        Ok(message) => {
            let _ = writeln!(stdout, "{message}");
            EXIT_OK
        }
        Err(message) => {
            let _ = writeln!(stderr, "driftmark mcp install: {message}");
            EXIT_ERROR
        }
    }
}

/// Adds the entry to the `.mcp.json` of the repository at `root`, or with
/// `write` false only says what it would add. Returns what happened, as
/// sentences for the user.
///
/// # Errors
///
/// A sentence when the file is not valid JSON, has a shape the entry cannot
/// go into, already names another server `driftmark`, or cannot be read or
/// written. The file is then left as it was.
pub fn install(root: &Path, write: bool) -> Result<String, String> {
    let path = root.join(MCP_FILE);
    let original = match std::fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(format!("{MCP_FILE} could not be read: {error}")),
    };
    let mut file: Value = match &original {
        Some(text) => serde_json::from_str(text).map_err(|error| {
            format!("{MCP_FILE} is not valid JSON ({error}), so nothing was added; fix the file, then run this again")
        })?,
        None => Value::Object(Map::new()),
    };
    let servers = servers(&mut file)?;
    if let Some((name, _)) = servers
        .iter()
        .find(|(_, server)| runs_the_connector(server))
    {
        return Ok(format!(
            "{MCP_FILE} already starts `driftmark mcp` as the server `{name}`; nothing to add"
        ));
    }
    if let Some(other) = servers.get(SERVER_NAME) {
        return Err(format!(
            "{MCP_FILE} already has a server named `{SERVER_NAME}` that runs {}, so nothing was added; rename or remove it, then run this again",
            describe(other)
        ));
    }
    if !write {
        let shown = serde_json::to_string_pretty(&json!({"mcpServers": {SERVER_NAME: entry()}}))
            .unwrap_or_default();
        return Ok(format!(
            "driftmark mcp install would add this server to {MCP_FILE}:\n\n{shown}\n\nRun `driftmark mcp install --write` to add it, or add the same with Claude Code:\n\n  {CLAUDE_COMMAND}\n\nClaude Code asks once whether to start the servers in {MCP_FILE}."
        ));
    }
    servers.insert(SERVER_NAME.to_owned(), entry());
    save(&path, &file, original.as_deref(), MCP_FILE)?;
    Ok(format!(
        "Added the server `{SERVER_NAME}` to {MCP_FILE}. Claude Code asks once whether to start it; `claude mcp list` shows whether it connects."
    ))
}

/// Adds [`NOTES_LINE`] to the first of [`INSTRUCTION_FILES`] at `root`, or
/// with `write` false only says what it would add. A file that names
/// `notes_list` already gets nothing; one that is a symbolic link is left to
/// the user, so nothing outside the repository is written. Returns what
/// happened, as sentences.
///
/// # Errors
///
/// A sentence when the file cannot be read or written.
pub fn notes_line(root: &Path, write: bool) -> Result<String, String> {
    let found = INSTRUCTION_FILES
        .iter()
        .find(|name| root.join(name).symlink_metadata().is_ok());
    let Some(name) = found else {
        return Ok(format!(
            "There is no AGENTS.md or CLAUDE.md at the repository root. To tell agents about the reader's notes, add this line to the file your agent reads:\n\n  {NOTES_LINE}"
        ));
    };
    let path = root.join(name);
    let link = path
        .symlink_metadata()
        .is_ok_and(|meta| meta.file_type().is_symlink());
    if link {
        return Ok(format!(
            "{name} is a link, so driftmark leaves it alone. To tell agents about the reader's notes, add this line to the file it leads to:\n\n  {NOTES_LINE}"
        ));
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("{name} could not be read: {error}"))?;
    if text.contains("notes_list") {
        return Ok(format!(
            "{name} already tells agents about `notes_list`; nothing to add there"
        ));
    }
    if !write {
        return Ok(format!(
            "It would add this line at the end of {name}, so agents look for the reader's notes:\n\n  {NOTES_LINE}"
        ));
    }
    std::fs::write(path.with_extension("md.driftmark-new"), appended(&text))
        .and_then(|()| std::fs::rename(path.with_extension("md.driftmark-new"), &path))
        .map_err(|error| {
            let _ = std::fs::remove_file(path.with_extension("md.driftmark-new"));
            format!("{name} could not be written: {error}")
        })?;
    Ok(format!(
        "Added a line about the reader's notes at the end of {name}."
    ))
}

/// `text` with [`NOTES_LINE`] as a paragraph of its own at the end, in the
/// file's own line endings.
fn appended(text: &str) -> String {
    let ending = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut out = text.to_owned();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push_str(ending);
    }
    let blank = out.is_empty() || out.ends_with(&format!("{ending}{ending}"));
    if !blank {
        out.push_str(ending);
    }
    out.push_str(NOTES_LINE);
    out.push_str(ending);
    out
}

/// The `mcpServers` object of the file, made when missing.
fn servers(file: &mut Value) -> Result<&mut Map<String, Value>, String> {
    let shape = |what: &str| {
        format!("{what} in {MCP_FILE}, so the server has nowhere to go; nothing was added")
    };
    file.as_object_mut()
        .ok_or_else(|| shape("the top level is not an object"))?
        .entry("mcpServers")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| shape("`mcpServers` is not an object"))
}

/// Whether a server entry starts `driftmark mcp`, with the program named
/// by any path.
fn runs_the_connector(server: &Value) -> bool {
    let program = server["command"].as_str().unwrap_or_default();
    let named = Path::new(program)
        .file_stem()
        .is_some_and(|stem| stem == "driftmark");
    named && server["args"].get(0).and_then(Value::as_str) == Some("mcp")
}

/// What a server entry runs, for a message.
fn describe(server: &Value) -> String {
    match (server["command"].as_str(), server["url"].as_str()) {
        (Some(command), _) => format!("`{command}`"),
        (None, Some(url)) => format!("`{url}`"),
        (None, None) => "something else".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_connector_is_found_under_any_name_and_path() {
        assert!(runs_the_connector(&entry()));
        assert!(runs_the_connector(
            &json!({"command": "/opt/bin/driftmark", "args": ["mcp", "--base", "origin/dev"]})
        ));
        assert!(runs_the_connector(
            &json!({"command": "driftmark.exe", "args": ["mcp"]})
        ));
        assert!(!runs_the_connector(
            &json!({"command": "driftmark", "args": ["serve"]})
        ));
        assert!(!runs_the_connector(
            &json!({"command": "other", "args": ["mcp"]})
        ));
        assert!(!runs_the_connector(
            &json!({"type": "http", "url": "https://x"})
        ));
        assert_eq!(describe(&json!({"command": "npx"})), "`npx`");
        assert_eq!(describe(&json!({"url": "https://x"})), "`https://x`");
        assert_eq!(describe(&json!({})), "something else");
    }

    #[test]
    fn the_line_is_its_own_paragraph_in_the_files_line_endings() {
        assert_eq!(appended(""), format!("{NOTES_LINE}\n"));
        assert_eq!(appended("# A\n"), format!("# A\n\n{NOTES_LINE}\n"));
        assert_eq!(appended("# A"), format!("# A\n\n{NOTES_LINE}\n"));
        assert_eq!(appended("# A\n\n"), format!("# A\n\n{NOTES_LINE}\n"));
        assert_eq!(
            appended("# A\r\nText.\r\n"),
            format!("# A\r\nText.\r\n\r\n{NOTES_LINE}\r\n")
        );
    }
}
