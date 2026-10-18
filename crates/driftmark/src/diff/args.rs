//! The command line of `driftmark diff`, read without touching the disk or
//! the repository. Every mistake found here exits with code 2.

use std::ffi::OsString;
use std::num::NonZeroUsize;
use std::path::PathBuf;

use driftmark_core::git::{RangeSpec, Roots};

/// What `driftmark diff` was asked to do.
#[derive(Debug, PartialEq)]
pub(crate) enum Request {
    /// Print the usage text.
    Help,
    /// Compare the two points of a range.
    Diff(Args),
}

/// The arguments of one comparison.
#[derive(Debug, PartialEq)]
pub(crate) struct Args {
    /// The range as written, such as `main...feature` or `--since
    /// 2026-10-01 main`, for the first line of the summary.
    pub label: String,
    /// The range.
    pub range: RangeSpec,
    /// The folders to read, relative to the repository root. No folders
    /// means the whole repository.
    pub roots: Roots,
    /// A folder inside the repository.
    pub repo: PathBuf,
    /// Print the whole change list as JSON instead of the summary.
    pub json: bool,
    /// Print the time of each step on standard error.
    pub timings: bool,
    /// How many threads read, parse and compare files; `None` means one per
    /// processor core.
    pub threads: Option<NonZeroUsize>,
}

/// Reads the arguments after `diff`.
///
/// Options may come anywhere before `--`, and take their value as the next
/// argument or after `=`. Words before `--` are the range, or the branch for
/// `--since`; words after it are folders.
///
/// # Errors
///
/// A sentence for the user when the command line is wrong.
pub(crate) fn parse(args: &[OsString]) -> Result<Request, String> {
    let mut json = false;
    let mut timings = false;
    let mut repo = None;
    let mut threads = None;
    let mut since = None;
    let mut words = Vec::new();
    let mut folders = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let text = arg
            .to_str()
            .ok_or_else(|| format!("`{}` is not valid text", arg.to_string_lossy()))?;
        let (name, inline) = match text.split_once('=') {
            Some((name, value)) if name.starts_with("--") => (name, Some(value)),
            _ => (text, None),
        };
        let flag = |set: &mut bool| {
            if inline.is_some() {
                return Err(format!("`{name}` takes no value"));
            }
            *set = true;
            Ok(())
        };
        match name {
            "--" if inline.is_none() => {
                for folder in args.by_ref() {
                    let folder = folder.to_str().ok_or_else(|| {
                        format!(
                            "the folder `{}` is not valid text",
                            folder.to_string_lossy()
                        )
                    })?;
                    folders.push(folder.to_owned());
                }
            }
            "--json" => flag(&mut json)?,
            "--timings" => flag(&mut timings)?,
            "--help" | "-h" => return Ok(Request::Help),
            "--repo" => repo = Some(PathBuf::from(value(&mut args, name, inline)?)),
            "--threads" => threads = Some(thread_count(&text_value(&mut args, name, inline)?)?),
            "--since" => since = Some(text_value(&mut args, name, inline)?),
            _ if name.len() > 1 && name.starts_with('-') => {
                return Err(format!("unknown option `{name}`"));
            }
            _ => words.push(text.to_owned()),
        }
    }
    let roots = Roots::new(&folders).map_err(|error| error.to_string())?;
    let (range, label) = match since {
        Some(date) => since_range(&date, &words)?,
        None => match words.as_slice() {
            [] => return Err("give a range, such as `main...feature` or `auto`".to_owned()),
            [text] => (
                RangeSpec::parse(text).map_err(|error| error.to_string())?,
                text.clone(),
            ),
            _ => return Err("give one range; put folders after `--`".to_owned()),
        },
    };
    Ok(Request::Diff(Args {
        label,
        range,
        roots,
        repo: repo.unwrap_or_else(|| PathBuf::from(".")),
        json,
        timings,
        threads,
    }))
}

