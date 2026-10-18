//! `driftmark diff`: what changed between two git points, ranked by meaning.
//!
//! The rules live in `driftmark_core`: ranges, documents, the rules file and
//! the comparison. This module reads the command line, opens the repository,
//! runs the comparison on several threads and prints the result, as a short
//! summary for a person, or in JSON for tools: the whole change list, or
//! only each document's counts.
//! `docs/features/commands/diff.md` is the user's guide.

mod args;
mod rules_file;
mod text;
mod work;

use std::ffi::OsString;
use std::fmt::Write as _;
use std::io::{BufWriter, ErrorKind, Write};
use std::time::{Duration, Instant};

use driftmark_core::diff::{ChangeList, RangeInfo};

use crate::git::Git;
use args::{Args, Json, Request};
pub use text::Terminal;

/// Exit code when the comparison ran, whether or not anything changed.
pub const EXIT_OK: u8 = 0;
/// Exit code when the comparison could not finish: the folder is not a git
/// repository, a name leads to no commit, the rules file is broken, git could
/// not be read, or the output could not be written.
pub const EXIT_ERROR: u8 = 1;
/// Exit code when the command line itself is wrong. driftmark finds this
/// before it opens anything.
pub const EXIT_USAGE: u8 = 2;

/// How `driftmark diff` is called.
const USAGE: &str = "\
usage: driftmark diff [<options>] <range> [-- <folder>...]
       driftmark diff [<options>] --since <date> [<branch>] [-- <folder>...]

Shows what changed in the Markdown files between two git points, ranked by
meaning: decision changes first, wording and formatting last.

Ranges:
  <base>..<head>     compare the two points directly
  <base>...<head>    compare <head> with where it left <base>
  auto               this branch's work since it left the default branch,
                     including changes not committed yet
  --since <date>     everything on <branch> (default HEAD) from that date;
                     write 2026-10-01 (UTC) or 2026-10-01T09:30:00+04:00

Options:
  --repo <path>      a folder inside the repository (default: here)
  --json             print the whole change list as JSON
  --json=summary     print one JSON entry per document with its counts
  --timings          print the time of each step on standard error
  --threads <n>      read and compare on n threads (default: one per core)

Folders after `--` limit the files read, such as `-- docs`; they start at
the repository root. The rules file is driftmark.toml at the repository
root, read at the head of the range.

Exit codes: 0 it ran, 1 it could not finish, 2 the command line is wrong.";

/// Runs `driftmark diff` with the arguments after `diff`, and returns the
/// exit code. `terminal` says what standard output is, so the summary knows
/// whether it may use colours and how wide a line may be; see
/// [`Terminal::detect`].
pub fn run(
    args: &[OsString],
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    terminal: Terminal,
) -> u8 {
    let started = Instant::now();
    let args = match args::parse(args) {
        Ok(Request::Diff(args)) => args,
        Ok(Request::Help) => {
            let _ = writeln!(stdout, "{USAGE}");
            return EXIT_OK;
        }
        Err(message) => {
            let _ = writeln!(stderr, "driftmark diff: {message}\n\n{USAGE}");
            return EXIT_USAGE;
        }
    };
    match diff(&args, started, stdout, stderr, terminal) {
        Ok(()) => EXIT_OK,
        Err(message) => {
            let _ = writeln!(stderr, "driftmark diff: {message}");
            EXIT_ERROR
        }
    }
}

/// The steps of one comparison, timed.
fn diff(
    args: &Args,
    started: Instant,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    terminal: Terminal,
) -> Result<(), String> {
    let mut clock = Clock::new(started);
    let git = Git::open(&args.repo).map_err(|error| {
        format!(
            "`{}` is not inside a git repository that can be read: {error}",
            args.repo.display()
        )
    })?;
    clock.lap("open the repository");
    let range = args
        .range
        .resolve(&git)
        .map_err(|error| error.to_string())?;
    clock.lap("resolve the range");
    let rules = rules_file::read(&git, &range.head)?;
    clock.lap("read the rules file");
    let changes = range
        .changes(&git, &args.roots)
        .map_err(|error| format!("the changed files could not be listed: {error}"))?;
    clock.lap("list the changed files");
    let threads = args.threads.map_or_else(
        || std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get),
        std::num::NonZeroUsize::get,
    );
    let compared = work::compare_all(&git, &rules, &changes, &range.head, threads)
        .map_err(|error| format!("a changed file could not be read: {error}"))?;
    clock.lap("read, parse and compare");
    for skipped in &compared.skipped {
        let _ = writeln!(
            stderr,
            "driftmark diff: skipped {}: {}",
            text::escape(&skipped.path.to_lossy()),
            skipped.reason
        );
    }
    let list = ChangeList::new(Some(RangeInfo::of(&range)), compared.documents);
    clock.lap("sort the documents");
    let written = write_output(args, &list, compared.skipped.len(), stdout, terminal);
    clock.lap("write the output");
    match written {
        Err(error) if error.kind() != ErrorKind::BrokenPipe => {
            return Err(format!("the output could not be written: {error}"));
        }
        // A reader that stopped early, such as `head`, has what it wanted.
        _ => {}
    }
    if args.timings {
        let report = timings(
            &clock,
            &compared.steps,
            compared.counts,
            changes.len(),
            compared.threads,
        );
        let _ = stderr.write_all(report.as_bytes());
    }
    Ok(())
}

