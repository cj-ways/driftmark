//! Helpers for the lint tests, and tests of the lint's public types.

use std::sync::Arc;

use super::{
    Anchors, DiagramCheck, DiagramProblem, Finding, LinkTargets, Linter, Report, Rule, Severity,
};
use crate::document::{DiagramLanguage, Document, ParseError};
use crate::rules::{Rules, RulesError};

/// A made-up repository: a list of files and their text.
struct Files<'f>(&'f [(&'f str, &'f str)]);

impl LinkTargets for Files<'_> {
    fn exists(&self, path: &str) -> bool {
        self.0.iter().any(|(file, _)| {
            *file == path
                || file
                    .strip_prefix(path)
                    .is_some_and(|rest| rest.starts_with('/'))
        })
    }

    fn anchors(&self, path: &str) -> Option<Arc<Anchors>> {
        let (_, text) = self.0.iter().find(|(file, _)| *file == path)?;
        if *text == "UNREADABLE" {
            return None;
        }
        let document = Document::parse(text.as_bytes().to_vec()).expect("test files parse");
        Some(Arc::new(Anchors::of(&document)))
    }
}

/// A diagram parser for tests. It refuses code holding the word `broken`,
/// at that word; code holding `vague`, with no position; and code holding
/// ` at N`, at byte N.
struct FakeDiagrams;

impl DiagramCheck for FakeDiagrams {
    fn check(&self, language: DiagramLanguage, code: &str) -> Result<(), DiagramProblem> {
        let _ = language;
        if let Some(offset) = code.find("broken") {
            return Err(DiagramProblem {
                message: "it says broken".to_owned(),
                offset: Some(offset),
            });
        }
        if code.contains("vague") {
            return Err(DiagramProblem {
                message: "it is vague".to_owned(),
                offset: None,
            });
        }
        if let Some((_, number)) = code.rsplit_once(" at ") {
            return Err(DiagramProblem {
                message: "it ends badly".to_owned(),
                offset: number.trim().parse().ok(),
            });
        }
        Ok(())
    }
}

/// Lints `source` as the file at `path` with a rules file and a made-up
/// repository.
pub(super) fn lint_in_with(
    rules: &str,
    files: &[(&str, &str)],
    path: &str,
    source: &str,
) -> Vec<Finding> {
    let rules = Rules::parse(rules).expect("the test rules file is valid");
    let linter = Linter {
        rules: &rules,
        targets: &Files(files),
        diagrams: &FakeDiagrams,
    };
    linter.lint(path, source.as_bytes().to_vec())
}

/// Lints `source` with the default rules in a made-up repository.
pub(super) fn lint_in(files: &[(&str, &str)], path: &str, source: &str) -> Vec<Finding> {
    lint_in_with("", files, path, source)
}

/// Lints `source` with a rules file in an empty repository.
pub(super) fn lint_with(rules: &str, path: &str, source: &str) -> Vec<Finding> {
    lint_in_with(rules, &[], path, source)
}

/// Lints `source` with the default rules in an empty repository.
pub(super) fn lint(path: &str, source: &str) -> Vec<Finding> {
    lint_with("", path, source)
}

/// Each finding as `line:column severity rule: message`.
pub(super) fn messages(findings: &[Finding]) -> Vec<String> {
    findings
        .iter()
        .map(|finding| {
            format!(
                "{}:{} {} {}: {}",
                finding.line, finding.column, finding.severity, finding.rule, finding.message
            )
        })
        .collect()
}

#[test]
fn every_rule_has_a_unique_id_that_reads_back() {
    for rule in Rule::ALL {
        assert_eq!(Rule::from_id(rule.id()), Some(rule));
        assert_eq!(rule.to_string(), rule.id());
        let parsed = Rules::parse(&format!("[lint.rules]\n{} = \"off\"\n", rule.id()))
            .expect("every id is a key of [lint.rules]");
        assert_eq!(parsed.lint().severity(rule), None);
    }
    assert_eq!(Rule::from_id("nope"), None);
}

