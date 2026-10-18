//! The shape of a document: one title first, a summary before the first
//! section, heading levels that do not skip, and the size limits on
//! headings, tables and the file.

use comrak::nodes::NodeValue;

use super::{Context, Report, Rule};
use crate::document::meta::{Head, head};
use crate::document::{Block, BlockKind};

pub(super) fn check(context: &Context<'_>, report: &mut Report<'_>) {
    if report.wants(Rule::Title) {
        title(context, report);
    }
    if report.wants(Rule::Summary) {
        summary(context, report);
    }
    if report.wants(Rule::HeadingLevels) {
        heading_levels(context, report);
    }
    let settings = context.settings;
    for block in &context.document.blocks {
        let at = || context.lines.at(block.range.start);
        match &block.kind {
            BlockKind::Heading { .. } => {
                let length = block.text.chars().count();
                if length > settings.max_heading_length {
                    report.add(
                        Rule::HeadingLength,
                        at(),
                        format!(
                            "this heading has {length} characters, more than the limit of {}; shorten it and say the rest in the text below it",
                            settings.max_heading_length
                        ),
                    );
                }
            }
            BlockKind::TableRow {
                header: true,
                cells,
            } if cells.len() > settings.max_table_columns => {
                report.add(
                    Rule::TableColumns,
                    at(),
                    format!(
                        "this table has {} columns, more than the limit of {}; split it into narrower tables, or turn some columns into rows",
                        cells.len(),
                        settings.max_table_columns
                    ),
                );
            }
            _ => {}
        }
    }
    if context.size > settings.max_file_size {
        report.add(
            Rule::FileSize,
            (1, 1),
            format!(
                "the file has {} bytes, more than the limit of {}; split it into smaller documents that link to each other",
                context.size, settings.max_file_size
            ),
        );
    }
}

/// A level-1 heading outside every container: what the document model
/// takes as the title.
fn is_title(block: &Block) -> bool {
    matches!(block.kind, BlockKind::Heading { level: 1 }) && block.containers.is_empty()
}

/// Whether a block may come before the title: front matter, or an HTML
/// comment, which renders nothing.
fn may_precede_title(context: &Context<'_>, block: &Block) -> bool {
    match block.kind {
        BlockKind::FrontMatter => true,
        BlockKind::Html => {
            block.text.is_empty() && context.document.source_of(block).starts_with("<!--")
        }
        _ => false,
    }
}

fn title(context: &Context<'_>, report: &mut Report<'_>) {
    let blocks = &context.document.blocks;
    let first = blocks
        .iter()
        .find(|block| !may_precede_title(context, block));
    let mut titles = blocks.iter().filter(|block| is_title(block));
    let Some(title) = titles.next() else {
        let at = first.map_or((1, 1), |block| context.lines.at(block.range.start));
        report.add(
            Rule::Title,
            at,
            "the file has no `#` title; start it with one, such as `# Release checklist`, right after any front matter".to_owned(),
        );
        return;
    };
    if let Some(first) = first.filter(|first| first.range != title.range) {
        report.add(
            Rule::Title,
            context.lines.at(first.range.start),
            "this comes before the `#` title, and only front matter may; move it below the title"
                .to_owned(),
        );
    }
    for extra in titles {
        report.add(
            Rule::Title,
            context.lines.at(extra.range.start),
            "this is a second `#` title, and a file has exactly one; make it a `##` section, or move it to a file of its own".to_owned(),
        );
    }
}

