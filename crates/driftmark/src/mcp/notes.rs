//! The note tools: the reader's notes, read and answered by an agent.
//!
//! They use the notes file `driftmark serve` writes (decision 0011, item 2),
//! so a note the reader makes on a page is at once in `notes_list`, and an
//! agent's reply shows on the page when it reloads. Three tools only read:
//! `notes_list`, `note_get` and `review_get`. Three write, as the agent:
//! `note_reply`, `note_propose_decision` and `note_mark_applied`. The loop's
//! rules are the core's (`driftmark_core::notes::next_status`); an agent
//! never decides, edits or withdraws a note, and each refusal says why in
//! words an agent can act on.

use std::collections::HashMap;
use std::sync::Arc;

use driftmark_core::document::Document;
use driftmark_core::document::outline::Outline;
use driftmark_core::git::{CommitInfo, GitRead, ObjectId, RepoPath, Snapshot};
use driftmark_core::notes::packet::{self, Round};
use driftmark_core::notes::{
    Action, Note, Outcome, Refusal, ReplyStance, Role, Side, Stance, Status, next_status,
};
use serde_json::{Value, json};

use super::fit;
use super::tools::{
    Args, Context, MAX_LIMIT, MAX_OFFSET, Tool, document_path, limit_schema, list_cut,
    offset_schema, page,
};
use crate::notes::{Act, Review, Scope, Store, StoreError};
use crate::serve::address::WORKTREE;
use crate::serve::placing::{
    Failure, MAX_TEXT, Packet, PlacedJson, Version, edited_side, lost, now, old_version, placed,
    version_on,
};

/// How many notes `notes_list` lists when the call names no limit. Each
/// short entry is about 200 tokens.
const LIST_LIMIT: usize = 50;

/// The most characters of a quote a short entry shows.
const QUOTE_CHARS: usize = 160;

/// The most characters of the reader's point a short entry shows.
const BODY_CHARS: usize = 240;

/// The statuses `notes_list` shows when the call names none: the notes
/// still in the loop.
const LIVE: [Status; 5] = [
    Status::Open,
    Status::Sent,
    Status::Answered,
    Status::Discussing,
    Status::Decided,
];

/// `notes_list`.
pub(crate) const LIST: Tool = Tool {
    name: "notes_list",
    title: "The reader's notes",
    description: "Lists the reader's notes on this repository's documents, oldest first. Each entry has the note's number (n12), its review (r3), the document and the headings above the quote, its status, the reader's stance, what the reader asks (change, explain or keep), whether it blocks the work, the quoted words and the reader's point, both shortened, and how far the discussion is. By default only notes still in the loop are listed: open, sent, answered, discussing and decided; give `status` \"all\" for every note. Filter by `path`, `review`, `status`, `stance` and `blocking`. Read one note in full with note_get, or a review's notes as one prompt with review_get.",
    arguments: list_arguments,
    required: &[],
    read_only: true,
    run: list,
};

/// `note_get`.
pub(crate) const GET: Tool = Tool {
    name: "note_get",
    title: "Read one note",
    description: "Reads one note in full: the reader's point, the quoted words with the text around them, the version the note was made on and its lines there, where the words are now in the version an agent edits (the range's newer side, or the files on disk for a document's notes) with the state the search found (exact, moved, fuzzy, outdated or orphaned) and the text there now, the section around them from the document's outline, the thread of replies, any proposal and decision, and the commits that applied it.",
    arguments: get_arguments,
    required: &["note"],
    read_only: true,
    run: get,
};

/// `review_get`.
pub(crate) const REVIEW: Tool = Tool {
    name: "review_get",
    title: "A review as one prompt",
    description: "Gives one round of a review exactly as the reader's \"Copy as prompt\" button gives it. Round 1 holds the notes under discussion: answer each with note_reply and edit nothing yet. Round 2 holds the notes the reader decided, each with the decision: apply them, commit, and record the commits with note_mark_applied. Each note's lines count in the version an agent edits. This changes nothing: the notes stay as they are until you answer them.",
    arguments: review_arguments,
    required: &["review"],
    read_only: true,
    run: review,
};

