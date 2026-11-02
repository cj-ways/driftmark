//! `documents_status`: which documents are current, superseded, archived or
//! untouched for long, by type: the status view of `/api/lifecycle`
//! (`docs/features/status-page.md`).

use driftmark_core::status::{Filter, Stage};
use serde_json::{Value, json};

use super::fit;
use super::tools::{
    Args, Context, MAX_LIMIT, MAX_OFFSET, Tool, limit_schema, list_cut, offset_schema, page, said,
};
use crate::serve::address::{Revision, WORKTREE};
use crate::serve::lifecycle::{DocJson, HeadJson};

/// How many documents an answer lists when the call names no limit: about
/// 200 characters each, so the default fits in the default size.
const DEFAULT_LIMIT: usize = 50;

/// The most entries of `skipped` an answer lists.
const SKIPPED_SHOWN: usize = 20;

/// `documents_status`.
pub(crate) const STATUS: Tool = Tool {
    name: "documents_status",
    title: "Which documents are current",
    description: "Lists the Markdown documents at a revision with each one's type, lifecycle (current, superseded, archived or none), status, date, the document that superseded it, and how many days ago a commit last changed it. The counts per type and lifecycle cover every document, whatever the filters. Filter with `types`, `lifecycle`, `path` (a folder, such as docs/decisions/) and `untouched` (only documents no commit changed for the rules file's untouched-days, 180 by default). Use it before relying on a document, to check it is still current, and to find what replaced a superseded one.",
    arguments,
    required: &[],
    read_only: true,
    run,
};

fn arguments() -> Value {
    json!({
        "revision": {
            "type": "string",
            "description": "A branch, a tag, a commit or an expression such as main~2, or \"worktree\" for the files on disk now. Default \"worktree\".",
        },
        "types": {
            "type": "array",
            "items": {"type": "string"},
            "description": "Keep only documents of these types from the rules file, such as [\"decision\"]. A file no type matches has the type \"document\".",
        },
        "lifecycle": {
            "type": "string",
            "enum": ["current", "superseded", "archived", "none"],
            "description": "Keep only documents in this lifecycle; \"none\" keeps those whose status gives none.",
        },
        "path": {
            "type": "string",
            "description": "Keep only documents whose path starts with this text, such as docs/decisions/.",
        },
        "untouched": {
            "type": "boolean",
            "description": "Keep only documents no commit changed for the rules file's untouched-days or more. Default false.",
        },
        "offset": offset_schema("documents"),
        "limit": limit_schema("documents", DEFAULT_LIMIT, MAX_LIMIT),
    })
}

/// The seconds since 1970 now.
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
        })
}

fn run(context: &Context, args: &Args) -> Result<Value, String> {
    let limit = args.max_chars()?;
    let offset = args.number("offset", 0, MAX_OFFSET, 0)?;
    let most = args.number("limit", 1, MAX_LIMIT, DEFAULT_LIMIT)?;
    let stage = args
        .text("lifecycle")?
        .map(|name| {
            Stage::parse(&name).ok_or_else(|| {
                "`lifecycle` must be current, superseded, archived or none".to_owned()
            })
        })
        .transpose()?;
    let filter = Filter {
        types: args.texts("types")?,
        stage,
        folder: args.text("path")?.filter(|path| !path.is_empty()),
        untouched: args.flag("untouched", false)?,
    };
    let default = if context.workdir.is_some() {
        WORKTREE
    } else {
        "HEAD"
    };
    let label = args.text("revision")?.unwrap_or_else(|| default.to_owned());
    let revision = if label == WORKTREE {
        Revision::Worktree
    } else {
        Revision::Name(label.clone())
    };
    let now = now();
    // The connector has no watcher, so the working tree is read every time.
    let status = context.repo.status(&revision, now, None).map_err(said)?;
    let listed = status.view.listed(&filter);
    let total = listed.len();
    let documents: Vec<Value> = listed
        .iter()
        .skip(offset)
        .take(most)
        .map(|&index| serde_json::to_value(DocJson::of(&status.view, index)).unwrap_or_default())
        .collect();
    let head = serde_json::to_value(HeadJson::of(&status, &label, &filter, now))
        .map_err(|error| format!("the answer could not be written as JSON: {error}"))?;
    let skipped = &status.skipped[..status.skipped.len().min(SKIPPED_SHOWN)];
    let build = |shown: usize| {
        let mut answer = head.clone();
        answer["documents"] = json!(&documents[..shown]);
        answer["skipped"] = json!(skipped);
        answer["page"] = page(offset, shown, total);
        if let Some(cut) = list_cut(args, "documents", offset, shown, total) {
            answer["cut"] = cut;
        }
        answer
    };
    let sizes: Vec<usize> = documents.iter().map(fit::size).collect();
    Ok(fit::most_that_fit(limit, &sizes, build).0)
}