fn summary(context: &Context<'_>, report: &mut Report<'_>) {
    let nodes: Vec<_> = context.root.children().collect();
    let title = nodes.iter().position(
        |node| matches!(&node.data().value, NodeValue::Heading(heading) if heading.level == 1),
    );
    let after_title = title.map_or(0, |at| at + 1);
    let keys = context.rules.masthead_keys();
    let mut next_heading = None;
    for &node in &nodes[after_title..] {
        match &node.data().value {
            NodeValue::Heading(_) => {
                next_heading = Some(node);
                break;
            }
            NodeValue::Paragraph | NodeValue::BlockQuote => match head(node, keys) {
                Head::Masthead(_) => {}
                Head::Prose(_) | Head::Other => return,
            },
            _ => {}
        }
    }
    let at = next_heading
        .or_else(|| title.map(|at| nodes[at]))
        .and_then(|node| context.positions.range(node))
        .map_or((1, 1), |range| context.lines.at(range.start));
    report.add(
        Rule::Summary,
        at,
        "the document has no summary before its first section; add a paragraph or a `>` quote under the title that says in a sentence or two what the document is for".to_owned(),
    );
}

fn heading_levels(context: &Context<'_>, report: &mut Report<'_>) {
    let mut previous = None;
    for block in &context.document.blocks {
        if let BlockKind::Heading { level } = block.kind
            && block.containers.is_empty()
        {
            if let Some(above) = previous
                && level > above + 1
            {
                report.add(
                    Rule::HeadingLevels,
                    context.lines.at(block.range.start),
                    format!(
                        "this {} heading skips a level after the {} heading above it; make it {}",
                        hashes(level),
                        hashes(above),
                        hashes(above + 1)
                    ),
                );
            }
            previous = Some(level);
        }
    }
}

/// A heading level written as its `#` signs, in backticks: `` `###` ``.
fn hashes(level: u8) -> String {
    format!("`{}`", "#".repeat(usize::from(level)))
}

#[cfg(test)]
mod tests {
    use super::super::tests::{lint, lint_with, messages};
    use super::hashes;

    #[test]
    fn a_title_first_and_a_summary_pass() {
        assert_eq!(
            lint("t.md", "# Title\n\nWhat this is.\n\n## Part\n\nText.\n"),
            []
        );
        assert_eq!(
            lint(
                "t.md",
                "---\nstatus: draft\n---\n<!-- made by a tool -->\n# Title\n\n> What this is.\n\n## Part\n"
            ),
            []
        );
    }

    #[test]
    fn a_missing_title_is_reported_where_the_text_starts() {
        assert_eq!(
            messages(&lint("t.md", "---\na: b\n---\n\nJust text.\n")),
            [
                "5:1 error title: the file has no `#` title; start it with one, such as `# Release checklist`, right after any front matter"
            ]
        );
        assert_eq!(
            messages(&lint("t.md", "")),
            [
                "1:1 error title: the file has no `#` title; start it with one, such as `# Release checklist`, right after any front matter",
                "1:1 warning summary: the document has no summary before its first section; add a paragraph or a `>` quote under the title that says in a sentence or two what the document is for",
            ]
        );
    }

    #[test]
    fn text_before_the_title_and_a_second_title_are_reported() {
        assert_eq!(
            messages(&lint(
                "t.md",
                "Intro.\n\n# Title\n\nSummary.\n\n> # Quoted, not a title\n\n# Second\n"
            )),
            [
                "1:1 error title: this comes before the `#` title, and only front matter may; move it below the title",
                "9:1 error title: this is a second `#` title, and a file has exactly one; make it a `##` section, or move it to a file of its own",
            ]
        );
        assert_eq!(
            messages(&lint("t.md", "<div></div>\n\n# Title\n\nSummary.\n")),
            [
                "1:1 error title: this comes before the `#` title, and only front matter may; move it below the title",
                "1:1 warning html: `<div>` is raw HTML outside the allowed list; write it in Markdown, or add \"div\" to `allowed-html` in the rules file's `[lint]` table",
            ]
        );
    }

