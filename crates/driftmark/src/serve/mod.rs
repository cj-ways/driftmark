//! `driftmark serve`: a small local server that reads any branch from git
//! when a page is opened, so nothing is rebuilt on a timer (decision 0001).
//!
//! The server lists the branches whose documents differ from the default
//! branch, shows a document at any revision, a range's change list and one
//! document's changes, each as a page and as JSON, and searches every
//! document at a revision. It caches what it parsed by blob id, watches the
//! working tree so open pages can reload, and lets no request through from
//! another machine without a token. Every request that writes notes or seen
//! marks needs the token on every address, and each records whether it came
//! from the reader's page (decision 0012).
//!
//! `docs/features/commands/serve.md` is the user's guide and the contract of
//! the routes.

// `address`, `compare`, `placing`, `repo` and `search` are shared with the
// MCP connector, which answers the same questions over another channel.
pub(crate) mod address;
mod app;
mod args;
mod assets;
mod cache;
pub(crate) mod compare;
mod guard;
mod marker;
mod notes;
mod page;
pub(crate) mod placing;
pub(crate) mod repo;
pub(crate) mod search;
mod token;
mod watch;

use std::ffi::OsString;
use std::fmt::Write as _;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::broadcast;

use crate::git::Git;
use crate::memory::{IDLE_AFTER, Idle, give_back};
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
                       [--token-file <path>] [--base <branch>] [--reader <name>]
                       [--review-dir <path>]

Serves the repository's Markdown documents and their changes as pages. Each
page reads git when it is opened, so branches show their latest commits.

Options:
  --repo <path>      a folder inside the repository (default: here)
  --addr <ip:port>   where to listen (default: 127.0.0.1:4170); port 0
                     picks a free port
  --token <value>    the reader's token, 16 characters or more
  --token-file <path>
                     read the token from this file instead, once at start;
                     on Unix only its owner may read it (chmod 600)
  --base <branch>    the branch the home page and `auto` compare with
                     (default: the rules file's [compare] default-branch,
                     else the first of origin/main, main, origin/master,
                     master, origin/dev, dev)
  --reader <name>    the name on your notes, replies and seen marks
                     (default: reader)
  --review-dir <path>
                     where review files go (default: the reviews folder
                     next to the notes file, in the git folder)

The token comes from --token, else --token-file, else the DRIFTMARK_TOKEN
environment variable, else a random one made at start. Saving notes and seen
marks always needs it; reading pages on 127.0.0.1 or ::1 needs it only when
it was given. A browser presents it once, through the address printed at
start, and keeps it in a cookie.

Notes are kept in driftmark/notes.db in the clone's shared git folder, so
every worktree sees them and git never commits them.

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

