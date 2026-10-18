use super::*;
use crate::rules::Rules;

fn parse(source: &str) -> Document {
    Document::parse(source.as_bytes().to_vec()).expect("parses")
}

fn classify(document: &Document) -> Classification {
    Rules::default().classify("doc.md", document)
}

fn diff(old: &str, new: &str) -> DocumentChanges {
    let (old, new) = (parse(old), parse(new));
    let (old_class, new_class) = (classify(&old), classify(&new));
    compare(
        "doc.md",
        Some(Version {
            document: &old,
            classification: &old_class,
        }),
        Some(Version {
            document: &new,
            classification: &new_class,
        }),
    )
    .expect("both sides")
}

fn mark(op: MarkOp, text: &str) -> Mark {
    super::mark(op, text)
}

fn block(source: &str) -> Block {
    parse(source).blocks.remove(0)
}

#[test]
fn documents_sort_by_level_then_count_then_path() {
    let document = |path: &str, highest: Option<Level>, changes: usize| DocumentChanges {
        path: path.to_owned(),
        file: FileState::Modified,
        status: None,
        summary: Summary {
            changes,
            highest,
            levels: LevelCounts {
                wording: changes,
                ..LevelCounts::default()
            },
            kinds: KindCounts {
                edited: changes,
                ..KindCounts::default()
            },
        },
        changes: Vec::new(),
        unchanged: Vec::new(),
    };
    let list = ChangeList::new(
        None,
        vec![
            document("e.md", None, 0),
            document("d.md", Some(Level::Wording), 1),
            document("c.md", Some(Level::Wording), 5),
            document("b.md", Some(Level::Decision), 1),
            document("a.md", Some(Level::Wording), 1),
        ],
    );
    let order: Vec<&str> = list.documents.iter().map(|d| d.path.as_str()).collect();
    assert_eq!(order, ["b.md", "c.md", "a.md", "d.md", "e.md"]);
    assert_eq!(list.totals.changes, 8);
    assert_eq!(list.totals.highest, Some(Level::Decision));
    assert_eq!(list.totals.levels.get(Level::Wording), 8);
    assert_eq!(list.totals.kinds.get(ChangeKind::Edited), 8);
}

#[test]
fn counts_are_kept_per_level_and_kind() {
    let mut levels = LevelCounts::default();
    let mut kinds = KindCounts::default();
    for (index, level) in Level::HIGHEST_FIRST.into_iter().enumerate() {
        *levels.slot(level) += index + 1;
    }
    for (index, kind) in ChangeKind::ALL.into_iter().enumerate() {
        *kinds.slot(kind) += index + 1;
    }
    let found: Vec<usize> = Level::HIGHEST_FIRST
        .iter()
        .map(|&l| levels.get(l))
        .collect();
    assert_eq!(found, [1, 2, 3, 4, 5]);
    let found: Vec<usize> = ChangeKind::ALL.iter().map(|&k| kinds.get(k)).collect();
    assert_eq!(found, [1, 2, 3, 4, 5, 6]);
    let mut total = Summary::default();
    let part = Summary {
        changes: 2,
        highest: Some(Level::Fact),
        levels,
        kinds,
    };
    total.merge(&part);
    total.merge(&part);
    assert_eq!(total.changes, 4);
    assert_eq!(total.highest, Some(Level::Fact));
    assert_eq!(total.levels.get(Level::Formatting), 10);
    assert_eq!(total.kinds.get(ChangeKind::MovedEdited), 12);
}

#[test]
fn views_follow_the_kept_share() {
    let short = block("One sentence only.\n");
    let prose = block("One. Two. Three. Four.\n");
    let three = block("One. Two. Three.\n");
    assert_eq!(choose_view(INLINE_MIN_KEPT, &short, &short), View::Inline);
    assert_eq!(choose_view(0.849, &prose, &prose), View::Sentences);
    assert_eq!(
        choose_view(REWRITTEN_BELOW, &prose, &prose),
        View::Sentences
    );
    assert_eq!(choose_view(0.349, &prose, &prose), View::Rewritten);
    assert_eq!(choose_view(0.5, &short, &short), View::SideBySide);
    assert_eq!(choose_view(0.5, &prose, &three), View::SideBySide);
    assert_eq!(choose_view(0.5, &three, &prose), View::SideBySide);
    assert!(is_prose(&prose));
    assert!(!is_prose(&three));
    assert!(is_prose(&block("- One. Two. Three. Four.\n")));
    assert!(is_prose(&block("> [!NOTE]\n> One. Two. Three. Four.\n")));
    let footnote = parse("A[^n]\n\n[^n]: One. Two. Three. Four.\n")
        .blocks
        .remove(1);
    assert!(is_prose(&footnote));
    assert!(!is_prose(&block("# One. Two. Three. Four.\n")));
}

