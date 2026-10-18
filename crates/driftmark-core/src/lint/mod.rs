//! The writing check: rules that keep agent-written Markdown in the shape
//! driftmark renders well (decision 0002), each with a message that names the
//! fix.
//!
//! [`Linter::lint`] checks one file. It needs three things the core cannot
//! get by itself, because the core reads no files: the rules file, a
//! [`LinkTargets`] that says which files and headings exist in the
//! repository, and a [`DiagramCheck`] that parses diagram code. The program
//! supplies all three.
//!
//! `docs/features/lint.md` states every rule, its default severity and the
//! `[lint]` settings.

mod anchors;
mod code;
mod html;
mod links;
mod meta;
mod position;
mod settings;
mod structure;

use std::fmt;
use std::sync::Arc;

use comrak::nodes::AstNode;
use comrak::{Arena, parse_document};
use serde::Deserialize;

use crate::document::markdown::{Positions, options};
use crate::document::{DiagramLanguage, Document, ParseError};
use crate::rules::Rules;

pub use anchors::Anchors;
pub(crate) use settings::RawLint;
pub use settings::{
    DEFAULT_ALLOWED_HTML, DEFAULT_MAX_CODE_LINE_LENGTH, DEFAULT_MAX_FILE_SIZE,
    DEFAULT_MAX_HEADING_LENGTH, DEFAULT_MAX_TABLE_COLUMNS, LintSettings,
};

use position::Lines;

/// One writing rule. As a key of the `[lint.rules]` table it reads from its
/// id, so an unknown id is a TOML error that names the line and lists the
/// real ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Rule {
    /// The file could not be read as Markdown at all.
    Parse,
    /// Exactly one `#` title, first in the file after any front matter.
    Title,
    /// A summary paragraph or `>` quote before the first section.
    Summary,
    /// No skipped heading levels.
    HeadingLevels,
    /// A heading is at most the configured number of characters.
    HeadingLength,
    /// Every code block is closed.
    CodeClosed,
    /// Every code block names its language.
    CodeLanguage,
    /// A code line is at most the configured number of characters.
    CodeLineLength,
    /// Diagram blocks parse.
    Diagram,
    /// Raw HTML only from the allow-list.
    Html,
    /// The front matter is valid YAML.
    FrontMatter,
    /// The front matter holds every key the document's type requires.
    RequiredField,
    /// The status is one the document's type lists.
    StatusValue,
    /// Relative links point to files that exist.
    Link,
    /// `#anchors` point to headings that exist.
    Anchor,
    /// A table has at most the configured number of columns.
    TableColumns,
    /// A file is at most the configured number of bytes.
    FileSize,
}

impl Rule {
    /// Every rule, in the order the documentation lists them.
    pub const ALL: [Rule; 17] = [
        Rule::Parse,
        Rule::Title,
        Rule::Summary,
        Rule::HeadingLevels,
        Rule::HeadingLength,
        Rule::CodeClosed,
        Rule::CodeLanguage,
        Rule::CodeLineLength,
        Rule::Diagram,
        Rule::Html,
        Rule::FrontMatter,
        Rule::RequiredField,
        Rule::StatusValue,
        Rule::Link,
        Rule::Anchor,
        Rule::TableColumns,
        Rule::FileSize,
    ];

    /// The rule's short id, as findings and the `[lint.rules]` table write
    /// it.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Rule::Parse => "parse",
            Rule::Title => "title",
            Rule::Summary => "summary",
            Rule::HeadingLevels => "heading-levels",
            Rule::HeadingLength => "heading-length",
            Rule::CodeClosed => "code-closed",
            Rule::CodeLanguage => "code-language",
            Rule::CodeLineLength => "code-line-length",
            Rule::Diagram => "diagram",
            Rule::Html => "html",
            Rule::FrontMatter => "front-matter",
            Rule::RequiredField => "required-field",
            Rule::StatusValue => "status-value",
            Rule::Link => "link",
            Rule::Anchor => "anchor",
            Rule::TableColumns => "table-columns",
            Rule::FileSize => "file-size",
        }
    }

    /// The rule with this id, if there is one.
    #[must_use]
    pub fn from_id(id: &str) -> Option<Rule> {
        Rule::ALL.into_iter().find(|rule| rule.id() == id)
    }

    /// How serious a finding of this rule is when the rules file does not
    /// say otherwise. Errors break how the page renders or what driftmark
    /// knows about the document; warnings are about shape.
    #[must_use]
    pub fn default_severity(self) -> Severity {
        match self {
            Rule::Parse
            | Rule::Title
            | Rule::CodeClosed
            | Rule::Diagram
            | Rule::FrontMatter
            | Rule::RequiredField
            | Rule::Link
            | Rule::Anchor => Severity::Error,
            Rule::Summary
            | Rule::HeadingLevels
            | Rule::HeadingLength
            | Rule::CodeLanguage
            | Rule::CodeLineLength
            | Rule::Html
            | Rule::StatusValue
            | Rule::TableColumns
            | Rule::FileSize => Severity::Warning,
        }
    }
}

impl fmt::Display for Rule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

/// How serious a finding is. Any error makes `driftmark lint` exit with 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// Must be fixed: the page renders wrong, or driftmark misreads the
    /// document.
    Error,
    /// Should be fixed: the document works but reads worse.
    Warning,
}

