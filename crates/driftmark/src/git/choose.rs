//! The reader the commands use: gix first, the `git` command when gix fails.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use driftmark_core::git::{
    FileChange, FileEntry, GitError, GitRead, ObjectId, RepoPath, Roots, Snapshot,
};

use super::{CommandReader, GixReader};

/// Where the fallback's explanation goes: standard error in the program, a
/// list in tests.
type Tell = Box<dyn Fn(&str) + Send + Sync>;

/// Reads a repo with gix, and switches to the `git` command when gix cannot
/// open the repo or read an object.
///
/// gix cannot yet fetch a file a partial clone left out, and it does not read
/// SHA-256 or reftable repos; the `git` command can. The switch is said once,
/// with the reason, and lasts for the rest of the run. Answers that git would
/// give the same way, such as an unknown branch name, do not switch.
pub struct Git {
    start: PathBuf,
    gix: Option<GixReader>,
    command: OnceLock<Result<CommandReader, GitError>>,
    gix_failed: AtomicBool,
    tell: Tell,
}

impl Git {
    /// Opens the repo that holds `start`. A switch to the `git` command is
    /// explained on standard error.
    ///
    /// # Errors
    ///
    /// When neither gix nor the `git` command can open the repo.
    pub fn open(start: &Path) -> Result<Self, GitError> {
        Self::open_telling(start, Box::new(|message| eprintln!("driftmark: {message}")))
    }

    /// Like [`Git::open`], but a switch to the `git` command is explained by
    /// calling `tell` instead of writing to standard error.
    ///
    /// # Errors
    ///
    /// When neither gix nor the `git` command can open the repo.
    pub fn open_telling(start: &Path, tell: Tell) -> Result<Self, GitError> {
        let command = OnceLock::new();
        let gix = match GixReader::open(start) {
            Ok(gix) => Some(gix),
            Err(error) => {
                let opened = CommandReader::open(start)?;
                tell(&format!(
                    "gix cannot open the repo ({error}); using the git command instead"
                ));
                let _ = command.set(Ok(opened));
                None
            }
        };
        let gix_failed = AtomicBool::new(gix.is_none());
        Ok(Self {
            start: start.to_path_buf(),
            gix,
            command,
            gix_failed,
            tell,
        })
    }

    /// Which reader answers now: `"gix"` or `"git command"`.
    #[must_use]
    pub fn reader(&self) -> &'static str {
        if self.gix_failed.load(Ordering::Acquire) {
            "git command"
        } else {
            "gix"
        }
    }

    /// The repo's working tree, where its files are on disk; `None` for a bare
    /// repo.
    #[must_use]
    pub fn workdir(&self) -> Option<&Path> {
        match &self.gix {
            Some(gix) => gix.workdir(),
            None => self
                .command
                .get()
                .and_then(|opened| opened.as_ref().ok())
                .and_then(CommandReader::workdir),
        }
    }

    /// Asks gix, unless it failed before, then the `git` command if gix fails
    /// in a way the command may not.
    fn ask<T>(
        &self,
        what: &str,
        with_gix: impl FnOnce(&GixReader) -> Result<T, GitError>,
        with_command: impl FnOnce(&CommandReader) -> Result<T, GitError>,
    ) -> Result<T, GitError> {
        let gix = self
            .gix
            .as_ref()
            .filter(|_| !self.gix_failed.load(Ordering::Acquire));
        let gix_error = match gix.map(with_gix) {
            Some(Err(error)) if command_may_succeed(&error) => error,
            Some(answer) => return answer,
            None => return with_command(self.command()?),
        };
        let command = self.command().map_err(|_| gix_error.clone())?;
        if !self.gix_failed.swap(true, Ordering::AcqRel) {
            (self.tell)(&format!(
                "gix could not {what} ({gix_error}); using the git command from now on"
            ));
        }
        with_command(command)
    }

    fn command(&self) -> Result<&CommandReader, GitError> {
        let opened = self
            .command
            .get_or_init(|| CommandReader::open(&self.start));
        opened.as_ref().map_err(Clone::clone)
    }
}

/// Whether the `git` command may answer where gix failed. A missing object
/// may be fetched by git in a partial clone; any other failure may be a gap in
/// gix. Unknown names, wrong kinds and missing files are facts about the repo,
/// which git would report the same way.
fn command_may_succeed(error: &GitError) -> bool {
    matches!(
        error,
        GitError::MissingObject { .. } | GitError::Other { .. }
    )
}

impl GitRead for Git {
    fn resolve(&self, name: &str) -> Result<ObjectId, GitError> {
        self.ask("resolve a name", |g| g.resolve(name), |c| c.resolve(name))
    }

    fn merge_bases(&self, a: &ObjectId, b: &ObjectId) -> Result<Vec<ObjectId>, GitError> {
        self.ask(
            "find a merge-base",
            |g| g.merge_bases(a, b),
            |c| c.merge_bases(a, b),
        )
    }

    fn read_commit(&self, id: &ObjectId) -> Result<Vec<u8>, GitError> {
        self.ask(
            "read a commit",
            |g| g.read_commit(id),
            |c| c.read_commit(id),
        )
    }

    fn markdown_files(&self, at: &Snapshot, roots: &Roots) -> Result<Vec<FileEntry>, GitError> {
        self.ask(
            "list files",
            |g| g.markdown_files(at, roots),
            |c| c.markdown_files(at, roots),
        )
    }

    fn markdown_changes(
        &self,
        base: &ObjectId,
        head: &ObjectId,
        roots: &Roots,
    ) -> Result<Vec<FileChange>, GitError> {
        self.ask(
            "list changes",
            |g| g.markdown_changes(base, head, roots),
            |c| c.markdown_changes(base, head, roots),
        )
    }

    fn read_blob(&self, id: &ObjectId) -> Result<Vec<u8>, GitError> {
        self.ask("read a file", |g| g.read_blob(id), |c| c.read_blob(id))
    }

    fn read_worktree_file(&self, path: &RepoPath) -> Result<Vec<u8>, GitError> {
        self.ask(
            "read a working-tree file",
            |g| g.read_worktree_file(path),
            |c| c.read_worktree_file(path),
        )
    }
}
