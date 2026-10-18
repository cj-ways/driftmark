use std::sync::atomic::{AtomicUsize, Ordering};

use driftmark_core::notes::{Anchor, Quote, Source};

use super::*;

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A notes file in a fresh temporary folder, removed afterwards.
struct Temp(PathBuf);

impl Temp {
    fn new() -> Temp {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("driftmark-store-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Temp(dir)
    }

    fn file(&self) -> PathBuf {
        self.0.join("driftmark").join("notes.db")
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The reader's page, as the server records it.
fn page() -> Via {
    Via::new(Source::Page, Some("Firefox"))
}

fn new_note(path: &str, quote: &str) -> NewNote {
    NewNote {
        target: Target {
            path: path.to_owned(),
            commit: None,
            blob: "a".repeat(40),
            side: Side::Reading,
            anchor: Anchor {
                start: 5,
                end: 5 + quote.len(),
                quote: Quote {
                    exact: quote.to_owned(),
                    prefix: "# A\n\n".to_owned(),
                    suffix: ".\n".to_owned(),
                },
                heading_path: vec!["A".to_owned()],
                block: 1,
                block_hash: "0123456789abcdef".to_owned(),
            },
        },
        stance: Stance::Disagree,
        ask: Ask::Change,
        blocking: true,
        body: "Too long.".to_owned(),
        author: "ana".to_owned(),
    }
}

#[test]
fn a_new_file_gets_its_folder_its_schema_and_wal() {
    let temp = Temp::new();
    let store = Store::open(&temp.file()).unwrap();
    assert_eq!(store.path(), temp.file());
    assert!(format!("{store:?}").contains("notes.db"));
    store
        .with(|connection| {
            let version: i64 =
                connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
            assert_eq!(version, SCHEMA_VERSION);
            let mode: String =
                connection.pragma_query_value(None, "journal_mode", |row| row.get(0))?;
            assert_eq!(mode, "wal");
            Ok(())
        })
        .unwrap();
}

#[test]
fn a_note_is_kept_with_every_field() {
    let temp = Temp::new();
    let store = Store::open(&temp.file()).unwrap();
    let scope = Scope::Document("docs/a.md".into());
    let note = store
        .add(&scope, &new_note("docs/a.md", "one hour"), 100, &page())
        .unwrap();
    assert_eq!(note.id, 1);
    assert_eq!(note.status, Status::Open);
    assert_eq!(note.target, new_note("docs/a.md", "one hour").target);
    assert_eq!(
        (note.stance, note.ask, note.blocking, note.body.as_str()),
        (Stance::Disagree, Ask::Change, true, "Too long.")
    );
    assert_eq!((note.created_at, note.updated_at), (100, 100));
    assert_eq!(note.author, "ana");
    let review = store.review(note.review).unwrap();
    assert_eq!(review.scope, scope);
    assert_eq!(review.created_at, 100);
    assert_eq!(store.find_review(&scope).unwrap(), Some(review.clone()));
    assert_eq!(store.review_for(&scope, 200).unwrap(), review);
    assert_eq!(
        store.find_review(&Scope::Range("main...x".into())).unwrap(),
        None
    );
    let mut with_commit = new_note("docs/a.md", "two");
    with_commit.target.commit = Some("c".repeat(40));
    with_commit.target.side = Side::Old;
    let second = store.add(&scope, &with_commit, 101, &page()).unwrap();
    assert_eq!(second.target.commit, Some("c".repeat(40)));
    assert_eq!(second.target.side, Side::Old);
    let other = store
        .add(
            &Scope::Range("main...feature".into()),
            &new_note("docs/b.md", "x"),
            102,
            &page(),
        )
        .unwrap();
    assert_ne!(other.review, note.review);
    assert_eq!(
        store.review(other.review).unwrap().scope.text(),
        "main...feature"
    );
    let ids = |notes: Vec<Note>| notes.iter().map(|note| note.id).collect::<Vec<_>>();
    assert_eq!(ids(store.notes_on("docs/a.md").unwrap()), [1, 2]);
    assert_eq!(store.count_on("docs/a.md").unwrap(), 2);
    assert_eq!(store.count_on("docs/none.md").unwrap(), 0);
    assert_eq!(ids(store.notes_in(other.review).unwrap()), [3]);
    let paths: HashSet<String> = ["docs/b.md".to_owned(), "docs/z.md".to_owned()].into();
    assert_eq!(ids(store.notes_on_any(&paths).unwrap()), [3]);
    assert_eq!(
        store.notes_on_any(&HashSet::new()).unwrap(),
        Vec::<Note>::new()
    );
    assert_eq!(store.note(9), Err(StoreError::NoNote(9)));
    assert_eq!(store.review(9), Err(StoreError::NoReview(9)));
}

#[test]
fn the_loop_runs_through_the_file() {
    let temp = Temp::new();
    let store = Store::open(&temp.file()).unwrap();
    let scope = Scope::Document("docs/a.md".into());
    let id = store
        .add(&scope, &new_note("docs/a.md", "one hour"), 1, &page())
        .unwrap()
        .id;
    edit_twice(&store, id);
    let sent = store
        .act(id, Role::Reader, "ana", &Act::Send, 3, &page())
        .unwrap();
    assert_eq!(sent.status, Status::Sent);
    let answered = store
        .act(
            id,
            Role::Agent,
            "agent-1",
            &Act::Reply {
                body: "It matches the old server.".into(),
                stance: Some(ReplyStance::Counter),
            },
            4,
            &page(),
        )
        .unwrap();
    assert_eq!(answered.status, Status::Answered);
    let proposed = store
        .act(
            id,
            Role::Agent,
            "agent-1",
            &Act::Propose {
                outcome: Outcome::Modify,
                summary: "Ten minutes.".into(),
            },
            5,
            &page(),
        )
        .unwrap();
    assert_eq!(
        proposed
            .proposal
            .as_ref()
            .map(|p| (p.outcome, p.by.as_str(), p.at)),
        Some((Outcome::Modify, "agent-1", 5))
    );
    let discussing = store
        .act(
            id,
            Role::Reader,
            "ana",
            &Act::Reply {
                body: "Ten is fine.".into(),
                stance: None,
            },
            6,
            &page(),
        )
        .unwrap();
    assert_eq!(discussing.status, Status::Discussing);
    assert_eq!(discussing.thread.len(), 2);
    assert_eq!(discussing.thread[0].role, Role::Agent);
    assert_eq!(discussing.thread[0].stance, Some(ReplyStance::Counter));
    assert_eq!(discussing.thread[1].stance, None);
    assert_eq!(discussing.thread[1].by, "ana");
    decide_and_apply(&store, &scope, id);
}

/// Two edits of note `id`: each changes only the fields it names.
fn edit_twice(store: &Store, id: u64) {
    let edited = store
        .act(
            id,
            Role::Reader,
            "ana",
            &Act::Edit(NoteEdit {
                stance: Some(Stance::Question),
                body: Some("Why one hour?".into()),
                ..NoteEdit::default()
            }),
            2,
            &page(),
        )
        .unwrap();
    assert_eq!(
        (
            edited.stance,
            edited.ask,
            edited.blocking,
            edited.body.as_str()
        ),
        (Stance::Question, Ask::Change, true, "Why one hour?")
    );
    assert_eq!(edited.updated_at, 2);
    let edited = store
        .act(
            id,
            Role::Reader,
            "ana",
            &Act::Edit(NoteEdit {
                ask: Some(Ask::Explain),
                blocking: Some(false),
                ..NoteEdit::default()
            }),
            2,
            &page(),
        )
        .unwrap();
    assert_eq!((edited.ask, edited.blocking), (Ask::Explain, false));
}

/// The second half of the loop: the decision, the commits, and what comes
/// after them.
fn decide_and_apply(store: &Store, scope: &Scope, id: u64) {
    let refused = store.act(
        id,
        Role::Agent,
        "agent-1",
        &Act::Decide {
            outcome: Outcome::Change,
            summary: "x".into(),
        },
        7,
        &page(),
    );
    assert_eq!(refused, Err(StoreError::Refused(Refusal::ReaderOnly)));
    let decided = store
        .act(
            id,
            Role::Reader,
            "ana",
            &Act::Decide {
                outcome: Outcome::Modify,
                summary: "Ten minutes, everywhere.".into(),
            },
            8,
            &page(),
        )
        .unwrap();
    assert_eq!(decided.status, Status::Decided);
    assert_eq!(
        decided.decision.as_ref().map(|d| d.summary.as_str()),
        Some("Ten minutes, everywhere.")
    );
    let applied = store
        .act(
            id,
            Role::Agent,
            "agent-1",
            &Act::Applied {
                commits: vec!["b".repeat(40), "a".repeat(40), "b".repeat(40)],
            },
            9,
            &page(),
        )
        .unwrap();
    assert_eq!(applied.status, Status::Applied);
    let commits: Vec<&str> = applied.applied.iter().map(|a| a.commit.as_str()).collect();
    assert_eq!(commits, ["a".repeat(40), "b".repeat(40)]);
    assert_eq!(
        store.act(id, Role::Reader, "ana", &Act::Withdraw, 10, &page()),
        Err(StoreError::Refused(Refusal::Final(Status::Applied)))
    );
    let other = store
        .add(scope, &new_note("docs/a.md", "x"), 11, &page())
        .unwrap()
        .id;
    let gone = store
        .act(other, Role::Reader, "ana", &Act::Withdraw, 12, &page())
        .unwrap();
    assert_eq!(gone.status, Status::Withdrawn);
    // Replies and commits stay with their own note.
    assert_eq!(store.note(other).unwrap().thread, Vec::<Reply>::new());
    assert_eq!(store.note(other).unwrap().applied, Vec::<Applied>::new());
    assert_eq!(
        store.act(77, Role::Reader, "ana", &Act::Send, 1, &page()),
        Err(StoreError::NoNote(77))
    );
}

#[test]
fn versions_and_seen_marks_are_kept() {
    let temp = Temp::new();
    let store = Store::open(&temp.file()).unwrap();
    assert_eq!(store.version("x").unwrap(), None);
    store.keep_version("x", b"# A\n").unwrap();
    store.keep_version("x", b"other bytes are ignored").unwrap();
    assert_eq!(store.version("x").unwrap(), Some(b"# A\n".to_vec()));
    let keys = vec!["k1".to_owned(), "k2".to_owned(), "k3".to_owned()];
    assert!(store.seen("ana", &keys).unwrap().is_empty());
    assert!(store.seen("ana", &[]).unwrap().is_empty());
    store.set_seen("ana", "k1", true, 1, &page()).unwrap();
    store.set_seen("ana", "k1", true, 2, &page()).unwrap();
    store.set_seen("ana", "k3", true, 2, &page()).unwrap();
    store.set_seen("bo", "k2", true, 2, &page()).unwrap();
    let seen = store.seen("ana", &keys).unwrap();
    assert_eq!(seen, ["k1".to_owned(), "k3".to_owned()].into());
    store.set_seen("ana", "k1", false, 3, &page()).unwrap();
    assert_eq!(store.seen("ana", &keys).unwrap(), ["k3".to_owned()].into());
    assert_eq!(store.seen("bo", &keys).unwrap(), ["k2".to_owned()].into());
}

#[test]
fn two_processes_share_the_file() {
    let temp = Temp::new();
    let serve = Store::open(&temp.file()).unwrap();
    let mcp = Store::open(&temp.file()).unwrap();
    let scope = Scope::Document("docs/a.md".into());
    let id = serve
        .add(&scope, &new_note("docs/a.md", "x"), 1, &page())
        .unwrap()
        .id;
    let reply = Act::Reply {
        body: "Seen.".into(),
        stance: Some(ReplyStance::Agree),
    };
    mcp.act(id, Role::Agent, "agent", &reply, 2, &page())
        .unwrap();
    assert_eq!(serve.note(id).unwrap().status, Status::Answered);
    // Reopening keeps everything and runs no migration twice.
    drop((serve, mcp));
    let again = Store::open(&temp.file()).unwrap();
    assert_eq!(again.note(id).unwrap().thread.len(), 1);
}

#[test]
fn a_decision_and_a_proposal_at_once_never_overwrite_each_other() {
    // The reader decides through one handle, then an agent proposes through
    // another. The proposal reads the status the decision left, inside its
    // own write, so it is refused instead of writing "answered" over
    // "decided".
    let temp = Temp::new();
    let serve = Store::open(&temp.file()).unwrap();
    let mcp = Store::open(&temp.file()).unwrap();
    let scope = Scope::Document("docs/a.md".into());
    let id = serve
        .add(&scope, &new_note("docs/a.md", "x"), 1, &page())
        .unwrap()
        .id;
    serve
        .act(
            id,
            Role::Reader,
            "ana",
            &Act::Decide {
                outcome: Outcome::Change,
                summary: "Yes.".into(),
            },
            2,
            &page(),
        )
        .unwrap();
    let late = mcp.act(
        id,
        Role::Agent,
        "agent",
        &Act::Propose {
            outcome: Outcome::Keep,
            summary: "No.".into(),
        },
        3,
        &page(),
    );
    assert_eq!(late, Err(StoreError::Refused(Refusal::AlreadyDecided)));
    let note = serve.note(id).unwrap();
    assert_eq!(note.status, Status::Decided);
    assert_eq!(note.proposal, None);
    // A writer that holds the file makes the other wait, then go on.
    let holder = Connection::open(temp.file()).unwrap();
    holder.execute_batch("BEGIN IMMEDIATE").unwrap();
    let waited = std::thread::spawn(move || {
        mcp.act(
            id,
            Role::Agent,
            "agent",
            &Act::Reply {
                body: "Applying.".into(),
                stance: None,
            },
            4,
            &page(),
        )
    });
    std::thread::sleep(std::time::Duration::from_millis(200));
    holder.execute_batch("COMMIT").unwrap();
    let replied = waited.join().unwrap().unwrap();
    assert_eq!(replied.status, Status::Decided);
    assert_eq!(replied.thread.len(), 1);
}

#[test]
fn a_file_from_a_newer_driftmark_is_refused() {
    let temp = Temp::new();
    drop(Store::open(&temp.file()).unwrap());
    let connection = Connection::open(temp.file()).unwrap();
    connection
        .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
        .unwrap();
    drop(connection);
    let error = Store::open(&temp.file()).unwrap_err();
    assert_eq!(
        error,
        StoreError::Newer {
            found: SCHEMA_VERSION + 1
        }
    );
    assert!(error.to_string().contains("made by a newer driftmark"));
}

#[test]
fn a_file_that_cannot_be_made_or_read_is_an_error() {
    let temp = Temp::new();
    std::fs::create_dir_all(&temp.0).unwrap();
    // A file where the folder should be.
    std::fs::write(temp.0.join("driftmark"), b"").unwrap();
    let error = Store::open(&temp.file()).unwrap_err();
    assert!(matches!(error, StoreError::File(_)), "{error:?}");
    assert!(error.to_string().starts_with("the notes file: cannot make"));
    // Not a database at all.
    let other = Temp::new();
    std::fs::create_dir_all(other.file().parent().unwrap()).unwrap();
    std::fs::write(
        other.file(),
        b"this is not SQLite, not at all, and long enough",
    )
    .unwrap();
    assert!(matches!(
        Store::open(&other.file()),
        Err(StoreError::File(_))
    ));
}

#[test]
fn stored_values_that_make_no_sense_are_reported() {
    let temp = Temp::new();
    let store = Store::open(&temp.file()).unwrap();
    let scope = Scope::Document("docs/a.md".into());
    let id = store
        .add(&scope, &new_note("docs/a.md", "x"), 1, &page())
        .unwrap()
        .id;
    for (column, value) in [
        ("status", "'done'"),
        ("stance", "'loud'"),
        ("heading_path", "'not json'"),
        ("decision", "'not json'"),
        ("start_byte", "'text'"),
        ("path", "X'07'"),
        ("commit_id", "X'07'"),
    ] {
        let temp = Temp::new();
        let broken = Store::open(&temp.file()).unwrap();
        broken
            .add(&scope, &new_note("docs/a.md", "x"), 1, &page())
            .unwrap();
        broken
            .with(|connection| {
                connection.execute(&format!("UPDATE notes SET {column} = {value}"), [])?;
                Ok(())
            })
            .unwrap();
        let error = broken.note(1).unwrap_err();
        assert!(matches!(error, StoreError::File(_)), "{column}: {error:?}");
    }
    assert!(store.note(id).is_ok());
    for (error, text) in [
        (StoreError::NoNote(4), "there is no note n4"),
        (StoreError::NoReview(5), "there is no review r5"),
        (
            StoreError::Refused(Refusal::AlreadyDecided),
            "the note is decided already",
        ),
    ] {
        assert_eq!(error.to_string(), text);
    }
}

/// One line per history entry: action, role, by, source, user agent, and
/// the warning or `-`.
fn history(note: &Note) -> Vec<String> {
    note.history
        .iter()
        .map(|event| {
            format!(
                "{} {} {} {} {} {}",
                event.action.name(),
                text_of(&event.role),
                event.by,
                event.source.name(),
                event.user_agent.as_deref().unwrap_or("-"),
                event.warning.as_deref().unwrap_or("-")
            )
        })
        .collect()
}

#[test]
fn every_action_is_kept_with_where_it_came_from() {
    let temp = Temp::new();
    let store = Store::open(&temp.file()).unwrap();
    let scope = Scope::Document("docs/a.md".into());
    let id = store
        .add(&scope, &new_note("docs/a.md", "x"), 1, &page())
        .unwrap()
        .id;
    let api = Via::new(Source::Api, Some("curl/8.5.0"));
    let mcp = Via::new(Source::Mcp, Some("claude-code 2.1"));
    let edit = Act::Edit(NoteEdit::default());
    store.act(id, Role::Reader, "ana", &edit, 2, &api).unwrap();
    store
        .act(id, Role::Reader, "ana", &Act::Send, 3, &page())
        .unwrap();
    let reply = Act::Reply {
        body: "Why?".into(),
        stance: Some(ReplyStance::Question),
    };
    store
        .act(id, Role::Agent, "claude-code", &reply, 4, &mcp)
        .unwrap();
    let decide = Act::Decide {
        outcome: Outcome::Keep,
        summary: "Keep it.".into(),
    };
    let note = store
        .act(id, Role::Reader, "ana", &decide, 5, &api)
        .unwrap();
    assert_eq!(
        history(&note),
        [
            "made reader ana page Firefox -",
            "edit reader ana api curl/8.5.0 Edited through the API, not on the page",
            "send reader ana page Firefox -",
            "reply agent claude-code mcp claude-code 2.1 -",
            "decide reader ana api curl/8.5.0 Decided through the API, not on the page",
        ]
    );
    // A refused action leaves no entry.
    let late = store.act(id, Role::Reader, "ana", &Act::Withdraw, 6, &api);
    assert_eq!(
        late,
        Err(StoreError::Refused(Refusal::Final(Status::Closed)))
    );
    assert_eq!(store.note(id).unwrap().history.len(), 5);
    // Each note has its own history.
    let other = store
        .add(&scope, &new_note("docs/a.md", "y"), 7, &api)
        .unwrap();
    assert_eq!(
        history(&other),
        ["made reader ana api curl/8.5.0 Made through the API, not on the page"]
    );
}

#[test]
fn a_seen_mark_keeps_where_it_came_from() {
    let temp = Temp::new();
    let store = Store::open(&temp.file()).unwrap();
    assert_eq!(store.seen_mark("ana", "k1").unwrap(), None);
    store.set_seen("ana", "k1", true, 1, &page()).unwrap();
    assert_eq!(
        store.seen_mark("ana", "k1").unwrap(),
        Some(SeenMark { at: 1, via: page() })
    );
    let api = Via::new(Source::Api, None);
    store.set_seen("ana", "k1", true, 2, &api).unwrap();
    assert_eq!(
        store.seen_mark("ana", "k1").unwrap(),
        Some(SeenMark {
            at: 2,
            via: api.clone()
        })
    );
    assert_eq!(store.seen_mark("bo", "k1").unwrap(), None);
    store.set_seen("ana", "k1", false, 3, &api).unwrap();
    assert_eq!(store.seen_mark("ana", "k1").unwrap(), None);
}

#[test]
fn a_user_agent_is_cut_short_and_cleaned() {
    let long = "A".repeat(AGENT_CHARS + 40);
    let via = Via::new(Source::Api, Some(&long));
    assert_eq!(via.user_agent.as_deref().map(str::len), Some(AGENT_CHARS));
    let via = Via::new(Source::Api, Some(" curl/8\u{7}\n "));
    assert_eq!(via.user_agent.as_deref(), Some("curl/8"));
    assert_eq!(Via::new(Source::Api, Some(" \t ")).user_agent, None);
    assert_eq!(Via::new(Source::Page, None).user_agent, None);
    let georgian = "ქართული ".repeat(40);
    let via = Via::new(Source::Api, Some(&georgian));
    assert_eq!(
        via.user_agent.as_deref().map(|text| text.chars().count()),
        Some(AGENT_CHARS - 1),
        "cut after a whole character, then trimmed"
    );
}

/// A notes file as version 1 of the schema left it: two notes, one of them
/// with a reply, a proposal, a decision and commits, the other withdrawn,
/// and a seen mark.
fn version_one(path: &Path) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let connection = Connection::open(path).unwrap();
    connection.execute_batch(MIGRATIONS[0].1).unwrap();
    connection
        .execute_batch(
            r#"
            INSERT INTO reviews VALUES (1, 'document', 'docs/a.md', 10);
            INSERT INTO notes VALUES (1, 1, 'docs/a.md', NULL, 'b', 'reading', 5, 8, 'one', '', '', '["A"]', 1, '0123456789abcdef',
              'disagree', 'change', 1, 'Too long.', 'applied',
              '{"outcome":"modify","summary":"Ten.","by":"agent-1","at":13}',
              '{"outcome":"modify","summary":"Ten, everywhere.","by":"ana","at":14}',
              'ana', 10, 15);
            INSERT INTO notes VALUES (2, 1, 'docs/a.md', NULL, 'b', 'reading', 5, 8, 'two', '', '', '["A"]', 1, '0123456789abcdef',
              'question', 'explain', 0, 'Why?', 'withdrawn', NULL, NULL, 'ana', 11, 16);
            INSERT INTO replies VALUES (1, 1, 'agent', 'agent-1', 12, 'counter', 'It matches.');
            INSERT INTO applied VALUES (1, 'aaaaaaa', 'agent-1', 15);
            INSERT INTO applied VALUES (1, 'bbbbbbb', 'agent-1', 15);
            INSERT INTO seen VALUES ('ana', 'k1', 9);
            PRAGMA user_version = 1;
            "#,
        )
        .unwrap();
}

#[test]
fn a_version_one_file_gets_the_sources_it_lacked() {
    let temp = Temp::new();
    version_one(&temp.file());
    let store = Store::open(&temp.file()).unwrap();
    store
        .with(|connection| {
            let version: i64 =
                connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
            assert_eq!(version, SCHEMA_VERSION);
            Ok(())
        })
        .unwrap();
    let unknown =
        "before driftmark recorded where actions come from, so it may not have come from the page";
    assert_eq!(
        history(&store.note(1).unwrap()),
        [
            format!("made reader ana unknown - Made {unknown}"),
            "reply agent agent-1 unknown - -".to_owned(),
            "propose agent agent-1 unknown - -".to_owned(),
            format!("decide reader ana unknown - Decided {unknown}"),
            "applied agent agent-1 unknown - -".to_owned(),
        ]
    );
    assert_eq!(
        history(&store.note(2).unwrap()),
        [
            format!("made reader ana unknown - Made {unknown}"),
            format!("withdraw reader ana unknown - Withdrawn {unknown}"),
        ]
    );
    assert_eq!(
        store.seen_mark("ana", "k1").unwrap(),
        Some(SeenMark {
            at: 9,
            via: Via::new(Source::Unknown, None)
        })
    );
    // The notes keep everything else, and new actions record their source.
    let note = store.note(1).unwrap();
    assert_eq!(note.thread.len(), 1);
    assert_eq!(note.applied.len(), 2);
    let added = store
        .add(
            &Scope::Document("docs/a.md".into()),
            &new_note("docs/a.md", "x"),
            20,
            &page(),
        )
        .unwrap();
    assert_eq!(added.id, 3);
    assert_eq!(history(&added), ["made reader ana page Firefox -"]);
    // Opening it again runs no migration twice.
    drop(store);
    let again = Store::open(&temp.file()).unwrap();
    assert_eq!(again.note(1).unwrap().history.len(), 5);
}

#[test]
fn stored_actions_that_make_no_sense_are_reported() {
    for (column, value) in [
        ("action", "'shout'"),
        ("source", "'phone'"),
        ("role", "'boss'"),
    ] {
        let temp = Temp::new();
        let store = Store::open(&temp.file()).unwrap();
        store
            .add(
                &Scope::Document("docs/a.md".into()),
                &new_note("docs/a.md", "x"),
                1,
                &page(),
            )
            .unwrap();
        store
            .with(|connection| {
                connection.execute(&format!("UPDATE actions SET {column} = {value}"), [])?;
                Ok(())
            })
            .unwrap();
        let error = store.note(1).unwrap_err();
        assert!(matches!(error, StoreError::File(_)), "{column}: {error:?}");
    }
    let temp = Temp::new();
    let store = Store::open(&temp.file()).unwrap();
    store.set_seen("ana", "k", true, 1, &page()).unwrap();
    store
        .with(|connection| {
            connection.execute("UPDATE seen SET source = 'phone'", [])?;
            Ok(())
        })
        .unwrap();
    assert!(matches!(
        store.seen_mark("ana", "k"),
        Err(StoreError::File(_))
    ));
}