#[test]
fn default_severities_split_errors_from_warnings() {
    let errors: Vec<&str> = Rule::ALL
        .into_iter()
        .filter(|rule| rule.default_severity() == Severity::Error)
        .map(Rule::id)
        .collect();
    assert_eq!(
        errors,
        [
            "parse",
            "title",
            "code-closed",
            "diagram",
            "front-matter",
            "required-field",
            "link",
            "anchor"
        ]
    );
    assert_eq!(Severity::Error.to_string(), "error");
    assert_eq!(Severity::Warning.name(), "warning");
}

#[test]
fn the_lint_table_changes_severities_and_limits() {
    let rules = Rules::parse(
        "[lint]\nmax-table-columns = 3\nmax-code-line-length = 4\nmax-file-size = 5\nmax-heading-length = 6\nallowed-html = [\"Img\"]\n\n[lint.rules]\ncode-language = \"error\"\ntitle = \"warning\"\nhtml = \"off\"\n",
    )
    .expect("valid");
    let lint = rules.lint();
    assert_eq!(lint.severity(Rule::CodeLanguage), Some(Severity::Error));
    assert_eq!(lint.severity(Rule::Title), Some(Severity::Warning));
    assert_eq!(lint.severity(Rule::Html), None);
    assert_eq!(lint.severity(Rule::Link), Some(Severity::Error));
    assert_eq!(lint.max_table_columns, 3);
    assert_eq!(lint.max_code_line_length, 4);
    assert_eq!(lint.max_file_size, 5);
    assert_eq!(lint.max_heading_length, 6);
    assert_eq!(lint.allowed_html, ["img"]);
    let defaults = Rules::default();
    assert_eq!(
        defaults.lint().max_table_columns,
        super::DEFAULT_MAX_TABLE_COLUMNS
    );
    assert_eq!(
        defaults.lint().max_code_line_length,
        super::DEFAULT_MAX_CODE_LINE_LENGTH
    );
    assert_eq!(defaults.lint().max_file_size, super::DEFAULT_MAX_FILE_SIZE);
    assert_eq!(
        defaults.lint().max_heading_length,
        super::DEFAULT_MAX_HEADING_LENGTH
    );
    assert_eq!(defaults.lint().allowed_html, super::DEFAULT_ALLOWED_HTML);
    assert_eq!(Rules::parse("[lint]\n").expect("valid"), Rules::default());
}

#[test]
fn a_changed_severity_shows_in_the_findings() {
    let found = lint_with(
        "[lint.rules]\ntitle = \"warning\"\nsummary = \"error\"\n",
        "t.md",
        "Text.\n",
    );
    assert_eq!(
        found
            .iter()
            .map(|finding| (finding.rule, finding.severity))
            .collect::<Vec<_>>(),
        [(Rule::Title, Severity::Warning)]
    );
    let found = lint_with("[lint.rules]\nsummary = \"error\"\n", "t.md", "# T\n");
    assert_eq!(found[0].severity, Severity::Error);
}

#[test]
fn a_wrong_lint_table_is_refused_with_its_line() {
    let syntax = |text: &str| {
        let message = Rules::parse(text).expect_err("refused").to_string();
        assert!(
            message.starts_with("the rules file could not be read: "),
            "{message}"
        );
        message
    };
    let unknown = syntax("[lint.rules]\ntitel = \"off\"\n");
    assert!(unknown.contains("line 2"), "{unknown}");
    assert!(unknown.contains("titel"), "{unknown}");
    assert!(unknown.contains("heading-levels"), "{unknown}");
    assert!(syntax("[lint.rules]\ntitle = \"loud\"\n").contains("loud"));
    assert!(syntax("[lint]\nmax-file-size = 0\n").contains("line 2"));
    assert!(syntax("[lint]\nmax-files = 3\n").contains("max-files"));
    let error =
        Rules::parse("[lint]\nallowed-html = [\"details\", \"<b>\"]\n").expect_err("invalid");
    assert_eq!(
        error,
        RulesError::Lint(
            "`allowed-html` holds \"<b>\", which is not an HTML element name; write names such as \"details\", without `<` or `>`".to_owned()
        )
    );
    assert_eq!(
        error.to_string(),
        "the rules file's [lint] table: `allowed-html` holds \"<b>\", which is not an HTML element name; write names such as \"details\", without `<` or `>`"
    );
    assert!(Rules::parse("[lint]\nallowed-html = [\"1b\"]\n").is_err());
    assert!(Rules::parse("[lint]\nallowed-html = [\"my-el2\"]\n").is_ok());
}

