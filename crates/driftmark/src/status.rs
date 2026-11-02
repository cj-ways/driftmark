//! The status view for the program: every document at one revision read,
//! parsed and classified on all cores.
//!
//! `driftmark-core` builds the view from what this module reads
//! ([`driftmark_core::status`]); the server and the MCP connector keep the
//! results and add when each document last changed
//! (`docs/features/status-page.md`).

use driftmark_core::document::Document;
use driftmark_core::git::{GitError, GitRead};
use driftmark_core::rules::{RepoFiles, Rules};
use driftmark_core::status::{Entry, needed};
use serde::Serialize;

use crate::search::{self, File, parallel};

/// How many files one run holds. Threads take runs one after another, so
/// small runs keep every thread busy to the end even when the large
/// documents of a repository sit next to each other in path order.
const RUN: usize = 16;

/// A file left out of the status view, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Unread {
    /// Its path.
    pub path: String,
    /// Why, as a sentence.
    pub reason: String,
}

/// Reads, parses and classifies `files` with `rules` on up to `threads`
/// threads. A document is parsed only up to the end of its head when its
/// type reads nothing from a later section
/// ([`driftmark_core::status::needed`]). `beside` answers for the files beside a document that a rules
/// location names, such as cc-sdd's `spec.json`. The entries keep the order
/// of `files`; a file gone from the disk is left out, and one that cannot be
/// parsed is listed in the second list with its reason.
///
/// # Errors
///
/// The first error from reading git, in the order of `files`.
pub fn entries(
    git: &dyn GitRead,
    files: &[File],
    rules: &Rules,
    beside: &(dyn RepoFiles + Sync),
    threads: usize,
) -> Result<Read, GitError> {
    let runs: Vec<(usize, &[File])> = files
        .chunks(RUN)
        .enumerate()
        .map(|(at, run)| (at * RUN, run))
        .collect();
    let done = parallel(
        &runs,
        threads,
        |(start, run)| read_run(git, *start, run, rules, beside),
        Result::is_err,
    );
    let mut entries = Vec::with_capacity(files.len());
    let mut unread = Vec::new();
    for run in done {
        let (found, left_out) = run?;
        entries.extend(found);
        unread.extend(left_out);
    }
    Ok((entries, unread))
}

/// What reading files gave: each entry with its file's position, and the
/// files that could not be parsed.
pub type Read = (Vec<(usize, Entry)>, Vec<Unread>);

/// What one run of files gave.
type Run = Result<Read, GitError>;

fn read_run(
    git: &dyn GitRead,
    start: usize,
    files: &[File],
    rules: &Rules,
    beside: &(dyn RepoFiles + Sync),
) -> Run {
    let mut entries = Vec::with_capacity(files.len());
    let mut unread = Vec::new();
    for (at, file) in files.iter().enumerate() {
        let read = match &file.read {
            search::Read::Blob(blob) => git.read_blob(blob),
            search::Read::Disk => git.read_worktree_file(&file.path),
            search::Read::Gone => continue,
        };
        let mut bytes = match read {
            Ok(bytes) => bytes,
            Err(GitError::MissingFile { .. }) => continue,
            Err(error) => return Err(error),
        };
        let path = file.path.to_lossy().into_owned();
        // Only the head is parsed when that is all the view needs.
        let head = needed(&bytes, rules.type_for(&path)).len();
        bytes.truncate(head);
        match Document::parse_with(bytes, rules.masthead_keys()) {
            Ok(document) => {
                let class = rules.classify_with(&path, &document, beside);
                let title = document.meta.title;
                entries.push((start + at, Entry::new(path, title, class)));
            }
            Err(error) => unread.push(Unread {
                path,
                reason: format!("it cannot be read: {error}"),
            }),
        }
    }
    Ok((entries, unread))
}