/// `note_reply`.
pub(crate) const REPLY: Tool = Tool {
    name: "note_reply",
    title: "Answer a note",
    description: "Adds your reply to a note's thread, with your stance: agree (the reader is right), counter (you argue for the text as it is, or for something else), question (you ask back) or proposal (you say what to do; note_propose_decision records it as a proposal the reader can accept in one step). The note becomes answered and the reader sees the reply on the page. A decided note stays decided. A reply edits no document.",
    arguments: reply_arguments,
    required: &["note", "body", "stance"],
    read_only: false,
    run: reply,
};

/// `note_propose_decision`.
pub(crate) const PROPOSE: Tool = Tool {
    name: "note_propose_decision",
    title: "Propose a decision",
    description: "Proposes what to do about a note: change the text as the note asks, change it another way (modify), or keep it, with a summary that says exactly what. Only the reader decides, on the note's page, where the proposal fills in the decision form. The note becomes answered until then. A note the reader decided already takes no new proposal.",
    arguments: propose_arguments,
    required: &["note", "outcome", "summary"],
    read_only: false,
    run: propose,
};

/// `note_mark_applied`.
pub(crate) const MARK_APPLIED: Tool = Tool {
    name: "note_mark_applied",
    title: "Record the commits that applied a note",
    description: "Records the commits that applied a decided note, after you edited the document as its decision says and committed. Each commit must exist and must change the note's document compared with its first parent; anything else is refused, with the reason. The note becomes applied, and its range's change page marks the changes those commits made as \"changed because of\" the note.",
    arguments: applied_arguments,
    required: &["note", "commits"],
    read_only: false,
    run: mark_applied,
};

fn note_schema() -> Value {
    json!({
        "type": ["integer", "string"],
        "description": "The note's number, such as 12 or \"n12\", as notes_list and the prompt write it.",
    })
}

fn list_arguments() -> Value {
    json!({
        "path": {
            "type": "string",
            "description": "Only the notes on this document, from the repository root, such as docs/guide.md.",
        },
        "review": {
            "type": ["integer", "string"],
            "description": "Only the notes of this review, such as 3 or \"r3\".",
        },
        "status": {
            "type": ["array", "string"],
            "items": {"type": "string"},
            "description": "Only notes with these statuses: open, sent, answered, discussing, decided, applied, closed, withdrawn; or \"all\". Default: open, sent, answered, discussing and decided.",
        },
        "stance": {
            "type": ["array", "string"],
            "items": {"type": "string"},
            "description": "Only notes whose reader's stance is one of these: agree, disagree, question, suggestion.",
        },
        "blocking": {
            "type": "boolean",
            "description": "Only notes that block the work (true), or only those that do not (false).",
        },
        "offset": offset_schema("notes"),
        "limit": limit_schema("notes", LIST_LIMIT, MAX_LIMIT),
    })
}

fn get_arguments() -> Value {
    json!({ "note": note_schema() })
}

fn review_arguments() -> Value {
    json!({
        "review": {
            "type": ["integer", "string"],
            "description": "The review's number, such as 3 or \"r3\", as notes_list gives it.",
        },
        "round": {
            "type": "integer",
            "minimum": 1,
            "maximum": 2,
            "description": "1 (default): the notes under discussion, to answer. 2: the decided notes, to apply.",
        },
        "offset": {
            "type": "integer",
            "minimum": 0,
            "description": "The byte of the prompt to read on from, as a cut answer's `cut.next` gives it.",
        },
    })
}

fn reply_arguments() -> Value {
    json!({
        "note": note_schema(),
        "body": {
            "type": "string",
            "description": "Your reply, in plain words: what you think and why. At most 20,000 bytes.",
        },
        "stance": {
            "type": "string",
            "enum": ["agree", "counter", "question", "proposal"],
            "description": "What the reply says about the note: agree, counter, question or proposal.",
        },
    })
}

fn propose_arguments() -> Value {
    json!({
        "note": note_schema(),
        "outcome": {
            "type": "string",
            "enum": ["change", "modify", "keep"],
            "description": "change: change the text as the note asks. modify: change it another way, which the summary says. keep: keep the text as it is.",
        },
        "summary": {
            "type": "string",
            "description": "Exactly what to do, in a sentence or two, so it can be applied as written. At most 20,000 bytes.",
        },
    })
}

fn applied_arguments() -> Value {
    json!({
        "note": note_schema(),
        "commits": {
            "type": ["array", "string"],
            "items": {"type": "string"},
            "description": "The commit ids that applied the note, 7 to 64 hex digits each, as git prints them.",
        },
    })
}

