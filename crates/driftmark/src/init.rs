//! `driftmark init`: sets up the starter layout in a repository that has no
//! docs system, and gives the block of agent instructions for `AGENTS.md` or
//! `CLAUDE.md`.
//!
//! Without `--write` it only says what it would do. With it, it creates the
//! folders and files that are missing and never changes one that exists. The
//! texts come from `driftmark_core::starter`; the contract is in
//! `docs/features/starter-layout.md`.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs::OpenOptions;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use driftmark_core::starter::{AGENT_BLOCK, LAYOUT, Part, agent_addition};

use crate::lint::{find_root, relative};

/// Exit code when the layout was set up, or would be.
const EXIT_DONE: u8 = 0;
/// Exit code when the command refused: outside a repository, an agent file
/// it may not touch, or a file it could not write.
const EXIT_REFUSED: u8 = 1;
/// Exit code for bad arguments.
const EXIT_USAGE: u8 = 2;

const USAGE: &str = "\
usage: driftmark init [--write] [--agent-file <file>]

Sets up driftmark's starter layout in this repository: docs/decisions,
docs/standards, docs/features, docs/working and docs/archive, each with a
README, the writing rules in docs/standards/writing.md, and the rules file
driftmark.toml. It also prints a block for AGENTS.md or CLAUDE.md that tells
agents where each kind of document goes.

  --write              create the folders and files that are missing; a file
                       that exists is never changed
  --agent-file <file>  add the block at the end of this file, once; without
                       --write, only say whether it would

Without --write it only prints what it would do.";

/// The file named by `--agent-file`, checked before anything is written.
struct AgentFile {
    /// The file itself, past any link, so a link's target gets the block.
    path: PathBuf,
    /// The file's path from the repository root, for messages.
    shown: String,
    /// The file's text, or `None` when it does not exist yet.
    existing: Option<String>,
}

impl AgentFile {
    /// Finds the file `given` names from the current folder `cwd`, and reads
    /// it.
    ///
    /// # Errors
    ///
    /// A sentence for the user when the file is outside the repository at
    /// `root`, is a link to nothing, or cannot be read as UTF-8 text.
    fn open(root: &Path, cwd: &Path, given: &Path) -> Result<AgentFile, String> {
        let full = cwd.join(given);
        let name = full
            .file_name()
            .ok_or_else(|| format!("`{}` names no file", given.display()))?;
        let folder = full
            .parent()
            .and_then(|folder| folder.canonicalize().ok())
            .ok_or_else(|| format!("the folder of `{}` does not exist", given.display()))?;
        let mut path = folder.join(name);
        if std::fs::symlink_metadata(&path).is_ok() {
            path = path.canonicalize().map_err(|_| {
                format!(
                    "`{}` is a link to a file that does not exist",
                    given.display()
                )
            })?;
        }
        let shown = relative(root, &path).ok_or_else(|| {
            format!(
                "`{}` is outside the repository at {}; name a file inside it",
                given.display(),
                root.display()
            )
        })?;
        let existing =
            match std::fs::read(&path) {
                Ok(bytes) => Some(String::from_utf8(bytes).map_err(|_| {
                    format!("{shown} is not UTF-8 text, so nothing was added to it")
                })?),
                Err(error) if error.kind() == ErrorKind::NotFound => None,
                Err(error) => return Err(format!("{shown} could not be read: {error}")),
            };
        Ok(AgentFile {
            path,
            shown,
            existing,
        })
    }
}

/// One line of the report: what happened to one part.
struct Row {
    /// "created", "would create", "exists" and the like.
    word: &'static str,
    /// The part's path from the repository root; folders end in `/`.
    path: String,
    /// More words after the path, or nothing.
    note: &'static str,
}

/// Runs `driftmark init` with the arguments after `init`, and returns the
/// exit code.
pub fn run(args: &[OsString], stdout: &mut dyn Write, stderr: &mut dyn Write) -> u8 {
    let mut write = false;
    let mut agent_file = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--write") => write = true,
            Some("--agent-file") => {
                let Some(path) = args.next() else {
                    let _ = writeln!(
                        stderr,
                        "driftmark init: `--agent-file` needs a file, such as `--agent-file AGENTS.md`\n\n{USAGE}"
                    );
                    return EXIT_USAGE;
                };
                agent_file = Some(PathBuf::from(path));
            }
            Some("--help" | "-h") => {
                let _ = writeln!(stdout, "{USAGE}");
                return EXIT_DONE;
            }
            _ => {
                let _ = writeln!(
                    stderr,
                    "driftmark init: unknown argument `{}`\n\n{USAGE}",
                    arg.to_string_lossy()
                );
                return EXIT_USAGE;
            }
        }
    }
    let refuse = |stderr: &mut dyn Write, message: &str| {
        let _ = writeln!(stderr, "driftmark init: {message}");
        EXIT_REFUSED
    };
    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(error) => {
            return refuse(
                stderr,
                &format!("the current folder could not be read: {error}"),
            );
        }
    };
    let root = match find_root(&cwd) {
        Ok(root) => root,
        Err(message) => return refuse(stderr, &message),
    };
    let agent = match agent_file
        .map(|given| AgentFile::open(&root, &cwd, &given))
        .transpose()
    {
        Ok(agent) => agent,
        Err(message) => return refuse(stderr, &message),
    };
    let mut rows = Vec::new();
    let outcome = set_up(&root, write, agent.as_ref(), &mut rows);
    let _ = write!(
        stdout,
        "{}",
        report(&root, write, agent.as_ref(), &rows, outcome.is_ok())
    );
    match outcome {
        Ok(()) => EXIT_DONE,
        Err(message) => refuse(stderr, &message),
    }
}

