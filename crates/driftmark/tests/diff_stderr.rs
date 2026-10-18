//! `driftmark diff` must not hold standard error while its worker threads
//! run: a worker that switches from gix to the `git` command says so on
//! standard error, and would otherwise wait forever for the lock.

mod support;

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use support::{DAY, T0, TempDir, TestRepo};

const PROGRAM: &str = env!("CARGO_BIN_EXE_driftmark");

/// A `file://` address for a local folder, which a partial clone needs: a
/// plain path clone copies every object and ignores the filter.
fn file_url(dir: &std::path::Path) -> String {
    let path = dir.to_string_lossy().replace('\\', "/");
    format!("file:///{}", path.trim_start_matches('/'))
}

#[test]
fn a_worker_that_falls_back_to_git_can_say_so() {
    let source = TestRepo::new("stderr-source");
    source.write("docs/a.md", "# A\n\nOne line.\n");
    let c1 = source.commit("one", T0);
    source.write("docs/a.md", "# A\n\nOne line, edited.\n");
    let c2 = source.commit("two", T0 + DAY);
    source.git(&["config", "uploadpack.allowFilter", "true"]);
    let target = TempDir::new("stderr-clone");
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

    // The trees are in the clone and the blobs are not, so gix lists the
    // change on the main thread and fails on the blob in a worker thread.
    let mut child = Command::new(PROGRAM)
        .arg("diff")
        .arg("--repo")
        .arg(&clone)
        .arg(format!("{c1}..{c2}"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start driftmark diff");
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        if let Some(status) = child.try_wait().expect("wait") {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("driftmark diff hung: a worker waited for standard error");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut stdout = String::new();
    let mut stderr = String::new();
    child
        .stdout
        .take()
        .expect("standard output")
        .read_to_string(&mut stdout)
        .expect("read standard output");
    child
        .stderr
        .take()
        .expect("standard error")
        .read_to_string(&mut stderr)
        .expect("read standard error");
    assert_eq!(status.code(), Some(0), "{stderr}");
    assert!(
        stderr.contains("using the git command from now on"),
        "{stderr}"
    );
    assert!(stdout.contains("docs/a.md"), "{stdout}");
}
