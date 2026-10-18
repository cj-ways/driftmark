//! A review's notes as text for an agent.
//!
//! The reader works in rounds. Round 1 hands over the notes still under
//! discussion and asks the agent to answer each one without editing yet.
//! Round 2 hands over the notes the reader decided and asks the agent to
//! apply them. [`prompt`] writes a round as text to paste into an agent
//! session; [`review_file`] writes the same as a Markdown file an agent can
//! read. Both name the tools of the MCP connector, and give an agent without
//! it a line under each note to answer on.

use std::fmt::Write as _;
use std::ops::Range;

use super::{AnchorState, Note, Placement, Role, Side, Status};

/// Which round of a review.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Round {
    /// Round 1: discuss the notes, change nothing yet.
    Discuss,
    /// Round 2: apply the decided notes.
    Apply,
}

impl Round {
    /// Whether a note belongs in this round: under discussion for round 1,
    /// decided for round 2.
    #[must_use]
    pub fn includes(self, note: &Note) -> bool {
        match self {
            Round::Discuss => matches!(
                note.status,
                Status::Open | Status::Sent | Status::Answered | Status::Discussing
            ),
            Round::Apply => note.status == Status::Decided,
        }
    }
}

/// The review a packet is about.
#[derive(Debug, Clone, Copy)]
pub struct Review<'a> {
    /// The review's number, shown as `r3`.
    pub id: u64,
    /// What it covers: a range such as `main...feature`, or a document's
    /// path.
    pub scope: &'a str,
    /// The version the notes were found in, such as a short commit id or
    /// `worktree`.
    pub at: &'a str,
}

/// One note of a packet, with where it sits in the version the agent will
/// edit.
#[derive(Debug, Clone, Copy)]
pub struct Item<'a> {
    /// The note.
    pub note: &'a Note,
    /// Where it sits now; `None` when it could not be looked for.
    pub placement: Option<&'a Placement>,
    /// The 1-based first and last line of that place.
    pub lines: Option<(usize, usize)>,
    /// The source text at that place, when it differs from the quote.
    pub current: Option<&'a str>,
}

/// The 1-based lines a byte range of `source` starts and ends on.
#[must_use]
pub fn lines(source: &str, range: &Range<usize>) -> (usize, usize) {
    let line = |at: usize| source[..at].matches('\n').count() + 1;
    let last = range.end.max(range.start + 1) - 1;
    (line(range.start), line(last.min(source.len())))
}

/// One round of a review as a prompt. Only the notes the round includes are
/// written, numbered from 1 in the order given.
#[must_use]
pub fn prompt(review: &Review<'_>, items: &[Item<'_>], round: Round) -> String {
    let chosen: Vec<&Item<'_>> = items
        .iter()
        .filter(|item| round.includes(item.note))
        .collect();
    let mut out = String::new();
    let noun = match (round, chosen.len()) {
        (Round::Discuss, 1) => "1 note".to_owned(),
        (Round::Discuss, n) => format!("{n} notes"),
        (Round::Apply, 1) => "1 decided note".to_owned(),
        (Round::Apply, n) => format!("{n} decided notes"),
    };
    let _ = writeln!(
        out,
        "Review r{} · {} · {noun} · at {}",
        review.id, review.scope, review.at
    );
    match round {
        Round::Discuss => out.push_str(
            "Round 1: discuss only. For each note, say whether you agree, disagree or have a question, and why.\n\
             Do not edit the documents yet. Answer each note with the note_reply tool, or write your answer on its \"Agent:\" line.\n",
        ),
        Round::Apply => out.push_str(
            "Round 2: apply the decided notes. Change each document as its decision says, and nothing else.\n\
             Then record each commit with the note_mark_applied tool, or write it on the note's \"Commit:\" line.\n",
        ),
    }
    for (number, item) in chosen.iter().enumerate() {
        out.push('\n');
        note(&mut out, number + 1, item, round);
    }
    out
}

