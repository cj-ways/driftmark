//! The pages of an exported file: a document's reading page, a folder's
//! index, and a range's change list.
//!
//! They follow the server's pages (`serve/page.rs`, decision 0010) and use
//! the same classes, so the same style sheet draws them. They differ where a
//! file must: every link leads to a place inside the file or stays text;
//! every id is unique in the file, because one file holds many pages; and
//! nothing offers notes or "Seen" (decision 0004 keeps notes in the notes
//! file, not in what is shared).

use std::fmt::Write as _;

use driftmark_core::diff::{ChangeList, DocumentChanges, FileState, Level};
use driftmark_core::document::Field;
use driftmark_core::render::{Section, words};
use driftmark_core::rules::Classification;

use crate::diff::text::level_name;
use crate::serve::compare::Skipped;
use crate::serve::page::{
    after_title, badge, document_counts, escape, pills, plural, rail_label, short,
};

/// The two buttons a change card's head carries on the server, which an
/// exported file leaves out: it has no notes and no "Seen". This is the
/// exact text `driftmark_core::render::changes` writes; a test checks it.
pub(crate) const CARD_BUTTONS: &str = " <span class=\"acts\"><button type=\"button\" class=\"seen\" disabled title=\"Arrives with notes\">Seen</button> <button type=\"button\" class=\"note\" disabled title=\"Arrives with notes\">Add a note</button></span>";

/// How many changes the rail lists one by one, in the list's order; it
/// counts the rest. A month of a large repository holds thousands, and
/// listing each would double the file.
const RAIL_CHANGES: usize = 300;

/// How many changes of one document the rail lists one by one.
const RAIL_PER_DOCUMENT: usize = 80;

/// Where a page was read: the revision as written and its commit, or the
/// working tree.
pub(crate) fn at_words(revision: &str, commit: Option<&str>) -> String {
    match commit {
        Some(commit) => format!(
            "at <code>{}</code>, commit <code>{}</code>",
            escape(revision),
            short(commit)
        ),
        None => "in the working tree, as it was on disk".to_owned(),
    }
}

/// One document's reading page.
pub(crate) struct ReadingPage<'a> {
    /// Its id in the file, such as `R3`; its own ids start with `R3-`.
    pub id: &'a str,
    /// The document's path.
    pub path: &'a str,
    /// Where it was read, from [`at_words`].
    pub at: &'a str,
    /// The rules file's reading of it.
    pub classification: Option<&'a Classification>,
    /// Its metadata fields.
    pub fields: &'a [Field],
    /// Its headings, with ids that start with the page's id.
    pub outline: &'a [Section],
    /// Its HTML, or why it cannot be shown.
    pub body: &'a str,
    /// Whether it could be read.
    pub readable: bool,
    /// A link back to the page it was reached from, and its words.
    pub back: Option<(&'a str, &'a str)>,
}

/// A document's reading page, as a page of the file.
pub(crate) fn reading(page: &ReadingPage<'_>) -> String {
    let back = page.back.map_or_else(String::new, |(href, words)| {
        format!(" · <a href=\"{}\">{}</a>", escape(href), escape(words))
    });
    let mut out = format!(
        "<section class=\"xpage xread\" id=\"{id}\">\n<article class=\"doc\">\n<header class=\"doc-head\"><p class=\"kicker\"><code>{}</code> {}{back}</p></header>\n",
        escape(page.path),
        page.at,
        id = page.id,
    );
    toc(&mut out, page.id, page.outline);
    let pills = pills(page.classification, page.fields);
    let body = if page.readable {
        let title = page.outline.iter().find(|section| section.level == 1);
        after_title(page.body, title, &pills)
    } else {
        format!("{pills}{}", page.body)
    };
    let _ = write!(
        out,
        "<div class=\"doc-body\">\n{body}</div>\n</article>\n</section>\n"
    );
    out
}

/// "On this page": the document's sections of levels 2 and 3. Its label's
/// id starts with the page's id and holds a capital letter, which no
/// heading's id has.
fn toc(out: &mut String, id: &str, outline: &[Section]) {
    let sections: Vec<&Section> = outline
        .iter()
        .filter(|section| (2..=3).contains(&section.level))
        .collect();
    if sections.len() < 2 {
        return;
    }
    let _ = write!(
        out,
        "<nav class=\"toc\" aria-labelledby=\"{id}-Toc\"><details data-wide-open><summary id=\"{id}-Toc\">On this page</summary><ol>"
    );
    for section in sections {
        let _ = write!(
            out,
            "<li data-level=\"{}\"><a href=\"#{}\">{}</a></li>",
            section.level,
            escape(&section.id),
            escape(&section.text)
        );
    }
    out.push_str("</ol></details></nav>\n");
}

/// One document of a folder's index.
pub(crate) struct Listed<'a> {
    /// Its path.
    pub path: &'a str,
    /// Its title, when it has one.
    pub title: Option<&'a str>,
    /// Its summary, when it has one.
    pub summary: Option<&'a str>,
    /// The rules file's reading of it.
    pub classification: Option<&'a Classification>,
    /// Its metadata fields.
    pub fields: &'a [Field],
    /// Its page's id, or `None` when the file left its page out.
    pub id: Option<String>,
}

