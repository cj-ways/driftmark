//! The status page: which documents are current, superseded, archived or
//! untouched for long, by type (`docs/features/status-page.md`).
//!
//! - On top, a table counts each type's documents by lifecycle; every count
//!   is a link that shows those documents.
//! - A form filters by type, lifecycle, folder and "untouched", with no
//!   script: it is a plain `GET` form.
//! - One section per type lists its documents. A superseded document is
//!   folded under the one that replaced it, the archive is folded, and so
//!   are the documents no type matches when there are typed ones.
//! - A page shows at most [`ROWS`] documents; links lead to the next ones.
//!
//! Lifecycles show as words, never by colour alone. "Untouched" is a plain
//! fact beside the date: nothing is sorted or marked as a problem by it.

use std::fmt::Write as _;

use driftmark_core::git::RepoPath;
use driftmark_core::rules::DEFAULT_TYPE;
use driftmark_core::status::{Doc, Filter, Group, Stage, Target, Touched, View};

use super::{capitalized, escape, plural, short, utc};
use crate::serve::address;
use crate::serve::lifecycle::{Params, StatusAt};

/// The most documents one page shows, folded ones included. A row takes
/// about 500 bytes of HTML and 30 after compression, so a full page stays
/// far under the 100 KB first-visit limit (`docs/features/pages.md`).
pub(crate) const ROWS: usize = 1_000;

/// What the status page shows.
pub(crate) struct StatusPage<'a> {
    /// The revision as the address wrote it.
    pub revision: &'a str,
    /// The view and the commit it was read at.
    pub status: &'a StatusAt,
    /// The filter and the page.
    pub params: &'a Params,
}

/// A stage's word on the page.
fn stage_word(stage: Stage) -> &'static str {
    match stage {
        Stage::Current => "Current",
        Stage::Superseded => "Superseded",
        Stage::Archived => "Archived",
        Stage::None => "No lifecycle",
    }
}

/// The address of this revision's status page with `filter` and `offset`.
fn address_of(revision: &str, filter: &Filter, offset: usize) -> String {
    let mut parts = Vec::new();
    let encode = |text: &str| {
        percent_encoding::utf8_percent_encode(text, percent_encoding::NON_ALPHANUMERIC).to_string()
    };
    for doc_type in &filter.types {
        parts.push(format!("type={}", encode(doc_type)));
    }
    if let Some(stage) = filter.stage {
        parts.push(format!("lifecycle={}", stage.name()));
    }
    if let Some(folder) = &filter.folder {
        parts.push(format!("folder={}", encode(folder)));
    }
    if filter.untouched {
        parts.push("untouched=yes".to_owned());
    }
    if offset > 0 {
        parts.push(format!("offset={offset}"));
    }
    let base = address::lifecycle(revision);
    if parts.is_empty() {
        base
    } else {
        format!("{base}?{}", parts.join("&"))
    }
}

/// The status page's content.
pub(crate) fn render(page: &StatusPage<'_>) -> String {
    let view = &page.status.view;
    let filter = &page.params.filter;
    let mut out =
        String::from("<main id=\"Main\" class=\"lifecycle\">\n<header class=\"page-head\">");
    let at = match &page.status.commit {
        Some(commit) => format!(
            "<code>{}</code> at commit <code>{}</code>",
            escape(page.revision),
            short(commit.as_hex())
        ),
        None => "The working tree, as it is on disk now".to_owned(),
    };
    let _ = writeln!(
        out,
        "<p class=\"kicker\">{at} · {} · <a href=\"{}\">JSON</a></p><h1>Which documents are current</h1>\
         <p class=\"lc-lede\">Each type's documents by lifecycle. \"Untouched\" means no commit changed a document for {} or more; it is a fact, not a fault.</p></header>",
        plural(view.totals.documents, "document"),
        escape(&address::json(&address_of(page.revision, filter, 0))),
        plural(view.untouched_days as usize, "day")
    );
    counts_table(&mut out, page);
    filter_form(&mut out, page);
    listing(&mut out, page);
    if !page.status.skipped.is_empty() {
        let _ = write!(
            out,
            "<details class=\"lc-skipped\"><summary>{} could not be read</summary><ul>",
            plural(page.status.skipped.len(), "file")
        );
        for unread in &page.status.skipped {
            let _ = write!(
                out,
                "<li><code>{}</code>: {}</li>",
                escape(&unread.path),
                escape(&unread.reason)
            );
        }
        out.push_str("</ul></details>\n");
    }
    out.push_str("</main>\n");
    out
}

