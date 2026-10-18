//! One document's change list as HTML: one element per change, in reading
//! order, with what the page needs as attributes.
//!
//! The text comes from the change list, so it is plain text with word marks:
//! `<del>` for words only the old version has, `<ins>` for words only the new
//! one has. The structure is written out in `docs/features/engine/html.md`.

use std::fmt::Write as _;

use crate::diff::{
    BlockRef, Change, ChangeKind, DocumentChanges, FileState, Level, Mark, MarkOp, Side, Signal,
    Subject, View, WholeDocument,
};
use crate::document::{BlockKind, DiagramLanguage, TaskState};

use super::words;

/// Renders one document's changes as an `<ol class="changes">`.
///
/// `id_prefix` starts every `id` the list holds, so a page can show many
/// documents: a change is `<prefix>.<index>`, its place-holder
/// `<prefix>.<index>.from`. The `.` keeps every one apart from heading
/// anchors, which never hold one.
#[must_use]
pub fn changes(document: &DocumentChanges, id_prefix: &str) -> String {
    let mut out = String::new();
    out.push_str("<ol class=\"changes\" data-path=\"");
    push_escaped(&mut out, &document.path);
    let _ = writeln!(out, "\" data-file=\"{}\">", file_state(document.file));
    change_items(&mut out, document, id_prefix);
    out.push_str("</ol>\n");
    out
}

fn file_state(file: FileState) -> &'static str {
    match file {
        FileState::Added => "added",
        FileState::Removed => "removed",
        FileState::Modified => "modified",
    }
}

/// A change about one block, with its place on both sides.
struct BlockChange<'c> {
    /// Its index in the change list.
    index: usize,
    change: &'c Change,
    old: &'c Side,
    new: &'c Side,
}

/// The signals of a change, by their JSON names, separated by spaces.
fn signal_names(change: &Change) -> String {
    let names: Vec<&str> = change.signals.iter().copied().map(signal_name).collect();
    names.join(" ")
}

/// A new or deleted document is one item (decision 0009): its title,
/// summary and size, not its blocks.
fn document_item(
    out: &mut String,
    index: usize,
    change: &Change,
    whole: &WholeDocument,
    id_prefix: &str,
) {
    let _ = writeln!(
        out,
        "<li id=\"{id_prefix}.{index}\" class=\"change document\" data-kind=\"{}\" data-level=\"{}\" data-signals=\"{}\" data-bytes=\"{}\" data-lines=\"{}\">",
        kind_name(change.kind),
        level_name(change.level),
        signal_names(change),
        whole.bytes,
        whole.lines
    );
    head(out, change);
    for (class, text) in [("title", &whole.title), ("summary", &whole.summary)] {
        if let Some(text) = text {
            let _ = write!(out, "<p class=\"{class}\">");
            push_escaped(out, text);
            out.push_str("</p>\n");
        }
    }
    let unit = if whole.lines == 1 { "line" } else { "lines" };
    let _ = writeln!(
        out,
        "<p class=\"size\" data-gen>{} {unit}</p>\n</li>",
        whole.lines
    );
}

/// The changes of one document: a whole document as one item, and block
/// changes with a place-holder where each moved block left and runs of
/// formatting-only changes folded.
fn change_items(out: &mut String, document: &DocumentChanges, id_prefix: &str) {
    let mut blocks = Vec::new();
    for (index, change) in document.changes.iter().enumerate() {
        match &change.subject {
            Subject::Document { document: whole } => {
                document_item(out, index, change, whole, id_prefix);
            }
            Subject::Block { old, new } => blocks.push(BlockChange {
                index,
                change,
                old,
                new,
            }),
        }
    }
    let mut left = departures(&blocks, &document.unchanged);
    let mut next_left = 0;
    let mut at = 0;
    while at < blocks.len() {
        let block = &blocks[at];
        let key = order_key(block);
        // Keys never tie: a place-holder sorts with removed blocks, by an
        // old index no removed block shares.
        while let Some(departure) = left
            .get(next_left)
            .filter(|departure| departure.key.cmp(&key).is_lt())
        {
            departure_item(out, departure, id_prefix);
            next_left += 1;
        }
        if block.change.kind == ChangeKind::Formatting {
            let run = blocks[at..]
                .iter()
                .take_while(|block| block.change.kind == ChangeKind::Formatting)
                .count();
            let noun = if run == 1 { "change" } else { "changes" };
            let _ = write!(
                out,
                "<li class=\"fold\" data-count=\"{run}\"><details><summary data-gen>{run} formatting-only {noun}</summary>\n<ol class=\"changes\">\n"
            );
            for block in &blocks[at..at + run] {
                change_item(out, block, id_prefix);
            }
            out.push_str("</ol>\n</details></li>\n");
            at += run;
        } else {
            change_item(out, block, id_prefix);
            at += 1;
        }
    }
    for departure in left.drain(next_left..) {
        departure_item(out, &departure, id_prefix);
    }
}

