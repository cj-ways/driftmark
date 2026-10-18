use super::*;

fn parse(source: &str) -> Document {
    Document::parse(source.as_bytes().to_vec()).expect("parses")
}

/// The anchor of the `nth` occurrence of `quote` in `document`.
fn anchor_on(document: &Document, quote: &str, nth: usize) -> Anchor {
    let start = document
        .source
        .match_indices(quote)
        .nth(nth)
        .map(|(at, _)| at)
        .expect("the quote is in the document");
    Anchor::capture(document, start..start + quote.len()).expect("captures")
}

/// Where a note on `quote` in `old` lands in `new`: its state, the text it
/// sits on, and its score.
fn moved_to(old: &str, quote: &str, new: &str) -> (AnchorState, Option<String>, f64) {
    let (old, new) = (parse(old), parse(new));
    let anchor = anchor_on(&old, quote, 0);
    let placement = reanchor(&anchor, Origin::Known(&old), &new);
    // A pairing made once gives the same answer.
    let map = crate::diff::block_map(&old, &new);
    assert_eq!(
        reanchor(&anchor, Origin::Paired(&old, &map), &new),
        placement
    );
    let text = placement
        .range
        .clone()
        .map(|range| new.source[range].to_owned());
    if let Some(range) = &placement.range {
        assert_eq!(placement.block, block_of(&new, range.start));
    }
    (placement.state, text, placement.score)
}

#[test]
fn a_capture_keeps_the_quote_its_context_and_its_place() {
    let document =
        parse("# Cache\n\n## Decision\n\nKeep each page for **one hour**, then drop it.\n");
    let anchor = anchor_on(&document, "one hour", 0);
    assert_eq!(anchor.quote.exact, "one hour");
    assert_eq!(anchor.quote.prefix, " Decision\n\nKeep each page for **");
    assert_eq!(anchor.quote.prefix.chars().count(), CONTEXT);
    assert_eq!(anchor.quote.suffix, "**, then drop it.\n");
    assert_eq!(anchor.heading_path, ["Cache", "Decision"]);
    assert_eq!(anchor.block, 2);
    assert_eq!(
        anchor.block_hash,
        format!("{:016x}", document.blocks[2].source_hash)
    );
    let start = anchor.start;
    assert_eq!(&document.source[start..anchor.end], "one hour");
}

#[test]
fn a_capture_refuses_what_cannot_be_a_quote() {
    let document = parse("# Tëst\n\nSome text.\n\n");
    let len = document.source.len();
    let (three, two) = (3, 2);
    for range in [0..0, three..two, 0..len + 1, 3..4, 7..9, len - 1..len] {
        assert_eq!(Anchor::capture(&document, range.clone()), None, "{range:?}");
    }
    assert!(Anchor::capture(&document, 0..len).is_some());
}

#[test]
fn the_same_bytes_keep_the_note_where_it_was() {
    let document = parse("# A\n\nOne two three.\n");
    let anchor = anchor_on(&document, "two", 0);
    let placement = reanchor(&anchor, Origin::Same, &document);
    assert_eq!(
        placement,
        Placement {
            state: AnchorState::Exact,
            range: Some(anchor.start..anchor.end),
            score: 1.0,
            block: Some(1),
        }
    );
    // Bytes that claim to be the same but are not are searched instead.
    let other = parse("# A\n\nZero. One two three.\n");
    let placement = reanchor(&anchor, Origin::Same, &other);
    assert_eq!(placement.state, AnchorState::Exact);
    assert_eq!(placement.range, Some(15..18));
    // A short quote whose context changed too is not trusted.
    let shuffled = parse("# A\n\nOne three two.\n");
    let placement = reanchor(&anchor, Origin::Same, &shuffled);
    assert_eq!(placement.state, AnchorState::Orphaned);
}

const LONG: &str = "The server keeps every rendered page for one hour before it drops it.";

#[test]
fn an_unchanged_sentence_stays_exact_when_its_neighbours_change() {
    let old = format!("# A\n\n## B\n\nFirst words here. {LONG} Last words here.\n");
    let new = format!("# A\n\n## B\n\nOther opening, quite new. {LONG} A new ending.\n\nMore.\n");
    let (state, text, score) = moved_to(&old, LONG, &new);
    assert_eq!(
        (state, text.as_deref(), score),
        (AnchorState::Exact, Some(LONG), 1.0)
    );
}

