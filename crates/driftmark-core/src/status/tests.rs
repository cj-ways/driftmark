use super::*;

/// 2026-07-01 00:00 UTC.
const NOW: i64 = 1_782_864_000;

fn id(digit: char) -> ObjectId {
    ObjectId::from_hex(&digit.to_string().repeat(40)).unwrap()
}

fn entry(path: &str, doc_type: &str, lifecycle: Option<Lifecycle>) -> Entry {
    Entry {
        path: path.into(),
        title: None,
        doc_type: doc_type.into(),
        status: None,
        status_links: Vec::new(),
        lifecycle,
        date: None,
        superseded_by: None,
        superseded_by_links: Vec::new(),
    }
}

fn superseded(path: &str, by: Option<&str>, links: &[&str]) -> Entry {
    Entry {
        superseded_by: by.map(str::to_owned),
        superseded_by_links: links.iter().map(|link| (*link).to_owned()).collect(),
        ..entry(path, "decision", Some(Lifecycle::Superseded))
    }
}

fn days_ago(days: i64) -> Touched {
    Touched::Committed {
        commit: id('a'),
        at: NOW - days * DAY - 3_600,
    }
}

fn rules() -> Rules {
    Rules::parse(
        "[[type]]\nname = \"decision\"\npaths = [\"docs/decisions/*.md\"]\n\n[[type]]\nname = \"feature\"\npaths = [\"docs/features/*.md\"]\n\n[[type]]\nname = \"decision\"\npaths = [\"adr/*.md\"]\n\n[[type]]\nname = \"empty\"\npaths = [\"none/*.md\"]\n",
    )
    .unwrap()
}

fn build(entries: Vec<Entry>) -> View {
    View::build(
        entries
            .into_iter()
            .map(|entry| (entry, days_ago(1)))
            .collect(),
        &rules(),
        NOW,
    )
}

fn target(view: &View, path: &str) -> Option<Target> {
    view.documents[view.find(path)?]
        .successor
        .as_ref()
        .map(|successor| successor.target.clone())
}

#[test]
fn an_entry_keeps_what_the_rules_say() {
    let class = Classification {
        doc_type: "decision".into(),
        status: Some("Superseded by B".into()),
        status_links: vec!["b.md".into()],
        lifecycle: Some(Lifecycle::Superseded),
        date: Some("2026-01-02".into()),
        superseded_by: Some("B".into()),
        superseded_by_links: vec!["c.md".into()],
        decision_blocks: vec![3],
    };
    let made = Entry::new("docs/a.md".into(), Some("A".into()), class);
    assert_eq!(
        made,
        Entry {
            path: "docs/a.md".into(),
            title: Some("A".into()),
            doc_type: "decision".into(),
            status: Some("Superseded by B".into()),
            status_links: vec!["b.md".into()],
            lifecycle: Some(Lifecycle::Superseded),
            date: Some("2026-01-02".into()),
            superseded_by: Some("B".into()),
            superseded_by_links: vec!["c.md".into()],
        }
    );
}

#[test]
fn days_count_whole_days_since_the_last_commit() {
    let view = View::build(
        vec![
            (entry("a.md", "document", None), days_ago(179)),
            (entry("b.md", "document", None), days_ago(180)),
            (entry("c.md", "document", None), Touched::Uncommitted),
            (
                entry("d.md", "document", None),
                Touched::Committed {
                    commit: id('b'),
                    at: NOW + DAY,
                },
            ),
            (
                entry("e.md", "document", None),
                Touched::Committed {
                    commit: id('c'),
                    at: NOW - DAY,
                },
            ),
        ],
        &Rules::default(),
        NOW,
    );
    let days: Vec<(Option<i64>, bool)> = view
        .documents
        .iter()
        .map(|doc| (doc.days, doc.untouched))
        .collect();
    assert_eq!(
        days,
        [
            (Some(179), false),
            (Some(180), true),
            (None, false),
            (Some(0), false),
            (Some(1), false)
        ]
    );
    assert_eq!(view.untouched_days, 180);
    assert_eq!(view.totals.untouched, 1);
    let short = Rules::parse("[status-view]\nuntouched-days = 1\n").unwrap();
    let view = View::build(
        vec![(entry("e.md", "document", None), days_ago(1))],
        &short,
        NOW,
    );
    assert!(view.documents[0].untouched);
}

