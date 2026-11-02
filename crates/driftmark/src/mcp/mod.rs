//! `driftmark mcp`: the same program, started by an agent such as Claude
//! Code, Codex or Cursor, answering over the Model Context Protocol on
//! standard input and output.
//!
//! The tools read what `driftmark serve` reads, through the same git
//! reader, cache and comparisons: what changed in a range, one document's
//! changes, a document or one of its sections, a search, which documents
//! are current, the writing check, and the reader's notes. Three note tools write to the notes file that
//! `serve` writes, as the agent: a reply, a proposed decision, and the
//! commits that applied a decision. No tool writes a document or runs a
//! program other than git. `docs/features/commands/mcp.md` is the contract
//! of every tool.
//!
//! - [`protocol`] is the only part that names the rmcp crate.
//! - [`tools`] lists the tools and reads their arguments; one module per
//!   tool family answers.
//! - [`fit`] keeps every answer inside its size limit.

mod changes;
mod document;
mod fit;
mod install;
mod lint;
mod notes;
mod protocol;
mod search;
mod status;
mod tools;

use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Map, Value};

use crate::git::Git;
use crate::memory::{IDLE_AFTER, Idle, give_back};
use crate::serve::placing::NotesFile;
use crate::serve::repo::Repo;
use protocol::Connector;

/// Exit code after the client ended the session, or after `install` did
/// its work.
pub const EXIT_OK: u8 = 0;
/// Exit code when the server could not start, or `install` refused.
pub const EXIT_ERROR: u8 = 1;
/// Exit code when the command line is wrong.
pub const EXIT_USAGE: u8 = 2;

/// The memory the connector keeps for parsed documents and counted ranges,
/// in bytes. An agent reads a handful of documents and ranges, and several
/// agents may each run a connector, so this is a quarter of the server's.
const CACHE_BYTES: usize = 32 * 1024 * 1024;

/// How `driftmark mcp` is called.
const USAGE: &str = "\
usage: driftmark mcp [--repo <path>] [--base <branch>]
       driftmark mcp install [--write] [--notes-line]

Answers an agent over the Model Context Protocol on standard input and
output: what changed in the docs, a document or one section of it, a search,
the writing check, and the reader's notes. An agent such as Claude Code
starts it; it stops when the agent closes standard input. It reads only
through git and the repository's files, never writes a document, and writes
only replies, proposals and applied commits to the notes file that
`driftmark serve` keeps.

Options:
  --repo <path>     a folder inside the repository (default: here)
  --base <branch>   the branch `auto` compares with (default: the rules
                    file's [compare] default-branch, else the first of
                    origin/main, main, origin/master, master, origin/dev, dev)

`driftmark mcp install` prints the entry for this repository's .mcp.json,
and the `claude mcp add` command that adds the same; with --write it adds
the entry to .mcp.json and leaves every other entry as it was. With
--notes-line it also shows, or with --write adds, a line at the end of
AGENTS.md (or CLAUDE.md) that tells agents to look for the reader's notes.

Exit codes: 0 the session ended or install did its work, 1 could not start
or install refused, 2 the command line is wrong.";

/// What the command line asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Request {
    /// Serve the protocol.
    Serve {
        /// A folder inside the repository.
        repo: PathBuf,
        /// The branch `auto` compares with, when named.
        base: Option<String>,
    },
    /// Print or write the `.mcp.json` entry.
    Install {
        /// Whether to write it.
        write: bool,
        /// Whether to add the line about notes to the agent instruction
        /// file too.
        line: bool,
    },
    /// Print the usage text.
    Help,
}

/// Reads the arguments after `mcp`.
fn parse(args: &[OsString]) -> Result<Request, String> {
    let words: Vec<&str> = args
        .iter()
        .map(|arg| arg.to_str().ok_or("an argument is not valid text"))
        .collect::<Result<_, _>>()?;
    match words.as_slice() {
        ["install", "--help" | "-h"] | ["--help" | "-h"] => return Ok(Request::Help),
        ["install", options @ ..] => {
            let (mut write, mut line) = (false, false);
            for option in options {
                match *option {
                    "--write" if !write => write = true,
                    "--notes-line" if !line => line = true,
                    _ => {
                        return Err("`install` takes only `--write` and `--notes-line`".to_owned());
                    }
                }
            }
            return Ok(Request::Install { write, line });
        }
        _ => {}
    }
    let mut repo = PathBuf::from(".");
    let mut base = None;
    let mut words = words.into_iter();
    while let Some(word) = words.next() {
        let (name, inline) = match word.split_once('=') {
            Some((name, value)) if name.starts_with("--") => (name, Some(value)),
            _ => (word, None),
        };
        let mut value = || {
            inline
                .map(str::to_owned)
                .or_else(|| words.next().map(str::to_owned))
                .filter(|value| !value.is_empty())
                .ok_or_else(|| format!("`{name}` needs a value"))
        };
        match name {
            "--repo" => repo = PathBuf::from(value()?),
            "--base" => base = Some(value()?),
            other => return Err(format!("unknown argument `{other}`")),
        }
    }
    Ok(Request::Serve { repo, base })
}