/// Where a moved block left: its change, the line it arrived at, and where
/// in the new version's reading order the place-holder goes.
struct Departure {
    index: usize,
    line: usize,
    key: (usize, u8, usize),
}

/// A change's place in reading order. A removed block sorts before the new
/// side's blocks of the same place, as the change list lists them.
fn order_key(block: &BlockChange<'_>) -> (usize, u8, usize) {
    if block.change.kind == ChangeKind::Removed {
        (block.new.at, 0, block.old.at)
    } else {
        (block.new.at, 1, block.index)
    }
}

/// The place-holders of the moved blocks, in reading order. A block left
/// from just after the last block before it that stayed in place, as a
/// removed block is listed.
fn departures(blocks: &[BlockChange<'_>], unchanged: &[[usize; 2]]) -> Vec<Departure> {
    let mut in_place: Vec<[usize; 2]> = unchanged.to_vec();
    in_place.extend(
        blocks
            .iter()
            .filter(|block| {
                matches!(
                    block.change.kind,
                    ChangeKind::Edited | ChangeKind::Formatting
                )
            })
            .map(|block| [block.old.at, block.new.at]),
    );
    in_place.sort_unstable();
    let mut found: Vec<Departure> = blocks
        .iter()
        .filter(|block| {
            matches!(
                block.change.kind,
                ChangeKind::Moved | ChangeKind::MovedEdited
            )
        })
        .map(|block| {
            // A moved block is never in place, so no pair has its old index.
            let before = in_place.partition_point(|pair| pair[0].cmp(&block.old.at).is_lt());
            let at = before
                .checked_sub(1)
                .map_or(0, |last| in_place[last][1] + 1);
            Departure {
                index: block.index,
                // A moved block is on both sides, so its new line is known.
                line: block.new.block.as_ref().map_or(0, |found| found.line),
                key: (at, 0, block.old.at),
            }
        })
        .collect();
    found.sort_by_key(|departure| departure.key);
    found
}

fn departure_item(out: &mut String, departure: &Departure, id_prefix: &str) {
    let index = departure.index;
    let _ = writeln!(
        out,
        "<li id=\"{id_prefix}.{index}.from\" class=\"moved-from\" data-new=\"{}\"><a href=\"#{id_prefix}.{index}\" data-gen>Moved to new line {}</a></li>",
        departure.key.0, departure.line
    );
}

/// One block change as an `<li class="change">`.
fn change_item(out: &mut String, block: &BlockChange<'_>, id_prefix: &str) {
    let BlockChange {
        index,
        change,
        old,
        new,
    } = *block;
    let shown = new.block.as_ref().or(old.block.as_ref());
    let _ = write!(
        out,
        "<li id=\"{id_prefix}.{index}\" class=\"change\" data-kind=\"{}\" data-level=\"{}\" data-signals=\"{}\" data-old=\"{}\" data-new=\"{}\"",
        kind_name(change.kind),
        level_name(change.level),
        signal_names(change),
        old.at,
        new.at
    );
    line_attribute(out, "data-old-line", old);
    line_attribute(out, "data-new-line", new);
    if let Some(edit) = &change.edit {
        let _ = write!(out, " data-view=\"{}\"", view_name(edit.view));
        if edit.fallback {
            out.push_str(" data-fallback");
        }
        if !edit.cells.is_empty() {
            let cells: Vec<String> = edit.cells.iter().map(ToString::to_string).collect();
            let _ = write!(out, " data-cells=\"{}\"", cells.join(" "));
        }
    }
    let new_details = new.block.as_ref().map(|found| details(&found.kind));
    let old_details = old.block.as_ref().map(|found| details(&found.kind));
    if let Some(found) = shown {
        let _ = write!(out, " data-block=\"{}\"", block_type(&found.kind));
        for (name, value) in new_details
            .as_ref()
            .or(old_details.as_ref())
            .into_iter()
            .flatten()
        {
            let _ = write!(out, " data-{name}=\"");
            push_escaped(out, value);
            out.push('"');
        }
    }
    if let (Some(old), Some(new)) = (&old_details, &new_details) {
        for (name, old_value) in old_changes(old, new) {
            let _ = write!(out, " data-old-{name}=\"");
            push_escaped(out, &old_value);
            out.push('"');
        }
    }
    out.push_str(">\n");
    head(out, change);
    if let Some(found) = shown
        && !found.heading_path.is_empty()
    {
        out.push_str("<ol class=\"path\">");
        for heading in found.heading_path.iter() {
            out.push_str("<li>");
            push_escaped(out, heading);
            out.push_str("</li>");
        }
        out.push_str("</ol>\n");
    }
    if matches!(change.kind, ChangeKind::Moved | ChangeKind::MovedEdited) {
        let line = old.block.as_ref().map_or(0, |found| found.line);
        let _ = writeln!(
            out,
            "<p class=\"moved\"><a href=\"#{id_prefix}.{index}.from\" data-gen>Moved here from old line {line}</a></p>"
        );
    }
    content(out, change, old.block.as_ref(), new.block.as_ref());
    out.push_str("</li>\n");
}

/// The line on top of a change: its level, what happened, how it is drawn
/// and why it ranks there, in words, then the places for "Seen" and "Add a
/// note". Those two arrive with notes in M3 (decision 0004), so they are
/// inactive buttons until then.
fn head(out: &mut String, change: &Change) {
    let _ = write!(
        out,
        "<div class=\"head\" data-gen><span class=\"level\" data-level=\"{}\">{}</span> <span class=\"kind\">{}</span>",
        level_name(change.level),
        words::level(change.level),
        words::kind(change)
    );
    if let Some(edit) = &change.edit {
        let view = if edit.fallback {
            "Both versions whole"
        } else {
            words::view(edit.view)
        };
        let _ = write!(out, " <span class=\"view\">{view}</span>");
    }
    let _ = writeln!(
        out,
        " <span class=\"why\">{}</span> <span class=\"acts\"><button type=\"button\" class=\"seen\" disabled title=\"Arrives with notes\">Seen</button> <button type=\"button\" class=\"note\" disabled title=\"Arrives with notes\">Add a note</button></span></div>",
        words::signals(change)
    );
}

fn line_attribute(out: &mut String, name: &str, side: &Side) {
    if let Some(block) = &side.block {
        let _ = write!(out, " {name}=\"{}\"", block.line);
    }
}

/// The text of a change, drawn the way its view says.
fn content(out: &mut String, change: &Change, old: Option<&BlockRef>, new: Option<&BlockRef>) {
    let code = new.or(old).is_some_and(|block| {
        matches!(
            block.kind,
            BlockKind::Code { .. } | BlockKind::Diagram { .. }
        )
    });
    match (&change.edit, change.kind) {
        (Some(edit), _) if edit.view == View::Lines => lines(out, &edit.lines, new),
        (Some(edit), _) if edit.view == View::Inline => {
            side(out, "both", |out| marks(out, &edit.marks, MarkOp::Kept));
        }
        (Some(edit), _) if edit.view == View::Sentences => {
            out.push_str("<div class=\"rows\">\n");
            for row in &edit.sentences {
                sentence_row(out, row);
            }
            out.push_str("</div>\n");
        }
        (Some(edit), _) if edit.view == View::SideBySide && !edit.marks.is_empty() => {
            side(out, "old", |out| marks(out, &edit.marks, MarkOp::Deleted));
            side(out, "new", |out| marks(out, &edit.marks, MarkOp::Inserted));
        }
        (Some(_), _) => {
            whole(out, "old", old, None, code);
            whole(out, "new", new, None, code);
        }
        (None, ChangeKind::Added) => whole(out, "new", new, Some("ins"), code),
        (None, ChangeKind::Removed) => whole(out, "old", old, Some("del"), code),
        (None, _) => whole(out, "new", new, None, code),
    }
}

/// A `<div class="side">` around what `fill` writes.
fn side(out: &mut String, which: &str, fill: impl FnOnce(&mut String)) {
    let _ = write!(out, "<div class=\"side\" data-side=\"{which}\">");
    fill(out);
    out.push_str("</div>\n");
}

/// One version of a block, whole, in `<del>` or `<ins>` when the whole
/// block came or went. Code keeps its lines in `<pre>`.
fn whole(out: &mut String, which: &str, block: Option<&BlockRef>, mark: Option<&str>, code: bool) {
    let text = block.map_or("", |block| block.text.as_str());
    side(out, which, |out| {
        if code {
            out.push_str("<pre><code>");
        }
        if let Some(mark) = mark {
            let _ = write!(out, "<{mark}>");
        }
        push_escaped(out, text);
        if let Some(mark) = mark {
            let _ = write!(out, "</{mark}>");
        }
        if code {
            out.push_str("</code></pre>");
        }
    });
}

/// Writes word marks for one side: kept marks, and the marks of `op`. With
/// `MarkOp::Kept` it writes every mark, for the inline view.
fn marks(out: &mut String, marks: &[Mark], op: MarkOp) {
    for mark in marks {
        let tag = match mark.op {
            MarkOp::Kept => None,
            MarkOp::Deleted if op != MarkOp::Inserted => Some("del"),
            MarkOp::Inserted if op != MarkOp::Deleted => Some("ins"),
            _ => continue,
        };
        if let Some(tag) = tag {
            let _ = write!(out, "<{tag}>");
        }
        push_escaped(out, &mark.text);
        if let Some(tag) = tag {
            let _ = write!(out, "</{tag}>");
        }
    }
}

/// One sentence row: a kept sentence once, otherwise its old and new side.
fn sentence_row(out: &mut String, row: &[Mark]) {
    let has = |op: MarkOp| row.iter().any(|mark| mark.op == op);
    let (deleted, inserted) = (has(MarkOp::Deleted), has(MarkOp::Inserted));
    let state = match (deleted, inserted) {
        (false, false) => "kept",
        (true, false) => "deleted",
        (false, true) => "inserted",
        (true, true) => "edited",
    };
    let _ = write!(out, "<div class=\"row\" data-row=\"{state}\">");
    if state == "kept" {
        side(out, "both", |out| marks(out, row, MarkOp::Kept));
    } else {
        if deleted {
            side(out, "old", |out| marks(out, row, MarkOp::Deleted));
        }
        if inserted {
            side(out, "new", |out| marks(out, row, MarkOp::Inserted));
        }
    }
    out.push_str("</div>\n");
}

/// The lines view of code: one line per row, kept, deleted or inserted.
fn lines(out: &mut String, lines: &[Mark], new: Option<&BlockRef>) {
    out.push_str("<pre class=\"lines\"><code");
    if let Some(language) = new.and_then(|block| code_language(&block.kind)) {
        out.push_str(" class=\"language-");
        push_escaped(out, language);
        out.push('"');
    }
    out.push('>');
    for line in lines {
        match line.op {
            MarkOp::Kept => push_escaped(out, &line.text),
            MarkOp::Deleted => {
                out.push_str("<del>");
                push_escaped(out, &line.text);
                out.push_str("</del>");
            }
            MarkOp::Inserted => {
                out.push_str("<ins>");
                push_escaped(out, &line.text);
                out.push_str("</ins>");
            }
        }
        out.push('\n');
    }
    out.push_str("</code></pre>\n");
}

fn code_language(kind: &BlockKind) -> Option<&str> {
    match kind {
        BlockKind::Code { language, .. } => language.as_deref(),
        BlockKind::Diagram {
            language: DiagramLanguage::Mermaid,
            ..
        } => Some("mermaid"),
        BlockKind::Diagram {
            language: DiagramLanguage::D2,
            ..
        } => Some("d2"),
        _ => None,
    }
}

/// The JSON names of the change list's kinds, levels, signals and views,
/// as `change-list.md` lists them. A test checks them against the JSON.
fn kind_name(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Formatting => "formatting",
        ChangeKind::Edited => "edited",
        ChangeKind::Added => "added",
        ChangeKind::Removed => "removed",
        ChangeKind::Moved => "moved",
        ChangeKind::MovedEdited => "moved_edited",
    }
}

