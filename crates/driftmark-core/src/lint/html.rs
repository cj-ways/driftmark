//! Raw HTML: only the elements in the allow-list.
//!
//! HTML blocks come from the block list; inline HTML from the comrak tree,
//! where each tag is a node of its own. Only opening tags are checked, so one
//! element gives one finding. Comments are allowed: they render nothing.

use comrak::nodes::NodeValue;

use super::{Context, Report, Rule};
use crate::document::BlockKind;

pub(super) fn check(context: &Context<'_>, report: &mut Report<'_>) {
    if !report.wants(Rule::Html) {
        return;
    }
    let source = context.document.source.as_str();
    for block in &context.document.blocks {
        if matches!(block.kind, BlockKind::Html) {
            scan(
                context,
                report,
                block.range.start,
                &source[block.range.clone()],
            );
        }
    }
    for node in context.root.descendants() {
        if matches!(node.data().value, NodeValue::HtmlInline(_))
            && let Some(range) = context.positions.range(node)
        {
            scan(context, report, range.start, &source[range]);
        }
    }
}

/// Reports every opening tag in `html` whose element is not allowed.
/// `start` is where `html` begins in the file.
fn scan(context: &Context<'_>, report: &mut Report<'_>, start: usize, html: &str) {
    let mut rest = html;
    while let Some(found) = rest.find('<') {
        let tag = &rest[found..];
        if let Some(comment) = tag.strip_prefix("<!--") {
            rest = comment.split_once("-->").map_or("", |(_, after)| after);
            continue;
        }
        rest = &tag[1..];
        if !rest.starts_with(|c: char| c.is_ascii_alphabetic()) {
            continue;
        }
        let name_length = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
            .unwrap_or(rest.len());
        let name = rest[..name_length].to_ascii_lowercase();
        if !context.settings.allowed_html.contains(&name) {
            report.add(
                Rule::Html,
                context.lines.at(start + html.len() - tag.len()),
                format!(
                    "`<{name}>` is raw HTML outside the allowed list; write it in Markdown, or add {name:?} to `allowed-html` in the rules file's `[lint]` table"
                ),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{lint, lint_with, messages};

    const HEAD: &str = "# Title\n\nSummary.\n\n";

    #[test]
    fn allowed_elements_and_comments_pass() {
        let source = format!(
            "{HEAD}<details>\n<summary>More</summary>\n\nHidden <kbd>Ctrl</kbd>, H<sub>2</sub>O, x<sup>2</sup>,<br>done.\n\n</details>\n\n<!-- a <div> inside a comment -->\n\nAn <!-- inline --> comment, and a < b, and `<code>`.\n\n<DETAILS>caps</DETAILS>\n"
        );
        assert_eq!(lint("t.md", &source), []);
    }

    #[test]
    fn other_elements_are_reported_once_each() {
        let source = format!(
            "{HEAD}<div align=\"center\">\n  <img src=\"a.png\">\n</div>\n\nSome <span>inline</span> and <b>bold</b>.\n\n<!-- open comment <script>\n"
        );
        assert_eq!(
            messages(&lint("t.md", &source)),
            [
                "5:1 warning html: `<div>` is raw HTML outside the allowed list; write it in Markdown, or add \"div\" to `allowed-html` in the rules file's `[lint]` table",
                "6:3 warning html: `<img>` is raw HTML outside the allowed list; write it in Markdown, or add \"img\" to `allowed-html` in the rules file's `[lint]` table",
                "9:6 warning html: `<span>` is raw HTML outside the allowed list; write it in Markdown, or add \"span\" to `allowed-html` in the rules file's `[lint]` table",
                "9:30 warning html: `<b>` is raw HTML outside the allowed list; write it in Markdown, or add \"b\" to `allowed-html` in the rules file's `[lint]` table",
            ]
        );
    }

    #[test]
    fn the_allow_list_comes_from_the_rules_file() {
        let rules = "[lint]\nallowed-html = [\"IMG\", \"a\"]\n";
        let source = format!("{HEAD}<img src=\"x.png\"> <a name=\"x\"></a> <kbd>K</kbd>\n");
        assert_eq!(
            messages(&lint_with(rules, "t.md", &source)),
            [
                "5:36 warning html: `<kbd>` is raw HTML outside the allowed list; write it in Markdown, or add \"kbd\" to `allowed-html` in the rules file's `[lint]` table"
            ]
        );
    }

    #[test]
    fn text_right_after_a_comment_is_read_and_tags_inside_it_are_not() {
        let source =
            format!("{HEAD}<!--<b>-->\n\n<details><!-- <i> x --><span>y</span></details>\n");
        assert_eq!(
            messages(&lint("t.md", &source)),
            [
                "7:24 warning html: `<span>` is raw HTML outside the allowed list; write it in Markdown, or add \"span\" to `allowed-html` in the rules file's `[lint]` table"
            ]
        );
    }

    #[test]
    fn turned_off_it_reports_nothing() {
        let found = lint_with(
            "[lint.rules]\nhtml = \"off\"\n",
            "t.md",
            &format!("{HEAD}<div>x</div>\n"),
        );
        assert_eq!(found, []);
    }

    #[test]
    fn a_tag_at_the_very_end_is_read() {
        let found = lint("t.md", &format!("{HEAD}<p\n"));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].message.split('`').nth(1), Some("<p>"));
    }
}