#[test]
fn types_and_lifecycles_are_counted_in_the_rules_order() {
    let view = build(vec![
        entry("notes/x.md", "document", None),
        entry("docs/features/f.md", "feature", Some(Lifecycle::Current)),
        entry("docs/decisions/1.md", "decision", Some(Lifecycle::Current)),
        entry("docs/decisions/2.md", "decision", Some(Lifecycle::Archived)),
        entry("adr/3.md", "decision", None),
        entry("x/z.md", "zeta", Some(Lifecycle::Current)),
        entry("x/y.md", "omega", Some(Lifecycle::Superseded)),
    ]);
    let names: Vec<&str> = view.types.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["decision", "feature", "omega", "zeta", "document"]);
    assert_eq!(
        view.types[0].counts,
        Counts {
            documents: 3,
            current: 1,
            superseded: 0,
            archived: 1,
            none: 1,
            untouched: 0
        }
    );
    assert_eq!(
        view.totals,
        Counts {
            documents: 7,
            current: 3,
            superseded: 1,
            archived: 1,
            none: 2,
            untouched: 0
        }
    );
    assert_eq!(view.totals.of(Stage::Current), 3);
    assert_eq!(view.totals.of(Stage::Superseded), 1);
    assert_eq!(view.totals.of(Stage::Archived), 1);
    assert_eq!(view.totals.of(Stage::None), 2);
    let paths: Vec<&str> = view.documents.iter().map(|d| d.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "adr/3.md",
            "docs/decisions/1.md",
            "docs/decisions/2.md",
            "docs/features/f.md",
            "notes/x.md",
            "x/y.md",
            "x/z.md"
        ]
    );
    assert_eq!(view.find("notes/x.md"), Some(4));
    assert_eq!(view.find("notes/y.md"), None);
    assert_eq!(view.documents[1].folder(), "docs/decisions/");
    assert_eq!(view.documents[4].folder(), "notes/");
    let only = build(vec![entry("a.md", "document", None)]);
    assert_eq!(only.types.len(), 1);
    assert_eq!(only.documents[0].folder(), "");
}

#[test]
fn stages_have_names() {
    for stage in Stage::ALL {
        assert_eq!(Stage::parse(stage.name()), Some(stage));
        assert_eq!(
            Stage::parse(&format!(" {} ", stage.name().to_uppercase())),
            Some(stage)
        );
    }
    assert_eq!(Stage::parse("draft"), None);
    assert_eq!(Stage::of(None), Stage::None);
    assert_eq!(Stage::of(Some(Lifecycle::Archived)), Stage::Archived);
}

