//! Notes: a reader's point on a sentence, the agent's replies, the reader's
//! decision, and the commits that applied it (decision 0004).
//!
//! A note sits on an exact stretch of a document's Markdown source at the
//! commit where it was made. [`anchor`] finds that stretch again in a later
//! version. [`packet`] writes a review's notes as a prompt or a review file
//! for an agent. [`caused`] finds the changes of a range that a commit
//! recorded on a note made, for "changed because of note n12". [`history`]
//! holds each action done to a note with where it came from, and the
//! warning a reader's action off the page carries. This module holds the
//! note itself and the rules of its loop: who may move a note to which
//! status. The program keeps notes in its notes file; nothing here reads or
//! writes it.
//!
//! `docs/features/engine/notes-model.md` states the contract in full.

pub mod anchor;
pub mod caused;
mod fuzzy;
pub mod history;
pub mod packet;

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::diff::{Change, Subject};

pub use anchor::{Anchor, AnchorState, Origin, Placement, Quote, locate_text, reanchor};
pub use caused::{Around, Ends, caused};
pub use history::{Event, EventAction, Source, warning};

/// Where a note was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    /// On a reading page.
    Reading,
    /// On the old side of a change page: text the newer version deleted or
    /// changed. Such a note stays on that old version.
    Old,
    /// On the new side of a change page.
    New,
}

/// What the reader thinks of the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stance {
    /// The text is right.
    Agree,
    /// The text is wrong.
    Disagree,
    /// The reader asks something.
    Question,
    /// The reader offers another way.
    Suggestion,
}

/// What the reader wants done.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ask {
    /// Change the document.
    Change,
    /// Explain the text, in the thread.
    Explain,
    /// Keep the text as it is.
    Keep,
}

/// Where a note's conversation is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Written, not sent to an agent yet.
    Open,
    /// Sent to an agent, no answer yet.
    Sent,
    /// An agent answered.
    Answered,
    /// The reader answered back; the discussion goes on.
    Discussing,
    /// The reader decided what happens.
    Decided,
    /// An agent applied the decision and named the commits.
    Applied,
    /// The reader decided to keep the text: closed without a change.
    Closed,
    /// The reader took the note back.
    Withdrawn,
}

impl Status {
    /// Every status, in the order of the loop.
    pub const ALL: [Status; 8] = [
        Status::Open,
        Status::Sent,
        Status::Answered,
        Status::Discussing,
        Status::Decided,
        Status::Applied,
        Status::Closed,
        Status::Withdrawn,
    ];

    /// The status's name in JSON and in the notes file.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Status::Open => "open",
            Status::Sent => "sent",
            Status::Answered => "answered",
            Status::Discussing => "discussing",
            Status::Decided => "decided",
            Status::Applied => "applied",
            Status::Closed => "closed",
            Status::Withdrawn => "withdrawn",
        }
    }

    /// The status with this name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Status> {
        Status::ALL.into_iter().find(|status| status.name() == name)
    }

    /// Whether the loop is over: nothing can change the note any more.
    #[must_use]
    pub fn is_final(self) -> bool {
        matches!(self, Status::Applied | Status::Closed | Status::Withdrawn)
    }
}

/// Who acts on a note.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// The person who reads the documents and wrote the note.
    Reader,
    /// An AI agent, through the MCP connector or a review file.
    Agent,
}

/// What a reply says about the note.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplyStance {
    /// The agent agrees with the reader.
    Agree,
    /// The agent argues for the text as it is, or for something else.
    Counter,
    /// The agent asks back.
    Question,
    /// The agent proposes what to do.
    Proposal,
}

/// One message in a note's thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reply {
    /// The reader or an agent.
    pub role: Role,
    /// Who wrote it: the reader's name, or the agent's session.
    pub by: String,
    /// When, in seconds since 1970 UTC.
    pub at: i64,
    /// What it says about the note, when the writer said.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stance: Option<ReplyStance>,
    /// The text.
    pub body: String,
}

/// What was decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Change the document as the note asked.
    Change,
    /// Change it, differently from what the note asked; the summary says how.
    Modify,
    /// Keep the document as it is.
    Keep,
}

/// A decision, or an agent's proposal of one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    /// What happens.
    pub outcome: Outcome,
    /// What exactly, in a sentence or two.
    pub summary: String,
    /// Who decided, or who proposed.
    pub by: String,
    /// When, in seconds since 1970 UTC.
    pub at: i64,
}

/// A commit that applied a note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Applied {
    /// The commit id, as written.
    pub commit: String,
    /// Who recorded it.
    pub by: String,
    /// When, in seconds since 1970 UTC.
    pub at: i64,
}

/// What a note was made on: the document, the version and the stretch of
/// source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    /// The document's path from the repository root.
    pub path: String,
    /// The commit the page showed, or `None` for the working tree.
    pub commit: Option<String>,
    /// The id of the exact bytes the note's offsets count in: the blob id git
    /// gives those bytes. A version that has this id needs no re-anchoring.
    pub blob: String,
    /// The page and side the note was made on.
    pub side: Side,
    /// The stretch of source, with its quote and its place.
    #[serde(flatten)]
    pub anchor: Anchor,
}

