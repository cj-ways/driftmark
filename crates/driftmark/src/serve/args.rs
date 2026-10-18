//! The command line of `driftmark serve`, read without touching the disk or
//! the network. Every mistake found here exits with code 2.

use std::ffi::OsString;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;

/// Where the server listens when `--addr` is not given: this machine only.
pub(crate) const DEFAULT_ADDR: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 4170);

/// What `driftmark serve` was asked to do.
#[derive(Debug, PartialEq)]
pub(crate) enum Request {
    /// Print the usage text.
    Help,
    /// Run the server.
    Serve(Args),
}

/// The arguments of one server run.
#[derive(Debug, PartialEq)]
pub(crate) struct Args {
    /// A folder inside the repository.
    pub repo: PathBuf,
    /// The address and port to listen on. Port 0 picks a free port.
    pub addr: SocketAddr,
    /// The token, when `--token` gave it.
    pub token: Option<String>,
    /// The file to read the token from, when `--token-file` names one. It
    /// is read when the server starts, not here.
    pub token_file: Option<PathBuf>,
    /// The branch the home page and `auto` compare with, when given.
    pub base: Option<String>,
    /// The reader's name on notes, replies and seen marks, when given.
    pub reader: Option<String>,
    /// Where review files go, when not next to the notes file.
    pub review_dir: Option<PathBuf>,
}

/// Reads the arguments after `serve`.
///
/// Options take their value as the next argument or after `=`.
///
/// # Errors
///
/// A sentence for the user when the command line is wrong.
pub(crate) fn parse(args: &[OsString]) -> Result<Request, String> {
    let mut repo = None;
    let mut addr = None;
    let mut token = None;
    let mut token_file = None;
    let mut base = None;
    let mut reader = None;
    let mut review_dir = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let text = arg
            .to_str()
            .ok_or_else(|| format!("`{}` is not valid text", arg.to_string_lossy()))?;
        let (name, inline) = match text.split_once('=') {
            Some((name, value)) if name.starts_with("--") => (name, Some(value)),
            _ => (text, None),
        };
        match name {
            "--help" | "-h" => return Ok(Request::Help),
            "--repo" => repo = Some(PathBuf::from(value(&mut args, name, inline)?)),
            "--addr" => addr = Some(address(&text_value(&mut args, name, inline)?)?),
            "--token" => {
                let value = text_value(&mut args, name, inline)?;
                token = Some(super::token::checked(&value, "`--token`")?);
            }
            "--token-file" => token_file = Some(PathBuf::from(value(&mut args, name, inline)?)),
            "--base" => base = Some(text_value(&mut args, name, inline)?),
            "--reader" => reader = Some(checked_reader(&text_value(&mut args, name, inline)?)?),
            "--review-dir" => {
                review_dir = Some(PathBuf::from(value(&mut args, name, inline)?));
            }
            _ if name.starts_with('-') => return Err(format!("unknown option `{name}`")),
            _ => {
                return Err(format!(
                    "`{text}` is not an option; serve takes no other words"
                ));
            }
        }
    }
    Ok(Request::Serve(Args {
        repo: repo.unwrap_or_else(|| PathBuf::from(".")),
        addr: addr.unwrap_or(DEFAULT_ADDR),
        token,
        token_file,
        base,
        reader,
        review_dir,
    }))
}

/// A reader's name is shown on every note and reply, so it must be short
/// and visible.
fn checked_reader(name: &str) -> Result<String, String> {
    let name = name.trim().to_owned();
    if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
        return Err("`--reader` takes a name of 1 to 64 visible characters".to_owned());
    }
    Ok(name)
}

/// The value of an option: after `=`, or the next argument.
fn value(
    args: &mut std::slice::Iter<'_, OsString>,
    name: &str,
    inline: Option<&str>,
) -> Result<OsString, String> {
    match inline {
        Some(value) => Ok(value.into()),
        None => args
            .next()
            .cloned()
            .ok_or_else(|| format!("`{name}` needs a value")),
    }
}

/// The value of an option that must be text.
fn text_value(
    args: &mut std::slice::Iter<'_, OsString>,
    name: &str,
    inline: Option<&str>,
) -> Result<String, String> {
    value(args, name, inline)?.into_string().map_err(|value| {
        format!(
            "the value of `{name}`, `{}`, is not valid text",
            value.to_string_lossy()
        )
    })
}