/// The counts per type and lifecycle, each a link to those documents.
fn counts_table(out: &mut String, page: &StatusPage<'_>) {
    let view = &page.status.view;
    let untouched = format!("Untouched {}+ days", view.untouched_days);
    let _ = write!(
        out,
        "<div class=\"lc-table\"><table><caption class=\"vh\">Documents by type and lifecycle</caption><thead><tr><th scope=\"col\">Type</th>"
    );
    for stage in Stage::ALL {
        let _ = write!(out, "<th scope=\"col\">{}</th>", stage_word(stage));
    }
    let _ = write!(
        out,
        "<th scope=\"col\">{untouched}</th><th scope=\"col\">All</th></tr></thead><tbody>"
    );
    let cell = |out: &mut String, label: &str, count: usize, filter: Filter| {
        let shown = if count == 0 {
            "0".to_owned()
        } else {
            format!(
                "<a href=\"{}\">{count}</a>",
                escape(&address_of(page.revision, &filter, 0))
            )
        };
        let _ = write!(out, "<td data-label=\"{}\">{shown}</td>", escape(label));
    };
    let rows = view
        .types
        .iter()
        .map(|counts| (Some(counts.name.as_str()), counts.counts))
        .chain([(None, view.totals)]);
    for (doc_type, counts) in rows {
        let types: Vec<String> = doc_type.map(str::to_owned).into_iter().collect();
        let name = doc_type.map_or_else(|| "Every type".to_owned(), type_word);
        let _ = write!(out, "<tr><th scope=\"row\">{}</th>", escape(&name));
        for stage in Stage::ALL {
            let filter = Filter {
                types: types.clone(),
                stage: Some(stage),
                ..Filter::default()
            };
            cell(out, stage_word(stage), counts.of(stage), filter);
        }
        let filter = Filter {
            types: types.clone(),
            untouched: true,
            ..Filter::default()
        };
        cell(out, &untouched, counts.untouched, filter);
        let filter = Filter {
            types,
            ..Filter::default()
        };
        cell(out, "All", counts.documents, filter);
        out.push_str("</tr>");
    }
    out.push_str("</tbody></table></div>\n");
}

/// A type's name as the page shows it: `document` is the documents no type
/// matches.
fn type_word(name: &str) -> String {
    if name == DEFAULT_TYPE {
        "No type".to_owned()
    } else {
        capitalized(name)
    }
}

