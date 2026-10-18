//! `driftmark new <kind> <title>`: starts a document of the starter layout
//! from its kind's template, and prints the new file's path.
//!
//! The kinds, the templates and the numbering live in
//! `driftmark_core::starter`; this module reads the clock and the decisions
//! folder, and writes the file. The contract is in
//! `docs/features/starter-layout.md`.

use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io::{ErrorKind, Write};
use std::path::Path;

use driftmark_core::starter::{DECISIONS_FOLDER, Kind, new_document, next_decision_number};

use crate::lint::find_root;

/// Exit code when the document was made.
const EXIT_DONE: u8 = 0;
/// Exit code when the command refused: outside a repository, a file that
/// exists, or a file it could not write.
const EXIT_REFUSED: u8 = 1;
/// Exit code for bad arguments, a bad title included.
const EXIT_USAGE: u8 = 2;

const USAGE: &str = "\
usage: driftmark new <kind> <title>

Starts a document from the starter layout's template for its kind, and prints
the new file's path from the repository root. The kinds, and where they go:

  decision   docs/decisions/<number>-<title>.md, numbered after the last one
  standard   docs/standards/<title>.md
  feature    docs/features/<title>.md
  plan       docs/working/<title>/plan.md
  status     docs/working/<title>/status.md

The title is everything after the kind, such as
`driftmark new decision \"Keep one database per service\"`. In the file name it
is written in lower case, with `-` between the words. A file that exists is
never changed.";

/// Runs `driftmark new` with the arguments after `new`, and returns the exit
/// code.
pub fn run(args: &[OsString], stdout: &mut dyn Write, stderr: &mut dyn Write) -> u8 {
    let usage = |stderr: &mut dyn Write, message: &str| {
        let _ = writeln!(stderr, "driftmark new: {message}\n\n{USAGE}");
        EXIT_USAGE
    };
    let Some(words) = args
        .iter()
        .map(|arg| arg.to_str())
        .collect::<Option<Vec<&str>>>()
    else {
        return usage(stderr, "the arguments are not UTF-8 text");
    };
    let (kind, rest) = match words.as_slice() {
        [] => return usage(stderr, "name a kind and a title"),
        ["--help" | "-h", ..] => {
            let _ = writeln!(stdout, "{USAGE}");
            return EXIT_DONE;
        }
        [kind, rest @ ..] => (*kind, rest),
    };
    let Some(kind) = Kind::from_name(kind) else {
        return usage(
            stderr,
            &format!("there is no kind `{kind}`; use decision, standard, feature, plan or status"),
        );
    };
    // A word starting with `--` is an option, unless `--` right after the
    // kind says the rest is all title.
    let title = if let ["--", title @ ..] = rest {
        title.join(" ")
    } else if let Some(option) = rest.iter().find(|word| word.starts_with("--")) {
        return usage(
            stderr,
            &format!("unknown option `{option}`; put `--` before a title that starts with `--`"),
        );
    } else {
        rest.join(" ")
    };
    let refuse = |stderr: &mut dyn Write, message: &str| {
        let _ = writeln!(stderr, "driftmark new: {message}");
        EXIT_REFUSED
    };
    let root = match std::env::current_dir() {
        Ok(cwd) => match find_root(&cwd) {
            Ok(root) => root,
            Err(message) => return refuse(stderr, &message),
        },
        Err(error) => {
            return refuse(
                stderr,
                &format!("the current folder could not be read: {error}"),
            );
        }
    };
    let number = if kind == Kind::Decision {
        match decision_names(&root) {
            Ok(names) => next_decision_number(names.iter().map(String::as_str)),
            Err(message) => return refuse(stderr, &message),
        }
    } else {
        0
    };
    let document = match new_document(kind, &title, &today(), number) {
        Ok(document) => document,
        Err(error) => return usage(stderr, &error.to_string()),
    };
    match write_new(&root.join(&document.path), &document.text) {
        Ok(()) => {
            let _ = writeln!(stdout, "{}", document.path);
            EXIT_DONE
        }
        Err(error) if error.kind() == ErrorKind::AlreadyExists => refuse(
            stderr,
            &format!(
                "{} already exists, so nothing was written; choose another title, or edit that file",
                document.path
            ),
        ),
        Err(error) => refuse(
            stderr,
            &format!("{} could not be written: {error}", document.path),
        ),
    }
}

/// Today's date in the local time zone, such as `2026-10-18`, for the new
/// document's `date`. jiff falls back to UTC when the time zone cannot be
/// found.
fn today() -> String {
    jiff::Zoned::now().date().to_string()
}

/// The file names in the decisions folder, or none when it does not exist.
fn decision_names(root: &Path) -> Result<Vec<String>, String> {
    let folder = root.join(DECISIONS_FOLDER);
    let entries = match std::fs::read_dir(&folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("{DECISIONS_FOLDER} could not be read: {error}")),
    };
    let mut names = Vec::new();
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("{DECISIONS_FOLDER} could not be read: {error}"))?;
        if let Some(name) = entry.file_name().to_str() {
            names.push(name.to_owned());
        }
    }
    Ok(names)
}

/// Writes a file that must not exist yet, making its folders first.
/// `create_new` refuses a file that exists, even one made a moment ago by
/// another process, so an existing file is never changed.
fn write_new(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)?;
    }
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?
        .write_all(text.as_bytes())
}
