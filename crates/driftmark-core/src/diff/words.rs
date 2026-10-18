//! What the change view marks inside an edited block: words, sentences and
//! code lines.
//!
//! Everything here works on a block's plain text, the text a reader sees. The
//! text is cut into tokens that, joined again, give the text back exactly, so
//! marks never change what the text says. The guard in [`keeps_new_text`]
//! checks that anyway.

use std::ops::Range;

use serde::Serialize;

use super::align::align;
use crate::document::{SpanKind, TextSpan};

/// What happened to one stretch of text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkOp {
    /// The text is on both sides.
    Kept,
    /// The text is only on the old side.
    Deleted,
    /// The text is only on the new side.
    Inserted,
}

/// One stretch of text inside an edited block, with what happened to it.
///
/// Kept and inserted stretches, joined, give the new text exactly. Kept and
/// deleted ones, joined, give the old text, except that kept white space is
/// shown as the new side writes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Mark {
    /// What happened.
    pub op: MarkOp,
    /// The text, as the side it belongs to writes it.
    pub text: String,
}

/// What a token is, for comparing: two tokens match when their keys are
/// equal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Key<'t> {
    /// A word: letters, digits and `_`, with joiners such as `.` or `-`
    /// inside it, as in `0.85` or `read-only`.
    Word(&'t str),
    /// A code span, kept whole.
    Code(&'t str),
    /// A link's text with its target, kept whole. A new target with the same
    /// text is a change.
    Link(&'t str, &'t str),
    /// An image's alt text with its address, kept whole.
    Image(&'t str, &'t str),
    /// White space within a line. Its width does not matter.
    Space,
    /// A tab, which separates table cells.
    Tab,
    /// A line break, or the empty line between two paragraphs of one item.
    Break,
    /// Any other character, such as punctuation or an emoji, with the marks
    /// that join it.
    Other(&'t str),
}

/// One piece of a block's plain text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Token<'t> {
    /// The text as written.
    pub(crate) text: &'t str,
    /// What it is.
    pub(crate) key: Key<'t>,
}

impl Token<'_> {
    /// Whether the token counts as a word when measuring how much was kept:
    /// words, code spans, links and images do; white space and punctuation do
    /// not.
    pub(crate) fn is_word(&self) -> bool {
        matches!(
            self.key,
            Key::Word(_) | Key::Code(_) | Key::Link(..) | Key::Image(..)
        )
    }
}

/// Cuts plain text into tokens. Links, images and code spans in `spans` stay
/// whole; where they nest, the outer one wins. Joined, the tokens give `text`
/// back exactly.
pub(crate) fn tokens<'t>(text: &'t str, spans: &'t [TextSpan]) -> Vec<Token<'t>> {
    let mut whole: Vec<&TextSpan> = spans
        .iter()
        .filter(|span| !span.range.is_empty() && text.get(span.range.clone()).is_some())
        .collect();
    whole.sort_by_key(|span| (span.range.start, std::cmp::Reverse(span.range.end)));
    let mut out = Vec::new();
    let mut at = 0;
    for span in whole {
        if span.range.start < at {
            continue;
        }
        plain_tokens(&text[at..span.range.start], &mut out);
        let inner = &text[span.range.clone()];
        let key = match &span.kind {
            SpanKind::Code => Key::Code(inner),
            SpanKind::Link { target } => Key::Link(inner, target),
            SpanKind::Image { target } => Key::Image(inner, target),
        };
        out.push(Token { text: inner, key });
        at = span.range.end;
    }
    plain_tokens(&text[at..], &mut out);
    out
}

/// Cuts text without spans into words, white space and other characters.
fn plain_tokens<'t>(text: &'t str, out: &mut Vec<Token<'t>>) {
    let mut rest = text;
    while let Some(first) = rest.chars().next() {
        let length = if first.is_whitespace() {
            rest.find(|c: char| !c.is_whitespace())
                .unwrap_or(rest.len())
        } else if is_word_char(first) {
            word_length(rest)
        } else {
            symbol_length(rest)
        };
        let piece = &rest[..length];
        let key = if first.is_whitespace() {
            if piece.contains('\n') {
                Key::Break
            } else if piece.contains('\t') {
                Key::Tab
            } else {
                Key::Space
            }
        } else if is_word_char(first) {
            Key::Word(piece)
        } else {
            Key::Other(piece)
        };
        out.push(Token { text: piece, key });
        rest = &rest[length..];
    }
}