#[test]
fn text_before_the_note_in_its_block_moves_the_offset_but_not_the_state() {
    let old = "# A\n\nAlpha beta gamma delta epsilon zeta eta theta.\n";
    let new = "# A\n\nNew start. Alpha beta gamma delta epsilon zeta eta theta.\n";
    let (state, text, _) = moved_to(old, "gamma delta", new);
    assert_eq!(
        (state, text.as_deref()),
        (AnchorState::Exact, Some("gamma delta"))
    );
}

#[test]
fn a_block_that_moved_takes_the_note_along() {
    let old = format!("# A\n\n## B\n\n{LONG}\n\n## C\n\nOther text in C.\n");
    let new = format!("# A\n\n## B\n\nNothing here now.\n\n## C\n\nOther text in C.\n\n{LONG}\n");
    let (state, text, _) = moved_to(&old, "every rendered page", &new);
    assert_eq!(
        (state, text.as_deref()),
        (AnchorState::Moved, Some("every rendered page"))
    );
    // Reordered inside its section: the pairing says the block moved.
    let old = format!("# A\n\n{LONG}\n\nSecond paragraph stays.\n\nThird one stays too.\n");
    let new = format!("# A\n\nSecond paragraph stays.\n\nThird one stays too.\n\n{LONG}\n");
    let (state, _, _) = moved_to(&old, LONG, &new);
    assert_eq!(state, AnchorState::Moved);
}

#[test]
fn a_reworded_quote_is_fuzzy() {
    let old = "# A\n\nKeep audit rows for thirty days, then delete them.\n";
    let new = "# A\n\nKeep audit rows for thirty days, then archive them.\n";
    let (state, text, score) = moved_to(
        old,
        "Keep audit rows for thirty days, then delete them.",
        new,
    );
    assert_eq!(state, AnchorState::Fuzzy);
    assert_eq!(
        text.as_deref(),
        Some("Keep audit rows for thirty days, then archive them.")
    );
    assert!((0.75..1.0).contains(&score), "{score}");
}

#[test]
fn a_rewritten_quote_is_outdated_on_its_block() {
    let old = "# A\n\nKeep audit rows for thirty days, then delete them.\n";
    let new = "# A\n\nAudit rows stay until retention is decided by the owner.\n";
    let (state, text, score) = moved_to(old, "Keep audit rows for thirty days", new);
    assert_eq!(state, AnchorState::Outdated);
    assert_eq!(
        text.as_deref(),
        Some("Audit rows stay until retention is decided by the owner.")
    );
    assert!(score.abs() < f64::EPSILON);
    // Part of a short quote survives, but its context does not agree well
    // enough: the note sits on the whole block.
    let new = "# A\n\nKeep audit rows for a while, then decide what happens to them.\n";
    let (state, text, score) = moved_to(old, "Keep audit rows for thirty days", new);
    assert_eq!(state, AnchorState::Outdated);
    assert_eq!(
        text.as_deref(),
        Some("Keep audit rows for a while, then decide what happens to them.")
    );
    assert!(score.abs() < f64::EPSILON);
    // Part of the quote survives with its context: the note sits on it.
    let old =
        "# A\n\nRows older than a year move to cold storage before they are deleted.\n\n## B\n";
    let new = "# A\n\nRows older than a year move to cold storage; nothing is deleted.\n\n## B\n";
    let (state, text, score) = moved_to(old, "move to cold storage before they are deleted", new);
    assert_eq!(state, AnchorState::Outdated);
    assert_eq!(
        text.as_deref(),
        Some("move to cold storage; nothing is deleted")
    );
    assert!((0.5..0.75).contains(&score), "{score}");
}

#[test]
fn a_deleted_quote_is_orphaned() {
    let old = format!("# A\n\n{LONG}\n\nThe rest of the text.\n");
    let new = "# A\n\nThe rest of the text.\n";
    assert_eq!(
        moved_to(&old, LONG, new),
        (AnchorState::Orphaned, None, 0.0)
    );
}

