//! Notes made at an old commit of a real repository, found again at a newer
//! one: the counts per anchoring state, and the time it takes.
//!
//! Ignored by default. Run it on this repository with
//! `cargo test -p driftmark --test notes_history -- --ignored --nocapture`.
//! `DRIFTMARK_NOTES_REPO` names another repository, `DRIFTMARK_NOTES_FROM`
//! and `DRIFTMARK_NOTES_TO` the two revisions (default `HEAD~12` and
//! `HEAD`), and `DRIFTMARK_NOTES_IN` a folder to keep to, such as `docs`.
//! It reads git only, and prints counts, never content.
//!
//! On every Markdown document at the old revision that still exists at the
//! newer one, it makes a note on the first sentence of each paragraph, list
//! item and table row, and on one word in the middle of it, as a reader
//! would, then re-anchors every note in the newer version.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

use driftmark::git::Git;
use driftmark_core::document::{BlockKind, Document};
use driftmark_core::git::{GitRead, Roots, Snapshot};
use driftmark_core::notes::{Anchor, Origin, reanchor};

/// The stretches a reader might note in one block: its first sentence, and
/// one word in the middle of it.
fn selections(document: &Document, block: usize) -> Vec<std::ops::Range<usize>> {
    let found = &document.blocks[block];
    let source = &document.source[found.range.clone()];
    let start = found.range.start + source.len()
        - source
            .trim_start_matches(['-', '*', '|', ' ', '>', '#'])
            .len();
    let rest = &document.source[start..found.range.end];
    let end = rest
        .find(". ")
        .map_or(rest.len(), |at| at + 1)
        .min(rest.len());
    let sentence = start..start + end;
    let mut out = vec![sentence.clone()];
    let text = &document.source[sentence];
    let words: Vec<(usize, &str)> = text
        .split_whitespace()
        .map(|word| (word.as_ptr() as usize - text.as_ptr() as usize, word))
        .filter(|(_, word)| word.chars().all(char::is_alphabetic) && word.len() > 3)
        .collect();
    if let Some(&(at, word)) = words.get(words.len() / 2) {
        out.push(start + at..start + at + word.len());
    }
    out
}

#[test]
#[ignore = "reads a whole history; run with --ignored"]
fn notes_from_an_old_commit_are_found_again() {
    let repo = std::env::var_os("DRIFTMARK_NOTES_REPO").map_or_else(
        || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."),
        PathBuf::from,
    );
    let from = std::env::var("DRIFTMARK_NOTES_FROM").unwrap_or_else(|_| "HEAD~12".to_owned());
    let to = std::env::var("DRIFTMARK_NOTES_TO").unwrap_or_else(|_| "HEAD".to_owned());
    let git = Git::open(&repo).expect("a repository");
    let old_commit = git.resolve(&from).expect("the old revision");
    let new_commit = git.resolve(&to).expect("the new revision");
    let roots = std::env::var("DRIFTMARK_NOTES_IN").map_or_else(
        |_| Roots::all(),
        |folder| Roots::new([folder]).expect("a folder"),
    );
    let files = git
        .markdown_files(&Snapshot::Commit(old_commit.clone()), &roots)
        .expect("the old files");
    let mut states: BTreeMap<&'static str, usize> = BTreeMap::new();
    let (mut documents, mut changed, mut notes) = (0, 0, 0);
    let mut slowest = (0.0_f64, 0);
    let started = Instant::now();
    let mut anchoring = 0.0;
    for file in files {
        let Ok(Some(new_blob)) = git.blob_at(&new_commit, &file.path) else {
            continue;
        };
        let parse = |id| Document::parse(git.read_blob(id).expect("a blob")).ok();
        let (Some(old), Some(new)) = (parse(&file.id), parse(&new_blob)) else {
            continue;
        };
        documents += 1;
        let same = file.id == new_blob;
        changed += usize::from(!same);
        let mut anchors = Vec::new();
        for (index, block) in old.blocks.iter().enumerate() {
            let worth_a_note = matches!(
                block.kind,
                BlockKind::Paragraph | BlockKind::ListItem(_) | BlockKind::TableRow { .. }
            );
            if worth_a_note {
                anchors.extend(
                    selections(&old, index)
                        .into_iter()
                        .filter_map(|range| Anchor::capture(&old, range)),
                );
            }
        }
        let clock = Instant::now();
        let map = driftmark_core::diff::block_map(&old, &new);
        for anchor in &anchors {
            let origin = if same {
                Origin::Same
            } else {
                Origin::Paired(&old, &map)
            };
            let placement = reanchor(anchor, origin, &new);
            *states.entry(placement.state.name()).or_default() += 1;
        }
        let took = clock.elapsed().as_secs_f64() * 1000.0;
        anchoring += took;
        if took > slowest.0 {
            slowest = (took, anchors.len());
        }
        notes += anchors.len();
    }
    println!(
        "{documents} documents ({changed} changed), {notes} notes, from {} to {}",
        &old_commit.as_hex()[..7],
        &new_commit.as_hex()[..7]
    );
    for (state, count) in &states {
        println!("  {state}: {count}");
    }
    println!(
        "  re-anchoring: {anchoring:.1} ms in all, slowest document {:.1} ms for {} notes; whole run {:.1} s",
        slowest.0,
        slowest.1,
        started.elapsed().as_secs_f64()
    );
    assert!(notes > 0, "no notes were made");
}
