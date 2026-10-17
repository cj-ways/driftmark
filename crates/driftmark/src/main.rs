//! The `driftmark` program. It has no commands yet; version 1 adds `serve`,
//! `diff`, `lint`, `mcp`, `init` and `export`.

use std::process::ExitCode;

/// Exit code for a command the program does not know, the shell convention for
/// "invoked wrongly".
const EXIT_USAGE: u8 = 2;

fn main() -> ExitCode {
    let first = std::env::args().nth(1);
    if let Some("--version" | "-V") = first.as_deref() {
        println!("driftmark {}", driftmark_core::version());
        ExitCode::SUCCESS
    } else {
        eprintln!(
            "driftmark {}: under development, no commands yet",
            driftmark_core::version()
        );
        ExitCode::from(EXIT_USAGE)
    }
}