#[test]
fn a_lost_version_is_searched_by_section_then_everywhere() {
    let old = parse(&format!("# A\n\n## B\n\n{LONG}\n"));
    let anchor = anchor_on(&old, LONG, 0);
    let new = parse(&format!("# A\n\n## B\n\nIntro.\n\n{LONG}\n"));
    let placement = reanchor(&anchor, Origin::Lost, &new);
    assert_eq!(placement.state, AnchorState::Exact);
    let elsewhere = parse(&format!("# A\n\n## C\n\n{LONG}\n"));
    let placement = reanchor(&anchor, Origin::Lost, &elsewhere);
    assert_eq!(placement.state, AnchorState::Moved);
    let gone = parse("# A\n\n## B\n\nSomething else entirely.\n");
    assert_eq!(
        reanchor(&anchor, Origin::Lost, &gone),
        Placement::orphaned()
    );
}

#[test]
fn a_short_quote_needs_its_context() {
    // "only" moved out of its sentence: the word alone is no evidence.
    let old = "# A\n\nThe cache holds only pages that were read today.\n\nIt can only grow.\n";
    let new = "# A\n\nIt can only grow.\n";
    let (state, _, _) = moved_to(old, "only", new);
    assert_eq!(state, AnchorState::Orphaned);
    // With its context intact it is found again.
    let new = "# A\n\nFirst a new line.\n\nThe cache holds only pages that were read today.\n";
    let (state, text, _) = moved_to(old, "only", new);
    assert_eq!((state, text.as_deref()), (AnchorState::Exact, Some("only")));
}

#[test]
fn a_quote_over_two_blocks_follows_both() {
    let old = "# A\n\nFirst paragraph ends here.\n\nSecond paragraph starts here.\n";
    let new =
        "# A\n\nAn intro.\n\nFirst paragraph ends here.\n\nSecond paragraph starts here, edited.\n";
    let (state, text, _) = moved_to(old, "ends here.\n\nSecond paragraph", new);
    assert_eq!(
        (state, text.as_deref()),
        (AnchorState::Exact, Some("ends here.\n\nSecond paragraph"))
    );
}

#[test]
fn a_note_before_its_block_follows_the_next_block() {
    // A range can start in the white space before a block, as a quote
    // marker does; it belongs to the next block.
    let document = parse("# A\n\n> Quoted words here.\n");
    let start = document.source.find("> ").unwrap();
    let anchor = Anchor::capture(&document, start..start + 8).unwrap();
    assert_eq!(anchor.block, 1);
    let new = parse("# A\n\nNew.\n\n> Quoted words here.\n");
    let placement = reanchor(&anchor, Origin::Known(&document), &new);
    assert_eq!(placement.state, AnchorState::Exact);
    assert_eq!(&new.source[placement.range.unwrap()], "> Quoted");
}

#[test]
fn selected_card_text_is_found_in_its_block_source() {
    let document = parse(
        "# A\n\nA **bold** phrase with *nested* words, and a bold end.\n\n| a | b |\n|---|---|\n| x | y |\n",
    );
    let found = |block: usize, text: &str, hint: usize| {
        locate_text(&document, block, text, hint).map(|range| document.source[range].to_owned())
    };
    assert_eq!(found(1, "bold", 0).as_deref(), Some("bold"));
    assert_eq!(found(1, "bold", 40).as_deref(), Some("bold"));
    let second = locate_text(&document, 1, "bold", 40).unwrap();
    assert!(second.start > 30);
    assert_eq!(
        found(1, " phrase with nested words ", 0).as_deref(),
        Some("phrase with *nested* words")
    );
    assert_eq!(found(3, "x\ty", 0).as_deref(), Some("x | y"));
    assert_eq!(found(1, "   ", 0), None);
    assert_eq!(found(9, "bold", 0), None);
    assert_eq!(found(1, "zzzzzzzzzzzz", 0), None);
}

#[test]
fn blocks_are_found_by_offset() {
    let document = parse("# A\n\nText.\n");
    assert_eq!(block_of(&document, 0), Some(0));
    assert_eq!(block_of(&document, 3), Some(1));
    assert_eq!(block_of(&document, 6), Some(1));
    assert_eq!(block_of(&document, 11), None);
}