fn level_name(level: Level) -> &'static str {
    match level {
        Level::Decision => "decision",
        Level::Rule => "rule",
        Level::Fact => "fact",
        Level::Wording => "wording",
        Level::Formatting => "formatting",
    }
}

fn signal_name(signal: Signal) -> &'static str {
    match signal {
        Signal::Status => "status",
        Signal::DecisionSection => "decision_section",
        Signal::Normative => "normative",
        Signal::Heading => "heading",
        Signal::Number => "number",
        Signal::Link => "link",
        Signal::TableCell => "table_cell",
        Signal::CodeLine => "code_line",
        Signal::TaskState => "task_state",
        Signal::Moved => "moved",
        Signal::Wording => "wording",
        Signal::Formatting => "formatting",
    }
}

fn view_name(view: View) -> &'static str {
    match view {
        View::Inline => "inline",
        View::Sentences => "sentences",
        View::SideBySide => "side_by_side",
        View::Rewritten => "rewritten",
        View::Lines => "lines",
    }
}

/// The `type` of a block kind in JSON, such as `list_item`.
fn block_type(kind: &BlockKind) -> &'static str {
    match kind {
        BlockKind::FrontMatter => "front_matter",
        BlockKind::Heading { .. } => "heading",
        BlockKind::Paragraph => "paragraph",
        BlockKind::ListItem(_) => "list_item",
        BlockKind::TableRow { .. } => "table_row",
        BlockKind::Code { .. } => "code",
        BlockKind::Diagram { .. } => "diagram",
        BlockKind::Alert { .. } => "alert",
        BlockKind::FootnoteDefinition { .. } => "footnote_definition",
        BlockKind::Html => "html",
        BlockKind::ThematicBreak => "thematic_break",
        BlockKind::Unrendered => "unrendered",
    }
}

