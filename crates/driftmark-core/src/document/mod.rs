//! The document model: one Markdown file as a flat list of blocks plus its
//! metadata. Every later part of driftmark works on this model: the change
//! view compares blocks, pages render them, notes anchor to them and the
//! status view reads the metadata.
//!
//! `docs/features/engine/document-model.md` states the contract in full: what
//! a block is, every field, and the rules for splitting a file into blocks.

mod blocks;
mod content;
pub mod markdown;
pub(crate) mod meta;
pub mod outline;
pub(crate) mod yaml;

use std::any::Any;
use std::fmt;
use std::ops::Range;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use comrak::{Arena, parse_document};
use serde::Serialize;

/// The most block quotes, list items, alerts and footnote definitions a block
/// may sit inside. Real documents nest a handful deep. The limit keeps a
/// hostile file from making every block carry a container path thousands of
/// entries long.
pub const MAX_DEPTH: usize = 100;

/// The largest file driftmark parses, in bytes: 2 MiB.
///
/// The parser's time and memory grow with the file. A hostile 2 MiB file
/// made only of nested list markers takes about half a second and 280 MB to
/// parse; at 4 MiB it takes twice that. The largest file in a real corpus of
/// about 3,500 documents has 479,045 bytes, so the limit leaves more than
/// four times that.
pub const MAX_FILE_SIZE: usize = 2 * 1024 * 1024;

/// The masthead keys every document knows. A `Key: Value` line under the
/// title is metadata only when its key is one of these or one the rules file
/// adds. Any other key is not metadata, so `> Short version: …` stays the
/// summary however short it is (decision 0009).
pub const MASTHEAD_KEYS: [&str; 8] = [
    "Status",
    "Date",
    "Supersedes",
    "Superseded by",
    "Owner",
    "Type",
    "Tags",
    "Version",
];

/// The keys whose masthead lines count as metadata: [`MASTHEAD_KEYS`] and
/// the keys a rules file adds. They match like [`Metadata::field`] keys,
/// without regard to case and with `-`, `_` and spaces the same.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MastheadKeys {
    /// Every known key, normalized, sorted and each once.
    keys: Vec<String>,
}

impl Default for MastheadKeys {
    /// [`MASTHEAD_KEYS`] alone.
    fn default() -> MastheadKeys {
        MastheadKeys::with(std::iter::empty::<&str>())
    }
}

impl MastheadKeys {
    /// [`MASTHEAD_KEYS`] and these keys too.
    #[must_use]
    pub fn with<K: AsRef<str>>(extra: impl IntoIterator<Item = K>) -> MastheadKeys {
        let mut keys: Vec<String> = MASTHEAD_KEYS
            .iter()
            .map(|key| normalize_key(key))
            .chain(extra.into_iter().map(|key| normalize_key(key.as_ref())))
            .collect();
        keys.sort_unstable();
        keys.dedup();
        MastheadKeys { keys }
    }

    /// Whether a masthead line with this key is metadata.
    #[must_use]
    pub fn knows(&self, key: &str) -> bool {
        self.keys.binary_search(&normalize_key(key)).is_ok()
    }
}

/// One Markdown file, parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    /// The file's text. Every block's range indexes into it.
    pub source: String,
    /// The blocks, in source order. Their ranges never overlap.
    pub blocks: Vec<Block>,
    /// The title, summary and key-value fields.
    pub meta: Metadata,
}

/// One unit of a document that the change view compares and a note can
/// attach to: a paragraph, a heading, a list item, a table row and so on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    /// What kind of block this is, with the details later parts need.
    pub kind: BlockKind,
    /// The block's source bytes, end exclusive. It starts at the block's first
    /// byte and ends after its last character that is not blank, so it never
    /// ends with a line ending. It always falls on character boundaries.
    pub range: Range<usize>,
    /// The titles of the headings this block sits under, outermost first.
    /// Only headings outside any container count. A heading's own path holds
    /// the headings above it, not itself.
    pub heading_path: Arc<[String]>,
    /// The containers this block sits inside, outermost first.
    pub containers: Arc<[Container]>,
    /// The index of the block of the innermost list item, alert or footnote
    /// definition this block sits inside. Block quotes have no block of their
    /// own, so they are never a parent.
    pub parent: Option<usize>,
    /// The rendered text without markup, for comparing words.
    pub text: String,
    /// The stretches of `text` that came from links, images and code spans,
    /// in the order they end, so a span inside a link comes before the link.
    /// Most blocks have none.
    pub spans: Vec<TextSpan>,
    /// A hash of the exact source bytes in `range`.
    pub source_hash: u64,
    /// A hash of the block's content with formatting removed. Two blocks with
    /// equal content hashes and different source hashes differ in formatting
    /// only.
    pub content_hash: u64,
}

