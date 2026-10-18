//! Which files and headings exist in a working tree, for the lint's link
//! checks.

use std::collections::{BTreeSet, HashMap};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use driftmark_core::document::Document;
use driftmark_core::lint::{Anchors, LinkTargets};

/// The files of a working tree on disk.
///
/// A name must match exactly, case included, on every system. GitHub and
/// Linux treat `Guide.md` and `guide.md` as two files, so a link that only
/// works on a Mac or Windows disk, which ignore case, is still broken. Each
/// folder is listed once and the listing is kept, as are the anchors of each
/// Markdown file read.
pub struct FileTargets {
    root: PathBuf,
    folders: Mutex<HashMap<PathBuf, Option<Arc<BTreeSet<OsString>>>>>,
    anchors: Mutex<HashMap<String, Option<Arc<Anchors>>>>,
}

impl FileTargets {
    /// Targets inside the working tree at `root`.
    #[must_use]
    pub fn new(root: &Path) -> FileTargets {
        FileTargets {
            root: root.to_path_buf(),
            folders: Mutex::new(HashMap::new()),
            anchors: Mutex::new(HashMap::new()),
        }
    }

    /// The names in one folder, or `None` when it is not a folder.
    fn names(&self, folder: &Path) -> Option<Arc<BTreeSet<OsString>>> {
        let cached = self
            .folders
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(folder)
            .cloned();
        if let Some(names) = cached {
            return names;
        }
        let names = std::fs::read_dir(folder).ok().map(|entries| {
            Arc::new(
                entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.file_name())
                    .collect(),
            )
        });
        self.folders
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(folder.to_path_buf(), names.clone());
        names
    }
}

impl LinkTargets for FileTargets {
    fn exists(&self, path: &str) -> bool {
        let mut folder = self.root.clone();
        for part in path.split('/') {
            let Some(names) = self.names(&folder) else {
                return false;
            };
            if !names.contains(OsStr::new(part)) {
                return false;
            }
            folder.push(part);
        }
        true
    }

    fn anchors(&self, path: &str) -> Option<Arc<Anchors>> {
        let cached = self
            .anchors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(path)
            .cloned();
        if let Some(anchors) = cached {
            return anchors;
        }
        let anchors = self
            .exists(path)
            .then(|| std::fs::read(self.root.join(path)).ok())
            .flatten()
            .and_then(|bytes| Document::parse(bytes).ok())
            .map(|document| Arc::new(Anchors::of(&document)));
        self.anchors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(path.to_owned(), anchors.clone());
        anchors
    }
}
