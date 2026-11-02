//! Tests of the page writers' parts. `tests/serve.rs` checks whole pages
//! from a real server.

use std::sync::Arc;

use driftmark_core::diff::{BlockRef, ChangeKind, KindCounts, LevelCounts, Side, WholeDocument};
use driftmark_core::document::{BlockKind, FieldOrigin};
use driftmark_core::rules::Classification;

use super::*;

fn summary(decision: usize, fact: usize, formatting: usize) -> Summary {
    let levels = LevelCounts {
        decision,
        rule: 0,
        fact,
        wording: 0,
        formatting,
    };
    let highest = Level::HIGHEST_FIRST
        .into_iter()
        .find(|level| levels.get(*level) > 0);
    Summary {
        changes: decision + fact + formatting,
        highest,
        levels,
        kinds: KindCounts::default(),
    }
}

fn block(text: &str, path: &[&str]) -> BlockRef {
    BlockRef {
        kind: BlockKind::Paragraph,
        start: 0,
        end: text.len(),
        line: 3,
        heading_path: path
            .iter()
            .map(|title| (*title).to_owned())
            .collect::<Vec<_>>()
            .into(),
        text: text.to_owned(),
    }
}

fn edit(text: &str, path: &[&str]) -> Change {
    Change {
        kind: ChangeKind::Edited,
        level: Level::Fact,
        signals: Vec::new(),
        subject: Subject::Block {
            old: Side {
                at: 1,
                block: Some(block(text, path)),
            },
            new: Side {
                at: 1,
                block: Some(block(text, path)),
            },
        },
        edit: None,
    }
}

#[test]
fn times_read_as_dates_and_as_how_long_ago() {
    assert_eq!(utc(0), "1970-01-01 00:00 UTC");
    assert_eq!(utc(1_767_225_600), "2026-01-01 00:00 UTC");
    assert_eq!(utc(951_782_400), "2000-02-29 00:00 UTC");
    assert_eq!(utc(-86_400), "1969-12-31 00:00 UTC");
    assert_eq!(iso(1_760_779_845), "2025-10-18T09:30:45Z");
    let now = 1_767_225_600;
    assert_eq!(ago(now - 5, now), "just now");
    assert_eq!(ago(now + 5, now), "just now");
    assert_eq!(ago(now - 60, now), "1 minute ago");
    assert_eq!(ago(now - 3599, now), "59 minutes ago");
    assert_eq!(ago(now - 3600, now), "1 hour ago");
    assert_eq!(ago(now - 7300, now), "2 hours ago");
    assert_eq!(ago(now - 90_000, now), "yesterday");
    assert_eq!(ago(now - 3 * 86_400, now), "3 days ago");
    assert_eq!(ago(now - 40 * 86_400, now), "2025-11-22");
    assert!(time(now - 60, now).contains("datetime=\"2025-12-31T23:59:00Z\""));
    assert!(super::now() > 1_700_000_000);
}

#[test]
fn escaping_covers_markup_and_quotes() {
    assert_eq!(
        escape("<a href=\"x\" title='y'>&</a>"),
        "&lt;a href=&quot;x&quot; title=&#39;y&#39;&gt;&amp;&lt;/a&gt;"
    );
    assert_eq!(short("0123456789abcdef"), "0123456789ab");
    assert_eq!(short("worktree"), "worktree");
}

