//! Both git readers give the same answer to every question, on repos built
//! to hold the cases that tend to break readers.

mod support;

use std::collections::BTreeSet;

use driftmark_core::git::{
    Branch, FileChange, FileEntry, GitError, GitRead, LastChange, ObjectId, ObjectKind, RangeSpec,
    RepoPath, Roots, Snapshot,
};
use support::{DAY, Readers, T0, TestRepo};

fn id(hex: &str) -> ObjectId {
    ObjectId::from_hex(hex).expect("a full id")
}

fn path(p: &str) -> RepoPath {
    RepoPath::new(p).expect("a valid path")
}

/// Commits of the history repo, by role.
struct History {
    repo: TestRepo,
    c1: String,
    c2: String,
    f1: String,
    f2: String,
    m3: String,
    merge: String,
    m5: String,
    other: String,
}

/// A repo with branches, a merge, a deleted file, a file in a subfolder,
/// files that are not Markdown, CRLF content, a mode-only change, a file that
/// becomes a folder, a moved file, tags, a remote branch and an unrelated
/// history.
fn history() -> History {
    let repo = TestRepo::new("history");
    repo.write("README.md", "# Readme\n");
    repo.write("docs/a.md", "# A\n\nFirst words.\n");
    repo.write("docs/sub/b.markdown", "# B\n");
    repo.write("docs/crlf.md", "# CRLF\r\n\r\nLine one\r\nline two\r\n");
    repo.write("docs/UPPER.MD", "# Upper\n");
    repo.write("docs/run.md", "# Run\n");
    repo.write("notes.txt", "not markdown\n");
    let c1 = repo.commit("first", T0);
    repo.git(&["tag", "v1"]);

    repo.write("docs/a.md", "# A\n\nFirst words, edited.\n");
    repo.remove("docs/sub/b.markdown");
    repo.write("docs/new.md", "# New\n");
    repo.write("notes.txt", "still not markdown\n");
    repo.git(&["add", "-A"]);
    repo.git(&["update-index", "--chmod=+x", "docs/run.md"]);
    let c2 = repo.commit_index("second", T0 + DAY);
    repo.git_at(T0 + DAY, &["tag", "-a", "v2", "-m", "version two"]);
    repo.git(&["update-ref", "refs/remotes/origin/main", &c2]);

    repo.git(&["checkout", "-q", "-b", "feature"]);
    repo.write("docs/a.md", "# A\n\nFirst words, edited on a branch.\n");
    repo.write("docs/feature.md", "# Feature\n");
    repo.write("docs/sub/b.markdown", "# B again\n");
    let f1 = repo.commit("feature one", T0 + 2 * DAY);
    repo.remove("docs/new.md");
    repo.write(
        "docs/new.md/inner.md",
        "# Inside a folder named like a file\n",
    );
    let f2 = repo.commit("feature two", T0 + 3 * DAY);

    repo.git(&["checkout", "-q", "main"]);
    repo.write("README.md", "# Readme\n\nMore.\n");
    repo.remove("docs/crlf.md");
    let m3 = repo.commit("main three", T0 + 4 * DAY);
    repo.git_at(
        T0 + 5 * DAY,
        &["merge", "-q", "--no-ff", "-m", "merge feature", "feature"],
    );
    let merge = repo.git(&["rev-parse", "HEAD"]);
    repo.git(&["mv", "docs/feature.md", "docs/moved.md"]);
    let m5 = repo.commit_index("move", T0 + 6 * DAY);

    repo.git(&["checkout", "-q", "--orphan", "other"]);
    repo.git(&["rm", "-q", "-r", "-f", "."]);
    repo.write("other.md", "# Unrelated\n");
    let other = repo.commit("unrelated", T0 + 7 * DAY);
    repo.git(&["checkout", "-q", "main"]);
    History {
        repo,
        c1,
        c2,
        f1,
        f2,
        m3,
        merge,
        m5,
        other,
    }
}

