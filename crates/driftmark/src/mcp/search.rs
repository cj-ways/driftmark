//! `search`: the server's full-text search, with the same query and
//! filters as `/api/search` (`docs/features/engine/search.md`).
//!
//! A commit's index comes from the same cache the server uses. The working
//! tree's index is built again only when its files changed: each search
//! lists the working tree with the git reader, which hashes every Markdown
//! file, and compares the list with the one the index was built from. The
//! server follows the working tree with a watcher instead; an agent's
//! connector searches far less often than a page that reloads, and starts
//! no background thread this way.

use std::sync::{Arc, Mutex, PoisonError};

use driftmark_core::git::{GitError, GitRead, Roots, Snapshot};
use driftmark_core::search::{Filter, Found, Piece, Query, search};
use serde_json::{Value, json};

use super::fit;
use super::tools::{Args, Context, MAX_OFFSET, Tool, list_cut, offset_schema, page, said};
use crate::search::{self as program, File, Layer, Read};
use crate::serve::address::WORKTREE;
use crate::serve::compare::{self, RulesAt};
use crate::serve::repo::{Problem, Repo};
use crate::serve::search::commit_layer;

/// How many results an answer lists when the call names no limit. Fewer
/// than the server's 20: an agent reads every result, a reader skims.
const DEFAULT_LIMIT: usize = 10;

/// The most results one answer may ask for.
const MAX_RESULTS: usize = 50;

/// The most entries of `skipped` an answer lists.
const SKIPPED_SHOWN: usize = 20;

/// `search`.
pub(crate) const SEARCH: Tool = Tool {
    name: "search",
    title: "Search the docs",
    description: "Finds the Markdown documents that hold every word of a query, best first: a match in the title ranks above one in a heading, and a heading above the body. Each result names the block that matches best, the headings above it, the section's anchor (for document_read's `section`) and a snippet with the matched words between ** marks. Words may be \"quoted phrases\"; the last word also finds longer words it starts. Filter by folder with `path` and by document type with `types`.",
    arguments,
    required: &["query"],
    run,
};

fn arguments() -> Value {
    json!({
        "query": {
            "type": "string",
            "description": "Words, and \"phrases\" in quotes; a document must hold all of them. Case does not matter. At most 32 words.",
        },
        "revision": {
            "type": "string",
            "description": "A branch, a tag, a commit or an expression such as main~2, or \"worktree\" for the files on disk now. Default \"worktree\".",
        },
        "path": {
            "type": "string",
            "description": "Keep only documents whose path starts with this text, such as docs/decisions/.",
        },
        "types": {
            "type": "array",
            "items": {"type": "string"},
            "description": "Keep only documents of these types from the rules file, such as [\"decision\"]. A file no type matches has the type \"document\".",
        },
        "offset": offset_schema("results"),
        "limit": {
            "type": "integer",
            "minimum": 1,
            "maximum": MAX_RESULTS,
            "description": format!("The most results to list. Default {DEFAULT_LIMIT}; fewer when they do not fit in `max_chars`."),
        },
    })
}

/// The working tree's index, and what it was built from.
struct Built {
    /// A hash of the listed files' paths and blob ids, and of the rules
    /// file's version.
    fingerprint: u64,
    /// The index.
    layer: Layer,
    /// The rules file it was built with.
    rules: Arc<RulesAt>,
}

/// The working tree's search index, built on the first search and again
/// whenever the working tree's Markdown files or rules file changed.
#[derive(Default)]
pub(crate) struct WorktreeSearch {
    built: Mutex<Option<Arc<Built>>>,
}

impl WorktreeSearch {
    /// The index of the working tree as it is now.
    fn current(&self, repo: &Repo) -> Result<Arc<Built>, Problem> {
        let files = repo
            .git
            .markdown_files(&Snapshot::Worktree, &Roots::all())?;
        let rules = repo.rules_at(&Snapshot::Worktree)?;
        let mut listed = format!("{:?}\n", rules.id).into_bytes();
        for file in &files {
            listed.extend_from_slice(file.path.as_bytes());
            listed.push(0);
            listed.extend_from_slice(file.id.as_hex().as_bytes());
            listed.push(b'\n');
        }
        let fingerprint = crate::serve::fingerprint(&listed);
        let mut built = self.built.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(index) = built
            .as_ref()
            .filter(|index| index.fingerprint == fingerprint)
        {
            return Ok(Arc::clone(index));
        }
        let files: Vec<File> = files
            .into_iter()
            .map(|file| File {
                path: file.path,
                read: Read::Disk,
            })
            .collect();
        let layer = program::build(&repo.git, &files, &rules.rules, repo.threads);
        drop(files);
        crate::memory::give_back(&repo.git);
        let index = Arc::new(Built {
            fingerprint,
            layer: layer?,
            rules,
        });
        *built = Some(Arc::clone(&index));
        Ok(index)
    }
}

