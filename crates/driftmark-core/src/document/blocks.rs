//! Splitting a parsed document into its flat list of blocks.
//!
//! The rules are written out in `docs/features/engine/document-model.md`. In
//! short: every leaf (paragraph, heading, code, HTML, rule, table row, front
//! matter) is a block. A list item, an alert or a footnote definition is one
//! block holding its leading paragraphs; anything after them is a block of its
//! own inside that container. A block quote is not a block; its contents are.
//! Source text that renders nothing and belongs to no block becomes an
//! "unrendered" block, so no edit can hide between blocks.

use std::ops::Range;
use std::sync::Arc;

use comrak::nodes::{AlertType, AstNode, ListType, NodeCodeBlock, NodeValue};

use super::content::{
    Canon, front_matter_body, hash, normalize_lines, strip_html, tag, unify_line_endings,
};
use super::markdown::Positions;
use super::{
    AlertKind, Block, BlockKind, Container, DiagramLanguage, ListItem, MAX_DEPTH, ParseError,
    TaskState, TextSpan,
};

/// One node still to visit, with what it sits inside.
struct Work<'a> {
    node: &'a AstNode<'a>,
    containers: Arc<[Container]>,
    parent: Option<usize>,
    /// The rendered number of an item of an ordered list.
    number: Option<usize>,
}

struct Splitter<'t> {
    source: &'t str,
    positions: Positions<'t>,
    blocks: Vec<Block>,
    /// One empty heading path that every block shares until the heading
    /// paths are filled in, so no block allocates one.
    no_headings: Arc<[String]>,
}

/// Splits the tree comrak parsed from `source` into blocks, in source order.
///
/// The walk keeps its own stack, so a deeply nested document cannot overflow
/// the thread's stack; nesting deeper than [`MAX_DEPTH`] is an error instead.
pub(crate) fn split<'a>(source: &str, root: &'a AstNode<'a>) -> Result<Vec<Block>, ParseError> {
    let mut splitter = Splitter {
        source,
        positions: Positions::new(source),
        blocks: Vec::new(),
        no_headings: Arc::from([]),
    };
    let mut stack = vec![Work {
        node: root,
        containers: Arc::from([]),
        parent: None,
        number: None,
    }];
    while let Some(work) = stack.pop() {
        splitter.visit(&work, &mut stack)?;
    }
    Ok(splitter.finish())
}

/// Text characters that end a line or pad it.
fn is_blank(character: char) -> bool {
    character.is_ascii_whitespace()
}

