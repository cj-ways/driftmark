use super::*;

fn document(text: &str) -> Document {
    Document::parse(text.as_bytes().to_vec()).unwrap()
}

/// The snippet of the first block of `text`, with matches in brackets.
fn shown(query: &str, text: &str) -> String {
    let document = document(text);
    Query::parse(query)
        .unwrap()
        .snippet(&document.blocks[0])
        .iter()
        .map(|piece| {
            if piece.hit {
                format!("[{}]", piece.text)
            } else {
                piece.text.clone()
            }
        })
        .collect()
}

#[test]
fn a_short_block_shows_whole_with_its_matches_marked() {
    assert_eq!(
        shown("cache pag", "We cache pages, CACHE them.\n"),
        "We [cache] [pages], [CACHE] them."
    );
    assert_eq!(shown("nothing", "Plain text.\n"), "Plain text.");
    assert_eq!(shown("plain", "Plain text.\n"), "[Plain] text.");
    assert_eq!(shown("text", "Plain text\n"), "Plain [text]");
}

#[test]
fn a_phrase_is_marked_as_one_piece_only_where_its_words_stand_together() {
    assert_eq!(
        shown("\"one hour\"", "One cache, one\nhour later, hour one.\n"),
        "One cache, [one hour] later, hour one."
    );
    assert_eq!(
        shown("one-hour", "A one-hour page.\n"),
        "A [one-hour] page."
    );
}

#[test]
fn overlapping_matches_join() {
    assert_eq!(shown("cache cach", "A cache.\n"), "A [cache].");
    assert_eq!(shown("文 章", "中文章。\n"), "中[文章]。");
    assert_eq!(shown("\"a b\" \"b c\"", "x a b c y\n"), "x [a b c] y");
}

#[test]
fn white_space_runs_become_one_space_and_table_cells_a_dot() {
    assert_eq!(shown("x", "```\nx  =  1\n\ty = 2\n```\n"), "[x] = 1 y = 2");
    assert_eq!(
        shown("ten", "| Page | Time |\n|---|---|\n| Home | ten min |\n"),
        "Page · Time"
    );
    let document = document("| Page | Time |\n|---|---|\n| Home | ten min |\n");
    let pieces = Query::parse("ten").unwrap().snippet(&document.blocks[1]);
    assert_eq!(
        pieces,
        [
            Piece {
                text: "Home · ".into(),
                hit: false
            },
            Piece {
                text: "ten".into(),
                hit: true
            },
            Piece {
                text: " min".into(),
                hit: false
            },
        ]
    );
}