#[test]
fn marks_that_fail_the_guard_fall_back_to_side_by_side() {
    let marks = vec![mark(MarkOp::Kept, "a "), mark(MarkOp::Inserted, "c")];
    let good = checked(
        View::Inline,
        0.9,
        marks.clone(),
        Vec::new(),
        Vec::new(),
        "a c",
    );
    assert_eq!(good.view, View::Inline);
    assert_eq!(good.marks, marks);
    assert!(!good.fallback);
    let bad = checked(View::Inline, 0.9, marks.clone(), Vec::new(), vec![1], "a b");
    assert_eq!(bad.view, View::SideBySide);
    assert!(bad.marks.is_empty() && bad.fallback);
    assert_eq!(bad.cells, [1]);
    assert!((bad.kept_in_order - 0.9).abs() < 1e-12);
    let rows = vec![marks.clone()];
    let sentences = checked(
        View::Sentences,
        0.5,
        marks.clone(),
        rows.clone(),
        Vec::new(),
        "a c",
    );
    assert_eq!(sentences.sentences, rows);
    assert_eq!(sentences.marks, Vec::<Mark>::new());
    let broken = checked(View::Sentences, 0.5, marks.clone(), rows, Vec::new(), "x");
    assert_eq!(broken.view, View::SideBySide);
    assert!(broken.sentences.is_empty() && broken.fallback);
    let side = checked(
        View::SideBySide,
        0.5,
        marks.clone(),
        Vec::new(),
        Vec::new(),
        "a c",
    );
    assert_eq!(side.marks, marks);
    let rewritten = checked(View::Rewritten, 0.1, marks, Vec::new(), Vec::new(), "x");
    assert_eq!(rewritten.view, View::Rewritten);
    assert!(rewritten.marks.is_empty() && !rewritten.fallback);
}

#[test]
fn shares_round_to_three_decimals() {
    assert!((rounded(2.0 / 3.0) - 0.667).abs() < 1e-12);
    assert!((rounded(0.12345) - 0.123).abs() < 1e-12);
    assert!((rounded(1.0) - 1.0).abs() < 1e-12);
}

#[test]
fn changed_cells_compare_position_by_position() {
    let row = |cells: &[&str]| BlockKind::TableRow {
        header: false,
        cells: cells.iter().map(|&cell| cell.to_owned()).collect(),
    };
    assert_eq!(changed_cells(&row(&["a", "b"]), &row(&["a", "c"])), [1]);
    assert_eq!(changed_cells(&row(&["a"]), &row(&["a", "c", "d"])), [1, 2]);
    assert_eq!(changed_cells(&row(&["x", "b"]), &row(&["y"])), [0, 1]);
    assert_eq!(
        changed_cells(&BlockKind::Paragraph, &row(&["a"])),
        Vec::<usize>::new()
    );
}

#[test]
fn status_changes_ignore_case_and_spaces() {
    let classification = |status: Option<&str>| Classification {
        status: status.map(str::to_owned),
        ..Rules::default().classify("doc.md", &parse("x\n"))
    };
    assert_eq!(
        status_change(
            &classification(Some(" Draft ")),
            &classification(Some("draft"))
        ),
        None
    );
    assert_eq!(
        status_change(&classification(None), &classification(None)),
        None
    );
    assert_eq!(
        status_change(&classification(None), &classification(Some("Done"))),
        Some(StatusChange {
            old: None,
            new: Some("Done".to_owned()),
        })
    );
}

#[test]
fn the_status_signal_goes_to_the_block_that_holds_the_status() {
    let mut changes = diff(
        "# T\n\nThe intro.\n\n> Status: Draft\n",
        "# T\n\nThe new intro.\n\n> Status: Done\n",
    )
    .changes;
    assert_eq!(changes.len(), 2);
    let status = StatusChange {
        old: Some("draft".to_owned()),
        new: Some("done".to_owned()),
    };
    mark_status(&mut changes, &status);
    assert_eq!(changes[0].signals, [Signal::Wording]);
    assert_eq!(changes[1].signals, [Signal::Status]);
    assert_eq!(changes[1].level, Level::Decision);
    // Found by the new side alone.
    let mut changes = diff("# T\n\nA\n", "# T\n\nStatus: Done\n").changes;
    let status = StatusChange {
        old: None,
        new: Some("Done".to_owned()),
    };
    mark_status(&mut changes, &status);
    assert_eq!(changes[0].signals, [Signal::Status]);
    // Found by the old side alone, among other signals.
    let mut changes = diff("# T\n\nMust be Draft 2\n", "# T\n\nMust\n").changes;
    let status = StatusChange {
        old: Some("Draft".to_owned()),
        new: None,
    };
    mark_status(&mut changes, &status);
    assert_eq!(changes[0].signals, [Signal::Status, Signal::Number]);
    // No block holds it: nothing changes.
    let mut changes = diff("# T\n\nA\n", "# T\n\nB\n").changes;
    let before = changes.clone();
    mark_status(&mut changes, &status);
    assert_eq!(changes, before);
}

