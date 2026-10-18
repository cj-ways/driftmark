use super::*;
use crate::document::Document;
use crate::search::{Entry, Shard};

fn path(text: &str) -> RepoPath {
    RepoPath::new(text).unwrap()
}

/// An index of `(path, type, text)` documents, in one shard.
fn index(documents: &[(&str, &str, &str)]) -> Index {
    let mut shard = Shard::new();
    for (name, doc_type, text) in documents {
        shard.add(Entry {
            path: &path(name),
            doc_type,
            document: &Document::parse(text.as_bytes().to_vec()).unwrap(),
        });
    }
    Index::from_shards(vec![shard])
}

fn query(text: &str) -> Query {
    Query::parse(text).unwrap()
}

/// The paths found, best first.
fn paths(found: &Found) -> Vec<String> {
    found
        .hits
        .iter()
        .map(|hit| hit.path.to_lossy().into_owned())
        .collect()
}

fn run(index: &Index, text: &str) -> Found {
    search(&[index], &query(text), &Filter::default(), 20)
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn title_hits_rank_above_heading_hits_and_heading_hits_above_body_hits() {
    let index = index(&[
        (
            "body.md",
            "d",
            "# One\n\nWe cache pages. Cache them twice, cache them thrice.\n",
        ),
        ("heading.md", "d", "# Two\n\n## Cache rules\n\nText.\n"),
        ("title.md", "d", "# Cache\n\nText.\n"),
        ("none.md", "d", "# Three\n\nNothing here.\n"),
    ]);
    let found = run(&index, "cache ");
    assert_eq!(paths(&found), ["title.md", "heading.md", "body.md"]);
    assert_eq!((found.documents, found.total), (4, 3));
}

#[test]
fn scores_add_each_clause_weight_times_where_it_stands() {
    let index = index(&[
        ("a.md", "d", "# Cache\n\nOne.\n"),
        ("b.md", "d", "Cache and cache.\n\nPages.\n"),
        ("c.md", "d", "Pages.\n"),
    ]);
    let ln = |x: f64| x.ln();
    let found = run(&index, "cache ");
    // Three documents, two hold `cache`.
    let weight = ln(1.0 + 3.0 / 2.0);
    assert!(close(found.hits[0].score, weight * 4.0), "{found:?}");
    assert!(close(found.hits[1].score, weight * 1.25), "{found:?}");
    let both = run(&index, "pages cache ");
    assert_eq!(paths(&both), ["b.md"]);
    let pages = ln(1.0 + 3.0 / 2.0);
    assert!(close(both.hits[0].score, pages + weight * 1.25), "{both:?}");
    let thrice = run(&index, "cache cache cache pages");
    assert_eq!(paths(&thrice), ["b.md"], "a clause counts once");
}

#[test]
fn a_rarer_word_weighs_more() {
    let index = index(&[
        ("a.md", "d", "## Rare\n\ncommon\n"),
        ("b.md", "d", "## Common\n\nrare\n"),
        ("c.md", "d", "common\n"),
        ("d.md", "d", "common\n"),
    ]);
    let found = run(&index, "common rare ");
    assert_eq!(paths(&found), ["a.md", "b.md"]);
    assert!(found.hits[0].score > found.hits[1].score);
}

#[test]
fn more_matches_never_lift_a_body_match_over_a_heading_match() {
    let many = "cache ".repeat(500);
    let index = index(&[
        ("a.md", "d", &format!("{many}\n")),
        ("b.md", "d", "## Cache\n"),
    ]);
    assert_eq!(paths(&run(&index, "cache ")), ["b.md", "a.md"]);
}

#[test]
fn equal_scores_rank_by_path() {
    let index = index(&[
        ("b.md", "d", "word\n"),
        ("a.md", "d", "word\n"),
        ("c.md", "d", "word\n"),
    ]);
    assert_eq!(paths(&run(&index, "word ")), ["a.md", "b.md", "c.md"]);
}

#[test]
fn every_clause_must_match() {
    let index = index(&[
        ("a.md", "d", "alpha beta\n"),
        ("b.md", "d", "alpha\n"),
        ("c.md", "d", "beta\n\nalpha\n"),
    ]);
    assert_eq!(paths(&run(&index, "alpha beta ")), ["a.md", "c.md"]);
    let none = run(&index, "alpha gamma ");
    assert_eq!((none.documents, none.total, none.hits.len()), (3, 0, 0));
    let empty = run(&index, "  ");
    assert_eq!((empty.documents, empty.total), (3, 0));
}

#[test]
fn phrases_need_their_words_together_and_in_one_block() {
    let index = index(&[
        ("a.md", "d", "Keep it for one hour.\n"),
        ("b.md", "d", "One cache, one hour later.\n"),
        ("c.md", "d", "Hour one.\n"),
        ("d.md", "d", "It took one\n\nhour.\n"),
        ("e.md", "d", "One-hour pages.\n"),
    ]);
    assert_eq!(
        paths(&run(&index, "\"one hour\"")),
        ["a.md", "b.md", "e.md"]
    );
    assert_eq!(paths(&run(&index, "one-hour")), ["a.md", "b.md", "e.md"]);
    assert_eq!(paths(&run(&index, "\"for one hour\"")), ["a.md"]);
    assert_eq!(paths(&run(&index, "\"one ho")), ["a.md", "b.md", "e.md"]);
    assert_eq!(paths(&run(&index, "\"hour one\"")), ["c.md"]);
}

#[test]
fn the_last_word_is_a_prefix_while_typing() {
    let index = index(&[
        ("a.md", "d", "Caching pages.\n"),
        ("b.md", "d", "A cache.\n"),
        ("c.md", "d", "Cash.\n"),
    ]);
    assert_eq!(paths(&run(&index, "cach")), ["a.md", "b.md"]);
    assert_eq!(run(&index, "cach ").total, 0);
    assert_eq!(paths(&run(&index, "ca")), ["a.md", "b.md", "c.md"]);
    assert_eq!(paths(&run(&index, "caching pag")), ["a.md"]);
}

#[test]
fn filters_keep_a_folder_and_some_types() {
    let index = index(&[
        ("docs/decisions/1.md", "decision", "word\n"),
        ("docs/drafts/2.md", "document", "word\n"),
        ("notes/3.md", "decision", "word\n"),
    ]);
    let filtered = |path: Option<&str>, types: &[&str]| {
        let filter = Filter {
            path: path.map(str::to_owned),
            types: types.iter().map(|&t| t.to_owned()).collect(),
        };
        paths(&search(&[&index], &query("word "), &filter, 20))
    };
    assert_eq!(
        filtered(Some("docs/decisions/"), &[]),
        ["docs/decisions/1.md"]
    );
    assert_eq!(
        filtered(Some("docs/d"), &[]),
        ["docs/decisions/1.md", "docs/drafts/2.md"]
    );
    assert_eq!(
        filtered(None, &["decision"]),
        ["docs/decisions/1.md", "notes/3.md"]
    );
    assert_eq!(
        filtered(None, &["document", "decision"]),
        ["docs/decisions/1.md", "docs/drafts/2.md", "notes/3.md"]
    );
    assert_eq!(
        filtered(Some("docs/"), &["decision"]),
        ["docs/decisions/1.md"]
    );
    assert_eq!(filtered(Some("other/"), &[]), Vec::<String>::new());
}

#[test]
fn the_limit_cuts_the_list_but_not_the_total() {
    let index = index(&[
        ("a.md", "d", "word\n"),
        ("b.md", "d", "word\n"),
        ("c.md", "d", "word\n"),
    ]);
    let found = search(&[&index], &query("word "), &Filter::default(), 2);
    assert_eq!(paths(&found), ["a.md", "b.md"]);
    assert_eq!(found.total, 3);
    let none = search(&[&index], &query("word "), &Filter::default(), 0);
    assert_eq!((none.total, none.hits.len()), (3, 0));
}

#[test]
fn a_later_index_replaces_documents_with_the_same_path() {
    let base = index(&[
        ("a.md", "d", "old words\n"),
        ("b.md", "d", "old words\n"),
        ("c.md", "d", "old words\n"),
    ]);
    let mut shard = Shard::new();
    shard.add(Entry {
        path: &path("a.md"),
        doc_type: "t",
        document: &Document::parse(b"new words\n".to_vec()).unwrap(),
    });
    shard.remove(&path("b.md"));
    shard.add(Entry {
        path: &path("d.md"),
        doc_type: "t",
        document: &Document::parse(b"# New\n\nold words\n".to_vec()).unwrap(),
    });
    let changes = Index::from_shards(vec![shard]);
    let layers = [&base, &changes];
    let found = search(&layers, &query("words "), &Filter::default(), 20);
    assert_eq!(found.documents, 3);
    let found_at: Vec<(String, usize, usize, String)> = found
        .hits
        .iter()
        .map(|hit| {
            (
                hit.path.to_lossy().into_owned(),
                hit.layer,
                hit.doc,
                hit.doc_type.clone(),
            )
        })
        .collect();
    assert_eq!(
        found_at,
        [
            ("a.md".into(), 1, 0, "t".into()),
            ("c.md".into(), 0, 2, "d".into()),
            ("d.md".into(), 1, 2, "t".into()),
        ]
    );
    assert_eq!(found.hits[2].title.as_deref(), Some("New"));
    assert_eq!(found.hits[0].title, None);
    let old = search(&layers, &query("old "), &Filter::default(), 20);
    assert_eq!(paths(&old), ["c.md", "d.md"]);
    let new = search(&layers, &query("new "), &Filter::default(), 20);
    assert_eq!(paths(&new), ["d.md", "a.md"]);
    let alone = search(&[&changes], &query("words "), &Filter::default(), 20);
    assert_eq!(alone.documents, 2);
}

#[test]
fn nothing_to_search_finds_nothing() {
    let empty = Index::from_shards(Vec::new());
    let found = run(&empty, "word");
    assert_eq!(found, Found::default());
    assert_eq!(
        search(&[], &query("word"), &Filter::default(), 5),
        Found::default()
    );
}

/// The block index each hit points at, by path.
fn blocks(index: &Index, text: &str) -> Vec<(String, usize)> {
    run(index, text)
        .hits
        .iter()
        .map(|hit| (hit.path.to_lossy().into_owned(), hit.block))
        .collect()
}

#[test]
fn the_best_block_holds_the_most_weight_and_avoids_the_title() {
    let index = index(&[
        // Blocks: 0 title, 1 paragraph, 2 heading, 3 paragraph.
        (
            "a.md",
            "d",
            "# Cache pages\n\nThe cache.\n\n## Cache\n\nThe cache and pages.\n",
        ),
        // Only the title matches.
        ("b.md", "d", "# Cache pages\n\nNothing.\n"),
        // A thematic break and a link definition are not indexed but count.
        ("c.md", "d", "***\n\n[x]: y\n\nOne cache.\n\nTwo cache.\n"),
        // Rare beats common.
        ("d.md", "d", "rare\n\npages pages\n\ncache\n"),
    ]);
    assert_eq!(
        blocks(&index, "cache "),
        [
            ("a.md".into(), 2),
            ("b.md".into(), 0),
            ("c.md".into(), 2),
            ("d.md".into(), 2),
        ]
    );
    assert_eq!(
        blocks(&index, "cache pages "),
        [("a.md".into(), 3), ("b.md".into(), 0), ("d.md".into(), 1)]
    );
    assert_eq!(blocks(&index, "rare cache ")[0], ("d.md".into(), 0));
    assert_eq!(blocks(&index, "cache rare ")[0], ("d.md".into(), 0));
}
