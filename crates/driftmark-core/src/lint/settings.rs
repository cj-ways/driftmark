//! The `[lint]` table of the rules file: which rules are on and how serious
//! they are, the allowed HTML elements, and the four size limits.

use std::collections::BTreeMap;
use std::num::NonZeroUsize;

use serde::Deserialize;

use super::{Rule, Severity};

/// The HTML elements allowed when the rules file names none. They render the
/// same on GitHub, GitLab and driftmark, and Markdown has no way to write
/// them.
pub const DEFAULT_ALLOWED_HTML: [&str; 6] = ["details", "summary", "br", "sub", "sup", "kbd"];

/// The most columns a table may have by default. Wider tables do not fit a
/// phone screen. On a 3,400-file corpus, 1 table in 300 is wider.
pub const DEFAULT_MAX_TABLE_COLUMNS: usize = 8;

/// The most characters a code line may have by default, so code needs no
/// sideways scrolling on a laptop.
pub const DEFAULT_MAX_CODE_LINE_LENGTH: usize = 120;

/// The most bytes a file may have by default. An agent reads a 100 KB file
/// as about 25,000 tokens, and a reader no longer finds their way in it.
pub const DEFAULT_MAX_FILE_SIZE: usize = 100_000;

/// The most characters a heading may have by default. Longer headings are
/// sentences, which belong in the text under them.
pub const DEFAULT_MAX_HEADING_LENGTH: usize = 100;

/// The writing check's settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LintSettings {
    /// What `[lint.rules]` sets: a severity, or `None` for a rule turned off.
    /// Rules it does not name keep their default severity.
    levels: BTreeMap<Rule, Option<Severity>>,
    /// The HTML element names allowed in raw HTML, in lower case.
    pub allowed_html: Vec<String>,
    /// The most columns a table may have.
    pub max_table_columns: usize,
    /// The most characters a line inside a code block may have.
    pub max_code_line_length: usize,
    /// The most bytes a file may have.
    pub max_file_size: usize,
    /// The most characters a heading may have.
    pub max_heading_length: usize,
}

impl Default for LintSettings {
    fn default() -> LintSettings {
        LintSettings {
            levels: BTreeMap::new(),
            allowed_html: DEFAULT_ALLOWED_HTML.map(str::to_owned).to_vec(),
            max_table_columns: DEFAULT_MAX_TABLE_COLUMNS,
            max_code_line_length: DEFAULT_MAX_CODE_LINE_LENGTH,
            max_file_size: DEFAULT_MAX_FILE_SIZE,
            max_heading_length: DEFAULT_MAX_HEADING_LENGTH,
        }
    }
}

impl LintSettings {
    /// How serious a finding of `rule` is, or `None` when the rule is off.
    #[must_use]
    pub fn severity(&self, rule: Rule) -> Option<Severity> {
        self.levels
            .get(&rule)
            .copied()
            .unwrap_or(Some(rule.default_severity()))
    }

    /// Checks the `[lint]` table and fills in the defaults.
    pub(crate) fn from_raw(raw: RawLint) -> Result<LintSettings, String> {
        let defaults = LintSettings::default();
        let allowed_html = match raw.allowed_html {
            Some(names) => names
                .into_iter()
                .map(|name| {
                    let valid = name.starts_with(|c: char| c.is_ascii_alphabetic())
                        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
                    if valid {
                        Ok(name.to_ascii_lowercase())
                    } else {
                        Err(format!(
                            "`allowed-html` holds {name:?}, which is not an HTML element name; write names such as \"details\", without `<` or `>`"
                        ))
                    }
                })
                .collect::<Result<Vec<_>, _>>()?,
            None => defaults.allowed_html,
        };
        let limit =
            |value: Option<NonZeroUsize>, default: usize| value.map_or(default, NonZeroUsize::get);
        Ok(LintSettings {
            levels: raw
                .rules
                .into_iter()
                .map(|(rule, level)| (rule, level.severity()))
                .collect(),
            allowed_html,
            max_table_columns: limit(raw.max_table_columns, defaults.max_table_columns),
            max_code_line_length: limit(raw.max_code_line_length, defaults.max_code_line_length),
            max_file_size: limit(raw.max_file_size, defaults.max_file_size),
            max_heading_length: limit(raw.max_heading_length, defaults.max_heading_length),
        })
    }
}

/// The `[lint]` table as TOML holds it, before it is checked.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub(crate) struct RawLint {
    allowed_html: Option<Vec<String>>,
    max_table_columns: Option<NonZeroUsize>,
    max_code_line_length: Option<NonZeroUsize>,
    max_file_size: Option<NonZeroUsize>,
    max_heading_length: Option<NonZeroUsize>,
    #[serde(default)]
    rules: BTreeMap<Rule, Level>,
}

/// A value of `[lint.rules]`.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Level {
    Error,
    Warning,
    Off,
}

impl Level {
    fn severity(self) -> Option<Severity> {
        match self {
            Level::Error => Some(Severity::Error),
            Level::Warning => Some(Severity::Warning),
            Level::Off => None,
        }
    }
}
