//! The rules file, `driftmark.toml`: which paths hold which document type,
//! where each type writes its status, date and "superseded by", what its
//! status values mean, and which of its headings mark decisions.
//!
//! The format is stated in `docs/features/engine/rules-file.md`. The program
//! reads the file; this module only parses its text, so the core stays free of
//! file access. A preset is a rules file in this same format, built into
//! driftmark ([`crate::presets`]), and `presets = [...]` names the ones a
//! repository uses. The file's `[lint]` table holds the writing check's
//! settings; [`crate::lint`] reads it. Its `[masthead]` table adds masthead
//! keys to the known ones, and its `[compare]` table names the branch `auto`
//! and the home page compare with.

use std::fmt;

use glob::{MatchOptions, Pattern};
use serde::Deserialize;

use crate::document::{
    BlockKind, Document, FieldOrigin, MAX_FILE_SIZE, MastheadKeys, MetaValue, normalize_key,
};
use crate::lint::{LintSettings, RawLint};

/// The type of a document that no rule matches.
pub const DEFAULT_TYPE: &str = "document";

/// How path patterns match: case matters, `*` and `?` never cross a `/`, and
/// `*` matches names that start with a dot. The lint's `exclude` patterns
/// match the same way.
pub(crate) const MATCHING: MatchOptions = MatchOptions {
    case_sensitive: true,
    require_literal_separator: true,
    require_literal_leading_dot: false,
};

/// The other files of a repository, as the rules need them: a file beside a
/// document that a location names, such as cc-sdd's `spec.json`, and the
/// marker files and folders a preset is found by.
///
/// The program answers for one side of a comparison, a commit or the working
/// tree, so the core reads no files itself. Paths are relative to the
/// repository root, with `/` between folders, as git writes them.
pub trait RepoFiles {
    /// The bytes of the regular file at `path`, or `None` when there is
    /// none.
    fn read(&self, path: &str) -> Option<Vec<u8>>;

    /// The Markdown files inside these folders, at any depth, in any order.
    /// A folder that is not there adds nothing. An empty folder name means
    /// the whole repository.
    fn markdown_in(&self, folders: &[&str]) -> Vec<String>;
}

/// A parsed rules file: an ordered list of document types, the writing
/// check's settings, and the masthead keys documents are read with.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rules {
    types: Vec<TypeRule>,
    lint: LintSettings,
    masthead_keys: MastheadKeys,
    /// The keys added to the built-in ones, as written, so the keys of a
    /// preset can join them.
    masthead_extra: Vec<String>,
    default_branch: Option<String>,
    presets: Vec<String>,
}

/// One `[[type]]` entry of a rules file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeRule {
    /// The type's name, such as `decision`.
    pub name: String,
    /// The path patterns, as written.
    pub paths: Vec<String>,
    patterns: Vec<Pattern>,
    /// Where documents of this type write their status.
    pub status: Option<Location>,
    /// Status values that mean the document is current.
    pub current: Vec<String>,
    /// Status values that mean another document replaced this one.
    pub superseded: Vec<String>,
    /// Status values that mean the document is kept only as a record.
    pub archived: Vec<String>,
    /// The lifecycle of a document of this type whose status does not say
    /// otherwise.
    pub lifecycle: Option<Lifecycle>,
    /// Where documents of this type write their date.
    pub date: Option<Location>,
    /// Where documents of this type name the document that replaced them.
    pub superseded_by: Option<Location>,
    /// Heading titles whose sections hold decisions, such as `Decision`.
    pub decision_headings: Vec<String>,
    /// Front matter keys every document of this type must have, with a value.
    /// The writing check reports the ones a document lacks.
    pub required: Vec<String>,
    /// Status values that are allowed but mean none of the three lifecycles,
    /// such as `proposed`. With the three lists they make up every status the
    /// writing check accepts.
    pub other_statuses: Vec<String>,
}

/// Where a value is written in a document. When more than one place is
/// given, they are tried in this order: front matter, masthead, section,
/// file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Location {
    /// A key of the YAML front matter.
    pub front_matter: Option<String>,
    /// A key of the masthead, the `Key: Value` lines under the title.
    pub masthead: Option<String>,
    /// The title of a section whose first line of text is the value, as in
    /// adr-tools' `## Status` section.
    pub section: Option<String>,
    /// The name of a JSON or YAML file beside the document, for layouts that
    /// keep a value outside the Markdown, as cc-sdd keeps a spec's phase in
    /// `spec.json`. The nearest such file counts: in the document's folder,
    /// or else in the closest folder above it.
    pub file: Option<String>,
    /// The key of the value in that file, with `.` between the keys of
    /// nested tables, as in `approvals.requirements.approved`.
    pub key: Option<String>,
}

/// Which of a type's status lists a value is in.
#[derive(Clone, Copy)]
enum StatusList {
    /// `current`, `superseded` or `archived`.
    Lifecycle(Lifecycle),
    /// `other-statuses`.
    Other,
}

/// A value read from a document through a [`Location`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located {
    /// The value as written, trimmed.
    pub text: String,
    /// The targets of the links in the value, when it came from the
    /// masthead.
    pub links: Vec<String>,
    /// Which of the location's places held it.
    pub place: Place,
}

/// One of the three places a [`Location`] can name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// A key of the YAML front matter.
    FrontMatter,
    /// A key of the masthead.
    Masthead,
    /// The first line of text under a section heading.
    Section,
    /// A key of a JSON or YAML file beside the document.
    File,
}

/// What a document's status means for the status view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lifecycle {
    /// The document says what is true now.
    Current,
    /// Another document replaced this one.
    Superseded,
    /// The document is kept only as a record.
    Archived,
}

/// What the rules say about one document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classification {
    /// The type's name, or [`DEFAULT_TYPE`] when no rule matches the path.
    pub doc_type: String,
    /// The status as written, when the type says where it lives and the
    /// document has one.
    pub status: Option<String>,
    /// What the status means, or the type's default lifecycle when the status
    /// is missing or not in any list.
    pub lifecycle: Option<Lifecycle>,
    /// The date as written.
    pub date: Option<String>,
    /// The text naming the document that replaced this one.
    pub superseded_by: Option<String>,
    /// The targets of the links in the "superseded by" value, when it was
    /// read from the masthead.
    pub superseded_by_links: Vec<String>,
    /// The indexes of the blocks in decision sections: decision headings and
    /// every block under them.
    pub decision_blocks: Vec<usize>,
}

/// Why a rules file could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RulesError {
    /// The text is not valid TOML, or a key or value has the wrong name or
    /// shape. The message comes from the TOML reader and names the line.
    Syntax(String),
    /// One type entry is valid TOML but says something impossible.
    Invalid {
        /// The entry's position among the `[[type]]` entries, from 1.
        number: usize,
        /// The entry's name, as written.
        name: String,
        /// What is wrong, as a sentence.
        problem: String,
    },
    /// The `[lint]` table is valid TOML but says something impossible, such
    /// as an HTML element name with a space in it.
    Lint(String),
}

