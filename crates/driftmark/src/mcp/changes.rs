//! `changes_summary` and `changes_document`: what changed in a range.
//!
//! Both read the range the way `driftmark diff` and the server do
//! (`docs/features/engine/git-ranges.md`), and answer with the change list's
//! own JSON (`docs/features/engine/change-list.md`): the summary form for a
//! range, and the full form for one document.

use driftmark_core::diff::{RangeInfo, SummaryList};
use driftmark_core::git::{BaseBranch, RangeSpec, ResolvedRange};
use serde_json::{Value, json};

use super::fit;
use super::tools::{
    Args, Context, MAX_LIMIT, MAX_OFFSET, Tool, document_path, limit_schema, list_cut,
    offset_schema, page, said,
};
use crate::serve::address;
use crate::serve::repo::Problem;

/// How many documents `changes_summary` lists when the call names no limit.
/// Each costs about 140 tokens, so 25 are about 3,500.
const SUMMARY_LIMIT: usize = 25;

/// How many changes `changes_document` lists when the call names no limit.
const CHANGES_LIMIT: usize = 50;

/// The most entries of `skipped` an answer lists: files that could not be
/// compared, such as a file that is not UTF-8.
const SKIPPED_SHOWN: usize = 20;

/// `changes_summary`.
pub(crate) const SUMMARY: Tool = Tool {
    name: "changes_summary",
    title: "What changed in the docs",
    description: "Lists the Markdown documents that changed between two git points, ranked by meaning: each document's highest level (decision, rule, fact, wording, formatting) and its counts of changes per level and kind, highest level first. The default range \"auto\" is this branch's work, committed or not, against the default branch. Call changes_document for one document's changes. The answer is the JSON of `driftmark diff --json=summary`, plus `skipped`, `page`, and `cut` when cut.",
    arguments: summary_arguments,
    required: &[],
    read_only: true,
    run: summary,
};

/// `changes_document`.
pub(crate) const DOCUMENT: Tool = Tool {
    name: "changes_document",
    title: "One document's changes",
    description: "Shows how one Markdown document changed between two git points: every change with its kind, level, signals, where it is in both versions, the plain text of the blocks, and the changed words marked. A new or deleted document is one change with its title, summary, size and full text; set full_text false to leave the text out. The answer is the change-list JSON of `driftmark diff --json` for that document, without `unchanged`, plus `skipped`, `page`, and `cut` when cut.",
    arguments: document_arguments,
    required: &["path"],
    read_only: true,
    run: document,
};

fn range_schema() -> Value {
    json!({
        "type": "string",
        "description": "The two git points: \"auto\" (default: the files on disk against where this branch left the default branch), \"<base>..<head>\" (two tips), \"<base>...<head>\" (head against where it left base, as a merge request shows), \"since/<date>\" or \"since/<date>/<branch>\" (everything since a date such as 2026-10-01, on the current branch or the one named).",
    })
}

fn base_schema() -> Value {
    json!({
        "type": "string",
        "description": "Only with range \"auto\": the branch to compare with, such as origin/dev. Default: the rules file's [compare] default-branch, else the first of origin/main, main, origin/master, master, origin/dev, dev.",
    })
}

fn summary_arguments() -> Value {
    json!({
        "range": range_schema(),
        "base": base_schema(),
        "offset": offset_schema("documents"),
        "limit": limit_schema("documents", SUMMARY_LIMIT, MAX_LIMIT),
    })
}

fn document_arguments() -> Value {
    json!({
        "path": {
            "type": "string",
            "description": "The document, from the repository root, such as docs/guide.md.",
        },
        "range": range_schema(),
        "base": base_schema(),
        "full_text": {
            "type": "boolean",
            "description": "Whether a new or deleted document carries its full text. Default true; set false to save room and read it with document_read instead.",
        },
        "offset": offset_schema("changes"),
        "limit": limit_schema("changes", CHANGES_LIMIT, MAX_LIMIT),
    })
}

