//! The command line of `driftmark export`, read without touching the disk or
//! the repository. Every mistake found here exits with code 2.

use std::ffi::OsString;
use std::path::PathBuf;

use driftmark_core::git::{RangeSpec, RepoPath, Roots};

use crate::serve::address::{self, Revision};

/// The size limit when `--max-size` is not given: 10 MB. An email client
/// sends an attachment as base64, a third larger, so a 10 MB file still
/// fits the 20 to 25 MB most mail services accept.
pub(crate) const DEFAULT_MAX_BYTES: u64 = 10_000_000;

/// What `driftmark export` was asked to do.
#[derive(Debug, PartialEq)]
pub(crate) enum Request {
    /// Print the usage text.
    Help,
    /// Write one file.
    Export(Args),
}

/// What goes into the file.
#[derive(Debug, PartialEq)]
pub(crate) enum What {
    /// One document at a revision: `export doc <revision> <path>`.
    Document {
        /// The revision, or the working tree.
        revision: Revision,
        /// The document's path from the repository root.
        path: RepoPath,
    },
    /// Every document under a folder: `export folder <revision> <folder>`.
    Folder {
        /// The revision, or the working tree.
        revision: Revision,
        /// The folder as written, for the page; `.` for the whole repository.
        label: String,
        /// The folder, as roots: none for the whole repository.
        roots: Roots,
    },
    /// A range's changes: `export changes <range> [-- <folder>...]`.
    Changes {
        /// The range as written, such as `main...feature` or `--since
        /// 2026-10-01 main`.
        label: String,
        /// The range.
        range: RangeSpec,
        /// The folders to keep; none keeps the whole repository.
        roots: Roots,
        /// The branch `auto` compares with, from `--base`.
        base: Option<String>,
    },
}

/// The arguments of one export.
#[derive(Debug, PartialEq)]
pub(crate) struct Args {
    /// What goes into the file.
    pub what: What,
    /// A folder inside the repository.
    pub repo: PathBuf,
    /// The file to write; `None` writes to standard output.
    pub output: Option<PathBuf>,
    /// The size the file should stay under, in bytes.
    pub max_bytes: u64,
}

/// Reads the arguments after `export`.
///
/// The first word names what to export: `doc`, `folder` or `changes`.
/// Options may come anywhere before `--` and take their value as the next
/// argument or after `=`.
///
/// # Errors
///
/// A sentence for the user when the command line is wrong.
pub(crate) fn parse(args: &[OsString]) -> Result<Request, String> {
    let mut output = None;
    let mut repo = None;
    let mut max_bytes = None;
    let mut since = None;
    let mut base = None;
    let mut words = Vec::new();
    let mut folders = Vec::new();
    let mut after_dashes = false;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let text = arg
            .to_str()
            .ok_or_else(|| format!("`{}` is not valid text", arg.to_string_lossy()))?;
        if after_dashes {
            folders.push(text.to_owned());
            continue;
        }
        let (name, inline) = match text.split_once('=') {
            Some((name, value)) if name.starts_with("--") => (name, Some(value)),
            _ => (text, None),
        };
        match name {
            "--" if inline.is_none() => after_dashes = true,
            "--help" | "-h" => return Ok(Request::Help),
            "-o" | "--output" => output = Some(PathBuf::from(value(&mut args, name, inline)?)),
            "--repo" => repo = Some(PathBuf::from(value(&mut args, name, inline)?)),
            "--max-size" => max_bytes = Some(size(&text_value(&mut args, name, inline)?)?),
            "--since" => since = Some(text_value(&mut args, name, inline)?),
            "--base" => base = Some(text_value(&mut args, name, inline)?),
            _ if name.len() > 1 && name.starts_with('-') => {
                return Err(format!("unknown option `{name}`"));
            }
            _ => words.push(text.to_owned()),
        }
    }
    let Some((kind, words)) = words.split_first() else {
        return Err("say what to export: `doc`, `folder` or `changes`".to_owned());
    };
    let what = match kind.as_str() {
        "doc" => {
            refuse_range_options(kind, since.as_ref(), base.as_ref(), &folders)?;
            document(words)?
        }
        "folder" => {
            refuse_range_options(kind, since.as_ref(), base.as_ref(), &folders)?;
            folder(words)?
        }
        "changes" => changes(words, since, base, &folders)?,
        other => {
            return Err(format!(
                "`{other}` is not something to export; give `doc`, `folder` or `changes`"
            ));
        }
    };
    Ok(Request::Export(Args {
        what,
        repo: repo.unwrap_or_else(|| PathBuf::from(".")),
        output,
        max_bytes: max_bytes.unwrap_or(DEFAULT_MAX_BYTES),
    }))
}

