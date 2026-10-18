use std::fmt::Write as _;

use super::*;
use crate::diff::{
    BlockRef, Change, ChangeKind, Level, Side as ChangeSide, Subject, WholeDocument,
};
use crate::document::BlockKind;

#[test]
fn statuses_have_names_both_ways() {
    let names: Vec<&str> = Status::ALL.iter().map(|status| status.name()).collect();
    assert_eq!(
        names,
        [
            "open",
            "sent",
            "answered",
            "discussing",
            "decided",
            "applied",
            "closed",
            "withdrawn"
        ]
    );
    for status in Status::ALL {
        assert_eq!(Status::from_name(status.name()), Some(status));
    }
    assert_eq!(Status::from_name("done"), None);
    let finals: Vec<Status> = Status::ALL.into_iter().filter(|s| s.is_final()).collect();
    assert_eq!(finals, [Status::Applied, Status::Closed, Status::Withdrawn]);
}

/// Every action by both roles from every status, written out as a table:
/// the status after it, or `-` with the refusal.
#[test]
fn the_loop_follows_its_rules() {
    let actions = [
        ("edit", Action::Edit),
        ("send", Action::Send),
        ("reply", Action::Reply),
        ("propose", Action::Propose),
        ("decide change", Action::Decide(Outcome::Change)),
        ("decide modify", Action::Decide(Outcome::Modify)),
        ("decide keep", Action::Decide(Outcome::Keep)),
        ("applied", Action::MarkApplied),
        ("withdraw", Action::Withdraw),
    ];
    let mut table = String::new();
    for status in Status::ALL {
        for (name, action) in actions {
            for (who, role) in [("reader", Role::Reader), ("agent", Role::Agent)] {
                let after = match next_status(status, role, action) {
                    Ok(next) => next.name().to_owned(),
                    Err(refusal) => format!("- {refusal}"),
                };
                let _ = writeln!(table, "{} {name} by {who}: {after}", status.name());
            }
        }
    }
    let expected = include_str!("loop.expected.txt");
    assert_eq!(table, expected, "\n{table}");
}

#[test]
fn refusals_read_as_sentences() {
    let error: &dyn std::error::Error = &Refusal::ReaderOnly;
    assert!(error.source().is_none());
    assert_eq!(
        Refusal::NotDecided(Status::Open).to_string(),
        "only a decided note can be applied; this one is open"
    );
}

fn block(text: &str) -> BlockRef {
    BlockRef {
        kind: BlockKind::Paragraph,
        start: 0,
        end: text.len(),
        line: 1,
        heading_path: std::sync::Arc::from(Vec::<String>::new()),
        text: text.to_owned(),
    }
}

fn change(kind: ChangeKind, old: Option<&str>, new: Option<&str>) -> Change {
    Change {
        kind,
        level: Level::Wording,
        signals: Vec::new(),
        subject: Subject::Block {
            old: ChangeSide {
                at: 0,
                block: old.map(block),
            },
            new: ChangeSide {
                at: 0,
                block: new.map(block),
            },
        },
        edit: None,
    }
}

#[test]
fn a_change_key_follows_the_text_not_the_place() {
    let edited = change(ChangeKind::Edited, Some("one hour"), Some("two hours"));
    let key = change_key("docs/a.md", &edited);
    assert_eq!(key.len(), 16);
    assert!(key.bytes().all(|b| b.is_ascii_hexdigit()));
    let mut elsewhere = edited.clone();
    if let Subject::Block { old, new } = &mut elsewhere.subject {
        old.at = 9;
        new.at = 12;
    }
    assert_eq!(change_key("docs/a.md", &elsewhere), key);
    let others = [
        change_key("docs/b.md", &edited),
        change_key(
            "docs/a.md",
            &change(ChangeKind::Edited, Some("one hour"), Some("three hours")),
        ),
        change_key(
            "docs/a.md",
            &change(ChangeKind::MovedEdited, Some("one hour"), Some("two hours")),
        ),
        change_key("docs/a.md", &change(ChangeKind::Added, None, Some("x"))),
        change_key("docs/a.md", &change(ChangeKind::Removed, Some("x"), None)),
        change_key(
            "docs/a.md",
            &change(ChangeKind::Formatting, Some("x"), Some("x")),
        ),
        change_key(
            "docs/a.md",
            &change(ChangeKind::Moved, Some("x"), Some("x")),
        ),
    ];
    for (at, other) in others.iter().enumerate() {
        assert_ne!(other, &key, "{at}");
        for later in &others[at + 1..] {
            assert_ne!(other, later);
        }
    }
    let whole = |source: &str| Change {
        kind: ChangeKind::Added,
        level: Level::Fact,
        signals: Vec::new(),
        subject: Subject::Document {
            document: WholeDocument {
                title: None,
                summary: None,
                bytes: source.len(),
                lines: 1,
                source: source.to_owned(),
            },
        },
        edit: None,
    };
    assert_ne!(
        change_key("n.md", &whole("# A\n")),
        change_key("n.md", &whole("# B\n"))
    );
    assert_eq!(
        change_key("n.md", &whole("# A\n")),
        change_key("n.md", &whole("# A\n"))
    );
}
