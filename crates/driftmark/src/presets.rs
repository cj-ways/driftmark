//! `driftmark presets`: the presets built into driftmark, and which ones a
//! repository uses. This module also reads, for the core, the files presets
//! and rules files look at beyond the documents: marker files such as
//! `openspec/config.yaml`, and files beside a document such as cc-sdd's
//! `spec.json`. It reads them at a commit, or in the working tree on disk.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::io::Write;
use std::path::{Path, PathBuf};

use driftmark_core::git::{GitRead, ObjectId, RepoPath, Roots, Snapshot};
use driftmark_core::presets::{PRESETS, Setup, find};
use driftmark_core::rules::{RepoFiles, Rules};

use crate::git::Git;
use crate::git::worktree;
use crate::lint::RULES_FILE;

/// Exit code when the command ran.
const EXIT_OK: u8 = 0;
/// Exit code when it could not run: bad arguments, no repository, an unknown
/// preset, or a rules file that does not parse.
const EXIT_USAGE: u8 = 2;

const USAGE: &str = "\
usage: driftmark presets [--repo <folder>]
       driftmark presets show <name> [--repo <folder>]

A preset is a ready-made rules file for a popular folder layout. The first
form lists the presets built into driftmark and says which ones the
repository uses. A repository with no driftmark.toml uses every preset whose
layout it has; one with a driftmark.toml uses the presets it names in
`presets = [...]`. The second form prints one preset's rules, ready to copy
into driftmark.toml and change.

  --repo <folder>   a folder inside the repository (default: the current one)

Exit codes: 0 done, 2 the command could not run.";

/// The other files of a repository at one side of a comparison.
pub enum FilesAt<'a> {
    /// The files of a commit.
    Commit(&'a dyn GitRead, &'a ObjectId),
    /// The files on disk in a working tree.
    Disk(DiskFiles),
    /// No files: the working tree of a bare repository.
    None,
}

/// The other files at `side`: the commit's, or those in the working tree on
/// disk.
#[must_use]
pub fn files_at<'a>(git: &'a Git, side: &'a Snapshot) -> FilesAt<'a> {
    match side {
        Snapshot::Commit(commit) => FilesAt::Commit(git, commit),
        Snapshot::Worktree => git
            .workdir()
            .map_or(FilesAt::None, |root| FilesAt::Disk(DiskFiles::new(root))),
    }
}

impl RepoFiles for FilesAt<'_> {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        match self {
            FilesAt::Commit(git, commit) => {
                let path = RepoPath::new(path)?;
                let blob = git.blob_at(commit, &path).ok()??;
                git.read_blob(&blob).ok()
            }
            FilesAt::Disk(disk) => disk.read(path),
            FilesAt::None => None,
        }
    }

    fn markdown_in(&self, folders: &[&str]) -> Vec<String> {
        match self {
            FilesAt::Commit(git, commit) => {
                let roots = if folders.iter().any(|folder| folder.is_empty()) {
                    Ok(Roots::all())
                } else {
                    Roots::new(folders)
                };
                let Ok(roots) = roots else {
                    return Vec::new();
                };
                git.markdown_files(&Snapshot::Commit((*commit).clone()), &roots)
                    .map(|files| {
                        files
                            .into_iter()
                            .map(|file| file.path.to_lossy().into_owned())
                            .collect()
                    })
                    .unwrap_or_default()
            }
            FilesAt::Disk(disk) => disk.markdown_in(folders),
            FilesAt::None => Vec::new(),
        }
    }
}

/// The files in a folder on disk. A path that passes through a symbolic link
/// counts as missing, so nothing outside the folder is read.
pub struct DiskFiles {
    root: PathBuf,
}

impl DiskFiles {
    /// The files under `root`, the repository's top folder.
    #[must_use]
    pub fn new(root: &Path) -> DiskFiles {
        DiskFiles {
            root: root.to_path_buf(),
        }
    }
}

impl RepoFiles for DiskFiles {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        let full = worktree::regular_file(&self.root, &RepoPath::new(path)?)?;
        std::fs::read(full).ok()
    }

    fn markdown_in(&self, folders: &[&str]) -> Vec<String> {
        let mut found = Vec::new();
        for folder in folders {
            let start = if folder.is_empty() {
                Some(self.root.clone())
            } else {
                RepoPath::new(*folder).and_then(|path| real_folder(&self.root, &path))
            };
            if let Some(start) = start {
                walk(&start, folder, &mut found);
            }
        }
        found.sort();
        found.dedup();
        found
    }
}

/// Where a repository folder is on disk, if it is a real folder and every
/// folder on the way is one too.
fn real_folder(root: &Path, path: &RepoPath) -> Option<PathBuf> {
    let mut full = root.to_path_buf();
    for part in path.as_bytes().split(|&b| b == b'/') {
        full.push(worktree::os_str(part)?);
        if !std::fs::symlink_metadata(&full).ok()?.file_type().is_dir() {
            return None;
        }
    }
    Some(full)
}