/// A snippet as one text, each match between `**` marks.
fn snippet_text(pieces: &[Piece]) -> String {
    pieces
        .iter()
        .map(|piece| {
            if piece.hit {
                format!("**{}**", piece.text)
            } else {
                piece.text.clone()
            }
        })
        .collect()
}

fn run(context: &Context, args: &Args) -> Result<Value, String> {
    let limit = args.max_chars()?;
    let text = args.text("query")?.unwrap_or_default();
    let query = Query::parse(&text).map_err(|error| error.to_string())?;
    let offset = args.number("offset", 0, MAX_OFFSET, 0)?;
    let most = args.number("limit", 1, MAX_RESULTS, DEFAULT_LIMIT)?;
    let filter = Filter {
        path: args.text("path")?.filter(|path| !path.is_empty()),
        types: args.texts("types")?,
    };
    let default = if context.workdir.is_some() {
        WORKTREE
    } else {
        "HEAD"
    };
    let revision = args.text("revision")?.unwrap_or_else(|| default.to_owned());
    let repo = &context.repo;
    let wanted = offset.saturating_add(most);
    let (found, matches, commit, skipped) = if revision == WORKTREE {
        if context.workdir.is_none() {
            return Err(said(GitError::NoWorktree.into()));
        }
        let index = context.worktree_search.current(repo).map_err(said)?;
        let found = page_of(
            search(&[&index.layer.index], &query, &filter, wanted),
            offset,
        );
        let matches = program::matches(&query, &found, repo.threads, |hit| {
            compare::worktree_document(&repo.git, &hit.path, &index.rules).ok()
        });
        (
            found,
            matches,
            WORKTREE.to_owned(),
            index.layer.unindexed.clone(),
        )
    } else {
        let commit = repo
            .git
            .resolve(&revision)
            .map_err(|error| said(error.into()))?;
        let (layer, rules) = commit_layer(repo, &commit).map_err(said)?;
        let found = page_of(search(&[&layer.index], &query, &filter, wanted), offset);
        let matches = program::matches(&query, &found, repo.threads, |hit| {
            let blob = layer.blobs[hit.doc].as_ref()?;
            compare::document(&repo.git, &repo.cache, blob, &rules).ok()
        });
        (
            found,
            matches,
            commit.as_hex().to_owned(),
            layer.unindexed.clone(),
        )
    };
    let results: Vec<Value> = matches
        .into_iter()
        .map(|found| {
            json!({
                "path": found.path,
                "title": found.title,
                "type": found.doc_type,
                "block": found.block,
                "headings": found.headings,
                "anchor": found.anchor,
                "snippet": snippet_text(&found.snippet),
            })
        })
        .collect();
    let skipped_shown = &skipped[..skipped.len().min(SKIPPED_SHOWN)];
    let total = found.total;
    let build = |shown: usize| {
        let mut answer = json!({
            "schema_version": 1,
            "query": text,
            "revision": revision,
            "commit": commit,
            "documents": found.documents,
            "total": total,
            "results": &results[..shown],
            "skipped": skipped_shown,
            "page": page(offset, shown, total),
        });
        if let Some(cut) = list_cut(args, "results", offset, shown, total) {
            answer["cut"] = cut;
        }
        answer
    };
    let sizes: Vec<usize> = results.iter().map(fit::size).collect();
    Ok(fit::most_that_fit(limit, &sizes, build).0)
}

/// The hits from number `offset` on.
fn page_of(mut found: Found, offset: usize) -> Found {
    found.hits.drain(..offset.min(found.hits.len()));
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_snippet_marks_its_matches() {
        let pieces = [
            Piece {
                text: "Keep each page for ".into(),
                hit: false,
            },
            Piece {
                text: "one hour".into(),
                hit: true,
            },
            Piece {
                text: ".".into(),
                hit: false,
            },
        ];
        assert_eq!(snippet_text(&pieces), "Keep each page for **one hour**.");
        assert_eq!(snippet_text(&[]), "");
    }

    #[test]
    fn a_page_drops_the_hits_before_its_offset() {
        let found = Found {
            documents: 9,
            total: 3,
            hits: Vec::new(),
        };
        assert_eq!(page_of(found.clone(), 5), found);
    }
}
