//! The lint's diagram check, with merman's Mermaid parser.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::OnceLock;

use driftmark_core::document::DiagramLanguage;
use driftmark_core::lint::{DiagramCheck, DiagramProblem};
use merman_core::{Engine, Error, ParseDiagnosticSpanKind, ParseOptions};

/// The longest parser message a finding repeats. merman's messages are
/// short, but one kind quotes the whole diagram.
const MESSAGE_LIMIT: usize = 200;

/// Parses Mermaid with merman. D2 is not checked: driftmark has no D2
/// parser, and the `d2` program may not be installed.
#[derive(Default)]
pub struct MermaidCheck {
    /// Made on the first Mermaid block, so files without diagrams do not
    /// pay its few milliseconds.
    engine: OnceLock<Engine>,
}

impl MermaidCheck {
    /// A check that makes its parser when it first needs it.
    #[must_use]
    pub fn new() -> MermaidCheck {
        MermaidCheck::default()
    }
}

impl DiagramCheck for MermaidCheck {
    fn check(&self, language: DiagramLanguage, code: &str) -> Result<(), DiagramProblem> {
        if language != DiagramLanguage::Mermaid {
            return Ok(());
        }
        let engine = self.engine.get_or_init(Engine::new);
        // A parser panic must not stop a lint run over a whole repo.
        let parsed = catch_unwind(AssertUnwindSafe(|| {
            engine.parse_diagram_sync(code, ParseOptions::strict())
        }));
        match parsed {
            Ok(Ok(Some(_))) => Ok(()),
            Ok(Ok(None) | Err(Error::DetectType(_))) => Err(DiagramProblem {
                message: "its first line names no diagram type; start it with one, such as `flowchart TD` or `sequenceDiagram`".to_owned(),
                offset: Some(0),
            }),
            Ok(Err(Error::DiagramParse { diagnostic, .. })) => Err(DiagramProblem {
                message: one_line(diagnostic.message()),
                offset: diagnostic
                    .span()
                    .filter(|_| diagnostic.span_kind() != ParseDiagnosticSpanKind::Fallback)
                    .map(|span| span.start),
            }),
            Ok(Err(other)) => Err(DiagramProblem {
                message: one_line(&other.to_string()),
                offset: None,
            }),
            Err(_) => Err(DiagramProblem {
                message: "the Mermaid parser stopped on it".to_owned(),
                offset: None,
            }),
        }
    }
}

/// A parser message as one short line.
fn one_line(message: &str) -> String {
    let words: Vec<&str> = message.split_whitespace().collect();
    let joined = words.join(" ");
    match joined.char_indices().nth(MESSAGE_LIMIT) {
        Some((cut, _)) => format!("{}…", &joined[..cut]),
        None => joined,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(code: &str) -> Result<(), DiagramProblem> {
        MermaidCheck::new().check(DiagramLanguage::Mermaid, code)
    }

    #[test]
    fn valid_diagrams_pass_and_d2_is_not_checked() {
        assert_eq!(check("flowchart TD\n  A --> B"), Ok(()));
        assert_eq!(check("sequenceDiagram\n  Alice->>Bob: hi"), Ok(()));
        assert_eq!(check("erDiagram\n  A ||--o{ B : has"), Ok(()));
        let d2 = MermaidCheck::new().check(DiagramLanguage::D2, "not d2 at all {{{");
        assert_eq!(d2, Ok(()));
    }

    #[test]
    fn parse_errors_carry_their_position() {
        let problem = check("flowchart TD\n  A[open --> B").expect_err("unterminated label");
        assert!(
            problem.message.contains("missing `]`"),
            "{}",
            problem.message
        );
        assert_eq!(problem.offset, Some(16));
    }

    #[test]
    fn a_missing_diagram_type_is_explained_without_quoting_the_code() {
        let problem = check("nonsense here\nand more").expect_err("no type");
        assert!(
            problem
                .message
                .starts_with("its first line names no diagram type")
        );
        assert!(!problem.message.contains("nonsense"));
        assert_eq!(problem.offset, Some(0));
    }

    #[test]
    fn errors_without_a_place_have_no_offset() {
        let problem = check("pie\n \"a\" : x").expect_err("bad pie value");
        assert_eq!(problem.offset, None);
        let yaml = check("---\ntitle: [x\n---\nflowchart TD\n A-->B").expect_err("bad YAML");
        assert!(yaml.message.contains("YAML"), "{}", yaml.message);
        assert_eq!(yaml.offset, None);
    }

    #[test]
    fn long_messages_are_one_short_line() {
        assert_eq!(one_line(" a\n  b\tc "), "a b c");
        let long = "x ".repeat(300);
        let short = one_line(&long);
        assert_eq!(short.chars().count(), MESSAGE_LIMIT + 1);
        assert!(short.ends_with('…'));
    }
}
