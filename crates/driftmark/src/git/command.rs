//! The reader that runs the `git` command.
//!
//! It is the test check for the gix reader, and the fallback for repos gix
//! cannot read yet: partial clones, where `git` fetches missing files itself,
//! and SHA-256 or reftable repos. Reading many blobs goes through one
//! long-lived `git cat-file --batch` process, as the speed spike measured.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Output, Stdio};
use std::sync::{Mutex, PoisonError};

use driftmark_core::git::{
    FileChange, FileEntry, GitError, GitRead, ObjectId, ObjectKind, RepoPath, Roots, Snapshot,
};

use super::worktree;

/// Environment variables that would point `git` at another repo, another
/// index or other path rules than the folder this reader opened. gix ignores
/// them too, so both readers read the same repo.
const FOREIGN_ENV: [&str; 13] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
    "GIT_PREFIX",
    "GIT_CEILING_DIRECTORIES",
    "GIT_LITERAL_PATHSPECS",
    "GIT_GLOB_PATHSPECS",
    "GIT_NOGLOB_PATHSPECS",
    "GIT_ICASE_PATHSPECS",
];

/// Reads a repo by running the `git` command.
pub struct CommandReader {
    /// Where `git` runs: the top of the working tree, or the git folder of a
    /// bare repo. Running at the top makes every path relative to it.
    run_in: PathBuf,
    /// The top of the working tree; `None` for a bare repo.
    workdir: Option<PathBuf>,
    /// The `git cat-file --batch` process, started on the first read.
    cat_file: Mutex<Option<CatFile>>,
}

impl CommandReader {
    /// Opens the repo that holds `start`, which may be any folder inside it.
    ///
    /// # Errors
    ///
    /// When `git` cannot be started or `start` is not inside a repo.
    pub fn open(start: &Path) -> Result<Self, GitError> {
        let found = run_ok(git_in(start).args([
            "rev-parse",
            "--absolute-git-dir",
            "--is-bare-repository",
        ]))?;
        let lines: Vec<&[u8]> = found.split(|&b| b == b'\n').collect();
        let (git_dir, bare) = match lines.as_slice() {
            [git_dir, bare, b""] => (output_path(git_dir)?, *bare == b"true"),
            _ => return Err(odd_output("rev-parse", &found)),
        };
        let workdir = if bare {
            None
        } else {
            let top = run_ok(git_in(start).args(["rev-parse", "--show-toplevel"]))?;
            Some(output_path(top.strip_suffix(b"\n").unwrap_or(&top))?)
        };
        Ok(Self {
            run_in: workdir.clone().unwrap_or(git_dir),
            workdir,
            cat_file: Mutex::new(None),
        })
    }

    /// The top of the working tree, or `None` for a bare repo.
    #[must_use]
    pub fn workdir(&self) -> Option<&Path> {
        self.workdir.as_deref()
    }

    /// A `git` command for this repo. Paths given to it are taken literally,
    /// never as patterns.
    fn git(&self) -> Command {
        let mut command = git_in(&self.run_in);
        command.arg("--literal-pathspecs");
        command
    }

    /// Reads one object through the long-lived `cat-file` process.
    fn read_object(&self, id: &ObjectId, expected: ObjectKind) -> Result<Vec<u8>, GitError> {
        let mut slot = self.cat_file.lock().unwrap_or_else(PoisonError::into_inner);
        let cat_file = match slot.as_mut() {
            Some(cat_file) => cat_file,
            None => slot.insert(CatFile::start(self.git())?),
        };
        match cat_file.read(id) {
            Ok(Some((kind, bytes))) if kind == expected.to_string() => Ok(bytes),
            Ok(Some(_)) => Err(GitError::WrongKind {
                id: id.clone(),
                expected,
            }),
            Ok(None) => Err(GitError::MissingObject { id: id.clone() }),
            Err(error) => {
                // The process is in an unknown state; start a new one next time.
                *slot = None;
                Err(other(format!("git cat-file failed: {error}")))
            }
        }
    }

    /// The paths in the index with a regular-file mode, and the untracked
    /// files git does not ignore.
    fn worktree_candidates(&self, roots: &Roots) -> Result<BTreeSet<RepoPath>, GitError> {
        let mut paths = BTreeSet::new();
        let staged = run_ok(with_roots(
            self.git().args(["ls-files", "-z", "--stage"]),
            roots,
        )?)?;
        // "<mode> <id> <stage>\t<path>", one per index entry.
        for record in records(&staged) {
            let (meta, path) = split_tab(record)?;
            let mode = meta.split(|&b| b == b' ').next().unwrap_or_default();
            if is_regular_file(mode) {
                paths.insert(repo_path(path)?);
            }
        }
        let untracked = run_ok(with_roots(
            self.git()
                .args(["ls-files", "-z", "--others", "--exclude-standard"]),
            roots,
        )?)?;
        for path in records(&untracked) {
            paths.insert(repo_path(path)?);
        }
        Ok(paths)
    }