// ---- reading the arguments ----------------------------------------------------

/// The notes file when it exists. A tool that reads never makes it.
fn existing(context: &Context) -> Result<Option<Arc<Store>>, String> {
    context.notes.existing().map_err(|error| error.to_string())
}

/// The note a call names.
fn note_id(args: &Args) -> Result<u64, String> {
    args.number_of("note", 'n')?
        .ok_or_else(|| "name the `note`, such as n12".to_owned())
}

/// Every value of a list argument, each read by `read`, or `None` when the
/// argument is not given.
fn values<T>(
    args: &Args,
    name: &str,
    read: impl Fn(&str) -> Option<T>,
    known: &str,
) -> Result<Option<Vec<T>>, String> {
    let texts = args.texts(name)?;
    if texts.is_empty() {
        return Ok(None);
    }
    texts
        .iter()
        .map(|text| {
            read(text.trim())
                .ok_or_else(|| format!("`{name}` has no value `{text}`; the values are {known}"))
        })
        .collect::<Result<Vec<T>, String>>()
        .map(Some)
}

fn stance_of(text: &str) -> Option<Stance> {
    serde_json::from_value(json!(text)).ok()
}

/// A text argument the call must give, with something in it, of at most
/// [`MAX_TEXT`] bytes.
fn words(args: &Args, name: &str) -> Result<String, String> {
    let text = args.text(name)?.unwrap_or_default();
    if text.trim().is_empty() {
        return Err(format!("`{name}` is empty; write it in plain words"));
    }
    if text.len() > MAX_TEXT {
        return Err(format!(
            "`{name}` is longer than {MAX_TEXT} bytes; say it in fewer words"
        ));
    }
    Ok(text)
}

/// `text`, shortened to `most` characters with `…` when it is longer.
fn shortened(text: &str, most: usize) -> String {
    if text.chars().count() <= most {
        return text.to_owned();
    }
    let mut short: String = text.chars().take(most - 1).collect();
    short.push('…');
    short
}

/// A review as answers write it.
fn review_json(review: &Review) -> Value {
    json!({
        "id": review.id,
        "kind": match review.scope { Scope::Range(_) => "range", Scope::Document(_) => "document" },
        "scope": review.scope.text(),
    })
}

/// A note in a few fields: enough to choose which to read in full.
fn short(note: &Note) -> Value {
    json!({
        "id": note.id,
        "review": note.review,
        "path": note.target.path,
        "under": note.target.anchor.heading_path,
        "side": note.target.side,
        "status": note.status.name(),
        "stance": note.stance,
        "ask": note.ask,
        "blocking": note.blocking,
        "quote": shortened(&note.target.anchor.quote.exact, QUOTE_CHARS),
        "body": shortened(&note.body, BODY_CHARS),
        "replies": note.thread.len(),
        "proposal": note.proposal.as_ref().map(|proposal| proposal.outcome),
        "decision": note.decision.as_ref().map(|decision| decision.outcome),
        "applied": note.applied.len(),
        "author": note.author,
        "updated_at": note.updated_at,
    })
}

// ---- notes_list ----------------------------------------------------------------