/// Letters, digits, `_` and the combining marks that sit on letters.
fn is_word_char(character: char) -> bool {
    character.is_alphanumeric() || character == '_' || is_combining(character)
}

/// Combining diacritical marks, which belong to the character before them.
fn is_combining(character: char) -> bool {
    matches!(character, '\u{0300}'..='\u{036F}' | '\u{1AB0}'..='\u{1AFF}' | '\u{1DC0}'..='\u{1DFF}' | '\u{20D0}'..='\u{20FF}' | '\u{FE20}'..='\u{FE2F}')
}

/// Characters that join two words into one when they sit between them, so
/// `0.85`, `2026-10-18`, `read-only`, `don't` and `docs/a.md` stay whole.
fn is_joiner(character: char) -> bool {
    matches!(
        character,
        '.' | ',' | ':' | '/' | '-' | '\'' | '\u{2019}' | '@'
    )
}

/// The length of the word at the start of `text`.
fn word_length(text: &str) -> usize {
    let mut end = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((at, character)) = chars.next() {
        if is_word_char(character) {
            end = at + character.len_utf8();
        } else if !is_joiner(character) || !chars.peek().is_some_and(|&(_, c)| is_word_char(c)) {
            break;
        }
    }
    end
}

/// The length of the symbol at the start of `text`, with the characters that
/// join it: variation selectors, skin tones, combining marks, a zero-width
/// joiner and the character after it, and the second half of a flag.
fn symbol_length(text: &str) -> usize {
    let mut chars = text.char_indices();
    let Some((_, first)) = chars.next() else {
        return 0;
    };
    let mut end = first.len_utf8();
    let mut flag_half = is_regional_indicator(first);
    while let Some((at, character)) = chars.next() {
        if is_variation_or_tone(character) || is_combining(character) {
            end = at + character.len_utf8();
        } else if character == '\u{200D}' {
            match chars.next() {
                Some((next_at, next)) => end = next_at + next.len_utf8(),
                None => end = at + character.len_utf8(),
            }
        } else if flag_half && is_regional_indicator(character) {
            end = at + character.len_utf8();
            flag_half = false;
        } else {
            break;
        }
    }
    end
}

/// Variation selectors and emoji skin tones.
fn is_variation_or_tone(character: char) -> bool {
    matches!(character, '\u{FE00}'..='\u{FE0F}' | '\u{1F3FB}'..='\u{1F3FF}')
}

/// The letters that pair up into flag emoji.
fn is_regional_indicator(character: char) -> bool {
    matches!(character, '\u{1F1E6}'..='\u{1F1FF}')
}

/// The marks between two token lists, and the tokens each side lost or
/// gained.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WordDiff<'t> {
    /// The marks, in order.
    pub(crate) marks: Vec<Mark>,
    /// The share of words kept in the same order, measured on the longer
    /// side. 1 when neither side has a word.
    pub(crate) kept_in_order: f64,
    /// The old tokens that are not kept.
    pub(crate) deleted: Vec<Token<'t>>,
    /// The new tokens that are not kept.
    pub(crate) inserted: Vec<Token<'t>>,
}