/// The filters, as a form that works without a script.
fn filter_form(out: &mut String, page: &StatusPage<'_>) {
    let view = &page.status.view;
    let filter = &page.params.filter;
    let _ = write!(
        out,
        "<form class=\"lc-filters\" method=\"get\" action=\"{}\" aria-label=\"Filters\">\
         <label>Type <select name=\"type\"><option value=\"\">Every type</option>",
        escape(&address::lifecycle(page.revision))
    );
    for counts in &view.types {
        let selected = if filter.types.contains(&counts.name) {
            " selected"
        } else {
            ""
        };
        let _ = write!(
            out,
            "<option value=\"{}\"{selected}>{}</option>",
            escape(&counts.name),
            escape(&type_word(&counts.name))
        );
    }
    out.push_str("</select></label><label>Lifecycle <select name=\"lifecycle\"><option value=\"\">Any</option>");
    for stage in Stage::ALL {
        let selected = if filter.stage == Some(stage) {
            " selected"
        } else {
            ""
        };
        let _ = write!(
            out,
            "<option value=\"{}\"{selected}>{}</option>",
            stage.name(),
            stage_word(stage)
        );
    }
    let _ = writeln!(
        out,
        "</select></label><label>Folder <input name=\"folder\" value=\"{}\" placeholder=\"docs/decisions/\" spellcheck=\"false\" autocomplete=\"off\"></label>\
         <label class=\"lc-check\"><input type=\"checkbox\" name=\"untouched\" value=\"yes\"{}> Untouched {}+ days</label>\
         <button type=\"submit\">Show</button> <a href=\"{}\">Clear</a></form>",
        escape(filter.folder.as_deref().unwrap_or_default()),
        if filter.untouched { " checked" } else { "" },
        view.untouched_days,
        escape(&address::lifecycle(page.revision))
    );
}

/// How many documents a row shows: itself and every document folded under
/// it.
fn weight(view: &View, index: usize) -> usize {
    1 + view.documents[index]
        .replaces
        .iter()
        .map(|&older| weight(view, older))
        .sum::<usize>()
}

/// How much of the listing a page has used: the rows skipped for its
/// `offset`, and the rows and documents shown.
struct Budget {
    /// Rows still to skip.
    skip: usize,
    /// Rows shown.
    rows: usize,
    /// Documents shown, folded ones included.
    documents: usize,
    /// Whether a row did not fit, so the page shows no more.
    full: bool,
}

impl Budget {
    /// Whether the next row, which shows `weight` documents, goes on the
    /// page. A row is skipped while the offset lasts; once one does not
    /// fit, no later one is shown.
    fn take(&mut self, weight: usize) -> Option<bool> {
        if self.skip > 0 {
            self.skip -= 1;
            return Some(false);
        }
        if self.full || (self.documents > 0 && self.documents + weight > ROWS) {
            self.full = true;
            return None;
        }
        self.rows += 1;
        self.documents += weight;
        Some(true)
    }
}

/// The groups of the page, from `offset` on, up to [`ROWS`] documents.
fn listing(out: &mut String, page: &StatusPage<'_>) {
    let view = &page.status.view;
    let filter = &page.params.filter;
    let groups = view.groups(filter);
    if groups.is_empty() {
        out.push_str("<p class=\"note\">No document matches these filters.</p>\n");
        return;
    }
    let all_rows: usize = groups.iter().map(|group| group.rows.len()).sum();
    let mut budget = Budget {
        skip: page.params.offset,
        rows: 0,
        documents: 0,
        full: false,
    };
    let typed = view.types.iter().any(|counts| counts.name != DEFAULT_TYPE);
    let mut body = String::new();
    for group in &groups {
        group_section(&mut body, page, group, &mut budget, typed);
    }
    let _ = writeln!(
        out,
        "<p class=\"lc-shown\">{} of the {} these filters list, folded ones included.</p>",
        if page.params.offset == 0 && budget.rows == all_rows {
            "All".to_owned()
        } else {
            format!("Showing {}", budget.documents)
        },
        plural(view.listed(filter).len().max(budget.documents), "document")
    );
    out.push_str(&body);
    let next = page.params.offset + budget.rows;
    if page.params.offset > 0 || next < all_rows {
        out.push_str("<p class=\"lc-more\">");
        if page.params.offset > 0 {
            let _ = write!(
                out,
                "<a href=\"{}\">← Back to the first documents</a> ",
                escape(&address_of(page.revision, filter, 0))
            );
        }
        if next < all_rows {
            let _ = write!(
                out,
                "<a href=\"{}\">Show the next documents →</a>",
                escape(&address_of(page.revision, filter, next))
            );
        }
        out.push_str("</p>\n");
    }
}

