use super::*;
use crate::diff::{ChangeKind, Version, compare};
use crate::rules::Rules;

fn parse(source: &str) -> Document {
    Document::parse(source.as_bytes().to_vec()).expect("parses")
}

/// The change list of one document between two versions, as a range page
/// shows it.
fn changes(base: Option<&Document>, head: Option<&Document>) -> DocumentChanges {
    let rules = Rules::default();
    let base_class = base.map(|document| rules.classify("doc.md", document));
    let head_class = head.map(|document| rules.classify("doc.md", document));
    compare(
        "doc.md",
        version(base, base_class.as_ref()),
        version(head, head_class.as_ref()),
    )
    .expect("a side")
}

fn version<'a>(
    document: Option<&'a Document>,
    classification: Option<&'a crate::rules::Classification>,
) -> Option<Version<'a>> {
    document
        .zip(classification)
        .map(|(document, classification)| Version {
            document,
            classification,
        })
}

/// A short description of each change of `list` at `indices`: its kind and
/// the text it shows.
fn described(list: &DocumentChanges, indices: &[usize]) -> Vec<String> {
    indices
        .iter()
        .map(|&index| {
            let change = &list.changes[index];
            let text = match &change.subject {
                Subject::Document { document } => format!("document {}", document.lines),
                Subject::Block { old, new } => new
                    .block
                    .as_ref()
                    .or(old.block.as_ref())
                    .map(|block| block.text.clone())
                    .unwrap_or_default(),
            };
            format!("{:?} {text}", change.kind)
        })
        .collect()
}

/// What a commit from `before` to `after` made of the range `base` to
/// `head`, described.
fn made(
    base: Option<&str>,
    before: Option<&str>,
    after: Option<&str>,
    head: Option<&str>,
) -> Vec<String> {
    let [base, before, after, head] = [base, before, after, head].map(|text| text.map(parse));
    let list = changes(base.as_ref(), head.as_ref());
    let found = caused(
        &list,
        Around {
            before: before.as_ref(),
            after: after.as_ref(),
        },
        Ends {
            base: base.as_ref(),
            head: head.as_ref(),
        },
    );
    described(&list, &found)
}

const BASE: &str = "# Cache\n\nAlpha keeps pages.\n\nBeta drops pages.\n\nGamma counts pages.\n";

#[test]
fn a_commit_that_is_the_whole_range_made_every_change() {
    let head = "# Cache\n\nAlpha keeps every page.\n\nGamma counts pages.\n\nDelta is new here.\n";
    assert_eq!(
        made(Some(BASE), Some(BASE), Some(head), Some(head)),
        [
            "Edited Alpha keeps every page.",
            "Removed Beta drops pages.",
            "Added Delta is new here."
        ]
    );
}

#[test]
fn each_of_two_commits_made_its_own_change() {
    let middle = "# Cache\n\nAlpha keeps every page.\n\nBeta drops pages.\n\nGamma counts pages.\n";
    let head =
        "# Cache\n\nAlpha keeps every page.\n\nBeta drops pages.\n\nGamma counts each page.\n";
    assert_eq!(
        made(Some(BASE), Some(BASE), Some(middle), Some(head)),
        ["Edited Alpha keeps every page."]
    );
    assert_eq!(
        made(Some(BASE), Some(middle), Some(head), Some(head)),
        ["Edited Gamma counts each page."]
    );
}

#[test]
fn a_block_edited_again_later_still_counts_for_the_first_commit() {
    let middle = "# Cache\n\nAlpha keeps every page.\n\nBeta drops pages.\n\nGamma counts pages.\n";
    let head =
        "# Cache\n\nAlpha keeps every single page.\n\nBeta drops pages.\n\nGamma counts pages.\n";
    let first = made(Some(BASE), Some(BASE), Some(middle), Some(head));
    assert_eq!(first, ["Edited Alpha keeps every single page."]);
    assert_eq!(
        made(Some(BASE), Some(middle), Some(head), Some(head)),
        first
    );
}

#[test]
fn a_deletion_is_followed_back_to_the_base() {
    let middle = "# Cache\n\nAlpha keeps pages.\n\nGamma counts pages.\n";
    let head = "# Cache\n\nAlpha keeps pages.\n\nGamma counts each page.\n";
    assert_eq!(
        made(Some(BASE), Some(BASE), Some(middle), Some(head)),
        ["Removed Beta drops pages."]
    );
    assert_eq!(
        made(Some(BASE), Some(middle), Some(head), Some(head)),
        ["Edited Gamma counts each page."]
    );
}

#[test]
fn a_moved_block_is_the_commits() {
    let head = "# Cache\n\nBeta drops pages.\n\nAlpha keeps pages.\n\nGamma counts pages.\n";
    let list = changes(Some(&parse(BASE)), Some(&parse(head)));
    assert!(
        list.changes
            .iter()
            .any(|change| change.kind == ChangeKind::Moved)
    );
    assert_eq!(
        made(Some(BASE), Some(BASE), Some(head), Some(head)).len(),
        list.changes.len()
    );
}

