//! The reader that uses gix, in this process.
//!
//! The speed spike found it faster than the `git` command at every step, and
//! it starts no processes, which matters most on Windows and in a server.

use std::collections::BTreeSet;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Mutex, PoisonError};

use driftmark_core::git::{
    Branch, FileChange, FileEntry, GitError, GitRead, ObjectId, ObjectKind, RepoPath, Roots,
    Snapshot,
};
use gix::bstr::ByteSlice;
use gix::diff::tree::recorder::Change as Record;
use gix::objs::TreeRefIter;

use super::worktree;

/// Bytes of decompressed pack data each repo handle keeps. The speed spike
/// measured no loss against gix's default cache at this size, and a cache
/// capped by bytes avoids the default cache's MPL-2.0 dependency.
const PACK_CACHE_BYTES: usize = 16 * 1024 * 1024;

/// Bytes of recently read objects each handle keeps. Walks over commits, as
/// for merge-bases and `--since`, read the same commits and trees again.
const OBJECT_CACHE_BYTES: usize = 4 * 1024 * 1024;

/// The directory walk can be stopped by setting a flag; driftmark never does.
static NEVER_INTERRUPT: AtomicBool = AtomicBool::new(false);

/// Reads a repo with gix.
///
/// A gix repo handle is not shared between threads, so this keeps a few
/// spare handles and lends one to each call. Many threads can read at once.
/// A spare handle keeps its caches, up to 20 MB, so after work on many
/// threads the caller drops the spare ones with [`GixReader::drop_spare`].
pub struct GixReader {
    repo: gix::ThreadSafeRepository,
    spare: Mutex<Vec<gix::Repository>>,
}

impl GixReader {
    /// Opens the repo that holds `start`, which may be any folder inside it.
    ///
    /// # Errors
    ///
    /// When `start` is not inside a repo, or gix cannot read this repo: it
    /// reads SHA-1 repos only, and not reftable.
    pub fn open(start: &Path) -> Result<Self, GitError> {
        let repo = gix::discover(start).map_err(|error| other(&error))?;
        Ok(Self {
            repo: repo.into_sync(),
            spare: Mutex::new(Vec::new()),
        })
    }

    /// The top of the working tree, or `None` for a bare repo.
    #[must_use]
    pub fn workdir(&self) -> Option<&Path> {
        self.repo.work_dir()
    }

    /// How many handles wait for reuse: about one for each thread that read
    /// while others did, since the last [`GixReader::drop_spare`].
    #[must_use]
    pub fn spare(&self) -> usize {
        self.spare
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    /// Drops the handles that wait for reuse, with their caches. A handle
    /// lent out now is kept; the next read makes a new one. Work on every
    /// core leaves one handle per thread, holding up to 20 MB each, which a
    /// long-running server would keep for nothing (decision 0011).
    pub fn drop_spare(&self) {
        let dropped =
            std::mem::take(&mut *self.spare.lock().unwrap_or_else(PoisonError::into_inner));
        // The caches are freed here, after the lock is let go, so a read on
        // another thread does not wait for them.
        drop(dropped);
    }

    /// Runs `f` with a repo handle of this thread's own.
    fn with<T>(&self, f: impl FnOnce(&gix::Repository) -> T) -> T {
        let spare = self
            .spare
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop();
        let repo = spare.unwrap_or_else(|| {
            let mut repo = self.repo.to_thread_local();
            repo.object_cache_size(OBJECT_CACHE_BYTES);
            repo.objects.set_pack_cache(|| {
                Box::new(gix::odb::pack::cache::lru::MemoryCappedHashmap::new(
                    PACK_CACHE_BYTES,
                ))
            });
            repo
        });
        let answer = f(&repo);
        self.spare
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(repo);
        answer
    }

    fn read_object(&self, id: &ObjectId, expected: ObjectKind) -> Result<Vec<u8>, GitError> {
        let kind = match expected {
            ObjectKind::Blob => gix::objs::Kind::Blob,
            ObjectKind::Commit => gix::objs::Kind::Commit,
        };
        self.with(|repo| match repo.try_find_object(to_gix(id)?) {
            Ok(Some(object)) if object.kind == kind => Ok(object.detach().data),
            Ok(Some(_)) => Err(GitError::WrongKind {
                id: id.clone(),
                expected,
            }),
            Ok(None) => Err(GitError::MissingObject { id: id.clone() }),
            Err(error) => Err(other(&error)),
        })
    }
}

impl GitRead for GixReader {
    fn resolve(&self, name: &str) -> Result<ObjectId, GitError> {
        let unknown = |_| GitError::UnknownRevision {
            name: name.to_owned(),
        };
        self.with(|repo| {
            let id = repo.rev_parse_single(name).map_err(unknown)?;
            let commit = id
                .object()
                .map_err(unknown)?
                .peel_to_commit()
                .map_err(unknown)?;
            from_gix(commit.id)
        })
    }

