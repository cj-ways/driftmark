//! The words a page shows for the change list's levels, kinds, views and
//! signals.
//!
//! The change list names them for programs (`moved_edited`,
//! `decision_section`). A reader needs plain words: "Moved and edited", "in a
//! decision section". Both the change HTML and the program's own pages, such
//! as the change page's list of every change, use these, so one change reads
//! the same everywhere.

use crate::diff::{Change, ChangeKind, Level, Signal, Subject, View};

/// The level as a word with a capital letter: `Decision`, `Rule`, `Fact`,
/// `Wording` or `Formatting`.
#[must_use]
pub fn level(level: Level) -> &'static str {
    match level {
        Level::Decision => "Decision",
        Level::Rule => "Rule",
        Level::Fact => "Fact",
        Level::Wording => "Wording",
        Level::Formatting => "Formatting",
    }
}

/// What happened, as a reader says it: `Edited`, `Moved and edited`, or
/// `New document` for a whole document that came.
#[must_use]
pub fn kind(change: &Change) -> &'static str {
    let whole = matches!(change.subject, Subject::Document { .. });
    match (change.kind, whole) {
        (ChangeKind::Added, true) => "New document",
        (ChangeKind::Removed, true) => "Deleted document",
        (ChangeKind::Formatting, _) => "Formatting only",
        (ChangeKind::Edited, _) => "Edited",
        (ChangeKind::Added, false) => "Added",
        (ChangeKind::Removed, false) => "Removed",
        (ChangeKind::Moved, _) => "Moved",
        (ChangeKind::MovedEdited, _) => "Moved and edited",
    }
}

/// How an edit is drawn, as the page names the view.
#[must_use]
pub fn view(view: View) -> &'static str {
    match view {
        View::Inline => "Word marks",
        View::SideBySide => "Side by side",
        View::Sentences => "Sentence by sentence",
        View::Rewritten => "Rewritten",
        View::Lines => "Line by line",
    }
}

/// Why a signal ranks a change, as a phrase that completes "It ranks here
/// because …" without the opening words.
#[must_use]
pub fn signal(signal: Signal) -> &'static str {
    match signal {
        Signal::Status => "the status changed",
        Signal::DecisionSection => "in a decision section",
        Signal::Normative => "a rule word such as must or never changed",
        Signal::Heading => "a heading changed",
        Signal::Number => "a number or a name changed",
        Signal::Link => "a link changed",
        Signal::TableCell => "a table row changed",
        Signal::CodeLine => "code changed",
        Signal::TaskState => "a checkbox was ticked or cleared",
        Signal::Moved => "it moved",
        Signal::Wording => "words changed",
        Signal::Formatting => "only the Markdown changed",
    }
}

/// Every signal of a change as phrases joined by ` · `.
#[must_use]
pub fn signals(change: &Change) -> String {
    let phrases: Vec<&str> = change.signals.iter().copied().map(signal).collect();
    phrases.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{Side, WholeDocument};

    fn change(kind: ChangeKind, whole: bool, signals: Vec<Signal>) -> Change {
        let subject = if whole {
            Subject::Document {
                document: WholeDocument {
                    title: None,
                    summary: None,
                    bytes: 0,
                    lines: 0,
                    source: String::new(),
                },
            }
        } else {
            Subject::Block {
                old: Side { at: 0, block: None },
                new: Side { at: 0, block: None },
            }
        };
        Change {
            kind,
            level: Level::Fact,
            signals,
            subject,
            edit: None,
        }
    }

    #[test]
    fn every_level_has_a_word() {
        let words: Vec<&str> = [
            Level::Decision,
            Level::Rule,
            Level::Fact,
            Level::Wording,
            Level::Formatting,
        ]
        .into_iter()
        .map(level)
        .collect();
        assert_eq!(words, ["Decision", "Rule", "Fact", "Wording", "Formatting"]);
    }

    #[test]
    fn kinds_tell_a_whole_document_from_a_block() {
        let word = |kind, whole| super::kind(&change(kind, whole, Vec::new()));
        assert_eq!(word(ChangeKind::Added, true), "New document");
        assert_eq!(word(ChangeKind::Removed, true), "Deleted document");
        assert_eq!(word(ChangeKind::Added, false), "Added");
        assert_eq!(word(ChangeKind::Removed, false), "Removed");
        assert_eq!(word(ChangeKind::Formatting, false), "Formatting only");
        assert_eq!(word(ChangeKind::Edited, false), "Edited");
        assert_eq!(word(ChangeKind::Moved, false), "Moved");
        assert_eq!(word(ChangeKind::MovedEdited, false), "Moved and edited");
    }

    #[test]
    fn every_view_has_a_name() {
        let names: Vec<&str> = [
            View::Inline,
            View::SideBySide,
            View::Sentences,
            View::Rewritten,
            View::Lines,
        ]
        .into_iter()
        .map(view)
        .collect();
        assert_eq!(
            names,
            [
                "Word marks",
                "Side by side",
                "Sentence by sentence",
                "Rewritten",
                "Line by line"
            ]
        );
    }

    #[test]
    fn signals_join_their_phrases() {
        let all = change(
            ChangeKind::Edited,
            false,
            vec![
                Signal::Status,
                Signal::DecisionSection,
                Signal::Normative,
                Signal::Heading,
                Signal::Number,
                Signal::Link,
                Signal::TableCell,
                Signal::CodeLine,
                Signal::TaskState,
                Signal::Moved,
                Signal::Wording,
                Signal::Formatting,
            ],
        );
        assert_eq!(
            signals(&all),
            "the status changed · in a decision section · a rule word such as must or never changed · a heading changed · a number or a name changed · a link changed · a table row changed · code changed · a checkbox was ticked or cleared · it moved · words changed · only the Markdown changed"
        );
        assert_eq!(signals(&change(ChangeKind::Edited, false, Vec::new())), "");
    }
}