/// One step of a word diff before it becomes marks.
#[derive(Clone, Copy)]
enum Step<'t> {
    Kept(Token<'t>, Token<'t>),
    Deleted(Token<'t>),
    Inserted(Token<'t>),
}

/// Compares two token lists word by word.
pub(crate) fn diff_words<'t>(old: &[Token<'t>], new: &[Token<'t>]) -> WordDiff<'t> {
    let old_keys: Vec<Key<'t>> = old.iter().map(|token| token.key).collect();
    let new_keys: Vec<Key<'t>> = new.iter().map(|token| token.key).collect();
    let mut steps = Vec::with_capacity(old.len().max(new.len()));
    let (mut o, mut n) = (0, 0);
    let mut kept_words = 0_usize;
    for (old_at, new_at) in align(&old_keys, &new_keys) {
        steps.extend(old[o..old_at].iter().map(|&token| Step::Deleted(token)));
        steps.extend(new[n..new_at].iter().map(|&token| Step::Inserted(token)));
        steps.push(Step::Kept(old[old_at], new[new_at]));
        kept_words += usize::from(new[new_at].is_word());
        o = old_at + 1;
        n = new_at + 1;
    }
    steps.extend(old[o..].iter().map(|&token| Step::Deleted(token)));
    steps.extend(new[n..].iter().map(|&token| Step::Inserted(token)));
    let deleted = old_only(&steps);
    let inserted = new_only(&steps);
    let longer = count_words(old).max(count_words(new));
    let kept_in_order = if longer == 0 {
        1.0
    } else {
        ratio(kept_words, longer)
    };
    WordDiff {
        marks: marks_of(&absorb_spaces(&steps)),
        kept_in_order,
        deleted,
        inserted,
    }
}

/// `part / whole` as a fraction. Counts stay far below 2^52, where `f64`
/// would round them.
#[expect(clippy::cast_precision_loss, reason = "counts are small")]
pub(crate) fn ratio(part: usize, whole: usize) -> f64 {
    part as f64 / whole as f64
}

fn count_words(tokens: &[Token<'_>]) -> usize {
    tokens.iter().filter(|token| token.is_word()).count()
}

fn old_only<'t>(steps: &[Step<'t>]) -> Vec<Token<'t>> {
    steps
        .iter()
        .filter_map(|step| match step {
            Step::Deleted(token) => Some(*token),
            Step::Kept(..) | Step::Inserted(_) => None,
        })
        .collect()
}

fn new_only<'t>(steps: &[Step<'t>]) -> Vec<Token<'t>> {
    steps
        .iter()
        .filter_map(|step| match step {
            Step::Inserted(token) => Some(*token),
            Step::Kept(..) | Step::Deleted(_) => None,
        })
        .collect()
}

/// Turns white space kept between two changes into a change, so that
/// `the quick brown` replaced by `a slow red` reads as one change instead of
/// three. Words and punctuation stay kept.
fn absorb_spaces<'t>(steps: &[Step<'t>]) -> Vec<Step<'t>> {
    let is_space = |token: &Token<'_>| matches!(token.key, Key::Space | Key::Tab | Key::Break);
    let mut out = Vec::with_capacity(steps.len());
    let mut at = 0;
    while at < steps.len() {
        let run = steps[at..]
            .iter()
            .take_while(|step| matches!(step, Step::Kept(..)))
            .count();
        if run == 0 {
            out.push(steps[at]);
            at += 1;
            continue;
        }
        let between_changes = at > 0 && at + run < steps.len();
        let only_space = steps[at..at + run]
            .iter()
            .all(|step| matches!(step, Step::Kept(_, new) if is_space(new)));
        for step in &steps[at..at + run] {
            match *step {
                Step::Kept(old, new) if between_changes && only_space => {
                    out.push(Step::Deleted(old));
                    out.push(Step::Inserted(new));
                }
                other => out.push(other),
            }
        }
        at += run;
    }
    out
}

/// Joins steps into marks: each run of kept tokens is one kept mark, and each
/// run of changes is one deleted mark followed by one inserted mark.
fn marks_of(steps: &[Step<'_>]) -> Vec<Mark> {
    let mut marks = Vec::new();
    let mut deleted = String::new();
    let mut inserted = String::new();
    let mut kept = String::new();
    for step in steps {
        match step {
            Step::Kept(_, new) => {
                flush_change(&mut marks, &mut deleted, &mut inserted);
                kept.push_str(new.text);
            }
            Step::Deleted(old) => {
                flush(&mut marks, MarkOp::Kept, &mut kept);
                deleted.push_str(old.text);
            }
            Step::Inserted(new) => {
                flush(&mut marks, MarkOp::Kept, &mut kept);
                inserted.push_str(new.text);
            }
        }
    }
    flush_change(&mut marks, &mut deleted, &mut inserted);
    flush(&mut marks, MarkOp::Kept, &mut kept);
    marks
}

fn flush_change(marks: &mut Vec<Mark>, deleted: &mut String, inserted: &mut String) {
    flush(marks, MarkOp::Deleted, deleted);
    flush(marks, MarkOp::Inserted, inserted);
}

fn flush(marks: &mut Vec<Mark>, op: MarkOp, text: &mut String) {
    if !text.is_empty() {
        marks.push(Mark {
            op,
            text: std::mem::take(text),
        });
    }
}

/// The guard: the kept and inserted marks, joined, must give the new text
/// exactly. If they do not, the marks would show something the document does
/// not say, and the change view shows the two versions whole instead.
pub(crate) fn keeps_new_text<'m>(
    marks: impl IntoIterator<Item = &'m Mark>,
    new_text: &str,
) -> bool {
    let mut rest = new_text;
    for mark in marks {
        if mark.op != MarkOp::Deleted {
            match rest.strip_prefix(mark.text.as_str()) {
                Some(after) => rest = after,
                None => return false,
            }
        }
    }
    rest.is_empty()
}

