use super::*;
use crate::notes::{
    Anchor, Applied, Ask, Decision, Outcome, Quote, Reply, ReplyStance, Stance, Target,
};

fn note(id: u64, status: Status) -> Note {
    Note {
        id,
        review: 3,
        target: Target {
            path: "docs/cache.md".into(),
            commit: Some("0123456789abcdef0123456789abcdef01234567".into()),
            blob: "b".repeat(40),
            side: Side::New,
            anchor: Anchor {
                start: 30,
                end: 70,
                quote: Quote {
                    exact: "Keep each page for one hour.".into(),
                    prefix: String::new(),
                    suffix: String::new(),
                },
                heading_path: vec!["Cache pages".into(), "Decision".into()],
                block: 4,
                block_hash: "0".repeat(16),
            },
        },
        stance: Stance::Disagree,
        ask: Ask::Change,
        blocking: true,
        body: "One hour is too long.\nPhones keep old pages.".into(),
        status,
        thread: Vec::new(),
        proposal: None,
        decision: None,
        applied: Vec::new(),
        author: "ana".into(),
        created_at: 1,
        updated_at: 1,
        history: Vec::new(),
    }
}

fn placed(state: AnchorState) -> Placement {
    Placement {
        state,
        range: Some(30..58),
        score: 1.0,
        block: Some(4),
    }
}

const REVIEW: Review<'static> = Review {
    id: 3,
    scope: "main...feature",
    at: "6a4b0aa",
};

#[test]
fn round_one_lists_the_notes_under_discussion() {
    let first = note(12, Status::Open);
    let mut second = note(13, Status::Answered);
    second.stance = Stance::Question;
    second.blocking = false;
    second.ask = Ask::Explain;
    second.target.side = Side::Old;
    second.target.anchor.heading_path.clear();
    second.thread.push(Reply {
        role: Role::Agent,
        by: "agent".into(),
        at: 2,
        stance: Some(ReplyStance::Counter),
        body: "The hour matches the old server.".into(),
    });
    second.thread.push(Reply {
        role: Role::Reader,
        by: "ana".into(),
        at: 3,
        stance: None,
        body: "The old server is not the target.".into(),
    });
    second.proposal = Some(Decision {
        outcome: Outcome::Modify,
        summary: "Make it ten minutes.".into(),
        by: "agent".into(),
        at: 4,
    });
    let decided = note(14, Status::Decided);
    let moved = placed(AnchorState::Moved);
    let fuzzy = placed(AnchorState::Fuzzy);
    let items = [
        Item {
            note: &first,
            placement: Some(&moved),
            lines: Some((12, 12)),
            current: None,
        },
        Item {
            note: &decided,
            placement: None,
            lines: None,
            current: None,
        },
        Item {
            note: &second,
            placement: Some(&fuzzy),
            lines: Some((12, 13)),
            current: Some("Keep each page for two hours.\n\nThen drop it."),
        },
    ];
    let text = prompt(&REVIEW, &items, Round::Discuss);
    assert_eq!(
        text,
        "Review r3 · main...feature · 2 notes · at 6a4b0aa
Round 1: discuss only. For each note, say whether you agree, disagree or have a question, and why.
Do not edit the documents yet. Answer each note with the note_reply tool, or write your answer on its \"Agent:\" line.

## 1 · n12 · disagree · blocking · ask: change
File: docs/cache.md, line 12, under Cache pages › Decision
> Keep each page for one hour.
The quoted text has moved since the note.
Reader: One hour is too long.
  Phones keep old pages.
Agent:

## 2 · n13 · question · ask: explain
File: docs/cache.md, lines 12–13
On the old side of a change, at 0123456: text the newer version removed or changed.
> Keep each page for one hour.
The text changed after the note. It now reads:
> Keep each page for two hours.
>
> Then drop it.
Reader: One hour is too long.
  Phones keep old pages.
Agent: The hour matches the old server.
Reader: The old server is not the target.
Proposed: Make it ten minutes.
Agent:
"
    );
}