/// A block kind's details as attribute names and values, without the
/// `data-` in front.
fn details(kind: &BlockKind) -> Vec<(&'static str, String)> {
    let mut found = Vec::new();
    match kind {
        BlockKind::Heading { level } => found.push(("heading", level.to_string())),
        BlockKind::ListItem(item) => {
            found.push(("depth", item.depth.to_string()));
            if let Some(number) = item.number {
                found.push(("number", number.to_string()));
            }
            if let Some(task) = item.task {
                let state = if task == TaskState::Done {
                    "done"
                } else {
                    "open"
                };
                found.push(("task", state.to_owned()));
            }
        }
        BlockKind::TableRow { header: true, .. } => found.push(("header", String::new())),
        BlockKind::Code {
            language: Some(language),
            ..
        } => found.push(("language", language.clone())),
        BlockKind::Diagram { .. } => {
            found.push((
                "diagram",
                code_language(kind).unwrap_or_default().to_owned(),
            ));
        }
        BlockKind::Alert { kind, .. } => found.push(("alert", kind.name().to_ascii_lowercase())),
        BlockKind::FootnoteDefinition { name } => found.push(("footnote", name.clone())),
        _ => {}
    }
    found
}

/// The details whose value the old version had differently, with the old
/// value; an empty value when the old version did not have the detail.
fn old_changes(
    old: &[(&'static str, String)],
    new: &[(&'static str, String)],
) -> Vec<(&'static str, String)> {
    let value = |list: &[(&'static str, String)], name: &str| {
        list.iter()
            .find(|(found, _)| *found == name)
            .map(|(_, value)| value.clone())
    };
    let mut found: Vec<(&'static str, String)> = Vec::new();
    for (name, _) in old.iter().chain(new) {
        let before = value(old, name);
        if before != value(new, name) && !found.iter().any(|(seen, _)| seen == name) {
            found.push((name, before.unwrap_or_default()));
        }
    }
    found
}

/// Escapes text for HTML: `&`, `<`, `>` and `"`.
fn push_escaped(out: &mut String, text: &str) {
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\0' => out.push('\u{fffd}'),
            '\r' => {}
            _ => out.push(character),
        }
    }
}

#[cfg(test)]
mod tests;