    fn merge_bases(&self, a: &ObjectId, b: &ObjectId) -> Result<Vec<ObjectId>, GitError> {
        let (a, b) = (to_gix(a)?, to_gix(b)?);
        self.with(|repo| {
            let bases = repo
                .merge_bases_many(a, &[b])
                .map_err(|error| other(&error))?;
            bases.into_iter().map(|id| from_gix(id.detach())).collect()
        })
    }

    fn read_commit(&self, id: &ObjectId) -> Result<Vec<u8>, GitError> {
        self.read_object(id, ObjectKind::Commit)
    }

    fn markdown_files(&self, at: &Snapshot, roots: &Roots) -> Result<Vec<FileEntry>, GitError> {
        let mut files = match at {
            Snapshot::Commit(commit) => {
                let commit = to_gix(commit)?;
                self.with(|repo| files_at_commit(repo, commit, roots))?
            }
            Snapshot::Worktree => self.with(|repo| files_in_worktree(repo, roots))?,
        };
        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(files)
    }

    fn markdown_changes(
        &self,
        base: &ObjectId,
        head: &ObjectId,
        roots: &Roots,
    ) -> Result<Vec<FileChange>, GitError> {
        let (base, head) = (to_gix(base)?, to_gix(head)?);
        let mut changes = self.with(|repo| changes_between(repo, base, head, roots))?;
        changes.sort_by(|a, b| a.path().cmp(b.path()));
        Ok(changes)
    }

    fn blob_at(&self, commit: &ObjectId, path: &RepoPath) -> Result<Option<ObjectId>, GitError> {
        let commit = to_gix(commit)?;
        let found = self.with(|repo| blob_in_tree(repo, commit_tree(repo, commit)?, path))?;
        found.map(from_gix).transpose()
    }

    fn branches(&self) -> Result<Vec<Branch>, GitError> {
        let mut branches = self.with(list_branches)?;
        branches.sort_by(|a, b| (a.remote, &a.name).cmp(&(b.remote, &b.name)));
        Ok(branches)
    }

    fn read_blob(&self, id: &ObjectId) -> Result<Vec<u8>, GitError> {
        self.read_object(id, ObjectKind::Blob)
    }