    #[test]
    fn a_masthead_alone_is_not_a_summary() {
        assert_eq!(
            messages(&lint(
                "t.md",
                "# Title\n\n> Status: Accepted | Date: 2026-01-05\n\n## Context\n"
            )),
            [
                "5:1 warning summary: the document has no summary before its first section; add a paragraph or a `>` quote under the title that says in a sentence or two what the document is for"
            ]
        );
        assert_eq!(
            lint(
                "t.md",
                "# Title\n\n**Status**: Draft\n\n- a list\n\nNow the summary.\n\n## Context\n"
            ),
            []
        );
        assert_eq!(lint("t.md", "# Title\n\n> - a quoted list\n"), []);
        let short = "# Title\n\n> Short version: a few words only.\n\n**Owner**: Ana\n\n## Part\n";
        assert_eq!(
            lint("t.md", short),
            [],
            "an unknown label is no masthead key"
        );
        assert_eq!(
            messages(&lint_with(
                "[masthead]\nkeys = [\"Short version\"]\n",
                "t.md",
                short
            )),
            [
                "7:1 warning summary: the document has no summary before its first section; add a paragraph or a `>` quote under the title that says in a sentence or two what the document is for"
            ]
        );
        assert_eq!(
            messages(&lint("t.md", "# Title\n\n- only a list\n")),
            [
                "1:1 warning summary: the document has no summary before its first section; add a paragraph or a `>` quote under the title that says in a sentence or two what the document is for"
            ]
        );
    }

    #[test]
    fn without_a_title_the_summary_is_looked_for_from_the_start() {
        let found = lint("t.md", "Some text first.\n\n## Part\n");
        assert_eq!(
            messages(&found),
            [
                "1:1 error title: the file has no `#` title; start it with one, such as `# Release checklist`, right after any front matter"
            ]
        );
        let found = lint("t.md", "## Part\n");
        assert_eq!(found.len(), 2);
        assert_eq!(found[1].rule.id(), "summary");
        assert_eq!((found[1].line, found[1].column), (1, 1));
    }

    #[test]
    fn shape_rules_turned_off_report_nothing() {
        let rules = "[lint.rules]\ntitle = \"off\"\nsummary = \"off\"\nheading-levels = \"off\"\n";
        assert_eq!(
            lint_with(rules, "t.md", "Text.\n\n# A\n\n# B\n\n#### C\n"),
            []
        );
    }

    #[test]
    fn skipped_heading_levels_are_reported() {
        assert_eq!(
            messages(&lint(
                "t.md",
                "# Title\n\nSummary.\n\n### Skips\n\n#### Fine\n\n## Back up\n\n> ##### Inside a quote\n\n#### Skips again\n"
            )),
            [
                "5:1 warning heading-levels: this `###` heading skips a level after the `#` heading above it; make it `##`",
                "13:1 warning heading-levels: this `####` heading skips a level after the `##` heading above it; make it `###`",
            ]
        );
        assert_eq!(hashes(2), "`##`");
    }

    #[test]
    fn long_headings_wide_tables_and_big_files_are_reported() {
        let rules = "[lint]\nmax-heading-length = 10\nmax-table-columns = 2\nmax-file-size = 60\n";
        let found = lint_with(
            rules,
            "t.md",
            "# Ten chars\n\nSummary.\n\n## Eleven char\n\n| a | b | c |\n|---|---|---|\n| 1 | 2 | 3 |\n\n| a | b |\n|---|---|\n",
        );
        assert_eq!(
            messages(&found),
            [
                "1:1 warning file-size: the file has 102 bytes, more than the limit of 60; split it into smaller documents that link to each other",
                "5:1 warning heading-length: this heading has 11 characters, more than the limit of 10; shorten it and say the rest in the text below it",
                "7:1 warning table-columns: this table has 3 columns, more than the limit of 2; split it into narrower tables, or turn some columns into rows",
            ]
        );
        let exactly = "# Ten chars.\n\nSummary.\n\n| a | b |\n|---|---|\n";
        let at_the_limits = format!(
            "[lint]\nmax-heading-length = 10\nmax-table-columns = 2\nmax-file-size = {}\n",
            exactly.len()
        );
        assert_eq!(lint_with(&at_the_limits, "t.md", exactly), []);
    }
}
