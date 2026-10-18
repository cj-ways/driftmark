//! The HTML pages, in the review-first design of decision 0010.
//!
//! - The home page is an inbox: the branches whose documents changed,
//!   grouped by their highest level, each with its counts, its time and the
//!   document to start with.
//! - A reading page shows a document with its title, status pills, summary
//!   and "on this page", and marks the sections that open branches change.
//! - A change page has a rail with every change, a box that says where to
//!   look first, and one card per change.
//!
//! `docs/features/pages.md` says what a reader sees on each. The document
//! HTML and the change HTML come from `driftmark_core::render`; this module
//! writes the page around them. Every piece of text from the repository is
//! escaped. The pages load only the program's own script and style sheet, as
//! the security headers demand, so nothing here is an inline script or an
//! inline style.

use std::collections::HashMap;
use std::fmt::Write as _;

use driftmark_core::diff::{
    Change, ChangeList, DocumentChanges, FileState, Level, Subject, Summary,
};
use driftmark_core::document::{Field, MetaValue};
use driftmark_core::git::RepoPath;
use driftmark_core::render::{Section, words};
use driftmark_core::rules::{Classification, Lifecycle};

use super::address;
use super::assets::Assets;
use super::compare::Skipped;
use super::repo::{BranchCount, Home};
use crate::diff::text::level_name;

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

/// The tab of the top bar a page belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tab {
    /// The inbox.
    Inbox,
    /// The working tree's changes.
    Checkout,
    /// No tab.
    None,
}

/// What every page has around its content.
pub(crate) struct Frame<'a> {
    /// The page's title, without "driftmark".
    pub title: &'a str,
    /// Whether and when it reloads.
    pub live: Live,
    /// The tab it belongs to.
    pub tab: Tab,
    /// Whether the repository has a working tree, which gets its own tab.
    pub worktree: bool,
    /// The revision the search box searches, when the page shows one.
    pub revision: Option<&'a str>,
}

impl<'a> Frame<'a> {
    /// A frame with no tab and no revision, which never reloads.
    pub(crate) fn plain(title: &'a str, worktree: bool) -> Frame<'a> {
        Frame {
            title,
            live: Live::No,
            tab: Tab::None,
            worktree,
            revision: None,
        }
    }
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

/// A whole page around `content`, which brings its own `<main>`.
///
/// The page script loads in `<head>` without `defer`, after the style
/// sheet: it sets the reader's chosen theme before the first paint, and
/// waits for the page to load before it does anything else. The browser
/// fetches it beside the style sheet, which blocks the first paint anyway.
pub(crate) fn page(assets: &Assets, frame: &Frame<'_>, content: &str) -> String {
    let live = match frame.live {
        Live::No => "",
        Live::Worktree => " data-live=\"worktree\"",
        Live::Home => " data-live=\"home\"",
    };
    let revision = frame.revision.map_or_else(String::new, |revision| {
        format!(" data-revision=\"{}\"", escape(revision))
    });
    let current = |tab: Tab| {
        if frame.tab == tab {
            " aria-current=\"page\""
        } else {
            ""
        }
    };
    let checkout = if frame.worktree {
        format!(
            "<a href=\"/changes/auto\"{}>This checkout</a>",
            current(Tab::Checkout)
        )
    } else {
        String::new()
    };
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <meta name=\"color-scheme\" content=\"light dark\">\n\
         <title>{title} · driftmark</title>\n\
         <link rel=\"stylesheet\" href=\"{css}\">\n\
         <script src=\"{js}\"></script>\n</head>\n<body{live}{revision}>\n\
         <a class=\"skip\" href=\"#Main\">Skip to the content</a>\n\
         <header class=\"top\"><a class=\"brand\" href=\"/\">driftmark</a>\
         <nav class=\"tabs\" aria-label=\"Sections\"><a href=\"/\"{inbox}>Inbox</a>{checkout}</nav>\
         <form class=\"search\" role=\"search\" action=\"/\" hidden>\
         <label class=\"vh\" for=\"Search\">Search the documents</label>\
         <input id=\"Search\" type=\"search\" name=\"q\" placeholder=\"Search\" autocomplete=\"off\" spellcheck=\"false\">\
         <div class=\"results\" id=\"Results\" role=\"region\" aria-label=\"Search results\" aria-live=\"polite\" hidden></div></form>\
         <button type=\"button\" class=\"tbtn\" data-theme-button hidden><span aria-hidden=\"true\">◐</span> <span class=\"tbtn-word\">Theme</span></button>\
         </header>\n{content}</body>\n</html>\n",
        title = escape(frame.title),
        css = assets.address("page.css"),
        js = assets.address("page.js"),
        inbox = current(Tab::Inbox),
    )
}

/// A page with no style sheet and no script, in the browser's own light
/// look: the answer to a request without the token, or for another host.
/// Such a request cannot load the program's files either, and a page that
/// asked for them would show the browser's dark default colours, whose
/// links fail contrast.
pub(crate) fn bare(title: &str, content: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <meta name=\"color-scheme\" content=\"light\">\n\
         <title>{} · driftmark</title>\n</head>\n<body>\n{content}</body>\n</html>\n",
        escape(title)
    )
}