#[test]
fn counts_and_bars_follow_the_levels() {
    let counts = summary(7, 18, 6);
    assert_eq!(level_counts(&counts), "7 decision · 18 fact · 6 formatting");
    assert_eq!(
        document_counts(&counts),
        "31 changes: 7 decision, 18 fact, 6 formatting"
    );
    assert_eq!(document_counts(&summary(0, 0, 0)), "0 changes");
    let drawn = bar(&counts);
    assert!(
        drawn.starts_with("<svg class=\"bar\" viewBox=\"0 0 31 1\""),
        "{drawn}"
    );
    assert!(
        drawn.contains("data-level=\"fact\" x=\"7\" width=\"18\""),
        "{drawn}"
    );
    assert!(
        drawn.contains("data-level=\"formatting\" x=\"25\" width=\"6\""),
        "{drawn}"
    );
    assert_eq!(bar(&summary(0, 0, 0)), "");
    assert_eq!(
        badge(Level::Rule),
        "<span class=\"lvl\" data-level=\"rule\">Rule</span>"
    );
    assert_eq!(plural(1, "change"), "1 change");
    assert_eq!(plural(2, "change"), "2 changes");
    assert_eq!(plural(3, "open branch"), "3 open branches");
}

#[test]
fn group_titles_say_what_to_do() {
    assert_eq!(
        group_title(Some(Level::Decision)),
        "Decisions changed: review these first"
    );
    assert_eq!(group_title(Some(Level::Rule)), "Rules changed");
    assert_eq!(group_title(Some(Level::Fact)), "Facts changed");
    assert_eq!(group_title(Some(Level::Wording)), "Only wording changed");
    assert_eq!(
        group_title(Some(Level::Formatting)),
        "Only formatting changed"
    );
    assert_eq!(group_title(None), "Nothing to rank");
}

#[test]
fn ids_on_change_pages_hold_a_capital_letter() {
    let section = section_id("docs/a.md");
    let prefix = change_prefix("docs/a.md");
    assert!(section.starts_with('D') && section.len() == 9, "{section}");
    assert!(prefix.starts_with('C') && prefix.len() == 9, "{prefix}");
    assert_eq!(section[1..], prefix[1..]);
    assert_ne!(section_id("docs/a.md"), section_id("docs/b.md"));
}

#[test]
fn pills_come_from_the_rules_or_the_front_matter() {
    let mut class = Classification {
        doc_type: "decision".into(),
        status: Some("accepted".into()),
        status_links: Vec::new(),
        lifecycle: Some(Lifecycle::Current),
        date: None,
        superseded_by: Some("0009 <x>".into()),
        superseded_by_links: Vec::new(),
        decision_blocks: Vec::new(),
    };
    let date = Field {
        key: "Date".into(),
        value: MetaValue::Text("2026-10-18".into()),
        origin: FieldOrigin::FrontMatter,
        links: Vec::new(),
    };
    let shown = pills(Some(&class), std::slice::from_ref(&date));
    assert_eq!(
        shown,
        "<ul class=\"pills\" data-gen aria-label=\"About this document\"><li class=\"pill\" data-pill=\"type\">Decision</li><li class=\"pill\" data-pill=\"status\">Status: accepted</li><li class=\"pill\" data-pill=\"lifecycle\" data-lifecycle=\"current\">Current</li><li class=\"pill\" data-pill=\"date\">Dated 2026-10-18</li><li class=\"pill\" data-pill=\"superseded-by\">Superseded by 0009 &lt;x&gt;</li></ul>\n"
    );
    class.doc_type = "document".into();
    class.status = None;
    class.lifecycle = Some(Lifecycle::Archived);
    class.superseded_by = None;
    let status = Field {
        key: "status".into(),
        value: MetaValue::Text("target".into()),
        origin: FieldOrigin::FrontMatter,
        links: Vec::new(),
    };
    let blank = Field {
        key: "date".into(),
        value: MetaValue::Text("  ".into()),
        origin: FieldOrigin::FrontMatter,
        links: Vec::new(),
    };
    let shown = pills(Some(&class), &[blank, status]);
    assert!(shown.contains("Status: target"), "{shown}");
    assert!(
        shown.contains("data-lifecycle=\"archived\">Archived"),
        "{shown}"
    );
    assert!(!shown.contains("Dated"), "{shown}");
    class.lifecycle = Some(Lifecycle::Superseded);
    assert!(pills(Some(&class), &[]).contains(">Superseded<"));
    class.lifecycle = None;
    assert_eq!(pills(Some(&class), &[]), "");
    assert_eq!(pills(None, &[]), "");
    assert_eq!(capitalized(""), "");
}