#[test]
fn a_block_the_commit_left_alone_is_not_the_commits() {
    // The commit edits Alpha; Beta and Gamma stay in place and the same. A
    // later commit edits Beta.
    let middle = "# Cache\n\nAlpha keeps every page.\n\nBeta drops pages.\n\nGamma counts pages.\n";
    let head =
        "# Cache\n\nAlpha keeps every page.\n\nBeta drops old pages.\n\nGamma counts pages.\n";
    assert_eq!(
        made(Some(BASE), Some(BASE), Some(middle), Some(head)),
        ["Edited Alpha keeps every page."]
    );
}

#[test]
fn a_removed_cards_place_is_not_a_block_the_commit_wrote() {
    // The commit edits Beta. A later commit removes the paragraph above it,
    // so the removed card's place in the head is the index Beta has there.
    let base = "# Cache\n\nAlpha keeps pages.\n\nRemoved later on.\n\nBeta drops pages.\n";
    let middle = "# Cache\n\nAlpha keeps pages.\n\nRemoved later on.\n\nBeta drops every page.\n";
    let head = "# Cache\n\nAlpha keeps pages.\n\nBeta drops every page.\n";
    let list = changes(Some(&parse(base)), Some(&parse(head)));
    let removed = list
        .changes
        .iter()
        .find(|change| change.kind == ChangeKind::Removed)
        .and_then(|change| change.sides())
        .map(|(_, new)| new.at);
    let edited = list
        .changes
        .iter()
        .find(|change| change.kind == ChangeKind::Edited)
        .and_then(|change| change.sides())
        .and_then(|(_, new)| new.block.as_ref().map(|_| new.at));
    assert!(removed.is_some());
    assert_eq!(removed, edited, "the test needs the two places to be equal");
    assert_eq!(
        made(Some(base), Some(base), Some(middle), Some(head)),
        ["Edited Beta drops every page."]
    );
}

#[test]
fn an_added_cards_place_is_not_a_block_the_commit_deleted() {
    // The commit deletes Beta. A later commit writes Beta back and adds a
    // list item above it, so the range keeps Beta, and the added card's
    // place in the base is Beta's index: a block the commit deleted.
    let base = "# Cache\n\nAlpha keeps pages.\n\nBeta drops pages.\n";
    let middle = "# Cache\n\nAlpha keeps pages.\n";
    let head = "# Cache\n\nAlpha keeps pages.\n\n- Fresh wording arrives.\n\nBeta drops pages.\n";
    let list = changes(Some(&parse(base)), Some(&parse(head)));
    let added = list
        .changes
        .iter()
        .find(|change| change.kind == ChangeKind::Added)
        .and_then(|change| change.sides())
        .map(|(old, _)| old.at);
    assert_eq!(added, Some(2), "the test needs Beta's index");
    assert_eq!(list.changes.len(), 1);
    assert_eq!(
        made(Some(base), Some(base), Some(middle), Some(head)),
        [] as [&str; 0]
    );
}

#[test]
fn a_new_document_is_the_commits_when_it_wrote_a_block_of_it() {
    let head = "# Cache\n\nAlpha keeps pages.\n";
    let later = "# Cache\n\nAlpha keeps every page.\n";
    assert_eq!(
        made(None, None, Some(head), Some(head)),
        ["Added document 3"]
    );
    assert_eq!(
        made(None, Some(head), Some(later), Some(later)),
        ["Added document 3"]
    );
    // Only blank lines between blocks changed: no block is the commit's.
    let spaced = "# Cache\n\n\nAlpha keeps pages.\n";
    assert_eq!(
        made(None, Some(head), Some(spaced), Some(spaced)),
        [] as [&str; 0]
    );
}

#[test]
fn a_deleted_document_is_the_commits_when_it_deleted_it() {
    assert_eq!(
        made(Some(BASE), Some(BASE), None, None),
        ["Removed document 7"]
    );
    // A commit that deleted one block before the document went counts too.
    let middle = "# Cache\n\nAlpha keeps pages.\n\nGamma counts pages.\n";
    assert_eq!(
        made(Some(BASE), Some(BASE), Some(middle), None),
        ["Removed document 7"]
    );
}

#[test]
fn a_commit_that_never_had_the_document_made_nothing() {
    assert_eq!(made(Some(BASE), None, None, Some(BASE)), [] as [&str; 0]);
    let head = "# Cache\n\nAlpha keeps every page.\n";
    assert_eq!(made(Some(BASE), None, None, Some(head)), [] as [&str; 0]);
}

#[test]
fn a_commit_before_a_document_the_range_added_follows_nothing_back() {
    // The range starts without the document, so a block the commit deleted
    // has nowhere to go back to; the document change is still the commit's.
    let first = "# Cache\n\nAlpha keeps pages.\n\nBeta drops pages.\n";
    let second = "# Cache\n\nAlpha keeps pages.\n";
    assert_eq!(
        made(None, Some(first), Some(second), Some(second)),
        ["Added document 3"]
    );
}