/// Adds the Markdown files under `folder`, named `prefix` from the root, to
/// `found`. Symbolic links and `.git` folders are not followed.
fn walk(folder: &Path, prefix: &str, found: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        if kind.is_dir() && name != ".git" {
            walk(&entry.path(), &path, found);
        } else if kind.is_file() && RepoPath::new(path.as_str()).is_some_and(|p| p.is_markdown()) {
            found.push(path);
        }
    }
}

/// What `driftmark presets` was asked to do.
#[derive(Debug, PartialEq, Eq)]
enum Request {
    Help,
    List { repo: PathBuf },
    Show { name: String, repo: PathBuf },
}

fn parse(args: &[OsString]) -> Result<Request, String> {
    let mut repo = None;
    let mut words = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let text = arg
            .to_str()
            .ok_or_else(|| format!("`{}` is not valid text", arg.to_string_lossy()))?;
        match text {
            "--help" | "-h" => return Ok(Request::Help),
            "--repo" => {
                let value = args.next().ok_or("`--repo` needs a value")?;
                repo = Some(PathBuf::from(value));
            }
            _ if text.starts_with("--repo=") => {
                repo = Some(PathBuf::from(&text["--repo=".len()..]));
            }
            _ if text.starts_with('-') => {
                return Err(format!("`{text}` is not an option of driftmark presets"));
            }
            _ => words.push(text.to_owned()),
        }
    }
    let repo = repo.unwrap_or_else(|| PathBuf::from("."));
    match words.as_slice() {
        [] => Ok(Request::List { repo }),
        [show, name] if show == "show" => Ok(Request::Show {
            name: name.clone(),
            repo,
        }),
        [show] if show == "show" => {
            Err("`driftmark presets show` needs a preset's name".to_owned())
        }
        _ => Err(format!(
            "`{}` is not a driftmark presets command",
            words.join(" ")
        )),
    }
}

/// Runs `driftmark presets` with the arguments after `presets`.
pub fn run(args: &[OsString], stdout: &mut dyn Write, stderr: &mut dyn Write) -> u8 {
    let request = match parse(args) {
        Ok(request) => request,
        Err(message) => {
            let _ = writeln!(stderr, "driftmark presets: {message}\n\n{USAGE}");
            return EXIT_USAGE;
        }
    };
    let done = match request {
        Request::Help => writeln!(stdout, "{USAGE}").map_err(|error| error.to_string()),
        Request::List { repo } => list(&repo, stdout),
        Request::Show { name, repo } => show(&name, &repo, stdout),
    };
    match done {
        Ok(()) => EXIT_OK,
        Err(message) => {
            let _ = writeln!(stderr, "driftmark presets: {message}");
            EXIT_USAGE
        }
    }
}

/// The side a repository's presets are read at: the working tree, or `HEAD`
/// in a bare repository.
fn side_of(git: &Git) -> Result<Snapshot, String> {
    if git.workdir().is_some() {
        Ok(Snapshot::Worktree)
    } else {
        git.resolve("HEAD")
            .map(Snapshot::Commit)
            .map_err(|error| format!("the repository has no commit to read: {error}"))
    }
}

fn list(repo: &Path, stdout: &mut dyn Write) -> Result<(), String> {
    let git =
        Git::open(repo).map_err(|error| format!("run it inside a git repository ({error})"))?;
    let side = side_of(&git)?;
    let own = crate::diff::rules_file::own(&git, &side)?;
    let found = driftmark_core::presets::detect(&files_at(&git, &side));
    let has_file = own.is_some();
    let setup = crate::diff::rules_file::with_presets(&git, &side, own)?;
    let mut out = String::new();
    for preset in PRESETS {
        let used = setup
            .presets
            .iter()
            .find(|in_use| in_use.preset.name == preset.name);
        let marker = found
            .iter()
            .find(|found| found.preset.name == preset.name)
            .map(|found| found.marker.as_str());
        let state = match (used, marker) {
            (Some(_), _) if has_file => format!("in use: {RULES_FILE} names it"),
            (Some(_), Some(marker)) => format!("in use: found {marker}"),
            (None, Some(marker)) => {
                format!("found {marker}, but not in use: {RULES_FILE} does not name it")
            }
            // A preset with no markers, such as the starter layout's, is
            // never looked for, so "not found" would mislead.
            (None, None) if preset.markers.is_empty() => {
                format!("used only when {RULES_FILE} names it")
            }
            (Some(_) | None, None) => "not found here".to_owned(),
        };
        let _ = writeln!(out, "{:<10} {}", preset.name, preset.title);
        let _ = writeln!(out, "{:<10} {state}", "");
        if let Some(in_use) = used
            && !in_use.folders.is_empty()
        {
            let folders: Vec<&str> = in_use
                .folders
                .iter()
                .map(|folder| {
                    if folder.is_empty() {
                        "the top folder"
                    } else {
                        folder.as_str()
                    }
                })
                .collect();
            let _ = writeln!(
                out,
                "{:<10} also reads the records in {}, named by .adr-dir or .log4brains.yml",
                "",
                folders.join(", ")
            );
        }
    }
    let names: Vec<String> = setup
        .presets
        .iter()
        .map(|in_use| format!("\"{}\"", in_use.preset.name))
        .collect();
    let names = names.join(", ");
    let _ = writeln!(out);
    let _ = match (has_file, names.is_empty()) {
        (false, false) => writeln!(
            out,
            "This repository has no {RULES_FILE}, so driftmark uses every preset it finds.\nTo keep this choice, or to add types of your own, write this line in {RULES_FILE}:\n\n  presets = [{names}]"
        ),
        (false, true) => writeln!(
            out,
            "No preset's layout is in this repository. To use one anyway, name it in\n{RULES_FILE}, as in `presets = [\"adr\"]`."
        ),
        (true, false) => writeln!(
            out,
            "{RULES_FILE} names the presets {names}. Its own types come first."
        ),
        (true, true) => writeln!(
            out,
            "{RULES_FILE} names no preset, so driftmark uses its own types only. To use\npresets, add a line such as `presets = [\"adr\"]` to it."
        ),
    };
    let _ = writeln!(
        out,
        "\nTo see a preset's rules, ready to copy, run `driftmark presets show <name>`."
    );
    stdout
        .write_all(out.as_bytes())
        .map_err(|error| error.to_string())
}