fn list(context: &Context, args: &Args) -> Result<Value, String> {
    let limit = args.max_chars()?;
    let path = args
        .text("path")?
        .map(|text| document_path(context, &text))
        .transpose()?
        .map(|path| path.to_lossy().into_owned());
    let review = args.number_of("review", 'r')?;
    let all = args.texts("status")?.iter().any(|text| text == "all");
    let statuses = if all {
        Status::ALL.to_vec()
    } else {
        values(
            args,
            "status",
            Status::from_name,
            "open, sent, answered, discussing, decided, applied, closed and withdrawn, or \"all\"",
        )?
        .unwrap_or_else(|| LIVE.to_vec())
    };
    let stances = values(
        args,
        "stance",
        stance_of,
        "agree, disagree, question and suggestion",
    )?;
    let blocking = args.given_flag("blocking")?;
    let offset = args.number("offset", 0, MAX_OFFSET, 0)?;
    let most = args.number("limit", 1, MAX_LIMIT, LIST_LIMIT)?;

    let store = existing(context)?;
    let notes: Vec<Note> = match (&store, review) {
        (None, Some(id)) => return Err(StoreError::NoReview(id).to_string()),
        (None, None) => Vec::new(),
        (Some(store), Some(id)) => {
            store.review(id).map_err(|error| error.to_string())?;
            store.notes_in(id).map_err(|error| error.to_string())?
        }
        (Some(store), None) => match &path {
            Some(path) => store.notes_on(path),
            None => store.notes(),
        }
        .map_err(|error| error.to_string())?,
    };
    let chosen: Vec<&Note> = notes
        .iter()
        .filter(|note| path.as_ref().is_none_or(|path| &note.target.path == path))
        .filter(|note| statuses.contains(&note.status))
        .filter(|note| {
            stances
                .as_ref()
                .is_none_or(|stances| stances.contains(&note.stance))
        })
        .filter(|note| blocking.is_none_or(|blocking| note.blocking == blocking))
        .collect();
    let total = chosen.len();
    let start = offset.min(total);
    let shown: Vec<&Note> = chosen[start..total.min(start.saturating_add(most))].to_vec();
    let entries: Vec<Value> = shown.iter().map(|note| short(note)).collect();
    let mut reviews: HashMap<u64, Value> = HashMap::new();
    if let Some(store) = &store {
        for note in &shown {
            if let std::collections::hash_map::Entry::Vacant(slot) = reviews.entry(note.review) {
                let found = store
                    .review(note.review)
                    .map_err(|error| error.to_string())?;
                slot.insert(review_json(&found));
            }
        }
    }
    let names: Vec<&str> = statuses.iter().map(|status| status.name()).collect();
    let build = |count: usize| {
        let mut ids: Vec<u64> = shown[..count].iter().map(|note| note.review).collect();
        ids.sort_unstable();
        ids.dedup();
        let listed: Vec<&Value> = ids.iter().filter_map(|id| reviews.get(id)).collect();
        let mut answer = json!({
            "schema_version": 1,
            "statuses": names,
            "total": total,
            "notes": &entries[..count],
            "reviews": listed,
            "page": page(offset, count, total),
        });
        if let Some(cut) = list_cut(args, "notes", offset, count, total) {
            answer["cut"] = cut;
        }
        answer
    };
    let sizes: Vec<usize> = entries.iter().map(fit::size).collect();
    Ok(fit::most_that_fit(limit, &sizes, build).0)
}

// ---- note_get --------------------------------------------------------------------

/// Where a note sits now, and the version it sits in, when it was found.
struct Now {
    /// The answer's `now`.
    json: Value,
    /// The version and the block the note starts in there.
    found: Option<(Arc<Document>, String, usize)>,
}

/// Where `note` sits in the version an agent edits for its review. A note on
/// the old side of a change stays on the version it was made on.
fn now_of(context: &Context, store: &Store, review: &Review, note: &Note) -> Result<Now, String> {
    let repo = &context.repo;
    let failed = |failure: Failure| failure.message;
    let (version, label) = if note.target.side == Side::Old {
        let version = old_version(repo, store, &note.target.blob).map(|document| Version {
            commit: note.target.commit.as_deref().and_then(ObjectId::from_hex),
            blob: note.target.blob.clone(),
            document,
        });
        let label = note
            .target
            .commit
            .clone()
            .unwrap_or_else(|| WORKTREE.to_owned());
        (version, label)
    } else {
        let edited = edited_side(repo, review).map_err(failed)?;
        let label = match &edited {
            Snapshot::Commit(commit) => commit.as_hex().to_owned(),
            Snapshot::Worktree => WORKTREE.to_owned(),
        };
        let version = match RepoPath::new(note.target.path.as_bytes()) {
            Some(path) => version_on(repo, &edited, &path).map_err(failed)?,
            None => None,
        };
        (version, label)
    };
    let Some(version) = version else {
        return Ok(Now {
            json: json!({
                "revision": label,
                "state": "orphaned",
                "why": format!("`{}` is not in that version", note.target.path),
            }),
            found: None,
        });
    };
    let side = if note.target.side == Side::Old {
        Side::Old
    } else {
        Side::New
    };
    let (placement, shown) = placed(
        repo,
        store,
        &context.notes.placements,
        std::slice::from_ref(note),
        &version,
        side,
    )
    .pop()
    .unwrap_or_else(|| {
        let placement = lost();
        let shown = PlacedJson::new(side, &placement, &version.document, "");
        (placement, shown)
    });
    let mut json = json!({
        "revision": label,
        "state": shown.state,
        "start": shown.start,
        "end": shown.end,
        "lines": shown.lines,
        "score": shown.score,
    });
    if let Some(text) = &shown.text {
        json["text"] = json!(text);
    }
    let found = placement
        .block
        .map(|block| (Arc::clone(&version.document), label, block));
    Ok(Now { json, found })
}