/// Every reader question, for every commit and pair of commits the names
/// lead to. Returns the commits found.
fn check_everything(readers: &Readers, names: &[&str]) -> Vec<ObjectId> {
    let mut commits = BTreeSet::new();
    for name in names {
        if let Ok(commit) = readers.same(&format!("resolve {name:?}"), |g| g.resolve(name)) {
            commits.insert(commit);
        }
    }
    let commits: Vec<ObjectId> = commits.into_iter().collect();
    let roots = [
        Roots::all(),
        Roots::new(["docs"]).expect("roots"),
        Roots::new(["docs/sub", "README.md", "missing"]).expect("roots"),
    ];

    let mut blobs = BTreeSet::new();
    for commit in &commits {
        check_blob_at(readers, commit);
        let raw = readers
            .same("read_commit", |g| g.read_commit(commit))
            .expect("a commit");
        assert!(raw.starts_with(b"tree "));
        let wrong = readers.same("read_blob of a commit", |g| g.read_blob(commit));
        assert_eq!(
            wrong,
            Err(GitError::WrongKind {
                id: commit.clone(),
                expected: ObjectKind::Blob
            })
        );
        for root in &roots {
            let at = Snapshot::Commit(commit.clone());
            let files = readers
                .same("markdown_files", |g| g.markdown_files(&at, root))
                .expect("files");
            assert!(
                files.windows(2).all(|w| w[0].path < w[1].path),
                "sorted, once each"
            );
            blobs.extend(files.into_iter().map(|f| f.id));
        }
    }
    for a in &commits {
        for b in &commits {
            readers
                .same("merge_bases", |g| g.merge_bases(a, b).map(sorted))
                .expect("merge-bases");
            for root in &roots {
                let changes = readers
                    .same("markdown_changes", |g| g.markdown_changes(a, b, root))
                    .expect("changes");
                assert!(changes.windows(2).all(|w| w[0].path() < w[1].path()));
                blobs.extend(changes.iter().filter_map(|c| c.old_blob().cloned()));
            }
        }
    }
    for blob in &blobs {
        readers
            .same("read_blob", |g| g.read_blob(blob))
            .expect("a blob");
        let wrong = readers.same("read_commit of a blob", |g| g.read_commit(blob));
        assert_eq!(
            wrong,
            Err(GitError::WrongKind {
                id: blob.clone(),
                expected: ObjectKind::Commit
            })
        );
    }
    let absent = id("0123456789abcdef0123456789abcdef01234567");
    let missing = Err(GitError::MissingObject { id: absent.clone() });
    // A missing object sends the chooser to the `git` command, so only the
    // two readers are asked here; the fallback has its own tests.
    for reader in [&readers.gix as &dyn GitRead, &readers.command] {
        assert_eq!(reader.read_blob(&absent), missing);
        assert_eq!(reader.read_commit(&absent), missing);
    }
    commits
}

/// Paths that are not plain Markdown files in some test repo: other files,
/// folders, links, submodules, paths through a file or a submodule, and paths
/// that exist nowhere.
const OTHER_PATHS: [&str; 12] = [
    "notes.txt",
    "docs",
    "docs/sub",
    "missing.md",
    "docs/a.md/inner.md",
    "docs/link.md",
    "docs/sub.md",
    "docs/sub.md/inner.md",
    "docs/new.md",
    "docs/new.md/inner.md",
    "other.md",
    "docs/caf\u{e9}.md",
];

/// `blob_at` finds every Markdown file the commit lists, with the same blob,
/// and the readers agree on every other path.
fn check_blob_at(readers: &Readers, commit: &ObjectId) {
    let at = Snapshot::Commit(commit.clone());
    let files = readers
        .same("markdown_files", |g| g.markdown_files(&at, &Roots::all()))
        .expect("files");
    for file in &files {
        let found = readers
            .same("blob_at a listed file", |g| g.blob_at(commit, &file.path))
            .expect("found");
        assert_eq!(found.as_ref(), Some(&file.id), "{:?}", file.path);
    }
    for other in OTHER_PATHS {
        readers
            .same(&format!("blob_at {other}"), |g| {
                g.blob_at(commit, &path(other))
            })
            .expect("an answer");
    }
}

fn sorted(mut ids: Vec<ObjectId>) -> Vec<ObjectId> {
    ids.sort();
    ids
}

fn paths(files: &[FileEntry]) -> Vec<String> {
    files
        .iter()
        .map(|f| f.path.to_lossy().into_owned())
        .collect()
}

