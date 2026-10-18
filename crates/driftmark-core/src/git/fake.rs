//! An in-memory git for testing the rules in this module without a repo.

use std::collections::HashMap;

use super::{
    FileChange, FileEntry, GitError, GitRead, ObjectId, RepoPath, Roots, Snapshot, diff_files,
};

/// An id made of one repeated hex digit, such as `1111…`.
pub(crate) fn id(digit: char) -> ObjectId {
    ObjectId::from_hex(&digit.to_string().repeat(40)).unwrap()
}

/// A file entry with a one-digit blob id.
pub(crate) fn entry(path: &str, digit: char) -> FileEntry {
    FileEntry {
        path: RepoPath::new(path).unwrap(),
        id: id(digit),
    }
}

/// Answers set up by each test. Anything not set up is an error.
pub(crate) struct FakeGit {
    names: HashMap<String, Result<ObjectId, GitError>>,
    bases: Result<Vec<ObjectId>, GitError>,
    commits: HashMap<ObjectId, Vec<u8>>,
    files: HashMap<Option<ObjectId>, Vec<FileEntry>>,
}

impl FakeGit {
    pub(crate) fn new() -> Self {
        Self {
            names: HashMap::new(),
            bases: Ok(Vec::new()),
            commits: HashMap::new(),
            files: HashMap::new(),
        }
    }

    pub(crate) fn name(mut self, name: &str, digit: char) -> Self {
        self.names.insert(name.into(), Ok(id(digit)));
        self
    }

    pub(crate) fn fail_name(mut self, name: &str) -> Self {
        let error = GitError::Other {
            message: "broken ref".into(),
        };
        self.names.insert(name.into(), Err(error));
        self
    }

    pub(crate) fn bases(mut self, digits: &[char]) -> Self {
        self.bases = Ok(digits.iter().map(|&d| id(d)).collect());
        self
    }

    pub(crate) fn bases_fail(mut self) -> Self {
        self.bases = Err(GitError::Other {
            message: "no bases".into(),
        });
        self
    }

    /// A commit with these parents, committed at `time`.
    pub(crate) fn commit(self, digit: char, parents: &[char], time: i64) -> Self {
        let parents: Vec<String> = parents
            .iter()
            .map(|&p| format!("parent {}\n", id(p)))
            .collect();
        let parents = parents.concat();
        let raw = format!(
            "tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\n{parents}\
             author A <a@example.com> {time} +0000\n\
             committer C <c@example.com> {time} +0000\n\nmessage\n"
        );
        self.raw_commit(digit, raw.as_bytes())
    }

    pub(crate) fn raw_commit(mut self, digit: char, raw: &[u8]) -> Self {
        self.commits.insert(id(digit), raw.to_vec());
        self
    }

    pub(crate) fn files(mut self, commit: char, files: &[FileEntry]) -> Self {
        self.files.insert(Some(id(commit)), files.to_vec());
        self
    }

    pub(crate) fn worktree(mut self, files: &[FileEntry]) -> Self {
        self.files.insert(None, files.to_vec());
        self
    }
}

impl GitRead for FakeGit {
    fn resolve(&self, name: &str) -> Result<ObjectId, GitError> {
        let unknown = || Err(GitError::UnknownRevision { name: name.into() });
        self.names.get(name).cloned().unwrap_or_else(unknown)
    }

    fn merge_bases(&self, _: &ObjectId, _: &ObjectId) -> Result<Vec<ObjectId>, GitError> {
        self.bases.clone()
    }

    fn read_commit(&self, id: &ObjectId) -> Result<Vec<u8>, GitError> {
        let missing = || GitError::MissingObject { id: id.clone() };
        self.commits.get(id).cloned().ok_or_else(missing)
    }

    fn markdown_files(&self, at: &Snapshot, roots: &Roots) -> Result<Vec<FileEntry>, GitError> {
        let key = match at {
            Snapshot::Commit(id) => Some(id.clone()),
            Snapshot::Worktree => None,
        };
        let files = self.files.get(&key).ok_or_else(|| GitError::Other {
            message: "no files".into(),
        })?;
        Ok(files
            .iter()
            .filter(|f| roots.selects(&f.path))
            .cloned()
            .collect())
    }

    fn markdown_changes(
        &self,
        base: &ObjectId,
        head: &ObjectId,
        roots: &Roots,
    ) -> Result<Vec<FileChange>, GitError> {
        let old = self.markdown_files(&Snapshot::Commit(base.clone()), roots)?;
        let new = self.markdown_files(&Snapshot::Commit(head.clone()), roots)?;
        Ok(diff_files(&old, &new))
    }

    fn read_blob(&self, id: &ObjectId) -> Result<Vec<u8>, GitError> {
        Err(GitError::MissingObject { id: id.clone() })
    }

    fn read_worktree_file(&self, path: &RepoPath) -> Result<Vec<u8>, GitError> {
        Err(GitError::MissingFile { path: path.clone() })
    }
}

#[test]
fn the_fake_has_no_blobs_or_worktree_files() {
    let git = FakeGit::new();
    assert_eq!(
        git.read_blob(&id('1')),
        Err(GitError::MissingObject { id: id('1') })
    );
    let path = RepoPath::new("a.md").unwrap();
    assert_eq!(
        git.read_worktree_file(&path),
        Err(GitError::MissingFile { path })
    );
}