/// Words `w0 w1 …`, each three characters or more, `count` of them.
fn numbered(count: usize) -> String {
    (0..count)
        .map(|n| format!("w{n:02}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Words `w{from}` to `w{to - 1}` of [`numbered`], joined.
fn words_between(from: usize, to: usize) -> String {
    (from..to)
        .map(|n| format!("w{n:02}"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn a_long_block_is_cut_around_its_first_match_with_some_lead() {
    // 100 words of 3 characters: 399 characters.
    let text = format!("{}\n", numbered(100));
    let cut = shown("w50", &text);
    assert_eq!(
        cut,
        format!(
            "…{} [w50] {}…",
            words_between(40, 50),
            words_between(51, 80)
        )
    );
    let (before, _) = cut.split_once("[w50]").unwrap();
    assert_eq!(
        before.chars().count(),
        1 + LEAD_CHARS,
        "the cut mark and the lead"
    );
    let early = shown("w05", &text);
    assert_eq!(
        early,
        format!("{} [w05] {}…", words_between(0, 5), words_between(6, 40))
    );
}

#[test]
fn text_that_is_not_a_match_is_one_piece() {
    let document = document(&format!("{}\n", numbered(100)));
    let pieces = Query::parse("w50").unwrap().snippet(&document.blocks[0]);
    assert_eq!(
        pieces,
        [
            Piece {
                text: format!("…{} ", words_between(40, 50)),
                hit: false
            },
            Piece {
                text: "w50".into(),
                hit: true
            },
            Piece {
                text: format!(" {}…", words_between(51, 80)),
                hit: false
            },
        ]
    );
}

#[test]
fn the_run_with_the_most_matches_wins_over_earlier_smaller_runs() {
    // One match, then two side by side, then three spread out.
    let gap = ["gap"; 50].join(" ");
    let wide = ["far"; 12].join(" ");
    let text = format!("key {gap} key key {gap} key {wide} key {wide} key {gap}\n");
    let cut = shown("key", &text);
    assert_eq!(cut.matches("[key]").count(), 3, "{cut}");
    assert!(!cut.contains("[key] [key]"), "{cut}");
    assert!(cut.starts_with("…gap gap"), "{cut}");
}

#[test]
fn a_cut_near_the_end_fills_the_snippet_from_before() {
    let text = format!("{}\n", numbered(100));
    assert_eq!(
        shown("w98", &text),
        format!("…{} [w98] w99", words_between(60, 98))
    );
}

#[test]
fn without_a_match_a_long_block_shows_its_start() {
    let text = format!("{}\n", numbered(100));
    let cut = shown("nothing", &text);
    assert_eq!(cut, format!("{}…", numbered(40)));
}

#[test]
fn the_cut_shows_the_run_with_the_most_matches() {
    let mut words: Vec<String> = numbered(120).split(' ').map(str::to_owned).collect();
    for at in [10, 60, 64, 68] {
        words[at] = "key".to_owned();
    }
    let text = format!("{}\n", words.join(" "));
    let cut = shown("key", &text);
    assert_eq!(cut.matches("[key]").count(), 3, "{cut}");
    assert!(cut.starts_with("…w50 "), "{cut}");
    // A tie goes to the first run.
    words[10] = "w10".to_owned();
    words[11] = "key".to_owned();
    words[13] = "key".to_owned();
    words[15] = "key".to_owned();
    let tied = shown("key", &format!("{}\n", words.join(" ")));
    assert!(tied.starts_with("…w01 "), "{tied}");
    assert_eq!(tied.matches("[key]").count(), 3, "{tied}");
}

#[test]
fn a_match_longer_than_the_snippet_shows_whole() {
    // 30 words of 10 characters: a phrase of 329 characters.
    let long = (0..30)
        .map(|n| format!("longword{n:02}"))
        .collect::<Vec<_>>()
        .join(" ");
    let text = format!("start {long} end\n");
    let cut = shown(&format!("\"{long}\""), &text);
    assert_eq!(cut, format!("start [{long}]…"));
    let text = format!("{} {long} end of it all\n", numbered(30));
    let cut = shown(&format!("\"{long}\""), &text);
    assert_eq!(cut, format!("…{} [{long}]…", words_between(20, 30)));
}

#[test]
fn cuts_fall_between_words_and_count_characters_not_bytes() {
    let georgian = "ქართული ".repeat(40);
    let text = format!("{georgian}სიტყვა {georgian}\n");
    let cut = shown("სიტყვა", &text);
    assert!(cut.starts_with("…ქართული"), "{cut}");
    assert!(cut.ends_with("ქართული…"), "{cut}");
    let words: Vec<&str> = cut.trim_matches('…').split(' ').collect();
    assert!(
        words.iter().all(|w| ["ქართული", "[სიტყვა]"].contains(w)),
        "{cut}"
    );
    let (before, _) = cut.split_once('[').unwrap();
    assert!(before.chars().count() <= 1 + LEAD_CHARS, "{cut}");
    let long = format!("{}{}\n", "a".repeat(150), "b ".repeat(20));
    let cut = shown("nothing", &long);
    assert!(cut.starts_with(&"a".repeat(150)), "{cut}");
}

/// Words of two letters, `aa`, `ab` and on, `count` of them.
fn pairs(count: usize) -> Vec<String> {
    (0..count)
        .map(|n| {
            let letter = |at: usize| char::from(b'a' + u8::try_from(at).unwrap());
            format!("{}{}", letter(n / 26), letter(n % 26))
        })
        .collect()
}

#[test]
fn a_cut_that_lands_on_a_space_starts_at_the_next_word() {
    // Three characters a word, so the lead of 40 ends on a space.
    let words = pairs(200);
    let text = format!("{}\n", words.join(" "));
    let cut = shown(&words[60], &text);
    assert_eq!(
        cut,
        format!(
            "…{} [{}] {}…",
            words[47..60].join(" "),
            words[60],
            words[61..100].join(" ")
        )
    );
}

#[test]
fn matches_exactly_the_snippet_length_apart_fit_together() {
    let text = format!(
        "key {} key {} key {} key {}\n",
        "a".repeat(152),
        "b".repeat(200),
        "c".repeat(50),
        "d".repeat(200)
    );
    let cut = shown("key", &text);
    assert!(cut.starts_with("[key] aaa"), "{cut}");
    assert!(cut.ends_with("aaa [key]…"), "{cut}");
}

#[test]
fn char_offsets_count_characters_before_each_byte() {
    assert_eq!(char_offsets("aბc", [0, 1, 4, 5].into_iter()), [0, 1, 2, 3]);
    assert_eq!(char_offsets("", std::iter::empty()), Vec::<usize>::new());
}

#[test]
fn places_name_the_section_and_its_anchor() {
    let document = document(
        "Intro.\n\n# Cache pages\n\n## Decision\n\nKeep it.\n\n> ## Quoted\n>\n> In a quote.\n\n## Decision\n\nAgain.\n",
    );
    let place_of = |block: usize| place(&document, block);
    assert_eq!(place_of(0), Place::default(), "no heading before it");
    assert_eq!(
        place_of(1),
        Place {
            headings: vec!["Cache pages".into()],
            anchor: Some("cache-pages".into()),
        }
    );
    assert_eq!(
        place_of(3),
        Place {
            headings: vec!["Cache pages".into(), "Decision".into()],
            anchor: Some("decision".into()),
        }
    );
    assert_eq!(
        place_of(4),
        Place {
            headings: vec!["Cache pages".into(), "Decision".into()],
            anchor: Some("quoted".into()),
        },
        "a heading in a quote opens no section but has its own anchor"
    );
    assert_eq!(place_of(5).anchor.as_deref(), Some("decision"));
    assert_eq!(place_of(7).anchor.as_deref(), Some("decision-1"));
    assert_eq!(place_of(99), Place::default());
}