#[test]
fn both_readers_agree_on_a_history_with_branches_and_merges() {
    let h = history();
    let short = &h.c1[..8];
    let names = [
        "HEAD",
        "main",
        "feature",
        "other",
        "v1",
        "v2",
        "origin/main",
        "HEAD~1",
        "main~1^2",
        "@",
        &h.c1,
        short,
        &h.f2,
        &h.m3,
        "nonexistent",
        "",
        "HEAD^{tree}",
        "v2^{tree}",
        "-n",
        "main..feature",
        "HEAD~99",
        "feature~1",
    ];
    let commits = check_everything(&h.repo.readers(), &names);
    assert_eq!(commits.len(), 8, "c1 c2 f1 f2 m3 merge m5 other");
    for wanted in [&h.c1, &h.c2, &h.f1, &h.f2, &h.m3, &h.merge, &h.m5, &h.other] {
        assert!(commits.contains(&id(wanted)));
    }
}

#[test]
fn branches_list_local_then_remote_and_skip_what_is_not_a_branch() {
    let h = history();
    // A symbolic ref names another branch; a ref to a tag object is not a
    // branch tip; a ref outside refs/heads and refs/remotes is no branch.
    h.repo.git(&[
        "symbolic-ref",
        "refs/remotes/origin/HEAD",
        "refs/remotes/origin/main",
    ]);
    let tag = h.repo.git(&["rev-parse", "v2"]);
    h.repo
        .git(&["update-ref", "refs/remotes/origin/tag-object", &tag]);
    h.repo
        .git(&["update-ref", "refs/remotes/fork/topic", &h.f1]);
    h.repo.git(&["update-ref", "refs/notes/x", &h.c1]);
    let readers = h.repo.readers();
    let branch = |name: &str, remote: bool, tip: &str, current: bool| Branch {
        name: name.into(),
        remote,
        tip: id(tip),
        current,
    };
    let expected = vec![
        branch("feature", false, &h.f2, false),
        branch("main", false, &h.m5, true),
        branch("other", false, &h.other, false),
        branch("fork/topic", true, &h.f1, false),
        branch("origin/main", true, &h.c2, false),
    ];
    assert_eq!(readers.same("branches", |g| g.branches()), Ok(expected));

    h.repo.git(&["checkout", "-q", "--detach", "main"]);
    let readers = h.repo.readers();
    let detached = readers
        .same("detached", |g| g.branches())
        .expect("branches");
    assert!(
        detached.iter().all(|b| !b.current),
        "no branch is checked out"
    );
}

#[test]
fn any_file_is_found_by_its_path_at_a_commit() {
    let h = history();
    let readers = h.repo.readers();
    let blob_at = |commit: &str, p: &str| {
        let commit = id(commit);
        readers
            .same(p, |g| g.blob_at(&commit, &path(p)))
            .expect("an answer")
    };
    let text = h.repo.git(&["rev-parse", &format!("{}:notes.txt", h.c1)]);
    assert_eq!(
        blob_at(&h.c1, "notes.txt"),
        Some(id(&text)),
        "not only Markdown"
    );
    let run = h.repo.git(&["rev-parse", &format!("{}:docs/run.md", h.c2)]);
    assert_eq!(blob_at(&h.c2, "docs/run.md"), Some(id(&run)), "executable");
    assert_eq!(blob_at(&h.c1, "docs"), None, "a folder");
    assert!(blob_at(&h.c1, "docs/sub/b.markdown").is_some());
    assert_eq!(blob_at(&h.c2, "docs/sub/b.markdown"), None, "deleted");
    assert_eq!(blob_at(&h.c1, "docs/a.md/x.md"), None, "through a file");
    assert_eq!(blob_at(&h.f2, "docs/new.md"), None, "now a folder");
    assert!(blob_at(&h.f2, "docs/new.md/inner.md").is_some());
}

#[test]
fn names_resolve_like_git() {
    let h = history();
    let readers = h.repo.readers();
    let resolve = |name: &str| readers.same(name, |g| g.resolve(name));
    assert_eq!(resolve("v1"), Ok(id(&h.c1)));
    assert_eq!(
        resolve("v2"),
        Ok(id(&h.c2)),
        "an annotated tag leads to its commit"
    );
    assert_eq!(resolve("origin/main"), Ok(id(&h.c2)));
    assert_eq!(resolve("main~1^2"), Ok(id(&h.f2)));
    assert_eq!(resolve(&h.c1[..8]), Ok(id(&h.c1)));
    for unknown in ["nonexistent", "", "HEAD^{tree}", "-n", "HEAD~99", "a\0b"] {
        assert_eq!(
            resolve(unknown),
            Err(GitError::UnknownRevision {
                name: unknown.into()
            })
        );
    }
}

