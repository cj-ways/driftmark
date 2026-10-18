//! `driftmark new <kind> <title>`: the five kinds of document, where each one
//! goes, and its template filled in with a title, a date and, for a
//! decision, a number.

use std::fmt;

/// The folder of decision records. The program lists its file names to find
/// the next number.
pub const DECISIONS_FOLDER: &str = "docs/decisions";

/// The most characters a name made from a title may have. Most titles fit,
/// and a name stays below the 255 bytes a file system allows, even when every
/// character takes four bytes.
const MAX_SLUG: usize = 60;

/// A kind of document that `driftmark new` can start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A numbered decision record in `docs/decisions/`.
    Decision,
    /// A standard in `docs/standards/`.
    Standard,
    /// A feature target in `docs/features/`.
    Feature,
    /// The plan of a working folder, `docs/working/<topic>/plan.md`.
    Plan,
    /// The status file of a working folder, `docs/working/<topic>/status.md`.
    Status,
}

impl Kind {
    /// Every kind, in the order the help text lists them.
    pub const ALL: [Kind; 5] = [
        Kind::Decision,
        Kind::Standard,
        Kind::Feature,
        Kind::Plan,
        Kind::Status,
    ];

    /// The kind's name, as the command line writes it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Kind::Decision => "decision",
            Kind::Standard => "standard",
            Kind::Feature => "feature",
            Kind::Plan => "plan",
            Kind::Status => "status",
        }
    }

    /// The kind with this name, if there is one.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|kind| kind.name() == name)
    }

    /// The kind's template. `{{title}}`, `{{date}}` and `{{number}}` mark
    /// where the values go; each HTML comment is guidance for the writer.
    fn template(self) -> &'static str {
        match self {
            Kind::Decision => include_str!("../../starter/templates/decision.md.tmpl"),
            Kind::Standard => include_str!("../../starter/templates/standard.md.tmpl"),
            Kind::Feature => include_str!("../../starter/templates/feature.md.tmpl"),
            Kind::Plan => include_str!("../../starter/templates/plan.md.tmpl"),
            Kind::Status => include_str!("../../starter/templates/status.md.tmpl"),
        }
    }

    /// Where a document of this kind goes. A plan and a status file share
    /// their topic's folder, so the same title puts both in one place.
    fn path(self, number: &str, slug: &str) -> String {
        match self {
            Kind::Decision => format!("{DECISIONS_FOLDER}/{number}-{slug}.md"),
            Kind::Standard => format!("docs/standards/{slug}.md"),
            Kind::Feature => format!("docs/features/{slug}.md"),
            Kind::Plan => format!("docs/working/{slug}/plan.md"),
            Kind::Status => format!("docs/working/{slug}/status.md"),
        }
    }
}

/// A document `driftmark new` is about to write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewDocument {
    /// Where it goes, relative to the repository root with `/` between
    /// folders.
    pub path: String,
    /// Its text: the kind's template, filled in.
    pub text: String,
}

/// Why a title cannot start a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitleError {
    /// The title is empty or only spaces.
    Empty,
    /// The title holds a line break, a tab or another control character, so
    /// it cannot be one heading line.
    NotOneLine,
    /// The title has no letter or digit, so no file name can be made from it.
    NoName,
}

impl fmt::Display for TitleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            TitleError::Empty => {
                "the title is empty; write it after the kind, such as `driftmark new decision \"Keep one database per service\"`"
            }
            TitleError::NotOneLine => {
                "the title holds a line break or a tab; write it on one line, with spaces between the words"
            }
            TitleError::NoName => {
                "the title has no letter or digit, so no file name can be made from it; use words in the title"
            }
        })
    }
}

impl std::error::Error for TitleError {}

/// The document `driftmark new` writes for a kind and a title.
///
/// `date` goes into the front matter as it is, such as `2026-10-18`; the
/// program reads it from the clock. `number` is the decision's number, from
/// [`next_decision_number`]. It is written with at least four digits, such as
/// `0007`, in the file name and the title. The other kinds have no number and
/// leave it out.
///
/// # Errors
///
/// A [`TitleError`] when the title is empty, spans more than one line, or
/// has no letter or digit to name a file with.
pub fn new_document(
    kind: Kind,
    title: &str,
    date: &str,
    number: u32,
) -> Result<NewDocument, TitleError> {
    let title = title.trim();
    if title.is_empty() {
        return Err(TitleError::Empty);
    }
    if title.chars().any(char::is_control) {
        return Err(TitleError::NotOneLine);
    }
    let slug = slug(title);
    if slug.is_empty() {
        return Err(TitleError::NoName);
    }
    let number = format!("{number:04}");
    // The title goes in last, so a title that holds `{{date}}` keeps it.
    let text = kind
        .template()
        .replace("{{date}}", date)
        .replace("{{number}}", &number)
        .replace("{{title}}", title);
    Ok(NewDocument {
        path: kind.path(&number, &slug),
        text,
    })
}