/// One round of a review as a Markdown file, for an agent that reads files
/// instead of using the MCP connector. `written` says when, in words.
#[must_use]
pub fn review_file(review: &Review<'_>, items: &[Item<'_>], round: Round, written: &str) -> String {
    let round_name = match round {
        Round::Discuss => "round 1, discuss",
        Round::Apply => "round 2, apply",
    };
    format!(
        "# Review r{} of {}\n\n> Written by driftmark on {written}, {round_name}. The notes file is the source; this file is a copy.\n\n{}",
        review.id,
        review.scope,
        prompt(review, items, round)
    )
}

fn note(out: &mut String, number: usize, item: &Item<'_>, round: Round) {
    let note = item.note;
    let mut head = format!("## {number} · n{}", note.id);
    let decided = note.decision.as_ref().filter(|_| round == Round::Apply);
    if let Some(decision) = decided {
        let _ = write!(head, " · decided: {}", outcome_word(decision.outcome));
    } else {
        let _ = write!(head, " · {}", stance_word(note.stance));
        if note.blocking {
            head.push_str(" · blocking");
        }
        let _ = write!(head, " · ask: {}", ask_word(note.ask));
    }
    let _ = writeln!(out, "{head}");
    let target = &note.target;
    let mut place = format!("File: {}", target.path);
    if let Some((first, last)) = item.lines {
        if first == last {
            let _ = write!(place, ", line {first}");
        } else {
            let _ = write!(place, ", lines {first}–{last}");
        }
    }
    if !target.anchor.heading_path.is_empty() {
        let _ = write!(place, ", under {}", target.anchor.heading_path.join(" › "));
    }
    let _ = writeln!(out, "{place}");
    if target.side == Side::Old {
        let at = target.commit.as_deref().map_or("the working tree", short);
        let _ = writeln!(
            out,
            "On the old side of a change, at {at}: text the newer version removed or changed."
        );
    }
    quoted(out, &target.anchor.quote.exact);
    match item.placement.map(|placement| placement.state) {
        Some(AnchorState::Moved) => out.push_str("The quoted text has moved since the note.\n"),
        Some(AnchorState::Fuzzy | AnchorState::Outdated) => {
            out.push_str("The text changed after the note.");
            match item.current {
                Some(current) => {
                    out.push_str(" It now reads:\n");
                    quoted(out, current);
                }
                None => out.push('\n'),
            }
        }
        Some(AnchorState::Orphaned) => {
            out.push_str("The quoted text is no longer in the document.\n");
        }
        Some(AnchorState::Exact) | None => {}
    }
    labelled(out, "Reader", &note.body);
    for reply in &note.thread {
        let who = match reply.role {
            Role::Reader => "Reader",
            Role::Agent => "Agent",
        };
        labelled(out, who, &reply.body);
    }
    if let Some(decision) = decided {
        labelled(out, "Decision", &decision.summary);
        out.push_str("Commit:\n");
    } else {
        if let Some(proposal) = &note.proposal {
            labelled(out, "Proposed", &proposal.summary);
        }
        out.push_str("Agent:\n");
    }
}

/// Text as a Markdown quote, every line marked.
fn quoted(out: &mut String, text: &str) {
    for line in text.lines() {
        if line.is_empty() {
            out.push_str(">\n");
        } else {
            let _ = writeln!(out, "> {line}");
        }
    }
}

/// `Label: text`, with later lines indented so they stay under the label.
fn labelled(out: &mut String, label: &str, text: &str) {
    let mut lines = text.lines();
    let _ = writeln!(out, "{label}: {}", lines.next().unwrap_or_default());
    for line in lines {
        let _ = writeln!(out, "  {line}");
    }
}

fn short(commit: &str) -> &str {
    commit.get(..7).unwrap_or(commit)
}

fn stance_word(stance: super::Stance) -> &'static str {
    use super::Stance;
    match stance {
        Stance::Agree => "agree",
        Stance::Disagree => "disagree",
        Stance::Question => "question",
        Stance::Suggestion => "suggestion",
    }
}

fn ask_word(ask: super::Ask) -> &'static str {
    use super::Ask;
    match ask {
        Ask::Change => "change",
        Ask::Explain => "explain",
        Ask::Keep => "keep",
    }
}

fn outcome_word(outcome: super::Outcome) -> &'static str {
    use super::Outcome;
    match outcome {
        Outcome::Change => "change",
        Outcome::Modify => "modify",
        Outcome::Keep => "keep",
    }
}

#[cfg(test)]
mod tests;