impl fmt::Display for RulesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RulesError::Syntax(message) => write!(f, "the rules file could not be read: {message}"),
            RulesError::Invalid {
                number,
                name,
                problem,
            } => write!(f, "the rules file's type {number} ({name:?}): {problem}"),
            RulesError::Lint(problem) => write!(f, "the rules file's [lint] table: {problem}"),
        }
    }
}

impl std::error::Error for RulesError {}

/// The file as TOML holds it, before it is checked.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRules {
    #[serde(default)]
    presets: Vec<PresetName>,
    #[serde(default, rename = "type")]
    types: Vec<RawType>,
    #[serde(default)]
    lint: RawLint,
    #[serde(default)]
    masthead: RawMasthead,
    #[serde(default)]
    compare: RawCompare,
}

/// The name of a built-in preset. A name driftmark does not know is refused
/// here, so the TOML reader names its line and the user sees the known names.
#[derive(Deserialize)]
#[serde(try_from = "String")]
struct PresetName(String);

impl TryFrom<String> for PresetName {
    type Error = String;

    fn try_from(name: String) -> Result<PresetName, String> {
        if crate::presets::find(&name).is_some() {
            Ok(PresetName(name))
        } else {
            let known: Vec<String> = crate::presets::PRESETS
                .iter()
                .map(|preset| format!("\"{}\"", preset.name))
                .collect();
            Err(format!(
                "driftmark has no preset {name:?}; the presets are {}",
                known.join(", ")
            ))
        }
    }
}

/// The `[compare]` table: what changes are compared with.
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct RawCompare {
    default_branch: Option<BranchName>,
}

/// A branch name from the rules file. It must be text git could take as a
/// branch name, or comparing would fail later with a less helpful message;
/// the TOML reader then names its line.
#[derive(Deserialize)]
#[serde(try_from = "String")]
struct BranchName(String);

impl TryFrom<String> for BranchName {
    type Error = String;

    fn try_from(name: String) -> Result<BranchName, String> {
        let plain = !name.is_empty()
            && !name.starts_with('-')
            && !name.contains("..")
            && !name.chars().any(|c| c.is_whitespace() || c.is_control());
        if plain {
            Ok(BranchName(name))
        } else {
            Err(format!(
                "the default branch {name:?} cannot name a branch; write a branch name such as \"origin/main\""
            ))
        }
    }
}

/// The `[masthead]` table: keys to read as metadata beside the known ones.
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMasthead {
    #[serde(default)]
    keys: Vec<KeyName>,
}

/// A masthead key from the rules file. It must have the shape every masthead
/// key has, or no line could match it; the TOML reader then names its line.
#[derive(Deserialize)]
#[serde(try_from = "String")]
struct KeyName(String);

impl TryFrom<String> for KeyName {
    type Error = String;