/// The number the next decision record takes: one more than the highest
/// number among `names`, the file names in the decisions folder, or 1 when
/// there is none. A record's name is its number, a `-`, its title and `.md`,
/// such as `0004-keep-one-database.md`; other names do not count.
///
/// Two branches that each add a record can still take the same number. The
/// second to merge renames its record.
pub fn next_decision_number<'a>(names: impl IntoIterator<Item = &'a str>) -> u32 {
    names
        .into_iter()
        .filter_map(record_number)
        .max()
        .map_or(1, |highest| highest.saturating_add(1))
}

/// The number at the start of a decision record's file name.
fn record_number(name: &str) -> Option<u32> {
    let rest = name.strip_suffix(".md")?;
    let digits = rest.find(|c: char| !c.is_ascii_digit())?;
    if digits == 0 || !rest[digits..].starts_with('-') {
        return None;
    }
    rest[..digits].parse().ok()
}

/// A title as a file or folder name: letters and digits in lower case, with
/// one `-` for each run of other characters. Apostrophes are dropped, so
/// "Don't cache" becomes `dont-cache`. A name longer than 60 characters is
/// cut after its last whole word that fits.
#[must_use]
pub fn slug(title: &str) -> String {
    let mut slug = String::new();
    let mut gap = false;
    for c in title.chars() {
        if c.is_alphanumeric() {
            if gap && !slug.is_empty() {
                slug.push('-');
            }
            gap = false;
            slug.extend(c.to_lowercase());
        } else if !matches!(c, '\'' | '\u{2019}') {
            gap = true;
        }
    }
    if slug.chars().count() <= MAX_SLUG {
        return slug;
    }
    // One character more than fits: when it is a `-`, the word before it
    // ends exactly at the limit and is kept.
    let head: String = slug.chars().take(MAX_SLUG + 1).collect();
    match head.rfind('-') {
        Some(end) => head[..end].to_owned(),
        None => head.chars().take(MAX_SLUG).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_have_names() {
        let names: Vec<&str> = Kind::ALL.into_iter().map(Kind::name).collect();
        assert_eq!(names, ["decision", "standard", "feature", "plan", "status"]);
        for kind in Kind::ALL {
            assert_eq!(Kind::from_name(kind.name()), Some(kind));
        }
        assert_eq!(Kind::from_name("Decision"), None);
        assert_eq!(Kind::from_name("adr"), None);
    }

    #[test]
    fn each_kind_has_its_place() {
        let paths: Vec<String> = Kind::ALL
            .into_iter()
            .map(|kind| {
                new_document(kind, "Faster search", "2026-10-18", 12)
                    .expect("a good title")
                    .path
            })
            .collect();
        assert_eq!(
            paths,
            [
                "docs/decisions/0012-faster-search.md",
                "docs/standards/faster-search.md",
                "docs/features/faster-search.md",
                "docs/working/faster-search/plan.md",
                "docs/working/faster-search/status.md",
            ]
        );
    }

    #[test]
    fn a_decision_gets_its_number_date_and_title() {
        let made = new_document(
            Kind::Decision,
            "  Keep one database per service ",
            "2026-10-18",
            7,
        )
        .expect("a good title");
        assert_eq!(
            made.path,
            "docs/decisions/0007-keep-one-database-per-service.md"
        );
        assert!(made.text.starts_with(
            "---\nstatus: proposed\ndate: 2026-10-18\n---\n\n# 0007 Keep one database per service\n\n> Short version: <!--"
        ));
        assert!(!made.text.contains("{{"));
    }

    #[test]
    fn numbers_grow_past_four_digits() {
        let made = new_document(Kind::Decision, "Later", "2026-10-18", 12_345).expect("a title");
        assert_eq!(made.path, "docs/decisions/12345-later.md");
        assert!(made.text.contains("\n# 12345 Later\n"));
    }

    #[test]
    fn plans_and_status_files_name_their_topic() {
        let plan = new_document(Kind::Plan, "Version 2", "2026-10-18", 1).expect("a title");
        assert!(plan.text.contains("\n# Version 2 plan\n"));
        let status = new_document(Kind::Status, "Version 2", "2026-10-18", 1).expect("a title");
        assert!(status.text.contains("\n# Version 2 status\n"));
        let standard = new_document(Kind::Standard, "Errors", "2026-10-18", 3).expect("a title");
        assert!(standard.text.contains("\n# Errors\n"));
        assert!(!standard.text.contains("0003"));
    }

    #[test]
    fn a_title_keeps_text_that_looks_like_a_placeholder() {
        let made = new_document(Kind::Feature, "Show {{date}} {{number}}", "2026-10-18", 1)
            .expect("a title");
        assert!(made.text.contains("\n# Show {{date}} {{number}}\n"));
        assert!(made.text.contains("\ndate: 2026-10-18\n"));
        assert_eq!(made.path, "docs/features/show-date-number.md");
    }

    #[test]
    fn bad_titles_are_refused() {
        let refused = |title: &str| new_document(Kind::Standard, title, "2026-10-18", 1);
        assert_eq!(refused(""), Err(TitleError::Empty));
        assert_eq!(refused(" \u{3000} "), Err(TitleError::Empty));
        assert_eq!(refused("Two\nlines"), Err(TitleError::NotOneLine));
        assert_eq!(refused("A\ttab"), Err(TitleError::NotOneLine));
        assert_eq!(refused("--- ?!"), Err(TitleError::NoName));
        assert_eq!(refused("''"), Err(TitleError::NoName));
    }

    #[test]
    fn title_errors_say_what_to_do() {
        assert_eq!(
            TitleError::Empty.to_string(),
            "the title is empty; write it after the kind, such as `driftmark new decision \"Keep one database per service\"`"
        );
        assert_eq!(
            TitleError::NotOneLine.to_string(),
            "the title holds a line break or a tab; write it on one line, with spaces between the words"
        );
        assert_eq!(
            TitleError::NoName.to_string(),
            "the title has no letter or digit, so no file name can be made from it; use words in the title"
        );
    }

    #[test]
    fn the_next_number_follows_the_highest() {
        assert_eq!(next_decision_number([]), 1);
        assert_eq!(next_decision_number(["README.md"]), 1);
        assert_eq!(
            next_decision_number(["0001-a.md", "0009-b.md", "0003-c.md", "README.md"]),
            10
        );
        assert_eq!(next_decision_number(["12-short.md"]), 13);
        assert_eq!(next_decision_number(["0004-a.md", "0004-b.md"]), 5);
    }

    #[test]
    fn only_record_names_count() {
        let not_records = [
            "0009-notes.txt",
            "0009.md",
            "0009_x.md",
            "-0009-x.md",
            "a0009-x.md",
            "0009-x.md.bak",
            "99999999999-too-big.md",
        ];
        for name in not_records {
            assert_eq!(next_decision_number([name, "0002-a.md"]), 3, "{name}");
        }
        let last = format!("{}-last.md", u32::MAX);
        assert_eq!(next_decision_number([last.as_str()]), u32::MAX);
    }

    #[test]
    fn slugs_keep_letters_and_digits() {
        let cases = [
            (
                "Keep one database per service",
                "keep-one-database-per-service",
            ),
            ("  Use  HTTP/2, not   HTTP/1.1! ", "use-http-2-not-http-1-1"),
            ("Don't cache", "dont-cache"),
            ("Don\u{2019}t cache", "dont-cache"),
            ("Ünïcode Straße", "ünïcode-straße"),
            ("ÉCOLE", "école"),
            ("---", ""),
            ("2026 roadmap", "2026-roadmap"),
        ];
        for (title, expected) in cases {
            assert_eq!(slug(title), expected, "{title}");
        }
    }

    #[test]
    fn long_slugs_end_on_a_whole_word() {
        let exactly = "a".repeat(60);
        assert_eq!(slug(&exactly), exactly);
        let words = format!("abcdef{}", " abcde".repeat(9));
        assert_eq!(slug(&words).chars().count(), 60);
        assert_eq!(slug(&words), words.replace(' ', "-"));
        let ends_at_limit = format!("{} {} tail", "a".repeat(9), "b".repeat(50));
        assert_eq!(
            slug(&ends_at_limit),
            format!("{}-{}", "a".repeat(9), "b".repeat(50))
        );
        let word = "abcde ".repeat(10);
        assert_eq!(slug(&word).chars().count(), 59);
        let fits = format!("{} more", "a".repeat(60));
        assert_eq!(slug(&fits), "a".repeat(60));
        let long = format!("{} {}", "b".repeat(50), "c".repeat(20));
        assert_eq!(slug(&long), "b".repeat(50));
        let one_word = "d".repeat(70);
        assert_eq!(slug(&one_word), "d".repeat(60));
        let wide = "ü".repeat(70);
        assert_eq!(slug(&wide), "ü".repeat(60));
    }
}
