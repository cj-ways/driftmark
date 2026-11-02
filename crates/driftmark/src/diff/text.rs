//! The summary `driftmark diff` prints for a person: one line per changed
//! document, then the totals. In a terminal every document line fits its
//! width, 80 columns unless `COLUMNS` says otherwise: a long path is
//! shortened from the front, as `git diff --stat` does. In a file or a pipe,
//! paths stay whole, because a program or an agent may read them.

use std::ffi::OsString;
use std::fmt::Write as _;

use driftmark_core::diff::{ChangeKind, ChangeList, DocumentChanges, FileState, Level};
use driftmark_core::git::Roots;

/// The width the totals are wrapped to when there is no terminal, and the
/// terminal width when `COLUMNS` does not give one.
const WIDTH: usize = 80;

/// The narrowest terminal width taken from `COLUMNS`. Below it the columns
/// cannot fit, so the default is used.
const MIN_WIDTH: usize = 40;

/// The fewest characters a shortened path keeps, even when the line then
/// runs past the terminal's width.
const MIN_PATH: usize = 20;

/// The width of the level column: "formatting", the longest level name.
const LEVEL_WIDTH: usize = 10;

/// What standard output is, for the summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Terminal {
    /// Colour the level names.
    pub colour: bool,
    /// The terminal's width in columns; `None` when standard output is a
    /// file or a pipe, so no path is shortened.
    pub width: Option<usize>,
}

impl Terminal {
    /// A file or a pipe: no colours, no width.
    pub const NONE: Terminal = Terminal {
        colour: false,
        width: None,
    };

    /// What standard output is, from whether it is a terminal, the
    /// environment, and whether this is Windows.
    ///
    /// Colours appear only in a terminal, when `NO_COLOR` is unset or empty
    /// (no-color.org) and `TERM` is not `dumb`. On Windows, only in Windows
    /// Terminal (`WT_SESSION`) or a terminal that sets `TERM`: the older
    /// console prints colour codes as text unless a program switches them
    /// on, which needs code outside the standard library. The width is
    /// `COLUMNS` when it holds a number of at least 40, otherwise 80; the
    /// standard library cannot ask the terminal itself.
    #[must_use]
    pub fn detect(
        terminal: bool,
        env: impl Fn(&str) -> Option<OsString>,
        windows: bool,
    ) -> Terminal {
        if !terminal {
            return Terminal::NONE;
        }
        let set = |name: &str| env(name).is_some_and(|value| !value.is_empty());
        let dumb = env("TERM").is_some_and(|term| term == "dumb");
        let colour_console = !windows || set("WT_SESSION") || set("TERM");
        let width = env("COLUMNS")
            .and_then(|columns| columns.to_str()?.trim().parse().ok())
            .filter(|&columns| columns >= MIN_WIDTH)
            .unwrap_or(WIDTH);
        Terminal {
            colour: !set("NO_COLOR") && !dumb && colour_console,
            width: Some(width),
        }
    }
}

/// What the summary says beyond the change list.
pub(crate) struct Context<'a> {
    /// The range as written.
    pub label: &'a str,
    /// The folders read; empty for the whole repository.
    pub roots: &'a Roots,
    /// How many changed files were left out.
    pub skipped: usize,
    /// Where the summary goes.
    pub terminal: Terminal,
}

/// The summary of a change list.
pub(crate) fn summary(list: &ChangeList, context: &Context<'_>) -> String {
    let mut out = String::new();
    let (base, head) = list.range.as_ref().map_or_else(
        || ("?".to_owned(), "?".to_owned()),
        |range| {
            let base = range
                .base
                .as_deref()
                .map_or_else(|| "before the first commit".to_owned(), short);
            let head = match range.head.as_str() {
                "worktree" => "the working tree".to_owned(),
                id => short(id),
            };
            (base, head)
        },
    );
    let _ = writeln!(out, "{}: from {base} to {head}", escape(context.label));
    if !context.roots.folders().is_empty() {
        let folders: Vec<String> = context
            .roots
            .folders()
            .iter()
            .map(|folder| escape(&folder.to_lossy()))
            .collect();
        let _ = writeln!(out, "Only in: {}", folders.join(", "));
    }
    for note in list.range.iter().flat_map(|range| &range.notes) {
        let _ = writeln!(out, "Note: {}", escape(note));
    }
    out.push('\n');
    if list.documents.is_empty() && context.skipped == 0 {
        out.push_str("No Markdown files changed.\n");
        return out;
    }
    if !list.documents.is_empty() {
        let counts: Vec<String> = list.documents.iter().map(count).collect();
        let width = counts
            .iter()
            .map(String::len)
            .chain([CHANGES.len()])
            .max()
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "{:<LEVEL_WIDTH$}  {CHANGES:>width$}  document",
            "level"
        );
        for (document, count) in list.documents.iter().zip(&counts) {
            let level = document.summary.highest.map_or("none", level_name);
            let level = paint(
                &format!("{level:<LEVEL_WIDTH$}"),
                document.summary.highest,
                context.terminal.colour,
            );
            let note = file_note(document);
            let mut path = escape(&document.path);
            if let Some(columns) = context.terminal.width {
                let used = LEVEL_WIDTH + 2 + width + 2 + note.chars().count();
                path = shorten(&path, columns.saturating_sub(used).max(MIN_PATH));
            }
            let _ = writeln!(out, "{level}  {count:>width$}  {path}{note}");
        }
        out.push('\n');
    }
    let columns = context.terminal.width.unwrap_or(WIDTH);
    out.push_str(&totals(list, context.skipped, columns));
    out
}