/// A time in seconds since 1970 as `2026-10-18 09:30 UTC`.
pub(crate) fn utc(seconds: i64) -> String {
    let (year, month, day) = civil(seconds.div_euclid(86_400));
    let in_day = seconds.rem_euclid(86_400);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        in_day / 3600,
        in_day % 3600 / 60
    )
}

/// A time as the `datetime` attribute of `<time>`: `2026-10-18T09:30:00Z`.
fn iso(seconds: i64) -> String {
    let (year, month, day) = civil(seconds.div_euclid(86_400));
    let in_day = seconds.rem_euclid(86_400);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        in_day / 3600,
        in_day % 3600 / 60,
        in_day % 60
    )
}

/// Howard Hinnant's days-to-civil algorithm, proleptic Gregorian.
fn civil(days: i64) -> (i64, i64, i64) {
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
    (year_of_era + era * 400 + i64::from(month <= 2), month, day)
}

/// How long ago a time was, in words: `just now`, `5 minutes ago`,
/// `3 hours ago`, `yesterday`, `4 days ago`, or the date after 30 days.
pub(crate) fn ago(seconds: i64, now: i64) -> String {
    let past = now - seconds;
    let plural = |n: i64, word: &str| {
        if n == 1 {
            format!("1 {word} ago")
        } else {
            format!("{n} {word}s ago")
        }
    };
    match past {
        ..60 => "just now".to_owned(),
        60..3600 => plural(past / 60, "minute"),
        3600..86_400 => plural(past / 3600, "hour"),
        86_400..172_800 => "yesterday".to_owned(),
        172_800..2_592_000 => plural(past / 86_400, "day"),
        _ => utc(seconds)[..10].to_owned(),
    }
}

/// A `<time>` element: how long ago, with the exact time as its title.
fn time(seconds: i64, now: i64) -> String {
    format!(
        "<time datetime=\"{}\" title=\"{}\">{}</time>",
        iso(seconds),
        utc(seconds),
        ago(seconds, now)
    )
}

/// The seconds since 1970 now.
pub(crate) fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
        })
}

/// The first 12 characters of an id, enough to find it again.
pub(crate) fn short(id: &str) -> &str {
    id.get(..12).unwrap_or(id)
}

fn plural(n: usize, word: &str) -> String {
    if n == 1 {
        format!("1 {word}")
    } else if word.ends_with("ch") {
        format!("{n} {word}es")
    } else {
        format!("{n} {word}s")
    }
}

/// A level badge: its sign comes from the style sheet, its word is here, so
/// the level never shows by colour alone.
fn badge(level: Level) -> String {
    format!(
        "<span class=\"lvl\" data-level=\"{}\">{}</span>",
        level_name(level),
        words::level(level)
    )
}

/// `7 decision · 18 fact · 6 wording`, levels with no changes left out.
fn level_counts(summary: &Summary) -> String {
    let parts: Vec<String> = Level::HIGHEST_FIRST
        .into_iter()
        .filter(|level| summary.levels.get(*level) > 0)
        .map(|level| format!("{} {}", summary.levels.get(level), level_name(level)))
        .collect();
    parts.join(" · ")
}

/// A bar with one segment per level, as wide as its share of the changes.
/// It is an SVG, because the page's security policy allows no inline style
/// to set a width.
fn bar(summary: &Summary) -> String {
    if summary.changes == 0 {
        return String::new();
    }
    let mut out = format!(
        "<svg class=\"bar\" viewBox=\"0 0 {} 1\" preserveAspectRatio=\"none\" aria-hidden=\"true\" focusable=\"false\">",
        summary.changes
    );
    let mut x = 0;
    for level in Level::HIGHEST_FIRST {
        let n = summary.levels.get(level);
        if n > 0 {
            let _ = write!(
                out,
                "<rect class=\"seg\" data-level=\"{}\" x=\"{x}\" width=\"{n}\" height=\"1\"/>",
                level_name(level)
            );
            x += n;
        }
    }
    out.push_str("</svg>");
    out
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

/// The words for each level's group on the home page.
fn group_title(level: Option<Level>) -> &'static str {
    match level {
        Some(Level::Decision) => "Decisions changed: review these first",
        Some(Level::Rule) => "Rules changed",
        Some(Level::Fact) => "Facts changed",
        Some(Level::Wording) => "Only wording changed",
        Some(Level::Formatting) => "Only formatting changed",
        None => "Nothing to rank",
    }
}

