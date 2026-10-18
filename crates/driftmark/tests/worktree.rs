//! The working-tree side: both readers list the same files with git's ids,
//! and `auto` shows uncommitted work.

mod support;

use driftmark_core::git::{
    FileChange, FileEntry, GitError, GitRead, ObjectId, RangeSpec, RepoPath, Roots, Snapshot,
};
use support::{T0, TestRepo, readers_at};

fn path(p: &str) -> RepoPath {
    RepoPath::new(p).expect("a valid path")
}

fn paths(files: &[FileEntry]) -> Vec<String> {
    files
        .iter()
        .map(|f| f.path.to_lossy().into_owned())
        .collect()
}

/// A committed base, then edits of every kind left uncommitted.
fn edited() -> TestRepo {
    let repo = TestRepo::new("worktree");
    repo.write(".gitignore", "ignored.md\n");
    repo.write("README.md", "# Readme\n");
    repo.write("docs/a.md", "# A\n");
    repo.write("docs/b.md", "# B\n");
    repo.write("docs/keep.txt", "text\n");
    repo.commit("base", T0);
    repo.write("docs/a.md", "# A, edited\n");
    repo.remove("docs/b.md");
    repo.write("docs/new.md", "# New\n");
    repo.write("docs/newdir/deep.md", "# Deep\n");
    repo.write("docs/ignored.md", "# Ignored\n");
    repo.write("docs/staged.md", "# Staged\n");
    repo.git(&["add", "docs/staged.md"]);
    repo.write("notes.txt", "not markdown\n");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("a.md", repo.dir.join("docs/link.md")).expect("a link");
        std::os::unix::fs::symlink("docs", repo.dir.join("linked")).expect("a folder link");
    }
    repo
}

#[test]
fn the_working_tree_holds_tracked_and_untracked_markdown_on_disk() {
    let repo = edited();
    let readers = repo.readers();
    let worktree = Snapshot::Worktree;
    let files = readers
        .same("worktree", |g| g.markdown_files(&worktree, &Roots::all()))
        .expect("files");
    assert_eq!(
        paths(&files),
        [
            "README.md",
            "docs/a.md",
            "docs/new.md",
            "docs/newdir/deep.md",
            "docs/staged.md"
        ],
        "deleted, ignored, linked and non-Markdown files are left out"
    );
    for file in &files {
        let hashed = repo.git(&["hash-object", "--", &file.path.to_lossy()]);
        assert_eq!(file.id.as_hex(), hashed, "{:?} has git's id", file.path);
    }
    let committed = repo.git(&["rev-parse", "HEAD:README.md"]);
    assert_eq!(
        files[0].id.as_hex(),
        committed,
        "an untouched file keeps its committed id"
    );

    let deep = Roots::new(["docs/newdir"]).expect("roots");
    let some = readers
        .same("worktree in a folder", |g| {
            g.markdown_files(&worktree, &deep)
        })
        .expect("files");
    assert_eq!(paths(&some), ["docs/newdir/deep.md"]);
}

#[test]
fn auto_shows_uncommitted_work() {
    let repo = edited();
    let readers = repo.readers();
    let auto = RangeSpec::parse("auto").expect("a range");
    let each: Vec<_> = readers.all().iter().map(|g| auto.resolve(*g)).collect();
    assert!(each.iter().all(|r| r == &each[0]), "{each:?}");
    let range = each[0].clone().expect("resolved");
    let head = ObjectId::from_hex(&repo.git(&["rev-parse", "HEAD"])).expect("an id");
    assert_eq!(
        range.base,
        Some(head),
        "on main itself, auto starts at HEAD"
    );

    let changes = readers
        .same("auto changes", |g| range.changes(g, &Roots::all()))
        .expect("changes");
    let summary: Vec<(String, bool, bool)> = changes
        .iter()
        .map(|c: &FileChange| {
            (
                c.path().to_lossy().into_owned(),
                c.old_blob().is_some(),
                c.new_blob().is_some(),
            )
        })
        .collect();
    let expected = [
        ("docs/a.md", true, true),
        ("docs/b.md", true, false),
        ("docs/new.md", false, true),
        ("docs/newdir/deep.md", false, true),
        ("docs/staged.md", false, true),
    ];
    let expected: Vec<(String, bool, bool)> = expected
        .iter()
        .map(|(p, o, n)| ((*p).to_owned(), *o, *n))
        .collect();
    assert_eq!(summary, expected);
}