impl Severity {
    /// `"error"` or `"warning"`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// One problem in one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// The rule that found it.
    pub rule: Rule,
    /// How serious it is, after the rules file's `[lint.rules]` table.
    pub severity: Severity,
    /// The 1-based line it is on.
    pub line: usize,
    /// The 1-based column, counted in characters, so an editor can jump to
    /// it.
    pub column: usize,
    /// What is wrong and how to fix it, as one sentence.
    pub message: String,
}

/// What the program tells the lint about the rest of the repository, so it
/// can check links. Paths are relative to the repository root, with `/`
/// between folders, as git writes them.
pub trait LinkTargets: Sync {
    /// Whether a file or folder exists at `path`. Names must match exactly,
    /// case included, as they do on GitHub.
    fn exists(&self, path: &str) -> bool;

    /// The heading anchors of the Markdown file at `path`, or `None` when
    /// it is not a Markdown file driftmark can read. The program builds them
    /// with [`Anchors::of`].
    fn anchors(&self, path: &str) -> Option<Arc<Anchors>>;
}

/// Parses diagram code, so the lint can report a diagram that will not draw.
/// The program implements it with merman's Mermaid parser, which keeps the
/// core free of merman.
pub trait DiagramCheck: Sync {
    /// Checks the code of one diagram block: the lines between its fences.
    ///
    /// # Errors
    ///
    /// A [`DiagramProblem`] when the code does not parse.
    fn check(&self, language: DiagramLanguage, code: &str) -> Result<(), DiagramProblem>;
}

/// Why a diagram does not parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagramProblem {
    /// The parser's explanation, as one line.
    pub message: String,
    /// The byte offset in the code where the problem is, when the parser
    /// knows it.
    pub offset: Option<usize>,
}

/// Checks Markdown files against the rules file's writing rules.
pub struct Linter<'a> {
    /// The rules file: document types, and the `[lint]` settings.
    pub rules: &'a Rules,
    /// Which files and headings exist, for link checks.
    pub targets: &'a dyn LinkTargets,
    /// The diagram parser.
    pub diagrams: &'a dyn DiagramCheck,
}

impl Linter<'_> {
    /// Checks one file. `path` is the file's place in the repository,
    /// relative to its root with `/` between folders; it decides the
    /// document's type and where relative links lead. The findings come in
    /// order of line, then column, then rule.
    #[must_use]
    pub fn lint(&self, path: &str, bytes: Vec<u8>) -> Vec<Finding> {
        let size = bytes.len();
        let mut report = Report::new(self.rules.lint());
        match Document::parse(bytes) {
            Ok(document) => self.check(path, size, &document, &mut report),
            Err(error) => report.parse_error(&error),
        }
        report.finish()
    }

    fn check(&self, path: &str, size: usize, document: &Document, report: &mut Report<'_>) {
        let arena = Arena::new();
        let root = parse_document(&arena, &document.source, &options());
        let context = Context {
            path,
            size,
            document,
            root,
            positions: Positions::new(&document.source),
            lines: Lines::new(&document.source),
            rules: self.rules,
            settings: self.rules.lint(),
            targets: self.targets,
            diagrams: self.diagrams,
        };
        structure::check(&context, report);
        code::check(&context, report);
        html::check(&context, report);
        meta::check(&context, report);
        links::check(&context, report);
    }
}

/// Everything a rule may look at for one file.
struct Context<'a> {
    path: &'a str,
    size: usize,
    document: &'a Document,
    /// The comrak tree of the same text, for inline content the block list
    /// does not keep: inline HTML and links.
    root: &'a AstNode<'a>,
    positions: Positions<'a>,
    lines: Lines<'a>,
    rules: &'a Rules,
    settings: &'a LintSettings,
    targets: &'a dyn LinkTargets,
    diagrams: &'a dyn DiagramCheck,
}

/// The findings of one file, collected as the rules run.
struct Report<'s> {
    settings: &'s LintSettings,
    findings: Vec<Finding>,
}

impl<'s> Report<'s> {
    fn new(settings: &'s LintSettings) -> Report<'s> {
        Report {
            settings,
            findings: Vec::new(),
        }
    }

    /// Whether `rule` is on. Rules that are off are not run at all, so a
    /// slow check such as the diagram parser costs nothing when turned off.
    fn wants(&self, rule: Rule) -> bool {
        self.settings.severity(rule).is_some()
    }

    /// Records a finding at a line and column, unless the rule is off.
    fn add(&mut self, rule: Rule, (line, column): (usize, usize), message: String) {
        if let Some(severity) = self.settings.severity(rule) {
            self.findings.push(Finding {
                rule,
                severity,
                line,
                column,
                message,
            });
        }
    }

    fn parse_error(&mut self, error: &ParseError) {
        let (line, fix) = match error {
            ParseError::NotUtf8 { line, .. } => (*line, "save the file as UTF-8"),
            ParseError::TooDeep { line, .. } => (
                *line,
                "nest block quotes, lists, alerts and footnotes less deeply",
            ),
            ParseError::BadPosition { line, .. } => (
                *line,
                "this is a driftmark bug; please report it with the file",
            ),
            ParseError::Internal { .. } => {
                (1, "this is a driftmark bug; please report it with the file")
            }
        };
        self.add(Rule::Parse, (line, 1), format!("{error}; {fix}"));
    }

    fn finish(mut self) -> Vec<Finding> {
        self.findings
            .sort_by_key(|finding| (finding.line, finding.column, finding.rule));
        self.findings
    }
}

#[cfg(test)]
mod tests;