/// The home page: the working tree, then the branches whose documents
/// differ from the default branch, grouped by their highest level, newest
/// first in each group.
pub(crate) fn home(home: &Home, worktree: &WorktreeLine<'_>, now: i64) -> String {
    let mut out = String::from("<main id=\"Main\" class=\"home\">\n<header class=\"page-head\">");
    let listed = home.changed.len();
    let kicker = match &home.default {
        Some(default) => format!(
            "{} {} documents · compared with <code>{}</code> at <code>{}</code>",
            plural(listed, "open branch"),
            if listed == 1 { "changes" } else { "change" },
            escape(&default.name),
            short(default.tip.as_hex())
        ),
        None => "No default branch".to_owned(),
    };
    let _ = writeln!(
        out,
        "<p class=\"kicker\">{kicker}</p><h1>Inbox</h1></header>"
    );
    worktree_card(&mut out, home, worktree, now);
    let Some(default) = &home.default else {
        let _ = writeln!(
            out,
            "<p class=\"note\">No default branch: none of {} exists, so no branch is compared.</p>\n</main>",
            driftmark_core::git::AUTO_BASES.join(", ")
        );
        return out;
    };
    if home.changed.is_empty() {
        out.push_str("<p class=\"note\">No open branch changes a document.</p>\n");
    }
    let levels = Level::HIGHEST_FIRST.into_iter().map(Some).chain([None]);
    for level in levels {
        let group: Vec<_> = home
            .changed
            .iter()
            .filter(|(_, count)| count.totals.highest == level)
            .collect();
        if group.is_empty() {
            continue;
        }
        let name = level.map_or("none", level_name);
        let shown = level.map_or_else(String::new, badge);
        let _ = writeln!(
            out,
            "<section class=\"group\" aria-labelledby=\"G-{name}\"><h2 id=\"G-{name}\" class=\"group-head\">{shown} <span>{}</span> <span class=\"group-n\">{}</span></h2>\n<ol class=\"cards\">",
            group_title(level),
            group.len()
        );
        for (branch, count) in group {
            let range = format!("{}...{}", default.name, branch.name);
            branch_card(
                &mut out,
                &branch.name,
                &range,
                branch.committed_at,
                count,
                now,
            );
        }
        out.push_str("</ol></section>\n");
    }
    let _ = writeln!(
        out,
        "<p class=\"left-out\">Not listed: {} without document changes, and {} that share no history with <code>{}</code>. <a href=\"/api/home\">This page as JSON</a>.</p>\n</main>",
        plural(home.unchanged, "branch"),
        plural(home.unrelated, "branch"),
        escape(&default.name)
    );
    out
}

fn worktree_card(out: &mut String, home: &Home, worktree: &WorktreeLine<'_>, now: i64) {
    let on = home.current.as_ref().map_or_else(String::new, |branch| {
        format!(" on <code>{}</code>", escape(branch))
    });
    let state = match worktree {
        WorktreeLine::None => return,
        WorktreeLine::Counting => "<p class=\"card-line\">Counting its changes…</p>".to_owned(),
        WorktreeLine::Counted {
            at,
            updating,
            result,
        } => {
            let again = if *updating {
                " · counting again…"
            } else {
                ""
            };
            match result {
                Ok(list) if list.totals.changes == 0 => format!(
                    "<p class=\"card-line\">No document changes · counted {}{again}</p>",
                    time(*at, now)
                ),
                Ok(list) => format!(
                    "<p class=\"card-line\">{} {} · {} · counted {}{again}</p>{}<p class=\"card-counts\">{}</p>",
                    list.totals.highest.map_or_else(String::new, badge),
                    plural(list.documents.len(), "document"),
                    plural(list.totals.changes, "change"),
                    time(*at, now),
                    bar(&list.totals),
                    level_counts(&list.totals)
                ),
                Err(why) => format!("<p class=\"card-line\">{}</p>", escape(why)),
            }
        }
    };
    let _ = writeln!(
        out,
        "<section class=\"checkout\" aria-labelledby=\"G-checkout\"><h2 id=\"G-checkout\" class=\"group-head\">This checkout{on}, committed or not</h2>\
         <div class=\"card\">{state}<a class=\"card-go\" href=\"/changes/auto\">See its changes →</a></div></section>"
    );
}

