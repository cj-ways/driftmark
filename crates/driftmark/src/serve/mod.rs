//! `driftmark serve`: a small local server that reads any branch from git
//! when a page is opened, so nothing is rebuilt on a timer (decision 0001).
//!
//! The server lists the branches whose documents differ from the default
//! branch, shows a document at any revision, a range's change list and one
//! document's changes, each as a page and as JSON. It caches what it parsed
//! by blob id, watches the working tree so open pages can reload, and lets
//! no request through from another machine without a token.
//!
//! `docs/features/commands/serve.md` is the user's guide and the contract of
//! the routes.

mod address;
mod app;
mod args;
mod assets;
mod cache;
mod compare;
mod guard;
mod page;
mod repo;
mod watch;

use std::ffi::OsString;
use std::fmt::Write as _;
use std::io::Write;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::broadcast;

use crate::git::Git;
use app::App;
use args::Request;
use repo::Repo;

/// Exit code after the server stopped on Ctrl-C.
pub const EXIT_OK: u8 = 0;
/// Exit code when the server could not start: the folder is not in a git
/// repository, or the address cannot be listened on.
pub const EXIT_ERROR: u8 = 1;
/// Exit code when the command line is wrong.
pub const EXIT_USAGE: u8 = 2;

/// How long the server waits, after Ctrl-C, for work it already started.
const STOP_WAIT: Duration = Duration::from_secs(2);

/// How `driftmark serve` is called.
const USAGE: &str = "\
usage: driftmark serve [--repo <path>] [--addr <ip:port>] [--token <value>]
                       [--base <branch>]

Serves the repository's Markdown documents and their changes as pages. Each
page reads git when it is opened, so branches show their latest commits.

Options:
  --repo <path>      a folder inside the repository (default: here)
  --addr <ip:port>   where to listen (default: 127.0.0.1:4170); port 0
                     picks a free port
  --token <value>    the token a browser must present, 16 characters or
                     more; an address other than 127.0.0.1 or ::1 always
                     needs one, and gets a random one when none is given
  --base <branch>    the branch the home page and `auto` compare with
                     (default: the rules file's [compare] default-branch,
                     else the first of origin/main, main, origin/master,
                     master, origin/dev, dev)

Open the address it prints. Stop it with Ctrl-C.

Exit codes: 0 stopped, 1 could not start, 2 the command line is wrong.";

/// Runs `driftmark serve` with the arguments after `serve`, until Ctrl-C,
/// and returns the exit code.
pub fn run(args: &[OsString], stdout: &mut dyn Write, stderr: &mut dyn Write) -> u8 {
    let args = match args::parse(args) {
        Ok(Request::Serve(args)) => args,
        Ok(Request::Help) => {
            let _ = writeln!(stdout, "{USAGE}");
            return EXIT_OK;
        }
        Err(message) => {
            let _ = writeln!(stderr, "driftmark serve: {message}\n\n{USAGE}");
            return EXIT_USAGE;
        }
    };
    match serve(&args, stdout) {
        Ok(()) => {
            let _ = writeln!(stdout, "driftmark serve: stopped");
            EXIT_OK
        }
        Err(message) => {
            let _ = writeln!(stderr, "driftmark serve: {message}");
            EXIT_ERROR
        }
    }
}