/// The range a call names, resolved to two points.
fn resolve(context: &Context, args: &Args) -> Result<ResolvedRange, String> {
    let label = args.text("range")?.unwrap_or_else(|| "auto".to_owned());
    let range = address::range_text(label).map_err(|bad| bad.0)?;
    match args.text("base")? {
        None => context.repo.resolve(&range).map_err(said),
        Some(base) if range.spec == RangeSpec::Auto => range
            .spec
            .resolve_with(&context.repo.git, &BaseBranch::Named(base))
            .map_err(|error| said(Problem::from(error))),
        Some(_) => Err(
            "`base` names the branch that `auto` compares with, so it goes only with the range \"auto\""
                .to_owned(),
        ),
    }
}

/// At most [`SKIPPED_SHOWN`] skipped files, and a sentence when there were
/// more.
fn skipped_part(skipped: &[crate::serve::compare::Skipped]) -> (Value, Option<String>) {
    let shown = &skipped[..skipped.len().min(SKIPPED_SHOWN)];
    let more = (skipped.len() > shown.len()).then(|| {
        format!(
            "only the first {SKIPPED_SHOWN} of {} files that could not be compared are listed in `skipped`",
            skipped.len()
        )
    });
    (json!(shown), more)
}

/// Adds a sentence to an answer's `cut`, making the `cut` when it has none.
fn also_cut(answer: &mut Value, sentence: Option<String>) {
    let Some(sentence) = sentence else {
        return;
    };
    match answer.get_mut("cut") {
        Some(cut) => {
            let reason = cut["reason"].as_str().unwrap_or_default();
            cut["reason"] = json!(format!("{reason}; {sentence}"));
        }
        None => answer["cut"] = fit::cut(&sentence, None),
    }
}

fn summary(context: &Context, args: &Args) -> Result<Value, String> {
    let limit = args.max_chars()?;
    let offset = args.number("offset", 0, MAX_OFFSET, 0)?;
    let most = args.number("limit", 1, MAX_LIMIT, SUMMARY_LIMIT)?;
    let resolved = resolve(context, args)?;
    let count = context.repo.range_count(&resolved).map_err(said)?;
    let info = RangeInfo::of(&resolved);
    let list = SummaryList {
        range: Some(&info),
        ..count.list.summary_list()
    };
    let total = list.documents.len();
    let start = offset.min(total);
    let documents: Vec<Value> = list.documents[start..total.min(start.saturating_add(most))]
        .iter()
        .map(|document| json!(document))
        .collect();
    let (skipped, more) = skipped_part(&count.skipped);
    let build = |shown: usize| {
        let mut answer = json!({
            "schema_version": list.schema_version,
            "range": list.range,
            "totals": list.totals,
            "documents": &documents[..shown],
            "skipped": skipped,
            "page": page(offset, shown, total),
        });
        if let Some(cut) = list_cut(args, "documents", offset, shown, total) {
            answer["cut"] = cut;
        }
        also_cut(&mut answer, more.clone());
        answer
    };
    let sizes: Vec<usize> = documents.iter().map(fit::size).collect();
    Ok(fit::most_that_fit(limit, &sizes, build).0)
}