/// A reader's note and its loop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Note {
    /// The note's number in its notes file, shown as `n12`.
    pub id: u64,
    /// The review it belongs to.
    pub review: u64,
    /// What it was made on.
    pub target: Target,
    /// What the reader thinks.
    pub stance: Stance,
    /// What the reader wants done.
    pub ask: Ask,
    /// Whether the reader holds the work until this note is settled.
    pub blocking: bool,
    /// The reader's point.
    pub body: String,
    /// Where the conversation is.
    pub status: Status,
    /// The replies, oldest first.
    pub thread: Vec<Reply>,
    /// What an agent proposed to decide, if it did.
    pub proposal: Option<Decision>,
    /// What the reader decided, once decided.
    pub decision: Option<Decision>,
    /// The commits that applied it.
    pub applied: Vec<Applied>,
    /// Who wrote it.
    pub author: String,
    /// When it was made, in seconds since 1970 UTC.
    pub created_at: i64,
    /// When it last changed, in seconds since 1970 UTC.
    pub updated_at: i64,
    /// Every action done to it, oldest first, each with where it came from
    /// (decision 0012).
    #[serde(default)]
    pub history: Vec<Event>,
}

/// Something a reader or an agent does to a note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Change the note's text, stance, ask or blocking flag.
    Edit,
    /// Hand the note to an agent, as a prompt or a review file.
    Send,
    /// Add a reply to the thread.
    Reply,
    /// Propose a decision.
    Propose,
    /// Decide.
    Decide(Outcome),
    /// Record the commits that applied the decision.
    MarkApplied,
    /// Take the note back.
    Withdraw,
}

/// Why an action was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The note's loop is over.
    Final(Status),
    /// Only the reader may do this.
    ReaderOnly,
    /// Only an agent may do this.
    AgentOnly,
    /// The note is decided, so it can no longer be edited, discussed into
    /// another decision by an agent, or proposed on.
    AlreadyDecided,
    /// Only a decided note can be applied.
    NotDecided(Status),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::Final(status) => {
                write!(f, "the note is {}, so it cannot change", status.name())
            }
            Refusal::ReaderOnly => f.write_str("only the reader can do this"),
            Refusal::AgentOnly => f.write_str("only an agent can do this; the reader decides"),
            Refusal::AlreadyDecided => f.write_str("the note is decided already"),
            Refusal::NotDecided(status) => write!(
                f,
                "only a decided note can be applied; this one is {}",
                status.name()
            ),
        }
    }
}

impl std::error::Error for Refusal {}

/// The status a note has after `who` does `action`, or why it may not.
///
/// The rules:
///
/// - Once applied, closed or withdrawn, a note never changes.
/// - Only the reader edits, sends, decides and withdraws. Deciding to keep
///   the text closes the note; any other outcome makes it decided.
/// - Only an agent proposes a decision, and only before the reader decided.
/// - A reply from an agent makes the note answered; a reply from the reader
///   to an answer makes it discussing. After the decision, replies keep it
///   decided.
/// - Either may record the commits that applied a decided note.
///
/// # Errors
///
/// The [`Refusal`] that says why the action is not allowed.
pub fn next_status(current: Status, who: Role, action: Action) -> Result<Status, Refusal> {
    if current.is_final() {
        return Err(Refusal::Final(current));
    }
    let decided = current == Status::Decided;
    match (action, who) {
        (Action::Edit | Action::Send | Action::Decide(_) | Action::Withdraw, Role::Agent) => {
            Err(Refusal::ReaderOnly)
        }
        (Action::Propose, Role::Reader) => Err(Refusal::AgentOnly),
        (Action::Edit | Action::Propose, _) if decided => Err(Refusal::AlreadyDecided),
        (Action::Edit, Role::Reader) => Ok(current),
        (Action::Send, Role::Reader) => Ok(if current == Status::Open {
            Status::Sent
        } else {
            current
        }),
        (Action::Reply, Role::Agent) => Ok(if decided { current } else { Status::Answered }),
        (Action::Reply, Role::Reader) => Ok(if current == Status::Answered {
            Status::Discussing
        } else {
            current
        }),
        (Action::Propose, Role::Agent) => Ok(Status::Answered),
        (Action::Decide(Outcome::Keep), Role::Reader) => Ok(Status::Closed),
        (Action::Decide(_), Role::Reader) => Ok(Status::Decided),
        (Action::MarkApplied, _) if decided => Ok(Status::Applied),
        (Action::MarkApplied, _) => Err(Refusal::NotDecided(current)),
        (Action::Withdraw, Role::Reader) => Ok(Status::Withdrawn),
    }
}

/// A key for one change of a change page that stays the same while the
/// change does: the document's path, the kind of change and the text on
/// each side. A reader's "seen" mark is kept under it, so a change seen once
/// stays seen after an agent adds unrelated commits, and a change whose text
/// moves on shows as new again.
///
/// It is 16 hex digits of an XXH3 hash.
#[must_use]
pub fn change_key(path: &str, change: &Change) -> String {
    use twox_hash::XxHash3_64;
    let mut parts: Vec<&str> = vec![path, kind_word(change)];
    match &change.subject {
        Subject::Block { old, new } => {
            for side in [old, new] {
                match &side.block {
                    Some(block) => parts.extend(["+", block.text.as_str()]),
                    None => parts.push("-"),
                }
            }
        }
        Subject::Document { document } => parts.push(&document.source),
    }
    let joined = parts.join("\u{0}");
    format!("{:016x}", XxHash3_64::oneshot(joined.as_bytes()))
}

fn kind_word(change: &Change) -> &'static str {
    use crate::diff::ChangeKind;
    match change.kind {
        ChangeKind::Formatting => "formatting",
        ChangeKind::Edited => "edited",
        ChangeKind::Added => "added",
        ChangeKind::Removed => "removed",
        ChangeKind::Moved => "moved",
        ChangeKind::MovedEdited => "moved_edited",
    }
}

#[cfg(test)]
mod tests;
