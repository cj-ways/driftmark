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

/// What `driftmark diff` prints. The last of `--json`, `--json=summary`
/// and `--format` on the command line counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Format {
    /// The summary for a person: one line per document, then the totals.
    Text,
    /// `--json`: the whole change list.
    Json,
    /// `--json=summary`: one entry per document with its counts.
    JsonSummary,
    /// `--format markdown-summary`: a short Markdown comment for a merge
    /// request.
    MarkdownSummary,
}

impl Format {
    /// The format a `--format` value names.
    fn parse(name: &str) -> Option<Format> {
        match name {
            "text" => Some(Format::Text),
            "json" => Some(Format::Json),
            "json-summary" => Some(Format::JsonSummary),
            "markdown-summary" => Some(Format::MarkdownSummary),
            _ => None,
        }
    }
}

/// The longest address `--link` takes. A browser takes about this much in
/// its address bar, and it keeps the comment short.
pub(crate) const MAX_LINK: usize = 2048;

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
    /// What to print: the text summary, JSON, or a Markdown comment.
    pub format: Format,
    /// Where the full rendered changes are, for the Markdown comment's last
    /// line: an `http` or `https` address.
    pub link: Option<String>,
    /// Print the time of each step on standard error.
    pub timings: bool,
    /// How many threads read, parse and compare files; `None` means one per
    /// processor core.
    pub threads: Option<NonZeroUsize>,
    /// The branch `auto` compares with, from `--base`; `None` leaves the
    /// choice to the rules file, then to the search order.
    pub base: Option<String>,
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
    let mut format = Format::Text;
    let mut link = None;
    let mut timings = false;
    let mut repo = None;
    let mut threads = None;
    let mut since = None;
    let mut base = None;
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
            "--json" => format = json_format(inline)?,
            "--format" => {
                let value = text_value(&mut args, name, inline)?;
                format = Format::parse(&value).ok_or_else(|| {
                    format!(
                        "`--format` takes `text`, `json`, `json-summary` or `markdown-summary`, not `{value}`"
                    )
                })?;
            }
            "--link" => link = Some(checked_link(text_value(&mut args, name, inline)?)?),
            "--timings" => flag(&mut timings)?,
            "--help" | "-h" => return Ok(Request::Help),
            "--repo" => repo = Some(PathBuf::from(value(&mut args, name, inline)?)),
            "--threads" => threads = Some(thread_count(&text_value(&mut args, name, inline)?)?),
            "--since" => since = Some(text_value(&mut args, name, inline)?),
            "--base" => base = Some(text_value(&mut args, name, inline)?),
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
    if base.is_some() && range != RangeSpec::Auto {
        return Err(format!(
            "`--base` names the branch `auto` compares with; `{label}` names its own points"
        ));
    }
    if link.is_some() && format != Format::MarkdownSummary {
        return Err(
            "`--link` names where the rendered changes are, for `--format markdown-summary` only"
                .to_owned(),
        );
    }
    Ok(Request::Diff(Args {
        label,
        range,
        roots,
        repo: repo.unwrap_or_else(|| PathBuf::from(".")),
        format,
        link,
        timings,
        threads,
        base,
    }))
}