impl Splitter<'_> {
    fn visit<'a>(&mut self, work: &Work<'a>, stack: &mut Vec<Work<'a>>) -> Result<(), ParseError> {
        let node = work.node;
        let data = node.data();
        match &data.value {
            NodeValue::FrontMatter(_) => {
                let range = self.leaf_range(node)?;
                let body = front_matter_body(&self.source[range.clone()]);
                let mut canon = Canon::new(tag::FRONT_MATTER, "", range.len());
                canon.mark(tag::BODY, &normalize_lines(body));
                let text = unify_line_endings(body).into_owned();
                self.push(BlockKind::FrontMatter, range, work, text, &canon);
            }
            NodeValue::Heading(heading) => {
                let kind = BlockKind::Heading {
                    level: heading.level,
                };
                self.prose(kind, tag::HEADING, &heading.level.to_string(), work)?;
            }
            NodeValue::Paragraph => self.prose(BlockKind::Paragraph, tag::PARAGRAPH, "", work)?,
            NodeValue::CodeBlock(code) => self.code_block(code, work)?,
            NodeValue::HtmlBlock(html) => {
                let range = self.leaf_range(node)?;
                let mut canon = Canon::new(tag::HTML, "", range.len());
                canon.mark(tag::BODY, &normalize_lines(&html.literal));
                let text = strip_html(&html.literal);
                self.push(BlockKind::Html, range, work, text, &canon);
            }
            NodeValue::ThematicBreak => {
                let range = self.leaf_range(node)?;
                let canon = Canon::new(tag::THEMATIC_BREAK, "", 0);
                self.push(BlockKind::ThematicBreak, range, work, String::new(), &canon);
            }
            NodeValue::Table(_) => self.table(node, work)?,
            NodeValue::BlockQuote => {
                let containers = deeper(work, Container::BlockQuote)?;
                for child in node.reverse_children() {
                    stack.push(Work {
                        node: child,
                        containers: containers.clone(),
                        parent: work.parent,
                        number: None,
                    });
                }
            }
            NodeValue::List(list) => {
                let ordered = list.list_type == ListType::Ordered;
                let items: Vec<_> = node.children().collect();
                for (index, item) in items.into_iter().enumerate().rev() {
                    stack.push(Work {
                        node: item,
                        containers: work.containers.clone(),
                        parent: work.parent,
                        number: ordered.then_some(list.start + index),
                    });
                }
            }
            NodeValue::Item(_) => self.list_item(work, None, stack)?,
            NodeValue::TaskItem(task) => {
                let state = if task.symbol.is_some() {
                    TaskState::Done
                } else {
                    TaskState::Open
                };
                self.list_item(work, Some(state), stack)?;
            }
            NodeValue::Alert(alert) => {
                let kind = AlertKind::from(alert.alert_type);
                // A title is one line, so the line feed cannot be part of it.
                let details = format!("{}\n{}", kind.name(), alert.title.as_deref().unwrap_or(""));
                let block = BlockKind::Alert {
                    kind,
                    title: alert.title.clone(),
                };
                self.item_like(work, block, tag::ALERT, &details, Container::Alert, stack)?;
            }
            NodeValue::FootnoteDefinition(definition) => {
                let block = BlockKind::FootnoteDefinition {
                    name: definition.name.clone(),
                };
                let container = Container::FootnoteDefinition;
                self.item_like(
                    work,
                    block,
                    tag::FOOTNOTE_DEFINITION,
                    &definition.name,
                    container,
                    stack,
                )?;
            }
            // The document itself. No other container can occur with
            // driftmark's parser options.
            _ => {
                for child in node.reverse_children() {
                    stack.push(Work {
                        node: child,
                        containers: work.containers.clone(),
                        parent: work.parent,
                        number: None,
                    });
                }
            }
        }
        Ok(())
    }

    /// A code block, or a diagram when its fence names Mermaid or D2.
    fn code_block(&mut self, code: &NodeCodeBlock, work: &Work<'_>) -> Result<(), ParseError> {
        let range = self.leaf_range(work.node)?;
        let info = code.info.trim();
        let language = info.split_whitespace().next();
        // An indented code block has no fence to close.
        let closed = !code.fenced || code.closed;
        let (kind, kind_tag) = match language {
            Some("mermaid") => (
                BlockKind::Diagram {
                    language: DiagramLanguage::Mermaid,
                    closed,
                },
                tag::DIAGRAM,
            ),
            Some("d2") => (
                BlockKind::Diagram {
                    language: DiagramLanguage::D2,
                    closed,
                },
                tag::DIAGRAM,
            ),
            _ => (
                BlockKind::Code {
                    language: language.map(str::to_owned),
                    closed,
                },
                tag::CODE,
            ),
        };
        let mut canon = Canon::new(kind_tag, info, range.len());
        canon.mark(tag::BODY, &normalize_lines(&code.literal));
        let literal = unify_line_endings(&code.literal);
        let text = literal.strip_suffix('\n').unwrap_or(&literal).to_owned();
        self.push(kind, range, work, text, &canon);
        Ok(())
    }

    /// A heading or a paragraph: a leaf whose content is inline text.
    fn prose(
        &mut self,
        kind: BlockKind,
        kind_tag: u8,
        details: &str,
        work: &Work<'_>,
    ) -> Result<(), ParseError> {
        let range = self.leaf_range(work.node)?;
        let mut canon = Canon::new(kind_tag, details, range.len());
        let mut text = String::with_capacity(range.len());
        let mut spans = Vec::new();
        canon.inline(work.node, &mut text, &mut spans);
        let (text, spans) = trimmed(text, spans);
        self.push_spans(kind, range, work, text, spans, &canon);
        Ok(())
    }

    fn list_item<'a>(
        &mut self,
        work: &Work<'a>,
        task: Option<TaskState>,
        stack: &mut Vec<Work<'a>>,
    ) -> Result<(), ParseError> {
        let ordered = work.number.is_some();
        let depth = work
            .containers
            .iter()
            .filter(|&&container| container == Container::ListItem)
            .count();
        let details = match (ordered, task) {
            (false, None) => "bullet",
            (false, Some(TaskState::Open)) => "bullet open",
            (false, Some(TaskState::Done)) => "bullet done",
            (true, None) => "ordered",
            (true, Some(TaskState::Open)) => "ordered open",
            (true, Some(TaskState::Done)) => "ordered done",
        };
        let kind = BlockKind::ListItem(ListItem {
            depth,
            ordered,
            number: work.number,
            task,
        });
        self.item_like(
            work,
            kind,
            tag::LIST_ITEM,
            details,
            Container::ListItem,
            stack,
        )
    }

    /// A list item, an alert or a footnote definition: one block for the
    /// container and its leading paragraphs, then each later child inside it.
    fn item_like<'a>(
        &mut self,
        work: &Work<'a>,
        kind: BlockKind,
        kind_tag: u8,
        details: &str,
        container: Container,
        stack: &mut Vec<Work<'a>>,
    ) -> Result<(), ParseError> {
        let node = work.node;
        let whole = self.range(node)?;
        let start = whole.start;
        // Most items hold one paragraph and nothing else, so the children are
        // walked a few times rather than collected.
        let is_paragraph =
            |child: &&'a AstNode<'a>| matches!(child.data().value, NodeValue::Paragraph);
        let lead = node.children().take_while(is_paragraph).count();
        let rest = node.children().count() - lead;
        let end = match (node.children().take(lead).last(), node.first_child()) {
            (Some(last), _) => self.leaf_range(last)?.end,
            (None, Some(first)) => self.markup_end(start, self.range(first)?.start),
            (None, None) => self.markup_end(start, whole.end),
        };
        let size = (start..end).len();
        let mut canon = Canon::new(kind_tag, details, size);
        let mut text = String::with_capacity(size);
        let mut spans = Vec::new();
        for (index, paragraph) in node.children().take(lead).enumerate() {
            if index > 0 {
                text.push_str("\n\n");
                canon.boundary(tag::NEXT_PARAGRAPH);
            }
            canon.inline(paragraph, &mut text, &mut spans);
        }
        let (text, spans) = trimmed(text, spans);
        let index = self.push_spans(kind, start..end, work, text, spans, &canon);
        if rest > 0 {
            let containers = deeper(work, container)?;
            for child in node.reverse_children().take(rest) {
                stack.push(Work {
                    node: child,
                    containers: containers.clone(),
                    parent: Some(index),
                    number: None,
                });
            }
        }
        Ok(())
    }

    /// One block per table row. The header row's range runs on to the end of
    /// the delimiter row under it, so every byte of a table is in a block.
    fn table<'a>(&mut self, node: &'a AstNode<'a>, work: &Work<'a>) -> Result<(), ParseError> {
        for row in node.children() {
            let header = matches!(row.data().value, NodeValue::TableRow(true));
            let mut range = self.leaf_range(row)?;
            if header {
                let delimiter_line = row.data().sourcepos.end.line + 1;
                range.end = self.line_end(delimiter_line).unwrap_or(range.end);
            }
            let details = if header { "header" } else { "body" };
            let mut canon = Canon::new(tag::TABLE_ROW, details, range.len());
            let mut cells = Vec::new();
            let mut text = String::with_capacity(range.len());
            let mut spans = Vec::new();
            for cell in row.children() {
                canon.boundary(tag::CELL);
                let mut cell_text = String::new();
                let mut cell_spans = Vec::new();
                canon.inline(cell, &mut cell_text, &mut cell_spans);
                let (cell_text, cell_spans) = trimmed(cell_text, cell_spans);
                if !cells.is_empty() {
                    text.push('\t');
                }
                let offset = text.len();
                text.push_str(&cell_text);
                spans.extend(cell_spans.into_iter().map(|mut span| {
                    span.range = span.range.start + offset..span.range.end + offset;
                    span
                }));
                cells.push(cell_text);
            }
            self.push_spans(
                BlockKind::TableRow { header, cells },
                range,
                work,
                text,
                spans,
                &canon,
            );
        }
        Ok(())
    }

    /// The source range of a node, or the error that names its position.
    fn range<'a>(&self, node: &'a AstNode<'a>) -> Result<Range<usize>, ParseError> {
        self.positions.range(node).ok_or_else(|| {
            let start = node.data().sourcepos.start;
            ParseError::BadPosition {
                line: start.line,
                column: start.column,
            }
        })
    }

    /// The range of a leaf block, without blank characters at its end.
    fn leaf_range<'a>(&self, node: &'a AstNode<'a>) -> Result<Range<usize>, ParseError> {
        let range = self.range(node)?;
        let kept = self.source[range.clone()].trim_end_matches(is_blank).len();
        Ok(range.start..range.start + kept)
    }

    /// Where the markup of a container ends, given where its first child
    /// starts: blank characters, and lines holding only `>` quote markers,
    /// are dropped from the end.
    fn markup_end(&self, start: usize, end: usize) -> usize {
        let mut kept = self.source[start..end].trim_end_matches(is_blank);
        while let Some((before, last_line)) = kept.rsplit_once(['\n', '\r'])
            && last_line.chars().all(|c| c == '>' || is_blank(c))
        {
            kept = before.trim_end_matches(is_blank);
        }
        start + kept.len()
    }

    /// The end of the text on 1-based line `line`, before its line ending and
    /// any blank characters.
    fn line_end(&self, line: usize) -> Option<usize> {
        let lines = self.positions.lines();
        let start = lines.line_start(line)?;
        let next = lines.line_start(line + 1).unwrap_or(self.source.len());
        Some(start + self.source[start..next].trim_end_matches(is_blank).len())
    }

    fn push(
        &mut self,
        kind: BlockKind,
        range: Range<usize>,
        work: &Work<'_>,
        text: String,
        canon: &Canon,
    ) -> usize {
        self.push_spans(kind, range, work, text, Vec::new(), canon)
    }

    fn push_spans(
        &mut self,
        kind: BlockKind,
        range: Range<usize>,
        work: &Work<'_>,
        text: String,
        spans: Vec<TextSpan>,
        canon: &Canon,
    ) -> usize {
        let source_hash = hash(self.source[range.clone()].as_bytes());
        self.blocks.push(Block {
            kind,
            range,
            heading_path: self.no_headings.clone(),
            containers: work.containers.clone(),
            parent: work.parent,
            text,
            spans,
            source_hash,
            content_hash: canon.finish(),
        });
        self.blocks.len() - 1
    }

    /// Adds the unrendered blocks, puts every block in source order, and fills
    /// in the heading paths.
    fn finish(self) -> Vec<Block> {
        let Splitter {
            source,
            mut blocks,
            no_headings,
            ..
        } = self;
        // Blocks come out of the walk in source order, except footnote
        // definitions: comrak moves them to the end of its tree.
        let in_order = blocks.is_sorted_by_key(|block| block.range.start);
        let gaps = if in_order {
            unrendered(source, blocks.iter().map(|block| block.range.clone()))
        } else {
            let mut ranges: Vec<Range<usize>> =
                blocks.iter().map(|block| block.range.clone()).collect();
            ranges.sort_by_key(|range| range.start);
            unrendered(source, ranges.into_iter())
        };
        if !in_order || !gaps.is_empty() {
            for gap in gaps {
                let text = &source[gap.clone()];
                let mut canon = Canon::new(tag::UNRENDERED, "", text.len());
                canon.text(text);
                blocks.push(Block {
                    kind: BlockKind::Unrendered,
                    source_hash: hash(text.as_bytes()),
                    content_hash: canon.finish(),
                    text: unify_line_endings(text).into_owned(),
                    spans: Vec::new(),
                    range: gap,
                    heading_path: no_headings.clone(),
                    containers: Arc::from([]),
                    parent: None,
                });
            }
            // The sort is stable and no two blocks start at the same byte.
            let mut keyed: Vec<(usize, Block)> = blocks.into_iter().enumerate().collect();
            keyed.sort_by_key(|(_, block)| block.range.start);
            let mut new_index = vec![0; keyed.len()];
            for (position, (old, _)) in keyed.iter().enumerate() {
                new_index[*old] = position;
            }
            blocks = keyed.into_iter().map(|(_, block)| block).collect();
            for block in &mut blocks {
                block.parent = block.parent.map(|parent| new_index[parent]);
            }
        }
        let mut sections: Vec<(u8, String)> = Vec::new();
        let mut path = no_headings;
        for block in &mut blocks {
            if let BlockKind::Heading { level } = block.kind
                && block.containers.is_empty()
            {
                // A heading closes the sections at its level and below; its
                // own path is the headings that remain above it.
                if sections.last().is_some_and(|(above, _)| *above >= level) {
                    while sections.last().is_some_and(|(above, _)| *above >= level) {
                        sections.pop();
                    }
                    path = sections.iter().map(|(_, title)| title.clone()).collect();
                }
                block.heading_path = path.clone();
                sections.push((level, block.text.clone()));
                path = sections.iter().map(|(_, title)| title.clone()).collect();
            } else {
                block.heading_path = path.clone();
            }
        }
        blocks
    }
}

