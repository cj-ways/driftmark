//! Presets: ready-made rules files for popular folder layouts, built into
//! driftmark, so the change ranking and the status view work with no setup.
//!
//! A preset is a rules file in the format of [`crate::rules`], kept as text so
//! a user can read it, copy it into `driftmark.toml` and change it. A
//! repository names the presets it uses with `presets = [...]` in its rules
//! file. A repository with no rules file uses every preset whose marker files
//! or folders it has. `docs/features/presets.md` describes each preset.
//!
//! The core reads no files: [`RepoFiles`] answers which marker files exist.

use std::fmt::Write as _;

use glob::Pattern;

use crate::document::yaml;
use crate::document::{MetaValue, normalize_key};
use crate::rules::{MATCHING, RepoFiles, Rules, RulesError};

#[cfg(test)]
mod tests;

/// One built-in preset.
#[derive(Debug)]
pub struct Preset {
    /// The name `presets = [...]` and `driftmark presets show` use.
    pub name: &'static str,
    /// What the preset is for, in a few words.
    pub title: &'static str,
    /// The rules file, as a user can copy it. It holds `[[type]]` entries
    /// and, when the layout needs them, a `[masthead]` table.
    pub text: &'static str,
    /// How the preset is found in a repository that has no rules file, tried
    /// in order. A preset with none is used only when a rules file names it.
    pub markers: &'static [Marker],
    /// Where the preset learns of folders its text does not name.
    pub folders: Folders,
}

/// A sign that a repository uses a preset's layout.
#[derive(Debug)]
pub enum Marker {
    /// A regular file at this path, such as `openspec/config.yaml`.
    File(&'static str),
    /// A Markdown file whose path matches this pattern, matched as a rules
    /// file's paths are.
    Markdown(&'static str),
    /// A Markdown file that one of the preset's own types matches.
    Types,
}

/// Where a preset learns of more folders than its text names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Folders {
    /// Nowhere: the text names every path.
    None,
    /// The folder adr-tools' `.adr-dir` names, and the `adrFolder` values of
    /// log4brains' `.log4brains.yml`. Their numbered and dated records join
    /// the `paths` list that holds [`FOLDERS_LINE`], right after that line.
    Adr,
}

/// The comment in the ADR preset after which the folders that `.adr-dir`
/// and `.log4brains.yml` name are added.
pub const FOLDERS_LINE: &str =
    "# driftmark adds the folders that .adr-dir and .log4brains.yml name here.";

/// The file names of ADR records inside a folder, as the preset's text writes
/// them for its own folders.
const RECORD_NAMES: [&str; 2] = ["[0-9]*.md", "[Aa][Dd][Rr]-[0-9]*.md"];

/// Every built-in preset. A repository with no rules file adds the types of
/// those it uses in this order.
pub const PRESETS: &[Preset] = &[
    Preset {
        name: "openspec",
        title: "OpenSpec: living specs, open changes and the archive",
        text: include_str!("openspec.toml"),
        markers: &[
            Marker::File("openspec/config.yaml"),
            Marker::File("openspec/project.md"),
            Marker::Types,
        ],
        folders: Folders::None,
    },
    Preset {
        name: "spec-kit",
        title: "GitHub Spec Kit: one folder per feature under specs/",
        text: include_str!("spec-kit.toml"),
        markers: &[
            Marker::File(".specify/memory/constitution.md"),
            Marker::File("memory/constitution.md"),
            Marker::Markdown("specs/[0-9][0-9][0-9]-*/spec.md"),
        ],
        folders: Folders::None,
    },
    Preset {
        name: "adr",
        title: "ADR folders: adr-tools, MADR, log4brains and Nygard-style records",
        text: include_str!("adr.toml"),
        markers: &[
            Marker::File(".adr-dir"),
            Marker::File(".log4brains.yml"),
            Marker::Types,
        ],
        folders: Folders::Adr,
    },
    Preset {
        name: "kiro",
        title: "Kiro and cc-sdd: specs and steering under .kiro/",
        text: include_str!("kiro.toml"),
        markers: &[Marker::Types],
        folders: Folders::None,
    },
];

/// The built-in preset with this name.
#[must_use]
pub fn find(name: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|preset| preset.name == name)
}

/// A preset whose layout a repository has, and the file that showed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// The preset.
    pub preset: &'static Preset,
    /// The path of the first marker found: a marker file, or the first
    /// Markdown file, in path order, that a marker pattern matches.
    pub marker: String,
}

/// Presets compare by name, because no two built-in presets share one.
impl PartialEq for Preset {
    fn eq(&self, other: &Preset) -> bool {
        self.name == other.name
    }
}

impl Eq for Preset {}

/// The built-in presets whose layout the repository has, in [`PRESETS`]
/// order.
#[must_use]
pub fn detect(files: &dyn RepoFiles) -> Vec<Found> {
    detect_in(PRESETS, files)
}

