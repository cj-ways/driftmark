//! `document_read`: a document at a revision, with its metadata, its
//! outline and its text, whole or for one section.

use std::ops::Range;

use driftmark_core::document::Document;
use driftmark_core::document::markdown::LineIndex;
use driftmark_core::document::outline::{Outline, Section};
use driftmark_core::rules::Lifecycle;
use serde_json::{Map, Value, json};

use super::fit;
use super::tools::{Args, Context, MAX_OFFSET, Tool, document_path, said};
use crate::serve::address::{Revision, WORKTREE};
use crate::serve::repo::DocumentAt;

/// The share of an answer's limit the outline may take before its deepest
/// headings are left out. The rest is for the text.
const OUTLINE_SHARE: usize = 40;

/// `document_read`.
pub(crate) const READ: Tool = Tool {
    name: "document_read",
    title: "Read a document",
    description: "Reads one Markdown document at a revision: its metadata (type, status, title, summary, size), its outline (each section's heading path, anchor, block index, first line and size) and its text, whole or for one section. Ask for a section by the anchor the outline or a search result gives. A long text is cut after a whole line; the answer's `cut.next` holds the arguments that read on.",
    arguments,
    required: &["path"],
    read_only: true,
    run: read,
};

fn arguments() -> Value {
    json!({
        "path": {
            "type": "string",
            "description": "The document, from the repository root, such as docs/guide.md.",
        },
        "revision": {
            "type": "string",
            "description": "A branch, a tag, a commit or an expression such as main~2, or \"worktree\" for the file on disk now, committed or not. Default \"worktree\".",
        },
        "section": {
            "type": "string",
            "description": "The anchor of a heading, without #, such as \"decision\": only that section's text is read, up to the next heading of its level. The outline and search results give anchors.",
        },
        "offset": {
            "type": "integer",
            "minimum": 0,
            "description": "The byte in the document to read on from, as a cut answer's `cut.next` gives it. The outline is left out when reading on.",
        },
        "outline": {
            "type": "boolean",
            "description": "Whether to list the outline. Default true, except when reading on with `offset`.",
        },
    })
}

/// A lifecycle as the answer writes it.
fn lifecycle_name(lifecycle: Lifecycle) -> &'static str {
    match lifecycle {
        Lifecycle::Current => "current",
        Lifecycle::Superseded => "superseded",
        Lifecycle::Archived => "archived",
    }
}

/// One outline entry: where the section is and how large it is.
fn entry(section: &Section, lines: &LineIndex) -> Value {
    json!({
        "path": section.path,
        "anchor": section.anchor,
        "block": section.block,
        "line": lines.line_of(section.bytes.start),
        "bytes": section.bytes.len(),
    })
}

/// The outline entries that fit in `room` characters: every section when
/// they fit, otherwise those down to the deepest level that fits, and at
/// least the first ones. Returns the entries and how many sections there
/// were.
fn outline_part(sections: &[&Section], lines: &LineIndex, room: usize) -> Vec<Value> {
    let all: Vec<(u8, Value)> = sections
        .iter()
        .map(|section| (section.level, entry(section, lines)))
        .collect();
    let deepest = all.iter().map(|(level, _)| *level).max().unwrap_or(0);
    for depth in (1..=deepest).rev() {
        let kept: Vec<Value> = all
            .iter()
            .filter(|(level, _)| *level <= depth)
            .map(|(_, value)| value.clone())
            .collect();
        if fit::size(&json!(kept)) <= room {
            return kept;
        }
    }
    // Even the top level does not fit: as many of the first entries as do.
    let mut used = 2;
    all.into_iter()
        .map(|(_, value)| value)
        .take_while(|value| {
            used += fit::size(value) + 1;
            used <= room
        })
        .collect()
}