    /// The blob ids git would store for these working-tree files, with its
    /// filters and line-ending conversion applied, as `git add` would.
    fn hash_files(&self, paths: &[RepoPath]) -> Result<Vec<ObjectId>, GitError> {
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        let mut input = Vec::new();
        for path in paths {
            quote_c_style(path.as_bytes(), &mut input);
            input.push(b'\n');
        }
        let mut command = self.git();
        command.args(["hash-object", "--stdin-paths"]);
        let output = run_with_input(command, input)?;
        let text = String::from_utf8_lossy(&output);
        let ids: Vec<ObjectId> = text.lines().map(parse_id).collect::<Result<_, _>>()?;
        if ids.len() == paths.len() {
            Ok(ids)
        } else {
            Err(odd_output("hash-object", &output))
        }
    }
}

impl GitRead for CommandReader {
    fn resolve(&self, name: &str) -> Result<ObjectId, GitError> {
        let unknown = || GitError::UnknownRevision {
            name: name.to_owned(),
        };
        if name.contains('\0') {
            return Err(unknown());
        }
        let mut command = self.git();
        command.args(["rev-parse", "--verify", "--quiet", "--end-of-options"]);
        command.arg(format!("{name}^{{commit}}"));
        let output = run(&mut command)?;
        match output.status.code() {
            Some(0) => parse_id(String::from_utf8_lossy(&output.stdout).trim_end()),
            Some(1) => Err(unknown()),
            _ => Err(failed("rev-parse", &output)),
        }
    }

    fn merge_bases(&self, a: &ObjectId, b: &ObjectId) -> Result<Vec<ObjectId>, GitError> {
        let mut command = self.git();
        command.args(["merge-base", "--all", a.as_hex(), b.as_hex()]);
        let output = run(&mut command)?;
        match output.status.code() {
            // `merge-base` exits with 1 when the commits share no history.
            Some(0 | 1) => String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(parse_id)
                .collect(),
            _ => Err(failed("merge-base", &output)),
        }
    }

    fn read_commit(&self, id: &ObjectId) -> Result<Vec<u8>, GitError> {
        self.read_object(id, ObjectKind::Commit)
    }