#[test]
fn working_tree_files_are_read_from_disk_and_never_through_links() {
    let repo = edited();
    let readers = repo.readers();
    let read = |p: &str| readers.same(p, |g| g.read_worktree_file(&path(p)));
    assert_eq!(read("docs/a.md"), Ok(b"# A, edited\n".to_vec()));
    assert_eq!(read("docs/newdir/deep.md"), Ok(b"# Deep\n".to_vec()));
    for missing in [
        "docs/b.md",
        "docs/newdir",
        "docs/link.md",
        "linked/a.md",
        "nothing/here.md",
    ] {
        assert_eq!(
            read(missing),
            Err(GitError::MissingFile {
                path: path(missing)
            }),
            "{missing}"
        );
    }
}

#[test]
fn line_ending_conversion_keeps_unchanged_files_unchanged() {
    let repo = TestRepo::new("autocrlf");
    repo.write("docs/a.md", "# A\n\nText.\n");
    repo.commit("lf", T0);
    repo.git(&["config", "core.autocrlf", "true"]);
    repo.write("docs/a.md", "# A\r\n\r\nText.\r\n");
    repo.write("docs/b.md", "# B\r\n");
    let readers = repo.readers();
    let worktree = Snapshot::Worktree;
    let files = readers
        .same("crlf worktree", |g| {
            g.markdown_files(&worktree, &Roots::all())
        })
        .expect("files");
    assert_eq!(paths(&files), ["docs/a.md", "docs/b.md"]);
    assert_eq!(
        files[0].id.as_hex(),
        repo.git(&["rev-parse", "HEAD:docs/a.md"]),
        "git calls it unchanged"
    );
    assert_eq!(
        files[1].id.as_hex(),
        repo.git_with_input(&["hash-object", "--stdin"], b"# B\n")
    );

    let range = RangeSpec::parse("auto")
        .expect("a range")
        .resolve(&readers.gix)
        .expect("resolved");
    let changes = readers
        .same("crlf changes", |g| range.changes(g, &Roots::all()))
        .expect("changes");
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].path(), &path("docs/b.md"));
    let bytes = readers.same("crlf bytes", |g| g.read_worktree_file(&path("docs/a.md")));
    assert_eq!(
        bytes,
        Ok(b"# A\r\n\r\nText.\r\n".to_vec()),
        "bytes stay as they are on disk"
    );
}

#[test]
fn readers_opened_in_a_subfolder_use_paths_from_the_top() {
    let repo = edited();
    let from_top = repo.readers();
    let from_docs = readers_at(&repo.dir.join("docs/newdir"));
    let worktree = Snapshot::Worktree;
    let top = from_top
        .same("top", |g| g.markdown_files(&worktree, &Roots::all()))
        .expect("files");
    let inner = from_docs
        .same("inner", |g| g.markdown_files(&worktree, &Roots::all()))
        .expect("files");
    assert_eq!(top, inner);
    let head = from_docs.same("head", |g| g.resolve("HEAD")).expect("head");
    let at = Snapshot::Commit(head);
    let committed = from_docs
        .same("committed", |g| g.markdown_files(&at, &Roots::all()))
        .expect("files");
    assert_eq!(paths(&committed), ["README.md", "docs/a.md", "docs/b.md"]);
}

#[test]
fn many_threads_read_at_once() {
    let repo = edited();
    let readers = repo.readers();
    let head = readers.command.resolve("HEAD").expect("head");
    let at = Snapshot::Commit(head);
    let files = readers
        .command
        .markdown_files(&at, &Roots::all())
        .expect("files");
    let expected: Vec<Vec<u8>> = files
        .iter()
        .map(|f| readers.command.read_blob(&f.id).expect("a blob"))
        .collect();
    std::thread::scope(|scope| {
        for reader in readers.all() {
            for _ in 0..4 {
                let (files, expected) = (&files, &expected);
                scope.spawn(move || {
                    for _ in 0..20 {
                        let read: Vec<Vec<u8>> = files
                            .iter()
                            .map(|f| reader.read_blob(&f.id).expect("a blob"))
                            .collect();
                        assert_eq!(&read, expected);
                    }
                });
            }
        }
    });
}