/// The section around the place a note was found, or the place it was made,
/// with its text: the heading path, the anchor, the lines and the text.
fn section_of(document: &Document, block: usize) -> Option<(Value, String)> {
    let outline = Outline::of(document);
    let section = outline.containing(block)?;
    let text = document.source[section.bytes.clone()].to_owned();
    let (first, last) = packet::lines(&document.source, &section.bytes);
    Some((
        json!({
            "path": section.path,
            "anchor": section.anchor,
            "lines": [first, last],
        }),
        text,
    ))
}

fn get(context: &Context, args: &Args) -> Result<Value, String> {
    let limit = args.max_chars()?;
    let id = note_id(args)?;
    let store = existing(context)?.ok_or_else(|| StoreError::NoNote(id).to_string())?;
    let note = store.note(id).map_err(|error| error.to_string())?;
    let review = store
        .review(note.review)
        .map_err(|error| error.to_string())?;
    let made = old_version(&context.repo, &store, &note.target.blob);
    let made_lines = made.as_ref().map(|document| {
        let (first, last) = packet::lines(
            &document.source,
            &(note.target.anchor.start..note.target.anchor.end),
        );
        [first, last]
    });
    let made_on = json!({
        "revision": note.target.commit.clone().unwrap_or_else(|| WORKTREE.to_owned()),
        "lines": made_lines,
    });
    let now = now_of(context, &store, &review, &note)?;
    // The section around the words now; when they are gone, around where
    // the note was made.
    let around = match &now.found {
        Some((document, label, block)) => {
            section_of(document, *block).map(|found| (found, label.clone()))
        }
        None => made.as_ref().and_then(|document| {
            section_of(document, note.target.anchor.block).map(|found| {
                (
                    found,
                    note.target
                        .commit
                        .clone()
                        .unwrap_or_else(|| WORKTREE.to_owned()),
                )
            })
        }),
    };
    let thread: Vec<Value> = note.thread.iter().map(|reply| json!(reply)).collect();
    let build = |from: usize, section_text: Option<&str>, cut: Option<&str>| {
        let mut shown = json!(note);
        shown["thread"] = json!(&thread[from..]);
        let mut answer = json!({
            "schema_version": 1,
            "note": shown,
            "review": review_json(&review),
            "made_on": made_on,
            "now": now.json,
        });
        if let Some(((section, _), label)) = &around {
            let mut section = section.clone();
            section["revision"] = json!(label);
            section["text"] = json!(section_text.unwrap_or_default());
            answer["section"] = section;
        }
        if let Some(reason) = cut {
            answer["cut"] = fit::cut(reason, None);
        }
        answer
    };
    // The thread's oldest replies go first when the answer does not fit;
    // then the section's text is cut after a whole line.
    // Room for the `cut` sentences, which are written last.
    let worst = "x".repeat(800);
    let mut from = 0;
    while from < thread.len() && fit::size(&build(from, None, Some(&worst))) > limit {
        from += 1;
    }
    let mut reasons = Vec::new();
    if from > 0 {
        reasons.push(format!(
            "the {from} oldest of the {} replies are left out; ask again with a larger `max_chars`, up to {}",
            thread.len(),
            fit::MAX_CHARS
        ));
    }
    let section_text = around.as_ref().map(|((section, text), label)| {
        let room = limit.saturating_sub(fit::size(&build(from, None, Some(&worst))));
        let end = fit::text_that_fits(text, room);
        if end < text.len() {
            reasons.push(format!(
                "the section's text was cut after a whole line, at byte {end} of {}; read it whole with document_read and {}",
                text.len(),
                json!({"path": note.target.path, "revision": label, "section": section["anchor"]})
            ));
        }
        &text[..end]
    });
    let reason = (!reasons.is_empty()).then(|| reasons.join("; "));
    Ok(build(from, section_text, reason.as_deref()))
}