#[test]
fn files_at_a_commit_are_markdown_regular_files() {
    let h = history();
    let readers = h.repo.readers();
    let at = Snapshot::Commit(id(&h.c1));
    let all = readers
        .same("files", |g| g.markdown_files(&at, &Roots::all()))
        .expect("files");
    assert_eq!(
        paths(&all),
        [
            "README.md",
            "docs/UPPER.MD",
            "docs/a.md",
            "docs/crlf.md",
            "docs/run.md",
            "docs/sub/b.markdown"
        ]
    );
    let crlf = &all[3].id;
    let bytes = readers
        .same("crlf bytes", |g| g.read_blob(crlf))
        .expect("a blob");
    assert_eq!(bytes, b"# CRLF\r\n\r\nLine one\r\nline two\r\n");

    let sub = Roots::new(["docs/sub", "README.md"]).expect("roots");
    let some = readers
        .same("files in sub", |g| g.markdown_files(&at, &sub))
        .expect("files");
    assert_eq!(
        paths(&some),
        ["docs/sub/b.markdown"],
        "a root that names a file holds nothing"
    );
}

#[test]
fn changes_have_no_renames_and_ignore_mode_only_edits() {
    let h = history();
    let readers = h.repo.readers();
    let changes = |a: &str, b: &str| {
        let (a, b) = (id(a), id(b));
        readers
            .same("changes", |g| g.markdown_changes(&a, &b, &Roots::all()))
            .expect("changes")
    };
    let kinds = |list: Vec<FileChange>| -> Vec<(String, &'static str)> {
        let kind = |c: &FileChange| match c {
            FileChange::Added { .. } => "added",
            FileChange::Removed { .. } => "removed",
            FileChange::Modified { .. } => "modified",
        };
        list.iter()
            .map(|c| (c.path().to_lossy().into_owned(), kind(c)))
            .collect()
    };
    let expect = |list: &[(&str, &'static str)]| -> Vec<(String, &'static str)> {
        list.iter().map(|(p, k)| ((*p).to_owned(), *k)).collect()
    };
    assert_eq!(
        kinds(changes(&h.c1, &h.c2)),
        expect(&[
            ("docs/a.md", "modified"),
            ("docs/new.md", "added"),
            ("docs/sub/b.markdown", "removed")
        ]),
        "the executable bit on docs/run.md and the text file are not changes"
    );
    assert_eq!(
        kinds(changes(&h.f1, &h.f2)),
        expect(&[
            ("docs/new.md", "removed"),
            ("docs/new.md/inner.md", "added")
        ])
    );
    assert_eq!(
        kinds(changes(&h.merge, &h.m5)),
        expect(&[("docs/feature.md", "removed"), ("docs/moved.md", "added")])
    );
    assert_eq!(changes(&h.m5, &h.m5), []);
}

#[test]
fn ranges_resolve_and_list_changes_alike() {
    let h = history();
    let readers = h.repo.readers();
    let docs = Roots::new(["docs"]).expect("roots");
    let specs = [
        RangeSpec::parse("main..feature"),
        RangeSpec::parse("main...feature"),
        RangeSpec::parse("v1..main"),
        RangeSpec::parse("..feature"),
        RangeSpec::parse("feature..."),
        RangeSpec::parse("auto"),
        RangeSpec::since("2026-01-02T12:00:00Z", Some("main")),
        RangeSpec::since("2025-12-31", None),
        RangeSpec::since("2026-02-01", Some("feature")),
    ];
    let mut resolved = Vec::new();
    for spec in specs {
        let spec = spec.expect("a range");
        let each: Vec<_> = readers.all().iter().map(|g| spec.resolve(*g)).collect();
        assert!(each.iter().all(|r| r == &each[0]), "{spec:?}: {each:?}");
        let range = each[0].clone().expect("a resolved range");
        readers
            .same("range changes", |g| range.changes(g, &docs))
            .expect("changes");
        resolved.push(range);
    }
    let base = |i: usize| resolved[i].base.clone();
    assert_eq!(
        base(1),
        Some(id(&h.f2)),
        "feature was merged into main, so main...feature starts at its tip"
    );
    assert_eq!(
        base(5),
        Some(id(&h.c2)),
        "auto starts where main left origin/main"
    );
    assert_eq!(resolved[5].head, Snapshot::Worktree);
    assert_eq!(
        base(6),
        Some(id(&h.c2)),
        "the last first-parent commit before the date"
    );
    assert_eq!(base(7), None, "the whole history is after the date");
    assert_eq!(
        base(8),
        Some(id(&h.f2)),
        "nothing on feature after the date"
    );
}

#[test]
fn several_merge_bases_come_back_from_both() {
    let repo = TestRepo::new("criss-cross");
    repo.write("a.md", "base\n");
    repo.commit("base", T0);
    repo.git(&["checkout", "-q", "-b", "x"]);
    repo.write("x.md", "x\n");
    let x1 = repo.commit("x1", T0 + DAY);
    repo.git(&["checkout", "-q", "main"]);
    repo.git(&["checkout", "-q", "-b", "y"]);
    repo.write("y.md", "y\n");
    let y1 = repo.commit("y1", T0 + 2 * DAY);
    repo.git(&["checkout", "-q", "x"]);
    repo.git_at(T0 + 3 * DAY, &["merge", "-q", "--no-ff", "-m", "x2", &y1]);
    repo.git(&["checkout", "-q", "y"]);
    repo.git_at(T0 + 4 * DAY, &["merge", "-q", "--no-ff", "-m", "y2", &x1]);

    let commits = check_everything(&repo.readers(), &["x", "y", "main"]);
    assert_eq!(commits.len(), 3);
    let readers = repo.readers();
    let (x, y) = (
        readers.command.resolve("x").expect("x"),
        readers.command.resolve("y").expect("y"),
    );
    let bases = readers
        .same("bases", |g| g.merge_bases(&x, &y).map(sorted))
        .expect("bases");
    assert_eq!(bases, sorted(vec![id(&x1), id(&y1)]));
    let range = RangeSpec::parse("x...y")
        .expect("a range")
        .resolve(&readers.gix)
        .expect("resolved");
    assert_eq!(range.notes.len(), 1, "{:?}", range.notes);
}

/// Index entries that only git plumbing makes: a symbolic link, a submodule,
/// a name that is not UTF-8, names with a newline and a quote, and a file
/// that turns into a symbolic link.
#[test]
fn odd_index_entries_read_alike() {
    let repo = TestRepo::new("odd");
    // Git for Windows refuses names Windows cannot hold unless told not to.
    repo.git(&["config", "core.protectNTFS", "false"]);
    repo.write("docs/x.md", "# X\n");
    repo.git(&["add", "-A"]);
    let link = repo.blob(b"x.md");
    let text = repo.blob(b"# Odd name\n");
    let gitlink = "1111111111111111111111111111111111111111";
    // A newline or a quote in a name: Unix only, as Git for Windows may
    // still refuse them.
    let odd_names: &[&[u8]] = if cfg!(windows) {
        &[]
    } else {
        &[b"docs/new\nline.md", b"docs/q\"uote.md"]
    };
    let mut entries = Vec::new();
    let mut listed = vec![
        ("120000", link.as_str(), &b"docs/link.md"[..]),
        ("160000", gitlink, b"docs/sub.md"),
        ("100644", text.as_str(), b"docs/caf\xe9.md"),
        ("100755", text.as_str(), b"docs/tool.md"),
    ];
    listed.extend(
        odd_names
            .iter()
            .map(|name| ("100644", text.as_str(), *name)),
    );
    for (mode, blob, name) in listed {
        entries.extend_from_slice(format!("{mode} {blob}\t").as_bytes());
        entries.extend_from_slice(name);
        entries.push(0);
    }
    repo.git_with_input(&["update-index", "-z", "--index-info"], &entries);
    let c1 = repo.commit_index("odd entries", T0);

    let mut relinks = Vec::new();
    let x_link = repo.blob(b"elsewhere.md");
    for (mode, blob, name) in [
        ("120000", x_link.as_str(), &b"docs/x.md"[..]),
        ("120000", link.as_str(), b"docs/tool.md"),
    ] {
        relinks.extend_from_slice(format!("{mode} {blob}\t").as_bytes());
        relinks.extend_from_slice(name);
        relinks.push(0);
    }
    repo.git_with_input(&["update-index", "-z", "--index-info"], &relinks);
    repo.git_with_input(
        &["update-index", "-z", "--force-remove", "--stdin"],
        b"docs/caf\xe9.md\0",
    );
    let c2 = repo.commit_index("files become links", T0 + DAY);

    check_everything(&repo.readers(), &["HEAD", "HEAD~1"]);
    let readers = repo.readers();
    let commit = id(&c1);
    for (odd, found) in [
        (&b"docs/link.md"[..], false),
        (b"docs/sub.md", false),
        (b"docs/sub.md/inner.md", false),
        (b"docs/caf\xe9.md", true),
    ] {
        let odd = RepoPath::new(odd).expect("a path");
        let answer = readers
            .same("blob_at an odd entry", |g| g.blob_at(&commit, &odd))
            .expect("an answer");
        assert_eq!(answer.is_some(), found, "{odd:?}");
    }
    let at = Snapshot::Commit(id(&c1));
    let files = readers
        .same("odd files", |g| g.markdown_files(&at, &Roots::all()))
        .expect("files");
    let names: Vec<&[u8]> = files.iter().map(|f| f.path.as_bytes()).collect();
    let mut expected: Vec<&[u8]> = vec![b"docs/caf\xe9.md", b"docs/tool.md", b"docs/x.md"];
    expected.extend(odd_names);
    expected.sort();
    assert_eq!(names, expected, "links and submodules are not files");
    let (a, b) = (id(&c1), id(&c2));
    let changed = readers
        .same("odd changes", |g| g.markdown_changes(&a, &b, &Roots::all()))
        .expect("changes");
    let removed: Vec<&[u8]> = changed.iter().map(|c| c.path().as_bytes()).collect();
    assert_eq!(
        removed,
        [&b"docs/caf\xe9.md"[..], b"docs/tool.md", b"docs/x.md"]
    );
    assert!(
        changed.iter().all(|c| c.new_blob().is_none()),
        "a file that became a link is removed"
    );
}

#[test]
fn a_bare_repo_has_no_working_tree() {
    let source = history();
    let bare = support::TempDir::new("bare");
    let target = bare.path().join("bare.git");
    source
        .repo
        .git(&["clone", "-q", "--bare", ".", &target.to_string_lossy()]);
    let readers = support::readers_at(&target);
    assert_eq!(readers.gix.workdir(), None);
    assert_eq!(readers.command.workdir(), None);
    let worktree = readers.same("bare worktree", |g| {
        g.markdown_files(&Snapshot::Worktree, &Roots::all())
    });
    assert_eq!(worktree, Err(GitError::NoWorktree));
    let read = readers.same("bare read", |g| g.read_worktree_file(&path("README.md")));
    assert_eq!(read, Err(GitError::NoWorktree));
    let commits = check_everything(&readers, &["main", "feature", "v2"]);
    assert_eq!(commits.len(), 3);
}

/// A reader that answers `last_changes` with the core's own walk, one
/// commit read and one comparison per commit, to check the readers' faster
/// walks against it.
struct Plain<'a>(&'a dyn GitRead);

