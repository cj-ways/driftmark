//! Code blocks and diagrams: every block closed and naming its language, no
//! code line over the limit, and every diagram parsing.

use super::{Context, Report, Rule};
use crate::document::{Block, BlockKind, DiagramLanguage};

pub(super) fn check(context: &Context<'_>, report: &mut Report<'_>) {
    for block in &context.document.blocks {
        match &block.kind {
            BlockKind::Code { language, closed } => {
                if !closed {
                    unclosed(context, report, block);
                }
                match language {
                    None => missing_language(context, report, block),
                    Some(language) => diagram_spelling(context, report, block, language),
                }
                long_lines(context, report, block);
            }
            BlockKind::Diagram { language, closed } => {
                if !closed {
                    unclosed(context, report, block);
                } else if report.wants(Rule::Diagram) {
                    diagram(context, report, block, *language);
                }
            }
            _ => {}
        }
    }
}

/// The opening fence of a fenced block, such as ```` ``` ```` or `~~~~`;
/// `None` for an indented block.
fn fence<'s>(context: &Context<'s>, block: &Block) -> Option<&'s str> {
    let source = context.document.source_of(block);
    let marker = source.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    Some(&source[..source.find(|c| c != marker).unwrap_or(source.len())])
}

fn unclosed(context: &Context<'_>, report: &mut Report<'_>, block: &Block) {
    let fence = fence(context, block).unwrap_or("```");
    report.add(
        Rule::CodeClosed,
        context.lines.at(block.range.start),
        format!(
            "this code block is never closed, so the rest of the file shows as code; add a closing {fence} line where the code ends"
        ),
    );
}

fn missing_language(context: &Context<'_>, report: &mut Report<'_>, block: &Block) {
    let message = if fence(context, block).is_some() {
        "this code block names no language; write it after the opening fence, such as ```sh, or ```text for plain text"
    } else {
        "this indented code block cannot name its language; write it as a fenced block instead, such as ```sh, or ```text for plain text"
    };
    report.add(
        Rule::CodeLanguage,
        context.lines.at(block.range.start),
        message.to_owned(),
    );
}

/// A code block whose language is a diagram language in the wrong case.
/// GitHub draws only lower-case `mermaid`, so driftmark does the same, and
/// such a block shows as code.
fn diagram_spelling(context: &Context<'_>, report: &mut Report<'_>, block: &Block, language: &str) {
    let lower = language.to_ascii_lowercase();
    if lower == "mermaid" || lower == "d2" {
        report.add(
            Rule::Diagram,
            context.lines.at(block.range.start),
            format!(
                "this block is written ```{language}, so it shows as code instead of a diagram; write ```{lower} in lower case"
            ),
        );
    }
}

/// The 1-based file line of each line of a code block's text: the line
/// after the opening fence for a fenced block, the block's own first line
/// for an indented one.
fn first_code_line(context: &Context<'_>, block: &Block) -> usize {
    let start = context.lines.line_of(block.range.start);
    if fence(context, block).is_some() {
        start + 1
    } else {
        start
    }
}

/// The column, from 1, of character `index` of a code line on file line
/// `line`. A code line is the end of its source line, after any indentation
/// and quote markers, so the characters before it are counted from the
/// source line's length.
fn column_in(context: &Context<'_>, line: usize, code_line: &str, index: usize) -> usize {
    let source_line = context.lines.text(line).chars().count();
    source_line.saturating_sub(code_line.chars().count()) + index + 1
}

fn long_lines(context: &Context<'_>, report: &mut Report<'_>, block: &Block) {
    let limit = context.settings.max_code_line_length;
    let first = first_code_line(context, block);
    for (number, code_line) in block.text.split('\n').enumerate() {
        let length = code_line.chars().count();
        if length > limit {
            let line = first + number;
            report.add(
                Rule::CodeLineLength,
                (line, column_in(context, line, code_line, limit)),
                format!(
                    "this code line has {length} characters, more than the limit of {limit}; break it into shorter lines"
                ),
            );
        }
    }
}