/// One type's section: its rows that fit the page, the archived ones
/// folded, and the whole section folded when it holds the documents no type
/// matches and typed documents are listed too.
fn group_section(
    out: &mut String,
    page: &StatusPage<'_>,
    group: &Group,
    budget: &mut Budget,
    typed: bool,
) {
    let view = &page.status.view;
    let filter = &page.params.filter;
    let (archived, active): (Vec<usize>, Vec<usize>) = group
        .rows
        .iter()
        .partition(|&&index| Stage::of(view.documents[index].lifecycle) == Stage::Archived);
    let mut parts = String::new();
    for (list, archive) in [(&active, false), (&archived, true)] {
        let mut items = String::new();
        let mut count = 0;
        for &index in list {
            match budget.take(weight(view, index)) {
                None => break,
                Some(false) => {}
                Some(true) => {
                    row(&mut items, page, index);
                    count += 1;
                }
            }
        }
        if count == 0 {
            continue;
        }
        if archive {
            let open = if filter.stage == Some(Stage::Archived) {
                " open"
            } else {
                ""
            };
            let _ = write!(
                parts,
                "<details class=\"lc-archive\"{open}><summary>Archived · {count}</summary><ol class=\"lc-list\">{items}</ol></details>"
            );
        } else {
            let _ = write!(parts, "<ol class=\"lc-list\">{items}</ol>");
        }
    }
    if parts.is_empty() {
        return;
    }
    let id = format!(
        "T{:08x}",
        crate::serve::fingerprint(group.name.as_bytes()) >> 32
    );
    let count = view
        .types
        .iter()
        .find(|counts| counts.name == group.name)
        .map_or(0, |counts| counts.counts.documents);
    let matching = (0..view.documents.len())
        .filter(|&index| {
            view.documents[index].doc_type == group.name && view.matches(index, filter)
        })
        .count();
    let shown = if matching == count {
        plural(count, "document")
    } else {
        format!("{matching} of {}", plural(count, "document"))
    };
    let _ = write!(
        out,
        "<section class=\"lc-group\" aria-labelledby=\"{id}\"><h2 id=\"{id}\" class=\"group-head\">{} <span class=\"group-n\">{shown}</span></h2>",
        escape(&type_word(&group.name)),
    );
    if typed && group.name == DEFAULT_TYPE && filter.types.is_empty() {
        let _ = write!(
            out,
            "<details class=\"lc-plain\"><summary>Documents no type of the rules file matches</summary>{parts}</details>"
        );
    } else {
        out.push_str(&parts);
    }
    out.push_str("</section>\n");
}

/// One document, with the documents it replaced folded under it.
fn row(out: &mut String, page: &StatusPage<'_>, index: usize) {
    let view = &page.status.view;
    let doc = &view.documents[index];
    let stage = Stage::of(doc.lifecycle);
    let file = &doc.path[doc.folder().len()..];
    let title = doc.title.as_deref().unwrap_or(file);
    // A document with no lifecycle carries no word: on a repository whose
    // rules name no lifecycles, every row would repeat it.
    let word = if stage == Stage::None {
        String::new()
    } else {
        format!(
            " <span class=\"lc-word\" data-lifecycle=\"{}\">{}</span>",
            stage.name(),
            stage_word(stage)
        )
    };
    let _ = write!(
        out,
        "<li class=\"lc-doc\" data-lifecycle=\"{}\"><p class=\"lc-top\"><a class=\"lc-title\" href=\"{}\">{}</a>{word}</p><p class=\"lc-meta\"><code>",
        stage.name(),
        escape(&reading(page.revision, &doc.path)),
        escape(title),
    );
    if !doc.folder().is_empty() {
        let folder = Filter {
            folder: Some(doc.folder().to_owned()),
            ..page.params.filter.clone()
        };
        let _ = write!(
            out,
            "<a href=\"{}\" title=\"Only this folder\">{}</a>",
            escape(&address_of(page.revision, &folder, 0)),
            escape(doc.folder())
        );
    }
    let _ = write!(out, "{}</code>", escape(file));
    if let Some(status) = &doc.status {
        let _ = write!(out, " · Status: {}", escape(status));
    }
    if let Some(date) = &doc.date {
        let _ = write!(out, " · Dated {}", escape(date));
    }
    let _ = write!(out, " · {}", changed(doc));
    out.push_str("</p>");
    successor_line(out, page, doc);
    if !doc.replaces.is_empty() {
        let _ = write!(
            out,
            "<details class=\"lc-replaced\"><summary>Replaced {}</summary><ol class=\"lc-list\">",
            plural(doc.replaces.len(), "document")
        );
        for &older in &doc.replaces {
            row(out, page, older);
        }
        out.push_str("</ol></details>");
    }
    out.push_str("</li>");
}

