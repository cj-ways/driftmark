//! Ranking by rules (decision 0005: no AI labels). Each change gets the
//! signals that apply to it, and its level is the highest level among them.
//! `docs/features/engine/change-list.md` has the table of signals and levels.

use std::collections::HashMap;

use serde::Serialize;

use super::words::{Key, Token};
use crate::document::{BlockKind, SpanKind, TextSpan};

/// How much a change matters, lowest first, so `Ord` puts the highest last.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    /// Only the way the text is written changed: wrapping, emphasis style,
    /// list markers, nesting.
    Formatting,
    /// Words changed, or a block moved, and no rule below applies.
    Wording,
    /// A heading, a number, a link, a table cell or a code line changed.
    Fact,
    /// A normative word such as `must` or `never` came or went.
    Rule,
    /// A status changed, or the change sits in a decision section.
    Decision,
}

impl Level {
    /// Every level, highest first.
    pub const HIGHEST_FIRST: [Level; 5] = [
        Level::Decision,
        Level::Rule,
        Level::Fact,
        Level::Wording,
        Level::Formatting,
    ];
}

/// One reason a change matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Signal {
    /// The document's status changed, and this block holds it.
    Status,
    /// The block sits, on either side, in a section the rules file marks as a
    /// decision.
    DecisionSection,
    /// A normative word or phrase from [`NORMATIVE_WORDS`] came or went.
    Normative,
    /// A heading was added, removed, renamed or moved to another level.
    Heading,
    /// A number, date, version or identifier came or went: a word with a
    /// digit, or a code span.
    Number,
    /// A link or image target came, went or changed.
    Link,
    /// A table row was added, removed or edited.
    TableCell,
    /// A code or diagram block was added, removed or edited.
    CodeLine,
    /// The block moved to another place in the document.
    Moved,
    /// Words changed, and none of the content signals above apply.
    Wording,
    /// Only formatting changed.
    Formatting,
}

impl Signal {
    /// The level this signal gives a change.
    #[must_use]
    pub fn level(self) -> Level {
        match self {
            Signal::Status | Signal::DecisionSection => Level::Decision,
            Signal::Normative => Level::Rule,
            Signal::Heading
            | Signal::Number
            | Signal::Link
            | Signal::TableCell
            | Signal::CodeLine => Level::Fact,
            Signal::Moved | Signal::Wording => Level::Wording,
            Signal::Formatting => Level::Formatting,
        }
    }
}

/// The level of a change with these signals: the highest any of them gives.
/// A change always has at least one signal; with none it would be formatting.
pub(crate) fn level_of(signals: &[Signal]) -> Level {
    signals
        .iter()
        .map(|signal| signal.level())
        .max()
        .unwrap_or(Level::Formatting)
}

/// The words and phrases that make a sentence a rule, compared without regard
/// to case. Two-word phrases come before their first word, so `must not` is
/// read as one phrase and not as `must`.
///
/// The brief named `must`, `must not`, `never`, `always`, `only`, `required`,
/// `forbidden`, `should` and `shall`. `should not` and `shall not` are added
/// so that turning `should` into `should not` counts, like `must not` does.
pub const NORMATIVE_WORDS: &[&str] = &[
    "must not",
    "should not",
    "shall not",
    "must",
    "never",
    "always",
    "only",
    "required",
    "forbidden",
    "should",
    "shall",
];

/// How often each normative word or phrase occurs in the words of a block,
/// in the order of [`NORMATIVE_WORDS`]. Words inside code spans and links do
/// not count. No phrase's second word is itself in the list, so a phrase never
/// hides a word after it.
pub(crate) fn normative_counts(tokens: &[Token<'_>]) -> Vec<usize> {
    let words: Vec<&str> = tokens
        .iter()
        .filter_map(|token| match token.key {
            Key::Word(word) => Some(word),
            _ => None,
        })
        .collect();
    let mut counts = vec![0; NORMATIVE_WORDS.len()];
    for (at, word) in words.iter().enumerate() {
        let found = NORMATIVE_WORDS
            .iter()
            .position(|entry| match entry.split_once(' ') {
                Some((first, second)) => {
                    word.eq_ignore_ascii_case(first)
                        && words
                            .get(at + 1)
                            .is_some_and(|next| next.eq_ignore_ascii_case(second))
                }
                None => word.eq_ignore_ascii_case(entry),
            });
        if let Some(index) = found {
            counts[index] += 1;
        }
    }
    counts
}

/// Whether a block that was added or removed holds a normative word. Every
/// phrase in [`NORMATIVE_WORDS`] starts with a word that is in the list on
/// its own, so looking for the single words finds the phrases too.
pub(crate) fn has_normative(tokens: &[Token<'_>]) -> bool {
    tokens.iter().any(|token| match token.key {
        Key::Word(word) => NORMATIVE_WORDS
            .iter()
            .any(|entry| !entry.contains(' ') && word.eq_ignore_ascii_case(entry)),
        _ => false,
    })
}

/// Whether a token is a number, date, version or identifier: a word that
/// holds a digit, or a code span.
fn is_fact(token: &Token<'_>) -> bool {
    match token.key {
        Key::Code(_) => true,
        Key::Word(text) | Key::Link(text, _) | Key::Image(text, _) => {
            text.bytes().any(|byte| byte.is_ascii_digit())
        }
        Key::Space | Key::Tab | Key::Break | Key::Other(_) => false,
    }
}

/// The numbers, dates, versions and identifiers among a block's tokens, with
/// how often each occurs.
pub(crate) fn facts<'t>(tokens: &[Token<'t>]) -> HashMap<&'t str, usize> {
    let mut found = HashMap::new();
    for token in tokens.iter().filter(|token| is_fact(token)) {
        *found.entry(token.text).or_insert(0) += 1;
    }
    found
}