/// The notes file the server uses (decision 0011, item 2), and where review
/// files are written: beside it, unless `--review-dir` says otherwise.
fn notes_file(args: &args::Args) -> placing::NotesFile {
    let notes_path = crate::notes::notes_file(&args.repo).unwrap_or_else(|_| {
        args.repo
            .join(".git")
            .join("driftmark")
            .join(crate::notes::FILE_NAME)
    });
    let reviews = args.review_dir.clone().unwrap_or_else(|| {
        notes_path.parent().map_or_else(
            || PathBuf::from(crate::notes::REVIEWS),
            |folder| folder.join(crate::notes::REVIEWS),
        )
    });
    placing::NotesFile::new(
        notes_path,
        reviews,
        args.reader.clone().unwrap_or_else(|| "reader".to_owned()),
    )
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
    let token = token_of(args)?;
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
    // Memory goes back to the system once no request ran for a second
    // (decision 0011).
    let idle = {
        let repo = Arc::clone(&repo);
        Arc::new(Idle::start(IDLE_AFTER, move || give_back(&repo.git)))
    };
    let search = watch.as_ref().map(|watch| {
        let index = Arc::new(search::WorktreeIndex::new(
            Arc::clone(&repo),
            Arc::clone(watch),
        ));
        index.start(events.subscribe());
        index
    });
    let notes = Arc::new(notes_file(args));
    let (stop, stopping) = tokio::sync::watch::channel(false);
    let mut stopped = stopping.clone();
    let app = Arc::new(App {
        repo,
        watch: watch.clone(),
        search,
        access: guard::Access::new(
            bound,
            token.value.clone(),
            token.from != token::TokenSource::Made,
        ),
        markers: marker::Markers::new(marker::LIFE, marker::MOST),
        assets: assets::Assets::built_in(),
        drawer: Arc::new(crate::diagram::Drawer::new()),
        notes,
        events,
        stopping,
    });
    let place = workdir.as_ref().map_or_else(
        || args.repo.display().to_string(),
        |dir| dir.display().to_string(),
    );
    let _ = stdout.write_all(greeting(&place, bound, &token).as_bytes());
    let _ = stdout.flush();
    let served = runtime.block_on(async move {
        let router = app::router(app).layer(axum::middleware::from_fn(
            move |request: axum::extract::Request, next: axum::middleware::Next| {
                let busy = idle.begin();
                async move {
                    let response = next.run(request).await;
                    drop(busy);
                    response
                }
            },
        ));
        let server = axum::serve(listener, router).with_graceful_shutdown(async move {
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

/// The server's token, from the first source given (`token.rs`).
fn token_of(args: &args::Args) -> Result<token::Token, String> {
    token::choose(
        args.token.as_deref(),
        args.token_file.as_deref(),
        std::env::var_os(token::VARIABLE),
        random_token,
    )
}

/// What the server prints when it starts: where it reads, the address to
/// open with the token, and what needs the token.
fn greeting(place: &str, bound: std::net::SocketAddr, token: &token::Token) -> String {
    let mut out = format!("driftmark serve: reading {place}\n");
    let _ = writeln!(
        out,
        "driftmark serve: open http://{bound}/?token={}",
        token.value
    );
    if bound.ip().is_unspecified() {
        let _ = writeln!(
            out,
            "driftmark serve: write this machine's name or address in place of {}",
            bound.ip()
        );
    }
    let made = token.from == token::TokenSource::Made;
    if !bound.ip().is_loopback() {
        out.push_str(
            "driftmark serve: every request needs the token, because the address reaches beyond this machine\n",
        );
    } else if made {
        out.push_str(
            "driftmark serve: pages open without the token on this machine; saving notes and seen marks needs it, once per browser\n",
        );
    } else {
        let _ = writeln!(
            out,
            "driftmark serve: every request needs the token, because it came from {}",
            token.from.words()
        );
    }
    if made {
        let _ = writeln!(
            out,
            "driftmark serve: the token is new at every start; keep one with --token-file or {}",
            token::VARIABLE
        );
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
        let made = |value: &str| token::Token {
            value: value.to_owned(),
            from: token::TokenSource::Made,
        };
        let local = greeting("/r", "127.0.0.1:4170".parse().unwrap(), &made("t0k"));
        assert_eq!(
            local,
            "driftmark serve: reading /r\n\
             driftmark serve: open http://127.0.0.1:4170/?token=t0k\n\
             driftmark serve: pages open without the token on this machine; saving notes and seen marks needs it, once per browser\n\
             driftmark serve: the token is new at every start; keep one with --token-file or DRIFTMARK_TOKEN\n\
             driftmark serve: press Ctrl-C to stop\n"
        );
        let open = greeting("/r", "0.0.0.0:80".parse().unwrap(), &made("t0k"));
        assert!(
            open.contains("open http://0.0.0.0:80/?token=t0k\n"),
            "{open}"
        );
        assert!(open.contains("in place of 0.0.0.0\n"), "{open}");
        assert!(open.contains("because the address reaches beyond this machine"));
        assert!(open.contains("the token is new at every start"));
        for (from, words) in [
            (token::TokenSource::Option, "--token"),
            (token::TokenSource::File, "--token-file"),
            (token::TokenSource::Environment, "DRIFTMARK_TOKEN"),
        ] {
            let given = token::Token {
                value: "t".to_owned(),
                from,
            };
            let asked = greeting("/r", "[::1]:9".parse().unwrap(), &given);
            assert!(asked.contains("open http://[::1]:9/?token=t\n"), "{asked}");
            assert!(
                asked.contains(&format!(
                    "every request needs the token, because it came from {words}\n"
                )),
                "{asked}"
            );
            assert!(!asked.contains("in place of"));
            assert!(!asked.contains("new at every start"));
            let far = greeting("/r", "100.64.0.1:9".parse().unwrap(), &given);
            assert!(far.contains("because the address reaches beyond this machine"));
        }
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
