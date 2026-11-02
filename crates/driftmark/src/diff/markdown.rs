//! The short Markdown comment `driftmark diff --format markdown-summary`
//! prints for a merge request: the counts per level, the ten highest-ranked
//! changes, and where the full rendered changes are.
//!
//! A CI job posts it as one comment and edits that comment on every later
//! push, so it starts with a hidden marker the job can find again
//! (`docs/features/ci.md`).
//!
//! Every text from the repository (paths, headings, the changed text) goes
//! inside a code span. GitHub and GitLab turn `@name` into a mention that
//! notifies someone, `#12` into a link to an issue, and raw HTML into
//! markup, but neither looks inside code. A document that quotes a team's
//! handle must not page that team because an agent edited the sentence.

use std::cmp::Reverse;
use std::fmt::Write as _;

use driftmark_core::diff::{Change, ChangeKind, ChangeList, DocumentChanges, Level, Subject};
use driftmark_core::git::Roots;

use super::text::{escape, level_name, number, plural, short};

/// The first line of every comment. The CI job finds its own comment by it,
/// so it never changes. Markdown hides an HTML comment.
pub(crate) const MARKER: &str = "<!-- driftmark:changes -->";

/// How many changes the comment lists, highest ranked first.
pub(crate) const TOP: usize = 10;

/// The most characters of changed text a listed change shows.
const EXCERPT: usize = 120;

/// The most characters of heading path a listed change shows.
const HEADINGS: usize = 80;

/// The most characters of a path the comment shows; a longer path loses its
/// first folders, as in the text summary.
const PATH: usize = 100;

/// The most folders the last line names; it counts the others.
const FOLDERS: usize = 5;

/// What the comment says beyond the change list.
pub(crate) struct Context<'a> {
    /// The range as written, for the command a reader can run.
    pub label: &'a str,
    /// The folders read; empty for the whole repository.
    pub roots: &'a Roots,
    /// How many changed files could not be compared.
    pub skipped: usize,
    /// Where the full rendered changes are, from `--link`.
    pub link: Option<&'a str>,
}

/// The comment for a change list.
pub(crate) fn summary(list: &ChangeList, context: &Context<'_>) -> String {
    let mut out = format!("{MARKER}\n### Documentation changes\n\n");
    let totals = &list.totals;
    if list.documents.is_empty() {
        out.push_str("No Markdown document changed.\n");
    } else {
        let levels: Vec<String> = Level::HIGHEST_FIRST
            .iter()
            .filter(|&&level| totals.levels.get(level) > 0)
            .map(|&level| format!("{} {}", number(totals.levels.get(level)), level_name(level)))
            .collect();
        let _ = write!(
            out,
            "**{} changed, {}**",
            plural(list.documents.len(), "document", "documents"),
            plural(totals.changes, "change", "changes"),
        );
        if levels.is_empty() {
            out.push_str(": blank lines only.\n");
        } else {
            let _ = writeln!(out, ": {}.", levels.join(", "));
        }
    }
    if context.skipped > 0 {
        let _ = writeln!(
            out,
            "\n{} could not be compared; the job's log names {}.",
            plural(context.skipped, "file", "files"),
            if context.skipped == 1 { "it" } else { "them" }
        );
    }
    let ranked = ranked(list);
    if !ranked.is_empty() {
        out.push_str("\nHighest ranked first:\n\n");
        for (index, (document, change)) in ranked.iter().take(TOP).enumerate() {
            let _ = writeln!(out, "{}. {}", index + 1, line(document, change));
        }
        if ranked.len() > TOP {
            let more = ranked.len() - TOP;
            let what = if more == 1 { "change" } else { "changes" };
            let _ = writeln!(out, "\n…and {} more {what}.", number(more));
        }
    }
    if !list.documents.is_empty() {
        out.push('\n');
        match context.link {
            Some(link) => {
                let _ = writeln!(
                    out,
                    "**[Every change, rendered](<{link}>)**: one HTML file that opens offline in any browser, with each changed document in full."
                );
            }
            None => {
                let _ = writeln!(
                    out,
                    "Every change, rendered: run {} for one HTML file that opens offline.",
                    code(&format!("driftmark export changes {}", context.label))
                );
            }
        }
    }
    out.push('\n');
    out.push_str(&footer(list, context.roots));
    out
}