/// Whether a block that was added or removed holds a number, date, version
/// or identifier.
pub(crate) fn has_fact(tokens: &[Token<'_>]) -> bool {
    tokens.iter().any(is_fact)
}

/// The link and image targets of a block, with how often each occurs. Links
/// without text count too.
pub(crate) fn targets(spans: &[TextSpan]) -> HashMap<&str, usize> {
    let mut found = HashMap::new();
    for span in spans {
        match &span.kind {
            SpanKind::Link { target } | SpanKind::Image { target } => {
                *found.entry(target.as_str()).or_insert(0) += 1;
            }
            SpanKind::Code => {}
        }
    }
    found
}

/// Whether a block that was added or removed holds a link or an image.
pub(crate) fn has_target(spans: &[TextSpan]) -> bool {
    spans
        .iter()
        .any(|span| !matches!(span.kind, SpanKind::Code))
}

/// The signals that come from a block's own kind, whatever changed in it.
pub(crate) fn kind_signals(kind: &BlockKind, signals: &mut Vec<Signal>) {
    match kind {
        BlockKind::Heading { .. } => signals.push(Signal::Heading),
        BlockKind::TableRow { .. } => signals.push(Signal::TableCell),
        BlockKind::Code { .. } | BlockKind::Diagram { .. } => signals.push(Signal::CodeLine),
        _ => {}
    }
}

/// Whether a change with these signals changed only words: none of the
/// signals that name what kind of content changed applies.
pub(crate) fn wording_only(signals: &[Signal]) -> bool {
    !signals.iter().any(|signal| {
        matches!(
            signal,
            Signal::Status
                | Signal::Normative
                | Signal::Heading
                | Signal::Number
                | Signal::Link
                | Signal::TableCell
                | Signal::CodeLine
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::words::tokens;
    use crate::document::{DiagramLanguage, ListItem};

    #[test]
    fn every_signal_has_its_level() {
        let table = [
            (Signal::Status, Level::Decision),
            (Signal::DecisionSection, Level::Decision),
            (Signal::Normative, Level::Rule),
            (Signal::Heading, Level::Fact),
            (Signal::Number, Level::Fact),
            (Signal::Link, Level::Fact),
            (Signal::TableCell, Level::Fact),
            (Signal::CodeLine, Level::Fact),
            (Signal::Moved, Level::Wording),
            (Signal::Wording, Level::Wording),
            (Signal::Formatting, Level::Formatting),
        ];
        for (signal, level) in table {
            assert_eq!(signal.level(), level, "{signal:?}");
        }
    }

    #[test]
    fn the_level_is_the_highest_signal() {
        assert_eq!(level_of(&[Signal::Wording, Signal::Number]), Level::Fact);
        assert_eq!(
            level_of(&[Signal::Normative, Signal::DecisionSection]),
            Level::Decision
        );
        assert_eq!(level_of(&[]), Level::Formatting);
        assert_eq!(Level::HIGHEST_FIRST[0], Level::Decision);
        assert!(Level::Rule > Level::Fact && Level::Wording > Level::Formatting);
    }

    #[test]
    fn normative_phrases_count_once() {
        let text = "You MUST NOT skip it; you must test, should not guess, Shall not wait, only never always required forbidden should shall.";
        let counts = normative_counts(&tokens(text, &[]));
        assert_eq!(counts, [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1]);
        assert_eq!(
            normative_counts(&tokens("must", &[])),
            [0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(
            normative_counts(&tokens("not must", &[])),
            [0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(normative_counts(&tokens("mustard", &[])), [0; 11]);
    }

    #[test]
    fn code_and_links_hold_no_normative_words() {
        let spans = [
            TextSpan {
                range: 0..4,
                kind: SpanKind::Code,
            },
            TextSpan {
                range: 5..10,
                kind: SpanKind::Link {
                    target: "a.md".to_owned(),
                },
            },
        ];
        assert_eq!(normative_counts(&tokens("only never", &spans)), [0; 11]);
    }

    #[test]
    fn facts_are_words_with_digits_and_code_spans() {
        let spans = [
            TextSpan {
                range: 0..5,
                kind: SpanKind::Code,
            },
            TextSpan {
                range: 6..12,
                kind: SpanKind::Link {
                    target: "a.md".to_owned(),
                },
            },
            TextSpan {
                range: 13..17,
                kind: SpanKind::Image {
                    target: "b.png".to_owned(),
                },
            },
        ];
        let found = facts(&tokens(
            "cargo ADR 62 fig1 at 30 or v1.2, 30 days\tx\n.",
            &spans,
        ));
        let mut found: Vec<(&str, usize)> = found.into_iter().collect();
        found.sort_unstable();
        assert_eq!(
            found,
            [
                ("30", 2),
                ("ADR 62", 1),
                ("cargo", 1),
                ("fig1", 1),
                ("v1.2", 1)
            ]
        );
        let plain_link = [TextSpan {
            range: 0..4,
            kind: SpanKind::Link {
                target: "a.md".to_owned(),
            },
        }];
        assert!(facts(&tokens("docs", &plain_link)).is_empty());
    }

    #[test]
    fn added_blocks_only_ask_whether_a_signal_is_there() {
        assert!(has_normative(&tokens("Agents Must run it", &[])));
        assert!(has_normative(&tokens("you should not", &[])));
        assert!(!has_normative(&tokens("not mustard", &[])));
        assert!(has_fact(&tokens("in 3 steps", &[])));
        assert!(!has_fact(&tokens("in three steps", &[])));
        let code = TextSpan {
            range: 0..1,
            kind: SpanKind::Code,
        };
        let image = TextSpan {
            range: 1..2,
            kind: SpanKind::Image {
                target: "a.png".to_owned(),
            },
        };
        assert!(has_fact(&tokens("x", std::slice::from_ref(&code))));
        assert!(!has_target(std::slice::from_ref(&code)));
        assert!(has_target(&[code, image]));
        assert!(!has_target(&[]));
    }

    #[test]
    fn targets_count_links_and_images_but_not_code() {
        let spans = [
            TextSpan {
                range: 0..1,
                kind: SpanKind::Link {
                    target: "a.md".to_owned(),
                },
            },
            TextSpan {
                range: 1..1,
                kind: SpanKind::Link {
                    target: "a.md".to_owned(),
                },
            },
            TextSpan {
                range: 2..3,
                kind: SpanKind::Image {
                    target: "b.png".to_owned(),
                },
            },
            TextSpan {
                range: 3..4,
                kind: SpanKind::Code,
            },
        ];
        let mut found: Vec<(&str, usize)> = targets(&spans).into_iter().collect();
        found.sort_unstable();
        assert_eq!(found, [("a.md", 2), ("b.png", 1)]);
    }

    #[test]
    fn some_kinds_carry_their_own_signal() {
        let signals_of = |kind: BlockKind| {
            let mut signals = Vec::new();
            kind_signals(&kind, &mut signals);
            signals
        };
        assert_eq!(
            signals_of(BlockKind::Heading { level: 2 }),
            [Signal::Heading]
        );
        assert_eq!(
            signals_of(BlockKind::TableRow {
                header: false,
                cells: Vec::new()
            }),
            [Signal::TableCell]
        );
        assert_eq!(
            signals_of(BlockKind::Code {
                language: None,
                closed: true
            }),
            [Signal::CodeLine]
        );
        assert_eq!(
            signals_of(BlockKind::Diagram {
                language: DiagramLanguage::Mermaid,
                closed: true
            }),
            [Signal::CodeLine]
        );
        assert_eq!(signals_of(BlockKind::Paragraph), []);
        assert_eq!(
            signals_of(BlockKind::ListItem(ListItem {
                depth: 0,
                ordered: false,
                number: None,
                task: None
            })),
            []
        );
    }

    #[test]
    fn wording_only_means_no_content_signal() {
        assert!(wording_only(&[]));
        assert!(wording_only(&[Signal::DecisionSection, Signal::Moved]));
        for signal in [
            Signal::Status,
            Signal::Normative,
            Signal::Heading,
            Signal::Number,
            Signal::Link,
            Signal::TableCell,
            Signal::CodeLine,
        ] {
            assert!(!wording_only(&[signal]), "{signal:?}");
        }
    }
}