/// Decision records that name their successors every way there is.
fn named_successors() -> View {
    let mut titled = entry(
        "docs/decisions/0003-store-on-disk.md",
        "decision",
        Some(Lifecycle::Current),
    );
    titled.title = Some("3.  Store files ON disk".into());
    let mut simple = entry("docs/other/simple.md", "document", None);
    simple.title = Some("Keep it simple".into());
    let mut twin_a = entry("docs/other/a.md", "document", None);
    twin_a.title = Some("Twin".into());
    let mut twin_b = entry("docs/other/b.md", "document", None);
    twin_b.title = Some("twin".into());
    build(vec![
        titled,
        simple,
        twin_a,
        twin_b,
        entry(
            "docs/decisions/0009-new.md",
            "decision",
            Some(Lifecycle::Current),
        ),
        entry(
            "docs/decisions/ADR-0012-x.md",
            "decision",
            Some(Lifecycle::Current),
        ),
        entry(
            "docs/decisions/ADR-0012-y.md",
            "decision",
            Some(Lifecycle::Current),
        ),
        entry("docs/features/f.md", "feature", Some(Lifecycle::Current)),
        superseded(
            "docs/decisions/0001-link.md",
            Some("New"),
            &["https://x.org/a.md", "0009-new.md#top"],
        ),
        superseded("docs/decisions/0002-name.md", Some("0009-new.md"), &[]),
        superseded(
            "docs/decisions/0004-root.md",
            Some("/docs/features/f.md"),
            &[],
        ),
        superseded(
            "docs/decisions/0005-title.md",
            Some("3. Store files on disk"),
            &[],
        ),
        superseded("docs/decisions/0006-number.md", Some("ADR 9"), &[]),
        superseded("docs/decisions/0007-two.md", Some("ADR-12"), &[]),
        superseded("docs/decisions/0008-twins.md", Some("Twin"), &[]),
        superseded(
            "docs/decisions/0010-gone.md",
            Some("Gone"),
            &["0099-gone.md", "0098-gone.md"],
        ),
        superseded(
            "docs/decisions/0011-gone-name.md",
            Some("0097-gone.md"),
            &[],
        ),
        superseded("docs/decisions/0013-none.md", None, &[]),
        superseded("docs/decisions/0014-words.md", Some("the new plan"), &[]),
        superseded(
            "docs/decisions/0016-only-a-title.md",
            Some(" keep  IT simple "),
            &[],
        ),
        superseded(
            "docs/decisions/0015-site.md",
            Some("site"),
            &["https://example.com"],
        ),
    ])
}

#[test]
fn successors_are_found_by_link_name_title_or_number() {
    let view = named_successors();
    let found = |path: &str| view.find(path).map(Target::Found);
    let new = found("docs/decisions/0009-new.md");
    let cases = [
        ("0001-link.md", new.clone()),
        ("0002-name.md", new.clone()),
        ("0004-root.md", found("docs/features/f.md")),
        (
            "0005-title.md",
            found("docs/decisions/0003-store-on-disk.md"),
        ),
        ("0006-number.md", new),
        ("0007-two.md", Some(Target::Unmatched)),
        ("0008-twins.md", Some(Target::Unmatched)),
        (
            "0010-gone.md",
            Some(Target::Missing("docs/decisions/0099-gone.md".into())),
        ),
        (
            "0011-gone-name.md",
            Some(Target::Missing("docs/decisions/0097-gone.md".into())),
        ),
        ("0013-none.md", Some(Target::Unnamed)),
        ("0014-words.md", Some(Target::Unmatched)),
        ("0015-site.md", Some(Target::Unmatched)),
        ("0016-only-a-title.md", found("docs/other/simple.md")),
    ];
    for (file, wanted) in cases {
        let path = format!("docs/decisions/{file}");
        assert_eq!(target(&view, &path), wanted, "{path}");
    }
    assert_eq!(target(&view, "docs/decisions/0009-new.md"), None);
    let newest = &view.documents[view.find("docs/decisions/0009-new.md").unwrap()];
    let replaced: Vec<&str> = newest
        .replaces
        .iter()
        .map(|&at| view.documents[at].path.as_str())
        .collect();
    assert_eq!(
        replaced,
        [
            "docs/decisions/0001-link.md",
            "docs/decisions/0002-name.md",
            "docs/decisions/0006-number.md"
        ]
    );
}