/// `text` without white space at either end, with `spans` moved to match. A
/// span reaching into the white space is cut at the new ends. The buffer is
/// reused when there is no white space to drop, which is nearly always.
fn trimmed(text: String, mut spans: Vec<TextSpan>) -> (String, Vec<TextSpan>) {
    let lead = text.len() - text.trim_start().len();
    let text = if text.starts_with(char::is_whitespace) || text.ends_with(char::is_whitespace) {
        text.trim().to_owned()
    } else {
        text
    };
    let length = text.len();
    for span in &mut spans {
        let start = span.range.start.saturating_sub(lead).min(length);
        let end = span.range.end.saturating_sub(lead).min(length);
        span.range = start..end;
    }
    (text, spans)
}

/// The containers a child of `work` sits in, or an error when that is deeper
/// than [`MAX_DEPTH`].
fn deeper(work: &Work<'_>, container: Container) -> Result<Arc<[Container]>, ParseError> {
    if work.containers.len() >= MAX_DEPTH {
        return Err(ParseError::TooDeep {
            line: work.node.data().sourcepos.start.line,
            limit: MAX_DEPTH,
        });
    }
    let mut containers = work.containers.to_vec();
    containers.push(container);
    Ok(Arc::from(containers))
}

/// The stretches of source between blocks that hold more than white space
/// and `>` quote markers: link reference definitions, and footnote
/// definitions nothing refers to. `ranges` are the blocks, sorted by start.
fn unrendered(source: &str, ranges: impl Iterator<Item = Range<usize>>) -> Vec<Range<usize>> {
    let mut gaps = Vec::new();
    let mut at = 0;
    for range in ranges.chain(std::iter::once(source.len()..source.len())) {
        gaps.extend(content_between(source, at, range.start.max(at)));
        at = at.max(range.end);
    }
    gaps
}