/// The sentences of a text, as byte ranges that cover it all. Each sentence
/// keeps the white space after it.
///
/// A sentence ends after `.`, `!`, `?` or `…`, and any closing quotes or
/// brackets, when white space follows and the next word does not start with a
/// lower-case letter. So `e.g. this` stays one sentence. The full-width stops
/// `。`, `！` and `？` end a sentence without white space, as Chinese and
/// Japanese write them. The empty line between two paragraphs of a list item
/// always ends one.
pub(crate) fn sentences(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((_, character)) = chars.next() {
        let ends_paragraph = character == '\n' && chars.peek().is_some_and(|&(_, c)| c == '\n');
        if !(is_terminator(character) || ends_paragraph) {
            continue;
        }
        while let Some(&(_, next)) = chars.peek() {
            if is_terminator(next) || is_closer(next) {
                chars.next();
            } else {
                break;
            }
        }
        let after = chars.peek().map_or(text.len(), |&(next_at, _)| next_at);
        let rest = &text[after..];
        let gap = rest.len() - rest.trim_start().len();
        let next = rest.trim_start().chars().next();
        // At the end of the text, the last sentence is pushed after the loop.
        let breaks = ends_paragraph
            || is_full_width_stop(character)
            || (gap > 0 && next.is_some_and(|next| !next.is_lowercase()));
        if breaks {
            let end = after + gap;
            out.push(start..end);
            start = end;
            while chars.peek().is_some_and(|&(next_at, _)| next_at < end) {
                chars.next();
            }
        }
    }
    if start < text.len() {
        out.push(start..text.len());
    }
    out
}

fn is_terminator(character: char) -> bool {
    matches!(character, '.' | '!' | '?' | '\u{2026}') || is_full_width_stop(character)
}

fn is_full_width_stop(character: char) -> bool {
    matches!(character, '\u{3002}' | '\u{FF01}' | '\u{FF1F}')
}

fn is_closer(character: char) -> bool {
    matches!(
        character,
        '"' | '\'' | ')' | ']' | '\u{201D}' | '\u{2019}' | '\u{BB}'
    )
}

/// The spans that lie inside `range`, moved to count from its start.
pub(crate) fn spans_within(spans: &[TextSpan], range: &Range<usize>) -> Vec<TextSpan> {
    spans
        .iter()
        .filter(|span| range.start <= span.range.start && span.range.end <= range.end)
        .map(|span| TextSpan {
            range: span.range.start - range.start..span.range.end - range.start,
            kind: span.kind.clone(),
        })
        .collect()
}