/// `--since`, `--base` and folders after `--` belong to `changes` only.
fn refuse_range_options(
    kind: &str,
    since: Option<&String>,
    base: Option<&String>,
    folders: &[String],
) -> Result<(), String> {
    if since.is_some() {
        return Err(format!(
            "`--since` belongs to `export changes`, not `export {kind}`"
        ));
    }
    if base.is_some() {
        return Err(format!(
            "`--base` belongs to `export changes`, not `export {kind}`"
        ));
    }
    if !folders.is_empty() {
        return Err(format!(
            "folders after `--` belong to `export changes`; `export {kind}` names its own"
        ));
    }
    Ok(())
}

/// `doc <revision> <path>`.
fn document(words: &[String]) -> Result<What, String> {
    let [revision, path] = words else {
        return Err(
            "`export doc` takes a revision and a path, such as `main docs/guide.md`".to_owned(),
        );
    };
    let path = RepoPath::new(path.trim_start_matches("./"))
        .filter(RepoPath::is_markdown)
        .ok_or_else(|| {
            format!("`{path}` is not the path of a Markdown file from the repository root")
        })?;
    Ok(What::Document {
        revision: revision_of(revision),
        path,
    })
}

/// `folder <revision> <folder>`.
fn folder(words: &[String]) -> Result<What, String> {
    let [revision, folder] = words else {
        return Err(
            "`export folder` takes a revision and a folder, such as `main docs/decisions`"
                .to_owned(),
        );
    };
    let trimmed = folder.trim_start_matches("./");
    let roots = if trimmed.is_empty() || trimmed == "." {
        Roots::all()
    } else {
        Roots::new([trimmed]).map_err(|error| error.to_string())?
    };
    let label = if roots.folders().is_empty() {
        ".".to_owned()
    } else {
        trimmed.trim_end_matches('/').to_owned()
    };
    Ok(What::Folder {
        revision: revision_of(revision),
        label,
        roots,
    })
}