/// Goes through the layout and the agent file. With `write` it creates what
/// is missing; without, it only looks. Each part adds its row as it is
/// done, so a failure leaves the rows of what was done before it.
///
/// # Errors
///
/// A sentence for the user when a folder or file cannot be made, or the
/// block cannot be added.
fn set_up(
    root: &Path,
    write: bool,
    agent: Option<&AgentFile>,
    rows: &mut Vec<Row>,
) -> Result<(), String> {
    let made = if write { "created" } else { "would create" };
    for part in LAYOUT {
        let full = root.join(part.path());
        let row = match part {
            Part::Folder(folder) => {
                let shown = format!("{folder}/");
                if full.is_dir() {
                    Row {
                        word: "exists",
                        path: shown,
                        note: "",
                    }
                } else {
                    if write {
                        std::fs::create_dir_all(&full)
                            .map_err(|error| format!("{shown} could not be made: {error}"))?;
                    }
                    Row {
                        word: made,
                        path: shown,
                        note: "",
                    }
                }
            }
            Part::File { path, text } => {
                let mut exists = std::fs::symlink_metadata(&full).is_ok();
                if write && !exists {
                    exists = !create(&full, text, path)?;
                }
                match (exists, write) {
                    (false, _) => Row {
                        word: made,
                        path: path.to_owned(),
                        note: "",
                    },
                    (true, false) => Row {
                        word: "exists",
                        path: path.to_owned(),
                        note: " (it stays as it is)",
                    },
                    (true, true) => Row {
                        word: "skipped",
                        path: path.to_owned(),
                        note: " (it exists, so it was left as it is)",
                    },
                }
            }
        };
        rows.push(row);
    }
    if let Some(agent) = agent {
        rows.push(add_block(agent, write)?);
    }
    Ok(())
}

/// Makes a file that does not exist yet, and says whether it made it.
/// `create_new` refuses a file that appeared since it was looked at, so an
/// existing file is never written; such a file counts as skipped.
fn create(full: &Path, text: &str, shown: &str) -> Result<bool, String> {
    let file = OpenOptions::new().write(true).create_new(true).open(full);
    match file {
        Ok(mut file) => file
            .write_all(text.as_bytes())
            .map(|()| true)
            .map_err(|error| format!("{shown} could not be written: {error}")),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => Ok(false),
        Err(error) => Err(format!("{shown} could not be written: {error}")),
    }
}

/// Adds the agent block at the end of the agent file, or with `write` false
/// only says whether it would. The file is opened to append, so its text so
/// far is never rewritten, and a link keeps pointing where it did.
fn add_block(agent: &AgentFile, write: bool) -> Result<Row, String> {
    let shown = agent.shown.clone();
    let Some(addition) = agent_addition(agent.existing.as_deref()) else {
        return Ok(Row {
            word: if write { "skipped" } else { "exists" },
            path: shown,
            note: " (it holds the agent block)",
        });
    };
    if write {
        OpenOptions::new()
            .append(true)
            .create(true)
            .open(&agent.path)
            .and_then(|mut file| file.write_all(addition.as_bytes()))
            .map_err(|error| format!("the agent block could not be added to {shown}: {error}"))?;
    }
    Ok(match (agent.existing.is_some(), write) {
        (true, true) => Row {
            word: "appended",
            path: shown,
            note: " (the agent block, at the end)",
        },
        (true, false) => Row {
            word: "would append",
            path: shown,
            note: " (the agent block, at the end)",
        },
        (false, true) => Row {
            word: "created",
            path: shown,
            note: " (with the agent block)",
        },
        (false, false) => Row {
            word: "would create",
            path: shown,
            note: " (with the agent block)",
        },
    })
}

/// What `init` prints: a line naming the repository, one row per part, what
/// to do next, and the agent block when the user still has to place it.
fn report(
    root: &Path,
    write: bool,
    agent: Option<&AgentFile>,
    rows: &[Row],
    finished: bool,
) -> String {
    let mut out = format!("driftmark init: the starter layout in {}.", root.display());
    if !write {
        out.push_str(" Nothing is written without --write.");
    }
    out.push_str("\n\n");
    let width = rows.iter().map(|row| row.word.len()).max().unwrap_or(0);
    for row in rows {
        let _ = writeln!(out, "  {:width$}  {}{}", row.word, row.path, row.note);
    }
    if !finished {
        return out;
    }
    out.push('\n');
    let missing = rows.iter().any(|row| row.word.starts_with("would"));
    if write {
        out.push_str("Next, start a document with `driftmark new <kind> \"<title>\"`, and check the documents with `driftmark lint`.\n");
    } else if missing {
        out.push_str("Run `driftmark init --write` to create what is missing. It never changes a file that exists.\n");
    } else {
        out.push_str("Everything is in place, so there is nothing to create.\n");
    }
    match agent {
        None => {
            out.push_str("\nAdd this block to AGENTS.md or CLAUDE.md, so agents know where each kind of document goes. `--agent-file AGENTS.md` adds it for you.\n\n");
            out.push_str(AGENT_BLOCK);
        }
        Some(agent) if !write && agent_addition(agent.existing.as_deref()).is_some() => {
            let _ = write!(
                out,
                "\nWith --write, this block goes at the end of {}:\n\n{AGENT_BLOCK}",
                agent.shown
            );
        }
        Some(_) => {}
    }
    out
}