// ---- review_get -----------------------------------------------------------------

fn review(context: &Context, args: &Args) -> Result<Value, String> {
    let limit = args.max_chars()?;
    let id = args
        .number_of("review", 'r')?
        .ok_or_else(|| "name the `review`, such as r3".to_owned())?;
    let round = match args.number("round", 1, 2, 1)? {
        1 => Round::Discuss,
        _ => Round::Apply,
    };
    let offset = args.given_number("offset", MAX_OFFSET)?;
    let store = existing(context)?.ok_or_else(|| StoreError::NoReview(id).to_string())?;
    let packet = Packet::of(&context.repo, &store, &context.notes.placements, id, round)
        .map_err(|failure| failure.message)?;
    let text = packet.prompt();
    let start = match offset {
        None => 0,
        Some(offset) if offset <= text.len() => (0..=offset)
            .rev()
            .find(|&at| text.is_char_boundary(at))
            .unwrap_or_default(),
        Some(offset) => {
            return Err(format!(
                "`offset` {offset} is past the prompt's end, at byte {}",
                text.len()
            ));
        }
    };
    let head = json!({
        "schema_version": 1,
        "review": review_json(&packet.review),
        "round": if round == Round::Discuss { 1 } else { 2 },
        "notes": packet.notes.iter().map(|note| note.id).collect::<Vec<u64>>(),
        "bytes": text.len(),
    });
    let build = |end: usize, cut: Option<Value>| {
        let mut answer = head.clone();
        answer["text_range"] = json!({"start": start, "end": end});
        answer["text"] = json!(&text[start..end]);
        if let Some(cut) = cut {
            answer["cut"] = cut;
        }
        answer
    };
    let whole = build(text.len(), None);
    if fit::size(&whole) <= limit {
        return Ok(whole);
    }
    let worst = fit::cut(
        &format!(
            "the prompt was cut after a whole line, at byte {} of {}; ask again with `offset` {} for the rest",
            text.len(),
            text.len(),
            text.len()
        ),
        Some(args.next(&[("offset", json!(text.len()))])),
    );
    let fixed = fit::size(&build(start, Some(worst)));
    let end = start + fit::text_that_fits(&text[start..], limit.saturating_sub(fixed + 16));
    let cut = fit::cut(
        &format!(
            "the prompt was cut after a whole line, at byte {end} of {}; ask again with `offset` {end} for the rest",
            text.len()
        ),
        Some(args.next(&[("offset", json!(end))])),
    );
    Ok(build(end, Some(cut)))
}

// ---- writing: replies, proposals, commits ---------------------------------------

/// What only the reader does, in a word, for a refusal.
fn verb(act: &Act) -> &'static str {
    match act {
        Act::Edit(_) => "edit",
        Act::Send => "send",
        Act::Decide { .. } => "decide",
        Act::Withdraw => "withdraw",
        Act::Reply { .. } | Act::Propose { .. } | Act::Applied { .. } => "do this to",
    }
}

/// Why the loop refused an agent's `act` on note `id`, in words an agent can
/// act on: what the note's state is, and what it can do instead.
pub(crate) fn refused(id: u64, act: &Act, refusal: Refusal) -> String {
    match refusal {
        Refusal::Final(Status::Withdrawn) => format!(
            "n{id} is withdrawn: the reader took it back, so it takes no reply, proposal or commit"
        ),
        Refusal::Final(Status::Closed) => format!(
            "n{id} is closed: the reader decided to keep the text as it is, so nothing more happens to it"
        ),
        Refusal::Final(_) => format!(
            "n{id} is applied: its commits are recorded and its loop is over, so it cannot change"
        ),
        Refusal::ReaderOnly => format!(
            "only the reader can {} a note, on its page, because the notes are the reader's own words and judgement; an agent answers with note_reply, suggests a decision with note_propose_decision, and records the commits that applied one with note_mark_applied",
            verb(act)
        ),
        Refusal::AlreadyDecided => format!(
            "n{id} is decided already, so it takes no new proposal; apply the decision (review_get round 2), commit, then record the commits with note_mark_applied"
        ),
        Refusal::NotDecided(status) => format!(
            "n{id} is {}, not decided, so no commit can be recorded on it yet; only the reader decides, on the note's page. Suggest a decision with note_propose_decision and wait for the reader",
            status.name()
        ),
        Refusal::AgentOnly => refusal.to_string(),
    }
}

