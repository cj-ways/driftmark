//! Reads reading HTML back the way a browser shows it, block by block, for
//! the check that a page's text matches the document model's text.
//!
//! The rules come from `docs/features/engine/html.md`, "Text on the page":
//! text inside `data-gen` is skipped, raw HTML shown as text
//! (`code.html`) and footnote numbers (`sup.footnote-ref`) are markup, and
//! an image counts as its alt text. It reads driftmark's own output only,
//! which always closes what it opens.

#![allow(dead_code, reason = "each test file uses part of this module")]

use std::collections::BTreeMap;

use driftmark_core::document::{Block, BlockKind, Document};

/// What one block's elements show.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Shown {
    /// The tag of the block's elements, such as `p`, `tr` or `pre`.
    pub tag: String,
    /// The text of each element, or of each cell for a table row.
    pub parts: Vec<String>,
}

struct Open {
    name: String,
    /// The block this element starts, if it has `data-k`.
    block: Option<usize>,
    /// Text inside is not shown as block text.
    skip: bool,
}

/// The text of every block that has an element on the page.
pub fn shown(html: &str) -> BTreeMap<usize, Shown> {
    let mut found: BTreeMap<usize, Shown> = BTreeMap::new();
    let mut stack: Vec<Open> = Vec::new();
    let mut rest = html;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('<') {
            let end = after.find('>').expect("a tag ends");
            let tag = &after[..end];
            rest = &after[end + 1..];
            if let Some(name) = tag.strip_prefix('/') {
                while let Some(open) = stack.pop() {
                    if open.name == name {
                        break;
                    }
                }
                continue;
            }
            let name = tag.split([' ', '\n']).next().unwrap_or_default().to_owned();
            let block = attribute(tag, "data-k").map(|k| k.parse().expect("a block index"));
            let class = attribute(tag, "class").unwrap_or_default();
            let skip = tag.contains(" data-gen")
                || (name == "code" && class == "html")
                || (name == "sup" && class == "footnote-ref")
                || name == "figure";
            let current = current_block(&stack);
            if name == "img" {
                if let Some(block) = current.filter(|_| !skipping(&stack)) {
                    push_text(
                        &mut found,
                        block,
                        &decode(&attribute(tag, "alt").unwrap_or_default()),
                    );
                }
                continue;
            }
            if matches!(name.as_str(), "br" | "input" | "hr" | "wbr") {
                continue;
            }
            if let Some(block) = block {
                let entry = found.entry(block).or_default();
                entry.tag.clone_from(&name);
                if name != "tr" {
                    entry.parts.push(String::new());
                }
            }
            if matches!(name.as_str(), "th" | "td")
                && let Some(block) = current
            {
                found.entry(block).or_default().parts.push(String::new());
            }
            stack.push(Open { name, block, skip });
        } else {
            let end = rest.find('<').unwrap_or(rest.len());
            let text = decode(&rest[..end]);
            rest = &rest[end..];
            if let Some(block) = current_block(&stack).filter(|_| !skipping(&stack)) {
                push_text(&mut found, block, &text);
            }
        }
    }
    found
}

fn current_block(stack: &[Open]) -> Option<usize> {
    stack.iter().rev().find_map(|open| open.block)
}

fn skipping(stack: &[Open]) -> bool {
    stack.iter().any(|open| open.skip)
}

fn push_text(found: &mut BTreeMap<usize, Shown>, block: usize, text: &str) {
    let entry = found.entry(block).or_default();
    if entry.parts.is_empty() {
        entry.parts.push(String::new());
    }
    entry.parts.last_mut().expect("a part").push_str(text);
}

/// The value of `name="…"` in a tag.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let start = tag.find(&format!(" {name}=\""))? + name.len() + 3;
    let end = tag[start..].find('"')? + start;
    Some(tag[start..end].to_owned())
}

/// Decodes the references comrak's writer makes.
fn decode(text: &str) -> String {
    text.replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&#x27;", "'")
        .replace("&amp;", "&")
}

/// White space as a browser draws it: runs of white space as one space,
/// none at the ends.
pub fn collapsed(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// What the page should show for a block, in the form [`compare`] checks.
fn expected(block: &Block) -> Vec<String> {
    match &block.kind {
        BlockKind::TableRow { cells, .. } => cells.iter().map(|cell| collapsed(cell)).collect(),
        BlockKind::Code { .. } | BlockKind::Diagram { .. } => vec![block.text.clone()],
        _ => vec![collapsed(&block.text)],
    }
}

/// The blocks whose page text differs from the document model's text, as
/// `(block, expected, shown)`.
pub fn compare(document: &Document, html: &str) -> Vec<(usize, Vec<String>, Vec<String>)> {
    let mut wrong = Vec::new();
    for (block, shown) in shown(html) {
        // A drawn diagram shows a picture, not its code.
        if shown.tag == "figure" {
            continue;
        }
        let model = &document.blocks[block];
        let got: Vec<String> = match &model.kind {
            BlockKind::TableRow { .. } => shown.parts.iter().map(|part| collapsed(part)).collect(),
            BlockKind::Code { .. } | BlockKind::Diagram { .. } => {
                let joined = shown.parts.concat();
                vec![joined.strip_suffix('\n').unwrap_or(&joined).to_owned()]
            }
            _ => vec![collapsed(&shown.parts.join("\n\n"))],
        };
        let want = expected(model);
        if got != want {
            wrong.push((block, want, got));
        }
    }
    wrong
}