/// Writes the JSON or the summary through one large buffer, so the output
/// goes out in big pieces even when standard output flushes every line.
fn write_output(
    args: &Args,
    list: &ChangeList,
    skipped: usize,
    stdout: &mut dyn Write,
    terminal: Terminal,
) -> std::io::Result<()> {
    let mut out = BufWriter::with_capacity(1 << 16, stdout);
    match args.json {
        Some(Json::Full) => {
            serde_json::to_writer(&mut out, list)?;
            out.write_all(b"\n")?;
        }
        Some(Json::Summary) => {
            serde_json::to_writer(&mut out, &list.summary_list())?;
            out.write_all(b"\n")?;
        }
        None => {
            let context = text::Context {
                label: &args.label,
                roots: &args.roots,
                skipped,
                terminal,
            };
            out.write_all(text::summary(list, &context).as_bytes())?;
        }
    }
    out.flush()
}

/// The time of each step since the command started.
struct Clock {
    started: Instant,
    last: Instant,
    laps: Vec<(&'static str, Duration)>,
}

impl Clock {
    fn new(started: Instant) -> Clock {
        Clock {
            started,
            last: started,
            laps: Vec::new(),
        }
    }

    /// Ends the step called `name`.
    fn lap(&mut self, name: &'static str) {
        let now = Instant::now();
        self.laps.push((name, now - self.last));
        self.last = now;
    }
}

/// The `--timings` report: each step in milliseconds, the per-file steps
/// summed over the threads, the total, and what the cache saved.
fn timings(
    clock: &Clock,
    steps: &work::Steps,
    counts: work::Counts,
    files: usize,
    threads: usize,
) -> String {
    let ms = |duration: Duration| duration.as_secs_f64() * 1000.0;
    let mut out = format!(
        "driftmark diff: timings in milliseconds, on {threads} {}\n",
        if threads == 1 { "thread" } else { "threads" }
    );
    for &(name, took) in &clock.laps {
        let _ = writeln!(out, "  {name:<30}{:>10.1}", ms(took));
        if name == "read, parse and compare" {
            for (part, took) in [
                ("reading, all threads", steps.read),
                ("parsing, all threads", steps.parse),
                ("comparing, all threads", steps.compare),
            ] {
                let _ = writeln!(out, "    {part:<28}{:>10.1}", ms(took));
            }
        }
    }
    let total = clock.last - clock.started;
    let _ = writeln!(out, "  {:<30}{:>10.1}", "total", ms(total));
    let _ = writeln!(
        out,
        "driftmark diff: {files} changed files, {} versions, {} parsed",
        counts.versions, counts.parsed
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A writer that fails every write with one kind of error.
    struct Failing(ErrorKind);

    impl Write for Failing {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(self.0.into())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Err(self.0.into())
        }
    }

    fn args(json: Option<Json>) -> Args {
        Args {
            label: "a..b".to_owned(),
            range: driftmark_core::git::RangeSpec::parse("a..b").unwrap(),
            roots: driftmark_core::git::Roots::all(),
            repo: ".".into(),
            json,
            timings: false,
            threads: None,
        }
    }

    #[test]
    fn output_errors_reach_the_caller() {
        let list = ChangeList::new(None, Vec::new());
        for json in [None, Some(Json::Full), Some(Json::Summary)] {
            let mut full = Failing(ErrorKind::StorageFull);
            let error = write_output(&args(json), &list, 0, &mut full, Terminal::NONE).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::StorageFull);
        }
    }

    #[test]
    fn help_and_usage_mistakes_need_no_repository() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        assert_eq!(
            run(&["--help".into()], &mut out, &mut err, Terminal::NONE),
            EXIT_OK
        );
        assert!(
            String::from_utf8(out)
                .unwrap()
                .starts_with("usage: driftmark diff")
        );
        let mut out = Vec::new();
        assert_eq!(
            run(&["main".into()], &mut out, &mut err, Terminal::NONE),
            EXIT_USAGE
        );
        let err = String::from_utf8(err).unwrap();
        assert!(
            err.starts_with("driftmark diff: `main` is not a range"),
            "{err}"
        );
        assert_eq!(out, b"");
    }

    #[test]
    fn timings_list_every_step_and_the_total() {
        let start = Instant::now();
        let mut clock = Clock::new(start);
        clock.lap("open the repository");
        clock.lap("read, parse and compare");
        let steps = work::Steps {
            read: Duration::from_micros(1500),
            parse: Duration::from_millis(12),
            compare: Duration::ZERO,
        };
        let counts = work::Counts {
            versions: 5,
            parsed: 4,
        };
        let report = timings(&clock, &steps, counts, 3, 1);
        let lines: Vec<&str> = report.lines().collect();
        assert_eq!(
            lines[0],
            "driftmark diff: timings in milliseconds, on 1 thread"
        );
        assert!(lines[1].starts_with("  open the repository "), "{report}");
        assert!(
            lines[2].starts_with("  read, parse and compare "),
            "{report}"
        );
        assert_eq!(lines[3], "    reading, all threads               1.5");
        assert_eq!(lines[4], "    parsing, all threads              12.0");
        assert_eq!(lines[5], "    comparing, all threads             0.0");
        assert!(lines[6].starts_with("  total "), "{report}");
        assert_eq!(
            lines[7],
            "driftmark diff: 3 changed files, 5 versions, 4 parsed"
        );
        assert!(
            timings(&clock, &steps, counts, 3, 4)
                .starts_with("driftmark diff: timings in milliseconds, on 4 threads\n")
        );
    }
}