#[test]
fn round_two_lists_the_decided_notes_with_their_decisions() {
    let mut decided = note(14, Status::Decided);
    decided.decision = Some(Decision {
        outcome: Outcome::Modify,
        summary: "Ten minutes.".into(),
        by: "ana".into(),
        at: 5,
    });
    decided.applied.push(Applied {
        commit: "c".repeat(40),
        by: "agent".into(),
        at: 6,
    });
    let open = note(15, Status::Open);
    let outdated = placed(AnchorState::Outdated);
    let orphaned = Placement::orphaned();
    let mut gone = decided.clone();
    gone.id = 16;
    gone.target.side = Side::Old;
    gone.target.commit = None;
    let items = [
        Item {
            note: &decided,
            placement: Some(&outdated),
            lines: Some((3, 4)),
            current: None,
        },
        Item {
            note: &open,
            placement: None,
            lines: None,
            current: None,
        },
        Item {
            note: &gone,
            placement: Some(&orphaned),
            lines: None,
            current: None,
        },
    ];
    let text = prompt(&REVIEW, &items, Round::Apply);
    assert_eq!(
        text,
        "Review r3 · main...feature · 2 decided notes · at 6a4b0aa
Round 2: apply the decided notes. Change each document as its decision says, and nothing else.
Then record each commit with the note_mark_applied tool, or write it on the note's \"Commit:\" line.

## 1 · n14 · decided: modify
File: docs/cache.md, lines 3–4, under Cache pages › Decision
> Keep each page for one hour.
The text changed after the note.
Reader: One hour is too long.
  Phones keep old pages.
Decision: Ten minutes.
Commit:

## 2 · n16 · decided: modify
File: docs/cache.md, under Cache pages › Decision
On the old side of a change, at the working tree: text the newer version removed or changed.
> Keep each page for one hour.
The quoted text is no longer in the document.
Reader: One hour is too long.
  Phones keep old pages.
Decision: Ten minutes.
Commit:
"
    );
}

#[test]
fn one_note_and_the_other_words() {
    let mut only = note(1, Status::Sent);
    only.stance = Stance::Agree;
    only.blocking = false;
    only.ask = Ask::Keep;
    only.body = String::new();
    let exact = placed(AnchorState::Exact);
    let items = [Item {
        note: &only,
        placement: Some(&exact),
        lines: None,
        current: None,
    }];
    let text = prompt(&REVIEW, &items, Round::Discuss);
    assert!(text.starts_with("Review r3 · main...feature · 1 note · at 6a4b0aa\n"));
    assert!(text.contains("## 1 · n1 · agree · ask: keep\n"));
    assert!(text.contains("> Keep each page for one hour.\nReader: \nAgent:\n"));
    let mut kept = note(2, Status::Decided);
    for (outcome, word) in [(Outcome::Change, "change"), (Outcome::Keep, "keep")] {
        kept.decision = Some(Decision {
            outcome,
            summary: "s".into(),
            by: "ana".into(),
            at: 1,
        });
        let items = [Item {
            note: &kept,
            placement: None,
            lines: None,
            current: None,
        }];
        let text = prompt(&REVIEW, &items, Round::Apply);
        assert!(text.contains(" · 1 decided note · "), "{text}");
        assert!(text.contains(&format!("## 1 · n2 · decided: {word}\n")));
    }
    let mut suggestion = note(3, Status::Open);
    suggestion.stance = Stance::Suggestion;
    let none: [Item<'_>; 0] = [];
    assert!(prompt(&REVIEW, &none, Round::Discuss).contains(" · 0 notes · "));
    let items = [Item {
        note: &suggestion,
        placement: None,
        lines: None,
        current: None,
    }];
    assert!(prompt(&REVIEW, &items, Round::Discuss).contains(" · suggestion · blocking · "));
}

#[test]
fn a_review_file_is_the_prompt_under_a_title() {
    let open = note(12, Status::Open);
    let items = [Item {
        note: &open,
        placement: None,
        lines: None,
        current: None,
    }];
    for (round, words) in [
        (Round::Discuss, "round 1, discuss"),
        (Round::Apply, "round 2, apply"),
    ] {
        let file = review_file(&REVIEW, &items, round, "2026-10-18 09:30 UTC");
        let head = format!(
            "# Review r3 of main...feature\n\n> Written by driftmark on 2026-10-18 09:30 UTC, {words}. The notes file is the source; this file is a copy.\n\n"
        );
        assert_eq!(file, format!("{head}{}", prompt(&REVIEW, &items, round)));
    }
}

#[test]
fn rounds_choose_their_notes() {
    let discuss: Vec<Status> = Status::ALL
        .into_iter()
        .filter(|&status| Round::Discuss.includes(&note(1, status)))
        .collect();
    assert_eq!(
        discuss,
        [
            Status::Open,
            Status::Sent,
            Status::Answered,
            Status::Discussing
        ]
    );
    let apply: Vec<Status> = Status::ALL
        .into_iter()
        .filter(|&status| Round::Apply.includes(&note(1, status)))
        .collect();
    assert_eq!(apply, [Status::Decided]);
}

#[test]
fn lines_count_from_one() {
    let source = "a\nbc\nd\n";
    assert_eq!(lines(source, &(0..1)), (1, 1));
    assert_eq!(lines(source, &(2..4)), (2, 2));
    assert_eq!(lines(source, &(2..5)), (2, 2));
    assert_eq!(lines(source, &(2..6)), (2, 3));
    assert_eq!(lines(source, &(5..5)), (3, 3));
    assert_eq!(lines(source, &(7..7)), (4, 4));
}