/// A stretch of a block's plain text that came from a link, an image or a
/// code span.
///
/// The plain text drops link targets and code-span backticks. The change view
/// needs both: it keeps such a stretch whole when it marks changed words, and
/// it compares link targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextSpan {
    /// The bytes of the block's `text` the span covers. It is empty for a link
    /// without text.
    pub range: Range<usize>,
    /// What the span came from.
    pub kind: SpanKind,
}

/// What a [`TextSpan`] came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpanKind {
    /// A code span.
    Code,
    /// A link, including an autolink.
    Link {
        /// The target. A reference link's target comes from its definition.
        target: String,
    },
    /// An image. Its text is its alt text.
    Image {
        /// The image's address.
        target: String,
    },
}

/// What a block is.
///
/// In JSON a kind is an object whose `type` names the variant in snake case,
/// with the variant's details beside it: `{"type": "heading", "level": 2}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BlockKind {
    /// YAML front matter between `---` lines at the top of the file.
    FrontMatter,
    /// A heading, written with `#` signs or underlined.
    Heading {
        /// 1 for `#` or a `===` underline, up to 6.
        level: u8,
    },
    /// A paragraph that is not the leading text of a list item, alert or
    /// footnote definition.
    Paragraph,
    /// A list item with its leading paragraphs.
    ListItem(ListItem),
    /// A table row. The header row's range includes the delimiter row under it.
    TableRow {
        /// Whether this is the header row.
        header: bool,
        /// The rendered text of each cell, without markup or padding.
        cells: Vec<String>,
    },
    /// A code block that is not a diagram.
    Code {
        /// The first word of the fence's info string, such as `rust`. `None`
        /// for an indented block or a fence without one.
        language: Option<String>,
        /// Whether the block ends with a closing fence. Indented blocks
        /// always count as closed.
        closed: bool,
    },
    /// A fenced code block that draws a diagram.
    Diagram {
        /// The diagram language.
        language: DiagramLanguage,
        /// Whether the block ends with a closing fence.
        closed: bool,
    },
    /// A GitHub alert such as `> [!WARNING]`, with its leading paragraphs.
    Alert {
        /// Which alert.
        kind: AlertKind,
        /// The title written after the marker, as in `> [!NOTE] Heads up`.
        title: Option<String>,
    },
    /// A footnote definition such as `[^1]: …`, with its leading paragraphs.
    FootnoteDefinition {
        /// The label between `[^` and `]`.
        name: String,
    },
    /// A block of raw HTML, including HTML comments.
    Html,
    /// A thematic break: `---`, `***` or `___` on its own line.
    ThematicBreak,
    /// Source text that renders nothing: link reference definitions, and
    /// footnote definitions that nothing refers to. It is a block so that an
    /// edit to it still shows up as a change.
    Unrendered,
}

/// The details of a list item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ListItem {
    /// How many list items this item sits inside: 0 for a top-level item.
    pub depth: usize,
    /// Whether the list is numbered.
    pub ordered: bool,
    /// The number the page shows for an item of a numbered list: the list's
    /// first number plus the item's position. `None` for a bullet list.
    pub number: Option<usize>,
    /// The checkbox of a task list item.
    pub task: Option<TaskState>,
}

/// The state of a task list item's checkbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    /// `[ ]`
    Open,
    /// `[x]` or `[X]`
    Done,
}

/// The languages driftmark draws as diagrams.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagramLanguage {
    /// A fenced block whose info string starts with `mermaid`.
    Mermaid,
    /// A fenced block whose info string starts with `d2`.
    D2,
}

/// The five GitHub alert kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertKind {
    /// `[!NOTE]`
    Note,
    /// `[!TIP]`
    Tip,
    /// `[!IMPORTANT]`
    Important,
    /// `[!WARNING]`
    Warning,
    /// `[!CAUTION]`
    Caution,
}