#[test]
fn a_move_inside_a_decision_section_is_a_decision() {
    let rules = Rules::parse(
        "[[type]]\nname = \"d\"\npaths = [\"*.md\"]\ndecision-headings = [\"Decision\"]\n",
    )
    .expect("rules");
    let old = parse("# T\n\n## Decision\n\nFirst point here.\n\nSecond point here.\n");
    let new = parse("# T\n\n## Decision\n\nSecond point here.\n\nFirst point here.\n");
    let (old_class, new_class) = (
        rules.classify("doc.md", &old),
        rules.classify("doc.md", &new),
    );
    let changes = compare(
        "doc.md",
        Some(Version {
            document: &old,
            classification: &old_class,
        }),
        Some(Version {
            document: &new,
            classification: &new_class,
        }),
    )
    .expect("both sides");
    assert_eq!(changes.changes.len(), 1);
    assert_eq!(
        changes.changes[0].signals,
        [Signal::DecisionSection, Signal::Moved]
    );
    assert_eq!(changes.changes[0].level, Level::Decision);
}

#[test]
fn decision_blocks_outside_the_document_are_ignored() {
    let document = parse("# T\n\nText.\n");
    let classification = Classification {
        decision_blocks: vec![1, 7],
        ..classify(&document)
    };
    let sided = Sided::new(Version {
        document: &document,
        classification: &classification,
    });
    assert_eq!(sided.decision, [false, true]);
}

#[test]
fn removed_blocks_sit_before_added_ones_in_a_gap() {
    let changes = diff(
        "# T\n\nGone away.\n\nEnd.\n",
        "# T\n\nArrived here.\n\nEnd.\n",
    )
    .changes;
    let kinds: Vec<ChangeKind> = changes.iter().map(|change| change.kind).collect();
    assert_eq!(kinds, [ChangeKind::Edited]);
    let changes = diff(
        "# T\n\nGone away.\n\nAlso gone.\n\nEnd.\n",
        "# T\n\nArrived here.\n\nEnd.\n",
    )
    .changes;
    let found: Vec<(ChangeKind, usize, usize)> = changes
        .iter()
        .filter_map(|change| {
            let (old, new) = change.sides()?;
            Some((change.kind, old.at, new.at))
        })
        .collect();
    assert_eq!(
        found,
        [
            (ChangeKind::Removed, 1, 1),
            (ChangeKind::Removed, 2, 1),
            (ChangeKind::Added, 3, 1)
        ]
    );
}

#[test]
fn sentence_rows_pair_the_sentences_between_kept_ones() {
    let old = block("Keep one. Old two. Old three. Keep four.\n");
    let new = block("Keep one. New two. Keep four. Extra five.\n");
    let rows: Vec<String> = sentence_rows(&old, &new)
        .iter()
        .map(|row| {
            row.iter()
                .map(|mark| match mark.op {
                    MarkOp::Kept => mark.text.clone(),
                    MarkOp::Deleted => format!("[-{}-]", mark.text),
                    MarkOp::Inserted => format!("{{+{}+}}", mark.text),
                })
                .collect()
        })
        .collect();
    assert_eq!(
        rows,
        [
            "Keep one. ",
            "[-Old-]{+New+} two. ",
            "[-Old three. -]",
            "Keep four. ",
            "{+Extra five.+}"
        ]
    );
}