fn read(context: &Context, args: &Args) -> Result<Value, String> {
    let limit = args.max_chars()?;
    let path = document_path(context, &args.text("path")?.unwrap_or_default())?;
    let default = if context.workdir.is_some() {
        WORKTREE
    } else {
        "HEAD"
    };
    let label = args.text("revision")?.unwrap_or_else(|| default.to_owned());
    let revision = if label == WORKTREE {
        Revision::Worktree
    } else {
        Revision::Name(label.clone())
    };
    let offset = args.given_number("offset", MAX_OFFSET)?;
    let with_outline = args.flag("outline", offset.is_none())?;
    let found = context.repo.document_at(&revision, &path).map_err(said)?;
    let text_path = path.to_lossy();
    let parsed = found.parsed.as_ref().as_ref().map_err(|error| {
        format!("`{text_path}` at `{label}` cannot be read as Markdown: {error}")
    })?;
    let source = parsed.source.as_str();
    let outline = Outline::of(parsed);
    let section = match args.text("section")? {
        None => None,
        Some(anchor) => Some(find_section(&outline, &anchor, &text_path)?),
    };
    let part = section.map_or(0..source.len(), |section| section.bytes.clone());
    let start = start_at(source, &part, offset)?;
    let in_part: Vec<&Section> = outline
        .sections
        .iter()
        .filter(|candidate| section.is_none_or(|section| section.blocks.contains(&candidate.block)))
        .collect();

    let mut head = metadata(&found, &label, &text_path, parsed);
    if let Some(section) = section {
        head.insert(
            "section".into(),
            json!({
                "path": section.path,
                "anchor": section.anchor,
                "block": section.block,
                "bytes": section.bytes.len(),
            }),
        );
    }
    let lines = LineIndex::new(source);
    let mut outline_note = None;
    if with_outline {
        let entries = outline_part(&in_part, &lines, limit * OUTLINE_SHARE / 100);
        if entries.len() < in_part.len() {
            outline_note = Some(format!(
                "the outline lists {} of {} sections, leaving out the deepest headings; read a section to see its own outline",
                entries.len(),
                in_part.len()
            ));
        }
        head.insert("outline".into(), json!(entries));
    }
    let text = Text {
        head,
        source,
        lines: &lines,
        part,
        start,
        outline_note,
    };
    Ok(text.fit(args, limit, with_outline))
}

/// The answer's first fields: where the document was read, and what the
/// rules file and its metadata say about it.
fn metadata(found: &DocumentAt, label: &str, path: &str, parsed: &Document) -> Map<String, Value> {
    let source = parsed.source.as_str();
    // `document_at` classified it with the files beside it, such as a
    // cc-sdd `spec.json`.
    let classification = found
        .classification
        .clone()
        .unwrap_or_else(|| found.rules.rules.classify(path, parsed));
    let mut head = Map::new();
    head.insert("schema_version".into(), json!(1));
    head.insert("path".into(), json!(path));
    head.insert("revision".into(), json!(label));
    head.insert(
        "commit".into(),
        json!(
            found
                .commit
                .as_ref()
                .map_or(WORKTREE, |commit| commit.as_hex())
        ),
    );
    if let Some(blob) = &found.blob {
        head.insert("blob".into(), json!(blob.as_hex()));
    }
    head.insert("type".into(), json!(classification.doc_type));
    let optional = [
        ("status", classification.status.clone()),
        (
            "lifecycle",
            classification
                .lifecycle
                .map(|lifecycle| lifecycle_name(lifecycle).to_owned()),
        ),
        ("date", classification.date.clone()),
        ("superseded_by", classification.superseded_by.clone()),
        ("title", parsed.meta.title.clone()),
        ("summary", parsed.meta.summary.clone()),
    ];
    for (name, value) in optional {
        if let Some(value) = value {
            head.insert(name.into(), json!(value));
        }
    }
    head.insert("bytes".into(), json!(source.len()));
    head.insert("lines".into(), json!(source.lines().count()));
    head
}

/// The section an anchor names, or a sentence that lists the anchors there
/// are. A leading `#` is allowed.
fn find_section<'a>(outline: &'a Outline, anchor: &str, path: &str) -> Result<&'a Section, String> {
    outline.find(anchor.trim_start_matches('#')).ok_or_else(|| {
        let anchors: Vec<&str> = outline
            .sections
            .iter()
            .take(30)
            .map(|section| section.anchor.as_str())
            .collect();
        format!(
            "`{path}` has no heading with the anchor `{anchor}`; its sections' anchors are: {}",
            anchors.join(", ")
        )
    })
}

/// Where reading starts: the start of the part, or the byte the call names,
/// moved back to the start of its character.
fn start_at(source: &str, part: &Range<usize>, offset: Option<usize>) -> Result<usize, String> {
    match offset {
        None => Ok(part.start),
        Some(offset) if part.contains(&offset) => Ok((part.start..=offset)
            .rev()
            .find(|&at| source.is_char_boundary(at))
            .unwrap_or(part.start)),
        Some(offset) => Err(format!(
            "`offset` {offset} is outside the text read, which runs from byte {} to byte {}",
            part.start, part.end
        )),
    }
}

