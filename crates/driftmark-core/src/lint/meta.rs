//! Front matter and status: YAML that parses, the keys the document's type
//! requires, and a status the type lists.

use super::{Context, Report, Rule};
use crate::document::{Block, BlockKind, FieldOrigin, normalize_key};
use crate::rules::{Location, Place, TypeRule};

pub(super) fn check(context: &Context<'_>, report: &mut Report<'_>) {
    let front_matter = context
        .document
        .blocks
        .first()
        .filter(|block| matches!(block.kind, BlockKind::FrontMatter));
    if let Some(error) = &context.document.meta.front_matter_error {
        let line = error_line(error).unwrap_or(1);
        report.add(
            Rule::FrontMatter,
            (line, 1),
            format!("{error}; fix the YAML between the `---` lines"),
        );
    }
    let Some(rule) = context.rules.type_for(context.path) else {
        return;
    };
    if context.document.meta.front_matter_error.is_none() {
        required(context, report, rule, front_matter.is_some());
    }
    if report.wants(Rule::StatusValue) {
        status(context, report, rule, front_matter);
    }
}

/// The file line a front matter error names. The document model writes it
/// as "…, on line 3 of the file".
fn error_line(error: &str) -> Option<usize> {
    let (_, after) = error.rsplit_once(", on line ")?;
    after.strip_suffix(" of the file")?.parse().ok()
}

/// Keys or values for a message, each in backticks and joined by `joint`:
/// `` `a` ``, `` `a` and `b` ``, `` `a`, `b` and `c` ``.
fn listing(items: &[&str], joint: &str) -> String {
    let quoted: Vec<String> = items.iter().map(|item| format!("`{item}`")).collect();
    match quoted.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} {joint} {last}", rest.join(", ")),
        _ => quoted.concat(),
    }
}

fn required(
    context: &Context<'_>,
    report: &mut Report<'_>,
    rule: &TypeRule,
    has_front_matter: bool,
) {
    let meta = &context.document.meta;
    let missing: Vec<&str> = rule
        .required
        .iter()
        .filter(|key| {
            meta.field(FieldOrigin::FrontMatter, key)
                .is_none_or(|field| {
                    field
                        .value
                        .as_text()
                        .is_some_and(|text| text.trim().is_empty())
                })
        })
        .map(String::as_str)
        .collect();
    if missing.is_empty() {
        return;
    }
    let keys = listing(&missing, "and");
    let message = if has_front_matter {
        let it = if missing.len() == 1 { "it" } else { "them" };
        format!(
            "the front matter lacks {keys}, which every `{}` document needs; add {it} with a value",
            rule.name
        )
    } else {
        format!(
            "the file has no front matter, and every `{}` document needs {keys} there; add front matter between `---` lines at the top of the file",
            rule.name
        )
    };
    report.add(Rule::RequiredField, (1, 1), message);
}

fn status(
    context: &Context<'_>,
    report: &mut Report<'_>,
    rule: &TypeRule,
    front_matter: Option<&Block>,
) {
    let Some(location) = &rule.status else {
        return;
    };
    let Some(found) = location.find(context.document) else {
        return;
    };
    let allowed: Vec<&str> = rule.statuses().collect();
    if allowed.is_empty() || rule.allows_status(&found.text) {
        return;
    }
    let start = place_start(context, location, found.place, front_matter);
    let word = found.text.split_whitespace().next().unwrap_or_default();
    let at = context.document.source[start..]
        .find(word)
        .map_or(start, |offset| start + offset);
    report.add(
        Rule::StatusValue,
        context.lines.at(at),
        format!(
            "the status {:?} is not one the `{}` type lists; use {}",
            found.text,
            rule.name,
            listing(&allowed, "or")
        ),
    );
}