    fn read_worktree_file(&self, path: &RepoPath) -> Result<Vec<u8>, GitError> {
        worktree::read(self.workdir(), path)
    }
}

/// The local and remote-tracking branches that point straight at a commit.
/// A ref gix cannot read is skipped, as `git for-each-ref` skips a broken
/// ref with a warning.
fn list_branches(repo: &gix::Repository) -> Result<Vec<Branch>, GitError> {
    let head = repo.head_name().map_err(|error| other(&error))?;
    let refs = repo.references().map_err(|error| other(&error))?;
    let local = refs.local_branches().map_err(|error| other(&error))?;
    let remote = refs.remote_branches().map_err(|error| other(&error))?;
    let mut branches = Vec::new();
    for (is_remote, prefix, iter) in [
        (false, &b"refs/heads/"[..], local),
        (true, &b"refs/remotes/"[..], remote),
    ] {
        for reference in iter.flatten() {
            let gix::refs::TargetRef::Object(tip) = reference.target() else {
                continue;
            };
            let header = repo.find_header(tip).map_err(|error| other(&error))?;
            if header.kind() != gix::objs::Kind::Commit {
                continue;
            }
            let full = reference.name().as_bstr();
            let short = full
                .strip_prefix(prefix)
                .and_then(|n| std::str::from_utf8(n).ok());
            let Some(name) = short else {
                continue;
            };
            branches.push(Branch {
                name: name.to_owned(),
                remote: is_remote,
                tip: from_gix(tip.to_owned())?,
                current: head.as_ref().is_some_and(|h| h.as_bstr() == full),
            });
        }
    }
    Ok(branches)
}

/// The blob of the regular file at `path` under `tree`. It walks one folder
/// at a time and stops at the first part that is not a folder, so a path
/// through a file or a submodule finds nothing, as `git ls-tree` does. gix's
/// own lookup would instead try to read a submodule's commit, which is not in
/// the repo.
fn blob_in_tree(
    repo: &gix::Repository,
    mut tree: gix::ObjectId,
    path: &RepoPath,
) -> Result<Option<gix::ObjectId>, GitError> {
    let mut parts = path.as_bytes().split(|&b| b == b'/').peekable();
    while let Some(part) = parts.next() {
        let data = repo.find_tree(tree).map_err(|error| other(&error))?;
        let mut found = None;
        for entry in TreeRefIter::from_bytes(&data.data, repo.object_hash()) {
            let entry = entry.map_err(|error| other(&error))?;
            if entry.filename.as_bytes() == part {
                found = Some((entry.mode, entry.oid.to_owned()));
                break;
            }
        }
        let Some((mode, id)) = found else {
            return Ok(None);
        };
        if parts.peek().is_none() {
            return Ok(mode.is_blob().then_some(id));
        }
        if !mode.is_tree() {
            return Ok(None);
        }
        tree = id;
    }
    Ok(None)
}

/// A root folder's path, and its tree if the folder exists.
type RootTree = (Vec<u8>, Option<gix::ObjectId>);

/// The tree of a commit.
fn commit_tree(repo: &gix::Repository, commit: gix::ObjectId) -> Result<gix::ObjectId, GitError> {
    let commit = repo.find_commit(commit).map_err(|error| other(&error))?;
    Ok(commit.tree_id().map_err(|error| other(&error))?.detach())
}

/// The trees to read: one per root folder, with the folder's path, or the
/// whole tree when there are no roots. A root that is missing, or is a file,
/// has no tree.
fn root_trees(
    repo: &gix::Repository,
    tree: gix::ObjectId,
    roots: &Roots,
) -> Result<Vec<RootTree>, GitError> {
    if roots.folders().is_empty() {
        return Ok(vec![(Vec::new(), Some(tree))]);
    }
    let tree = repo.find_tree(tree).map_err(|error| other(&error))?;
    let mut found = Vec::new();
    for root in roots.folders() {
        let parts = root.as_bytes().split(|&b| b == b'/');
        let entry = tree.lookup_entry(parts).map_err(|error| other(&error))?;
        let subtree = entry.filter(|e| e.mode().is_tree()).map(|e| e.object_id());
        found.push((root.as_bytes().to_vec(), subtree));
    }
    Ok(found)
}

/// Joins a folder path and a path inside it.
fn join(prefix: &[u8], name: &[u8]) -> Vec<u8> {
    if prefix.is_empty() {
        name.to_vec()
    } else {
        [prefix, b"/", name].concat()
    }
}

fn files_at_commit(
    repo: &gix::Repository,
    commit: gix::ObjectId,
    roots: &Roots,
) -> Result<Vec<FileEntry>, GitError> {
    let mut files = Vec::new();
    let mut folders: Vec<(Vec<u8>, gix::ObjectId)> =
        root_trees(repo, commit_tree(repo, commit)?, roots)?
            .into_iter()
            .filter_map(|(prefix, tree)| tree.map(|tree| (prefix, tree)))
            .collect();
    while let Some((prefix, tree)) = folders.pop() {
        let tree = repo.find_tree(tree).map_err(|error| other(&error))?;
        for entry in TreeRefIter::from_bytes(&tree.data, repo.object_hash()) {
            let entry = entry.map_err(|error| other(&error))?;
            let path = join(&prefix, entry.filename);
            if entry.mode.is_tree() {
                folders.push((path, entry.oid.to_owned()));
            } else if entry.mode.is_blob() {
                let path = repo_path(path)?;
                if roots.selects(&path) {
                    files.push(FileEntry {
                        path,
                        id: from_gix(entry.oid.to_owned())?,
                    });
                }
            }
        }
    }
    Ok(files)
}

fn changes_between(
    repo: &gix::Repository,
    base: gix::ObjectId,
    head: gix::ObjectId,
    roots: &Roots,
) -> Result<Vec<FileChange>, GitError> {
    let empty = gix::ObjectId::empty_tree(repo.object_hash());
    let old_roots = root_trees(repo, commit_tree(repo, base)?, roots)?;
    let new_roots = root_trees(repo, commit_tree(repo, head)?, roots)?;
    let mut changes = Vec::new();
    for ((prefix, old), (_, new)) in old_roots.into_iter().zip(new_roots) {
        let (old, new) = (old.unwrap_or(empty), new.unwrap_or(empty));
        if old == new {
            continue;
        }
        let old = repo.find_tree(old).map_err(|error| other(&error))?;
        let new = repo.find_tree(new).map_err(|error| other(&error))?;
        // Rename tracking stays off: a moved file is a removal and an addition.
        let mut recorder = gix::diff::tree::Recorder::default();
        gix::diff::tree(
            TreeRefIter::from_bytes(&old.data, repo.object_hash()),
            TreeRefIter::from_bytes(&new.data, repo.object_hash()),
            gix::diff::tree::State::default(),
            &repo.objects,
            &mut recorder,
        )
        .map_err(|error| other(&error))?;
        for record in recorder.records {
            let (path, old, new) = match record {
                Record::Addition {
                    entry_mode,
                    oid,
                    path,
                    ..
                } => (path, None, blob(entry_mode, oid)),
                Record::Deletion {
                    entry_mode,
                    oid,
                    path,
                    ..
                } => (path, blob(entry_mode, oid), None),
                Record::Modification {
                    previous_entry_mode,
                    previous_oid,
                    entry_mode,
                    oid,
                    path,
                } => (
                    path,
                    blob(previous_entry_mode, previous_oid),
                    blob(entry_mode, oid),
                ),
            };
            let path = repo_path(join(&prefix, &path))?;
            if roots.selects(&path) {
                let (old, new) = (
                    old.map(from_gix).transpose()?,
                    new.map(from_gix).transpose()?,
                );
                changes.extend(FileChange::between(path, old, new));
            }
        }
    }
    Ok(changes)
}

/// The blob of a tree entry, if the entry is a regular file.
fn blob(mode: gix::objs::tree::EntryMode, id: gix::ObjectId) -> Option<gix::ObjectId> {
    mode.is_blob().then_some(id)
}

fn files_in_worktree(repo: &gix::Repository, roots: &Roots) -> Result<Vec<FileEntry>, GitError> {
    let workdir = repo.workdir().ok_or(GitError::NoWorktree)?.to_path_buf();
    let (mut pipeline, index) = repo.filter_pipeline(None).map_err(|error| other(&error))?;

    let mut paths = BTreeSet::new();
    for entry in index.entries() {
        let regular = matches!(
            entry.mode,
            gix::index::entry::Mode::FILE | gix::index::entry::Mode::FILE_EXECUTABLE
        );
        if regular {
            paths.insert(repo_path(entry.path(&index).to_vec())?);
        }
    }
    let patterns: Vec<gix::bstr::BString> = roots
        .folders()
        .iter()
        .map(|root| [&b":(literal)"[..], root.as_bytes()].concat().into())
        .collect();
    let options = repo
        .dirwalk_options()
        .map_err(|error| other(&error))?
        .emit_untracked(gix::dir::walk::EmissionMode::Matching)
        .emit_ignored(None)
        .emit_tracked(false)
        .recurse_repositories(false);
    let walk = repo
        .dirwalk_iter(index.clone(), patterns, (&NEVER_INTERRUPT).into(), options)
        .map_err(|error| other(&error))?;
    for item in walk {
        let entry = item.map_err(|error| other(&error))?.entry;
        let untracked_file = entry.status == gix::dir::entry::Status::Untracked
            && entry.disk_kind == Some(gix::dir::entry::Kind::File);
        if untracked_file {
            paths.insert(repo_path(entry.rela_path.to_vec())?);
        }
    }

    let mut files = Vec::new();
    for path in paths {
        if !roots.selects(&path) {
            continue;
        }
        let Some(full) = worktree::regular_file(&workdir, &path) else {
            continue;
        };
        let file = std::fs::File::open(&full).map_err(|error| other(&error))?;
        let relative =
            gix::path::from_bstr(path.as_bytes().as_bstr()).map_err(|error| other(&error))?;
        let mut converted = pipeline
            .convert_to_git(file, &relative, &index)
            .map_err(|error| other(&error))?;
        let mut bytes = Vec::new();
        converted
            .read_to_end(&mut bytes)
            .map_err(|error| other(&error))?;
        let id = gix::objs::compute_hash(repo.object_hash(), gix::objs::Kind::Blob, &bytes)
            .map_err(|error| other(&error))?;
        files.push(FileEntry {
            path,
            id: from_gix(id)?,
        });
    }
    Ok(files)
}

fn to_gix(id: &ObjectId) -> Result<gix::ObjectId, GitError> {
    gix::ObjectId::from_hex(id.as_hex().as_bytes()).map_err(|error| other(&error))
}

fn from_gix(id: gix::ObjectId) -> Result<ObjectId, GitError> {
    let hex = id.to_hex().to_string();
    ObjectId::from_hex(&hex).ok_or_else(|| GitError::Other {
        message: format!("gix gave the id `{hex}`"),
    })
}

fn repo_path(bytes: Vec<u8>) -> Result<RepoPath, GitError> {
    let shown = String::from_utf8_lossy(&bytes).into_owned();
    RepoPath::new(bytes).ok_or_else(|| GitError::Other {
        message: format!("gix gave the path `{shown}`"),
    })
}

fn other(error: &dyn std::fmt::Display) -> GitError {
    GitError::Other {
        message: format!("gix: {error}"),
    }
}