#[test]
fn reading_pages_link_their_pills_to_the_status_page() {
    let class = Classification {
        doc_type: "decision record".into(),
        status: Some("accepted".into()),
        status_links: Vec::new(),
        lifecycle: Some(Lifecycle::Current),
        date: Some("2026-10-18".into()),
        superseded_by: None,
        superseded_by_links: Vec::new(),
        decision_blocks: Vec::new(),
    };
    assert_eq!(
        linked_pills(Some(&class), &[], "feature/x"),
        "<ul class=\"pills\" data-gen aria-label=\"About this document\"><li class=\"pill\" data-pill=\"type\"><a href=\"/lifecycle/feature/x?type=decision%20record\">Decision record</a></li><li class=\"pill\" data-pill=\"status\"><a href=\"/lifecycle/feature/x?type=decision%20record&amp;lifecycle=current\">Status: accepted</a></li><li class=\"pill\" data-pill=\"lifecycle\" data-lifecycle=\"current\"><a href=\"/lifecycle/feature/x?type=decision%20record&amp;lifecycle=current\">Current</a></li><li class=\"pill\" data-pill=\"date\">Dated 2026-10-18</li></ul>\n"
    );
    let plain = Classification {
        doc_type: "document".into(),
        status: None,
        lifecycle: None,
        date: None,
        ..class
    };
    assert_eq!(linked_pills(Some(&plain), &[], "main"), "");
    assert_eq!(linked_pills(None, &[], "main"), "");
    assert_eq!(
        link_pill("<li data-pill=\"type\">", "type", "x"),
        "<li data-pill=\"type\">"
    );
    assert_eq!(
        link_pill("<li data-pill=\"type\"", "type", "x"),
        "<li data-pill=\"type\""
    );
}

#[test]
fn pills_go_right_after_the_title() {
    let title = Section {
        level: 1,
        id: "rules".into(),
        text: "Rules".into(),
        block: 0,
    };
    let body = "<h1 id=\"rules\" data-k=\"0\">Rules</h1>\n<p data-k=\"1\">x</p>\n";
    assert_eq!(
        after_title(body, Some(&title), "<ul></ul>\n"),
        "<h1 id=\"rules\" data-k=\"0\">Rules</h1>\n<ul></ul>\n<p data-k=\"1\">x</p>\n"
    );
    assert_eq!(
        after_title(body, None, "<ul></ul>"),
        format!("<ul></ul>{body}")
    );
    let other = Section {
        id: "missing".into(),
        ..title
    };
    assert!(after_title(body, Some(&other), "P").starts_with('P'));
}