impl GitRead for Plain<'_> {
    fn resolve(&self, name: &str) -> Result<ObjectId, GitError> {
        self.0.resolve(name)
    }
    fn merge_bases(&self, a: &ObjectId, b: &ObjectId) -> Result<Vec<ObjectId>, GitError> {
        self.0.merge_bases(a, b)
    }
    fn read_commit(&self, id: &ObjectId) -> Result<Vec<u8>, GitError> {
        self.0.read_commit(id)
    }
    fn markdown_files(&self, at: &Snapshot, roots: &Roots) -> Result<Vec<FileEntry>, GitError> {
        self.0.markdown_files(at, roots)
    }
    fn markdown_changes(
        &self,
        a: &ObjectId,
        b: &ObjectId,
        roots: &Roots,
    ) -> Result<Vec<FileChange>, GitError> {
        self.0.markdown_changes(a, b, roots)
    }
    fn blob_at(&self, commit: &ObjectId, path: &RepoPath) -> Result<Option<ObjectId>, GitError> {
        self.0.blob_at(commit, path)
    }
    fn branches(&self) -> Result<Vec<Branch>, GitError> {
        self.0.branches()
    }
    fn read_blob(&self, id: &ObjectId) -> Result<Vec<u8>, GitError> {
        self.0.read_blob(id)
    }
    fn read_worktree_file(&self, path: &RepoPath) -> Result<Vec<u8>, GitError> {
        self.0.read_worktree_file(path)
    }
}

