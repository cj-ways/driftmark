//! The JSON names of notes: the shape the notes file, the server and the
//! MCP connector share. `docs/features/engine/notes-model.md` writes it out.

use driftmark_core::notes::{
    Anchor, AnchorState, Applied, Ask, Decision, Event, EventAction, Note, Outcome, Quote, Reply,
    ReplyStance, Role, Side, Source, Stance, Status, Target,
};

#[test]
fn statuses_and_states_use_their_names() {
    for status in Status::ALL {
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, format!("\"{}\"", status.name()));
    }
    for state in [
        AnchorState::Exact,
        AnchorState::Moved,
        AnchorState::Fuzzy,
        AnchorState::Outdated,
        AnchorState::Orphaned,
    ] {
        assert_eq!(serde_json::to_value(state).unwrap(), state.name());
    }
}

#[test]
fn a_note_reads_and_writes_its_json_names() {
    let note = Note {
        id: 12,
        review: 3,
        target: Target {
            path: "docs/a.md".into(),
            commit: Some("a".repeat(40)),
            blob: "b".repeat(40),
            side: Side::New,
            anchor: Anchor {
                start: 4,
                end: 9,
                quote: Quote {
                    exact: "hello".into(),
                    prefix: "# A\n".into(),
                    suffix: String::new(),
                },
                heading_path: vec!["A".into()],
                block: 1,
                block_hash: "00000000000000ff".into(),
            },
        },
        stance: Stance::Suggestion,
        ask: Ask::Explain,
        blocking: true,
        body: "Why?".into(),
        status: Status::Answered,
        thread: vec![Reply {
            role: Role::Agent,
            by: "claude".into(),
            at: 5,
            stance: Some(ReplyStance::Counter),
            body: "Because.".into(),
        }],
        proposal: Some(Decision {
            outcome: Outcome::Modify,
            summary: "Say why.".into(),
            by: "claude".into(),
            at: 6,
        }),
        decision: None,
        applied: vec![Applied {
            commit: "c".repeat(40),
            by: "claude".into(),
            at: 7,
        }],
        author: "reader".into(),
        created_at: 1,
        updated_at: 7,
        history: vec![
            Event::new(
                EventAction::Made,
                Role::Reader,
                "reader".into(),
                1,
                Source::Page,
                Some("Firefox".into()),
            ),
            Event::new(
                EventAction::Reply,
                Role::Agent,
                "claude".into(),
                5,
                Source::Mcp,
                Some("claude 1.0".into()),
            ),
        ],
    };
    let json = serde_json::to_value(&note).unwrap();
    assert_eq!(json["target"]["side"], "new");
    assert_eq!(json["target"]["start"], 4);
    assert_eq!(json["target"]["quote"]["exact"], "hello");
    assert_eq!(json["stance"], "suggestion");
    assert_eq!(json["ask"], "explain");
    assert_eq!(json["thread"][0]["role"], "agent");
    assert_eq!(json["thread"][0]["stance"], "counter");
    assert_eq!(json["proposal"]["outcome"], "modify");
    assert_eq!(json["history"][0]["action"], "made");
    assert_eq!(json["history"][1]["source"], "mcp");
    assert_eq!(json["history"][1]["user_agent"], "claude 1.0");
    let back: Note = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(back, note);
    // A note written before notes had a history reads with an empty one.
    let mut old = json;
    old.as_object_mut().unwrap().remove("history");
    let back: Note = serde_json::from_value(old).unwrap();
    assert_eq!(back.history, Vec::<Event>::new());
    let quiet = Reply {
        stance: None,
        ..note.thread[0].clone()
    };
    assert!(
        serde_json::to_value(&quiet)
            .unwrap()
            .get("stance")
            .is_none()
    );
    for (side, name) in [(Side::Reading, "reading"), (Side::Old, "old")] {
        assert_eq!(serde_json::to_value(side).unwrap(), name);
    }
    assert_eq!(serde_json::to_value(Role::Reader).unwrap(), "reader");
}

#[test]
fn a_history_entry_reads_and_writes_its_json_names() {
    for source in Source::ALL {
        assert_eq!(serde_json::to_value(source).unwrap(), source.name());
    }
    for action in EventAction::ALL {
        assert_eq!(serde_json::to_value(action).unwrap(), action.name());
    }
    let quiet = Event::new(
        EventAction::Decide,
        Role::Reader,
        "Ana".into(),
        5,
        Source::Page,
        Some("Firefox".into()),
    );
    let json = serde_json::to_value(&quiet).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "action": "decide", "role": "reader", "by": "Ana", "at": 5,
            "source": "page", "user_agent": "Firefox",
        })
    );
    let loud = Event::new(
        EventAction::Decide,
        Role::Reader,
        "Ana".into(),
        5,
        Source::Api,
        None,
    );
    let json = serde_json::to_value(&loud).unwrap();
    assert_eq!(json["warning"], "Decided through the API, not on the page");
    assert!(json.get("user_agent").is_none());
    let back: Event = serde_json::from_value(json).unwrap();
    assert_eq!(back, loud);
}
