//! The HTML pages, as plain placeholders.
//!
//! The look and the real page templates belong to later lanes (decision
//! 0010). Until they land, a document shows as its source in `<pre>`, a
//! change list as the summary `driftmark diff` prints, with a link per
//! document, and one document's changes as a short text listing. Every page
//! links to its JSON, which is the contract the real pages will read.
//!
//! Every piece of text from the repository is escaped. The pages load only
//! the program's own script and style sheet, as the security headers demand.

use std::fmt::Write as _;

use driftmark_core::diff::{Change, ChangeList, FileState, Level, Subject};
use driftmark_core::git::RepoPath;

use super::address;
use super::assets::Assets;
use super::compare::{Parsed, Skipped};
use super::repo::Home;
use crate::diff::text::{self, Context, Terminal, level_name};

/// Which open pages reload, and on which event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Live {
    /// The page never reloads by itself.
    No,
    /// The page shows the working tree: it reloads when a file changes.
    Worktree,
    /// The home page: it reloads when the working tree was counted again.
    Home,
}

/// Escapes text for HTML, in element content and in quoted attributes.
pub(crate) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// A whole page around `body`.
pub(crate) fn page(assets: &Assets, title: &str, live: Live, body: &str) -> String {
    let live = match live {
        Live::No => "",
        Live::Worktree => " data-live=\"worktree\"",
        Live::Home => " data-live=\"home\"",
    };
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>{title} · driftmark</title>\n\
         <link rel=\"stylesheet\" href=\"{css}\">\n\
         <script src=\"{js}\" defer></script>\n</head>\n<body{live}>\n\
         <header><a href=\"/\">driftmark</a></header>\n<main>\n{body}</main>\n</body>\n</html>\n",
        title = escape(title),
        css = assets.address("page.css"),
        js = assets.address("page.js"),
    )
}

/// A time in seconds since 1970 as `2026-10-18 09:30 UTC`.
pub(crate) fn utc(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let in_day = seconds.rem_euclid(86_400);
    // Howard Hinnant's days-to-civil algorithm, proleptic Gregorian.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        in_day / 3600,
        in_day % 3600 / 60
    )
}

/// The first 12 characters of an id, enough to find it again.
pub(crate) fn short(id: &str) -> &str {
    id.get(..12).unwrap_or(id)
}

fn counts(level: Option<Level>, changes: usize, documents: usize) -> String {
    let level = level.map_or("none", level_name);
    let plural = |n: usize, word: &str| {
        if n == 1 {
            format!("{n} {word}")
        } else {
            format!("{n} {word}s")
        }
    };
    format!(
        "<span class=\"level\" data-level=\"{level}\">{level}</span>, {} in {}",
        plural(changes, "change"),
        plural(documents, "document")
    )
}

/// What the home page knows of the working tree.
pub(crate) enum WorktreeLine<'a> {
    /// The repo has no working tree.
    None,
    /// The first count is still running.
    Counting,
    /// The last count, and whether a newer one is running.
    Counted {
        /// When it finished.
        at: i64,
        /// Whether a newer count is running.
        updating: bool,
        /// The counts, or why `auto` cannot be counted.
        result: Result<&'a ChangeList, &'a str>,
    },
}