/// `(path, commit, seconds after T0)` for each last change.
fn lasts(changes: &[LastChange]) -> Vec<(String, String, i64)> {
    changes
        .iter()
        .map(|change| {
            (
                change.path.to_lossy().into_owned(),
                change.commit.as_hex().to_owned(),
                change.committed_at - T0,
            )
        })
        .collect()
}

#[test]
fn every_reader_finds_the_same_last_changes() {
    let h = history();
    let readers = h.repo.readers();
    let roots = [
        Roots::all(),
        Roots::new(["docs"]).expect("roots"),
        Roots::new(["docs/sub", "README.md", "missing"]).expect("roots"),
    ];
    for commit in [&h.c1, &h.c2, &h.f1, &h.f2, &h.m3, &h.merge, &h.m5, &h.other] {
        let commit = id(commit);
        for root in &roots {
            let found = readers
                .same("last_changes", |g| g.last_changes(&commit, root))
                .expect("an answer");
            let files = readers
                .command
                .markdown_files(&Snapshot::Commit(commit.clone()), root)
                .expect("files");
            assert_eq!(
                found.iter().map(|change| &change.path).collect::<Vec<_>>(),
                files.iter().map(|file| &file.path).collect::<Vec<_>>(),
                "one entry per file of the commit"
            );
            assert_eq!(
                Plain(&readers.command).last_changes(&commit, root),
                Ok(found),
                "the core's walk agrees"
            );
        }
    }
    let at_main = readers
        .same("last_changes", |g| {
            g.last_changes(&id(&h.m5), &Roots::all())
        })
        .expect("an answer");
    assert_eq!(
        lasts(&at_main),
        [
            ("README.md".into(), h.m3.clone(), 4 * DAY),
            ("docs/UPPER.MD".into(), h.c1.clone(), 0),
            ("docs/a.md".into(), h.merge.clone(), 5 * DAY),
            ("docs/moved.md".into(), h.m5.clone(), 6 * DAY),
            ("docs/new.md/inner.md".into(), h.merge.clone(), 5 * DAY),
            ("docs/run.md".into(), h.c1.clone(), 0),
            ("docs/sub/b.markdown".into(), h.merge.clone(), 5 * DAY),
        ],
        "a merge counts for what it brought in; a new mode alone is no change"
    );
    let unknown = id("0123456789abcdef0123456789abcdef01234567");
    for reader in [&readers.gix as &dyn GitRead, &readers.command] {
        assert!(reader.last_changes(&unknown, &Roots::all()).is_err());
    }
}