/// A folder's index: every document, grouped by the folder it sits in,
/// each with its title, path, pills and summary.
pub(crate) fn index(heading: &str, at: &str, listed: &[Listed<'_>], cut: &str) -> String {
    let mut out = format!(
        "<section class=\"xpage xindex\" id=\"Index\">\n<header class=\"page-head\"><p class=\"kicker\"><code>{}</code> {at} · {}</p><h1>{}</h1></header>\n{cut}",
        escape(heading),
        plural(listed.len(), "document"),
        escape(heading)
    );
    if listed.is_empty() {
        out.push_str("<p class=\"note\">This folder holds no Markdown document.</p>\n");
    }
    let mut folder: Option<&str> = None;
    for document in listed {
        let parent = document
            .path
            .rsplit_once('/')
            .map_or("", |(parent, _)| parent);
        if folder != Some(parent) {
            if folder.is_some() {
                out.push_str("</ol>\n");
            }
            let name = if parent.is_empty() { "/" } else { parent };
            let _ = write!(
                out,
                "<h2><code>{}</code></h2>\n<ol class=\"xlist\">",
                escape(name)
            );
            folder = Some(parent);
        }
        let name = escape(document.title.unwrap_or(document.path));
        let title = match &document.id {
            Some(page) => format!("<a class=\"x-title\" href=\"#{page}\">{name}</a>"),
            None => format!(
                "<span class=\"x-title\">{name}</span> <span class=\"x-out\">· not in this file</span>"
            ),
        };
        let summary = document.summary.map_or_else(String::new, |summary| {
            format!("<p class=\"x-summary\">{}</p>", escape(summary))
        });
        let _ = write!(
            out,
            "<li>{title}<span class=\"x-path\"><code>{}</code></span>{}{summary}</li>",
            escape(document.path),
            pills(document.classification, document.fields).trim_end()
        );
    }
    if folder.is_some() {
        out.push_str("</ol>\n");
    }
    out.push_str("</section>\n");
    out
}

/// One document of a range, as the file shows it.
pub(crate) struct Changed<'a> {
    /// Its changes.
    pub document: &'a DocumentChanges,
    /// Its number in the file: its section is `D<n>`, its cards `C<n>.…`,
    /// and its reading page, when the file holds it, `R<n>`.
    pub number: usize,
    /// Its title, from its newer version, when it has one.
    pub title: Option<&'a str>,
    /// Its change cards, without their buttons; `None` when the file left
    /// them out.
    pub cards: Option<&'a str>,
    /// Whether the file holds its reading page.
    pub reading: bool,
}

/// What a range's change page shows.
pub(crate) struct ChangesPage<'a> {
    /// The range as written.
    pub label: &'a str,
    /// The page's heading: the branch of a branch's range, or the range.
    pub heading: &'a str,
    /// The kicker after the range: where it was compared from and to.
    pub compared: &'a str,
    /// The range's documents with their counts, as listed.
    pub list: &'a ChangeList,
    /// Every document of the list, in its order.
    pub documents: &'a [Changed<'a>],
    /// The files that could not be compared.
    pub skipped: &'a [Skipped],
    /// What the file left out, already written, or empty.
    pub cut: &'a str,
}

/// A range's change page: the rail, what needs the reader's eyes, then one
/// section per document with its cards, and the documents the file left
/// out.
pub(crate) fn changes(page: &ChangesPage<'_>) -> String {
    let mut out =
        String::from("<section class=\"xpage\" id=\"Changes\">\n<div class=\"reviewwrap\">\n");
    rail(&mut out, page);
    let _ = write!(
        out,
        "<div class=\"review\">\n<header class=\"page-head\"><p class=\"kicker\">Changes in <code>{}</code> · {}</p><h1>{}</h1></header>\n{}",
        escape(page.label),
        page.compared,
        escape(page.heading),
        page.cut
    );
    if page.list.documents.is_empty() {
        out.push_str("<p class=\"note\">No document changed in this range.</p>\n");
    }
    attention(&mut out, page);
    for changed in page.documents {
        if let Some(cards) = changed.cards {
            section(&mut out, changed, cards);
        }
    }
    let left: Vec<&Changed<'_>> = page
        .documents
        .iter()
        .filter(|changed| changed.cards.is_none())
        .collect();
    if !left.is_empty() {
        let _ = write!(
            out,
            "<section class=\"more\" aria-labelledby=\"More\"><h2 id=\"More\">{} not in this file</h2><ol class=\"more-list\">",
            plural(left.len(), "more document")
        );
        for changed in left {
            let document = changed.document;
            let _ = write!(
                out,
                "<li>{} <code>{}</code><span class=\"more-n\">{}{}</span></li>",
                document.summary.highest.map_or_else(String::new, badge),
                escape(&document.path),
                document_counts(&document.summary),
                file_state(document.file)
            );
        }
        out.push_str("</ol></section>\n");
    }
    if !page.skipped.is_empty() {
        out.push_str("<section class=\"skipped\" aria-labelledby=\"Skipped\"><h2 id=\"Skipped\">Not compared</h2><ul>");
        for file in page.skipped {
            let _ = write!(
                out,
                "<li><code>{}</code>: {}</li>",
                escape(&file.path),
                escape(&file.reason)
            );
        }
        out.push_str("</ul></section>\n");
    }
    out.push_str("</div>\n</div>\n</section>\n");
    out
}