fn branch_card(
    out: &mut String,
    name: &str,
    range: &str,
    committed_at: i64,
    count: &BranchCount,
    now: i64,
) {
    let href = escape(&address::changes(range));
    let level = count.totals.highest.map_or("none", level_name);
    let _ = write!(
        out,
        "<li class=\"card\" data-level=\"{level}\"><div class=\"card-top\"><a class=\"card-name\" href=\"{href}\"><code>{}</code></a>{}</div>\
         <p class=\"card-line\">{} · {}</p>{}<p class=\"card-counts\">{}</p>",
        escape(name),
        time(committed_at, now),
        plural(count.documents, "document"),
        plural(count.totals.changes, "change"),
        bar(&count.totals),
        level_counts(&count.totals)
    );
    if let Some(start) = &count.start {
        let changes = start.summary.highest.map_or_else(
            || plural(start.summary.changes, "change"),
            |level| {
                plural(start.summary.levels.get(level), "change").replacen(
                    ' ',
                    &format!(" {} ", level_name(level)),
                    1,
                )
            },
        );
        let _ = write!(
            out,
            "<p class=\"card-start\">Start with <a href=\"{}\"><code>{}</code></a>: {changes}</p>",
            escape(&format!(
                "{}#{}",
                address::changes(range),
                section_id(&start.path)
            )),
            escape(&start.path)
        );
    }
    let _ = writeln!(
        out,
        "<a class=\"card-go\" href=\"{href}\">Review<span class=\"vh\"> {}</span> →</a></li>",
        escape(name)
    );
}

/// The id of a document's section on a change page: a `D` and a hash of
/// its path. The capital letter keeps it apart from every heading anchor.
pub(crate) fn section_id(path: &str) -> String {
    format!("D{:08x}", super::fingerprint(path.as_bytes()) >> 32)
}

/// The prefix of a document's change ids on a change page. It depends only
/// on the path, so a document's change HTML is the same on every page that
/// shows it, and the cache keeps one copy.
pub(crate) fn change_prefix(path: &str) -> String {
    format!("C{:08x}", super::fingerprint(path.as_bytes()) >> 32)
}

// ---- the reading page ----------------------------------------------------

/// One open branch that changes the document a reading page shows.
pub(crate) struct Waiting {
    /// The branch, or `This checkout` for the working tree.
    pub name: String,
    /// The range that shows its changes.
    pub range: String,
    /// When its tip was made, or when the working tree was counted.
    pub at: i64,
    /// The document's changes on that branch.
    pub changes: DocumentChanges,
}

/// What a reading page shows.
pub(crate) struct Reading<'a> {
    /// The document's path.
    pub path: &'a RepoPath,
    /// The revision as the address wrote it.
    pub revision: &'a str,
    /// The commit, or `None` for the working tree.
    pub commit: Option<&'a str>,
    /// The rules file's reading of the document.
    pub classification: Option<&'a Classification>,
    /// The document's metadata fields: its front matter and masthead.
    pub fields: &'a [Field],
    /// The document's headings.
    pub outline: &'a [Section],
    /// The document's HTML, or why it cannot be shown.
    pub body: &'a str,
    /// Whether the document could be read.
    pub readable: bool,
    /// The open branches that change it.
    pub waiting: &'a [Waiting],
}

/// How many changes each section has waiting on open branches, by heading
/// id. A change counts for the innermost heading above it that the
/// document still has.
fn pending(outline: &[Section], waiting: &[Waiting]) -> HashMap<String, usize> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for branch in waiting {
        for change in &branch.changes.changes {
            let Subject::Block { old, new } = &change.subject else {
                continue;
            };
            let Some(block) = new.block.as_ref().or(old.block.as_ref()) else {
                continue;
            };
            let found = block
                .heading_path
                .iter()
                .rev()
                .find_map(|title| outline.iter().find(|section| &section.text == title));
            if let Some(section) = found {
                *counts.entry(section.id.clone()).or_default() += 1;
            }
        }
    }
    counts
}

/// A metadata field's text by its key, ignoring case: the status or date a
/// document writes when no rules file type says where to read it.
fn field<'f>(fields: &'f [Field], key: &str) -> Option<&'f str> {
    fields.iter().find_map(|field| match &field.value {
        MetaValue::Text(text) if field.key.eq_ignore_ascii_case(key) && !text.trim().is_empty() => {
            Some(text.as_str())
        }
        _ => None,
    })
}