/// Every change of the list with its document, highest level first. Within
/// a level the documents keep the change list's order (most changes first,
/// then by path), and a document's changes keep their reading order, so the
/// same list always ranks the same way.
fn ranked(list: &ChangeList) -> Vec<(&DocumentChanges, &Change)> {
    let mut all: Vec<(&DocumentChanges, &Change)> = list
        .documents
        .iter()
        .flat_map(|document| {
            document
                .changes
                .iter()
                .map(move |change| (document, change))
        })
        .collect();
    // A stable sort keeps the order above within each level.
    all.sort_by_key(|(_, change)| Reverse(change.level));
    all
}

/// One listed change: `**level**, kind · document › heading: text`.
fn line(document: &DocumentChanges, change: &Change) -> String {
    let path = code(&super::text::shorten(&escape(&document.path), PATH));
    let mut out = format!(
        "**{}**, {} · {path}",
        level_name(change.level),
        kind_words(change)
    );
    let (headings, text) = match &change.subject {
        Subject::Document { document } => (
            None,
            document
                .title
                .as_deref()
                .or(document.summary.as_deref())
                .unwrap_or_default(),
        ),
        Subject::Block { old, new } => match new.block.as_ref().or(old.block.as_ref()) {
            Some(block) => (Some(&block.heading_path[..]), block.text.as_str()),
            None => (None, ""),
        },
    };
    if let Some(headings) = headings.filter(|headings| !headings.is_empty()) {
        let _ = write!(out, " › {}", code(&heading_path(headings)));
    }
    let text = one_line(text);
    if !text.is_empty() {
        let _ = write!(out, ": {}", code(&cut(&text, EXCERPT)));
    }
    out
}

/// What happened, in words.
fn kind_words(change: &Change) -> &'static str {
    match (&change.subject, change.kind) {
        (Subject::Document { .. }, ChangeKind::Removed) => "deleted document",
        (Subject::Document { .. }, _) => "new document",
        (_, ChangeKind::Formatting) => "formatting only",
        (_, ChangeKind::Edited) => "edited",
        (_, ChangeKind::Added) => "added",
        (_, ChangeKind::Removed) => "removed",
        (_, ChangeKind::Moved) => "moved",
        (_, ChangeKind::MovedEdited) => "moved and edited",
    }
}

/// The headings above a block, outermost first, joined by ` › `. When they
/// do not fit in [`HEADINGS`] characters the outer ones go first, written
/// `…`, because the nearest heading says most about the change.
fn heading_path(headings: &[String]) -> String {
    let headings: Vec<String> = headings.iter().map(|heading| one_line(heading)).collect();
    let all = headings.join(" › ");
    if all.chars().count() <= HEADINGS {
        return all;
    }
    let mut kept: Vec<&str> = Vec::new();
    let mut length = "…".chars().count();
    for heading in headings.iter().rev() {
        let more = " › ".chars().count() + heading.chars().count();
        if length + more > HEADINGS {
            break;
        }
        length += more;
        kept.push(heading);
    }
    if kept.is_empty() {
        let last = headings.last().map(String::as_str).unwrap_or_default();
        return cut(last, HEADINGS);
    }
    kept.reverse();
    format!("… › {}", kept.join(" › "))
}

/// Text on one line: every run of white space turned into one space and
/// the ends trimmed, then the control characters left escaped as in the
/// text summary.
fn one_line(text: &str) -> String {
    escape(&text.split_whitespace().collect::<Vec<&str>>().join(" "))
}

/// At most `max` characters: a longer text keeps its first `max - 1` and
/// ends with `…`.
fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let kept: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", kept.trim_end())
}