/// The home page: the branches whose documents differ from the default
/// branch, newest first.
pub(crate) fn home(home: &Home, worktree: &WorktreeLine<'_>) -> String {
    let mut out = String::from("<h1>Branches with document changes</h1>\n");
    let checkout = match &home.current {
        Some(branch) => format!("This checkout, on <code>{}</code>", escape(branch)),
        None => "This checkout".to_owned(),
    };
    match worktree {
        WorktreeLine::None => {}
        WorktreeLine::Counting => {
            let _ = writeln!(
                out,
                "<p class=\"worktree\"><a href=\"/changes/auto\">{checkout}</a>: counting its changes…</p>"
            );
        }
        WorktreeLine::Counted {
            at,
            updating,
            result,
        } => {
            let state = match result {
                Ok(list) => counts(
                    list.totals.highest,
                    list.totals.changes,
                    list.documents.len(),
                ),
                Err(why) => escape(why),
            };
            let updating = if *updating { ", counting again…" } else { "" };
            let _ = writeln!(
                out,
                "<p class=\"worktree\"><a href=\"/changes/auto\">{checkout}</a>, committed or not: {state} (counted {}{updating})</p>",
                utc(*at)
            );
        }
    }
    let Some(default) = &home.default else {
        let _ = writeln!(
            out,
            "<p>No default branch: none of {} exists, so no branch is compared.</p>",
            driftmark_core::git::AUTO_BASES.join(", ")
        );
        return out;
    };
    let _ = writeln!(
        out,
        "<p>Compared with <code>{}</code> at <code>{}</code>, each from where it left it.</p>",
        escape(&default.name),
        short(default.tip.as_hex())
    );
    if home.changed.is_empty() {
        out.push_str("<p>No branch changes a document.</p>\n");
    } else {
        out.push_str("<ol class=\"inbox\">\n");
        for (branch, count) in &home.changed {
            let range = format!("{}...{}", default.name, branch.name);
            let _ = writeln!(
                out,
                "<li><a href=\"{}\">{}</a>: {}, committed {}</li>",
                escape(&address::changes(&range)),
                escape(&branch.name),
                counts(count.totals.highest, count.totals.changes, count.documents),
                utc(branch.committed_at)
            );
        }
        out.push_str("</ol>\n");
    }
    let _ = writeln!(
        out,
        "<p class=\"left-out\">Not listed: {} branches without document changes, {} that share no history with <code>{}</code>.</p>",
        home.unchanged,
        home.unrelated,
        escape(&default.name)
    );
    out.push_str("<p><a href=\"/api/home\">This page as JSON</a></p>\n");
    out
}

/// A document's part of its reading page: its source, or why it cannot be
/// read. This part is cached by blob.
pub(crate) fn document_body(parsed: &Parsed) -> String {
    match parsed {
        Ok(document) => format!("<pre class=\"source\">{}</pre>\n", escape(&document.source)),
        Err(error) => format!(
            "<p class=\"problem\">This document cannot be shown: {}.</p>\n",
            escape(&error.to_string())
        ),
    }
}

/// A reading page around its cached part.
pub(crate) fn document(
    title: &str,
    revision: &str,
    commit: Option<&str>,
    path: &RepoPath,
    body: &str,
) -> String {
    let at = match commit {
        Some(commit) => format!(
            "at <code>{}</code>, commit <code>{}</code>",
            escape(revision),
            short(commit)
        ),
        None => "in the working tree, as it is on disk now".to_owned(),
    };
    format!(
        "<h1>{}</h1>\n<p class=\"place\"><code>{}</code> {at}. <a href=\"{}\">As JSON</a></p>\n{body}",
        escape(title),
        escape(&path.to_lossy()),
        escape(&address::json(&address::document(revision, path)))
    )
}

/// A range's change list: the summary `driftmark diff` prints, then a link
/// to each document's changes.
pub(crate) fn range(label: &str, list: &ChangeList, skipped: &[Skipped]) -> String {
    let context = Context {
        label,
        roots: &driftmark_core::git::Roots::all(),
        skipped: skipped.len(),
        terminal: Terminal::NONE,
    };
    let mut out = format!(
        "<h1>Changes in <code>{}</code></h1>\n<pre class=\"summary\">{}</pre>\n",
        escape(label),
        escape(&text::summary(list, &context))
    );
    if !list.documents.is_empty() {
        out.push_str("<ol class=\"documents\">\n");
        for document in &list.documents {
            let path = driftmark_core::git::RepoPath::new(document.path.as_bytes());
            let name = escape(&document.path);
            let state = match document.file {
                FileState::Added => " (new)",
                FileState::Removed => " (deleted)",
                FileState::Modified => "",
            };
            let level = document.summary.highest.map_or("none", level_name);
            match path {
                Some(path) => {
                    let _ = writeln!(
                        out,
                        "<li><a href=\"{}\">{name}</a>{state}: <span class=\"level\" data-level=\"{level}\">{level}</span></li>",
                        escape(&address::document_changes(label, &path))
                    );
                }
                None => {
                    let _ = writeln!(out, "<li>{name}{state}: {level}</li>");
                }
            }
        }
        out.push_str("</ol>\n");
    }
    out.push_str(&skipped_list(skipped));
    let _ = writeln!(
        out,
        "<p><a href=\"{}\">This list as JSON</a></p>",
        escape(&address::json(&address::changes(label)))
    );
    out
}

fn skipped_list(skipped: &[Skipped]) -> String {
    if skipped.is_empty() {
        return String::new();
    }
    let mut out = String::from("<h2>Not compared</h2>\n<ul class=\"skipped\">\n");
    for file in skipped {
        let _ = writeln!(
            out,
            "<li><code>{}</code>: {}</li>",
            escape(&file.path),
            escape(&file.reason)
        );
    }
    out.push_str("</ul>\n");
    out
}