    fn markdown_files(&self, at: &Snapshot, roots: &Roots) -> Result<Vec<FileEntry>, GitError> {
        let Snapshot::Commit(commit) = at else {
            let workdir = self.workdir.as_deref().ok_or(GitError::NoWorktree)?;
            let candidates = self.worktree_candidates(roots)?;
            let paths: Vec<RepoPath> = candidates
                .into_iter()
                .filter(|path| {
                    roots.selects(path) && worktree::regular_file(workdir, path).is_some()
                })
                .collect();
            let ids = self.hash_files(&paths)?;
            let files = paths
                .into_iter()
                .zip(ids)
                .map(|(path, id)| FileEntry { path, id });
            return Ok(files.collect());
        };
        let mut command = self.git();
        command.args(["ls-tree", "-r", "-z", "--full-tree", commit.as_hex(), "--"]);
        let listing = run_ok(with_roots(&mut command, roots)?)?;
        let mut files = Vec::new();
        // "<mode> <type> <id>\t<path>"
        for record in records(&listing) {
            let (meta, path) = split_tab(record)?;
            let fields: Vec<&[u8]> = meta.split(|&b| b == b' ').collect();
            let [mode, _kind, id] = fields.as_slice() else {
                return Err(odd_output("ls-tree", record));
            };
            let path = repo_path(path)?;
            if is_regular_file(mode) && roots.selects(&path) {
                files.push(FileEntry {
                    path,
                    id: parse_id(&String::from_utf8_lossy(id))?,
                });
            }
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(files)
    }

    fn markdown_changes(
        &self,
        base: &ObjectId,
        head: &ObjectId,
        roots: &Roots,
    ) -> Result<Vec<FileChange>, GitError> {
        let mut command = self.git();
        command.args([
            "diff-tree",
            "-r",
            "-z",
            "--no-renames",
            base.as_hex(),
            head.as_hex(),
            "--",
        ]);
        let listing = run_ok(with_roots(&mut command, roots)?)?;
        let mut changes = Vec::new();
        // ":<old mode> <new mode> <old id> <new id> <status>" then the path.
        let mut fields = records(&listing);
        while let Some(meta) = fields.next() {
            let path = repo_path(fields.next().ok_or_else(|| odd_output("diff-tree", meta))?)?;
            let meta = meta.strip_prefix(b":").unwrap_or(meta);
            let parts: Vec<&[u8]> = meta.split(|&b| b == b' ').collect();
            let [old_mode, new_mode, old_id, new_id, _status] = parts.as_slice() else {
                return Err(odd_output("diff-tree", meta));
            };
            if !roots.selects(&path) {
                continue;
            }
            let side = |mode: &[u8], id: &[u8]| -> Result<Option<ObjectId>, GitError> {
                let id = String::from_utf8_lossy(id);
                let absent = !is_regular_file(mode) || id.bytes().all(|b| b == b'0');
                if absent {
                    Ok(None)
                } else {
                    parse_id(&id).map(Some)
                }
            };
            let (old, new) = (side(old_mode, old_id)?, side(new_mode, new_id)?);
            changes.extend(FileChange::between(path, old, new));
        }
        changes.sort_by(|a, b| a.path().cmp(b.path()));
        Ok(changes)
    }

    fn blob_at(&self, commit: &ObjectId, path: &RepoPath) -> Result<Option<ObjectId>, GitError> {
        let mut command = self.git();
        command.args(["ls-tree", "-z", "--full-tree", commit.as_hex(), "--"]);
        command.arg(path_arg(path)?);
        let listing = run_ok(&mut command)?;
        // "<mode> <type> <id>\t<path>": the entry itself, even when it is a
        // folder; nothing when no entry has that path.
        for record in records(&listing) {
            let (meta, found) = split_tab(record)?;
            let fields: Vec<&[u8]> = meta.split(|&b| b == b' ').collect();
            let [mode, _kind, id] = fields.as_slice() else {
                return Err(odd_output("ls-tree", record));
            };
            if found == path.as_bytes() && is_regular_file(mode) {
                return parse_id(&String::from_utf8_lossy(id)).map(Some);
            }
        }
        Ok(None)
    }

    fn read_blob(&self, id: &ObjectId) -> Result<Vec<u8>, GitError> {
        self.read_object(id, ObjectKind::Blob)
    }

    fn read_worktree_file(&self, path: &RepoPath) -> Result<Vec<u8>, GitError> {
        worktree::read(self.workdir.as_deref(), path)
    }
}

/// One `git cat-file --batch` process. It answers one object at a time:
/// the id goes in, a header line and the bytes come out.
struct CatFile {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}

impl CatFile {
    fn start(mut command: Command) -> Result<Self, GitError> {
        command.args(["cat-file", "--batch"]);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command
            .spawn()
            .map_err(|error| other(format!("could not start git cat-file: {error}")))?;
        let pipes = child.stdin.take().zip(child.stdout.take());
        let Some((input, output)) = pipes else {
            return Err(other("git cat-file has no pipes".to_owned()));
        };
        Ok(Self {
            child,
            input,
            output: BufReader::new(output),
        })
    }

    /// The kind and bytes of an object, or `None` when it is missing.
    fn read(&mut self, id: &ObjectId) -> std::io::Result<Option<(String, Vec<u8>)>> {
        writeln!(self.input, "{id}")?;
        self.input.flush()?;
        let mut header = String::new();
        self.output.read_line(&mut header)?;
        let fields: Vec<&str> = header.trim_end().split(' ').collect();
        match fields.as_slice() {
            [_, "missing"] => Ok(None),
            [_, kind, size] => {
                let size = size.parse().map_err(|_| bad_header(&header))?;
                let mut bytes = vec![0; size];
                self.output.read_exact(&mut bytes)?;
                let mut newline = [0];
                self.output.read_exact(&mut newline)?;
                Ok(Some(((*kind).to_owned(), bytes)))
            }
            _ => Err(bad_header(&header)),
        }
    }
}

impl Drop for CatFile {
    fn drop(&mut self) {
        // The input pipe is still open here, so the process would wait for
        // more ids; stop it instead. Errors mean it has already gone.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn bad_header(header: &str) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!("unexpected answer `{}`", header.trim_end()),
    )
}

/// A `git` command run in `folder`, cleared of variables that would make it
/// read another repo, and never allowed to take a lock or ask for a password.
fn git_in(folder: &Path) -> Command {
    let mut command = Command::new("git");
    command.arg("-C").arg(folder);
    for name in FOREIGN_ENV {
        command.env_remove(name);
    }
    command
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0");
    command.stdin(Stdio::null());
    command
}

/// Adds `-- <roots>` limits, or nothing for the whole repo. The command must
/// already end with `--` when roots follow it.
fn with_roots<'a>(command: &'a mut Command, roots: &Roots) -> Result<&'a mut Command, GitError> {
    for root in roots.folders() {
        command.arg(path_arg(root)?);
    }
    Ok(command)
}

fn path_arg(path: &RepoPath) -> Result<&OsStr, GitError> {
    worktree::os_str(path.as_bytes())
        .ok_or_else(|| other(format!("`{}` is not a valid path here", path.to_lossy())))
}