/// The path in at most `room` characters. A longer path loses its first
/// folders, written `.../`, so the file name and its nearest folders stay.
/// When even the file name does not fit, it keeps its last characters.
pub(crate) fn shorten(path: &str, room: usize) -> String {
    if path.chars().count() <= room {
        return path.to_owned();
    }
    let parts: Vec<&str> = path.split('/').collect();
    let mut kept = 0;
    let mut length = "...".len();
    for part in parts.iter().rev() {
        let more = 1 + part.chars().count();
        if length + more > room {
            break;
        }
        length += more;
        kept += 1;
    }
    if kept > 0 {
        return format!(".../{}", parts[parts.len() - kept..].join("/"));
    }
    let tail: Vec<char> = path.chars().rev().take(room.saturating_sub(3)).collect();
    format!("...{}", tail.into_iter().rev().collect::<String>())
}

/// The header of the count column.
const CHANGES: &str = "changes";

/// `3 of 5`: the changes at the document's highest level, of all its
/// changes. `0` for a file whose blocks did not change.
fn count(document: &DocumentChanges) -> String {
    let summary = &document.summary;
    match summary.highest {
        Some(level) => format!(
            "{} of {}",
            number(summary.levels.get(level)),
            number(summary.changes)
        ),
        None => "0".to_owned(),
    }
}

/// What the line adds after the path: a new or deleted file, a status that
/// changed, or bytes that changed only between blocks.
fn file_note(document: &DocumentChanges) -> String {
    match document.file {
        FileState::Added => " (new)".to_owned(),
        FileState::Removed => " (deleted)".to_owned(),
        FileState::Modified => match &document.status {
            Some(status) => {
                let side =
                    |value: &Option<String>| value.as_deref().map_or("none".to_owned(), escape);
                format!(" (status {} -> {})", side(&status.old), side(&status.new))
            }
            None if document.summary.changes == 0 => " (blank lines only)".to_owned(),
            None => String::new(),
        },
    }
}

/// The totals: documents and changes, then the changes by level and by kind.
fn totals(list: &ChangeList, skipped: usize, columns: usize) -> String {
    let totals = &list.totals;
    let mut out = format!(
        "{} changed, {}",
        plural(list.documents.len(), "document", "documents"),
        plural(totals.changes, "change", "changes")
    );
    if skipped > 0 {
        let _ = write!(
            out,
            "; {} skipped, named on standard error",
            plural(skipped, "file", "files")
        );
    }
    out.push('\n');
    let levels: Vec<String> = Level::HIGHEST_FIRST
        .iter()
        .filter(|&&level| totals.levels.get(level) > 0)
        .map(|&level| format!("{} {}", number(totals.levels.get(level)), level_name(level)))
        .collect();
    out.push_str(&wrapped("By level", &levels, columns));
    let kinds: Vec<String> = ChangeKind::ALL
        .iter()
        .filter(|&&kind| totals.kinds.get(kind) > 0)
        .map(|&kind| format!("{} {}", number(totals.kinds.get(kind)), kind_name(kind)))
        .collect();
    out.push_str(&wrapped("By kind", &kinds, columns));
    out
}