/// One document's changes as a text listing: each change's level, kind,
/// signals and place, then its old and new text. This part is cached by the
/// pair of blobs.
pub(crate) fn changes_body(list: &ChangeList, skipped: &[Skipped]) -> String {
    let mut out = skipped_list(skipped);
    for document in &list.documents {
        let mut listing = String::new();
        for (number, change) in document.changes.iter().enumerate() {
            change_text(&mut listing, number + 1, change);
        }
        if document.changes.is_empty() {
            listing.push_str("Only blank lines changed.\n");
        }
        let _ = writeln!(out, "<pre class=\"changes\">{}</pre>", escape(&listing));
    }
    out
}

fn change_text(out: &mut String, number: usize, change: &Change) {
    let signals: Vec<String> = change
        .signals
        .iter()
        .map(|signal| {
            serde_json::to_string(signal)
                .unwrap_or_default()
                .trim_matches('"')
                .to_owned()
        })
        .collect();
    let kind = serde_json::to_string(&change.kind).unwrap_or_default();
    let _ = writeln!(
        out,
        "{number}. {}, {}: {}",
        level_name(change.level),
        kind.trim_matches('"'),
        signals.join(", ")
    );
    match &change.subject {
        Subject::Document { document } => {
            let title = document.title.as_deref().unwrap_or("untitled");
            let _ = writeln!(
                out,
                "   the whole document, \"{title}\": {} lines, {} bytes",
                document.lines, document.bytes
            );
        }
        Subject::Block { old, new } => {
            let place = new
                .block
                .as_ref()
                .or(old.block.as_ref())
                .map(|block| block.heading_path.join(" > "))
                .unwrap_or_default();
            if !place.is_empty() {
                let _ = writeln!(out, "   under: {place}");
            }
            for (sign, side) in [("-", old), ("+", new)] {
                if let Some(block) = &side.block {
                    for line in block.text.lines() {
                        let _ = writeln!(out, "   {sign} {line}");
                    }
                }
            }
        }
    }
}

/// One document's changes page around its cached part.
pub(crate) fn document_changes(label: &str, path: &RepoPath, body: &str) -> String {
    format!(
        "<h1><code>{}</code></h1>\n<p class=\"place\">Its changes in <a href=\"{}\"><code>{}</code></a>. <a href=\"{}\">As JSON</a></p>\n{body}",
        escape(&path.to_lossy()),
        escape(&address::changes(label)),
        escape(label),
        escape(&address::json(&address::document_changes(label, path)))
    )
}

/// The page for an address that names nothing, or a read that failed.
pub(crate) fn problem(missing: bool, message: &str) -> String {
    let heading = if missing {
        "Not found"
    } else {
        "This page could not be made"
    };
    format!(
        "<h1>{heading}</h1>\n<p class=\"problem\">{}</p>\n<p><a href=\"/\">Back to the branches</a></p>\n",
        escape(message)
    )
}

/// The page for a request without the token.
pub(crate) fn token_needed(wrong: bool) -> String {
    let first = if wrong {
        "That token is not this server's token."
    } else {
        "This driftmark server needs its token."
    };
    format!(
        "<h1>Token needed</h1>\n<p>{first} Open the address that <code>driftmark serve</code> printed when it started; it ends in <code>?token=</code> and the token. This browser then remembers it.</p>\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaping_covers_markup_and_quotes() {
        assert_eq!(
            escape("<a href=\"x\" title='y'>&</a>"),
            "&lt;a href=&quot;x&quot; title=&#39;y&#39;&gt;&amp;&lt;/a&gt;"
        );
    }

    #[test]
    fn times_read_as_utc_dates() {
        assert_eq!(utc(0), "1970-01-01 00:00 UTC");
        assert_eq!(utc(1_767_225_600), "2026-01-01 00:00 UTC");
        assert_eq!(utc(1_760_779_800), "2025-10-18 09:30 UTC");
        assert_eq!(utc(951_782_400), "2000-02-29 00:00 UTC");
        assert_eq!(utc(-86_400), "1969-12-31 00:00 UTC");
    }

    #[test]
    fn short_ids_keep_twelve_characters() {
        assert_eq!(short("0123456789abcdef"), "0123456789ab");
        assert_eq!(short("worktree"), "worktree");
    }
}