fn address(text: &str) -> Result<SocketAddr, String> {
    text.parse().map_err(|_| {
        format!("`--addr` takes an address and a port, such as `127.0.0.1:4170` or `[::1]:4170`, not `{text}`")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_words(words: &[&str]) -> Result<Request, String> {
        let args: Vec<OsString> = words.iter().map(OsString::from).collect();
        parse(&args)
    }

    fn serve(words: &[&str]) -> Args {
        match parse_words(words) {
            Ok(Request::Serve(args)) => args,
            other => panic!("{words:?} gave {other:?}"),
        }
    }

    #[test]
    fn defaults_listen_on_this_machine_only() {
        assert_eq!(
            serve(&[]),
            Args {
                repo: PathBuf::from("."),
                addr: DEFAULT_ADDR,
                token: None,
                token_file: None,
                base: None,
                reader: None,
                review_dir: None,
            }
        );
        assert!(DEFAULT_ADDR.ip().is_loopback());
    }

    #[test]
    fn options_take_their_value_after_a_space_or_an_equals_sign() {
        let token = "a-B_c.d~0123456789";
        let args = serve(&["--repo", "/tmp/r", "--addr=[::1]:0", "--token", token]);
        assert_eq!(args.repo, PathBuf::from("/tmp/r"));
        assert_eq!(args.addr, "[::1]:0".parse().unwrap());
        assert_eq!(args.token.as_deref(), Some(token));
        assert_eq!(
            serve(&["--base", "origin/dev"]).base.as_deref(),
            Some("origin/dev")
        );
        assert_eq!(serve(&["--base=dev"]).base.as_deref(), Some("dev"));
        let args = serve(&["--repo=x", "--addr", "100.64.0.1:8000"]);
        assert_eq!(args.repo, PathBuf::from("x"));
        assert_eq!(args.addr.to_string(), "100.64.0.1:8000");
        let args = serve(&["--token-file", "/run/t", "--token=a-B_c.d~0123456789"]);
        assert_eq!(args.token_file, Some(PathBuf::from("/run/t")));
        assert_eq!(args.token.as_deref(), Some("a-B_c.d~0123456789"));
        assert_eq!(
            serve(&["--token-file=t"]).token_file,
            Some(PathBuf::from("t"))
        );
        let args = serve(&["--reader", " Ana ", "--review-dir=/tmp/reviews"]);
        assert_eq!(args.reader.as_deref(), Some("Ana"));
        assert_eq!(args.review_dir, Some(PathBuf::from("/tmp/reviews")));
        assert_eq!(parse_words(&["-h"]), Ok(Request::Help));
        assert_eq!(parse_words(&["--repo", "x", "--help"]), Ok(Request::Help));
    }

    #[test]
    fn mistakes_say_what_to_write() {
        for (words, start) in [
            (&["--addr", "localhost:80"][..], "`--addr` takes an address"),
            (&["--addr"], "`--addr` needs a value"),
            (&["--base"], "`--base` needs a value"),
            (&["--token", "short"], "`--token` needs at least 16"),
            (&["--token=has space in it 1234"], "`--token` may hold only"),
            (&["--reader", "  "], "`--reader` takes a name"),
            (&["--reader", "a\nb"], "`--reader` takes a name"),
            (&["--review-dir"], "`--review-dir` needs a value"),
            (&["--token-file"], "`--token-file` needs a value"),
            (&["--port", "1"], "unknown option `--port`"),
            (&["main"], "`main` is not an option"),
        ] {
            let error = parse_words(words).unwrap_err();
            assert!(error.starts_with(start), "{words:?}: {error}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn values_that_are_not_text_are_refused() {
        use std::os::unix::ffi::OsStringExt;
        let odd = OsString::from_vec(vec![b'-', b'-', 0xff]);
        assert!(parse(&[odd]).unwrap_err().ends_with("is not valid text"));
        let odd = OsString::from_vec(vec![0xff]);
        let error = parse(&["--addr".into(), odd]).unwrap_err();
        assert!(error.starts_with("the value of `--addr`"), "{error}");
    }
}
