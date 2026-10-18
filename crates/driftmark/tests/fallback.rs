//! The chooser uses gix, and switches to the `git` command, saying why, when
//! gix cannot open the repo or read an object.

mod support;

use std::sync::{Arc, Mutex};

use driftmark::git::{CommandReader, Git, GixReader};
use driftmark_core::git::{GitError, GitRead, ObjectId, Roots, Snapshot};
use support::{DAY, T0, TempDir, TestRepo};

fn id(hex: &str) -> ObjectId {
    ObjectId::from_hex(hex).expect("a full id")
}

/// A chooser whose messages land in a list.
fn chooser(dir: &std::path::Path) -> (Git, Arc<Mutex<Vec<String>>>) {
    let told = Arc::new(Mutex::new(Vec::new()));
    let sink = told.clone();
    let tell = Box::new(move |message: &str| sink.lock().expect("lock").push(message.to_owned()));
    (
        Git::open_telling(dir, tell).expect("the chooser opens the repo"),
        told,
    )
}

fn messages(told: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    told.lock().expect("lock").clone()
}

#[test]
fn a_repo_gix_reads_stays_with_gix() {
    let repo = TestRepo::new("plain");
    repo.write("a.md", "# A\n");
    repo.commit("one", T0);
    let git = Git::open(&repo.dir).expect("opens");
    assert_eq!(git.reader(), "gix");
    assert!(git.resolve("HEAD").is_ok());
    assert_eq!(
        git.resolve("nope"),
        Err(GitError::UnknownRevision {
            name: "nope".into()
        })
    );
    assert_eq!(
        git.reader(),
        "gix",
        "an unknown name is not a reason to switch"
    );
}

#[test]
fn a_sha256_repo_is_read_with_the_git_command() {
    let repo = TestRepo::init("sha256", &["--object-format=sha256"]);
    repo.write("docs/a.md", "# A\n");
    repo.commit("one", T0);
    assert!(
        GixReader::open(&repo.dir).is_err(),
        "gix reads SHA-1 repos only"
    );

    let (git, told) = chooser(&repo.dir);
    assert_eq!(git.reader(), "git command");
    let said = messages(&told);
    assert_eq!(said.len(), 1);
    assert!(
        said[0].starts_with("gix cannot open the repo ("),
        "{said:?}"
    );
    assert!(
        said[0].ends_with("); using the git command instead"),
        "{said:?}"
    );

    let head = git.resolve("HEAD").expect("head");
    assert_eq!(head.as_hex().len(), 64);
    let files = git
        .markdown_files(&Snapshot::Commit(head), &Roots::all())
        .expect("files");
    assert_eq!(files.len(), 1);
    assert_eq!(git.read_blob(&files[0].id), Ok(b"# A\n".to_vec()));
    let worktree = git
        .markdown_files(&Snapshot::Worktree, &Roots::all())
        .expect("files");
    assert_eq!(worktree, files);
    assert_eq!(messages(&told).len(), 1, "said once");
}

/// A `file://` address for a local folder, which a partial clone needs: a
/// plain path clone copies every object and ignores the filter.
fn file_url(dir: &std::path::Path) -> String {
    let path = dir.to_string_lossy().replace('\\', "/");
    format!("file:///{}", path.trim_start_matches('/'))
}

#[test]
fn a_partial_clone_gets_missing_files_through_git() {
    let source = TestRepo::new("promisor");
    source.write("docs/a.md", "# A\n");
    let c1 = source.commit("one", T0);
    source.write("docs/a.md", "# A, edited\n");
    let c2 = source.commit("two", T0 + DAY);
    source.git(&["config", "uploadpack.allowFilter", "true"]);
    let target = TempDir::new("partial");
    let clone = target.path().join("clone");
    let url = file_url(&source.dir);
    source.git(&[
        "clone",
        "-q",
        "--filter=blob:none",
        "--no-checkout",
        &url,
        &clone.to_string_lossy(),
    ]);

    let (git, told) = chooser(&clone);
    let changes = git
        .markdown_changes(&id(&c1), &id(&c2), &Roots::all())
        .expect("changes");
    assert_eq!(changes.len(), 1);
    assert_eq!(
        git.reader(),
        "gix",
        "trees are in the clone, so gix lists the changes"
    );

    let new = changes[0].new_blob().expect("a new blob").clone();
    let gix = GixReader::open(&clone).expect("gix opens the clone");
    assert_eq!(
        gix.read_blob(&new),
        Err(GitError::MissingObject { id: new.clone() })
    );
    assert_eq!(
        git.read_blob(&new),
        Ok(b"# A, edited\n".to_vec()),
        "git fetches it"
    );
    assert_eq!(git.reader(), "git command");
    let said = messages(&told);
    assert_eq!(said.len(), 1);
    assert!(
        said[0].starts_with("gix could not read a file (object "),
        "{said:?}"
    );
    assert!(
        said[0].ends_with("is not in the repo); using the git command from now on"),
        "{said:?}"
    );

    let old = changes[0].old_blob().expect("an old blob").clone();
    assert_eq!(git.read_blob(&old), Ok(b"# A\n".to_vec()));
    assert_eq!(messages(&told).len(), 1, "the switch is said once");
}

#[test]
fn a_lost_object_is_reported_by_both() {
    let repo = TestRepo::new("lost");
    repo.write("a.md", "# A\n");
    repo.write("b.md", "# B\n");
    repo.commit("one", T0);
    let a = repo.git(&["rev-parse", "HEAD:a.md"]);
    let b = repo.git(&["rev-parse", "HEAD:b.md"]);
    let object = repo
        .dir
        .join(".git")
        .join("objects")
        .join(&a[..2])
        .join(&a[2..]);
    let mut permissions = std::fs::metadata(&object)
        .expect("a loose object")
        .permissions();
    #[expect(
        clippy::permissions_set_readonly_false,
        reason = "a test removes the file next"
    )]
    permissions.set_readonly(false);
    std::fs::set_permissions(&object, permissions).expect("make it writable");
    std::fs::remove_file(&object).expect("lose the object");

    let (git, told) = chooser(&repo.dir);
    let missing = Err(GitError::MissingObject { id: id(&a) });
    assert_eq!(git.read_blob(&id(&a)), missing, "git has no copy either");
    assert_eq!(git.reader(), "git command");
    assert_eq!(messages(&told).len(), 1);
    assert_eq!(git.read_blob(&id(&b)), Ok(b"# B\n".to_vec()));
    let command = CommandReader::open(&repo.dir).expect("opens");
    assert_eq!(command.read_blob(&id(&a)), missing);
}

#[test]
fn a_folder_outside_any_repo_opens_nowhere() {
    let plain = TempDir::new("plain-folder");
    assert!(GixReader::open(plain.path()).is_err());
    assert!(CommandReader::open(plain.path()).is_err());
    let said = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = said.clone();
    let tell = Box::new(move |m: &str| sink.lock().expect("lock").push(m.to_owned()));
    assert!(Git::open_telling(plain.path(), tell).is_err());
    assert!(Git::open(plain.path()).is_err());
    assert!(
        messages(&said).is_empty(),
        "nothing to switch to, so nothing said"
    );
}