#[test]
fn a_shallow_clone_stops_at_its_oldest_commit() {
    let h = history();
    let clone = support::TempDir::new("shallow");
    let target = clone.path().join("shallow");
    let source = format!("file://{}", h.repo.dir.to_string_lossy());
    h.repo.git(&[
        "clone",
        "-q",
        "--depth",
        "2",
        "--branch",
        "main",
        &source,
        &target.to_string_lossy(),
    ]);
    let readers = support::readers_at(&target);
    let found = readers
        .same("shallow last_changes", |g| {
            g.last_changes(&id(&h.m5), &Roots::all())
        })
        .expect("an answer");
    // With depth 2 the clone holds m5 and the merge; the merge has no
    // parent there, so it holds every file not changed since.
    assert_eq!(
        lasts(&found),
        [
            ("README.md".into(), h.merge.clone(), 5 * DAY),
            ("docs/UPPER.MD".into(), h.merge.clone(), 5 * DAY),
            ("docs/a.md".into(), h.merge.clone(), 5 * DAY),
            ("docs/moved.md".into(), h.m5.clone(), 6 * DAY),
            ("docs/new.md/inner.md".into(), h.merge.clone(), 5 * DAY),
            ("docs/run.md".into(), h.merge.clone(), 5 * DAY),
            ("docs/sub/b.markdown".into(), h.merge.clone(), 5 * DAY),
        ]
    );
}