/// The status pills of a reading page: type, status, lifecycle and date.
/// The status and the date come from the rules file's type, or, when it
/// names no place for them, from the document's `status` and `date` fields.
fn pills(classification: Option<&Classification>, fields: &[Field]) -> String {
    let Some(class) = classification else {
        return String::new();
    };
    let mut class = class.clone();
    if class.status.is_none() {
        class.status = field(fields, "status").map(str::to_owned);
    }
    if class.date.is_none() {
        class.date = field(fields, "date").map(str::to_owned);
    }
    let mut items = Vec::new();
    if class.doc_type != "document" {
        items.push(format!(
            "<li class=\"pill\" data-pill=\"type\">{}</li>",
            escape(&capitalized(&class.doc_type))
        ));
    }
    if let Some(status) = &class.status {
        items.push(format!(
            "<li class=\"pill\" data-pill=\"status\">Status: {}</li>",
            escape(status)
        ));
    }
    if let Some(lifecycle) = class.lifecycle {
        let word = match lifecycle {
            Lifecycle::Current => "Current",
            Lifecycle::Superseded => "Superseded",
            Lifecycle::Archived => "Archived",
        };
        items.push(format!(
            "<li class=\"pill\" data-pill=\"lifecycle\" data-lifecycle=\"{}\">{word}</li>",
            word.to_ascii_lowercase()
        ));
    }
    if let Some(date) = &class.date {
        items.push(format!(
            "<li class=\"pill\" data-pill=\"date\">Dated {}</li>",
            escape(date)
        ));
    }
    if let Some(by) = &class.superseded_by {
        items.push(format!(
            "<li class=\"pill\" data-pill=\"superseded-by\">Superseded by {}</li>",
            escape(by)
        ));
    }
    if items.is_empty() {
        return String::new();
    }
    format!(
        "<ul class=\"pills\" data-gen aria-label=\"About this document\">{}</ul>\n",
        items.concat()
    )
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().collect::<String>() + chars.as_str()
    })
}

/// Puts `extra` into the document HTML right after the title's heading, or
/// returns it in front when the document has no such heading.
fn after_title(body: &str, title: Option<&Section>, extra: &str) -> String {
    let at = title.and_then(|title| {
        let opening = format!("<h1 id=\"{}\"", escape(&title.id));
        let start = body.find(&opening)?;
        let close = body[start..].find("</h1>")? + start + "</h1>".len();
        Some(close + usize::from(body[close..].starts_with('\n')))
    });
    match at {
        Some(at) => format!("{}{extra}{}", &body[..at], &body[at..]),
        None => format!("{extra}{body}"),
    }
}

/// Adds a flag to each heading with changes waiting, before its anchor
/// link, pointing at the first branch's changes of the document.
fn flag_sections(body: &str, counts: &HashMap<String, usize>, href: &str) -> String {
    let mut out = body.to_owned();
    for (id, count) in counts {
        let anchor = format!("<a class=\"anchor\" href=\"#{}\"", escape(id));
        if let Some(at) = out.find(&anchor) {
            let words = if *count == 1 {
                "1 change waits".to_owned()
            } else {
                format!("{count} changes wait")
            };
            let flag = format!(
                "<a class=\"pending-flag\" href=\"{}\" data-gen>{words}</a>",
                escape(href)
            );
            out.insert_str(at, &flag);
        }
    }
    out
}

/// A reading page.
pub(crate) fn reading(page: &Reading<'_>, now: i64) -> String {
    let path = page.path.to_lossy();
    let counts = pending(page.outline, page.waiting);
    let mut out = String::from("<main id=\"Main\" class=\"read\">\n");
    waiting_box(&mut out, page, now);
    let at = match page.commit {
        Some(commit) => format!(
            "at <code>{}</code>, commit <code>{}</code>",
            escape(page.revision),
            short(commit)
        ),
        None => "in the working tree, as it is on disk now".to_owned(),
    };
    let _ = write!(
        out,
        "<article class=\"doc\">\n<header class=\"doc-head\"><p class=\"kicker\"><code>{}</code> {at} · <a href=\"{}\">JSON</a></p></header>\n",
        escape(&path),
        escape(&address::json(&address::document(page.revision, page.path)))
    );
    toc(&mut out, page.outline, &counts);
    let title = page.outline.iter().find(|section| section.level == 1);
    let pills = pills(page.classification, page.fields);
    let body = if page.readable {
        let href = page.waiting.first().map_or_else(String::new, |branch| {
            address::document_changes(&branch.range, page.path)
        });
        let flagged = flag_sections(page.body, &counts, &href);
        after_title(&flagged, title, &pills)
    } else {
        format!("{pills}{}", page.body)
    };
    let _ = write!(
        out,
        "<div class=\"doc-body\">\n{body}</div>\n</article>\n</main>\n"
    );
    out
}

fn waiting_box(out: &mut String, page: &Reading<'_>, now: i64) {
    if page.waiting.is_empty() {
        return;
    }
    let count = page.waiting.len();
    let _ = write!(
        out,
        "<aside class=\"waiting\" aria-labelledby=\"Waiting\"><p id=\"Waiting\" class=\"waiting-title\">{} {} this document</p><ul class=\"waiting-list\">",
        plural(count, "open branch"),
        if count == 1 { "changes" } else { "change" }
    );
    for branch in page.waiting {
        let summary = &branch.changes.summary;
        let _ = write!(
            out,
            "<li><a href=\"{}\"><code>{}</code></a> {} {}, {}</li>",
            escape(&address::document_changes(&branch.range, page.path)),
            escape(&branch.name),
            summary.highest.map_or_else(String::new, badge),
            plural(summary.changes, "change"),
            time(branch.at, now)
        );
    }
    out.push_str("</ul></aside>\n");
}