/// Where the place that held a status starts, so the status itself can be
/// found from there: its key's line in the front matter, the end of the
/// title for a masthead, the end of the section's heading for a section.
fn place_start(
    context: &Context<'_>,
    location: &Location,
    place: Place,
    front_matter: Option<&Block>,
) -> usize {
    let blocks = &context.document.blocks;
    let heading_end = |title: Option<&str>| {
        blocks
            .iter()
            .find(|block| {
                block.containers.is_empty()
                    && match (&block.kind, title) {
                        (BlockKind::Heading { level }, None) => *level == 1,
                        (BlockKind::Heading { .. }, Some(title)) => {
                            normalize_key(&block.text) == normalize_key(title)
                        }
                        _ => false,
                    }
            })
            .map_or(0, |block| block.range.end)
    };
    match place {
        Place::FrontMatter => {
            let key = normalize_key(location.front_matter.as_deref().unwrap_or_default());
            front_matter.map_or(0, |block| {
                let source = context.document.source_of(block);
                let mut offset = block.range.start;
                for line in source.split_inclusive('\n') {
                    if line
                        .split_once(':')
                        .is_some_and(|(written, _)| normalize_key(written) == key)
                    {
                        return offset;
                    }
                    offset += line.len();
                }
                block.range.start
            })
        }
        Place::Masthead => heading_end(None),
        Place::Section => heading_end(location.section.as_deref()),
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{lint_with, messages};
    use super::{error_line, listing};

    const RULES: &str = r#"
[[type]]
name = "decision"
paths = ["docs/decisions/*.md"]
status = { front-matter = "status", masthead = "Status" }
current = ["accepted"]
superseded = ["superseded"]
other-statuses = ["proposed"]
required = ["status", "date"]

[[type]]
name = "adr"
paths = ["adr/*.md"]
status = { section = "Status" }
current = ["accepted"]

[[type]]
name = "note"
paths = ["notes/*.md"]
status = { front-matter = "status" }

[[type]]
name = "plain"
paths = ["plain/*.md"]
"#;

    fn found(path: &str, source: &str) -> Vec<String> {
        messages(&lint_with(RULES, path, source))
    }

    #[test]
    fn valid_front_matter_passes() {
        let source = "---\nstatus: proposed\ndate: 2026-01-05\n---\n\n# A\n\nSummary.\n";
        assert_eq!(found("docs/decisions/a.md", source), Vec::<String>::new());
        assert_eq!(
            found("elsewhere.md", "# A\n\nSummary.\n"),
            Vec::<String>::new()
        );
        assert_eq!(
            found(
                "notes/a.md",
                "---\nstatus: anything\n---\n\n# A\n\nSummary.\n"
            ),
            Vec::<String>::new()
        );
        assert_eq!(
            found(
                "plain/a.md",
                "---\nstatus: anything\n---\n\n# A\n\nSummary.\n"
            ),
            Vec::<String>::new()
        );
    }

    #[test]
    fn yaml_that_does_not_parse_is_reported_on_its_line() {
        assert_eq!(
            found(
                "x.md",
                "---\nstatus: [accepted\ndate: 2026-01-05\n---\n\n# A\n\nSummary.\n"
            ),
            [
                "3:1 error front-matter: the front matter is not valid YAML: illegal placement of ':' indicator, on line 3 of the file; fix the YAML between the `---` lines"
            ]
        );
        assert_eq!(error_line("no line here"), None);
        assert_eq!(error_line("x, on line two of the file"), None);
        assert_eq!(error_line("x, on line 4"), None);
    }

    #[test]
    fn missing_required_keys_are_listed() {
        assert_eq!(
            found(
                "docs/decisions/a.md",
                "---\nstatus: accepted\ndate: \" \"\n---\n\n# A\n\nSummary.\n"
            ),
            [
                "1:1 error required-field: the front matter lacks `date`, which every `decision` document needs; add it with a value"
            ]
        );
        assert_eq!(
            found(
                "docs/decisions/a.md",
                "---\nother: x\n---\n\n# A\n\nSummary.\n"
            ),
            [
                "1:1 error required-field: the front matter lacks `status` and `date`, which every `decision` document needs; add them with a value"
            ]
        );
        assert_eq!(
            found(
                "docs/decisions/a.md",
                "---\nstatus: [a, b]\ndate: 2026\n---\n\n# A\n\nSummary.\n"
            ),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_file_without_front_matter_is_told_to_add_it() {
        assert_eq!(
            found(
                "docs/decisions/a.md",
                "# A\n\n> Status: accepted\n\nSummary.\n"
            ),
            [
                "1:1 error required-field: the file has no front matter, and every `decision` document needs `status` and `date` there; add front matter between `---` lines at the top of the file"
            ]
        );
    }

    #[test]
    fn broken_yaml_hides_the_required_check() {
        let lines = found(
            "docs/decisions/a.md",
            "---\nstatus: [x\n---\n\n# A\n\nSummary.\n",
        );
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("front-matter"), "{lines:?}");
    }

    #[test]
    fn unlisted_statuses_are_reported_where_they_are_written() {
        assert_eq!(
            found(
                "docs/decisions/a.md",
                "---\ntitle: draft notes\ndate: 2026-01-05\nstatus: draft\n---\n\n# A\n\nSummary.\n"
            ),
            [
                "4:9 warning status-value: the status \"draft\" is not one the `decision` type lists; use `accepted`, `superseded` or `proposed`"
            ]
        );
        assert_eq!(
            found(
                "docs/decisions/a.md",
                "---\ndate: 2026-01-05\nrequired: x\n---\n\n# Draft A\n\n> Status: Draft | Date: x\n\nSummary.\n"
            )
            .iter()
            .filter(|line| line.contains("status-value"))
            .collect::<Vec<_>>(),
            ["8:11 warning status-value: the status \"Draft\" is not one the `decision` type lists; use `accepted`, `superseded` or `proposed`"]
        );
        assert_eq!(
            found(
                "docs/decisions/a.md",
                "---\ndate: x\n\"status\": draft\n---\n\n# A\n\nSummary.\n"
            ),
            [
                "3:11 warning status-value: the status \"draft\" is not one the `decision` type lists; use `accepted`, `superseded` or `proposed`"
            ]
        );
        assert_eq!(
            found(
                "adr/0001-a.md",
                "# 1. Use X\n\nRejected ideas are listed here.\n\n## Status\n\nRejected\n"
            ),
            [
                "7:1 warning status-value: the status \"Rejected\" is not one the `adr` type lists; use `accepted`"
            ]
        );
        assert_eq!(
            found(
                "adr/0001-a.md",
                "# 1. Use X\n\nSummary.\n\n## Status\n\nRejected for now\n"
            ),
            [
                "7:1 warning status-value: the status \"Rejected for now\" is not one the `adr` type lists; use `accepted`"
            ]
        );
    }

    #[test]
    fn a_status_found_nowhere_is_not_checked_here() {
        assert_eq!(
            found("adr/0001-a.md", "# 1. Use X\n\nSummary.\n"),
            Vec::<String>::new()
        );
        let off = lint_with(
            &format!("{RULES}\n[lint.rules]\nstatus-value = \"off\"\n"),
            "adr/a.md",
            "# A\n\nSummary.\n\n## Status\n\nRejected\n",
        );
        assert_eq!(off, []);
    }

    #[test]
    fn listings_read_as_a_sentence() {
        assert_eq!(listing(&["a"], "and"), "`a`");
        assert_eq!(listing(&["a", "b"], "or"), "`a` or `b`");
        assert_eq!(listing(&["a", "b", "c"], "and"), "`a`, `b` and `c`");
        assert_eq!(listing(&[], "and"), "");
    }
}