#[test]
fn a_superseded_status_can_name_the_successor() {
    let mut by_status = entry(
        "docs/decisions/0001-a.md",
        "decision",
        Some(Lifecycle::Superseded),
    );
    by_status.status = Some("Superseded by 2".into());
    let mut by_link = entry(
        "docs/decisions/0003-c.md",
        "decision",
        Some(Lifecycle::Superseded),
    );
    by_link.status = Some("Superseded".into());
    by_link.status_links = vec!["0002-b.md".into()];
    let mut current = entry(
        "docs/decisions/0002-b.md",
        "decision",
        Some(Lifecycle::Current),
    );
    current.status = Some("Accepted by everyone".into());
    current.status_links = vec!["0001-a.md".into()];
    let mut no_status = entry(
        "docs/decisions/0004-d.md",
        "decision",
        Some(Lifecycle::Superseded),
    );
    no_status.status = None;
    let view = build(vec![by_status, by_link, current, no_status]);
    let b = view.find("docs/decisions/0002-b.md").unwrap();
    assert_eq!(
        target(&view, "docs/decisions/0001-a.md"),
        Some(Target::Found(b))
    );
    assert_eq!(
        view.documents[0]
            .successor
            .as_ref()
            .unwrap()
            .text
            .as_deref(),
        Some("2")
    );
    assert_eq!(
        target(&view, "docs/decisions/0003-c.md"),
        Some(Target::Found(b))
    );
    assert_eq!(view.documents[2].successor.as_ref().unwrap().text, None);
    assert_eq!(target(&view, "docs/decisions/0002-b.md"), None);
    assert_eq!(
        target(&view, "docs/decisions/0004-d.md"),
        Some(Target::Unnamed)
    );
    // A current document that names a successor gets a link, but does not
    // fold under it.
    let mut named = entry(
        "docs/decisions/0005-e.md",
        "decision",
        Some(Lifecycle::Current),
    );
    named.superseded_by = Some("0006-f.md".into());
    let view = build(vec![
        named,
        entry(
            "docs/decisions/0006-f.md",
            "decision",
            Some(Lifecycle::Current),
        ),
    ]);
    assert_eq!(
        target(&view, "docs/decisions/0005-e.md"),
        Some(Target::Found(1))
    );
    assert_eq!(view.documents[0].folds_under(), None);
    assert_eq!(view.documents[1].replaces, Vec::<usize>::new());
}

#[test]
fn circles_do_not_fold() {
    let view = build(vec![
        superseded("docs/decisions/a.md", Some("b.md"), &[]),
        superseded("docs/decisions/b.md", Some("c.md"), &[]),
        superseded("docs/decisions/c.md", Some("a.md"), &[]),
        superseded("docs/decisions/d.md", Some("a.md"), &[]),
        superseded("docs/decisions/e.md", Some("e.md"), &[]),
        superseded("docs/decisions/f.md", Some("g.md"), &[]),
        superseded("docs/decisions/g.md", Some("f.md"), &[]),
        superseded("docs/decisions/h.md", Some("f.md"), &[]),
    ]);
    assert_eq!(
        target(&view, "docs/decisions/a.md"),
        Some(Target::Circle(1))
    );
    assert_eq!(
        target(&view, "docs/decisions/b.md"),
        Some(Target::Circle(2))
    );
    assert_eq!(
        target(&view, "docs/decisions/c.md"),
        Some(Target::Circle(0))
    );
    assert_eq!(target(&view, "docs/decisions/d.md"), Some(Target::Found(0)));
    assert_eq!(
        target(&view, "docs/decisions/e.md"),
        Some(Target::Circle(4))
    );
    assert_eq!(
        target(&view, "docs/decisions/f.md"),
        Some(Target::Circle(6))
    );
    assert_eq!(
        target(&view, "docs/decisions/g.md"),
        Some(Target::Circle(5))
    );
    assert_eq!(target(&view, "docs/decisions/h.md"), Some(Target::Found(5)));
    assert_eq!(view.documents[0].replaces, [3]);
    assert_eq!(view.documents[5].replaces, [7]);
    let rows = &view.groups(&Filter::default())[0].rows;
    assert_eq!(rows, &[0, 1, 2, 4, 5, 6]);
}

