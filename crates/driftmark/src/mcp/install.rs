//! `driftmark mcp install`: the entry that makes Claude Code start the
//! connector, in the repository's `.mcp.json`.
//!
//! Claude Code reads a project's servers from `.mcp.json` at its root and
//! asks once before it starts them. Without `--write` this prints the entry
//! and the `claude mcp add` command that adds the same; with it, the entry
//! is added and every other entry, key and the file's own indentation stay,
//! the way `driftmark hook install` treats `.claude/settings.json`.

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

/// Runs `driftmark mcp install`, and returns the exit code.
pub(super) fn run(write: bool, stdout: &mut dyn Write, stderr: &mut dyn Write) -> u8 {
    let outcome = std::env::current_dir()
        .map_err(|error| format!("the current folder could not be read: {error}"))
        .and_then(|cwd| find_root(&cwd))
        .and_then(|root| install(&root, write));
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
}