fn document(context: &Context, args: &Args) -> Result<Value, String> {
    let limit = args.max_chars()?;
    let path = document_path(context, &args.text("path")?.unwrap_or_default())?;
    let full_text = args.flag("full_text", true)?;
    let offset = args.number("offset", 0, MAX_OFFSET, 0)?;
    let most = args.number("limit", 1, MAX_LIMIT, CHANGES_LIMIT)?;
    let resolved = resolve(context, args)?;
    let located = context.repo.locate(&resolved, &path).map_err(said)?;
    let compared = context
        .repo
        .document_changes(&resolved, located)
        .map_err(said)?;
    let mut list = json!(compared.list);
    let (skipped, _) = skipped_part(&compared.skipped);
    let Some(mut entry) = list["documents"].as_array_mut().and_then(Vec::pop) else {
        // The document could not be compared; `skipped` says why.
        list["skipped"] = skipped;
        return Ok(list);
    };
    // `unchanged` lines up the two versions on a page; an agent reads the
    // changes, and on a long document the pairs alone fill an answer.
    if let Some(entry) = entry.as_object_mut() {
        entry.remove("unchanged");
    }
    let mut changes = match entry.get_mut("changes").map(Value::take) {
        Some(Value::Array(changes)) => changes,
        _ => Vec::new(),
    };
    if !full_text {
        for change in &mut changes {
            if let Some(document) = change.get_mut("document").and_then(Value::as_object_mut) {
                document.remove("source");
            }
        }
    }
    let total = changes.len();
    let start = offset.min(total);
    let mut shown_changes: Vec<Value> = changes
        .drain(start..total.min(start.saturating_add(most)))
        .collect();
    let build = |shown_changes: &[Value], shown: usize, trimmed: bool| {
        let mut entry = entry.clone();
        entry["changes"] = json!(&shown_changes[..shown]);
        let mut answer = json!({
            "schema_version": list["schema_version"],
            "range": list["range"],
            "totals": list["totals"],
            "documents": [entry],
            "skipped": skipped,
            "page": page(offset, shown, total),
        });
        if let Some(cut) = list_cut(args, "changes", offset, shown, total) {
            answer["cut"] = cut;
        }
        if trimmed {
            also_cut(
                &mut answer,
                Some(format!(
                    "change number {offset} is too large to show whole, so its text was left out; read it with document_read: the whole document when it is new or deleted, otherwise the lines its `block` objects name"
                )),
            );
        }
        answer
    };
    let sizes: Vec<usize> = shown_changes.iter().map(fit::size).collect();
    let (answer, shown) = fit::most_that_fit(limit, &sizes, |n| build(&shown_changes, n, false));
    if shown > 0 || shown_changes.is_empty() {
        return Ok(answer);
    }
    // Not even the first change fits: show it without its text, so the
    // agent still learns what it is and where.
    trim(&mut shown_changes[0]);
    Ok(build(&shown_changes, 1, true))
}

/// Leaves out a change's texts: the blocks' plain text, the edit's marks,
/// sentences and lines, and a whole document's full text.
fn trim(change: &mut Value) {
    for side in ["old", "new"] {
        // `get_mut`, not indexing: indexing a missing side would add it.
        if let Some(block) = change
            .get_mut(side)
            .and_then(|side| side.get_mut("block"))
            .and_then(Value::as_object_mut)
        {
            block.remove("text");
        }
    }
    if let Some(change) = change.as_object_mut() {
        change.remove("edit");
    }
    if let Some(document) = change.get_mut("document").and_then(Value::as_object_mut) {
        document.remove("source");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trimming_leaves_where_a_change_is_but_not_its_text() {
        let mut edited = json!({
            "kind": "edited", "level": "fact", "signals": ["number"],
            "old": {"at": 1, "block": {"line": 3, "text": "old words"}},
            "new": {"at": 1, "block": {"line": 3, "text": "new words"}},
            "edit": {"view": "inline", "marks": []},
        });
        trim(&mut edited);
        assert_eq!(
            edited,
            json!({
                "kind": "edited", "level": "fact", "signals": ["number"],
                "old": {"at": 1, "block": {"line": 3}},
                "new": {"at": 1, "block": {"line": 3}},
            })
        );
        let mut added = json!({"kind": "added", "document": {"title": "T", "source": "# T\n"}});
        trim(&mut added);
        assert_eq!(added, json!({"kind": "added", "document": {"title": "T"}}));
        let mut removed =
            json!({"kind": "removed", "old": {"at": 0, "block": {"text": "x"}}, "new": {"at": 0}});
        trim(&mut removed);
        assert_eq!(
            removed,
            json!({"kind": "removed", "old": {"at": 0, "block": {}}, "new": {"at": 0}})
        );
    }

    #[test]
    fn a_sentence_joins_an_existing_cut_or_makes_one() {
        let mut answer = json!({"cut": {"reason": "first", "next": {"offset": 2}}});
        also_cut(&mut answer, Some("second".into()));
        assert_eq!(
            answer["cut"],
            json!({"reason": "first; second", "next": {"offset": 2}})
        );
        let mut plain = json!({});
        also_cut(&mut plain, None);
        assert_eq!(plain, json!({}));
        also_cut(&mut plain, Some("only".into()));
        assert_eq!(plain, json!({"cut": {"reason": "only"}}));
    }
}