fn serve(args: &args::Args, stdout: &mut dyn Write) -> Result<(), String> {
    let git = Git::open(&args.repo).map_err(|error| {
        format!(
            "`{}` is not inside a git repository that can be read: {error}",
            args.repo.display()
        )
    })?;
    let workdir = git.workdir().map(std::path::Path::to_path_buf);
    let threads = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    let repo = Arc::new(Repo::new(git, cache::CAP_BYTES, threads, args.base.clone()));
    let token = match &args.token {
        Some(token) => Some(token.clone()),
        None if args.addr.ip().is_loopback() => None,
        None => Some(random_token()?),
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("driftmark-serve")
        .build()
        .map_err(|error| format!("the server could not start: {error}"))?;
    let listener = runtime
        .block_on(tokio::net::TcpListener::bind(args.addr))
        .map_err(|error| format!("could not listen on {}: {error}", args.addr))?;
    let bound = listener
        .local_addr()
        .map_err(|error| format!("could not read the address listened on: {error}"))?;
    let (events, _) = broadcast::channel(64);
    let watch = workdir.as_ref().map(|dir| {
        Arc::new(watch::Watch::new(
            Arc::clone(&repo),
            dir.clone(),
            events.clone(),
        ))
    });
    if let Some(watch) = &watch {
        watch.start();
    }
    let (stop, stopping) = tokio::sync::watch::channel(false);
    let mut stopped = stopping.clone();
    let app = Arc::new(App {
        repo,
        watch: watch.clone(),
        access: guard::Access::new(bound, token.clone()),
        assets: assets::Assets::built_in(),
        events,
        stopping,
    });
    let place = workdir.as_ref().map_or_else(
        || args.repo.display().to_string(),
        |dir| dir.display().to_string(),
    );
    let _ = stdout
        .write_all(greeting(&place, bound, token.as_deref(), args.token.is_some()).as_bytes());
    let _ = stdout.flush();
    let served = runtime.block_on(async move {
        let server = axum::serve(listener, app::router(app)).with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
            let _ = stop.send(true);
        });
        // After Ctrl-C, requests under way get STOP_WAIT to finish; a long
        // one does not keep the program running.
        let deadline = async move {
            let _ = stopped.wait_for(|stopped| *stopped).await;
            tokio::time::sleep(STOP_WAIT).await;
        };
        tokio::select! {
            result = server => result,
            () = deadline => Ok(()),
        }
    });
    if let Some(watch) = &watch {
        watch.stop();
    }
    // Git work still running on a blocking thread is left behind; the
    // process ends right after.
    runtime.shutdown_timeout(Duration::from_millis(100));
    served.map_err(|error| format!("the server stopped: {error}"))
}

/// What the server prints when it starts: where it reads, and the address
/// to open, with the token when one is needed.
fn greeting(place: &str, bound: std::net::SocketAddr, token: Option<&str>, asked: bool) -> String {
    let mut out = format!("driftmark serve: reading {place}\n");
    match token {
        None => {
            let _ = writeln!(out, "driftmark serve: open http://{bound}/");
        }
        Some(token) => {
            let _ = writeln!(out, "driftmark serve: open http://{bound}/?token={token}");
            if bound.ip().is_unspecified() {
                let _ = writeln!(
                    out,
                    "driftmark serve: write this machine's name or address in place of {}",
                    bound.ip()
                );
            }
            let why = if asked {
                "--token was given"
            } else {
                "the address reaches beyond this machine"
            };
            let _ = writeln!(
                out,
                "driftmark serve: every request needs the token, because {why}"
            );
        }
    }
    out.push_str("driftmark serve: press Ctrl-C to stop\n");
    out
}

/// A random token: 128 bits from the system, as 32 hex digits.
fn random_token() -> Result<String, String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|error| format!("could not make a random token: {error}"))?;
    Ok(bytes.iter().fold(String::new(), |mut hex, byte| {
        let _ = write!(hex, "{byte:02x}");
        hex
    }))
}

/// A 64-bit FNV-1a hash of some bytes. It names versions, such as a style
/// sheet in its address or a rules file on disk; it is not a secret.
pub(crate) fn fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_greeting_names_the_address_and_the_token() {
        let local = greeting("/r", "127.0.0.1:4170".parse().unwrap(), None, false);
        assert_eq!(
            local,
            "driftmark serve: reading /r\ndriftmark serve: open http://127.0.0.1:4170/\ndriftmark serve: press Ctrl-C to stop\n"
        );
        let open = greeting("/r", "0.0.0.0:80".parse().unwrap(), Some("t0k"), false);
        assert!(
            open.contains("open http://0.0.0.0:80/?token=t0k\n"),
            "{open}"
        );
        assert!(open.contains("in place of 0.0.0.0\n"), "{open}");
        assert!(open.contains("because the address reaches beyond this machine"));
        let asked = greeting("/r", "[::1]:9".parse().unwrap(), Some("t"), true);
        assert!(asked.contains("open http://[::1]:9/?token=t\n"), "{asked}");
        assert!(asked.contains("because --token was given"));
        assert!(!asked.contains("in place of"));
    }

    #[test]
    fn random_tokens_are_32_hex_digits_and_differ() {
        let (a, b) = (random_token().unwrap(), random_token().unwrap());
        assert_eq!(a.len(), 32);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn fingerprints_follow_fnv1a() {
        assert_eq!(fingerprint(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fingerprint(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_ne!(fingerprint(b"ab"), fingerprint(b"ba"));
    }
}