impl AlertKind {
    /// The kind as GitHub writes it inside the marker, such as `"NOTE"`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            AlertKind::Note => "NOTE",
            AlertKind::Tip => "TIP",
            AlertKind::Important => "IMPORTANT",
            AlertKind::Warning => "WARNING",
            AlertKind::Caution => "CAUTION",
        }
    }
}

/// A container a block can sit inside.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Container {
    /// A `>` block quote that is not an alert.
    BlockQuote,
    /// A list item. One entry per level of list nesting.
    ListItem,
    /// A GitHub alert.
    Alert,
    /// A footnote definition.
    FootnoteDefinition,
}

/// A document's title, summary and key-value fields.
///
/// Status, date and "superseded by" are not read here: where they live
/// depends on the document's type, which the rules file decides. See
/// [`crate::rules::Rules::classify`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Metadata {
    /// The text of the first level-1 heading outside any container.
    pub title: Option<String>,
    /// The text of the plain-prose quote right under the title.
    pub summary: Option<String>,
    /// The fields of the front matter, then those of the masthead, in written
    /// order. Every front matter key is kept; a masthead key only when it is
    /// a known one ([`MastheadKeys`]).
    pub fields: Vec<Field>,
    /// Why the front matter could not be read, when it could not. The rest of
    /// the document is still parsed.
    pub front_matter_error: Option<String>,
}

impl Metadata {
    /// The first field from `origin` whose key matches `key`. Keys match
    /// without regard to case, and `-`, `_` and spaces count as the same, so
    /// `superseded-by` finds `superseded_by` and `Superseded by`.
    #[must_use]
    pub fn field(&self, origin: FieldOrigin, key: &str) -> Option<&Field> {
        let wanted = normalize_key(key);
        self.fields
            .iter()
            .find(|field| field.origin == origin && normalize_key(&field.key) == wanted)
    }
}

/// A key in the form keys are compared in: lower case, with `-`, `_` and runs
/// of spaces turned into one space.
#[must_use]
pub fn normalize_key(key: &str) -> String {
    key.to_lowercase()
        .replace(['-', '_'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// One key and its value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// The key as written, without bold markers.
    pub key: String,
    /// The value.
    pub value: MetaValue,
    /// Where the field was written.
    pub origin: FieldOrigin,
    /// The targets of the links in a masthead value, in order. Front matter
    /// fields have none.
    pub links: Vec<String>,
}

/// Where a field was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldOrigin {
    /// In the YAML front matter.
    FrontMatter,
    /// In the masthead: the `Key: Value` lines right under the title.
    Masthead,
}

/// A metadata value. Every scalar is text as written: YAML's `007` stays
/// `"007"`, and a masthead value is its rendered text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetaValue {
    /// A single value.
    Text(String),
    /// A YAML sequence.
    List(Vec<MetaValue>),
    /// A YAML mapping, in written order.
    Map(Vec<(String, MetaValue)>),
}

impl MetaValue {
    /// The text of a single value; `None` for a list or a map.
    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match self {
            MetaValue::Text(text) => Some(text),
            MetaValue::List(_) | MetaValue::Map(_) => None,
        }
    }
}

/// Why a file could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// The file is not UTF-8 text.
    NotUtf8 {
        /// The byte offset of the first byte that is not valid UTF-8.
        offset: usize,
        /// The 1-based line that byte is on.
        line: usize,
    },
    /// The file has more than [`MAX_FILE_SIZE`] bytes.
    TooLarge {
        /// The file's size in bytes.
        size: usize,
        /// The limit.
        limit: usize,
    },
    /// Block quotes, lists, alerts and footnotes nest deeper than
    /// [`MAX_DEPTH`].
    TooDeep {
        /// The 1-based line where the nesting goes past the limit.
        line: usize,
        /// The limit.
        limit: usize,
    },
    /// The Markdown parser gave a position outside the file or inside a
    /// character. This is a parser bug, never a fault of the file.
    BadPosition {
        /// The 1-based line comrak named.
        line: usize,
        /// The 1-based byte column comrak named.
        column: usize,
    },
    /// The Markdown parser stopped with an internal error.
    Internal {
        /// The parser's own message.
        message: String,
    },
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::NotUtf8 { offset, line } => write!(
                f,
                "the file is not UTF-8 text: byte {offset}, on line {line}, is not valid UTF-8"
            ),
            ParseError::TooLarge { size, limit } => write!(
                f,
                "the file has {size} bytes, more than the {limit} bytes driftmark reads"
            ),
            ParseError::TooDeep { line, limit } => write!(
                f,
                "on line {line}, block quotes, lists, alerts and footnotes nest more than {limit} levels deep"
            ),
            ParseError::BadPosition { line, column } => write!(
                f,
                "the Markdown parser gave a position outside the file (line {line}, column {column}); this is a bug in the parser"
            ),
            ParseError::Internal { message } => {
                write!(f, "the Markdown parser stopped on this file: {message}")
            }
        }
    }
}