/// The `--since` range, with the branch from the words before `--`.
fn since_range(date: &str, words: &[String]) -> Result<(RangeSpec, String), String> {
    let branch = match words {
        [] => None,
        [branch] if branch.contains("..") => {
            return Err(format!(
                "`--since` takes a branch name, not the range `{branch}`"
            ));
        }
        [branch] => Some(branch.as_str()),
        _ => return Err("`--since` takes one branch; put folders after `--`".to_owned()),
    };
    let range = RangeSpec::since(date, branch).map_err(|error| error.to_string())?;
    let label = match branch {
        Some(branch) => format!("--since {date} {branch}"),
        None => format!("--since {date}"),
    };
    Ok((range, label))
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

fn thread_count(text: &str) -> Result<NonZeroUsize, String> {
    text.parse()
        .map_err(|_| format!("`--threads` takes a whole number from 1, not `{text}`"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_words(words: &[&str]) -> Result<Request, String> {
        let args: Vec<OsString> = words.iter().map(OsString::from).collect();
        parse(&args)
    }

    fn diff(words: &[&str]) -> Args {
        match parse_words(words) {
            Ok(Request::Diff(args)) => args,
            other => panic!("{words:?}: {other:?}"),
        }
    }

    fn refused(words: &[&str]) -> String {
        match parse_words(words) {
            Err(message) => message,
            other => panic!("{words:?} was accepted: {other:?}"),
        }
    }

    #[test]
    fn a_range_alone_reads_the_whole_repository_here() {
        let args = diff(&["main...feature"]);
        assert_eq!(
            args,
            Args {
                label: "main...feature".into(),
                range: RangeSpec::parse("main...feature").unwrap(),
                roots: Roots::all(),
                repo: PathBuf::from("."),
                json: false,
                timings: false,
                threads: None,
            }
        );
    }

    #[test]
    fn options_come_anywhere_before_the_folders() {
        let args = diff(&[
            "--json",
            "--repo",
            "/work/docs-repo",
            "auto",
            "--timings",
            "--threads=3",
            "--",
            "docs",
            "specs/api/",
        ]);
        assert_eq!(args.range, RangeSpec::Auto);
        assert_eq!(args.label, "auto");
        assert_eq!(args.repo, PathBuf::from("/work/docs-repo"));
        assert!(args.json && args.timings);
        assert_eq!(args.threads, NonZeroUsize::new(3));
        assert_eq!(args.roots, Roots::new(["docs", "specs/api"]).unwrap());
        let args = diff(&["a..b", "--repo=elsewhere", "--threads", "1"]);
        assert_eq!(args.repo, PathBuf::from("elsewhere"));
        assert_eq!(args.threads, NonZeroUsize::new(1));
    }

    #[test]
    fn since_takes_a_date_and_an_optional_branch() {
        let args = diff(&["--since", "2026-10-01"]);
        assert_eq!(args.range, RangeSpec::since("2026-10-01", None).unwrap());
        assert_eq!(args.label, "--since 2026-10-01");
        let args = diff(&["main", "--since=2026-10-01T09:30:00+04:00", "--", "docs"]);
        assert_eq!(
            args.range,
            RangeSpec::since("2026-10-01T09:30:00+04:00", Some("main")).unwrap()
        );
        assert_eq!(args.label, "--since 2026-10-01T09:30:00+04:00 main");
        assert_eq!(args.roots, Roots::new(["docs"]).unwrap());
    }

    #[test]
    fn help_wins_wherever_it_is() {
        assert_eq!(parse_words(&["--help"]), Ok(Request::Help));
        assert_eq!(parse_words(&["main..b", "-h"]), Ok(Request::Help));
    }

    #[test]
    fn mistakes_are_named() {
        let cases: [(&[&str], &str); 16] = [
            (&[], "give a range, such as `main...feature` or `auto`"),
            (&["main"], "`main` is not a range"),
            (&["a..b", "docs"], "give one range; put folders after `--`"),
            (&["--nope", "a..b"], "unknown option `--nope`"),
            (&["-x"], "unknown option `-x`"),
            (&["a..b", "--json=yes"], "`--json` takes no value"),
            (&["a..b", "--timings=1"], "`--timings` takes no value"),
            (&["a..b", "--repo"], "`--repo` needs a value"),
            (
                &["a..b", "--threads", "0"],
                "`--threads` takes a whole number from 1, not `0`",
            ),
            (&["a..b", "--threads=many"], "not `many`"),
            (&["--since", "last week"], "`last week` is not a date"),
            (&["--since"], "`--since` needs a value"),
            (
                &["--since", "2026-10-01", "a..b"],
                "`--since` takes a branch name, not the range `a..b`",
            ),
            (
                &["--since", "2026-10-01", "a", "b"],
                "`--since` takes one branch; put folders after `--`",
            ),
            (&["a..b", "--", "../up"], "../up"),
            (&["a..b", "--", "/docs"], "/docs"),
        ];
        for (words, wanted) in cases {
            let message = refused(words);
            assert!(message.contains(wanted), "{words:?}: {message}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn arguments_that_are_not_text_are_refused_except_a_repo_path() {
        use std::os::unix::ffi::OsStringExt;
        let odd = OsString::from_vec(b"caf\xe9".to_vec());
        let with = |args: Vec<OsString>| parse(&args);
        let range = OsString::from("a..b");
        let message = with(vec![odd.clone()]).unwrap_err();
        assert!(message.contains("is not valid text"), "{message}");
        let message = with(vec![range.clone(), "--".into(), odd.clone()]).unwrap_err();
        assert!(message.starts_with("the folder `caf"), "{message}");
        let message = with(vec![range.clone(), "--since".into(), odd.clone()]).unwrap_err();
        assert!(message.starts_with("the value of `--since`"), "{message}");
        let Ok(Request::Diff(args)) = with(vec![range, "--repo".into(), odd.clone()]) else {
            panic!("a repo path need not be text");
        };
        assert_eq!(args.repo, PathBuf::from(odd));
    }
}