/// Does `act` to note `id` as the agent that called.
pub(crate) fn act_as_agent(
    context: &Context,
    args: &Args,
    id: u64,
    act: &Act,
) -> Result<Note, String> {
    let store = existing(context)?.ok_or_else(|| StoreError::NoNote(id).to_string())?;
    store
        .act(id, Role::Agent, args.client(), act, now())
        .map_err(|error| match error {
            StoreError::Refused(refusal) => refused(id, act, refusal),
            other => other.to_string(),
        })
}

fn reply(context: &Context, args: &Args) -> Result<Value, String> {
    let id = note_id(args)?;
    let body = words(args, "body")?;
    let stance: ReplyStance = args
        .text("stance")?
        .and_then(|text| serde_json::from_value(json!(text.trim())).ok())
        .ok_or_else(|| "`stance` must be agree, counter, question or proposal".to_owned())?;
    let note = act_as_agent(
        context,
        args,
        id,
        &Act::Reply {
            body,
            stance: Some(stance),
        },
    )?;
    let message = if note.status == Status::Decided {
        format!("Your reply is on n{id}. It stays decided; the reader sees the reply on its page.")
    } else {
        format!(
            "Your reply is on n{id}, which is now answered. The reader sees it on the note's page and replies or decides there."
        )
    };
    Ok(json!({
        "schema_version": 1,
        "note": short(&note),
        "reply": note.thread.last(),
        "message": message,
    }))
}

fn propose(context: &Context, args: &Args) -> Result<Value, String> {
    let id = note_id(args)?;
    let outcome: Outcome = args
        .text("outcome")?
        .and_then(|text| serde_json::from_value(json!(text.trim())).ok())
        .ok_or_else(|| {
            "`outcome` must be change, modify or keep; a proposal never decides, the reader does"
                .to_owned()
        })?;
    let summary = words(args, "summary")?;
    let note = act_as_agent(context, args, id, &Act::Propose { outcome, summary })?;
    Ok(json!({
        "schema_version": 1,
        "note": short(&note),
        "proposal": note.proposal,
        "message": format!(
            "Proposed for n{id}. Only the reader decides, on the note's page, where your proposal fills in the form; until then the note is answered. Apply it only once its status is decided."
        ),
    }))
}

/// The commit `written` names, in full, when it changes `path` compared
/// with its first parent.
fn commit_that_changed(context: &Context, written: &str, path: &str) -> Result<ObjectId, String> {
    let text = written.trim();
    if !(7..=64).contains(&text.len()) || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!(
            "`{written}` is not a commit id; give 7 to 64 hex digits, as `git log` prints them"
        ));
    }
    let git = &context.repo.git;
    let commit = git
        .resolve(&text.to_ascii_lowercase())
        .map_err(|_| format!("`{written}` does not name a commit in this repository"))?;
    let repo_path = RepoPath::new(path.as_bytes())
        .ok_or_else(|| format!("the note's document `{path}` is not a path git can hold"))?;
    let unreadable =
        |error: driftmark_core::git::GitError| format!("git could not be read: {error}");
    let info = CommitInfo::parse(&git.read_commit(&commit).map_err(unreadable)?)
        .ok_or_else(|| format!("`{written}` is not a commit git can read"))?;
    let after = git.blob_at(&commit, &repo_path).map_err(unreadable)?;
    let before = match info.parents.first() {
        Some(parent) => git.blob_at(parent, &repo_path).map_err(unreadable)?,
        None => None,
    };
    if after == before {
        let compared = info.parents.first().map_or_else(
            || "it is the first commit and has no such file".to_owned(),
            |parent| {
                format!(
                    "the file is the same as in its parent {}",
                    &parent.as_hex()[..7]
                )
            },
        );
        return Err(format!(
            "`{written}` did not change `{path}`, the document of the note: {compared}. Record only the commits that edited the document as the decision says"
        ));
    }
    Ok(commit)
}