/// `changes <range>`, or `changes --since <date> [<branch>]`.
fn changes(
    words: &[String],
    since: Option<String>,
    base: Option<String>,
    folders: &[String],
) -> Result<What, String> {
    let roots = Roots::new(folders).map_err(|error| error.to_string())?;
    let (range, label) = match since {
        Some(date) => crate::diff::args::since_range(&date, words)?,
        None => match words {
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
    Ok(What::Changes {
        label,
        range,
        roots,
        base,
    })
}

/// A revision as written: `worktree` is the files on disk, as in the
/// server's addresses.
fn revision_of(text: &str) -> Revision {
    if text == address::WORKTREE {
        Revision::Worktree
    } else {
        Revision::Name(text.to_owned())
    }
}

/// A size such as `10M`, `500K` or `2000000`: bytes, or thousands (`K`) or
/// millions (`M`) of bytes, as mail services count their limits.
fn size(text: &str) -> Result<u64, String> {
    let wrong =
        || format!("`--max-size` takes a size such as `10M`, `500K` or `2000000`, not `{text}`");
    let (number, unit) = match text.char_indices().last() {
        Some((at, 'K' | 'k')) => (&text[..at], 1_000),
        Some((at, 'M' | 'm')) => (&text[..at], 1_000_000),
        _ => (text, 1),
    };
    let count: u64 = number.parse().map_err(|_| wrong())?;
    count
        .checked_mul(unit)
        .filter(|bytes| *bytes > 0)
        .ok_or_else(wrong)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_words(words: &[&str]) -> Result<Request, String> {
        let args: Vec<OsString> = words.iter().map(OsString::from).collect();
        parse(&args)
    }

    fn export(words: &[&str]) -> Args {
        match parse_words(words) {
            Ok(Request::Export(args)) => args,
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
    fn a_document_at_a_revision_goes_to_standard_output() {
        let args = export(&["doc", "main", "docs/a.md"]);
        assert_eq!(
            args,
            Args {
                what: What::Document {
                    revision: Revision::Name("main".into()),
                    path: RepoPath::new("docs/a.md").unwrap(),
                },
                repo: PathBuf::from("."),
                output: None,
                max_bytes: DEFAULT_MAX_BYTES,
            }
        );
        let args = export(&[
            "doc",
            "worktree",
            "./README.md",
            "-o",
            "out.html",
            "--repo=r",
        ]);
        assert_eq!(
            args.what,
            What::Document {
                revision: Revision::Worktree,
                path: RepoPath::new("README.md").unwrap(),
            }
        );
        assert_eq!(args.output, Some(PathBuf::from("out.html")));
        assert_eq!(args.repo, PathBuf::from("r"));
    }

    #[test]
    fn a_folder_is_read_from_the_repository_root() {
        let args = export(&["folder", "v1.0", "docs/decisions/", "--output", "d.html"]);
        assert_eq!(
            args.what,
            What::Folder {
                revision: Revision::Name("v1.0".into()),
                label: "docs/decisions".into(),
                roots: Roots::new(["docs/decisions"]).unwrap(),
            }
        );
        for whole in [".", "./"] {
            let args = export(&["folder", "main", whole]);
            assert_eq!(
                args.what,
                What::Folder {
                    revision: Revision::Name("main".into()),
                    label: ".".into(),
                    roots: Roots::all(),
                }
            );
        }
    }

    #[test]
    fn changes_take_the_ranges_of_diff() {
        let args = export(&[
            "changes",
            "main...feature",
            "--max-size",
            "500K",
            "--",
            "docs",
            "specs",
        ]);
        assert_eq!(
            args.what,
            What::Changes {
                label: "main...feature".into(),
                range: RangeSpec::parse("main...feature").unwrap(),
                roots: Roots::new(["docs", "specs"]).unwrap(),
                base: None,
            }
        );
        assert_eq!(args.max_bytes, 500_000);
        let args = export(&["changes", "--since", "2026-10-01", "main"]);
        assert_eq!(
            args.what,
            What::Changes {
                label: "--since 2026-10-01 main".into(),
                range: RangeSpec::since("2026-10-01", Some("main")).unwrap(),
                roots: Roots::all(),
                base: None,
            }
        );
        let args = export(&["changes", "auto", "--base", "origin/dev"]);
        assert!(matches!(args.what, What::Changes { base: Some(ref b), .. } if b == "origin/dev"));
    }

    #[test]
    fn sizes_count_bytes_thousands_and_millions() {
        assert_eq!(size("2000000"), Ok(2_000_000));
        assert_eq!(size("500K"), Ok(500_000));
        assert_eq!(size("3k"), Ok(3_000));
        assert_eq!(size("10M"), Ok(10_000_000));
        assert_eq!(size("2m"), Ok(2_000_000));
        for wrong in [
            "",
            "M",
            "0",
            "0K",
            "-1",
            "1.5M",
            "10MB",
            "99999999999999999999M",
        ] {
            assert!(size(wrong).is_err(), "{wrong}");
        }
        assert_eq!(
            size("ten").unwrap_err(),
            "`--max-size` takes a size such as `10M`, `500K` or `2000000`, not `ten`"
        );
    }

    #[test]
    fn help_wins_and_mistakes_are_named() {
        assert_eq!(parse_words(&["doc", "--help"]), Ok(Request::Help));
        assert_eq!(parse_words(&["-h"]), Ok(Request::Help));
        for (words, message) in [
            (&[][..], "say what to export: `doc`, `folder` or `changes`"),
            (
                &["page", "x"][..],
                "`page` is not something to export; give `doc`, `folder` or `changes`",
            ),
            (
                &["doc", "main"][..],
                "`export doc` takes a revision and a path, such as `main docs/guide.md`",
            ),
            (
                &["doc", "main", "a.txt"][..],
                "`a.txt` is not the path of a Markdown file from the repository root",
            ),
            (
                &["doc", "main", "../a.md"][..],
                "`../a.md` is not the path of a Markdown file from the repository root",
            ),
            (
                &["folder", "main"][..],
                "`export folder` takes a revision and a folder, such as `main docs/decisions`",
            ),
            (
                &["folder", "main", "../x"][..],
                "`../x` is not a folder inside the repo; write it like `docs` or `specs/api`",
            ),
            (
                &["changes"][..],
                "give a range, such as `main...feature` or `auto`",
            ),
            (
                &["changes", "a..b", "c"][..],
                "give one range; put folders after `--`",
            ),
            (
                &["changes", "a..b", "--base", "dev"][..],
                "`--base` names the branch `auto` compares with; `a..b` names its own points",
            ),
            (
                &["doc", "main", "a.md", "--since", "2026-10-01"][..],
                "`--since` belongs to `export changes`, not `export doc`",
            ),
            (
                &["folder", "main", "docs", "--base", "dev"][..],
                "`--base` belongs to `export changes`, not `export folder`",
            ),
            (
                &["doc", "main", "a.md", "--", "docs"][..],
                "folders after `--` belong to `export changes`; `export doc` names its own",
            ),
            (
                &["doc", "main", "a.md", "--nope"][..],
                "unknown option `--nope`",
            ),
            (&["doc", "main", "a.md", "-o"][..], "`-o` needs a value"),
        ] {
            assert_eq!(refused(words), message, "{words:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn words_that_are_not_text_are_refused() {
        use std::os::unix::ffi::OsStringExt;
        let bad = OsString::from_vec(vec![b'a', 0xff]);
        assert_eq!(
            parse(&[OsString::from("doc"), bad.clone()]).unwrap_err(),
            "`a\u{fffd}` is not valid text"
        );
        let after = [OsString::from("--max-size"), bad];
        assert_eq!(
            parse(&after).unwrap_err(),
            "the value of `--max-size`, `a\u{fffd}`, is not valid text"
        );
    }
}