fn toc(out: &mut String, outline: &[Section], counts: &HashMap<String, usize>) {
    let sections: Vec<&Section> = outline
        .iter()
        .filter(|section| (2..=3).contains(&section.level))
        .collect();
    if sections.len() < 2 {
        return;
    }
    out.push_str("<nav class=\"toc\" aria-labelledby=\"Toc\"><details data-wide-open><summary id=\"Toc\">On this page");
    if !counts.is_empty() {
        out.push_str(" <span class=\"toc-hint\">· numbers count changes waiting</span>");
    }
    out.push_str("</summary><ol>");
    for section in sections {
        let pending = counts.get(&section.id).map_or_else(String::new, |n| {
            format!(
                " <span class=\"pending\" title=\"Changes wait on an open branch\">{n}<span class=\"vh\"> {} waiting</span></span>",
                if *n == 1 { "change" } else { "changes" }
            )
        });
        let _ = write!(
            out,
            "<li data-level=\"{}\"><a href=\"#{}\">{}</a>{pending}</li>",
            section.level,
            escape(&section.id),
            escape(&section.text)
        );
    }
    out.push_str("</ol></details></nav>\n");
}

// ---- change pages --------------------------------------------------------

/// One document whose changes a change page shows in full.
pub(crate) struct Shown<'a> {
    /// Its changes.
    pub document: &'a DocumentChanges,
    /// Its title, from its newer version, when it has one.
    pub title: Option<String>,
    /// Its change HTML from `driftmark_core::render::changes`.
    pub html: &'a str,
}

/// What a change page shows.
pub(crate) struct Review<'a> {
    /// The range as the address wrote it.
    pub label: &'a str,
    /// The page's heading: the branch name for a branch's range, otherwise
    /// the range.
    pub heading: &'a str,
    /// The range's documents with their counts, sorted for a reader.
    pub list: &'a ChangeList,
    /// The documents shown in full, in the list's order.
    pub shown: &'a [Shown<'a>],
    /// The files that could not be compared.
    pub skipped: &'a [Skipped],
    /// The revision to read a changed document at: the newer side.
    pub head: &'a str,
    /// The revision to read a deleted document at: the older side.
    pub base: Option<&'a str>,
    /// The one document of a document's own page.
    pub single: Option<&'a RepoPath>,
}

/// The first words of a change's text, for the rail.
fn rail_label(change: &Change) -> String {
    let text = match &change.subject {
        Subject::Document { document } => document
            .title
            .clone()
            .unwrap_or_else(|| format!("{} lines", document.lines)),
        Subject::Block { old, new } => new
            .block
            .as_ref()
            .or(old.block.as_ref())
            .map_or_else(String::new, |block| block.text.clone()),
    };
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut cut: String = flat.chars().take(48).collect();
    if cut.len() < flat.len() {
        cut.push('…');
    }
    format!("{}: {cut}", words::kind(change))
}

/// The largest number of changes the rail lists for one document; the
/// rest are counted.
const RAIL_PER_DOCUMENT: usize = 80;

/// The most documents a range's page lists below the ones it shows in
/// full. They are the highest ranked; the range's JSON lists every one. A
/// month of a large repository changes thousands of documents, and listing
/// them all would make the page heavier than the 100 KB limit
/// (`docs/features/pages.md`).
const MORE_LISTED: usize = 300;

fn rail(out: &mut String, page: &Review<'_>) {
    let totals = &page.list.totals;
    let _ = write!(
        out,
        "<nav class=\"rail\" aria-label=\"Every change\"><details class=\"rail-box\" data-wide-open><summary><span class=\"rail-title\"><code>{}</code></span><span class=\"rail-progress\">{} in {}</span></summary>",
        escape(page.heading),
        plural(totals.changes, "change"),
        plural(page.list.documents.len(), "document")
    );
    out.push_str("<ul class=\"chips\" aria-label=\"Changes by level\">");
    for level in Level::HIGHEST_FIRST {
        let n = totals.levels.get(level);
        if n > 0 {
            let _ = write!(
                out,
                "<li>{} <span class=\"chip-n\">{n}</span></li>",
                badge(level)
            );
        }
    }
    out.push_str("</ul><ol class=\"rail-docs\">");
    for shown in page.shown {
        let document = shown.document;
        let name = shown.title.as_deref().unwrap_or(&document.path);
        let _ = write!(
            out,
            "<li class=\"rail-doc\"><a class=\"rail-doc-name\" href=\"#{}\">{}</a> {}<ol class=\"rail-changes\">",
            section_id(&document.path),
            escape(name),
            document.summary.highest.map_or_else(String::new, badge)
        );
        let prefix = change_prefix(&document.path);
        for (index, change) in document.changes.iter().enumerate().take(RAIL_PER_DOCUMENT) {
            let _ = write!(
                out,
                "<li data-level=\"{}\"><a href=\"#{prefix}.{index}\"><span class=\"dot\" aria-hidden=\"true\"></span><span class=\"rail-label\">{}</span><span class=\"vh\"> ({})</span></a></li>",
                level_name(change.level),
                escape(&rail_label(change)),
                words::level(change.level)
            );
        }
        let more = document.changes.len().saturating_sub(RAIL_PER_DOCUMENT);
        if more > 0 {
            let _ = write!(out, "<li class=\"rail-more\">and {more} more</li>");
        }
        out.push_str("</ol></li>");
    }
    let rest = page.list.documents.len().saturating_sub(page.shown.len());
    if rest > 0 {
        let _ = write!(
            out,
            "<li class=\"rail-doc\"><a class=\"rail-doc-name\" href=\"#More\">{}, each on its own page</a></li>",
            plural(rest, "more document")
        );
    }
    out.push_str("</ol></details></nav>\n");
}