/// The lines of two code blocks, each marked kept, deleted or inserted, and
/// the share of lines kept in order, measured on the longer side.
pub(crate) fn diff_lines(old: &str, new: &str) -> (Vec<Mark>, f64) {
    let old_lines: Vec<&str> = old.split('\n').collect();
    let new_lines: Vec<&str> = new.split('\n').collect();
    let mut marks = Vec::new();
    let (mut o, mut n) = (0, 0);
    let line = |op: MarkOp, text: &str| Mark {
        op,
        text: text.to_owned(),
    };
    let pairs = align(&old_lines, &new_lines);
    let kept = pairs.len();
    for (old_at, new_at) in pairs {
        marks.extend(
            old_lines[o..old_at]
                .iter()
                .map(|text| line(MarkOp::Deleted, text)),
        );
        marks.extend(
            new_lines[n..new_at]
                .iter()
                .map(|text| line(MarkOp::Inserted, text)),
        );
        marks.push(line(MarkOp::Kept, new_lines[new_at]));
        o = old_at + 1;
        n = new_at + 1;
    }
    marks.extend(
        old_lines[o..]
            .iter()
            .map(|text| line(MarkOp::Deleted, text)),
    );
    marks.extend(
        new_lines[n..]
            .iter()
            .map(|text| line(MarkOp::Inserted, text)),
    );
    let share = ratio(kept, old_lines.len().max(new_lines.len()));
    (marks, share)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(range: Range<usize>, kind: SpanKind) -> TextSpan {
        TextSpan { range, kind }
    }

    fn link(target: &str) -> SpanKind {
        SpanKind::Link {
            target: target.to_owned(),
        }
    }

    fn texts<'t>(tokens: &[Token<'t>]) -> Vec<&'t str> {
        tokens.iter().map(|token| token.text).collect()
    }

    fn plain(text: &str) -> Vec<Token<'_>> {
        tokens(text, &[])
    }

    fn shown(marks: &[Mark]) -> String {
        marks
            .iter()
            .map(|mark| match mark.op {
                MarkOp::Kept => mark.text.clone(),
                MarkOp::Deleted => format!("[-{}-]", mark.text),
                MarkOp::Inserted => format!("{{+{}+}}", mark.text),
            })
            .collect()
    }

    #[test]
    fn words_keep_their_joiners_and_split_at_everything_else() {
        assert_eq!(
            texts(&plain(
                "Keep 0.85, read-only (don't) docs/a.md at 2026-10-18."
            )),
            [
                "Keep",
                " ",
                "0.85",
                ",",
                " ",
                "read-only",
                " ",
                "(",
                "don't",
                ")",
                " ",
                "docs/a.md",
                " ",
                "at",
                " ",
                "2026-10-18",
                "."
            ]
        );
        assert_eq!(texts(&plain("a-")), ["a", "-"]);
        assert_eq!(texts(&plain("a--b")), ["a", "-", "-", "b"]);
        assert_eq!(texts(&plain("snake_case x’y")), ["snake_case", " ", "x’y"]);
        assert_eq!(texts(&plain("ქართული ტექსტი")), ["ქართული", " ", "ტექსტი"]);
        assert_eq!(texts(&plain("cafe\u{301} ok")), ["cafe\u{301}", " ", "ok"]);
    }

    #[test]
    fn white_space_keys_tell_spaces_tabs_and_breaks_apart() {
        let keys: Vec<Key<'_>> = plain("a  b\tc\nd \n\ne")
            .into_iter()
            .map(|token| token.key)
            .collect();
        assert_eq!(
            keys,
            [
                Key::Word("a"),
                Key::Space,
                Key::Word("b"),
                Key::Tab,
                Key::Word("c"),
                Key::Break,
                Key::Word("d"),
                Key::Break,
                Key::Word("e"),
            ]
        );
    }

    #[test]
    fn symbols_keep_the_characters_that_join_them() {
        assert_eq!(texts(&plain("✅\u{FE0F}x")), ["✅\u{FE0F}", "x"]);
        assert_eq!(
            texts(&plain("👩\u{200D}💻 dev")),
            ["👩\u{200D}💻", " ", "dev"]
        );
        assert_eq!(texts(&plain("👋\u{1F3FD}!")), ["👋\u{1F3FD}", "!"]);
        assert_eq!(texts(&plain("🇬🇪🇬🇪")), ["🇬🇪", "🇬🇪"]);
        assert_eq!(texts(&plain("!\u{301}?")), ["!\u{301}", "?"]);
        assert_eq!(texts(&plain("!\u{200D}")), ["!\u{200D}"]);
        assert_eq!(symbol_length(""), 0);
    }

    #[test]
    fn links_images_and_code_spans_stay_whole() {
        let text = "see the setup guide and cargo build, logo";
        let spans = [
            span(24..35, SpanKind::Code),
            span(8..19, link("a.md")),
            span(
                37..41,
                SpanKind::Image {
                    target: "l.png".to_owned(),
                },
            ),
            // Nested inside the link, so the link wins.
            span(8..13, SpanKind::Code),
            // Empty, or outside the text: ignored.
            span(3..3, link("x.md")),
            span(40..90, SpanKind::Code),
        ];
        let found = tokens(text, &spans);
        assert_eq!(
            texts(&found),
            [
                "see",
                " ",
                "the",
                " ",
                "setup guide",
                " ",
                "and",
                " ",
                "cargo build",
                ",",
                " ",
                "logo"
            ]
        );
        assert_eq!(found[4].key, Key::Link("setup guide", "a.md"));
        assert_eq!(found[8].key, Key::Code("cargo build"));
        assert_eq!(found[11].key, Key::Image("logo", "l.png"));
        assert!(found.iter().all(|token| !token.text.is_empty()));
        assert_eq!(texts(&found).concat(), text);
    }

    #[test]
    fn spans_that_overlap_keep_the_first() {
        let spans = [span(0..3, SpanKind::Code), span(2..5, SpanKind::Code)];
        assert_eq!(texts(&tokens("abcde", &spans)), ["abc", "de"]);
    }

    #[test]
    fn only_words_spans_and_images_count_as_words() {
        let tokens = [
            Token {
                text: "a",
                key: Key::Word("a"),
            },
            Token {
                text: "b",
                key: Key::Code("b"),
            },
            Token {
                text: "c",
                key: Key::Link("c", "x"),
            },
            Token {
                text: "d",
                key: Key::Image("d", "y"),
            },
            Token {
                text: " ",
                key: Key::Space,
            },
            Token {
                text: "\t",
                key: Key::Tab,
            },
            Token {
                text: "\n",
                key: Key::Break,
            },
            Token {
                text: ".",
                key: Key::Other("."),
            },
        ];
        let words: Vec<bool> = tokens.iter().map(Token::is_word).collect();
        assert_eq!(words, [true, true, true, true, false, false, false, false]);
        assert_eq!(count_words(&tokens), 4);
    }

    #[test]
    fn word_marks_show_what_changed() {
        let diff = diff_words(
            &plain("Keep each page for one hour."),
            &plain("Keep each page for two hours."),
        );
        assert_eq!(
            shown(&diff.marks),
            "Keep each page for [-one hour-]{+two hours+}."
        );
        assert_eq!(texts(&diff.deleted), ["one", "hour"]);
        assert_eq!(texts(&diff.inserted), ["two", "hours"]);
        assert!((diff.kept_in_order - 4.0 / 6.0).abs() < 1e-9);
        assert!(keeps_new_text(&diff.marks, "Keep each page for two hours."));
    }

    #[test]
    fn kept_white_space_at_the_ends_stays_kept() {
        let diff = diff_words(&plain("a b"), &plain("c b"));
        assert_eq!(shown(&diff.marks), "[-a-]{+c+} b");
        let diff = diff_words(&plain("b a"), &plain("b c"));
        assert_eq!(shown(&diff.marks), "b [-a-]{+c+}");
        let diff = diff_words(&plain("x y z"), &plain("x q z"));
        assert_eq!(shown(&diff.marks), "x [-y-]{+q+} z");
        // Kept white space and a kept word between two changes.
        let diff = diff_words(&plain("a b c"), &plain("d b e"));
        assert_eq!(shown(&diff.marks), "[-a-]{+d+} b [-c-]{+e+}");
        // White space alone at either end, as a sentence row or a table row
        // with an empty first cell has it, stays kept too.
        let diff = diff_words(&plain("a "), &plain("b "));
        assert_eq!(shown(&diff.marks), "[-a-]{+b+} ");
        let diff = diff_words(&plain("\ta"), &plain("\tb"));
        assert_eq!(shown(&diff.marks), "\t[-a-]{+b+}");
    }

    #[test]
    fn white_space_between_two_changes_joins_them() {
        let diff = diff_words(&plain("the quick brown fox"), &plain("a slow red fox"));
        assert_eq!(shown(&diff.marks), "[-the quick brown-]{+a slow red+} fox");
        assert!((diff.kept_in_order - 0.25).abs() < 1e-9);
    }

    #[test]
    fn nothing_to_compare_counts_as_all_kept() {
        let diff = diff_words(&plain("—"), &plain("–"));
        assert_eq!(shown(&diff.marks), "[-—-]{+–+}");
        assert!((diff.kept_in_order - 1.0).abs() < 1e-9);
        let diff = diff_words(&plain("a"), &plain(""));
        assert_eq!(shown(&diff.marks), "[-a-]");
        assert!(diff.kept_in_order.abs() < 1e-9);
    }

    #[test]
    fn the_guard_refuses_marks_that_do_not_spell_the_new_text() {
        let mark = |op, text: &str| Mark {
            op,
            text: text.to_owned(),
        };
        let marks = [
            mark(MarkOp::Kept, "a "),
            mark(MarkOp::Deleted, "b"),
            mark(MarkOp::Inserted, "c"),
        ];
        assert!(keeps_new_text(&marks, "a c"));
        assert!(!keeps_new_text(&marks, "a c d"));
        assert!(!keeps_new_text(&marks, "a b"));
        assert!(!keeps_new_text(&marks, "a"));
        assert!(keeps_new_text(&[], ""));
    }

    fn pieces(text: &str) -> Vec<&str> {
        sentences(text)
            .into_iter()
            .map(|range| &text[range])
            .collect()
    }

    #[test]
    fn sentences_split_after_their_end() {
        let text = "One. Two! Three? “Four.” Five e.g. six. Seven…\n\nEight";
        let found: Vec<&str> = sentences(text).into_iter().map(|r| &text[r]).collect();
        assert_eq!(
            found,
            [
                "One. ",
                "Two! ",
                "Three? ",
                "“Four.” ",
                "Five e.g. six. ",
                "Seven…\n\n",
                "Eight"
            ]
        );
        let found: Vec<&str> = sentences("A\n\nb")
            .into_iter()
            .map(|r| &"A\n\nb"[r])
            .collect();
        assert_eq!(found, ["A\n\n", "b"]);
        assert_eq!(pieces("Ends here."), ["Ends here."]);
        assert_eq!(pieces("v1.2 is out"), ["v1.2 is out"]);
        assert_eq!(sentences("Done.)  Next"), [0..8, 8..12]);
        assert_eq!(sentences("Done. 2 more"), [0..6, 6..12]);
        assert_eq!(pieces("Wait.  "), ["Wait.  "]);
        let japanese = "これは文です。次の文！終わり？";
        let found: Vec<&str> = sentences(japanese)
            .into_iter()
            .map(|r| &japanese[r])
            .collect();
        assert_eq!(found, ["これは文です。", "次の文！", "終わり？"]);
        assert_eq!(sentences(""), Vec::<Range<usize>>::new());
        assert_eq!(pieces("a\nb"), ["a\nb"]);
        // A full-width stop right after a sentence ends is read as a stop.
        assert_eq!(pieces("A. 。B"), ["A. ", "。", "B"]);
    }

    #[test]
    fn spans_within_a_sentence_count_from_its_start() {
        let spans = [
            span(0..2, SpanKind::Code),
            span(4..6, link("a.md")),
            span(5..9, SpanKind::Code),
        ];
        assert_eq!(spans_within(&spans, &(4..8)), [span(0..2, link("a.md"))]);
        assert_eq!(spans_within(&spans, &(0..2)), [span(0..2, SpanKind::Code)]);
    }

    #[test]
    fn code_lines_are_marked_one_by_one() {
        let (marks, share) = diff_lines("a\nb\nc", "a\nB\nc\nd");
        let found: Vec<(MarkOp, &str)> = marks
            .iter()
            .map(|mark| (mark.op, mark.text.as_str()))
            .collect();
        assert_eq!(
            found,
            [
                (MarkOp::Kept, "a"),
                (MarkOp::Deleted, "b"),
                (MarkOp::Inserted, "B"),
                (MarkOp::Kept, "c"),
                (MarkOp::Inserted, "d"),
            ]
        );
        assert!((share - 0.5).abs() < 1e-9);
        let (marks, share) = diff_lines("x\ny", "z");
        assert_eq!(marks.len(), 3);
        assert!(share.abs() < 1e-9);
    }

    #[test]
    fn ratios_divide() {
        assert!((ratio(1, 4) - 0.25).abs() < 1e-12);
    }
}