/// A by B, B by C; C current; D archived; E superseded with no successor
/// found; N with no lifecycle, untouched.
fn chain() -> View {
    View::build(
        vec![
            (
                superseded("docs/decisions/a.md", Some("b.md"), &[]),
                days_ago(400),
            ),
            (
                superseded("docs/decisions/b.md", Some("c.md"), &[]),
                days_ago(200),
            ),
            (
                entry("docs/decisions/c.md", "decision", Some(Lifecycle::Current)),
                days_ago(10),
            ),
            (
                entry("docs/decisions/d.md", "decision", Some(Lifecycle::Archived)),
                days_ago(500),
            ),
            (
                superseded("docs/decisions/e.md", Some("gone.md"), &[]),
                days_ago(5),
            ),
            (entry("notes/n.md", "document", None), days_ago(300)),
            (
                entry("docs/features/f.md", "feature", Some(Lifecycle::Current)),
                Touched::Uncommitted,
            ),
        ],
        &rules(),
        NOW,
    )
}

fn rows(view: &View, filter: &Filter) -> Vec<(String, Vec<String>)> {
    view.groups(filter)
        .into_iter()
        .map(|group| {
            let paths = group
                .rows
                .iter()
                .map(|&at| view.documents[at].path.clone())
                .collect();
            (group.name, paths)
        })
        .collect()
}

#[test]
fn a_chain_folds_under_its_newest_matching_document() {
    let view = chain();
    let c = view.find("docs/decisions/c.md").unwrap();
    let b = view.find("docs/decisions/b.md").unwrap();
    let a = view.find("docs/decisions/a.md").unwrap();
    assert_eq!(view.documents[c].replaces, [b]);
    assert_eq!(view.documents[b].replaces, [a]);
    let owned = |items: &[(&str, &[&str])]| -> Vec<(String, Vec<String>)> {
        items
            .iter()
            .map(|(name, paths)| {
                (
                    (*name).to_owned(),
                    paths.iter().map(|path| (*path).to_owned()).collect(),
                )
            })
            .collect()
    };
    assert_eq!(
        rows(&view, &Filter::default()),
        owned(&[
            (
                "decision",
                &[
                    "docs/decisions/c.md",
                    "docs/decisions/d.md",
                    "docs/decisions/e.md"
                ]
            ),
            ("feature", &["docs/features/f.md"]),
            ("document", &["notes/n.md"]),
        ])
    );
    let superseded_only = Filter {
        stage: Some(Stage::Superseded),
        ..Filter::default()
    };
    assert_eq!(
        rows(&view, &superseded_only),
        owned(&[("decision", &["docs/decisions/b.md", "docs/decisions/e.md"])])
    );
    assert_eq!(
        view.listed(&superseded_only),
        [a, b, view.find("docs/decisions/e.md").unwrap()]
    );
    let untouched = Filter {
        untouched: true,
        ..Filter::default()
    };
    assert_eq!(
        rows(&view, &untouched),
        owned(&[
            ("decision", &["docs/decisions/b.md", "docs/decisions/d.md"]),
            ("document", &["notes/n.md"]),
        ])
    );
    let folder = Filter {
        folder: Some("docs/".into()),
        types: vec!["feature".into(), "document".into()],
        ..Filter::default()
    };
    assert_eq!(
        rows(&view, &folder),
        owned(&[("feature", &["docs/features/f.md"])])
    );
    let none = Filter {
        stage: Some(Stage::None),
        folder: Some("notes/n".into()),
        ..Filter::default()
    };
    assert_eq!(rows(&view, &none), owned(&[("document", &["notes/n.md"])]));
    assert_eq!(
        rows(
            &view,
            &Filter {
                types: vec!["nothing".into()],
                ..Filter::default()
            }
        ),
        Vec::new()
    );
    assert_eq!(view.documents[a].days, Some(400));
    assert!(view.documents[a].untouched);
    assert_eq!(
        view.documents[view.find("docs/features/f.md").unwrap()].days,
        None
    );
}