/// The address of one document's changes in the range, from a path as
/// text.
fn doc_changes_address(label: &str, path: &str) -> String {
    RepoPath::new(path.as_bytes()).map_or_else(
        || address::changes(label),
        |path| address::document_changes(label, &path),
    )
}

/// The box at the top of a change page: what needs the reader's eyes first.
fn attention(out: &mut String, page: &Review<'_>) {
    let totals = &page.list.totals;
    let Some(first) = page.list.documents.first() else {
        return;
    };
    let Some(level) = totals.highest else {
        return;
    };
    let shown = page
        .shown
        .iter()
        .find(|shown| shown.document.path == first.path);
    let name = shown
        .and_then(|shown| shown.title.as_deref())
        .unwrap_or(&first.path);
    let target = match shown {
        Some(_) => format!("#{}", section_id(&first.path)),
        None => doc_changes_address(page.label, &first.path),
    };
    let n = first.summary.levels.get(level);
    let _ = write!(
        out,
        "<section class=\"attention\" aria-labelledby=\"Attention\"><h2 id=\"Attention\" class=\"vh\">What needs your eyes</h2>\
         <p class=\"att-lead\">{} <strong>{}</strong> in <a href=\"{}\">{}</a>. Start there.</p><ul class=\"att-rest\">",
        badge(level),
        plural(n, &format!("{} change", level_name(level))),
        escape(&target),
        escape(name)
    );
    for other in Level::HIGHEST_FIRST {
        let count = totals.levels.get(other);
        if count == 0 {
            continue;
        }
        let rest = if other == level { count - n } else { count };
        if rest == 0 {
            continue;
        }
        let elsewhere = if other == level { " elsewhere" } else { "" };
        let folded = if other == Level::Formatting {
            "-only changes, folded"
        } else {
            " changes"
        };
        let _ = write!(
            out,
            "<li>{rest} {}{folded}{elsewhere}</li>",
            level_name(other)
        );
    }
    out.push_str("</ul></section>\n");
}

/// The counts of a document's changes, as `14 changes: 7 decision, 1
/// wording, 6 formatting`.
fn document_counts(summary: &Summary) -> String {
    let levels = level_counts(summary).replace(" · ", ", ");
    if levels.is_empty() {
        plural(summary.changes, "change")
    } else {
        format!("{}: {levels}", plural(summary.changes, "change"))
    }
}

fn document_section(out: &mut String, page: &Review<'_>, shown: &Shown<'_>) {
    let document = shown.document;
    let id = section_id(&document.path);
    let revision = match document.file {
        FileState::Removed => page.base.unwrap_or(page.head),
        _ => page.head,
    };
    let read =
        RepoPath::new(document.path.as_bytes()).map(|path| address::document(revision, &path));
    let name = shown.title.as_deref().unwrap_or(&document.path);
    let heading = match &read {
        Some(read) => format!("<a href=\"{}\">{}</a>", escape(read), escape(name)),
        None => escape(name),
    };
    let state = match document.file {
        FileState::Added => " · new",
        FileState::Removed => " · deleted",
        FileState::Modified => "",
    };
    let own = if page.single.is_some() {
        String::new()
    } else {
        format!(
            " · <a href=\"{}\">its own page</a>",
            escape(&doc_changes_address(page.label, &document.path))
        )
    };
    let status = document.status.as_ref().map_or_else(String::new, |status| {
        format!(
            "<span class=\"docchg-status\">status {} → {}</span>",
            escape(status.old.as_deref().unwrap_or("none")),
            escape(status.new.as_deref().unwrap_or("none"))
        )
    });
    let _ = write!(
        out,
        "<section class=\"docchg\" id=\"{id}\" aria-labelledby=\"{id}-t\"><header class=\"docchg-head\"><h2 id=\"{id}-t\">{heading}</h2>\
         <p class=\"docchg-path\"><code>{}</code>{state}{own}</p><p class=\"docchg-meta\">{}<span>{}</span>{status}</p></header>\n{}</section>\n",
        escape(&document.path),
        document.summary.highest.map_or_else(String::new, badge),
        document_counts(&document.summary),
        shown.html
    );
}