impl std::error::Error for ParseError {}

impl Document {
    /// Parses one Markdown file, reading only [`MASTHEAD_KEYS`] from its
    /// masthead. With a rules file, use [`Document::parse_with`] and the
    /// rules' [`crate::rules::Rules::masthead_keys`].
    ///
    /// # Errors
    ///
    /// As for [`Document::parse_with`].
    pub fn parse(source: Vec<u8>) -> Result<Document, ParseError> {
        Document::parse_with(source, &MastheadKeys::default())
    }

    /// Parses one Markdown file. A masthead line is metadata only when `keys`
    /// knows its key.
    ///
    /// # Errors
    ///
    /// A [`ParseError`] when the file is larger than [`MAX_FILE_SIZE`], when
    /// the bytes are not UTF-8, when blocks nest deeper than [`MAX_DEPTH`], or
    /// when the Markdown parser fails. A parser panic is caught and returned
    /// as [`ParseError::Internal`], so one bad file cannot stop a server.
    ///
    /// The size, and the `>` markers that open each line, are checked before
    /// the parser sees the file. The parser takes time that grows with the
    /// square of the quote markers on one line: 50,000 of them took two
    /// seconds.
    pub fn parse_with(source: Vec<u8>, keys: &MastheadKeys) -> Result<Document, ParseError> {
        if source.len() > MAX_FILE_SIZE {
            return Err(ParseError::TooLarge {
                size: source.len(),
                limit: MAX_FILE_SIZE,
            });
        }
        let source = String::from_utf8(source).map_err(|error| {
            let offset = error.utf8_error().valid_up_to();
            let valid = std::str::from_utf8(&error.as_bytes()[..offset]).unwrap_or("");
            ParseError::NotUtf8 {
                offset,
                line: markdown::LineIndex::new(valid).line_of(offset),
            }
        })?;
        check_quote_markers(&source)?;
        guarded(|| {
            let arena = Arena::new();
            let root = parse_document(&arena, &source, &markdown::options());
            let blocks = blocks::split(&source, root)?;
            let meta = meta::read(root, keys);
            Ok((blocks, meta))
        })
        .map(|(blocks, meta)| Document {
            source,
            blocks,
            meta,
        })
    }

    /// The source text of one of this document's blocks.
    #[must_use]
    pub fn source_of(&self, block: &Block) -> &str {
        self.source.get(block.range.clone()).unwrap_or("")
    }
}

/// Refuses a file with a line that starts with more than [`MAX_DEPTH`] `>`
/// quote markers, before the parser sees it.
///
/// Such a line would nest deeper than the block list allows anyway. Spaces
/// and tabs between the markers count as part of the run. A line inside a code
/// block is counted too, because the check runs before parsing; a code line
/// that starts with more than 100 `>` signs is not expected in real documents.
fn check_quote_markers(source: &str) -> Result<(), ParseError> {
    let bytes = source.as_bytes();
    let mut line = 1;
    let mut markers = 0;
    let mut in_prefix = true;
    for (at, &byte) in bytes.iter().enumerate() {
        match byte {
            // A line ends at a line feed, and at a carriage return that no
            // line feed follows, as in the parser.
            b'\n' | b'\r' => {
                line += usize::from(byte == b'\n' || bytes.get(at + 1) != Some(&b'\n'));
                markers = 0;
                in_prefix = true;
            }
            b'>' if in_prefix => {
                markers += 1;
                if markers > MAX_DEPTH {
                    return Err(ParseError::TooDeep {
                        line,
                        limit: MAX_DEPTH,
                    });
                }
            }
            b' ' | b'\t' => {}
            _ => in_prefix = false,
        }
    }
    Ok(())
}