    fn try_from(key: String) -> Result<KeyName, String> {
        if crate::document::meta::is_key(key.trim()) {
            Ok(KeyName(key))
        } else {
            Err(format!(
                "the masthead key {key:?} cannot match a masthead line; a key starts with a letter, has one to four words and at most 40 characters, and holds only letters, digits, spaces and -_/.'()"
            ))
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct RawType {
    name: String,
    paths: Vec<String>,
    status: Option<Location>,
    #[serde(default)]
    current: Vec<String>,
    #[serde(default)]
    superseded: Vec<String>,
    #[serde(default)]
    archived: Vec<String>,
    lifecycle: Option<Lifecycle>,
    date: Option<Location>,
    superseded_by: Option<Location>,
    #[serde(default)]
    decision_headings: Vec<String>,
    #[serde(default)]
    required: Vec<String>,
    #[serde(default)]
    other_statuses: Vec<String>,
}

impl Rules {
    /// Parses the text of a rules file.
    ///
    /// # Errors
    ///
    /// [`RulesError::Syntax`] when the text is not valid TOML or holds an
    /// unknown key, [`RulesError::Invalid`] when a type has no name, no
    /// paths, a pattern that does not parse, an empty location, status values
    /// without a status location, or one status value in two lists, and
    /// [`RulesError::Lint`] when the `[lint]` table names an impossible HTML
    /// element.
    pub fn parse(text: &str) -> Result<Rules, RulesError> {
        let raw: RawRules =
            toml::from_str(text).map_err(|error| RulesError::Syntax(error.to_string()))?;
        let types = raw
            .types
            .into_iter()
            .enumerate()
            .map(|(index, raw)| TypeRule::new(index + 1, raw))
            .collect::<Result<Vec<_>, _>>()?;
        let lint = LintSettings::from_raw(raw.lint).map_err(RulesError::Lint)?;
        let named = types
            .iter()
            .flat_map(|rule| [&rule.status, &rule.date, &rule.superseded_by])
            .flatten()
            .filter_map(|location| location.masthead.clone());
        let masthead_extra: Vec<String> = raw
            .masthead
            .keys
            .into_iter()
            .map(|KeyName(key)| key)
            .chain(named)
            .collect();
        let mut presets: Vec<String> = Vec::new();
        for PresetName(name) in raw.presets {
            if !presets.contains(&name) {
                presets.push(name);
            }
        }
        Ok(Rules {
            types,
            lint,
            masthead_keys: MastheadKeys::with(&masthead_extra),
            masthead_extra,
            default_branch: raw.compare.default_branch.map(|BranchName(name)| name),
            presets,
        })
    }

    /// The presets the file names in `presets = [...]`, each once, in the
    /// order written. Their types are not in these rules yet;
    /// [`crate::presets::Setup::build`] adds them after the file's own.
    #[must_use]
    pub fn presets(&self) -> &[String] {
        &self.presets
    }

    /// Adds a preset's types after these rules' own, and its masthead keys to
    /// theirs. The preset's `[lint]` and `[compare]` tables are not used: the
    /// writing check's settings and the default branch belong to the
    /// repository's own file.
    pub(crate) fn append(&mut self, preset: Rules) {
        self.types.extend(preset.types);
        self.masthead_extra.extend(preset.masthead_extra);
        self.masthead_keys = MastheadKeys::with(&self.masthead_extra);
    }

    /// The masthead keys documents should be parsed with
    /// ([`Document::parse_with`]): the known ones, those in the `[masthead]`
    /// table, and every key a type's `masthead` location names.
    #[must_use]
    pub fn masthead_keys(&self) -> &MastheadKeys {
        &self.masthead_keys
    }

    /// The branch `auto` and the home page compare with, from the
    /// `[compare]` table's `default-branch`, as written. `None` means the
    /// first that exists of [`crate::git::AUTO_BASES`].
    #[must_use]
    pub fn default_branch(&self) -> Option<&str> {
        self.default_branch.as_deref()
    }

    /// The writing check's settings from the `[lint]` table, or the defaults
    /// when the file has none.
    #[must_use]
    pub fn lint(&self) -> &LintSettings {
        &self.lint
    }

    /// The types, in the order the file lists them.
    #[must_use]
    pub fn types(&self) -> &[TypeRule] {
        &self.types
    }

    /// The first type whose patterns match `path`. `path` is relative to the
    /// repository root, with `/` between folders, as git writes it.
    #[must_use]
    pub fn type_for(&self, path: &str) -> Option<&TypeRule> {
        self.types.iter().find(|rule| rule.matches(path))
    }

    /// The type, status, date, "superseded by" and decision sections of the
    /// document at `path`. A location that names a file beside the document
    /// finds nothing here; [`Rules::classify_with`] reads such files.
    #[must_use]
    pub fn classify(&self, path: &str, document: &Document) -> Classification {
        self.classify_from(path, document, None)
    }

    /// As [`Rules::classify`], with the repository's other files, so a
    /// location can read a value from a file beside the document, such as
    /// cc-sdd's `spec.json`.
    #[must_use]
    pub fn classify_with(
        &self,
        path: &str,
        document: &Document,
        files: &dyn RepoFiles,
    ) -> Classification {
        self.classify_from(path, document, Some(files))
    }

    /// [`Rules::classify`] with the other files, when they are known.
    fn classify_from(
        &self,
        path: &str,
        document: &Document,
        files: Option<&dyn RepoFiles>,
    ) -> Classification {
        match self.type_for(path) {
            Some(rule) => rule.classify(path, document, files),
            None => Classification {
                doc_type: DEFAULT_TYPE.to_owned(),
                status: None,
                lifecycle: None,
                date: None,
                superseded_by: None,
                superseded_by_links: Vec::new(),
                decision_blocks: Vec::new(),
            },
        }
    }
}

impl TypeRule {
    fn new(number: usize, raw: RawType) -> Result<TypeRule, RulesError> {
        let invalid = |problem: String| RulesError::Invalid {
            number,
            name: raw.name.clone(),
            problem,
        };
        if raw.name.trim().is_empty() {
            return Err(invalid("its name is empty".to_owned()));
        }
        if raw.paths.is_empty() {
            return Err(invalid("it lists no paths".to_owned()));
        }
        let mut patterns = Vec::new();
        for path in &raw.paths {
            let pattern = Pattern::new(path).map_err(|error| {
                invalid(format!("the path {path:?} is not a valid pattern: {error}"))
            })?;
            patterns.push(pattern);
        }
        let locations = [
            ("status", &raw.status),
            ("date", &raw.date),
            ("superseded-by", &raw.superseded_by),
        ];
        for (key, location) in locations {
            if let Some(location) = location {
                location
                    .check()
                    .map_err(|problem| invalid(format!("`{key}` {problem}")))?;
            }
        }
        let lists = [
            &raw.current,
            &raw.superseded,
            &raw.archived,
            &raw.other_statuses,
        ];
        let mut seen = Vec::new();
        for value in lists.into_iter().flatten() {
            let normalized = normalize_status(value);
            if normalized.is_empty() {
                return Err(invalid(format!(
                    "the status value {value:?} has no letters or digits"
                )));
            }
            if seen.contains(&normalized) {
                return Err(invalid(format!(
                    "the status value {value:?} is in more than one place"
                )));
            }
            seen.push(normalized);
        }
        if raw.status.is_none() && !seen.is_empty() {
            return Err(invalid(
                "it lists status values but no `status` location".to_owned(),
            ));
        }
        if raw
            .decision_headings
            .iter()
            .any(|heading| heading.trim().is_empty())
        {
            return Err(invalid("a decision heading is empty".to_owned()));
        }
        if raw.required.iter().any(|key| key.trim().is_empty()) {
            return Err(invalid("a required key is empty".to_owned()));
        }
        Ok(TypeRule {
            name: raw.name,
            paths: raw.paths,
            patterns,
            status: raw.status,
            current: raw.current,
            superseded: raw.superseded,
            archived: raw.archived,
            lifecycle: raw.lifecycle,
            date: raw.date,
            superseded_by: raw.superseded_by,
            decision_headings: raw.decision_headings,
            required: raw.required,
            other_statuses: raw.other_statuses,
        })
    }

    /// Whether one of this type's patterns matches `path`.
    #[must_use]
    pub fn matches(&self, path: &str) -> bool {
        self.patterns
            .iter()
            .any(|pattern| pattern.matches_with(path, MATCHING))
    }

    /// What a status value means for this type, if one of its lists holds
    /// it.
    ///
    /// Case and leading symbols such as `✅ ` or `**` do not matter, and the
    /// status may go on after the listed value: `Superseded by 0009` matches
    /// `superseded`. When several listed values match, the longest wins.
    #[must_use]
    pub fn lifecycle_of(&self, status: &str) -> Option<Lifecycle> {
        match self.best_match(status)? {
            StatusList::Lifecycle(lifecycle) => Some(lifecycle),
            StatusList::Other => None,
        }
    }

    /// Whether one of this type's four status lists holds `status`, matched
    /// as in [`TypeRule::lifecycle_of`]. `other-statuses` counts too.
    #[must_use]
    pub fn allows_status(&self, status: &str) -> bool {
        self.best_match(status).is_some()
    }

    /// Every status value this type lists, in the order the file writes
    /// them: `current`, `superseded`, `archived`, then `other-statuses`.
    pub fn statuses(&self) -> impl Iterator<Item = &str> {
        [
            &self.current,
            &self.superseded,
            &self.archived,
            &self.other_statuses,
        ]
        .into_iter()
        .flatten()
        .map(String::as_str)
    }

    /// The list that holds the longest listed value matching `status`.
    fn best_match(&self, status: &str) -> Option<StatusList> {
        let status = normalize_status(status);
        [
            (StatusList::Lifecycle(Lifecycle::Current), &self.current),
            (
                StatusList::Lifecycle(Lifecycle::Superseded),
                &self.superseded,
            ),
            (StatusList::Lifecycle(Lifecycle::Archived), &self.archived),
            (StatusList::Other, &self.other_statuses),
        ]
        .into_iter()
        .flat_map(|(lifecycle, values)| {
            values
                .iter()
                .map(move |value| (lifecycle, normalize_status(value)))
        })
        .filter(|(_, value)| {
            status
                .strip_prefix(value.as_str())
                .is_some_and(|rest| !rest.starts_with(char::is_alphanumeric))
        })
        .max_by_key(|(_, value)| value.len())
        .map(|(lifecycle, _)| lifecycle)
    }

    /// Whether a heading with this title opens a decision section. Titles
    /// compare like keys: without regard to case, with `-`, `_` and spaces
    /// counting as the same.
    #[must_use]
    pub fn is_decision_heading(&self, title: &str) -> bool {
        let title = normalize_key(title);
        self.decision_headings
            .iter()
            .any(|heading| normalize_key(heading) == title)
    }

    fn classify(
        &self,
        path: &str,
        document: &Document,
        files: Option<&dyn RepoFiles>,
    ) -> Classification {
        let find = |location: &Option<Location>| {
            let location = location.as_ref()?;
            match files {
                Some(files) => location.find_in(path, document, files),
                None => location.find(document),
            }
        };
        let status = find(&self.status);
        let lifecycle = status
            .as_ref()
            .and_then(|status| self.lifecycle_of(&status.text))
            .or(self.lifecycle);
        let date = find(&self.date);
        let superseded_by = find(&self.superseded_by);
        let decision_blocks = document
            .blocks
            .iter()
            .enumerate()
            .filter(|(_, block)| {
                let own = top_level_heading(&block.kind, block.containers.is_empty()).is_some()
                    && self.is_decision_heading(&block.text);
                own || block
                    .heading_path
                    .iter()
                    .any(|title| self.is_decision_heading(title))
            })
            .map(|(index, _)| index)
            .collect();
        let (superseded_by, superseded_by_links) =
            superseded_by.map(|found| (found.text, found.links)).unzip();
        Classification {
            doc_type: self.name.clone(),
            status: status.map(|found| found.text),
            lifecycle,
            date: date.map(|found| found.text),
            superseded_by,
            superseded_by_links: superseded_by_links.unwrap_or_default(),
            decision_blocks,
        }
    }
}

impl Location {
    /// What is wrong with this location, to finish the sentence "`status` …".
    fn check(&self) -> Result<(), String> {
        let places = [
            &self.front_matter,
            &self.masthead,
            &self.section,
            &self.file,
            &self.key,
        ];
        if places.iter().all(|place| place.is_none()) {
            return Err("names no place to look".to_owned());
        }
        if places
            .iter()
            .any(|place| place.as_deref().is_some_and(|name| name.trim().is_empty()))
        {
            return Err("has an empty name".to_owned());
        }
        match (&self.file, &self.key) {
            (Some(_), None) => Err("names a file but no key to read in it".to_owned()),
            (None, Some(_)) => Err("names a key but no file to read it from".to_owned()),
            (Some(file), Some(_)) if file.contains(['/', '\\']) || file == "." || file == ".." => {
                Err(format!(
                    "names the file {file:?}; write a file name without folders, and driftmark looks for it beside the document and in the folders above"
                ))
            }
            (Some(_), Some(key)) if key.split('.').any(|part| part.trim().is_empty()) => Err(
                format!("has the key {key:?}, which has an empty part between its dots"),
            ),
            _ => Ok(()),
        }
    }

    /// The value at this location in `document`, with the targets of the
    /// links in it and the place that held it. The places are tried in the
    /// order front matter, masthead, section. Empty values count as missing.
    /// A file place finds nothing here; [`Location::find_in`] reads it.
    #[must_use]
    pub fn find(&self, document: &Document) -> Option<Located> {
        self.in_document(document)
    }

    /// As [`Location::find`], for the document at `path`, reading a file
    /// place from `files` when the document holds none of the other places.
    #[must_use]
    pub fn find_in(
        &self,
        path: &str,
        document: &Document,
        files: &dyn RepoFiles,
    ) -> Option<Located> {
        self.in_document(document).or_else(|| {
            let (file, key) = (self.file.as_deref()?, self.key.as_deref()?);
            let text = file_value(path, file, key, files)?;
            Some(Located {
                text,
                links: Vec::new(),
                place: Place::File,
            })
        })
    }

    /// The value in the document itself: front matter, masthead, section.
    fn in_document(&self, document: &Document) -> Option<Located> {
        let meta = &document.meta;
        let field = |origin: FieldOrigin, key: &Option<String>, place: Place| {
            key.as_deref()
                .and_then(|key| meta.field(origin, key))
                .and_then(|field| {
                    let text = field.value.as_text()?.trim();
                    (!text.is_empty()).then(|| Located {
                        text: text.to_owned(),
                        links: field.links.clone(),
                        place,
                    })
                })
        };
        field(
            FieldOrigin::FrontMatter,
            &self.front_matter,
            Place::FrontMatter,
        )
        .or_else(|| field(FieldOrigin::Masthead, &self.masthead, Place::Masthead))
        .or_else(|| {
            let title = self.section.as_deref()?;
            section_line(document, title).map(|text| Located {
                text,
                links: Vec::new(),
                place: Place::Section,
            })
        })
    }
}

/// The level of a heading outside any container; `None` for anything else.
fn top_level_heading(kind: &BlockKind, outside_containers: bool) -> Option<u8> {
    match kind {
        BlockKind::Heading { level } if outside_containers => Some(*level),
        _ => None,
    }
}

/// The first line of text in the section under the first heading titled
/// `title`, skipping deeper headings.
fn section_line(document: &Document, title: &str) -> Option<String> {
    let wanted = normalize_key(title);
    let mut blocks = document.blocks.iter().skip_while(|block| {
        top_level_heading(&block.kind, block.containers.is_empty()).is_none()
            || normalize_key(&block.text) != wanted
    });
    let heading = blocks.next()?;
    let level = top_level_heading(&heading.kind, true).unwrap_or(0);
    blocks
        .take_while(|block| {
            top_level_heading(&block.kind, block.containers.is_empty())
                .is_none_or(|deeper| deeper > level)
        })
        .filter(|block| !matches!(block.kind, BlockKind::Heading { .. }))
        .find_map(|block| {
            block
                .text
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .map(str::to_owned)
        })
}

/// The value at `key` in the nearest file named `name` beside the document at
/// `path`: in its folder, or else in the closest folder above it. The file is
/// JSON or YAML; JSON is read as YAML, which holds it. A value must be text:
/// a number or `true` reads as written, and a list or a table is no value.
fn file_value(path: &str, name: &str, key: &str, files: &dyn RepoFiles) -> Option<String> {
    let bytes = folders_above(path)
        .into_iter()
        .find_map(|folder| files.read(&join(folder, name)))?;
    if bytes.len() > MAX_FILE_SIZE {
        return None;
    }
    let text = std::str::from_utf8(&bytes).ok()?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut entries = crate::document::yaml::read(&untabbed(text)).ok()?;
    let (tables, last) = match key.rsplit_once('.') {
        Some((tables, last)) => (Some(tables), last),
        None => (None, key),
    };
    for part in tables.into_iter().flat_map(|tables| tables.split('.')) {
        entries = match entry(entries, part)? {
            MetaValue::Map(inner) => inner,
            MetaValue::Text(_) | MetaValue::List(_) => return None,
        };
    }
    let value = entry(entries, last)?;
    let text = value.as_text()?.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// The value of one key of a table, keys compared as front matter keys are.
fn entry(entries: Vec<(String, MetaValue)>, key: &str) -> Option<MetaValue> {
    let wanted = normalize_key(key);
    entries
        .into_iter()
        .find(|(name, _)| normalize_key(name) == wanted)
        .map(|(_, value)| value)
}

/// JSON with its indentation's tabs turned into spaces. YAML refuses a tab
/// that indents a line, and many JSON files are indented with tabs. A tab at
/// the start of a JSON line is never inside a string, because a JSON string
/// cannot hold a line break.
fn untabbed(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let body = line.trim_start_matches([' ', '\t']);
        let indent = line.len() - body.len();
        out.push_str(&" ".repeat(indent));
        out.push_str(body);
    }
    out
}

/// The folders from the one holding `path` up to the repository root, the
/// root written as the empty name.
fn folders_above(path: &str) -> Vec<&str> {
    let mut folders = Vec::new();
    let mut rest = path;
    while let Some((folder, _)) = rest.rsplit_once('/') {
        folders.push(folder);
        rest = folder;
    }
    folders.push("");
    folders
}

/// A file name inside a folder, the root written as the empty name.
fn join(folder: &str, name: &str) -> String {
    if folder.is_empty() {
        name.to_owned()
    } else {
        format!("{folder}/{name}")
    }
}

/// A status value in the form values are compared in: lower case, without
/// leading symbols or trailing spaces.
fn normalize_status(value: &str) -> String {
    value
        .trim_start_matches(|c: char| !c.is_alphanumeric())
        .trim_end()
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document(source: &str) -> Document {
        Document::parse(source.as_bytes().to_vec()).expect("the test document parses")
    }

    /// The problem a rules file is refused for, without the type prefix.
    fn invalid(text: &str) -> String {
        let message = Rules::parse(text).expect_err("an invalid type").to_string();
        message
            .split_once("): ")
            .map(|(_, problem)| problem.to_owned())
            .expect("a type problem")
    }

    /// The message of a rules file refused as unreadable.
    fn unreadable(text: &str) -> String {
        let message = Rules::parse(text)
            .expect_err("an unreadable file")
            .to_string();
        message
            .strip_prefix("the rules file could not be read: ")
            .map(str::to_owned)
            .expect("an unreadable file")
    }

    const DECISIONS: &str = r#"
[[type]]
name = "decision"
paths = ["docs/decisions/*.md"]
status = { front-matter = "status", masthead = "Status" }
current = ["accepted"]
superseded = ["superseded"]
archived = ["rejected", "Superseded in part"]
date = { front-matter = "date" }
superseded-by = { front-matter = "superseded-by", masthead = "Superseded by" }
decision-headings = ["Decision"]

[[type]]
name = "archive"
paths = ["docs/archive/**"]
lifecycle = "archived"

[[type]]
name = "adr"
paths = ["doc/adr/*.md", "docs/adr/*.md"]
status = { section = "Status" }
superseded = ["superceded"]
current = ["accepted"]
"#;

    #[test]
    fn an_empty_file_has_no_types() {
        let rules = Rules::parse("").expect("valid");
        assert_eq!(rules.types(), []);
        assert_eq!(rules, Rules::default());
        let classification = rules.classify("docs/a.md", &document("# A\n"));
        assert_eq!(classification.doc_type, DEFAULT_TYPE);
        assert_eq!(classification.status, None);
        assert_eq!(classification.lifecycle, None);
        assert_eq!(classification.decision_blocks, []);
    }

    #[test]
    fn the_masthead_table_and_masthead_locations_add_known_keys() {
        let rules = Rules::parse(
            "[masthead]\nkeys = [\"Deciders\", \" Feature Branch \"]\n\n[[type]]\nname = \"t\"\npaths = [\"*.md\"]\nstatus = { masthead = \"Estado\" }\ndate = { front-matter = \"date\", masthead = \"Fecha\" }\n",
        )
        .expect("valid");
        let keys = rules.masthead_keys();
        for key in ["Deciders", "feature-branch", "Estado", "Fecha", "Status"] {
            assert!(keys.knows(key), "{key}");
        }
        assert!(!keys.knows("Short version"));
        assert!(!Rules::default().masthead_keys().knows("Deciders"));
        let source = b"# T\n\n> Estado: Aceptado | Deciders: Ana | Short version: x\n".to_vec();
        let parsed = Document::parse_with(source, keys).expect("parses");
        assert_eq!(parsed.meta.fields.len(), 2);
        assert_eq!(
            rules.classify("a.md", &parsed).status.as_deref(),
            Some("Aceptado")
        );
        let message = unreadable("[masthead]\nkeys = [\"Status\", \"a*b\"]\n");
        assert!(message.contains("line 2"), "{message}");
        assert!(
            message.contains("the masthead key \"a*b\" cannot match a masthead line"),
            "{message}"
        );
        assert!(unreadable("[masthead]\nkeys = [\" \"]\n").contains("cannot match"));
        assert!(unreadable("[masthead]\nother = 1\n").contains("other"));
    }

    #[test]
    fn the_compare_table_names_the_default_branch() {
        let rules = Rules::parse("[compare]\ndefault-branch = \"origin/dev\"\n").expect("valid");
        assert_eq!(rules.default_branch(), Some("origin/dev"));
        assert_eq!(Rules::parse("").expect("valid").default_branch(), None);
        assert_eq!(Rules::default().default_branch(), None);
        for bad in ["", "-x", "a..b", "a b", "a\\u0007b"] {
            let text = format!("[compare]\ndefault-branch = \"{bad}\"\n");
            let message = unreadable(&text);
            assert!(message.contains("line 2"), "{bad}: {message}");
            assert!(message.contains("cannot name a branch"), "{bad}: {message}");
        }
        assert!(unreadable("[compare]\nbranch = \"dev\"\n").contains("branch"));
    }

    #[test]
    fn the_first_matching_type_wins() {
        let rules = Rules::parse(DECISIONS).expect("valid");
        assert_eq!(rules.types().len(), 3);
        assert_eq!(
            rules
                .type_for("docs/decisions/0001-a.md")
                .map(|rule| rule.name.as_str()),
            Some("decision")
        );
        assert_eq!(
            rules
                .type_for("docs/archive/x/y.md")
                .map(|rule| rule.name.as_str()),
            Some("archive")
        );
        assert_eq!(
            rules
                .type_for("doc/adr/0002-b.md")
                .map(|rule| rule.name.as_str()),
            Some("adr")
        );
        assert_eq!(
            rules
                .type_for("docs/adr/0002-b.md")
                .map(|rule| rule.name.as_str()),
            Some("adr")
        );
        assert!(rules.type_for("docs/decisions/old/0001-a.md").is_none());
        assert!(rules.type_for("Docs/decisions/0001-a.md").is_none());
        assert!(rules.type_for("README.md").is_none());
    }

    #[test]
    fn patterns_follow_the_documented_rules() {
        let rules = Rules::parse(
            "[[type]]\nname = \"t\"\npaths = [\"specs/*/spec.md\", \".kiro/**/tasks.md\", \"adr/[0-9]*.md\"]\n",
        )
        .expect("valid");
        let rule = &rules.types()[0];
        assert!(rule.matches("specs/001-login/spec.md"));
        assert!(!rule.matches("specs/a/b/spec.md"));
        assert!(rule.matches(".kiro/specs/x/tasks.md"));
        assert!(rule.matches(".kiro/tasks.md"));
        assert!(rule.matches("adr/0001-x.md"));
        assert!(!rule.matches("adr/x.md"));
    }

    #[test]
    fn status_comes_from_front_matter_before_the_masthead() {
        let rules = Rules::parse(DECISIONS).expect("valid");
        let both =
            document("---\nstatus: accepted\ndate: 2026-01-05\n---\n\n# A\n\n> Status: rejected\n");
        let found = rules.classify("docs/decisions/0001-a.md", &both);
        assert_eq!(found.doc_type, "decision");
        assert_eq!(found.status.as_deref(), Some("accepted"));
        assert_eq!(found.lifecycle, Some(Lifecycle::Current));
        assert_eq!(found.date.as_deref(), Some("2026-01-05"));
        let masthead =
            document("# A\n\n> Status: Superseded | Superseded by: [0009 New](0009-new.md)\n");
        let found = rules.classify("docs/decisions/0001-a.md", &masthead);
        assert_eq!(found.status.as_deref(), Some("Superseded"));
        assert_eq!(found.lifecycle, Some(Lifecycle::Superseded));
        assert_eq!(found.superseded_by.as_deref(), Some("0009 New"));
        assert_eq!(found.superseded_by_links, vec!["0009-new.md".to_owned()]);
        assert_eq!(found.date, None);
    }

    #[test]
    fn empty_or_non_text_values_count_as_missing() {
        let rules = Rules::parse(DECISIONS).expect("valid");
        let empty = document("---\nstatus: \"  \"\n---\n\n# A\n\n> Status: rejected\n");
        assert_eq!(
            rules
                .classify("docs/decisions/a.md", &empty)
                .status
                .as_deref(),
            Some("rejected")
        );
        let list = document("---\nstatus: [a, b]\n---\n\n# A\n");
        assert_eq!(rules.classify("docs/decisions/a.md", &list).status, None);
    }

    #[test]
    fn status_can_come_from_a_section() {
        let rules = Rules::parse(DECISIONS).expect("valid");
        let adr = document(
            "# 1. Use X\n\nDate: 2018-02-01\n\n## Status\n\n### Note\n\n\nSuperceded by [2. Use Y](0002-use-y.md)\n\nAccepted\n\n## Context\n\nText.\n",
        );
        let found = rules.classify("doc/adr/0001-use-x.md", &adr);
        assert_eq!(found.status.as_deref(), Some("Superceded by 2. Use Y"));
        assert_eq!(found.lifecycle, Some(Lifecycle::Superseded));
        let empty = document("# A\n\n## Status\n\n## Context\n\nText.\n");
        assert_eq!(rules.classify("doc/adr/a.md", &empty).status, None);
        let missing = document("# A\n\n> ## Status\n>\n> Accepted\n");
        assert_eq!(rules.classify("doc/adr/a.md", &missing).status, None);
        let blank_first = document("# A\n\n## Status\n\n```\n\nAccepted\n```\n");
        assert_eq!(
            rules
                .classify("doc/adr/a.md", &blank_first)
                .status
                .as_deref(),
            Some("Accepted")
        );
    }

    #[test]
    fn a_type_default_lifecycle_fills_in() {
        let rules = Rules::parse(DECISIONS).expect("valid");
        let found = rules.classify("docs/archive/old.md", &document("# Old\n"));
        assert_eq!(found.doc_type, "archive");
        assert_eq!(found.status, None);
        assert_eq!(found.lifecycle, Some(Lifecycle::Archived));
        let unknown = document("---\nstatus: draft\n---\n\n# A\n");
        assert_eq!(
            rules.classify("docs/decisions/a.md", &unknown).lifecycle,
            None
        );
    }

    #[test]
    fn status_values_match_by_word_and_the_longest_wins() {
        let rules = Rules::parse(DECISIONS).expect("valid");
        let rule = &rules.types()[0];
        assert_eq!(
            rule.lifecycle_of("**Accepted** (2026-01-05)"),
            Some(Lifecycle::Current)
        );
        assert_eq!(rule.lifecycle_of("✅ accepted"), Some(Lifecycle::Current));
        assert_eq!(
            rule.lifecycle_of("Superseded by 0009"),
            Some(Lifecycle::Superseded)
        );
        assert_eq!(
            rule.lifecycle_of("superseded in part by 0010"),
            Some(Lifecycle::Archived)
        );
        assert_eq!(rule.lifecycle_of("acceptedly"), None);
        assert_eq!(rule.lifecycle_of("draft"), None);
        assert_eq!(rule.lifecycle_of(""), None);
    }

    #[test]
    fn decision_sections_are_found() {
        let rules = Rules::parse(DECISIONS).expect("valid");
        let found = rules.classify(
            "docs/decisions/a.md",
            &document("# A\n\n## Context\n\nWhy.\n\n## decision\n\nWe pick X.\n\n### Details\n\n- one\n\n## Costs\n\nSome.\n\n> ## Decision\n"),
        );
        assert_eq!(found.decision_blocks, vec![3, 4, 5, 6]);
        assert!(rules.types()[0].is_decision_heading("  DECISION "));
        assert!(!rules.types()[0].is_decision_heading("Decisions"));
    }

    #[test]
    fn other_statuses_are_allowed_without_a_lifecycle() {
        let rules = Rules::parse(
            "[[type]]\nname = \"d\"\npaths = [\"*.md\"]\nstatus = { masthead = \"Status\" }\ncurrent = [\"accepted\"]\nsuperseded = [\"superseded\"]\narchived = [\"rejected\"]\nother-statuses = [\"proposed\", \"superseded in part\"]\nlifecycle = \"current\"\nrequired = [\"date\"]\n",
        )
        .expect("valid");
        let rule = &rules.types()[0];
        assert_eq!(rule.required, ["date"]);
        assert_eq!(
            rule.statuses().collect::<Vec<_>>(),
            [
                "accepted",
                "superseded",
                "rejected",
                "proposed",
                "superseded in part"
            ]
        );
        assert!(rule.allows_status("Proposed (2026-01)"));
        assert_eq!(rule.lifecycle_of("Proposed"), None);
        assert!(rule.allows_status("superseded in part by 0012"));
        assert_eq!(rule.lifecycle_of("superseded in part by 0012"), None);
        assert_eq!(
            rule.lifecycle_of("superseded by 0012"),
            Some(Lifecycle::Superseded)
        );
        assert!(!rule.allows_status("draft"));
        let found = rules.classify("a.md", &document("# A\n\n> Status: proposed\n"));
        assert_eq!(found.status.as_deref(), Some("proposed"));
        assert_eq!(found.lifecycle, Some(Lifecycle::Current));
    }

    #[test]
    fn locations_say_which_place_held_the_value() {
        let location = Location {
            front_matter: Some("status".to_owned()),
            masthead: Some("Status".to_owned()),
            section: Some("Status".to_owned()),
            file: None,
            key: None,
        };
        let place = |source: &str| {
            location
                .find(&document(source))
                .map(|found| (found.text, found.place))
        };
        assert_eq!(
            place("---\nstatus: a\n---\n\n# T\n\n> Status: b\n"),
            Some(("a".to_owned(), Place::FrontMatter))
        );
        assert_eq!(
            place("# T\n\n> Status: [b](b.md) | Date: x\n"),
            Some(("b".to_owned(), Place::Masthead))
        );
        assert_eq!(
            place("# T\n\n## Status\n\nc\n"),
            Some(("c".to_owned(), Place::Section))
        );
        assert_eq!(place("# T\n"), None);
        let links = location
            .find(&document("# T\n\n> Status: [b](b.md) | Date: x\n"))
            .map(|found| found.links);
        assert_eq!(links, Some(vec!["b.md".to_owned()]));
    }

    #[test]
    fn impossible_new_keys_are_refused() {
        let base = "[[type]]\nname = \"t\"\npaths = [\"a/*.md\"]\n";
        assert_eq!(
            invalid(&format!("{base}required = [\"status\", \" \"]\n")),
            "a required key is empty"
        );
        assert_eq!(
            invalid(&format!("{base}other-statuses = [\"draft\"]\n")),
            "it lists status values but no `status` location"
        );
        assert_eq!(
            invalid(&format!(
                "{base}status = {{ masthead = \"S\" }}\ncurrent = [\"draft\"]\nother-statuses = [\"Draft\"]\n"
            )),
            "the status value \"Draft\" is in more than one place"
        );
    }

    #[test]
    fn syntax_errors_name_the_line() {
        let message =
            unreadable("[[type]]\nname = \"x\"\npaths = [\"a\"]\nstauts = { masthead = \"S\" }\n");
        assert!(message.contains("line 4"), "{message}");
        assert!(message.contains("stauts"), "{message}");
        assert!(unreadable("[[type]]\nname = 3\n").contains("line 2"));
        assert!(unreadable("other = 1\n").contains("other"));
        assert!(
            unreadable("[[type]]\nname = \"x\"\npaths = [\"a\"]\nlifecycle = \"gone\"\n")
                .contains("gone")
        );
        assert!(
            unreadable("[[type]]\nname = \"x\"\npaths = [\"a\"]\nstatus = { field = \"s\" }\n")
                .contains("field")
        );
        assert!(unreadable("[[type]]\nname = \"x\"\n").contains("paths"));
    }

    #[test]
    fn impossible_types_are_refused_with_a_reason() {
        let base = "[[type]]\nname = \"t\"\npaths = [\"a/*.md\"]\n";
        assert_eq!(
            invalid("[[type]]\nname = \" \"\npaths = [\"a\"]\n"),
            "its name is empty"
        );
        assert_eq!(
            invalid("[[type]]\nname = \"t\"\npaths = []\n"),
            "it lists no paths"
        );
        assert_eq!(
            invalid("[[type]]\nname = \"t\"\npaths = [\"a/**b\"]\n"),
            "the path \"a/**b\" is not a valid pattern: Pattern syntax error near position 4: recursive wildcards must form a single path component"
        );
        assert_eq!(
            invalid(&format!("{base}status = {{}}\n")),
            "`status` names no place to look"
        );
        assert_eq!(
            invalid(&format!("{base}date = {{ masthead = \" \" }}\n")),
            "`date` has an empty name"
        );
        assert_eq!(
            invalid(&format!("{base}superseded-by = {{ section = \"\" }}\n")),
            "`superseded-by` has an empty name"
        );
        assert_eq!(
            invalid(&format!("{base}current = [\"x\"]\n")),
            "it lists status values but no `status` location"
        );
        assert_eq!(
            invalid(&format!(
                "{base}status = {{ masthead = \"S\" }}\ncurrent = [\"Done\"]\narchived = [\"done\"]\n"
            )),
            "the status value \"done\" is in more than one place"
        );
        assert_eq!(
            invalid(&format!(
                "{base}status = {{ masthead = \"S\" }}\ncurrent = [\"--\"]\n"
            )),
            "the status value \"--\" has no letters or digits"
        );
        assert_eq!(
            invalid(&format!(
                "{base}decision-headings = [\"Decision\", \" \"]\n"
            )),
            "a decision heading is empty"
        );
        let error = Rules::parse(&format!("{base}[[type]]\nname = \"u\"\npaths = []\n"))
            .expect_err("invalid");
        assert_eq!(
            error.to_string(),
            "the rules file's type 2 (\"u\"): it lists no paths"
        );
    }

    #[test]
    fn the_presets_key_names_built_in_presets_once_each() {
        let rules =
            Rules::parse("presets = [\"openspec\", \"adr\", \"openspec\"]\n").expect("valid");
        assert_eq!(rules.presets(), ["openspec", "adr"]);
        assert_eq!(rules.types(), []);
        assert_eq!(Rules::default().presets(), [] as [String; 0]);
        let message = unreadable("\npresets = [\"adr\", \"nope\"]\n");
        assert!(message.contains("line 2"), "{message}");
        assert!(
            message.contains(
                "driftmark has no preset \"nope\"; the presets are \"openspec\", \"spec-kit\", \"adr\", \"kiro\""
            ),
            "{message}"
        );
    }

    #[test]
    fn a_preset_adds_its_types_after_the_own_and_its_masthead_keys() {
        let mut own = Rules::parse(
            "[lint]\nmax-heading-length = 50\n\n[[type]]\nname = \"own\"\npaths = [\"specs/*/spec.md\"]\n",
        )
        .expect("valid");
        let lint = own.lint().clone();
        let preset = Rules::parse(
            "[masthead]\nkeys = [\"Created\"]\n\n[lint]\nmax-heading-length = 7\n\n[compare]\ndefault-branch = \"x\"\n\n[[type]]\nname = \"feature-spec\"\npaths = [\"specs/*/spec.md\", \"specs/*/plan.md\"]\n",
        )
        .expect("valid");
        assert!(!own.masthead_keys().knows("Created"));
        own.append(preset);
        let names: Vec<&str> = own.types().iter().map(|rule| rule.name.as_str()).collect();
        assert_eq!(names, ["own", "feature-spec"]);
        assert_eq!(
            own.type_for("specs/a/spec.md").map(|r| r.name.as_str()),
            Some("own")
        );
        assert_eq!(
            own.type_for("specs/a/plan.md").map(|r| r.name.as_str()),
            Some("feature-spec")
        );
        assert!(own.masthead_keys().knows("Created"));
        assert_eq!(own.lint(), &lint);
        assert_eq!(own.default_branch(), None);
    }

    /// A made-up repository: files by path, for locations that read a file.
    struct Files(Vec<(&'static str, Vec<u8>)>);

    impl RepoFiles for Files {
        fn read(&self, path: &str) -> Option<Vec<u8>> {
            self.0
                .iter()
                .find(|(name, _)| *name == path)
                .map(|(_, bytes)| bytes.clone())
        }

        fn markdown_in(&self, _folders: &[&str]) -> Vec<String> {
            Vec::new()
        }
    }

    const SPEC_JSON: &str = "{\n  \"feature_name\": \"albums\",\n  \"Updated_At\": \"2026-01-05\",\n  \"phase\": \"tasks-generated\",\n  \"approvals\": {\n    \"requirements\": { \"generated\": true, \"approved\": true },\n    \"design\": { \"approved\": \"  \" }\n  },\n  \"tags\": [\"a\", \"b\"]\n}\n";

    fn in_file(key: &str) -> Location {
        Location {
            front_matter: None,
            masthead: None,
            section: None,
            file: Some("spec.json".to_owned()),
            key: Some(key.to_owned()),
        }
    }

    #[test]
    fn a_file_location_reads_the_nearest_file_beside_the_document() {
        let doc = document("# Requirements\n");
        let files = Files(vec![
            (
                ".kiro/specs/albums/spec.json",
                SPEC_JSON.as_bytes().to_vec(),
            ),
            (".kiro/spec.json", b"{\"phase\": \"from-above\"}".to_vec()),
            ("spec.json", b"phase: at-the-root\n".to_vec()),
        ]);
        let read = |path: &str, key: &str| {
            in_file(key)
                .find_in(path, &doc, &files)
                .map(|found| (found.text, found.place))
        };
        let here = ".kiro/specs/albums/requirements.md";
        assert_eq!(
            read(here, "phase"),
            Some(("tasks-generated".to_owned(), Place::File))
        );
        assert_eq!(
            read(here, "approvals.requirements.approved"),
            Some(("true".to_owned(), Place::File))
        );
        assert_eq!(
            read(here, "updated-at").map(|f| f.0).as_deref(),
            Some("2026-01-05")
        );
        assert_eq!(
            read(".kiro/specs/other/x.md", "phase")
                .map(|f| f.0)
                .as_deref(),
            Some("from-above")
        );
        assert_eq!(
            read("top.md", "phase").map(|f| f.0).as_deref(),
            Some("at-the-root")
        );
        for missing in [
            "status",
            "approvals",
            "tags",
            "approvals.design.approved",
            "approvals.tasks.approved",
            "phase.inner",
            "tags.first",
        ] {
            assert_eq!(read(here, missing), None, "{missing}");
        }
        assert_eq!(in_file("phase").find(&doc), None);
        let no_key = Location {
            key: None,
            ..in_file("phase")
        };
        assert_eq!(no_key.find_in(here, &doc, &files), None);
        let no_file = Location {
            file: None,
            ..in_file("phase")
        };
        assert_eq!(no_file.find_in(here, &doc, &files), None);
        assert_eq!(files.markdown_in(&[""]), Vec::<String>::new());
    }

    #[test]
    fn a_file_that_cannot_be_read_gives_no_value() {
        let doc = document("# T\n");
        let read = |bytes: Vec<u8>| {
            in_file("phase")
                .find_in("a/doc.md", &doc, &Files(vec![("a/spec.json", bytes)]))
                .map(|found| found.text)
        };
        assert_eq!(
            read(b"\xef\xbb\xbf{\"phase\": \"bom\"}".to_vec()).as_deref(),
            Some("bom")
        );
        assert_eq!(
            read(b"{\n\t\"phase\":\n\t\t\"tabs\"\n}\n".to_vec()).as_deref(),
            Some("tabs")
        );
        assert_eq!(read(b"{\"phase\": \"a\"".to_vec()), None);
        assert_eq!(read(b"[1, 2]".to_vec()), None);
        assert_eq!(read(vec![0xff, 0xfe]), None);
        let mut largest = b"{\"phase\": \"big\"}\n".to_vec();
        largest.resize(MAX_FILE_SIZE, b' ');
        assert_eq!(read(largest.clone()).as_deref(), Some("big"));
        largest.push(b' ');
        assert_eq!(read(largest), None);
    }

    #[test]
    fn a_yaml_file_keeps_its_nesting() {
        let doc = document("# T\n");
        let yaml = b"approvals:\n  requirements:\n    approved: yes\n  design:\n    approved: no\n";
        let files = Files(vec![("a/spec.json", yaml.to_vec())]);
        let read = |key: &str| {
            in_file(key)
                .find_in("a/doc.md", &doc, &files)
                .map(|found| found.text)
        };
        assert_eq!(read("approvals.design.approved").as_deref(), Some("no"));
        assert_eq!(
            read("approvals.requirements.approved").as_deref(),
            Some("yes")
        );
    }

    #[test]
    fn the_document_wins_over_a_file_and_classify_with_reads_files() {
        let rules = Rules::parse(
            "[[type]]\nname = \"requirements\"\npaths = [\".kiro/specs/*/requirements.md\"]\nstatus = { front-matter = \"status\", file = \"spec.json\", key = \"phase\" }\ndate = { file = \"spec.json\", key = \"updated_at\" }\nlifecycle = \"current\"\n",
        )
        .expect("valid");
        let files = Files(vec![(
            ".kiro/specs/albums/spec.json",
            SPEC_JSON.as_bytes().to_vec(),
        )]);
        let path = ".kiro/specs/albums/requirements.md";
        let plain = document("# Requirements\n");
        let found = rules.classify_with(path, &plain, &files);
        assert_eq!(found.status.as_deref(), Some("tasks-generated"));
        // The file writes the key as `Updated_At`; keys match as front
        // matter keys do.
        assert_eq!(found.date.as_deref(), Some("2026-01-05"));
        assert_eq!(found.lifecycle, Some(Lifecycle::Current));
        let without = rules.classify(path, &plain);
        assert_eq!(without.status, None);
        let written = document("---\nstatus: approved\n---\n\n# Requirements\n");
        assert_eq!(
            rules
                .classify_with(path, &written, &files)
                .status
                .as_deref(),
            Some("approved")
        );
        assert_eq!(
            rules.classify_with("README.md", &plain, &files).doc_type,
            DEFAULT_TYPE
        );
    }

    #[test]
    fn impossible_file_locations_are_refused() {
        let base = "[[type]]\nname = \"t\"\npaths = [\"a/*.md\"]\n";
        let cases = [
            (
                "{ file = \"spec.json\" }",
                "`status` names a file but no key to read in it",
            ),
            (
                "{ key = \"phase\" }",
                "`status` names a key but no file to read it from",
            ),
            (
                "{ file = \" \", key = \"phase\" }",
                "`status` has an empty name",
            ),
            (
                "{ file = \"spec.json\", key = \"\" }",
                "`status` has an empty name",
            ),
            (
                "{ file = \"a/spec.json\", key = \"phase\" }",
                "`status` names the file \"a/spec.json\"; write a file name without folders, and driftmark looks for it beside the document and in the folders above",
            ),
            (
                "{ file = \"a\\\\b\", key = \"phase\" }",
                "`status` names the file \"a\\\\b\"; write a file name without folders, and driftmark looks for it beside the document and in the folders above",
            ),
            (
                "{ file = \".\", key = \"phase\" }",
                "`status` names the file \".\"; write a file name without folders, and driftmark looks for it beside the document and in the folders above",
            ),
            (
                "{ file = \"..\", key = \"phase\" }",
                "`status` names the file \"..\"; write a file name without folders, and driftmark looks for it beside the document and in the folders above",
            ),
            (
                "{ file = \"spec.json\", key = \"a..b\" }",
                "`status` has the key \"a..b\", which has an empty part between its dots",
            ),
            (
                "{ file = \"spec.json\", key = \".a\" }",
                "`status` has the key \".a\", which has an empty part between its dots",
            ),
        ];
        for (location, problem) in cases {
            assert_eq!(
                invalid(&format!("{base}status = {location}\n")),
                problem,
                "{location}"
            );
        }
        assert!(
            Rules::parse(&format!(
                "{base}status = {{ file = \"..x\", key = \"a.b\" }}\n"
            ))
            .is_ok()
        );
    }
}