/// `Head: a, b, c` on lines of at most `columns` characters, breaking only
/// between items. Nothing when there are no items.
fn wrapped(head: &str, items: &[String], columns: usize) -> String {
    let mut out = String::new();
    if items.is_empty() {
        return out;
    }
    let mut line = format!("{head}:");
    let mut has_item = false;
    for (index, item) in items.iter().enumerate() {
        let comma = if index + 1 < items.len() { "," } else { "" };
        let piece = format!("{item}{comma}");
        if has_item && line.chars().count() + 1 + piece.chars().count() > columns {
            out.push_str(&line);
            out.push('\n');
            " ".clone_into(&mut line);
        }
        line.push(' ');
        line.push_str(&piece);
        has_item = true;
    }
    out.push_str(&line);
    out.push('\n');
    out
}

/// The name of a level, as the JSON writes it.
pub(crate) fn level_name(level: Level) -> &'static str {
    match level {
        Level::Decision => "decision",
        Level::Rule => "rule",
        Level::Fact => "fact",
        Level::Wording => "wording",
        Level::Formatting => "formatting",
    }
}

/// The name of a kind of change, in words.
fn kind_name(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Formatting => "formatting",
        ChangeKind::Edited => "edited",
        ChangeKind::Added => "added",
        ChangeKind::Removed => "removed",
        ChangeKind::Moved => "moved",
        ChangeKind::MovedEdited => "moved and edited",
    }
}

/// Colours a level column. Higher levels stand out more: decision in bold
/// red, rule in bold yellow, fact in cyan, formatting dimmed.
fn paint(text: &str, level: Option<Level>, colour: bool) -> String {
    let code = match level {
        _ if !colour => None,
        Some(Level::Decision) => Some("1;31"),
        Some(Level::Rule) => Some("1;33"),
        Some(Level::Fact) => Some("36"),
        Some(Level::Formatting) | None => Some("2"),
        Some(Level::Wording) => None,
    };
    match code {
        Some(code) => format!("\x1b[{code}m{text}\x1b[0m"),
        None => text.to_owned(),
    }
}

/// `1 thing` or `2 things`, with thousands separated.
pub(crate) fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{} {}", number(n), if n == 1 { one } else { many })
}