/// The range from the first to the last character of content between `start`
/// and `end`, if there is any. A byte order mark counts as markup.
fn content_between(source: &str, start: usize, end: usize) -> Option<Range<usize>> {
    let mut first = None;
    let mut last = start;
    let mut line_start = start;
    for line in source[start..end].split_inclusive(['\n', '\r']) {
        let markup = line.len()
            - line
                .trim_start_matches(|c: char| c == '>' || c == '\u{feff}' || is_blank(c))
                .len();
        let content = line[markup..].trim_end_matches(is_blank);
        if !content.is_empty() {
            first.get_or_insert(line_start + markup);
            last = line_start + markup + content.len();
        }
        line_start += line.len();
    }
    first.map(|first| first..last)
}

impl From<AlertType> for AlertKind {
    fn from(alert: AlertType) -> AlertKind {
        match alert {
            AlertType::Note => AlertKind::Note,
            AlertType::Tip => AlertKind::Tip,
            AlertType::Important => AlertKind::Important,
            AlertType::Warning => AlertKind::Warning,
            AlertType::Caution => AlertKind::Caution,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::markdown::options;
    use comrak::nodes::LineColumn;
    use comrak::{Arena, parse_document};

    fn blocks(source: &str) -> Vec<Block> {
        let arena = Arena::new();
        let root = parse_document(&arena, source, &options());
        split(source, root).expect("the test text splits")
    }

    fn sources(source: &str) -> Vec<&str> {
        blocks(source)
            .into_iter()
            .map(|block| &source[block.range])
            .collect()
    }

    #[test]
    fn markup_end_drops_blank_lines_and_quote_markers() {
        let source = "> [!NOTE] a->\n> \n>\n> - x\n";
        let splitter = Splitter {
            source,
            positions: Positions::new(source),
            blocks: Vec::new(),
            no_headings: Arc::from([]),
        };
        assert_eq!(&source[..splitter.markup_end(0, 21)], "> [!NOTE] a->");
        assert_eq!(splitter.markup_end(3, 3), 3);
        let first_line = "> >\n";
        let only = Splitter {
            source: first_line,
            positions: Positions::new(first_line),
            blocks: Vec::new(),
            no_headings: Arc::from([]),
        };
        assert_eq!(only.markup_end(0, 4), 3);
        let nested = "a\n> >\n";
        let splitter = Splitter {
            source: nested,
            positions: Positions::new(nested),
            blocks: Vec::new(),
            no_headings: Arc::from([]),
        };
        assert_eq!(splitter.markup_end(0, 6), 1);
        let content = "a\n>b";
        let splitter = Splitter {
            source: content,
            positions: Positions::new(content),
            blocks: Vec::new(),
            no_headings: Arc::from([]),
        };
        assert_eq!(splitter.markup_end(0, 4), 4);
    }

    #[test]
    fn line_end_skips_the_line_ending_and_trailing_blanks() {
        let source = "a\n|--| \r\nb";
        let splitter = Splitter {
            source,
            positions: Positions::new(source),
            blocks: Vec::new(),
            no_headings: Arc::from([]),
        };
        assert_eq!(splitter.line_end(2), Some(6));
        assert_eq!(splitter.line_end(3), Some(10));
        assert_eq!(splitter.line_end(4), None);
    }

    #[test]
    fn content_between_ignores_white_space_quote_markers_and_a_byte_order_mark() {
        let source = "\u{feff}\n> \n  [a]: x  \n>\n> [b]: y\n\n";
        let range = content_between(source, 0, source.len()).expect("content");
        assert_eq!(&source[range], "[a]: x  \n>\n> [b]: y");
        assert_eq!(content_between(source, 0, 6), None);
        assert_eq!(content_between("a\rb", 0, 3), Some(0..3));
    }

    #[test]
    fn unrendered_finds_content_between_and_after_blocks() {
        let source = "# A\n\n[x]: y\n\nB\n\n[z]: w\n";
        assert_eq!(
            unrendered(source, [0..3, 13..14].into_iter()),
            vec![5..11, 16..22]
        );
        assert_eq!(
            unrendered("ab", std::iter::once(0..2)),
            Vec::<Range<usize>>::new()
        );
        assert_eq!(
            unrendered("ab", [0..1, 0..2].into_iter()),
            Vec::<Range<usize>>::new()
        );
    }

    #[test]
    fn every_alert_kind_is_kept() {
        let kinds: Vec<AlertKind> = [
            AlertType::Note,
            AlertType::Tip,
            AlertType::Important,
            AlertType::Warning,
            AlertType::Caution,
        ]
        .into_iter()
        .map(AlertKind::from)
        .collect();
        assert_eq!(
            kinds,
            [
                AlertKind::Note,
                AlertKind::Tip,
                AlertKind::Important,
                AlertKind::Warning,
                AlertKind::Caution
            ]
        );
    }

    #[test]
    fn a_bad_position_from_comrak_is_an_error() {
        let source = "a\n";
        let arena = Arena::new();
        let root = parse_document(&arena, source, &options());
        let paragraph = root.first_child().expect("a paragraph");
        paragraph.data_mut().sourcepos.start = LineColumn { line: 4, column: 2 };
        assert_eq!(
            split(source, root).expect_err("a bad position"),
            ParseError::BadPosition { line: 4, column: 2 }
        );
    }

    #[test]
    fn nesting_deeper_than_the_limit_is_an_error() {
        let fits = format!("{} x\n", ">".repeat(MAX_DEPTH));
        assert_eq!(blocks(&fits)[0].containers.len(), MAX_DEPTH);
        let too_deep = format!("a\n\n{} x\n", ">".repeat(MAX_DEPTH + 1));
        let arena = Arena::new();
        let root = parse_document(&arena, &too_deep, &options());
        assert_eq!(
            split(&too_deep, root).expect_err("too deep"),
            ParseError::TooDeep {
                line: 3,
                limit: MAX_DEPTH
            }
        );
    }

    #[test]
    fn a_container_with_no_leading_paragraph_keeps_only_its_marker() {
        assert_eq!(
            sources(
                "- ```sh\n  x\n  ```\n-\n> [!NOTE]\n> - y\n\n[^n]:\n    ```\n    z\n    ```\n\nUse[^n].\n"
            ),
            vec![
                "-",
                "```sh\n  x\n  ```",
                "-",
                "> [!NOTE]",
                "- y",
                "[^n]:",
                "```\n    z\n    ```",
                "Use[^n]."
            ]
        );
    }

    #[test]
    fn a_footnote_definition_keeps_its_place_in_the_source() {
        let found = blocks("A[^n].\n\n[^n]: Note.\n\n- after\n");
        assert_eq!(
            found[1].kind,
            BlockKind::FootnoteDefinition {
                name: "n".to_owned()
            }
        );
        assert_eq!(
            found[2].kind,
            BlockKind::ListItem(ListItem {
                depth: 0,
                ordered: false,
                number: None,
                task: None
            })
        );
    }

    #[test]
    fn parents_point_at_the_container_block_after_sorting() {
        let found = blocks("A[^n].\n\n[^n]: Note.\n\n    - inner\n\n- outer\n  - nested\n");
        assert_eq!(found[2].parent, Some(1));
        assert_eq!(found[4].parent, Some(3));
        assert_eq!(found[3].parent, None);
    }

    #[test]
    fn a_bad_position_in_any_node_is_an_error_never_a_panic() {
        let source = concat!(
            include_str!("../../../../fixtures/block-kinds/input.md"),
            include_str!("../../../../fixtures/nesting/input.md"),
            "\n-\n\n> [!NOTE]\n"
        );
        let nodes = {
            let arena = Arena::new();
            parse_document(&arena, source, &options())
                .descendants()
                .count()
        };
        let bad = LineColumn {
            line: 9999,
            column: 1,
        };
        let mut errors = 0;
        // Each node in turn gets a position past the end of the file: first
        // at both ends, then at its end only.
        for (index, both_ends) in (0..nodes).flat_map(|index| [(index, true), (index, false)]) {
            let arena = Arena::new();
            let root = parse_document(&arena, source, &options());
            let node = root.descendants().nth(index).expect("the node");
            let start = node.data().sourcepos.start;
            node.data_mut().sourcepos.end = bad;
            if both_ends {
                node.data_mut().sourcepos.start = bad;
            }
            match split(source, root) {
                Ok(_) => {}
                Err(error) => {
                    let named = [bad, start].map(|position| ParseError::BadPosition {
                        line: position.line,
                        column: position.column,
                    });
                    assert!(named.contains(&error), "{error}");
                    errors += 1;
                }
            }
        }
        assert!(errors > 100, "{errors}");
    }

    #[test]
    fn list_nesting_counts_toward_the_depth_limit() {
        let nested = |levels: usize| -> String {
            (0..levels).fold(String::new(), |mut text, level| {
                text.push_str(&"  ".repeat(level));
                text.push_str("- item\n");
                text
            })
        };
        let fits = nested(MAX_DEPTH + 1);
        let deepest = blocks(&fits).pop().expect("blocks");
        assert_eq!(deepest.containers.len(), MAX_DEPTH);
        let too_deep = nested(MAX_DEPTH + 2);
        let arena = Arena::new();
        let root = parse_document(&arena, &too_deep, &options());
        assert_eq!(
            split(&too_deep, root).expect_err("too deep"),
            ParseError::TooDeep {
                line: MAX_DEPTH + 1,
                limit: MAX_DEPTH
            }
        );
    }

    #[test]
    fn ordered_task_items_keep_both_details() {
        let found = blocks("1. [ ] open\n2. [x] done\n");
        assert_eq!(
            found[0].kind,
            BlockKind::ListItem(ListItem {
                depth: 0,
                ordered: true,
                number: Some(1),
                task: Some(TaskState::Open)
            })
        );
        assert_eq!(
            found[1].kind,
            BlockKind::ListItem(ListItem {
                depth: 0,
                ordered: true,
                number: Some(2),
                task: Some(TaskState::Done)
            })
        );
        assert_ne!(
            found[0].content_hash,
            blocks("- [ ] open\n")[0].content_hash
        );
    }

    #[test]
    fn plain_text_has_no_white_space_at_its_ends() {
        let found = blocks("# Title&#32;\n\n&#32;Text\n");
        assert_eq!(found[0].text, "Title");
        assert_eq!(found[1].text, "Text");
        assert_eq!(trimmed("a b".to_owned(), Vec::new()).0, "a b");
    }

    #[test]
    fn spans_follow_the_trimmed_text() {
        let span = |range: Range<usize>| TextSpan {
            range,
            kind: crate::document::SpanKind::Code,
        };
        let (text, spans) = trimmed(
            "  ab cd  ".to_owned(),
            vec![span(0..4), span(5..7), span(6..9)],
        );
        assert_eq!(text, "ab cd");
        assert_eq!(spans, [span(0..2), span(3..5), span(4..5)]);
        let (text, spans) = trimmed("ab ".to_owned(), vec![span(1..3)]);
        assert_eq!(text, "ab");
        assert_eq!(spans, [span(1..2)]);
    }

    #[test]
    fn blocks_carry_their_spans() {
        let found = blocks(concat!(
            "<b> `x` and [y](y.md)</b> ![i](i.png)\n\n",
            "- `a`\n\n  [b](b.md)\n\n",
            "| `c` | [d](d.md) |\n|---|---|\n|  | `e` |\n",
        ));
        let spans = |index: usize| -> Vec<(String, String)> {
            let block = &found[index];
            block
                .spans
                .iter()
                .map(|span| {
                    let text = block.text[span.range.clone()].to_owned();
                    let kind = match &span.kind {
                        crate::document::SpanKind::Code => "code".to_owned(),
                        crate::document::SpanKind::Link { target }
                        | crate::document::SpanKind::Image { target } => target.clone(),
                    };
                    (text, kind)
                })
                .collect()
        };
        let pair = |text: &str, kind: &str| (text.to_owned(), kind.to_owned());
        assert_eq!(found[0].text, "x and y i");
        assert_eq!(
            spans(0),
            [pair("x", "code"), pair("y", "y.md"), pair("i", "i.png")]
        );
        assert_eq!(found[1].text, "a\n\nb");
        assert_eq!(spans(1), [pair("a", "code"), pair("b", "b.md")]);
        assert_eq!(found[2].text, "c\td");
        assert_eq!(spans(2), [pair("c", "code"), pair("d", "d.md")]);
        assert_eq!(found[3].text, "\te");
        assert_eq!(spans(3), [pair("e", "code")]);
        assert_eq!(
            found[1].kind,
            BlockKind::ListItem(ListItem {
                depth: 0,
                ordered: false,
                number: None,
                task: None,
            })
        );
    }

    /// The canonical form of each kind, written out. A change here changes
    /// every stored content hash, so it must be deliberate.
    #[test]
    fn content_forms_are_fixed() {
        let source = concat!(
            "---\na: 1\n---\n\n",
            "## Title\n\n",
            "- one\n\n  two\n\n",
            "1. [x] done\n\n",
            "> [!NOTE] Hi\n> three\n\n",
            "[^n]: four\n\n",
            "x[^n]\n\n",
            "| a | b |\n|---|---|\n| 1 | 2 |\n\n",
            "```sh\necho\n```\n\n",
            "```mermaid\nA-->B\n```\n\n",
            "<div>\nx\n</div>\n\n",
            "***\n\n",
            "[a]: b\n",
        );
        let found = blocks(source);
        let mut expected = Vec::new();
        let mut front = Canon::new(tag::FRONT_MATTER, "", 0);
        front.mark(tag::BODY, "a: 1");
        expected.push(front);
        let mut heading = Canon::new(tag::HEADING, "2", 0);
        heading.text("Title");
        expected.push(heading);
        let mut item = Canon::new(tag::LIST_ITEM, "bullet", 0);
        item.text("one");
        item.boundary(tag::NEXT_PARAGRAPH);
        item.text("two");
        expected.push(item);
        let mut task = Canon::new(tag::LIST_ITEM, "ordered done", 0);
        task.text("done");
        expected.push(task);
        let mut alert = Canon::new(tag::ALERT, "NOTE\nHi", 0);
        alert.text("three");
        expected.push(alert);
        let mut footnote = Canon::new(tag::FOOTNOTE_DEFINITION, "n", 0);
        footnote.text("four");
        expected.push(footnote);
        let mut paragraph = Canon::new(tag::PARAGRAPH, "", 0);
        paragraph.text("x");
        paragraph.mark(tag::FOOTNOTE_REFERENCE, "n");
        expected.push(paragraph);
        for (details, cells) in [("header", ["a", "b"]), ("body", ["1", "2"])] {
            let mut row = Canon::new(tag::TABLE_ROW, details, 0);
            for cell in cells {
                row.boundary(tag::CELL);
                row.text(cell);
            }
            expected.push(row);
        }
        let mut code = Canon::new(tag::CODE, "sh", 0);
        code.mark(tag::BODY, "echo");
        expected.push(code);
        let mut diagram = Canon::new(tag::DIAGRAM, "mermaid", 0);
        diagram.mark(tag::BODY, "A-->B");
        expected.push(diagram);
        let mut html = Canon::new(tag::HTML, "", 0);
        html.mark(tag::BODY, "<div>\nx\n</div>");
        expected.push(html);
        expected.push(Canon::new(tag::THEMATIC_BREAK, "", 0));
        let mut unrendered = Canon::new(tag::UNRENDERED, "", 0);
        unrendered.text("[a]: b");
        expected.push(unrendered);
        let actual: Vec<u64> = found.iter().map(|block| block.content_hash).collect();
        let expected: Vec<u64> = expected.iter().map(Canon::finish).collect();
        assert_eq!(actual, expected);
    }
}