#[test]
fn states_have_names() {
    for (state, name) in [
        (AnchorState::Exact, "exact"),
        (AnchorState::Moved, "moved"),
        (AnchorState::Fuzzy, "fuzzy"),
        (AnchorState::Outdated, "outdated"),
        (AnchorState::Orphaned, "orphaned"),
    ] {
        assert_eq!(state.name(), name);
    }
}

#[test]
fn a_quote_at_either_end_of_the_file_has_no_context_there() {
    let old = parse("Hello world, at the very start.");
    let anchor = anchor_on(&old, "Hello world", 0);
    assert_eq!(anchor.quote.prefix, "");
    let end = anchor_on(&old, "the very start.", 0);
    assert_eq!(end.quote.suffix, "");
    let new = parse("An intro first.\n\nHello world, at the very start.");
    for anchor in [anchor, end] {
        let placement = reanchor(&anchor, Origin::Known(&old), &new);
        assert_eq!(placement.state, AnchorState::Exact);
        assert_eq!(
            &new.source[placement.range.unwrap()],
            anchor.quote.exact.as_str()
        );
    }
}

#[test]
fn a_score_weighs_quote_context_and_place() {
    let new = parse("xyabcw\n");
    let mut anchor = Anchor::capture(&new, 2..5).unwrap();
    anchor.quote.prefix = "xq".into();
    anchor.quote.suffix = "z".into();
    anchor.start = 0;
    let search = Search::new(&anchor, &new, None);
    let scored = search.score(2, 5, 1);
    let expected = (50.0 * (2.0 / 3.0) + 20.0 * 0.5 + 20.0 * 0.0 + 2.0 * (5.0 / 7.0)) / 92.0;
    assert!((scored.score - expected).abs() < 1e-12, "{}", scored.score);
    assert!((scored.quote - 2.0 / 3.0).abs() < 1e-12);
    assert_eq!((scored.range, scored.errors), (2..5, 1));
}

#[test]
fn a_search_allows_half_the_quote_in_edits() {
    let new = parse("abxxxxxx abcdxxxx\n");
    let mut anchor = Anchor::capture(&new, 0..8).unwrap();
    anchor.quote.exact = "abcdefgh".into();
    let search = Search::new(&anchor, &new, None);
    let window = 0..new.source.len() - 1;
    // "abcdxxxx" is four edits away: half of eight, still found.
    let found = search.best(window).unwrap();
    assert_eq!(found.errors, 4);
    assert!(new.source[found.range].starts_with("abcd"));
    // Without it, the closest is six edits away: too many.
    let narrow = parse("abxxxxxx\n");
    let search = Search::new(&anchor, &narrow, None);
    assert!(search.best(0..8).is_none());
}

#[test]
fn the_best_match_wins_and_a_tie_keeps_the_first() {
    let filler = "a".repeat(40);
    // Two places with the same text around them, the expected place exactly
    // in the middle: a tie, and the first wins.
    let text = format!("{filler}q{}q{filler}\n", "a".repeat(39));
    let new = parse(&text);
    let mut anchor = Anchor::capture(&new, 40..41).unwrap();
    let first = 40;
    let second = 80;
    anchor.start = usize::midpoint(first, second);
    let search = Search::new(&anchor, &new, None);
    let window = 0..text.len() - 1;
    assert_eq!(search.best(window.clone()).unwrap().range, first..first + 1);
    // Closer to the second, the second wins.
    anchor.start = second;
    let search = Search::new(&anchor, &new, None);
    assert_eq!(
        search.best(window.clone()).unwrap().range,
        second..second + 1
    );
    // Better context beats a closer place.
    let text = format!("{filler} the q here {filler} other q there {filler}\n");
    let new = parse(&text);
    let at = text.find("q here").unwrap();
    let mut anchor = Anchor::capture(&new, at..at + 1).unwrap();
    let other = text.find("q there").unwrap();
    anchor.start = other;
    let search = Search::new(&anchor, &new, None);
    assert_eq!(search.best(0..text.len() - 1).unwrap().range, at..at + 1);
}