/// [`detect`] over any list, so tests can try a list of their own.
fn detect_in(list: &'static [Preset], files: &dyn RepoFiles) -> Vec<Found> {
    let patterns: Vec<Vec<Pattern>> = list.iter().map(Preset::marker_patterns).collect();
    let mut folders: Vec<String> = patterns
        .iter()
        .flatten()
        .map(|pattern| folder_of(pattern.as_str()).to_owned())
        .collect();
    folders.sort();
    folders.dedup();
    let wanted: Vec<&str> = folders.iter().map(String::as_str).collect();
    // One listing serves every preset, so a commit's trees are walked once.
    let mut markdown = if wanted.is_empty() {
        Vec::new()
    } else {
        files.markdown_in(&wanted)
    };
    markdown.sort();
    list.iter()
        .zip(&patterns)
        .filter_map(|(preset, patterns)| {
            let file = preset.markers.iter().find_map(|marker| match marker {
                Marker::File(path) => files.read(path).map(|_| (*path).to_owned()),
                Marker::Markdown(_) | Marker::Types => None,
            });
            let marker = file.or_else(|| {
                markdown
                    .iter()
                    .find(|path| patterns.iter().any(|p| p.matches_with(path, MATCHING)))
                    .cloned()
            })?;
            Some(Found { preset, marker })
        })
        .collect()
}

impl Preset {
    /// The patterns of this preset's Markdown markers: those written out,
    /// and the paths of its own types for [`Marker::Types`].
    fn marker_patterns(&self) -> Vec<Pattern> {
        let types: Vec<String> = Rules::parse(self.text)
            .map(|rules| {
                rules
                    .types()
                    .iter()
                    .flat_map(|rule| rule.paths.clone())
                    .collect()
            })
            .unwrap_or_default();
        self.markers
            .iter()
            .flat_map(|marker| match marker {
                Marker::File(_) => Vec::new(),
                Marker::Markdown(pattern) => vec![(*pattern).to_owned()],
                Marker::Types => types.clone(),
            })
            .filter_map(|pattern| Pattern::new(&pattern).ok())
            .collect()
    }

    /// The folders, beyond those its text names, whose files this preset
    /// reads in this repository, each once, in the order found.
    #[must_use]
    pub fn folders(&self, files: &dyn RepoFiles) -> Vec<String> {
        match self.folders {
            Folders::None => Vec::new(),
            Folders::Adr => {
                let mut found = Vec::new();
                let adr_dir = files.read(".adr-dir").and_then(|bytes| {
                    String::from_utf8(bytes)
                        .ok()?
                        .lines()
                        .map(str::trim)
                        .find(|line| !line.is_empty())
                        .map(str::to_owned)
                });
                let log4brains = files
                    .read(".log4brains.yml")
                    .map(|bytes| log4brains_folders(&bytes))
                    .unwrap_or_default();
                for folder in adr_dir.into_iter().chain(log4brains) {
                    // A folder the text names already, such as log4brains'
                    // default `docs/adr`, adds nothing.
                    if let Some(folder) = clean_folder(&folder)
                        && !found.contains(&folder)
                        && !self.text.contains(&record_entry(&folder, RECORD_NAMES[0]))
                    {
                        found.push(folder);
                    }
                }
                found
            }
        }
    }

    /// The rules file text with these folders' records added, as
    /// `driftmark presets show` prints it for a repository. A folder whose
    /// records the text already names is left out.
    #[must_use]
    pub fn text_with(&self, folders: &[String]) -> String {
        let Some(at) = self.text.find(FOLDERS_LINE) else {
            return self.text.to_owned();
        };
        let line_end = self.text[at..]
            .find('\n')
            .map_or(self.text.len(), |end| at + end + 1);
        let mut added = String::new();
        for folder in folders {
            for name in RECORD_NAMES {
                let entry = record_entry(folder, name);
                if !self.text.contains(&entry) {
                    let _ = writeln!(added, "  {entry}");
                }
            }
        }
        format!(
            "{}{added}{}",
            &self.text[..line_end],
            &self.text[line_end..]
        )
    }

    /// The rules this preset gives with these extra folders.
    ///
    /// # Errors
    ///
    /// A [`RulesError::Syntax`] naming the preset when its text is not a
    /// valid rules file. The built-in presets are tested to be valid.
    pub fn rules(&self, folders: &[String]) -> Result<Rules, RulesError> {
        Rules::parse(&self.text_with(folders)).map_err(|error| {
            RulesError::Syntax(format!(
                "driftmark's built-in preset {:?} is broken, which is a bug in driftmark: {error}",
                self.name
            ))
        })
    }
}

/// The entry of a `paths` list for the records named `name` in `folder`, as
/// TOML text: the folder's wildcards escaped, then quoted, with a comma.
fn record_entry(folder: &str, name: &str) -> String {
    let pattern = if folder.is_empty() {
        name.to_owned()
    } else {
        format!("{}/{name}", Pattern::escape(folder))
    };
    format!("\"{}\",", toml_escaped(&pattern))
}