/// When a document last changed, in words, with the exact time as its
/// title.
fn changed(doc: &Doc) -> String {
    match (&doc.touched, doc.days) {
        (Touched::Committed { at, .. }, Some(days)) => {
            let ago = match days {
                0 => "today".to_owned(),
                1 => "yesterday".to_owned(),
                _ => format!("{days} days ago"),
            };
            let old = if doc.untouched {
                " <span class=\"lc-old\">untouched</span>"
            } else {
                ""
            };
            format!(
                "last changed <time datetime=\"{}\" title=\"{}\">{ago}</time>{old}",
                &utc(*at)[..10],
                utc(*at)
            )
        }
        _ => "not committed yet".to_owned(),
    }
}

/// The address of a document's reading page at the page's revision.
fn reading(revision: &str, path: &str) -> String {
    RepoPath::new(path).map_or_else(String::new, |path| address::document(revision, &path))
}

/// What a document says replaced it: a link, or why there is none.
fn successor_line(out: &mut String, page: &StatusPage<'_>, doc: &Doc) {
    let Some(successor) = &doc.successor else {
        return;
    };
    let view = &page.status.view;
    let link = |at: usize| {
        let newer = &view.documents[at];
        format!(
            "<a href=\"{}\">{}</a>",
            escape(&reading(page.revision, &newer.path)),
            escape(newer.title.as_deref().unwrap_or(&newer.path))
        )
    };
    let named = successor
        .text
        .as_deref()
        .map_or_else(String::new, |text| format!(" “{}”", escape(text)));
    let said = match &successor.target {
        Target::Found(at) => format!("Superseded by {}", link(*at)),
        Target::Circle(at) => format!(
            "Superseded by {}, but following “superseded by” from there leads back here",
            link(*at)
        ),
        Target::Missing(path) => format!(
            "Superseded by{named}: <code>{}</code> is not a document at this revision",
            escape(path)
        ),
        Target::Unmatched => format!(
            "Superseded by{named}: no document at this revision has that path, title or number"
        ),
        Target::Unnamed => "Superseded, but the document does not say by what".to_owned(),
    };
    let _ = write!(out, "<p class=\"lc-succ\">{said}</p>");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_keep_the_filter() {
        let filter = Filter {
            types: vec!["adr".into(), "a b".into()],
            stage: Some(Stage::Superseded),
            folder: Some("docs/x/".into()),
            untouched: true,
        };
        assert_eq!(
            address_of("main", &filter, 20),
            "/lifecycle/main?type=adr&type=a%20b&lifecycle=superseded&folder=docs%2Fx%2F&untouched=yes&offset=20"
        );
        assert_eq!(
            address_of("worktree", &Filter::default(), 0),
            "/lifecycle/worktree"
        );
    }

    #[test]
    fn words_name_every_stage_and_the_untyped() {
        let words: Vec<&str> = Stage::ALL.into_iter().map(stage_word).collect();
        assert_eq!(words, ["Current", "Superseded", "Archived", "No lifecycle"]);
        assert_eq!(type_word("document"), "No type");
        assert_eq!(type_word("adr"), "Adr");
    }
}