fn show(name: &str, repo: &Path, stdout: &mut dyn Write) -> Result<(), String> {
    let preset = find(name).ok_or_else(|| {
        let known: Vec<&str> = PRESETS.iter().map(|preset| preset.name).collect();
        format!(
            "driftmark has no preset {name:?}; the presets are {}",
            known.join(", ")
        )
    })?;
    // Inside a repository, a preset that reads folders named in marker files
    // shows them, so the copy works there. Outside one, the text is printed
    // as it is built in.
    let folders = Git::open(repo)
        .ok()
        .and_then(|git| {
            let side = side_of(&git).ok()?;
            Some(preset.folders(&files_at(&git, &side)))
        })
        .unwrap_or_default();
    stdout
        .write_all(preset.text_with(&folders).as_bytes())
        .map_err(|error| error.to_string())
}

/// The rules a repository uses at one side, and the presets in use: those
/// its rules file names, or, with no rules file, those found there. This is
/// what `driftmark diff`, `serve` and `mcp` read documents with.
///
/// # Errors
///
/// A sentence when the rules file cannot be read or is not valid.
pub fn setup_at(git: &Git, side: &Snapshot) -> Result<Setup, String> {
    let own = crate::diff::rules_file::own(git, side)?;
    crate::diff::rules_file::with_presets(git, side, own)
}

/// The rules of the working tree at `root`, for `driftmark lint`: the rules
/// file on disk with the presets it names, or, without one, the presets
/// found there.
///
/// # Errors
///
/// A sentence when the rules file cannot be read or is not valid.
pub fn rules_on_disk(root: &Path) -> Result<Rules, String> {
    let own = match std::fs::read_to_string(root.join(RULES_FILE)) {
        Ok(text) => Some(
            Rules::parse(&text)
                .map_err(|error| format!("{RULES_FILE}: {}", error.to_string().trim_end()))?,
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(format!("{RULES_FILE} could not be read: {error}")),
    };
    Setup::build(own, &DiskFiles::new(root))
        .map(|setup| setup.rules)
        .map_err(|error| format!("{RULES_FILE}: {}", error.to_string().trim_end()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(args: &[&str]) -> Result<Request, String> {
        parse(&args.iter().map(OsString::from).collect::<Vec<_>>())
    }

    #[test]
    fn the_command_line_is_read() {
        assert_eq!(words(&[]), Ok(Request::List { repo: ".".into() }));
        assert_eq!(
            words(&["--repo", "r"]),
            Ok(Request::List { repo: "r".into() })
        );
        assert_eq!(
            words(&["show", "adr", "--repo=r"]),
            Ok(Request::Show {
                name: "adr".to_owned(),
                repo: "r".into()
            })
        );
        assert_eq!(words(&["show", "--help"]), Ok(Request::Help));
        assert_eq!(words(&["--repo"]), Err("`--repo` needs a value".to_owned()));
        assert_eq!(
            words(&["show"]),
            Err("`driftmark presets show` needs a preset's name".to_owned())
        );
        assert_eq!(
            words(&["list", "x"]),
            Err("`list x` is not a driftmark presets command".to_owned())
        );
        assert_eq!(
            words(&["--json"]),
            Err("`--json` is not an option of driftmark presets".to_owned())
        );
    }
}