/// The `--since` range, with the branch from the words before `--`.
pub(crate) fn since_range(date: &str, words: &[String]) -> Result<(RangeSpec, String), String> {
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

/// The format `--json` asks for: the whole list, or with `=summary` one
/// entry per document.
fn json_format(inline: Option<&str>) -> Result<Format, String> {
    match inline {
        None => Ok(Format::Json),
        Some("summary") => Ok(Format::JsonSummary),
        Some(other) => Err(format!(
            "`--json` takes only `summary` after `=`, not `{other}`"
        )),
    }
}

/// The `--link` address, when the comment can carry it safely: `http` or
/// `https` with something after the scheme, at most [`MAX_LINK`]
/// characters, and no white space, control character, `<`, `>` or
/// backslash. The comment writes it between `<` and `>`, so none of those
/// may end it early or escape a character, and no other scheme, such as
/// `javascript:`, reaches a reader's click.
fn checked_link(text: String) -> Result<String, String> {
    let scheme = ["https://", "http://"].iter().any(|scheme| {
        text.len() > scheme.len()
            && text
                .get(..scheme.len())
                .is_some_and(|start| start.eq_ignore_ascii_case(scheme))
    });
    let odd = text
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '<' | '>' | '\\'));
    if !scheme || odd || text.chars().count() > MAX_LINK {
        return Err(format!(
            "`--link` takes an `https://` or `http://` address of at most {MAX_LINK} characters, without spaces, `<`, `>` or `\\`"
        ));
    }
    Ok(text)
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
                format: Format::Text,
                link: None,
                timings: false,
                threads: None,
                base: None,
            }
        );
    }

    #[test]
    fn base_names_the_branch_auto_compares_with() {
        assert_eq!(
            diff(&["auto", "--base", "origin/dev"]).base.as_deref(),
            Some("origin/dev")
        );
        assert_eq!(diff(&["--base=dev", "auto"]).base.as_deref(), Some("dev"));
        assert_eq!(diff(&["auto"]).base, None);
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
        assert_eq!(args.format, Format::Json);
        assert!(args.timings);
        assert_eq!(args.threads, NonZeroUsize::new(3));
        assert_eq!(args.roots, Roots::new(["docs", "specs/api"]).unwrap());
        let args = diff(&["a..b", "--repo=elsewhere", "--threads", "1"]);
        assert_eq!(args.repo, PathBuf::from("elsewhere"));
        assert_eq!(args.threads, NonZeroUsize::new(1));
        assert_eq!(args.format, Format::Text);
        let args = diff(&["--json=summary", "a..b"]);
        assert_eq!(args.format, Format::JsonSummary);
        let args = diff(&["--json=summary", "a..b", "--json"]);
        assert_eq!(args.format, Format::Json, "the last one counts");
        let args = diff(&["--json", "a..b", "--format", "markdown-summary"]);
        assert_eq!(args.format, Format::MarkdownSummary);
        let args = diff(&["--format=json-summary", "a..b"]);
        assert_eq!(args.format, Format::JsonSummary);
        let args = diff(&["--format=markdown-summary", "a..b", "--format", "text"]);
        assert_eq!(args.format, Format::Text);
    }

    #[test]
    fn link_takes_a_plain_web_address() {
        let args = diff(&[
            "a...b",
            "--format",
            "markdown-summary",
            "--link=https://example.com/runs/7/artifacts/9",
        ]);
        assert_eq!(
            args.link.as_deref(),
            Some("https://example.com/runs/7/artifacts/9")
        );
        assert!(checked_link("HTTP://EXAMPLE.COM/a".to_owned()).is_ok());
        let longest = format!("https://example.com/{}", "a".repeat(MAX_LINK - 20));
        assert!(checked_link(longest.clone()).is_ok());
        for bad in [
            format!("{longest}a"),
            "ftp://example.com/a".to_owned(),
            "https:/example.com".to_owned(),
            "https://".to_owned(),
            "https://example.com/a b".to_owned(),
            "https://example.com/a>b".to_owned(),
            "https://example.com/a<b".to_owned(),
            "https://example.com/a\\)".to_owned(),
            "https://example.com/a\nb".to_owned(),
            "data:text/html,x".to_owned(),
            String::new(),
            "é".to_owned(),
        ] {
            assert!(checked_link(bad.clone()).is_err(), "{bad:?}");
        }
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
        let cases: [(&[&str], &str); 24] = [
            (
                &["a..b", "--format", "html"],
                "`--format` takes `text`, `json`, `json-summary` or `markdown-summary`, not `html`",
            ),
            (&["a..b", "--format"], "`--format` needs a value"),
            (
                &["a..b", "--link", "https://example.com/r"],
                "`--link` names where the rendered changes are, for `--format markdown-summary` only",
            ),
            (
                &[
                    "a..b",
                    "--format=markdown-summary",
                    "--link",
                    "javascript:alert(1)",
                ],
                "`--link` takes an `https://` or `http://` address",
            ),
            (
                &["a...b", "--base", "dev"],
                "`--base` names the branch `auto` compares with; `a...b` names its own points",
            ),
            (
                &["--since", "2026-10-01", "--base=dev"],
                "`--since 2026-10-01` names its own points",
            ),
            (&["auto", "--base"], "`--base` needs a value"),
            (&[], "give a range, such as `main...feature` or `auto`"),
            (&["main"], "`main` is not a range"),
            (&["a..b", "docs"], "give one range; put folders after `--`"),
            (&["--nope", "a..b"], "unknown option `--nope`"),
            (&["-x"], "unknown option `-x`"),
            (
                &["a..b", "--json=yes"],
                "`--json` takes only `summary` after `=`, not `yes`",
            ),
            (
                &["a..b", "--json="],
                "`--json` takes only `summary` after `=`, not ``",
            ),
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
