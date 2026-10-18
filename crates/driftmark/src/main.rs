//! The `driftmark` program. Version 1 adds `serve`, `diff`, `mcp`, `init` and
//! `export` to the commands here.

use std::ffi::OsString;
use std::io::Write;
use std::process::ExitCode;

/// Exit code for a command the program does not know, the shell convention for
/// "invoked wrongly".
const EXIT_USAGE: u8 = 2;

const USAGE: &str = "\
usage: driftmark <command> [<arguments>]

Commands:
  lint           check Markdown files against the writing rules
  hook install   run the lint after each edit Claude Code makes

Run `driftmark <command> --help` for a command's options.";

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let mut stdout = std::io::stdout().lock();
    let mut stderr = std::io::stderr().lock();
    let code = match args.first().and_then(|arg| arg.to_str()) {
        Some("--version" | "-V") => {
            let _ = writeln!(stdout, "driftmark {}", driftmark_core::version());
            0
        }
        Some("--help" | "-h" | "help") => {
            let _ = writeln!(stdout, "{USAGE}");
            0
        }
        Some("lint") => driftmark::lint::run(
            &args[1..],
            &mut std::io::stdin().lock(),
            &mut stdout,
            &mut stderr,
        ),
        Some("hook") => driftmark::hook::run(&args[1..], &mut stdout, &mut stderr),
        _ => {
            let _ = writeln!(stderr, "{USAGE}");
            EXIT_USAGE
        }
    };
    let _ = stdout.flush();
    ExitCode::from(code)
}