fn file_state(file: FileState) -> &'static str {
    match file {
        FileState::Added => " · new",
        FileState::Removed => " · deleted",
        FileState::Modified => "",
    }
}

/// The rail: the range's counts, then each document the file shows with
/// its changes, up to [`RAIL_CHANGES`] of them.
fn rail(out: &mut String, page: &ChangesPage<'_>) {
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
    let mut listed = 0;
    let mut left = 0;
    for changed in page.documents {
        if changed.cards.is_none() {
            left += 1;
            continue;
        }
        let document = changed.document;
        let n = changed.number;
        let _ = write!(
            out,
            "<li class=\"rail-doc\"><a class=\"rail-doc-name\" href=\"#D{n}\">{}</a> {}<ol class=\"rail-changes\">",
            escape(changed.title.unwrap_or(&document.path)),
            document.summary.highest.map_or_else(String::new, badge)
        );
        let room = RAIL_CHANGES.saturating_sub(listed).min(RAIL_PER_DOCUMENT);
        for (index, change) in document.changes.iter().enumerate().take(room) {
            let _ = write!(
                out,
                "<li data-level=\"{}\"><a href=\"#C{n}.{index}\"><span class=\"dot\" aria-hidden=\"true\"></span><span class=\"rail-label\">{}</span><span class=\"vh\"> ({})</span></a></li>",
                level_name(change.level),
                escape(&rail_label(change)),
                words::level(change.level)
            );
        }
        let shown = document.changes.len().min(room);
        listed += shown;
        let more = document.changes.len() - shown;
        if more > 0 {
            let _ = write!(out, "<li class=\"rail-more\">and {more} more</li>");
        }
        out.push_str("</ol></li>");
    }
    if left > 0 {
        let _ = write!(
            out,
            "<li class=\"rail-doc\"><a class=\"rail-doc-name\" href=\"#More\">{} not in this file</a></li>",
            plural(left, "more document")
        );
    }
    out.push_str("</ol></details></nav>\n");
}

/// The box that says where to look first: the first document of the list,
/// at the range's highest level.
fn attention(out: &mut String, page: &ChangesPage<'_>) {
    let totals = &page.list.totals;
    let (Some(first), Some(level)) = (page.documents.first(), totals.highest) else {
        return;
    };
    let document = first.document;
    let name = escape(first.title.unwrap_or(&document.path));
    let target = if first.cards.is_some() {
        format!("<a href=\"#D{}\">{name}</a>", first.number)
    } else {
        name
    };
    let n = document.summary.levels.get(level);
    let _ = write!(
        out,
        "<section class=\"attention\" aria-labelledby=\"Attention\"><h2 id=\"Attention\" class=\"vh\">What needs your eyes</h2>\
         <p class=\"att-lead\">{} <strong>{}</strong> in {target}. Start there.</p><ul class=\"att-rest\">",
        badge(level),
        plural(n, &format!("{} change", level_name(level)))
    );
    for other in Level::HIGHEST_FIRST {
        let count = totals.levels.get(other);
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

/// One document's section of the change page.
fn section(out: &mut String, changed: &Changed<'_>, cards: &str) {
    let document = changed.document;
    let n = changed.number;
    let name = escape(changed.title.unwrap_or(&document.path));
    let heading = if changed.reading {
        format!("<a href=\"#R{n}\">{name}</a>")
    } else {
        name
    };
    let status = document.status.as_ref().map_or_else(String::new, |status| {
        format!(
            "<span class=\"docchg-status\">status {} → {}</span>",
            escape(status.old.as_deref().unwrap_or("none")),
            escape(status.new.as_deref().unwrap_or("none"))
        )
    });
    let read = if changed.reading {
        format!(" · <a href=\"#R{n}\">read it whole</a>")
    } else {
        String::new()
    };
    let _ = write!(
        out,
        "<section class=\"docchg\" id=\"D{n}\" aria-labelledby=\"D{n}-t\"><header class=\"docchg-head\"><h2 id=\"D{n}-t\">{heading}</h2>\
         <p class=\"docchg-path\"><code>{}</code>{}{read}</p><p class=\"docchg-meta\">{}<span>{}</span>{status}</p></header>\n{cards}</section>\n",
        escape(&document.path),
        file_state(document.file),
        document.summary.highest.map_or_else(String::new, badge),
        document_counts(&document.summary),
    );
}