/// Runs `driftmark mcp` with the arguments after `mcp`, and returns the exit
/// code. While serving, standard output carries only the protocol, so
/// `stdout` is used by `install` and `--help` alone.
pub fn run(args: &[OsString], stdout: &mut dyn Write, stderr: &mut dyn Write) -> u8 {
    let request = match parse(args) {
        Ok(request) => request,
        Err(message) => {
            let _ = writeln!(stderr, "driftmark mcp: {message}\n\n{USAGE}");
            return EXIT_USAGE;
        }
    };
    match request {
        Request::Help => {
            let _ = writeln!(stdout, "{USAGE}");
            EXIT_OK
        }
        Request::Install { write, line } => install::run(write, line, stdout, stderr),
        Request::Serve { repo, base } => match serve(&repo, base) {
            Ok(()) => EXIT_OK,
            Err(message) => {
                let _ = writeln!(stderr, "driftmark mcp: {message}");
                EXIT_ERROR
            }
        },
    }
}

/// Serves the tools over standard input and output until the client closes
/// standard input.
fn serve(start: &std::path::Path, base: Option<String>) -> Result<(), String> {
    let git = Git::open(start).map_err(|error| {
        format!(
            "`{}` is not inside a git repository that can be read: {error}",
            start.display()
        )
    })?;
    let workdir = git.workdir().map(std::path::Path::to_path_buf);
    let threads = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    let repo = Arc::new(Repo::new(git, CACHE_BYTES, threads, base));
    // Memory goes back to the system once no call ran for a second
    // (decision 0011).
    let idle = {
        let repo = Arc::clone(&repo);
        Idle::start(IDLE_AFTER, move || give_back(&repo.git))
    };
    // The notes file of decision 0011, item 2, shared with `driftmark serve`.
    let notes_path = crate::notes::notes_file(start).map_err(|error| {
        format!(
            "the notes file of `{}` cannot be found: {error}",
            start.display()
        )
    })?;
    let reviews = notes_path.with_file_name(crate::notes::REVIEWS);
    let context = tools::Context {
        repo,
        workdir,
        worktree_search: search::WorktreeSearch::default(),
        notes: NotesFile::new(notes_path, reviews, String::new()),
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("driftmark-mcp")
        .build()
        .map_err(|error| format!("the connector could not start: {error}"))?;
    let served = runtime.block_on(protocol::serve_stdio(Arc::new(Watched {
        tools: tools::Driftmark::new(context),
        idle,
    })));
    // A tool still running on a blocking thread is left behind; the process
    // ends right after.
    runtime.shutdown_timeout(Duration::from_millis(100));
    served
}

/// The tools, with each call marked as work for the [`Idle`] watcher.
struct Watched {
    tools: tools::Driftmark,
    idle: Idle,
}

impl Connector for Watched {
    fn instructions(&self) -> String {
        self.tools.instructions()
    }

    fn tools(&self) -> Vec<protocol::ToolInfo> {
        self.tools.tools()
    }

    fn call(
        &self,
        name: &str,
        arguments: Map<String, Value>,
        client: &protocol::Client,
    ) -> Option<protocol::Reply> {
        let _busy = self.idle.begin();
        self.tools.call(name, arguments, client)
    }

    fn missing(&self, name: &str) -> String {
        self.tools.missing(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(words: &[&str]) -> Result<Request, String> {
        let args: Vec<OsString> = words.iter().map(OsString::from).collect();
        parse(&args)
    }

    #[test]
    fn the_command_line_names_the_repository_and_the_base() {
        assert_eq!(
            read(&[]),
            Ok(Request::Serve {
                repo: PathBuf::from("."),
                base: None
            })
        );
        assert_eq!(
            read(&["--repo", "/r", "--base=origin/dev"]),
            Ok(Request::Serve {
                repo: PathBuf::from("/r"),
                base: Some("origin/dev".into())
            })
        );
        assert_eq!(read(&["--repo"]), Err("`--repo` needs a value".into()));
        assert_eq!(read(&["--base="]), Err("`--base` needs a value".into()));
        assert_eq!(read(&["serve"]), Err("unknown argument `serve`".into()));
        assert_eq!(read(&["--help"]), Ok(Request::Help));
        assert_eq!(read(&["install", "-h"]), Ok(Request::Help));
        assert_eq!(
            read(&["install"]),
            Ok(Request::Install {
                write: false,
                line: false
            })
        );
        assert_eq!(
            read(&["install", "--write"]),
            Ok(Request::Install {
                write: true,
                line: false
            })
        );
        assert_eq!(
            read(&["install", "--notes-line", "--write"]),
            Ok(Request::Install {
                write: true,
                line: true
            })
        );
        for wrong in [
            &["install", "--force"][..],
            &["install", "--write", "--write"],
        ] {
            assert_eq!(
                read(wrong),
                Err("`install` takes only `--write` and `--notes-line`".into())
            );
        }
    }
}
