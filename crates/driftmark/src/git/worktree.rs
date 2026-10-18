//! Files on disk, shared by both readers so they decide alike.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use driftmark_core::git::{GitError, RepoPath};

/// The bytes of a repo path as an OS path part. On Unix any bytes work. On
/// Windows paths are Unicode, so a name that is not UTF-8 cannot exist there.
#[cfg(unix)]
#[expect(clippy::unnecessary_wraps, reason = "the Windows version can fail")]
pub(crate) fn os_str(bytes: &[u8]) -> Option<&OsStr> {
    use std::os::unix::ffi::OsStrExt;
    Some(OsStr::from_bytes(bytes))
}

/// The bytes of a repo path as an OS path part. On Unix any bytes work. On
/// Windows paths are Unicode, so a name that is not UTF-8 cannot exist there.
#[cfg(not(unix))]
pub(crate) fn os_str(bytes: &[u8]) -> Option<&OsStr> {
    std::str::from_utf8(bytes).ok().map(OsStr::new)
}

/// Where a repo path is on disk, if a regular file is there and every folder
/// on the way is a real folder. A symbolic link anywhere on the way counts as
/// no file, so a link in the working tree cannot make driftmark read a file
/// outside the repo.
pub(crate) fn regular_file(workdir: &Path, path: &RepoPath) -> Option<PathBuf> {
    let mut full = workdir.to_path_buf();
    let mut parts = path.as_bytes().split(|&b| b == b'/').peekable();
    while let Some(part) = parts.next() {
        full.push(os_str(part)?);
        let kind = std::fs::symlink_metadata(&full).ok()?.file_type();
        let wanted = if parts.peek().is_some() {
            kind.is_dir()
        } else {
            kind.is_file()
        };
        if !wanted {
            return None;
        }
    }
    Some(full)
}

/// The bytes of a working-tree file, for [`driftmark_core::git::GitRead::read_worktree_file`].
pub(crate) fn read(workdir: Option<&Path>, path: &RepoPath) -> Result<Vec<u8>, GitError> {
    let workdir = workdir.ok_or(GitError::NoWorktree)?;
    let missing = || GitError::MissingFile { path: path.clone() };
    let full = regular_file(workdir, path).ok_or_else(missing)?;
    std::fs::read(full).map_err(|error| GitError::Other {
        message: format!("could not read `{}`: {error}", path.to_lossy()),
    })
}