fn diagram(
    context: &Context<'_>,
    report: &mut Report<'_>,
    block: &Block,
    language: DiagramLanguage,
) {
    let Err(problem) = context.diagrams.check(language, &block.text) else {
        return;
    };
    let name = match language {
        DiagramLanguage::Mermaid => "Mermaid",
        DiagramLanguage::D2 => "D2",
    };
    let at = match problem.offset {
        Some(offset) => {
            let mut offset = offset.min(block.text.len());
            while !block.text.is_char_boundary(offset) {
                offset -= 1;
            }
            let before = &block.text[..offset];
            let number = before.matches('\n').count();
            let line_start = before.rfind('\n').map_or(0, |at| at + 1);
            let code_line = block.text[line_start..].split('\n').next().unwrap_or("");
            let line = first_code_line(context, block) + number;
            let index = block.text[line_start..offset].chars().count();
            (line, column_in(context, line, code_line, index))
        }
        None => context.lines.at(block.range.start),
    };
    report.add(
        Rule::Diagram,
        at,
        format!(
            "this {name} diagram will not draw: {}; fix the diagram code",
            problem.message
        ),
    );
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::super::tests::{lint, lint_with, messages};
    use super::super::{Anchors, DiagramCheck, DiagramProblem, LinkTargets, Linter};
    use crate::document::DiagramLanguage;
    use crate::rules::Rules;

    const HEAD: &str = "# Title\n\nSummary.\n\n";

    fn body(text: &str) -> Vec<String> {
        messages(&lint("t.md", &format!("{HEAD}{text}")))
    }

    #[test]
    fn closed_named_blocks_pass() {
        assert_eq!(
            body("```rust\nfn main() {}\n```\n\n~~~text\nplain\n~~~\n"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn unclosed_blocks_name_their_own_fence() {
        assert_eq!(
            body("````sh\nls\n"),
            [
                "5:1 error code-closed: this code block is never closed, so the rest of the file shows as code; add a closing ```` line where the code ends"
            ]
        );
        assert_eq!(
            body("~~~\nls\n"),
            [
                "5:1 error code-closed: this code block is never closed, so the rest of the file shows as code; add a closing ~~~ line where the code ends",
                "5:1 warning code-language: this code block names no language; write it after the opening fence, such as ```sh, or ```text for plain text",
            ]
        );
        assert_eq!(
            body("```mermaid\nflowchart TD\n"),
            [
                "5:1 error code-closed: this code block is never closed, so the rest of the file shows as code; add a closing ``` line where the code ends"
            ]
        );
    }

    #[test]
    fn blocks_without_a_language_are_reported() {
        assert_eq!(
            body("```\nls\n```\n\nText:\n\n    indented code\n"),
            [
                "5:1 warning code-language: this code block names no language; write it after the opening fence, such as ```sh, or ```text for plain text",
                "11:5 warning code-language: this indented code block cannot name its language; write it as a fenced block instead, such as ```sh, or ```text for plain text",
            ]
        );
    }

    #[test]
    fn diagram_languages_must_be_lower_case() {
        assert_eq!(
            body("```Mermaid\nflowchart TD\n```\n\n```D2\na -> b\n```\n\n```mermaidish\nx\n```\n"),
            [
                "5:1 error diagram: this block is written ```Mermaid, so it shows as code instead of a diagram; write ```mermaid in lower case",
                "9:1 error diagram: this block is written ```D2, so it shows as code instead of a diagram; write ```d2 in lower case",
            ]
        );
    }

    #[test]
    fn long_code_lines_point_past_the_limit() {
        let rules = "[lint]\nmax-code-line-length = 5\n";
        let source = format!(
            "{HEAD}```text\n12345\n123456\n```\n\n> ```text\n> 1234567\n> ```\n\n- item\n\n  ```text\n  abcdef\n  ```\n\nText:\n\n    abcdefg\n\n```mermaid\nflowchart TD\n  long --> line\n```\n"
        );
        assert_eq!(
            messages(&lint_with(rules, "t.md", &source)),
            [
                "7:6 warning code-line-length: this code line has 6 characters, more than the limit of 5; break it into shorter lines",
                "11:8 warning code-line-length: this code line has 7 characters, more than the limit of 5; break it into shorter lines",
                "17:8 warning code-line-length: this code line has 6 characters, more than the limit of 5; break it into shorter lines",
                "22:5 warning code-language: this indented code block cannot name its language; write it as a fenced block instead, such as ```sh, or ```text for plain text",
                "22:10 warning code-line-length: this code line has 7 characters, more than the limit of 5; break it into shorter lines",
            ]
        );
    }

    #[test]
    fn diagram_problems_point_into_the_code() {
        // The fake diagram parser fails on the word `broken`, and names its
        // offset; on `vague` it fails without one.
        assert_eq!(
            body(
                "```mermaid\nflowchart TD\n  A --> broken\n```\n\n> ```mermaid\n> é broken\n> ```\n\n```d2\nvague\n```\n\n```mermaid\nfine\n```\n"
            ),
            [
                "7:9 error diagram: this Mermaid diagram will not draw: it says broken; fix the diagram code",
                "11:5 error diagram: this Mermaid diagram will not draw: it says broken; fix the diagram code",
                "14:1 error diagram: this D2 diagram will not draw: it is vague; fix the diagram code",
            ]
        );
    }

    #[test]
    fn diagram_offsets_past_the_code_or_inside_a_character_are_clamped() {
        assert_eq!(
            body("```mermaid\nend at 999\n```\n\n```mermaid\nmid é at 5\n```\n"),
            [
                "6:11 error diagram: this Mermaid diagram will not draw: it ends badly; fix the diagram code",
                "10:5 error diagram: this Mermaid diagram will not draw: it ends badly; fix the diagram code",
            ]
        );
    }

    /// A diagram parser that counts its calls and accepts everything.
    struct Counting(AtomicUsize);

    impl DiagramCheck for Counting {
        fn check(&self, _: DiagramLanguage, _: &str) -> Result<(), DiagramProblem> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    #[test]
    fn a_rule_turned_off_skips_the_parser() {
        let source = format!("{HEAD}```mermaid\nflowchart TD\n```\n");
        let calls = |rules: &str| {
            let rules = Rules::parse(rules).expect("valid");
            let counting = Counting(AtomicUsize::new(0));
            let linter = Linter {
                rules: &rules,
                targets: &NoFiles,
                diagrams: &counting,
            };
            assert_eq!(linter.lint("t.md", source.as_bytes().to_vec()), []);
            counting.0.load(Ordering::Relaxed)
        };
        assert_eq!(calls(""), 1);
        assert_eq!(calls("[lint.rules]\ndiagram = \"off\"\n"), 0);
    }

    /// A repository with no files.
    struct NoFiles;

    impl LinkTargets for NoFiles {
        fn exists(&self, _: &str) -> bool {
            false
        }

        fn anchors(&self, _: &str) -> Option<Arc<Anchors>> {
            None
        }
    }

    #[test]
    fn an_empty_repository_has_nothing() {
        assert!(!NoFiles.exists("a.md"));
        assert!(NoFiles.anchors("a.md").is_none());
    }
}