#[test]
fn sections_with_changes_waiting_are_flagged() {
    let outline = vec![
        Section {
            level: 1,
            id: "t".into(),
            text: "T".into(),
            block: 0,
        },
        Section {
            level: 2,
            id: "cache".into(),
            text: "Cache".into(),
            block: 1,
        },
    ];
    let mut changes = DocumentChanges {
        path: "docs/a.md".into(),
        file: FileState::Modified,
        status: None,
        summary: summary(0, 2, 0),
        changes: vec![
            edit("one", &["T", "Cache", "Gone"]),
            edit("two", &["T", "Cache"]),
            edit("three", &["Elsewhere"]),
        ],
        unchanged: Vec::new(),
    };
    changes.changes.push(Change {
        kind: ChangeKind::Added,
        level: Level::Fact,
        signals: Vec::new(),
        subject: Subject::Document {
            document: WholeDocument {
                title: None,
                summary: None,
                bytes: 1,
                lines: 1,
                source: String::new(),
            },
        },
        edit: None,
    });
    let waiting = vec![Waiting {
        name: "feature".into(),
        range: "main...feature".into(),
        at: 0,
        changes,
    }];
    let counts = pending(&outline, &waiting);
    assert_eq!(counts.get("cache"), Some(&2));
    assert_eq!(counts.len(), 1);
    let body = "<h2 id=\"cache\" data-k=\"1\">Cache<a class=\"anchor\" href=\"#cache\" aria-label=\"Link to this section\"></a></h2>\n";
    let flagged = flag_sections(body, &counts, "/changes/x");
    assert!(
        flagged.contains("Cache<a class=\"pending-flag\" href=\"/changes/x\" data-gen>2 changes wait</a><a class=\"anchor\""),
        "{flagged}"
    );
    let one: HashMap<String, usize> = [("cache".to_owned(), 1)].into();
    assert!(flag_sections(body, &one, "/x").contains(">1 change waits<"));
    let mut toc_html = String::new();
    toc(&mut toc_html, &outline, &counts);
    assert_eq!(toc_html, "", "one section gets no list");
    let mut longer = outline.clone();
    longer.push(Section {
        level: 3,
        id: "size".into(),
        text: "Size".into(),
        block: 2,
    });
    toc(&mut toc_html, &longer, &counts);
    assert!(
        toc_html
            .contains("<li data-level=\"2\"><a href=\"#cache\">Cache</a> <span class=\"pending\""),
        "{toc_html}"
    );
    assert!(
        toc_html.contains("2<span class=\"vh\"> changes waiting</span>"),
        "{toc_html}"
    );
    assert!(
        toc_html.contains("<li data-level=\"3\"><a href=\"#size\">Size</a></li>"),
        "{toc_html}"
    );
    let mut quiet = String::new();
    toc(&mut quiet, &longer, &one);
    assert!(
        quiet.contains("1<span class=\"vh\"> change waiting</span>"),
        "{quiet}"
    );
}

#[test]
fn rail_labels_name_the_kind_and_the_first_words() {
    let long = edit(&"word ".repeat(30), &[]);
    let label = rail_label(&long);
    assert!(label.starts_with("Edited: word word"), "{label}");
    assert!(label.ends_with('…'), "{label}");
    let short = edit("Short  text\nhere", &[]);
    assert_eq!(rail_label(&short), "Edited: Short text here");
    let whole = Change {
        kind: ChangeKind::Removed,
        level: Level::Fact,
        signals: Vec::new(),
        subject: Subject::Document {
            document: WholeDocument {
                title: Some("Old plan".into()),
                summary: None,
                bytes: 9,
                lines: 4,
                source: String::new(),
            },
        },
        edit: None,
    };
    assert_eq!(rail_label(&whole), "Deleted document: Old plan");
    let untitled = Change {
        subject: Subject::Document {
            document: WholeDocument {
                title: None,
                summary: None,
                bytes: 9,
                lines: 4,
                source: String::new(),
            },
        },
        ..whole
    };
    assert_eq!(rail_label(&untitled), "Deleted document: 4 lines");
    let _ = Arc::new(());
}

#[test]
fn problem_and_token_pages_say_what_to_do() {
    let missing = problem(true, "`x` <y>");
    assert!(missing.contains("<h1>Not found</h1>"));
    assert!(missing.contains("`x` &lt;y&gt;"));
    assert!(problem(false, "z").contains("This page could not be made"));
    assert!(token_needed(true).contains("That token is not this server's token."));
    assert!(token_needed(false).contains("needs its token"));
}

#[test]
fn a_bare_page_loads_nothing() {
    let page = bare("Token <needed>", "<main>x</main>\n");
    assert!(page.contains("<title>Token &lt;needed&gt; · driftmark</title>"));
    assert!(page.contains("<meta name=\"color-scheme\" content=\"light\">"));
    assert!(
        !page.contains("<link") && !page.contains("<script"),
        "{page}"
    );
    assert!(page.contains("<body>\n<main>x</main>\n</body>"));
}