#[test]
fn a_block_that_leaves_a_decision_section_keeps_the_signal() {
    let rules = Rules::parse(
        "[[type]]\nname = \"d\"\npaths = [\"*.md\"]\ndecision-headings = [\"Decision\"]\n",
    )
    .expect("rules");
    let old = parse("# T\n\n## Decision\n\nKeep pages for a day.\n");
    let new = parse("# T\n\n## Notes\n\nKeep pages for a week.\n");
    let (old_class, new_class) = (
        rules.classify("doc.md", &old),
        rules.classify("doc.md", &new),
    );
    let changes = compare(
        "doc.md",
        Some(Version {
            document: &old,
            classification: &old_class,
        }),
        Some(Version {
            document: &new,
            classification: &new_class,
        }),
    )
    .expect("both sides");
    let edited = |changes: &DocumentChanges| {
        changes
            .changes
            .iter()
            .find(|change| change.kind == ChangeKind::Edited)
            .expect("the paragraph is edited")
            .signals
            .clone()
    };
    assert_eq!(edited(&changes), [Signal::DecisionSection, Signal::Wording]);
    // And the other way round: a block that enters one.
    let changes = compare(
        "doc.md",
        Some(Version {
            document: &new,
            classification: &new_class,
        }),
        Some(Version {
            document: &old,
            classification: &old_class,
        }),
    )
    .expect("both sides");
    assert_eq!(edited(&changes), [Signal::DecisionSection, Signal::Wording]);
}

#[test]
fn a_new_or_deleted_document_is_one_change() {
    let rules = Rules::parse(
        "[[type]]\nname = \"d\"\npaths = [\"*.md\"]\ndecision-headings = [\"Decision\"]\n",
    )
    .expect("rules");
    let source =
        "# Pages\n\n## Decision\n\nKeep pages for an hour.\n\nOnly admins may clear them.\n";
    let new = parse(source);
    let class = rules.classify("doc.md", &new);
    let added = compare(
        "doc.md",
        None,
        Some(Version {
            document: &new,
            classification: &class,
        }),
    )
    .expect("one side");
    assert_eq!(added.file, FileState::Added);
    assert_eq!(added.changes.len(), 1);
    assert_eq!(added.summary.kinds.get(ChangeKind::Added), 1);
    let change = &added.changes[0];
    assert_eq!(change.kind, ChangeKind::Added);
    assert_eq!(
        change.signals,
        [Signal::DecisionSection, Signal::Heading],
        "`only` in a new block is no rule change"
    );
    assert_eq!(change.level, Level::Decision);
    assert_eq!(change.sides(), None);
    assert_eq!(change.edit, None);
    let whole = |changes: &DocumentChanges| match &changes.changes[0].subject {
        Subject::Document { document } => document.clone(),
        Subject::Block { .. } => panic!("a block change"),
    };
    assert_eq!(
        whole(&added),
        WholeDocument {
            title: Some("Pages".to_owned()),
            summary: None,
            bytes: source.len(),
            lines: 7,
            source: source.to_owned(),
        }
    );
    let empty = parse("");
    let class = classify(&empty);
    let removed = compare(
        "doc.md",
        Some(Version {
            document: &empty,
            classification: &class,
        }),
        None,
    )
    .expect("one side");
    assert_eq!(removed.file, FileState::Removed);
    assert_eq!(removed.changes[0].kind, ChangeKind::Removed);
    assert_eq!(removed.changes[0].signals, [Signal::Wording]);
    assert_eq!((whole(&removed).bytes, whole(&removed).lines), (0, 0));
    let lines_of = |source: &str| {
        let document = parse(source);
        let class = classify(&document);
        let added = compare(
            "doc.md",
            None,
            Some(Version {
                document: &document,
                classification: &class,
            }),
        )
        .expect("one side");
        whole(&added).lines
    };
    assert_eq!(lines_of("x"), 1);
    assert_eq!(lines_of("x\n"), 1);
    assert_eq!(lines_of("x\ny"), 2);
    assert_eq!(lines_of("x\r\ny\r\n"), 2);
}

#[test]
fn a_ticked_checkbox_is_a_fact_whatever_the_words() {
    let changes = diff(
        "# T\n\n- [ ] Tag the commit.\n- [x] Ship it.\n",
        "# T\n\n- [x] Tag the new commit.\n- [x] Ship it now.\n",
    )
    .changes;
    let signals: Vec<&[Signal]> = changes
        .iter()
        .map(|change| change.signals.as_slice())
        .collect();
    assert_eq!(signals, [&[Signal::TaskState][..], &[Signal::Wording][..]]);
    assert_eq!(changes[0].level, Level::Fact);
}

#[test]
fn the_block_map_pairs_blocks_as_the_change_list_does() {
    let old = parse("# T\n\nFirst one.\n\nSecond one.\n\nGone for good.\n");
    let new = parse("# T\n\nSecond one.\n\nFirst one, edited.\n\nBrand new.\n");
    let map = block_map(&old, &new);
    assert_eq!(map.old_to_new, vec![Some(0), Some(2), Some(1), None]);
    assert_eq!(map.in_place, vec![true, false, true, false]);
}