fn mark_applied(context: &Context, args: &Args) -> Result<Value, String> {
    let id = note_id(args)?;
    let written = args.texts("commits")?;
    if written.is_empty() {
        return Err("name at least one commit in `commits`".to_owned());
    }
    let store = existing(context)?.ok_or_else(|| StoreError::NoNote(id).to_string())?;
    let note = store.note(id).map_err(|error| error.to_string())?;
    let act = Act::Applied {
        commits: Vec::new(),
    };
    // The loop answers first: a note that is not decided needs the reader
    // before any commit matters.
    if let Err(refusal) = next_status(note.status, Role::Agent, Action::MarkApplied) {
        return Err(refused(id, &act, refusal));
    }
    let mut commits: Vec<String> = Vec::new();
    for text in &written {
        let commit = commit_that_changed(context, text, &note.target.path)?;
        let full = commit.as_hex().to_owned();
        if !commits.contains(&full) {
            commits.push(full);
        }
    }
    let count = commits.len();
    let note = act_as_agent(context, args, id, &Act::Applied { commits })?;
    let noun = if count == 1 { "commit" } else { "commits" };
    Ok(json!({
        "schema_version": 1,
        "note": short(&note),
        "applied": note.applied,
        "message": format!(
            "Recorded {count} {noun} on n{id}, which is now applied. The change page of its range marks the changes they made as \"changed because of n{id}\"."
        ),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_texts_are_shortened_to_a_whole_character() {
        assert_eq!(shortened("short", 10), "short");
        assert_eq!(shortened("exactly", 7), "exactly");
        assert_eq!(shortened("ჯგუფი და ტექსტი", 5), "ჯგუფ…");
    }

    #[test]
    fn every_refusal_says_why_and_what_to_do_instead() {
        let edit = Act::Edit(crate::notes::NoteEdit::default());
        let decide = Act::Decide {
            outcome: Outcome::Change,
            summary: String::new(),
        };
        for (act, word) in [
            (&edit, "edit"),
            (&Act::Send, "send"),
            (&decide, "decide"),
            (&Act::Withdraw, "withdraw"),
        ] {
            let said = refused(3, act, Refusal::ReaderOnly);
            assert!(
                said.starts_with(&format!("only the reader can {word} a note")),
                "{said}"
            );
            assert!(said.contains("note_reply"), "{said}");
        }
        let reply = Act::Reply {
            body: String::new(),
            stance: None,
        };
        assert!(refused(3, &reply, Refusal::ReaderOnly).contains("can do this to a note"));
        assert!(
            refused(3, &reply, Refusal::Final(Status::Withdrawn)).starts_with("n3 is withdrawn")
        );
        assert!(refused(3, &reply, Refusal::Final(Status::Closed)).starts_with("n3 is closed"));
        assert!(refused(3, &reply, Refusal::Final(Status::Applied)).starts_with("n3 is applied"));
        assert!(
            refused(3, &reply, Refusal::AlreadyDecided)
                .contains("decided already, so it takes no new proposal")
        );
        assert!(
            refused(3, &reply, Refusal::NotDecided(Status::Sent))
                .starts_with("n3 is sent, not decided")
        );
        assert_eq!(
            refused(3, &reply, Refusal::AgentOnly),
            Refusal::AgentOnly.to_string()
        );
    }

    #[test]
    fn list_values_are_read_or_refused_with_the_known_ones() {
        let args =
            Args::of(json!({"stance": ["agree", " question "], "bad": "nope", "status": "open"}));
        assert_eq!(
            values(&args, "stance", stance_of, "x"),
            Ok(Some(vec![Stance::Agree, Stance::Question]))
        );
        assert_eq!(values(&args, "missing", stance_of, "x"), Ok(None));
        assert_eq!(
            values(&args, "bad", stance_of, "agree, disagree"),
            Err("`bad` has no value `nope`; the values are agree, disagree".to_owned())
        );
        assert_eq!(
            values(&args, "status", Status::from_name, "x"),
            Ok(Some(vec![Status::Open]))
        );
    }

    #[test]
    fn a_review_is_written_with_its_kind() {
        let range = Review {
            id: 2,
            scope: Scope::Range("main...feature".into()),
            created_at: 1,
        };
        assert_eq!(
            review_json(&range),
            json!({"id": 2, "kind": "range", "scope": "main...feature"})
        );
        let document = Review {
            id: 3,
            scope: Scope::Document("docs/a.md".into()),
            created_at: 1,
        };
        assert_eq!(review_json(&document)["kind"], "document");
    }
}