#[test]
fn the_notes_section_is_searched_before_the_rest() {
    // The quote is in its own section with a new lead-in, and in another
    // section after the very words it used to follow. The own section wins.
    let old = parse(&format!(
        "# A\n\n## B\n\nIntro sentence here now.\n\n{LONG}\n\n## C\n\nNothing.\n"
    ));
    let anchor = anchor_on(&old, LONG, 0);
    let new = parse(&format!(
        "# A\n\n## B\n\nA different lead-in.\n\n{LONG} And more.\n\n## C\n\nIntro sentence here now.\n\n{LONG}\n"
    ));
    let placement = reanchor(&anchor, Origin::Lost, &new);
    assert_eq!(placement.state, AnchorState::Exact);
    let range = placement.range.unwrap();
    assert!(range.start < new.source.find("## C").unwrap(), "{range:?}");
    // Searched everywhere, the other place has the better context.
    let search = Search::new(&anchor, &new, None);
    let everywhere = search.best(0..new.source.len()).unwrap();
    assert!(everywhere.range.start > new.source.find("## C").unwrap());
}

#[test]
fn a_quote_that_ends_where_the_next_block_starts_covers_one_block() {
    let old = parse("# A\n\nFirst para words.\n\nSecond para here.\n");
    let start = old.source.find("First").unwrap();
    let end = old.source.find("Second").unwrap();
    let anchor = Anchor::capture(&old, start..end).unwrap();
    let new = parse("# A\n\nCompletely different now.\n\nSecond para here.\n");
    let placement = reanchor(&anchor, Origin::Known(&old), &new);
    assert_eq!(placement.state, AnchorState::Outdated);
    assert_eq!(
        &new.source[placement.range.unwrap()],
        "Completely different now."
    );
}

#[test]
fn an_outdated_short_quote_sits_on_its_whole_block() {
    let old = "# A\n\nThe cache holds at most 64 MB.\n";
    let new = "# A\n\nThe cache holds at most 128 MB.\n";
    let (state, text, score) = moved_to(old, "64 MB", new);
    assert_eq!(state, AnchorState::Outdated);
    assert_eq!(text.as_deref(), Some("The cache holds at most 128 MB."));
    assert!(score.abs() < f64::EPSILON);
}

#[test]
fn a_fuzzy_match_takes_whole_words() {
    let old = "# A\n\nFreeze the branch two days before the release, and tell every team.\n";
    let new = "# A\n\nFreeze the branch three days before the release, and tell every team.\n";
    let (state, text, _) = moved_to(old, "two days before the release, and tell every team", new);
    assert_eq!(state, AnchorState::Fuzzy);
    assert_eq!(
        text.as_deref(),
        Some("three days before the release, and tell every team")
    );
    // Bytes: `ab` 0..2, `cd` 3..5, `-` 5, `ëf` 6..9, `gh` 10..12.
    assert_eq!(whole_words("ab cd-ëf gh", 4..6), 3..6);
    assert_eq!(whole_words("ab cd-ëf gh", 4..8), 3..9);
    assert_eq!(whole_words("ab cd-ëf gh", 3..9), 3..9);
    assert_eq!(whole_words("ab cd-ëf gh", 2..3), 2..3);
    assert_eq!(whole_words("abc", 1..2), 0..3);
}

#[test]
fn text_after_the_last_block_cannot_carry_a_note() {
    // A lone quote marker makes no block.
    let document = parse("# A\n\n>\n");
    assert_eq!(&document.source[5..6], ">");
    assert_eq!(Anchor::capture(&document, 5..6), None);
}

#[test]
fn a_known_version_that_does_not_hold_the_note_is_searched_like_a_lost_one() {
    let long = parse(&format!("# A\n\nSome words first.\n\n{LONG}\n"));
    let anchor = anchor_on(&long, LONG, 0);
    let short = parse("# A\n");
    let placement = reanchor(&anchor, Origin::Known(&short), &long);
    assert_eq!(placement.state, AnchorState::Exact);
    assert_eq!(&long.source[placement.range.unwrap()], LONG);
}