#[test]
fn exclude_patterns_name_the_files_a_walk_skips() {
    let rules =
        Rules::parse("[lint]\nexclude = [\"fixtures/**\", \"CLAUDE.md\", \"docs/*/draft-*.md\"]\n")
            .expect("valid");
    let lint = rules.lint();
    assert_eq!(
        lint.exclude,
        ["fixtures/**", "CLAUDE.md", "docs/*/draft-*.md"]
    );
    for path in [
        "fixtures/lint/title/fail.md",
        "fixtures/a.md",
        "CLAUDE.md",
        "docs/x/draft-1.md",
    ] {
        assert!(lint.excludes(path), "{path}");
    }
    for path in [
        "docs/CLAUDE.md",
        "claude.md",
        "docs/x/y/draft-1.md",
        "fixtures.md",
        "README.md",
    ] {
        assert!(!lint.excludes(path), "{path}");
    }
    assert!(!Rules::default().lint().excludes("CLAUDE.md"));
    let error = Rules::parse("[lint]\nexclude = [\"a/**b\"]\n").expect_err("invalid");
    assert_eq!(
        error,
        RulesError::Lint(
            "`exclude` holds \"a/**b\", which is not a valid pattern: Pattern syntax error near position 4: recursive wildcards must form a single path component".to_owned()
        )
    );
}

#[test]
fn a_file_that_is_not_markdown_gets_one_parse_finding() {
    let found = Linter {
        rules: &Rules::default(),
        targets: &Files(&[]),
        diagrams: &FakeDiagrams,
    }
    .lint("t.md", b"# Title\n\nbad \xff\n".to_vec());
    assert_eq!(
        messages(&found),
        [
            "3:1 error parse: the file is not UTF-8 text: byte 13, on line 3, is not valid UTF-8; save the file as UTF-8"
        ]
    );
    let deep = format!(
        "# T\n\nS.\n\n{} x\n",
        ">".repeat(crate::document::MAX_DEPTH + 1)
    );
    assert_eq!(
        messages(&lint("t.md", &deep)),
        [
            "5:1 error parse: on line 5, block quotes, lists, alerts and footnotes nest more than 100 levels deep; nest block quotes, lists, alerts and footnotes less deeply"
        ]
    );
}

#[test]
fn parser_failures_ask_for_a_bug_report() {
    let settings = super::LintSettings::default();
    let mut report = Report::new(&settings);
    report.parse_error(&ParseError::BadPosition { line: 4, column: 2 });
    report.parse_error(&ParseError::Internal {
        message: "boom".to_owned(),
    });
    assert_eq!(
        messages(&report.finish()),
        [
            "1:1 error parse: the Markdown parser stopped on this file: boom; this is a driftmark bug; please report it with the file",
            "4:1 error parse: the Markdown parser gave a position outside the file (line 4, column 2); this is a bug in the parser; this is a driftmark bug; please report it with the file",
        ]
    );
}

#[test]
fn findings_are_sorted_by_line_column_and_rule() {
    let found = lint(
        "t.md",
        "Intro <span>x</span>.\n\n# T\n\n```\nx\n```\n\n### Skip\n",
    );
    let order: Vec<(usize, usize, &str)> = found
        .iter()
        .map(|finding| (finding.line, finding.column, finding.rule.id()))
        .collect();
    assert_eq!(
        order,
        [
            (1, 1, "title"),
            (1, 7, "html"),
            (5, 1, "code-language"),
            (9, 1, "summary"),
            (9, 1, "heading-levels"),
        ]
    );
}

#[test]
fn the_fakes_answer_as_documented() {
    let files = Files(&[("a/b.md", "# B\n"), ("bad.md", "UNREADABLE")]);
    assert!(files.exists("a"));
    assert!(files.exists("a/b.md"));
    assert!(!files.exists("a/b"));
    assert!(
        files
            .anchors("a/b.md")
            .is_some_and(|anchors| anchors.contains("b"))
    );
    assert!(files.anchors("bad.md").is_none());
    assert!(files.anchors("c.md").is_none());
    assert_eq!(FakeDiagrams.check(DiagramLanguage::D2, "fine"), Ok(()));
}