/// The text of an answer, and everything around it.
struct Text<'a> {
    /// The answer without its text.
    head: Map<String, Value>,
    /// The whole document.
    source: &'a str,
    /// Its lines.
    lines: &'a LineIndex,
    /// The bytes read: the document or one section.
    part: Range<usize>,
    /// Where this answer's text starts.
    start: usize,
    /// Why the outline lost entries, when it did.
    outline_note: Option<String>,
}

impl Text<'_> {
    /// The answer with the text from `start` to `end`.
    fn answer(&self, end: usize, cut: Option<Value>) -> Value {
        let mut answer = self.head.clone();
        let last = if end > self.start {
            end - 1
        } else {
            self.start
        };
        answer.insert(
            "text_range".into(),
            json!({
                "start": self.start,
                "end": end,
                "first_line": self.lines.line_of(self.start),
                "last_line": self.lines.line_of(last),
            }),
        );
        answer.insert("text".into(), json!(&self.source[self.start..end]));
        if let Some(cut) = cut {
            answer.insert("cut".into(), cut);
        }
        Value::Object(answer)
    }

    /// Why the answer that ends at `end` is cut.
    fn reason(&self, end: usize) -> String {
        let mut parts = Vec::new();
        if end < self.part.end {
            parts.push(format!(
                "the text was cut after line {} of {}, at byte {end}; ask again with `offset` {end} to read on",
                self.lines.line_of(end.saturating_sub(1)),
                self.lines.line_of(self.part.end.saturating_sub(1)),
            ));
        }
        parts.extend(self.outline_note.clone());
        parts.join("; ")
    }

    /// The answer with as much text as fits in `limit` characters, cut after
    /// a whole line when one fits.
    fn fit(&self, args: &Args, limit: usize, with_outline: bool) -> Value {
        let next = |end: usize| {
            let mut changes = vec![("offset", json!(end))];
            if with_outline {
                changes.push(("outline", json!(false)));
            }
            args.next(&changes)
        };
        let whole = self.answer(self.part.end, None);
        if fit::size(&whole) <= limit && self.outline_note.is_none() {
            return whole;
        }
        let worst = fit::cut(&self.reason(self.start), Some(next(self.part.end)));
        let fixed = fit::size(&self.answer(self.start, Some(worst)));
        // The cut's sentence and offset take a few digits more or less than
        // in the estimate.
        let rest = &self.source[self.start..self.part.end];
        let end = self.start + fit::text_that_fits(rest, limit.saturating_sub(fixed + 64));
        let next = (end < self.part.end).then(|| next(end));
        self.answer(end, Some(fit::cut(&self.reason(end), next)))
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::*;

    #[test]
    fn a_long_outline_loses_its_deepest_headings_first() {
        let mut source = String::from("# Title\n\n");
        for n in 0..40 {
            let _ = write!(source, "## Part {n}\n\n### Detail {n}\n\nText.\n\n");
        }
        let document = Document::parse(source.into_bytes()).unwrap();
        let outline = Outline::of(&document);
        let sections: Vec<&Section> = outline.sections.iter().collect();
        let lines = LineIndex::new(&document.source);
        let all = outline_part(&sections, &lines, 100_000);
        assert_eq!(all.len(), 81);
        assert_eq!(all[1]["path"], json!(["Title", "Part 0"]));
        assert_eq!(all[1]["anchor"], "part-0");
        assert_eq!(all[1]["line"], 3);
        let two_levels = outline_part(&sections, &lines, 5_000);
        assert_eq!(two_levels.len(), 41);
        assert!(fit::size(&json!(two_levels)) <= 5_000);
        let first = outline_part(&sections, &lines, 300);
        assert!(!first.is_empty() && first.len() < 41);
        assert!(fit::size(&json!(first)) <= 300);
        assert_eq!(outline_part(&[], &lines, 10), Vec::<Value>::new());
    }

    #[test]
    fn lifecycles_are_written_in_lower_case() {
        assert_eq!(lifecycle_name(Lifecycle::Current), "current");
        assert_eq!(lifecycle_name(Lifecycle::Superseded), "superseded");
        assert_eq!(lifecycle_name(Lifecycle::Archived), "archived");
    }
}