/// The `adrFolder` values of a `.log4brains.yml`: the project's, then each
/// package's.
fn log4brains_folders(bytes: &[u8]) -> Vec<String> {
    let Some(entries) = std::str::from_utf8(bytes)
        .ok()
        .and_then(|text| yaml::read(text).ok())
    else {
        return Vec::new();
    };
    let Some(MetaValue::Map(project)) = value(&entries, "project") else {
        return Vec::new();
    };
    let packages = match value(project, "packages") {
        Some(MetaValue::List(items)) => items.as_slice(),
        _ => &[],
    };
    std::iter::once(project.as_slice())
        .chain(packages.iter().filter_map(|item| match item {
            MetaValue::Map(entries) => Some(entries.as_slice()),
            MetaValue::Text(_) | MetaValue::List(_) => None,
        }))
        .filter_map(|entries| value(entries, "adrFolder")?.as_text().map(str::to_owned))
        .collect()
}

/// The value of a key in a YAML table, keys compared as front matter keys
/// are.
fn value<'a>(entries: &'a [(String, MetaValue)], key: &str) -> Option<&'a MetaValue> {
    let wanted = normalize_key(key);
    entries
        .iter()
        .find(|(name, _)| normalize_key(name) == wanted)
        .map(|(_, value)| value)
}

/// A folder from a marker file as a path from the repository root, with `/`
/// between folders and no `./` or trailing `/`; the root is the empty name.
/// `None` for a folder outside the repository, or one with control
/// characters or empty parts in it.
fn clean_folder(written: &str) -> Option<String> {
    let mut folder = written.trim().replace('\\', "/");
    while let Some(rest) = folder.strip_prefix("./") {
        folder = rest.to_owned();
    }
    let folder = folder.trim_end_matches('/');
    if folder == "." || folder.is_empty() {
        return Some(String::new());
    }
    let plain = !folder.starts_with('/')
        && !folder.chars().any(char::is_control)
        && folder
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..");
    plain.then(|| folder.to_owned())
}

/// Text for inside a TOML string between double quotes.
fn toml_escaped(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

/// The folder a pattern's files sit under before its first wildcard: its
/// leading parts that hold no `*`, `?` or `[`, without the last part. The
/// root is the empty name.
fn folder_of(pattern: &str) -> &str {
    let mut end = 0;
    let mut parts = pattern.split('/').peekable();
    while let Some(part) = parts.next() {
        if parts.peek().is_none() || part.contains(['*', '?', '[']) {
            break;
        }
        end += part.len() + 1;
    }
    pattern[..end].trim_end_matches('/')
}

/// The rules a repository uses: its rules file's own types, then the types of
/// each preset it uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Setup {
    /// The combined rules. The writing check's settings and the default
    /// branch come from the rules file alone.
    pub rules: Rules,
    /// The presets in use, in the order their types were added.
    pub presets: Vec<InUse>,
}

/// A preset a repository uses, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InUse {
    /// The preset.
    pub preset: &'static Preset,
    /// The marker that showed the layout, when the repository has no rules
    /// file; `None` when the rules file names the preset.
    pub found: Option<String>,
    /// The folders beyond its text's own that the preset reads here.
    pub folders: Vec<String>,
}

impl Setup {
    /// The rules for a repository. With a rules file (`own`), they are its
    /// types, then the types of each preset it names, in the order named.
    /// With none, they are the types of every preset [`detect`] finds.
    ///
    /// # Errors
    ///
    /// When a preset's text is not a valid rules file, which the tests of
    /// the built-in presets rule out.
    pub fn build(own: Option<Rules>, files: &dyn RepoFiles) -> Result<Setup, RulesError> {
        Setup::build_from(PRESETS, own, files)
    }

    /// [`Setup::build`] over any list, so tests can try a list of their own.
    fn build_from(
        list: &'static [Preset],
        own: Option<Rules>,
        files: &dyn RepoFiles,
    ) -> Result<Setup, RulesError> {
        let (mut rules, chosen): (Rules, Vec<(&'static Preset, Option<String>)>) =
            if let Some(rules) = own {
                let named = rules
                    .presets()
                    .iter()
                    .filter_map(|name| list.iter().find(|preset| preset.name == name))
                    .map(|preset| (preset, None))
                    .collect();
                (rules, named)
            } else {
                let found = detect_in(list, files)
                    .into_iter()
                    .map(|found| (found.preset, Some(found.marker)))
                    .collect();
                (Rules::default(), found)
            };
        let mut presets = Vec::new();
        for (preset, found) in chosen {
            let folders = preset.folders(files);
            rules.append(preset.rules(&folders)?);
            presets.push(InUse {
                preset,
                found,
                folders,
            });
        }
        Ok(Setup { rules, presets })
    }
}