/// A whole number with a comma between thousands: `161,799`.
pub(crate) fn number(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// The first 12 characters of a commit id.
pub(crate) fn short(id: &str) -> String {
    id.chars().take(12).collect()
}

/// Text from the repository with its control characters written as escapes,
/// such as `\n` and `\u{1b}`. A file name or a status can hold any of them;
/// written raw, a line break would split the line, and an escape sequence
/// could change the terminal.
pub(crate) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_control() {
            out.extend(c.escape_default());
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use driftmark_core::diff::{RangeInfo, StatusChange, Summary};

    use super::*;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| OsString::from(value))
        }
    }

    #[test]
    fn colour_and_width_only_in_a_terminal() {
        let colour = |terminal, pairs: &[(&str, &str)], windows| {
            Terminal::detect(terminal, env(pairs), windows).colour
        };
        assert!(colour(true, &[], false));
        assert!(!colour(false, &[], false), "not a terminal");
        assert!(!colour(true, &[("NO_COLOR", "1")], false));
        assert!(colour(true, &[("NO_COLOR", "")], false), "empty is unset");
        assert!(!colour(true, &[("TERM", "dumb")], false));
        assert!(!colour(true, &[], true), "the older Windows console");
        assert!(colour(true, &[("WT_SESSION", "x")], true));
        assert!(colour(true, &[("TERM", "xterm")], true));
        assert_eq!(
            Terminal::detect(false, env(&[("WT_SESSION", "x"), ("COLUMNS", "120")]), true),
            Terminal::NONE
        );
        let width = |pairs: &[(&str, &str)]| Terminal::detect(true, env(pairs), false).width;
        assert_eq!(width(&[]), Some(80));
        assert_eq!(width(&[("COLUMNS", "132")]), Some(132));
        assert_eq!(width(&[("COLUMNS", " 40 ")]), Some(40));
        assert_eq!(width(&[("COLUMNS", "39")]), Some(80), "too narrow");
        assert_eq!(width(&[("COLUMNS", "wide")]), Some(80));
    }

    #[test]
    fn long_paths_lose_their_first_folders() {
        let path = "docs/features/pages/cache/lifetime/decisions.md";
        assert_eq!(shorten(path, 60), path);
        assert_eq!(shorten(path, path.len()), path);
        assert_eq!(
            shorten(path, path.len() - 1),
            ".../features/pages/cache/lifetime/decisions.md"
        );
        assert_eq!(shorten(path, 30), ".../lifetime/decisions.md");
        assert_eq!(shorten(path, 16), ".../decisions.md");
        assert_eq!(shorten(path, 15), "...decisions.md");
        assert_eq!(shorten(path, 8), "...ns.md");
        assert_eq!(shorten("ქართული/ტექსტი/ფაილი.md", 16), ".../ფაილი.md");
    }

    #[test]
    fn level_names_match_the_json() {
        for level in Level::HIGHEST_FIRST {
            assert_eq!(
                serde_json::to_value(level).unwrap(),
                serde_json::json!(level_name(level))
            );
        }
    }

    #[test]
    fn numbers_group_thousands() {
        let cases = [
            (0, "0"),
            (7, "7"),
            (999, "999"),
            (1000, "1,000"),
            (161_799, "161,799"),
            (1_234_567, "1,234,567"),
        ];
        for (n, text) in cases {
            assert_eq!(number(n), text);
        }
    }

    #[test]
    fn control_characters_are_escaped() {
        assert_eq!(escape("docs/new\nline.md"), "docs/new\\nline.md");
        assert_eq!(escape("a\u{1b}[2Jb\u{9b}c"), "a\\u{1b}[2Jb\\u{9b}c");
        assert_eq!(escape("café ქართული"), "café ქართული");
    }

    #[test]
    fn long_totals_wrap_between_items() {
        let items: Vec<String> = [
            "3,000 decision",
            "24,504 rule",
            "107,202 fact",
            "30,023 wording",
            "70 formatting",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
        assert_eq!(
            wrapped("By level", &items, 80),
            "By level: 3,000 decision, 24,504 rule, 107,202 fact, 30,023 wording,\n  70 formatting\n"
        );
        assert_eq!(wrapped("By kind", &[], 80), "");
        let long = vec!["x".repeat(90), "y".to_owned()];
        assert_eq!(
            wrapped("H", &long, 80),
            format!("H: {},\n  y\n", "x".repeat(90))
        );
        let two = ["one".to_owned(), "two".to_owned()];
        assert_eq!(wrapped("H", &two, 10), "H: one,\n  two\n");
        assert_eq!(wrapped("H", &two, 11), "H: one, two\n");
    }

    fn document(path: &str, file: FileState, levels: &[Level]) -> DocumentChanges {
        let mut summary = Summary::default();
        for &level in levels {
            summary.changes += 1;
            summary.highest = summary.highest.max(Some(level));
            match level {
                Level::Decision => summary.levels.decision += 1,
                Level::Rule => summary.levels.rule += 1,
                Level::Fact => summary.levels.fact += 1,
                Level::Wording => summary.levels.wording += 1,
                Level::Formatting => summary.levels.formatting += 1,
            }
            summary.kinds.edited += 1;
        }
        DocumentChanges {
            path: path.to_owned(),
            file,
            status: None,
            summary,
            changes: Vec::new(),
            unchanged: Vec::new(),
        }
    }

    fn list(documents: Vec<DocumentChanges>) -> ChangeList {
        ChangeList::new(
            Some(RangeInfo {
                base: Some("a1b2c3d4e5f60718293a4b5c6d7e8f9012345678".to_owned()),
                head: "worktree".to_owned(),
                notes: vec!["compared with `origin/main`".to_owned()],
            }),
            documents,
        )
    }

    #[test]
    fn the_summary_lists_documents_by_level_with_their_counts() {
        let mut decision = document(
            "docs/decisions/0002-cache.md",
            FileState::Modified,
            &[Level::Decision, Level::Decision, Level::Fact],
        );
        decision.status = Some(StatusChange {
            old: Some("proposed".to_owned()),
            new: Some("accepted".to_owned()),
        });
        let list = list(vec![
            document("docs/notes.md", FileState::Modified, &[]),
            document("docs/new.md", FileState::Added, &[Level::Fact; 12]),
            decision,
            document("docs/old.md", FileState::Removed, &[Level::Wording]),
        ]);
        let roots = Roots::new(["docs"]).unwrap();
        let context = Context {
            label: "auto",
            roots: &roots,
            skipped: 1,
            terminal: Terminal::NONE,
        };
        assert_eq!(
            summary(&list, &context),
            "\
auto: from a1b2c3d4e5f6 to the working tree
Only in: docs
Note: compared with `origin/main`

level        changes  document
decision      2 of 3  docs/decisions/0002-cache.md (status proposed -> accepted)
fact        12 of 12  docs/new.md (new)
wording       1 of 1  docs/old.md (deleted)
none               0  docs/notes.md (blank lines only)

4 documents changed, 16 changes; 1 file skipped, named on standard error
By level: 2 decision, 13 fact, 1 wording
By kind: 16 edited
"
        );
    }

    #[test]
    fn colour_marks_only_the_level_column() {
        let list = list(vec![
            document("a.md", FileState::Modified, &[Level::Decision]),
            document("b.md", FileState::Modified, &[Level::Rule]),
            document("c.md", FileState::Modified, &[Level::Fact]),
            document("d.md", FileState::Modified, &[Level::Wording]),
            document("e.md", FileState::Modified, &[Level::Formatting]),
            document("f.md", FileState::Modified, &[]),
        ]);
        let context = Context {
            label: "a..b",
            roots: &Roots::all(),
            skipped: 0,
            terminal: Terminal {
                colour: true,
                width: None,
            },
        };
        let text = summary(&list, &context);
        for line in [
            "\x1b[1;31mdecision  \x1b[0m   1 of 1  a.md\n",
            "\x1b[1;33mrule      \x1b[0m   1 of 1  b.md\n",
            "\x1b[36mfact      \x1b[0m   1 of 1  c.md\n",
            "\nwording      1 of 1  d.md\n",
            "\x1b[2mformatting\x1b[0m   1 of 1  e.md\n",
            "\x1b[2mnone      \x1b[0m        0  f.md (blank lines only)\n",
        ] {
            assert!(text.contains(line), "{line:?} in\n{text}");
        }
        let plain = Context {
            terminal: Terminal::NONE,
            ..context
        };
        assert!(!summary(&list, &plain).contains('\x1b'));
    }

    #[test]
    fn in_a_terminal_every_document_line_fits_its_width() {
        let long = "docs/features/pages/cache/rendered-page-ttl/decisions/0012-one-hour.md";
        let mut status = document(long, FileState::Modified, &[Level::Decision]);
        status.status = Some(StatusChange {
            old: Some("proposed".to_owned()),
            new: Some("accepted".to_owned()),
        });
        let list = list(vec![
            status,
            document(
                &long.replace(".md", "-b.md"),
                FileState::Added,
                &[Level::Fact],
            ),
            document("docs/short.md", FileState::Modified, &[Level::Wording]),
        ]);
        let roots = Roots::all();
        let at = |width| Context {
            label: "a..b",
            roots: &roots,
            skipped: 0,
            terminal: Terminal {
                colour: false,
                width,
            },
        };
        let text = summary(&list, &at(Some(80)));
        let rows: Vec<&str> = text.lines().filter(|l| l.contains(".md")).collect();
        assert_eq!(
            rows,
            [
                "decision     1 of 1  .../0012-one-hour.md (status proposed -> accepted)",
                "fact         1 of 1  .../rendered-page-ttl/decisions/0012-one-hour-b.md (new)",
                "wording      1 of 1  docs/short.md",
            ]
        );
        assert!(rows.iter().all(|row| row.chars().count() <= 80));
        let piped = summary(&list, &at(None));
        assert!(piped.contains(&format!("  {long} (status")), "{piped}");
    }

    #[test]
    fn an_empty_range_says_so() {
        let mut empty = list(Vec::new());
        empty.range = Some(RangeInfo {
            base: None,
            head: "0123456789abcdef0123456789abcdef01234567".to_owned(),
            notes: Vec::new(),
        });
        let context = Context {
            label: "--since 2026-10-01",
            roots: &Roots::all(),
            skipped: 0,
            terminal: Terminal::NONE,
        };
        assert_eq!(
            summary(&empty, &context),
            "--since 2026-10-01: from before the first commit to 0123456789ab\n\nNo Markdown files changed.\n"
        );
        let only_skipped = Context {
            skipped: 2,
            ..context
        };
        assert!(summary(&empty, &only_skipped).ends_with(
            "\n0 documents changed, 0 changes; 2 files skipped, named on standard error\n"
        ));
        empty.range = None;
        assert!(summary(&empty, &only_skipped).starts_with("--since 2026-10-01: from ? to ?\n"));
    }

    #[test]
    fn a_status_that_appears_or_goes_says_none() {
        let mut gone = document("a.md", FileState::Modified, &[Level::Decision]);
        gone.status = Some(StatusChange {
            old: Some("accepted\u{1b}".to_owned()),
            new: None,
        });
        assert_eq!(file_note(&gone), " (status accepted\\u{1b} -> none)");
    }
}