/// `text` as a Markdown code span: between backtick runs one longer than
/// any run inside it, with a space inside each end when the text starts or
/// ends with a backtick, or with spaces at both ends, as `CommonMark` asks.
/// Inside it, nothing is a mention, a reference, a link or HTML.
fn code(text: &str) -> String {
    let mut longest = 0;
    let mut run = 0;
    for c in text.chars() {
        run = if c == '`' { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    let fence = "`".repeat(longest + 1);
    let pad = text.starts_with('`')
        || text.ends_with('`')
        || (text.starts_with(' ') && text.ends_with(' ') && !text.trim().is_empty());
    let space = if pad { " " } else { "" };
    format!("{fence}{space}{text}{space}{fence}")
}

/// The last line, in small print: what was compared, and the folders read.
fn footer(list: &ChangeList, roots: &Roots) -> String {
    let mut out = format!("<sub>driftmark {}", driftmark_core::version());
    if let Some(range) = &list.range {
        let base = range
            .base
            .as_deref()
            .map_or_else(|| "the empty history".to_owned(), |id| code(&short(id)));
        let head = match range.head.as_str() {
            "worktree" => "the working tree".to_owned(),
            id => code(&short(id)),
        };
        let _ = write!(out, " compared {base} with {head}");
    }
    let all = roots.folders();
    if !all.is_empty() {
        let mut folders: Vec<String> = all
            .iter()
            .take(FOLDERS)
            .map(|folder| code(&cut(&one_line(&folder.to_lossy()), PATH)))
            .collect();
        if all.len() > FOLDERS {
            let more = all.len() - FOLDERS;
            let what = if more == 1 { "folder" } else { "folders" };
            folders.push(format!("and {} more {what}", number(more)));
        }
        let _ = write!(out, ", only in {}", folders.join(", "));
    }
    out.push_str(".</sub>\n");
    out
}

#[cfg(test)]
mod tests {
    use driftmark_core::diff::{BlockRef, FileState, RangeInfo, Side, Summary, WholeDocument};
    use driftmark_core::document::BlockKind;

    use super::*;

    fn block(text: &str, headings: &[&str]) -> BlockRef {
        BlockRef {
            kind: BlockKind::Paragraph,
            start: 0,
            end: text.len(),
            line: 1,
            heading_path: headings
                .iter()
                .map(|h| (*h).to_owned())
                .collect::<Vec<_>>()
                .into(),
            text: text.to_owned(),
        }
    }

    fn edited(level: Level, text: &str, headings: &[&str]) -> Change {
        Change {
            kind: ChangeKind::Edited,
            level,
            signals: Vec::new(),
            subject: Subject::Block {
                old: Side {
                    at: 0,
                    block: Some(block("old words", headings)),
                },
                new: Side {
                    at: 0,
                    block: Some(block(text, headings)),
                },
            },
            edit: None,
        }
    }

    fn removed(level: Level, text: &str) -> Change {
        Change {
            kind: ChangeKind::Removed,
            level,
            signals: Vec::new(),
            subject: Subject::Block {
                old: Side {
                    at: 2,
                    block: Some(block(text, &["Notes"])),
                },
                new: Side { at: 2, block: None },
            },
            edit: None,
        }
    }

    fn whole(kind: ChangeKind, level: Level, title: Option<&str>) -> Change {
        Change {
            kind,
            level,
            signals: Vec::new(),
            subject: Subject::Document {
                document: WholeDocument {
                    title: title.map(str::to_owned),
                    summary: Some("A summary.".to_owned()),
                    bytes: 10,
                    lines: 1,
                    source: String::new(),
                },
            },
            edit: None,
        }
    }

    fn document(path: &str, file: FileState, changes: Vec<Change>) -> DocumentChanges {
        let mut summary = Summary::default();
        for change in &changes {
            summary.changes += 1;
            summary.highest = summary.highest.max(Some(change.level));
            match change.level {
                Level::Decision => summary.levels.decision += 1,
                Level::Rule => summary.levels.rule += 1,
                Level::Fact => summary.levels.fact += 1,
                Level::Wording => summary.levels.wording += 1,
                Level::Formatting => summary.levels.formatting += 1,
            }
        }
        DocumentChanges {
            path: path.to_owned(),
            file,
            status: None,
            summary,
            changes,
            unchanged: Vec::new(),
        }
    }

    fn list(documents: Vec<DocumentChanges>) -> ChangeList {
        ChangeList::new(
            Some(RangeInfo {
                base: Some("3828db3aa1840000000000000000000000000000".to_owned()),
                head: "2cc1179a6f870000000000000000000000000000".to_owned(),
                notes: Vec::new(),
            }),
            documents,
        )
    }

    fn context<'a>(roots: &'a Roots, link: Option<&'a str>) -> Context<'a> {
        Context {
            label: "main...feature",
            roots,
            skipped: 0,
            link,
        }
    }

    #[test]
    fn the_comment_lists_counts_then_changes_by_rank() {
        let list = list(vec![
            document(
                "docs/guide.md",
                FileState::Modified,
                vec![edited(
                    Level::Wording,
                    "Run the  build\nfirst.",
                    &["Guide", "Build"],
                )],
            ),
            document(
                "docs/decisions/0002-cache.md",
                FileState::Modified,
                vec![
                    edited(Level::Fact, "It holds 500 pages.", &[]),
                    edited(
                        Level::Decision,
                        "Keep each rendered page in the cache for 90 minutes.",
                        &["Cache rendered pages", "Decision"],
                    ),
                    removed(Level::Wording, "The cache lives in memory."),
                ],
            ),
            document(
                "docs/new.md",
                FileState::Added,
                vec![whole(ChangeKind::Added, Level::Fact, Some("Cache sizes"))],
            ),
            document(
                "docs/old.md",
                FileState::Removed,
                vec![whole(ChangeKind::Removed, Level::Fact, None)],
            ),
        ]);
        let roots = Roots::new(["docs"]).unwrap();
        assert_eq!(
            summary(
                &list,
                &context(&roots, Some("https://example.com/runs/7/artifacts/9"))
            ),
            format!(
                "\
<!-- driftmark:changes -->
### Documentation changes

**4 documents changed, 6 changes**: 1 decision, 3 fact, 2 wording.

Highest ranked first:

1. **decision**, edited · `docs/decisions/0002-cache.md` › `Cache rendered pages › Decision`: `Keep each rendered page in the cache for 90 minutes.`
2. **fact**, edited · `docs/decisions/0002-cache.md`: `It holds 500 pages.`
3. **fact**, new document · `docs/new.md`: `Cache sizes`
4. **fact**, deleted document · `docs/old.md`: `A summary.`
5. **wording**, removed · `docs/decisions/0002-cache.md` › `Notes`: `The cache lives in memory.`
6. **wording**, edited · `docs/guide.md` › `Guide › Build`: `Run the build first.`

**[Every change, rendered](<https://example.com/runs/7/artifacts/9>)**: one HTML file that opens offline in any browser, with each changed document in full.

<sub>driftmark {} compared `3828db3aa184` with `2cc1179a6f87`, only in `docs`.</sub>
",
                driftmark_core::version()
            )
        );
    }

    #[test]
    fn only_the_top_ten_are_listed_and_the_rest_are_counted() {
        let changes: Vec<Change> = (0..25)
            .map(|n| {
                let level = if n == 24 {
                    Level::Decision
                } else {
                    Level::Wording
                };
                edited(level, &format!("change {n}"), &[])
            })
            .collect();
        let list = list(vec![document("a.md", FileState::Modified, changes)]);
        let roots = Roots::all();
        let text = summary(&list, &context(&roots, None));
        let items: Vec<&str> = text.lines().filter(|line| line.contains(" · ")).collect();
        assert_eq!(items.len(), TOP);
        assert_eq!(
            items[0], "1. **decision**, edited · `a.md`: `change 24`",
            "the highest level comes first wherever it is"
        );
        assert_eq!(items[1], "2. **wording**, edited · `a.md`: `change 0`");
        assert_eq!(items[9], "10. **wording**, edited · `a.md`: `change 8`");
        assert!(text.contains("\n…and 15 more changes.\n"), "{text}");
        assert!(
            text.contains(
                "Every change, rendered: run `driftmark export changes main...feature` for one HTML file that opens offline.\n"
            ),
            "{text}"
        );
    }

    #[test]
    fn nothing_changed_still_carries_the_marker() {
        let empty = list(Vec::new());
        let roots = Roots::all();
        let text = summary(&empty, &context(&roots, Some("https://example.com/x")));
        assert!(text.starts_with(MARKER), "{text}");
        assert!(text.contains("\nNo Markdown document changed.\n"), "{text}");
        assert!(
            !text.contains("Every change"),
            "no link without changes: {text}"
        );
        let mut skipped = context(&roots, None);
        skipped.skipped = 2;
        assert!(
            summary(&empty, &skipped)
                .contains("\n2 files could not be compared; the job's log names them.\n")
        );
        skipped.skipped = 1;
        assert!(
            summary(&empty, &skipped)
                .contains("1 file could not be compared; the job's log names it.")
        );
        let no_range = ChangeList::new(None, Vec::new());
        assert!(
            summary(&no_range, &context(&roots, None)).ends_with(&format!(
                "<sub>driftmark {}.</sub>\n",
                driftmark_core::version()
            ))
        );
    }

    #[test]
    fn blank_line_changes_have_no_levels_to_count() {
        let list = list(vec![document("a.md", FileState::Modified, Vec::new())]);
        let roots = Roots::all();
        let text = summary(&list, &context(&roots, None));
        assert!(
            text.contains("**1 document changed, 0 changes**: blank lines only.\n"),
            "{text}"
        );
        assert!(!text.contains("Highest ranked"), "{text}");
    }

    #[test]
    fn repository_text_cannot_mention_link_or_inject_markup() {
        let hostile = "@team please see #12 <img src=x onerror=alert(1)> [x](javascript:alert(1)) **bold** ``tick`` |";
        let list = list(vec![document(
            "docs/@team/#12.md",
            FileState::Modified,
            vec![edited(Level::Rule, hostile, &["`Heading`", "@owners"])],
        )]);
        let roots = Roots::all();
        let text = summary(&list, &context(&roots, None));
        let item = text.lines().find(|line| line.starts_with("1. ")).unwrap();
        assert_eq!(
            item,
            "1. **rule**, edited · `docs/@team/#12.md` › `` `Heading` › @owners ``: ```@team please see #12 <img src=x onerror=alert(1)> [x](javascript:alert(1)) **bold** ``tick`` |```"
        );
    }

    #[test]
    fn code_spans_survive_any_text() {
        assert_eq!(code("plain"), "`plain`");
        assert_eq!(code("a`b"), "``a`b``");
        assert_eq!(code("`a`"), "`` `a` ``");
        assert_eq!(code("a ``` b"), "````a ``` b````");
        assert_eq!(code(" both "), "`  both  `");
        assert_eq!(code(" left"), "` left`");
        assert_eq!(code("  "), "`  `");
        assert_eq!(code(""), "``");
    }

    #[test]
    fn long_text_is_cut_and_control_characters_escaped() {
        assert_eq!(cut("abcdef", 6), "abcdef");
        assert_eq!(cut("abcdefg", 6), "abcde…");
        assert_eq!(cut("abcd  fg", 6), "abcd…", "no space before the ellipsis");
        assert_eq!(cut("ქართული ტექსტი", 8), "ქართული…");
        assert_eq!(one_line("a\n\tb\u{1b}[2J  c\r\n"), "a b\\u{1b}[2J c");
        assert_eq!(one_line("  \n "), "");
    }

    #[test]
    fn heading_paths_keep_the_nearest_headings() {
        let short = ["Guide".to_owned(), "Build".to_owned()];
        assert_eq!(heading_path(&short), "Guide › Build");
        let long: Vec<String> = [
            "A long outer heading about the whole plan",
            "Middle part",
            "Decision",
        ]
        .iter()
        .map(|h| (*h).to_owned())
        .collect();
        let all = long.join(" › ");
        assert!(all.chars().count() <= HEADINGS, "{}", all.chars().count());
        assert_eq!(heading_path(&long), all);
        let mut longer = long.clone();
        longer.insert(0, "x".repeat(40));
        assert_eq!(
            heading_path(&longer),
            "… › A long outer heading about the whole plan › Middle part › Decision"
        );
        let exact = vec!["z".repeat(50), "y".repeat(65), "Decision".to_owned()];
        let kept = heading_path(&exact);
        assert_eq!(kept, format!("… › {} › Decision", "y".repeat(65)));
        assert_eq!(kept.chars().count(), HEADINGS, "exactly the limit fits");
        let huge = vec!["y".repeat(200)];
        assert_eq!(
            heading_path(&huge),
            format!("{}…", "y".repeat(HEADINGS - 1))
        );
    }

    #[test]
    fn the_worst_case_stays_far_under_both_hosts_limits() {
        // GitHub refuses a comment body over 65,536 characters; GitLab takes
        // up to 1,000,000. Backticks are the worst text: each code span
        // around them needs a fence longer than the longest run inside.
        let tick_path = format!("{}.md", "`".repeat(300));
        let headings: Vec<String> = (0..50).map(|_| "`".repeat(400)).collect();
        let heading_refs: Vec<&str> = headings.iter().map(String::as_str).collect();
        let changes: Vec<Change> = (0..50)
            .map(|_| edited(Level::Decision, &"`".repeat(10_000), &heading_refs))
            .collect();
        let documents: Vec<DocumentChanges> = (0..20)
            .map(|n| {
                document(
                    &format!("{tick_path}{n}"),
                    FileState::Modified,
                    changes.clone(),
                )
            })
            .collect();
        let list = list(documents);
        let folders: Vec<String> = (0..100)
            .map(|n| format!("{}{n}", "`".repeat(200)))
            .collect();
        let roots = Roots::new(&folders).unwrap();
        let link = format!(
            "https://example.com/{}",
            "a".repeat(super::super::args::MAX_LINK - 20)
        );
        let text = summary(&list, &context(&roots, Some(&link)));
        assert!(text.chars().count() < 16_000, "{}", text.chars().count());
        assert_eq!(
            text.lines().filter(|line| line.contains(" · ")).count(),
            TOP
        );
        assert!(text.contains("…and 990 more changes."), "{text}");
        assert!(text.contains(", and 95 more folders.</sub>"), "{text}");
    }

    #[test]
    fn a_block_with_neither_side_shows_only_its_place() {
        let mut change = removed(Level::Wording, "x");
        change.subject = Subject::Block {
            old: Side { at: 0, block: None },
            new: Side { at: 0, block: None },
        };
        let doc = document("a.md", FileState::Modified, Vec::new());
        assert_eq!(line(&doc, &change), "**wording**, removed · `a.md`");
        let untitled = whole(ChangeKind::Added, Level::Fact, None);
        let mut nothing = untitled.clone();
        if let Subject::Document { document } = &mut nothing.subject {
            document.summary = None;
        }
        assert_eq!(line(&doc, &nothing), "**fact**, new document · `a.md`");
        let moved = Change {
            kind: ChangeKind::MovedEdited,
            ..edited(Level::Wording, "w", &[])
        };
        assert_eq!(kind_words(&moved), "moved and edited");
        for (kind, words) in [
            (ChangeKind::Formatting, "formatting only"),
            (ChangeKind::Added, "added"),
            (ChangeKind::Moved, "moved"),
        ] {
            let change = Change {
                kind,
                ..edited(Level::Wording, "w", &[])
            };
            assert_eq!(kind_words(&change), words);
        }
    }
}