fn run(command: &mut Command) -> Result<Output, GitError> {
    command
        .output()
        .map_err(|error| other(format!("could not run git: {error}")))
}

/// Runs a command and returns its output, or an error carrying git's message.
fn run_ok(command: &mut Command) -> Result<Vec<u8>, GitError> {
    let output = run(command)?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        // The subcommand: the first word after `-C <folder>` that is no option.
        let mut words = command.get_args().skip(2).map(OsStr::to_string_lossy);
        let what = words
            .find(|word| !word.starts_with('-'))
            .unwrap_or_default();
        Err(failed(&what, &output))
    }
}

/// Runs a command with `input` on its standard input. A thread writes the
/// input while this one reads the output, so neither side waits on a full
/// pipe.
fn run_with_input(mut command: Command, input: Vec<u8>) -> Result<Vec<u8>, GitError> {
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| other(format!("could not run git: {error}")))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| other("git has no input pipe".to_owned()))?;
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let output = child.wait_with_output();
    let written = writer.join();
    let output = output.map_err(|error| other(format!("git failed: {error}")))?;
    if !output.status.success() {
        return Err(failed("hash-object", &output));
    }
    match written {
        Ok(Ok(())) => Ok(output.stdout),
        _ => Err(other("could not send paths to git".to_owned())),
    }
}

/// Writes a path the way git quotes paths in C style, which
/// `git hash-object --stdin-paths` reads back. Quoting every path keeps names
/// with a newline or a quote intact.
fn quote_c_style(path: &[u8], out: &mut Vec<u8>) {
    out.push(b'"');
    for &byte in path {
        match byte {
            b'"' | b'\\' => out.extend([b'\\', byte]),
            b'\n' => out.extend(b"\\n"),
            b'\t' => out.extend(b"\\t"),
            0..0x20 | 0x7f => out.extend(format!("\\{byte:03o}").bytes()),
            _ => out.push(byte),
        }
    }
    out.push(b'"');
}

/// The records of `-z` output, without the empty one after the last NUL.
fn records(output: &[u8]) -> impl Iterator<Item = &[u8]> {
    output
        .split(|&b| b == 0)
        .filter(|record| !record.is_empty())
}

fn split_tab(record: &[u8]) -> Result<(&[u8], &[u8]), GitError> {
    let tab = record.iter().position(|&b| b == b'\t');
    tab.map(|at| (&record[..at], &record[at + 1..]))
        .ok_or_else(|| odd_output("ls", record))
}

/// Regular files only: the type bits of the mode say "file", which covers
/// 100644, 100755 and old modes such as 100664. Symbolic links (120000) and
/// submodules (160000) are not documents. This is gix's rule too.
fn is_regular_file(mode: &[u8]) -> bool {
    let mode = std::str::from_utf8(mode)
        .ok()
        .and_then(|m| u32::from_str_radix(m, 8).ok());
    mode.is_some_and(|m| m & 0o170_000 == 0o100_000)
}

fn repo_path(bytes: &[u8]) -> Result<RepoPath, GitError> {
    RepoPath::new(bytes).ok_or_else(|| odd_output("a path", bytes))
}

fn parse_id(hex: &str) -> Result<ObjectId, GitError> {
    ObjectId::from_hex(hex)
        .ok_or_else(|| other(format!("git gave `{hex}` where an object id belongs")))
}

/// A path git printed, without its line ending.
fn output_path(bytes: &[u8]) -> Result<PathBuf, GitError> {
    worktree::os_str(bytes)
        .map(PathBuf::from)
        .ok_or_else(|| odd_output("rev-parse", bytes))
}

fn other(message: String) -> GitError {
    GitError::Other { message }
}

fn odd_output(what: &str, bytes: &[u8]) -> GitError {
    other(format!(
        "unexpected output from git {what}: `{}`",
        String::from_utf8_lossy(bytes)
    ))
}

fn failed(what: &str, output: &Output) -> GitError {
    let stderr = String::from_utf8_lossy(&output.stderr);
    other(format!("git {what} failed: {}", stderr.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_quoted_for_git() {
        let mut out = Vec::new();
        quote_c_style(b"a \"b\"\\c\nd\te\x01\x7f\xe9.md", &mut out);
        assert_eq!(out, b"\"a \\\"b\\\"\\\\c\\nd\\te\\001\\177\xe9.md\"");
    }

    #[test]
    fn only_regular_file_modes_count() {
        for yes in [&b"100644"[..], b"100755", b"100664"] {
            assert!(is_regular_file(yes));
        }
        for no in [
            &b"120000"[..],
            b"160000",
            b"040000",
            b"000000",
            b"",
            b"x",
            b"\xff",
        ] {
            assert!(!is_regular_file(no));
        }
    }
}