/// Runs `work`, turning a panic into [`ParseError::Internal`].
fn guarded<T>(work: impl FnOnce() -> Result<T, ParseError>) -> Result<T, ParseError> {
    catch_unwind(AssertUnwindSafe(work)).unwrap_or_else(|payload| {
        Err(ParseError::Internal {
            message: panic_message(payload.as_ref()),
        })
    })
}

/// The message a panic was raised with.
fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "a panic without a message".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_utf8_names_the_byte_and_its_line() {
        let error = Document::parse(b"ok\nalso ok\nbad \xff here".to_vec()).expect_err("not UTF-8");
        assert_eq!(
            error,
            ParseError::NotUtf8 {
                offset: 15,
                line: 3
            }
        );
        assert_eq!(
            error.to_string(),
            "the file is not UTF-8 text: byte 15, on line 3, is not valid UTF-8"
        );
        assert_eq!(
            Document::parse(vec![0xC3]).expect_err("cut character"),
            ParseError::NotUtf8 { offset: 0, line: 1 }
        );
    }

    #[test]
    fn errors_read_as_sentences() {
        assert_eq!(
            ParseError::TooDeep {
                line: 7,
                limit: 100
            }
            .to_string(),
            "on line 7, block quotes, lists, alerts and footnotes nest more than 100 levels deep"
        );
        assert_eq!(
            ParseError::BadPosition { line: 2, column: 9 }.to_string(),
            "the Markdown parser gave a position outside the file (line 2, column 9); this is a bug in the parser"
        );
        assert_eq!(
            ParseError::Internal {
                message: "boom".to_owned()
            }
            .to_string(),
            "the Markdown parser stopped on this file: boom"
        );
    }

    #[test]
    fn a_panic_becomes_an_internal_error() {
        let text: Result<(), ParseError> = guarded(|| panic!("plain text"));
        assert_eq!(
            text,
            Err(ParseError::Internal {
                message: "plain text".to_owned()
            })
        );
        let value = 3;
        let formatted: Result<(), ParseError> = guarded(|| panic!("formatted {value}"));
        assert_eq!(
            formatted,
            Err(ParseError::Internal {
                message: "formatted 3".to_owned()
            })
        );
        let other: Result<(), ParseError> = guarded(|| std::panic::panic_any(7_u8));
        assert_eq!(
            other,
            Err(ParseError::Internal {
                message: "a panic without a message".to_owned()
            })
        );
        assert_eq!(guarded(|| Ok(5)), Ok(5));
    }

    #[test]
    fn too_deep_documents_are_refused() {
        let source = format!("{} x\n", ">".repeat(MAX_DEPTH + 1));
        assert_eq!(
            Document::parse(source.into_bytes()),
            Err(ParseError::TooDeep {
                line: 1,
                limit: MAX_DEPTH
            })
        );
    }

    #[test]
    fn files_over_the_size_limit_are_refused_before_parsing() {
        let mut largest = "word ".repeat(MAX_FILE_SIZE / 5).into_bytes();
        largest.resize(MAX_FILE_SIZE, b'x');
        assert!(Document::parse(largest.clone()).is_ok());
        largest.push(b'x');
        let error = Document::parse(largest).expect_err("too large");
        assert_eq!(
            error,
            ParseError::TooLarge {
                size: MAX_FILE_SIZE + 1,
                limit: MAX_FILE_SIZE
            }
        );
        assert_eq!(
            error.to_string(),
            "the file has 2097153 bytes, more than the 2097152 bytes driftmark reads"
        );
        // Too large comes first, even for bytes that are not UTF-8.
        assert_eq!(
            Document::parse(vec![0xff; MAX_FILE_SIZE + 1]),
            Err(ParseError::TooLarge {
                size: MAX_FILE_SIZE + 1,
                limit: MAX_FILE_SIZE
            })
        );
    }

    #[test]
    fn a_line_with_too_many_quote_markers_is_refused_before_parsing() {
        let limit = ">".repeat(MAX_DEPTH);
        assert!(Document::parse(format!("{limit} x\n").into_bytes()).is_ok());
        let spaced = "> ".repeat(MAX_DEPTH + 1);
        for (source, line) in [
            (format!("{limit}> x"), 1),
            (format!("a\nb\r\nc\rd\n\t{spaced}x\n"), 5),
            (format!("a\r\n\r\n{limit}>"), 3),
        ] {
            assert_eq!(
                check_quote_markers(&source),
                Err(ParseError::TooDeep {
                    line,
                    limit: MAX_DEPTH
                }),
                "{source:?}"
            );
        }
        // Markers after other text, or spread over lines, do not count.
        let after_text = format!("a {limit}>\n");
        assert_eq!(check_quote_markers(&after_text), Ok(()));
        let split_lines = format!("{limit}\n>\r{limit}\r\n>");
        assert_eq!(check_quote_markers(&split_lines), Ok(()));
        assert_eq!(
            Document::parse(format!("x\n{limit}>").into_bytes()),
            Err(ParseError::TooDeep {
                line: 2,
                limit: MAX_DEPTH
            })
        );
    }

    #[test]
    fn nesting_the_quote_check_cannot_see_is_still_refused() {
        // List items nested over many lines hold no `>` sign, so only the
        // block list sees how deep they go.
        let nested = (0..=MAX_DEPTH + 1).fold(String::new(), |mut text, level| {
            text.push_str(&"  ".repeat(level));
            text.push_str("- item\n");
            text
        });
        assert_eq!(check_quote_markers(&nested), Ok(()));
        assert_eq!(
            Document::parse(nested.into_bytes()),
            Err(ParseError::TooDeep {
                line: MAX_DEPTH + 1,
                limit: MAX_DEPTH
            })
        );
    }

    #[test]
    fn source_of_slices_the_block() {
        let document = Document::parse(b"# Title\n\nBody text.\n".to_vec()).expect("parses");
        assert_eq!(document.source_of(&document.blocks[1]), "Body text.");
        let mut outside = document.blocks[1].clone();
        outside.range = 50..60;
        assert_eq!(document.source_of(&outside), "");
    }

    #[test]
    fn field_lookup_ignores_case_and_separators() {
        let document = Document::parse(
            b"---\nsuperseded_by: x\n---\n\n# T\n\n> Superseded by: y | Status: z\n".to_vec(),
        )
        .expect("parses");
        let meta = &document.meta;
        assert_eq!(
            meta.field(FieldOrigin::FrontMatter, "Superseded-By")
                .and_then(|field| field.value.as_text()),
            Some("x")
        );
        assert_eq!(
            meta.field(FieldOrigin::Masthead, "superseded-by")
                .and_then(|field| field.value.as_text()),
            Some("y")
        );
        assert_eq!(meta.field(FieldOrigin::Masthead, "date"), None);
        assert_eq!(normalize_key("  Last__updated - At "), "last updated at");
    }

    #[test]
    fn known_keys_match_like_field_keys() {
        let keys = MastheadKeys::default();
        for key in MASTHEAD_KEYS {
            assert!(keys.knows(key), "{key}");
        }
        assert!(keys.knows("superseded-BY"));
        assert!(!keys.knows("Short version"));
        assert!(!keys.knows("Deciders"));
        let more = MastheadKeys::with(["Deciders", "status"]);
        assert!(more.knows("deciders") && more.knows("Status"));
        assert_eq!(more.keys.len(), MASTHEAD_KEYS.len() + 1);
        let source = b"# T\n\n> Deciders: Ana | Status: Draft\n".to_vec();
        let plain = Document::parse(source.clone()).expect("parses");
        assert_eq!(plain.meta.fields.len(), 1);
        let with = Document::parse_with(source, &more).expect("parses");
        assert_eq!(with.meta.fields.len(), 2);
    }

    #[test]
    fn only_a_single_value_has_text() {
        assert_eq!(MetaValue::Text("a".to_owned()).as_text(), Some("a"));
        assert_eq!(MetaValue::List(Vec::new()).as_text(), None);
        assert_eq!(MetaValue::Map(Vec::new()).as_text(), None);
    }

    #[test]
    fn alert_names_match_the_markers() {
        let names: Vec<&str> = [
            AlertKind::Note,
            AlertKind::Tip,
            AlertKind::Important,
            AlertKind::Warning,
            AlertKind::Caution,
        ]
        .into_iter()
        .map(AlertKind::name)
        .collect();
        assert_eq!(names, ["NOTE", "TIP", "IMPORTANT", "WARNING", "CAUTION"]);
    }
}
