//! The reader's notes, kept in one SQLite file per clone (decision 0004).
//!
//! - [`notes_file`] says where the file is: `<git common dir>/driftmark/notes.db`,
//!   inside the repository's shared git folder, so every worktree of a clone,
//!   and every agent working in one, sees the same notes, and git never
//!   commits them.
//! - [`store`] reads and writes the file.
//! - [`place`] finds each note again in a version of its document, with the
//!   rules in `driftmark_core::notes::anchor`.
//!
//! `driftmark serve` and `driftmark mcp` use all three. The rules of
//! the notes loop and the anchoring live in the core.

pub mod store;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use driftmark_core::diff::{BlockMap, block_map};
use driftmark_core::document::Document;
use driftmark_core::notes::{Note, Origin, Placement, reanchor};

pub use store::{Act, NewNote, NoteEdit, Review, Scope, SeenMark, Store, StoreError, Via};

/// The notes file's name inside the `driftmark` folder of the git common
/// directory.
pub const FILE_NAME: &str = "notes.db";

/// The folder, beside the notes file, where review files go by default.
pub const REVIEWS: &str = "reviews";

/// Where the notes file of the repository that holds `start` is: the
/// `driftmark` folder in the git common directory, the one
/// `git rev-parse --git-common-dir` names. For a linked worktree that is the
/// main clone's git folder, so all worktrees share it.
///
/// # Errors
///
/// When `start` is not inside a git repository.
pub fn notes_file(start: &Path) -> Result<PathBuf, String> {
    let common = match gix::discover(start) {
        Ok(repo) => repo.common_dir().to_path_buf(),
        Err(gix_error) => common_dir_by_command(start)
            .map_err(|error| format!("cannot find the git folder: {gix_error}; {error}"))?,
    };
    // A linked worktree names it as `<clone>/.git/worktrees/<name>/../..`;
    // the real folder reads better in messages.
    let common = std::fs::canonicalize(&common)
        .or_else(|_| std::path::absolute(&common))
        .unwrap_or(common);
    Ok(common.join("driftmark").join(FILE_NAME))
}

/// The git common directory as the `git` command reports it.
fn common_dir_by_command(start: &Path) -> Result<PathBuf, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(start)
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .output()
        .map_err(|error| format!("git could not start: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    let text = String::from_utf8(output.stdout)
        .map_err(|_| "git named a folder that is not valid text".to_owned())?;
    Ok(PathBuf::from(text.trim_end_matches(['\n', '\r'])))
}

/// The id git gives a blob with these bytes, as 40 hex digits. A note keeps
/// it as the version it was made on, so a later version with the same bytes
/// needs no re-anchoring.
#[must_use]
pub fn blob_id(bytes: &[u8]) -> String {
    gix::objs::compute_hash(gix::hash::Kind::Sha1, gix::objs::Kind::Blob, bytes)
        .map_or_else(|_| String::new(), |id| id.to_hex().to_string())
}

/// Finds each note again in `document`, the version whose bytes have the
/// blob id `version`.
///
/// `old` returns the version a note was made on, by its blob id, when it can
/// still be read; each is asked for once. A note on that same version stays
/// where it was. A note whose version is lost is searched for by its quote
/// and headings alone.
pub fn place(
    notes: &[Note],
    document: &Document,
    version: &str,
    mut old: impl FnMut(&str) -> Option<Arc<Document>>,
) -> Vec<Placement> {
    // Each old version is read, and paired with this one, once.
    let mut versions: HashMap<String, Option<(Arc<Document>, BlockMap)>> = HashMap::new();
    notes
        .iter()
        .map(|note| {
            let blob = note.target.blob.as_str();
            if blob == version {
                return reanchor(&note.target.anchor, Origin::Same, document);
            }
            let found = versions.entry(blob.to_owned()).or_insert_with(|| {
                old(blob).map(|old| {
                    let map = block_map(&old, document);
                    (old, map)
                })
            });
            let origin = found
                .as_ref()
                .map_or(Origin::Lost, |(old, map)| Origin::Paired(old, map));
            reanchor(&note.target.anchor, origin, document)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_ids_are_the_ones_git_gives() {
        // `printf 'hello\n' | git hash-object --stdin`
        assert_eq!(
            blob_id(b"hello\n"),
            "ce013625030ba8dba906f756967f9e9ca394464a"
        );
        assert_eq!(blob_id(b""), "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391");
    }
}