/// A change page: a range's changes, or one document's.
pub(crate) fn review(page: &Review<'_>) -> String {
    let mut out = String::from("<div class=\"reviewwrap\">\n");
    rail(&mut out, page);
    let notes = page
        .list
        .range
        .as_ref()
        .map(|range| range.notes.join("; "))
        .unwrap_or_default();
    let base = page
        .list
        .range
        .as_ref()
        .and_then(|range| range.base.as_deref());
    let compared = match base {
        Some(base) => format!("compared from <code>{}</code>", short(base)),
        None => "compared with an empty history".to_owned(),
    };
    let back = match page.single {
        Some(_) => format!(
            " · <a href=\"{}\">every document of this range</a>",
            escape(&address::changes(page.label))
        ),
        None => String::new(),
    };
    let json = match page.single {
        Some(path) => address::json(&address::document_changes(page.label, path)),
        None => address::json(&address::changes(page.label)),
    };
    let _ = write!(
        out,
        "<main id=\"Main\" class=\"review\">\n<header class=\"page-head\"><p class=\"kicker\">Changes in <code>{}</code> · {compared}{}{back} · <a href=\"{}\">JSON</a></p><h1>{}</h1></header>\n",
        escape(page.label),
        if notes.is_empty() {
            String::new()
        } else {
            format!(" · {}", escape(&notes))
        },
        escape(&json),
        escape(page.heading)
    );
    if page.list.documents.is_empty() {
        out.push_str("<p class=\"note\">No document changed in this range.</p>\n");
    }
    attention(&mut out, page);
    for shown in page.shown {
        document_section(&mut out, page, shown);
    }
    let rest: Vec<_> = page
        .list
        .documents
        .iter()
        .filter(|document| {
            !page
                .shown
                .iter()
                .any(|shown| shown.document.path == document.path)
        })
        .collect();
    if !rest.is_empty() {
        let _ = write!(
            out,
            "<section class=\"more\" aria-labelledby=\"More\"><h2 id=\"More\">{} more, each on its own page</h2><ol class=\"more-list\">",
            plural(rest.len(), "document")
        );
        for document in rest.iter().take(MORE_LISTED) {
            let state = match document.file {
                FileState::Added => " · new",
                FileState::Removed => " · deleted",
                FileState::Modified => "",
            };
            let _ = write!(
                out,
                "<li>{} <a href=\"{}\"><code>{}</code></a><span class=\"more-n\">{}{state}</span></li>",
                document.summary.highest.map_or_else(String::new, badge),
                escape(&doc_changes_address(page.label, &document.path)),
                escape(&document.path),
                document_counts(&document.summary)
            );
        }
        out.push_str("</ol>");
        if rest.len() > MORE_LISTED {
            let _ = write!(
                out,
                "<p class=\"note\">And {} more, lower in the ranking. <a href=\"{}\">The range's JSON</a> lists every one.</p>",
                rest.len() - MORE_LISTED,
                escape(&json)
            );
        }
        out.push_str("</section>\n");
    }
    skipped_list(&mut out, page.skipped);
    out.push_str("</main>\n</div>\n");
    out
}

fn skipped_list(out: &mut String, skipped: &[Skipped]) {
    if skipped.is_empty() {
        return;
    }
    out.push_str("<section class=\"skipped\" aria-labelledby=\"Skipped\"><h2 id=\"Skipped\">Not compared</h2><ul>");
    for file in skipped {
        let _ = write!(
            out,
            "<li><code>{}</code>: {}</li>",
            escape(&file.path),
            escape(&file.reason)
        );
    }
    out.push_str("</ul></section>\n");
}

// ---- problems --------------------------------------------------------------

/// The page for an address that names nothing, or a read that failed.
pub(crate) fn problem(missing: bool, message: &str) -> String {
    let heading = if missing {
        "Not found"
    } else {
        "This page could not be made"
    };
    format!(
        "<main id=\"Main\" class=\"plain\">\n<h1>{heading}</h1>\n<p class=\"problem\">{}</p>\n<p><a href=\"/\">Back to the inbox</a></p>\n</main>\n",
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
        "<main id=\"Main\" class=\"plain\">\n<h1>Token needed</h1>\n<p>{first} Open the address that <code>driftmark serve</code> printed when it started; it ends in <code>?token=</code> and the token. This browser then remembers it.</p>\n</main>\n"
    )
}

#[cfg(test)]
mod tests;
