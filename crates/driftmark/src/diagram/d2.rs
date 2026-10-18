//! D2 drawn by the `d2` program, when it is installed (decision 0003).
//!
//! The diagram goes to `d2` on standard input and the SVG comes back on
//! standard output. A drawing that takes longer than [`TIMEOUT`] is stopped,
//! and the block stays code.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long `d2` may take for one diagram.
pub(super) const TIMEOUT: Duration = Duration::from_secs(10);

/// The largest drawing kept, in bytes. A larger answer is refused.
const MAX_SVG: usize = 16 * 1024 * 1024;

/// The `d2` program.
pub(super) struct D2 {
    program: PathBuf,
    timeout: Duration,
}

impl D2 {
    /// `d2` on the `PATH`, or `None` when it is not installed.
    pub(super) fn find() -> Option<D2> {
        let name = if cfg!(windows) { "d2.exe" } else { "d2" };
        let path = std::env::var_os("PATH")?;
        std::env::split_paths(&path)
            .map(|folder| folder.join(name))
            .find(|candidate| candidate.is_file())
            .map(D2::at)
    }

    /// The program at `program`.
    pub(super) fn at(program: PathBuf) -> D2 {
        D2 {
            program,
            timeout: TIMEOUT,
        }
    }

    /// The diagram as `d2`'s SVG, not yet cleaned.
    ///
    /// # Errors
    ///
    /// When `d2` cannot start, takes too long, fails or answers with no SVG;
    /// the message says which, with the first line of what it printed.
    pub(super) fn draw(&self, code: &str) -> Result<String, String> {
        let mut child = Command::new(&self.program)
            .args(["--pad", "8", "-", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("d2 could not start: {error}"))?;
        let mut stdin = child.stdin.take();
        let input = code.as_bytes().to_vec();
        let writer = std::thread::spawn(move || {
            if let Some(stdin) = stdin.as_mut() {
                let _ = stdin.write_all(&input);
            }
        });
        let mut stdout = child.stdout.take();
        let reader = std::thread::spawn(move || {
            let mut out = Vec::new();
            if let Some(stdout) = stdout.as_mut() {
                let _ = stdout.take(MAX_SVG as u64 + 1).read_to_end(&mut out);
            }
            out
        });
        let mut stderr = child.stderr.take();
        let complaints = std::thread::spawn(move || {
            let mut out = Vec::new();
            if let Some(stderr) = stderr.as_mut() {
                let _ = stderr.take(64 * 1024).read_to_end(&mut out);
            }
            out
        });
        let started = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) if started.elapsed() < self.timeout => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
            }
        };
        let Some(status) = status else {
            // The threads are left to end when the pipes close: a child
            // that d2 started may still hold them open.
            return Err(format!(
                "d2 took longer than {} ms",
                self.timeout.as_millis()
            ));
        };
        let _ = writer.join();
        let out = reader.join().unwrap_or_default();
        let complaint = complaints.join().unwrap_or_default();
        if !status.success() {
            let said = String::from_utf8_lossy(&complaint);
            let line = said.lines().find(|line| !line.trim().is_empty());
            return Err(match line {
                Some(line) => format!("d2 failed: {}", line.trim()),
                None => format!("d2 failed with {status}"),
            });
        }
        if out.len() > MAX_SVG {
            return Err("d2's drawing is larger than 16 MB".to_owned());
        }
        String::from_utf8(out).map_err(|_| "d2's drawing is not UTF-8".to_owned())
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// A shell script standing in for `d2`, in a fresh folder.
    #[cfg(unix)]
    fn fake(name: &str, body: &str) -> D2 {
        use std::os::unix::fs::PermissionsExt;
        let folder =
            std::env::temp_dir().join(format!("driftmark-d2-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let program = folder.join("d2");
        std::fs::write(&program, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        D2 {
            program,
            timeout: Duration::from_millis(500),
        }
    }

    /// Draws, trying again while the script is still held open for
    /// writing: another test thread that starts a process at the moment
    /// the script is written keeps it open until its own program starts
    /// ("Text file busy"). Only a test writes and runs a program at once.
    #[cfg(unix)]
    pub(in crate::diagram) fn draw_retrying(d2: &D2, code: &str) -> Result<String, String> {
        for _ in 0..50 {
            match d2.draw(code) {
                Err(error) if error.contains("Text file busy") => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                other => return other,
            }
        }
        d2.draw(code)
    }

    #[cfg(unix)]
    #[test]
    fn a_drawing_comes_back_on_standard_output() {
        let d2 = fake("ok", "cat >/dev/null; printf '<svg viewBox=\"0 0 1 1\"/>'");
        assert_eq!(
            draw_retrying(&d2, "a -> b").unwrap(),
            "<svg viewBox=\"0 0 1 1\"/>"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_failure_reports_its_first_line() {
        let d2 = fake(
            "fail",
            "cat >/dev/null; echo '' >&2; echo 'err: bad shape' >&2; exit 1",
        );
        assert_eq!(
            draw_retrying(&d2, "x").unwrap_err(),
            "d2 failed: err: bad shape"
        );
        let quiet = fake("quiet", "exit 3");
        assert!(
            draw_retrying(&quiet, "x")
                .unwrap_err()
                .starts_with("d2 failed with ")
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_slow_drawing_is_stopped() {
        let d2 = fake("slow", "sleep 5");
        let started = Instant::now();
        assert_eq!(
            draw_retrying(&d2, "x").unwrap_err(),
            "d2 took longer than 500 ms"
        );
        assert!(started.elapsed() < Duration::from_secs(4));
    }

    #[cfg(unix)]
    #[test]
    fn bytes_that_are_not_text_are_refused() {
        let d2 = fake("binary", "cat >/dev/null; printf '\\377\\376'");
        assert_eq!(
            draw_retrying(&d2, "x").unwrap_err(),
            "d2's drawing is not UTF-8"
        );
    }

    #[test]
    fn a_missing_program_cannot_start() {
        let d2 = D2::at(PathBuf::from("/nonexistent/driftmark-test/d2"));
        assert!(
            d2.draw("x")
                .unwrap_err()
                .starts_with("d2 could not start: ")
        );
    }

    #[test]
    fn finding_d2_looks_on_the_path() {
        // Whether this machine has d2 or not, the answer is a file or none.
        if let Some(found) = D2::find() {
            assert!(found.program.is_file());
        }
    }
}
