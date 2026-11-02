//! The reading page: one document as HTML, with the position tables notes
//! need.
//!
//! comrak's HTML writer walks the tree and writes the containers it already
//! writes well: block quotes, lists and emphasis. driftmark writes every other
//! node itself, so that each block that holds text gets its `data-k` marker,
//! every piece of text lands in the block's position table, links and images
//! pass the address rules, and raw HTML passes the allow-list.
//! `docs/features/engine/html.md` states the structure in full.

use std::fmt::Write as _;

use comrak::html::{self, ChildRendering, Context};
use comrak::nodes::{
    AlertType, AstNode, NodeAlert, NodeCodeBlock, NodeFootnoteDefinition, NodeFootnoteReference,
    NodeLink, NodeValue, TableAlignment,
};
use comrak::options::Plugins;
use comrak::{Arena, parse_document};

use super::html::{self as raw, Allowed, Place, Token, Value};
use super::pieces::{Kind, Mapper, Piece};
use super::url::{Address, classify, decode_attribute};
use super::{Diagram, LinkKind, PositionTables, Reading, Resolved, Section, Setup};
use crate::document::markdown::{code_span_text, inline_text, options};
use crate::document::{AlertKind, BlockKind, DiagramLanguage, Document};
use crate::lint::Anchors;

type Node<'a> = &'a AstNode<'a>;

/// Renders one document as reading HTML.
///
/// The document is parsed again with the same options, so its blocks and the
/// tree line up. Nothing here fails: a node whose source position cannot be
/// read is still shown, only without a place in the position table.
#[must_use]
pub fn reading(document: &Document, setup: &Setup<'_>) -> Reading {
    reading_prefixed(document, setup, "")
}

/// Renders one document as reading HTML for a page that holds several
/// documents, such as a one-file export.
///
/// Every id the document's HTML carries starts with `prefix`: its headings'
/// ids, which the outline repeats, and its footnotes'. So does every link to
/// a place on the same page, `#install` as much as a footnote's number, so
/// it still leads to this document's own heading. Two documents with a
/// `## Install` section can then sit on one page without taking each other's
/// ids. A drawn diagram keeps the ids its [`super::DiagramRender`] gave it.
///
/// The prefix is written as it is, so it should be letters, digits and `-`,
/// and hold a character no heading anchor has, such as a capital letter.
#[must_use]
pub fn reading_prefixed(document: &Document, setup: &Setup<'_>, prefix: &str) -> Reading {
    let arena = Arena::new();
    let options = options();
    let root = parse_document(&arena, &document.source, &options);
    let ids: Vec<Option<String>> = Anchors::by_block(document)
        .into_iter()
        .map(|id| id.map(|id| format!("{prefix}{id}")))
        .collect();
    let outline = outline(document, &ids);
    let mut html = String::with_capacity(document.source.len() * 2);
    let positions = render(root, &mut html, document, setup, ids, prefix);
    Reading {
        html,
        positions,
        outline,
    }
}

/// Writes the tree under `root` into `html` and returns the position tables.
/// `ids` already carry `prefix`; footnotes and links on the same page get it
/// as they are written.
fn render(
    root: Node<'_>,
    html: &mut String,
    document: &Document,
    setup: &Setup<'_>,
    ids: Vec<Option<String>>,
    prefix: &str,
) -> PositionTables {
    let state = State {
        document,
        setup,
        mapper: Mapper::new(&document.source),
        allowed: Allowed::new(setup.allowed_html),
        prefix,
        ids,
        tables: vec![Vec::new(); document.blocks.len()],
        cursor: None,
        resume: None,
        closers: Vec::new(),
        paragraph: None,
        heading_id: None,
        inline_open: Vec::new(),
        flow_open: Vec::new(),
        footnotes: 0,
        backref: None,
        table: TableState::default(),
    };
    let options = options();
    html::format_document_with_formatter(root, &options, html, &Plugins::default(), format, state)
        .map(State::finish)
        .unwrap_or_default()
}

/// The headings outside any container, in order, for an "on this page"
/// list.
fn outline(document: &Document, ids: &[Option<String>]) -> Vec<Section> {
    document
        .blocks
        .iter()
        .zip(ids)
        .enumerate()
        .filter_map(|(block, (found, id))| match (&found.kind, id) {
            (BlockKind::Heading { level }, Some(id)) if found.containers.is_empty() => {
                Some(Section {
                    level: *level,
                    id: id.clone(),
                    text: found.text.clone(),
                    block,
                })
            }
            _ => None,
        })
        .collect()
}

/// Where the page is in a block that holds text: the block, the rendered
/// UTF-16 units written since the last piece, and where the last piece
/// ended in the source.
#[derive(Debug, Clone, Copy)]
struct Cursor {
    block: usize,
    gap: usize,
    last_source: usize,
}

/// The footnote back links still to write for the definition being
/// written.
struct Backref {
    name: String,
    references: u32,
    number: u32,
}

/// The table being written.
#[derive(Default)]
struct TableState {
    alignments: Vec<TableAlignment>,
    header: bool,
    cell: usize,
    body_open: bool,
    /// The header row's cell texts. Each body cell carries its column's in
    /// `data-label`, so a phone can show a row as a card whose values each
    /// say which column they are.
    labels: Vec<String>,
}

struct State<'r> {
    document: &'r Document,
    setup: &'r Setup<'r>,
    mapper: Mapper<'r>,
    allowed: Allowed,
    /// What every id on the page starts with (`reading_prefixed`).
    prefix: &'r str,
    /// Heading ids by block, with the prefix.
    ids: Vec<Option<String>>,
    /// The position table of each block.
    tables: Vec<Vec<i64>>,
    /// The block whose element is open, if any.
    cursor: Option<Cursor>,
    /// The block whose element was closed last, so a block written as two
    /// elements (two paragraphs of one list item) continues its table.
    resume: Option<Cursor>,
    /// The closing tags of open links.
    closers: Vec<&'static str>,
    /// The tag of the open paragraph.
    paragraph: Option<&'static str>,
    /// The id of the open heading.
    heading_id: Option<String>,
    /// Raw HTML elements open inside the current paragraph, heading or cell.
    /// Those never nest, so the list empties where each one ends.
    inline_open: Vec<&'static str>,
    /// Raw HTML elements opened by HTML blocks, with the container depth
    /// they were opened at.
    flow_open: Vec<(&'static str, usize)>,
    /// Footnote definitions written so far.
    footnotes: u32,
    backref: Option<Backref>,
    table: TableState,
}

impl State<'_> {
    fn finish(self) -> PositionTables {
        PositionTables {
            blocks: self.tables,
        }
    }

    /// The block that starts where `node` starts.
    fn block_at(&self, node: Node<'_>) -> Option<usize> {
        let start = self.mapper.range(node)?.start;
        self.document
            .blocks
            .binary_search_by_key(&start, |block| block.range.start)
            .ok()
    }

    /// The block a paragraph's text belongs to: its own, or, for the leading
    /// paragraphs of a list item, an alert or a footnote, the container's.
    fn block_of_paragraph(&self, node: Node<'_>) -> Option<usize> {
        let leading = node
            .preceding_siblings()
            .skip(1)
            .all(|sibling| matches!(sibling.data().value, NodeValue::Paragraph));
        let owner = node
            .parent()
            .filter(|parent| leading && is_item_like(parent))
            .unwrap_or(node);
        self.block_at(owner)
    }

    fn open_leaf(&mut self, block: usize) {
        let cursor = match self.resume.take() {
            Some(cursor) if cursor.block == block => cursor,
            _ => {
                let start = self.document.blocks[block].range.start;
                self.tables[block] = vec![number(start)];
                Cursor {
                    block,
                    gap: 0,
                    last_source: start,
                }
            }
        };
        self.cursor = Some(cursor);
    }

    fn close_leaf(&mut self) {
        self.resume = self.cursor.take();
    }

    /// Adds a piece, `rendered` UTF-16 units long, to the open block's table.
    fn note(&mut self, piece: &Piece, rendered: usize) {
        if let Some(cursor) = &mut self.cursor {
            let atomic = piece.kind != Kind::Verbatim;
            let table = &mut self.tables[cursor.block];
            table.push(number(cursor.gap));
            table.push(number(rendered * 2 + usize::from(atomic)));
            table.push(number(piece.source.start) - number(cursor.last_source));
            if atomic {
                table.push(number(piece.source.len()));
            }
            cursor.gap = 0;
            cursor.last_source = piece.source.end;
        }
    }

    /// Counts rendered text that maps to no source, such as a line break.
    fn advance(&mut self, rendered: usize) {
        if let Some(cursor) = &mut self.cursor {
            cursor.gap += rendered;
        }
    }

    /// Where a link or image leads, or `None` when it is shown as text.
    fn resolve(&self, address: &str, kind: LinkKind) -> Option<String> {
        match classify(address) {
            Address::Fragment => {
                // A place on the same page carries the page's prefix, as the
                // ids it may point at do. A fragment always holds its `#`.
                let (before, place) = address.split_once('#').unwrap_or((address, ""));
                Some(format!("{before}#{}{place}", self.prefix))
            }
            Address::Web => Some(address.to_owned()),
            Address::Refused => None,
            Address::Relative => match self.setup.links.resolve(address, kind) {
                Resolved::Url(url) => (classify(&url) != Address::Refused).then_some(url),
                Resolved::Text => None,
            },
        }
    }
}

/// A file offset or a count as a table number. Files are at most 2 MiB, so
/// every value fits.
fn number(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// The length of text as the browser counts it, in UTF-16 units.
fn units(text: &str) -> usize {
    text.encode_utf16().count()
}

/// Text with every line ending as a line feed, as the browser reads it.
fn line_feeds(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn is_item_like(node: Node<'_>) -> bool {
    matches!(
        node.data().value,
        NodeValue::Item(_)
            | NodeValue::TaskItem(_)
            | NodeValue::Alert(_)
            | NodeValue::FootnoteDefinition(_)
    )
}

fn is_tight(node: Node<'_>) -> bool {
    node.parent()
        .and_then(comrak::arena_tree::Node::parent)
        .is_some_and(|list| matches!(list.data().value, NodeValue::List(list) if list.tight))
}

type Writer<'c, 'o, 'r> = Context<'c, 'o, State<'r>>;

// Writing to a `String` cannot fail, so the results of these writes are not
// looked at.
fn put(context: &mut Writer<'_, '_, '_>, text: &str) {
    let _ = context.write_str(text);
}

fn put_escaped(context: &mut Writer<'_, '_, '_>, text: &str) {
    let _ = context.escape(text);
}

fn put_href(context: &mut Writer<'_, '_, '_>, address: &str) {
    let _ = context.escape_href(address);
}

fn new_line(context: &mut Writer<'_, '_, '_>) {
    let _ = context.cr();
}

/// Writes pieces of text and adds them to the open block's table.
fn write_pieces(context: &mut Writer<'_, '_, '_>, pieces: &[Piece]) {
    for piece in pieces.iter().filter(|piece| !piece.text.is_empty()) {
        put_escaped(context, &piece.text);
        context.user.note(piece, units(&piece.text));
    }
}

/// Writes text from its pieces, or, when it has none, as text that maps to
/// no source.
fn write_mapped(context: &mut Writer<'_, '_, '_>, pieces: Option<Vec<Piece>>, text: &str) {
    if let Some(pieces) = pieces {
        write_pieces(context, &pieces);
    } else {
        put_escaped(context, text);
        context.user.advance(units(text));
    }
}

/// comrak's formatter callback: driftmark's writer for each node.
fn format(
    context: &mut Writer<'_, '_, '_>,
    node: Node<'_>,
    entering: bool,
) -> Result<ChildRendering, std::fmt::Error> {
    let value = &node.data().value;
    let children = match value {
        NodeValue::Document => document_end(context, entering),
        NodeValue::Paragraph => paragraph(context, node, entering),
        NodeValue::Heading(heading) => heading_block(context, node, entering, heading.level),
        NodeValue::Text(literal) => text_node(context, node, literal, entering),
        NodeValue::Code(code) => code_span(context, node, &code.literal, entering),
        NodeValue::SoftBreak => line_end(context, "\n", entering),
        NodeValue::LineBreak => line_end(context, "<br>\n", entering),
        NodeValue::HtmlInline(literal) => {
            if entering {
                inline_html(context, node, literal);
            }
            ChildRendering::HTML
        }
        NodeValue::Link(link) => link_node(context, link, entering),
        NodeValue::Image(link) => image(context, node, link, entering),
        NodeValue::FootnoteReference(reference) => {
            footnote_reference(context, node, reference, entering)
        }
        NodeValue::FootnoteDefinition(definition) => {
            footnote_definition(context, node, definition, entering)
        }
        NodeValue::CodeBlock(code) => {
            if entering {
                code_block(context, node, code);
            }
            ChildRendering::HTML
        }
        NodeValue::HtmlBlock(block) => {
            if entering {
                new_line(context);
                flow_html(context, node, &block.literal);
            }
            ChildRendering::HTML
        }
        NodeValue::Table(table) => table_block(context, &table.alignments, entering),
        NodeValue::TableRow(header) => table_row(context, node, *header, entering),
        NodeValue::TableCell => table_cell(context, entering),
        NodeValue::TaskItem(task) => task_item(context, node, task.symbol.is_some(), entering),
        NodeValue::Alert(alert) => alert_block(context, node, alert, entering),
        NodeValue::Item(_) | NodeValue::BlockQuote => {
            if !entering {
                leave_container(context, node);
            }
            return html::format_node_default(context, node, entering);
        }
        _ => return html::format_node_default(context, node, entering),
    };
    Ok(children)
}

/// Closes what the document left open: raw HTML elements and the footnote
/// list.
fn document_end(context: &mut Writer<'_, '_, '_>, entering: bool) -> ChildRendering {
    if !entering {
        close_flow(context, 0);
        if context.user.footnotes > 0 {
            put(context, "</ol>\n</section>\n");
        }
    }
    ChildRendering::HTML
}

fn text_node(
    context: &mut Writer<'_, '_, '_>,
    node: Node<'_>,
    literal: &str,
    entering: bool,
) -> ChildRendering {
    if entering {
        let pieces = context.user.mapper.text(node, literal);
        write_mapped(context, pieces, literal);
    }
    ChildRendering::HTML
}

fn code_span(
    context: &mut Writer<'_, '_, '_>,
    node: Node<'_>,
    literal: &str,
    entering: bool,
) -> ChildRendering {
    if entering {
        put(context, "<code>");
        let pieces = context.user.mapper.code_span(node, literal);
        write_mapped(context, pieces, &code_span_text(literal));
        put(context, "</code>");
    }
    ChildRendering::HTML
}

/// A soft or hard line break: one line feed of rendered text that maps to
/// no source.
fn line_end(context: &mut Writer<'_, '_, '_>, html: &str, entering: bool) -> ChildRendering {
    if entering {
        put(context, html);
        context.user.advance(1);
    }
    ChildRendering::HTML
}

fn table_block(
    context: &mut Writer<'_, '_, '_>,
    alignments: &[TableAlignment],
    entering: bool,
) -> ChildRendering {
    if entering {
        new_line(context);
        put(
            context,
            "<div class=\"table\" role=\"region\" tabindex=\"0\" aria-label=\"Table\">\n<table>\n",
        );
        context.user.table = TableState {
            alignments: alignments.to_vec(),
            ..TableState::default()
        };
    } else {
        if context.user.table.body_open {
            put(context, "</tbody>\n");
        }
        put(context, "</table>\n</div>\n");
    }
    ChildRendering::HTML
}

fn task_item(
    context: &mut Writer<'_, '_, '_>,
    node: Node<'_>,
    done: bool,
    entering: bool,
) -> ChildRendering {
    if entering {
        new_line(context);
        // The box has no label of its own, so it carries its state as a
        // name for screen readers.
        let (state, checked, name) = if done {
            ("done", " checked", "Done")
        } else {
            ("open", "", "Not done")
        };
        let _ = write!(
            context,
            "<li class=\"task\" data-task=\"{state}\"><input type=\"checkbox\" disabled{checked} aria-label=\"{name}\"> "
        );
    } else {
        leave_container(context, node);
        put(context, "</li>\n");
    }
    ChildRendering::HTML
}

/// Closes the raw HTML elements an ending container opened.
fn leave_container(context: &mut Writer<'_, '_, '_>, container: Node<'_>) {
    close_flow(context, container_depth(container));
}

/// How many block quotes, list items, alerts and footnotes hold `node`, the
/// node itself included.
fn container_depth(node: Node<'_>) -> usize {
    node.ancestors()
        .filter(|ancestor| {
            is_item_like(ancestor) || matches!(ancestor.data().value, NodeValue::BlockQuote)
        })
        .count()
}

fn paragraph(context: &mut Writer<'_, '_, '_>, node: Node<'_>, entering: bool) -> ChildRendering {
    if entering {
        let tag = if is_tight(node) { "span" } else { "p" };
        new_line_unless_tight(context, tag);
        let block = context.user.block_of_paragraph(node);
        open_element(context, tag, block);
        context.user.paragraph = Some(tag);
    } else {
        close_inline(context);
        let ends_footnote = node.next_sibling().is_none()
            && node.parent().is_some_and(|parent| {
                matches!(parent.data().value, NodeValue::FootnoteDefinition(_))
            });
        if ends_footnote {
            write_backref(context);
        }
        context.user.close_leaf();
        let tag = context.user.paragraph.take().unwrap_or("p");
        let _ = write!(context, "</{tag}>");
        if tag == "p" {
            put(context, "\n");
        }
    }
    ChildRendering::HTML
}

fn new_line_unless_tight(context: &mut Writer<'_, '_, '_>, tag: &str) {
    if tag == "p" {
        new_line(context);
    }
}

/// Writes `<tag data-k="n">`, or `<tag>` when no block was found, and
/// starts the block's table.
fn open_element(context: &mut Writer<'_, '_, '_>, tag: &str, block: Option<usize>) {
    let _ = write!(context, "<{tag}");
    if let Some(block) = block {
        let _ = write!(context, " data-k=\"{block}\"");
        context.user.open_leaf(block);
    }
    put(context, ">");
}

fn heading_block(
    context: &mut Writer<'_, '_, '_>,
    node: Node<'_>,
    entering: bool,
    level: u8,
) -> ChildRendering {
    if entering {
        new_line(context);
        let block = context.user.block_at(node);
        let id = block.and_then(|block| context.user.ids[block].clone());
        let _ = write!(context, "<h{level}");
        if let Some(id) = &id {
            put(context, " id=\"");
            put_escaped(context, id);
            put(context, "\"");
        }
        if let Some(block) = block {
            let _ = write!(context, " data-k=\"{block}\"");
            context.user.open_leaf(block);
        }
        put(context, ">");
        context.user.heading_id = id;
    } else {
        close_inline(context);
        context.user.close_leaf();
        if let Some(id) = context.user.heading_id.take() {
            put(context, "<a class=\"anchor\" href=\"#");
            put_escaped(context, &id);
            put(context, "\" aria-label=\"Link to this section\"></a>");
        }
        let _ = writeln!(context, "</h{level}>");
    }
    ChildRendering::HTML
}

fn link_node(context: &mut Writer<'_, '_, '_>, link: &NodeLink, entering: bool) -> ChildRendering {
    if entering {
        let closer = if let Some(href) = context.user.resolve(&link.url, LinkKind::Link) {
            put(context, "<a href=\"");
            put_href(context, &href);
            if !link.title.is_empty() {
                put(context, "\" title=\"");
                put_escaped(context, &link.title);
            }
            put(context, "\">");
            "</a>"
        } else {
            put(context, "<span class=\"link-off\">");
            "</span>"
        };
        context.user.closers.push(closer);
    } else {
        let closer = context.user.closers.pop().unwrap_or_default();
        put(context, closer);
    }
    ChildRendering::HTML
}

fn image(
    context: &mut Writer<'_, '_, '_>,
    node: Node<'_>,
    link: &NodeLink,
    entering: bool,
) -> ChildRendering {
    if entering {
        let alt = inline_text(node);
        if let Some(source) = context.user.resolve(&link.url, LinkKind::Image) {
            put(context, "<img src=\"");
            put_href(context, &source);
            put(context, "\" alt=\"");
            put_escaped(context, &alt);
            if !link.title.is_empty() {
                put(context, "\" title=\"");
                put_escaped(context, &link.title);
            }
            put(context, "\">");
            if let Some(piece) = context.user.mapper.whole(node, Kind::Object, "") {
                context.user.note(&piece, 0);
            }
        } else {
            put(context, "<span class=\"image-off\">");
            let piece = context.user.mapper.whole(node, Kind::Atomic, &alt);
            write_mapped(context, piece.map(|piece| vec![piece]), &alt);
            put(context, "</span>");
        }
    }
    ChildRendering::Skip
}

fn footnote_reference(
    context: &mut Writer<'_, '_, '_>,
    node: Node<'_>,
    reference: &NodeFootnoteReference,
    entering: bool,
) -> ChildRendering {
    if entering {
        let prefix = context.user.prefix;
        put(context, "<sup class=\"footnote-ref\"><a href=\"#");
        put_href(context, prefix);
        put(context, "fn:");
        put_href(context, &reference.name);
        put(context, "\" id=\"");
        put_href(context, prefix);
        put(context, "fnref:");
        put_href(context, &reference.name);
        if reference.ref_num > 1 {
            let _ = write!(context, ":{}", reference.ref_num);
        }
        put(context, "\">");
        let number = reference.ix.to_string();
        let piece = context.user.mapper.whole(node, Kind::Atomic, &number);
        write_mapped(context, piece.map(|piece| vec![piece]), &number);
        put(context, "</a></sup>");
    }
    ChildRendering::HTML
}

fn footnote_definition(
    context: &mut Writer<'_, '_, '_>,
    node: Node<'_>,
    definition: &NodeFootnoteDefinition,
    entering: bool,
) -> ChildRendering {
    if entering {
        if context.user.footnotes == 0 {
            // comrak moves the definitions to the end, so the body is done.
            close_flow(context, 0);
            new_line(context);
            put(context, "<section class=\"footnotes\">\n<ol>\n");
        }
        context.user.footnotes += 1;
        let prefix = context.user.prefix;
        put(context, "<li id=\"");
        put_href(context, prefix);
        put(context, "fn:");
        put_href(context, &definition.name);
        put(context, "\">\n");
        context.user.backref = Some(Backref {
            name: definition.name.clone(),
            references: definition.total_references,
            number: context.user.footnotes,
        });
    } else {
        write_backref(context);
        leave_container(context, node);
        put(context, "</li>\n");
    }
    ChildRendering::HTML
}

/// Writes the links back to a footnote's references, once. They are page
/// furniture, so they sit in one `data-gen` element with the space before
/// them.
fn write_backref(context: &mut Writer<'_, '_, '_>) {
    if let Some(backref) = context.user.backref.take() {
        let prefix = context.user.prefix;
        put(context, "<span class=\"footnote-back\" data-gen>");
        for reference in 1..=backref.references {
            put(context, " <a href=\"#");
            put_href(context, prefix);
            put(context, "fnref:");
            put_href(context, &backref.name);
            if reference > 1 {
                let _ = write!(context, ":{reference}");
            }
            let _ = write!(
                context,
                "\" aria-label=\"Back to reference {}\">↩",
                backref.number
            );
            if reference > 1 {
                let _ = write!(context, "<sup>{reference}</sup>");
            }
            put(context, "</a>");
        }
        put(context, "</span>");
    }
}

fn code_block(context: &mut Writer<'_, '_, '_>, node: Node<'_>, code: &NodeCodeBlock) {
    new_line(context);
    let block = context.user.block_at(node);
    // The same rule as the document model: the info string's first word
    // names the language, and `mermaid` or `d2` makes a diagram.
    let language = code.info.split_whitespace().next();
    let diagram = match language {
        Some("mermaid") => Some(DiagramLanguage::Mermaid),
        Some("d2") => Some(DiagramLanguage::D2),
        _ => None,
    };
    let mut error = None;
    if let Some(diagram) = diagram {
        let drawn = block.map(|block| {
            let setup = context.user.setup;
            setup.diagrams.draw(&Diagram {
                language: diagram,
                code: &context.user.document.blocks[block].text,
                block,
            })
        });
        match drawn {
            Some(Ok(Some(svg))) => {
                put(context, "<figure class=\"diagram\"");
                write_block_attributes(context, block, Some(diagram));
                put(context, ">");
                put(context, &svg);
                put(context, "</figure>\n");
                context.user.close_leaf();
                return;
            }
            Some(Err(message)) => error = Some(message),
            _ => {}
        }
    }
    put(context, "<pre");
    write_block_attributes(context, block, diagram);
    if diagram.is_some() {
        put(context, " data-undrawn");
    } else if let Some(language) = language {
        // The page labels code with its language and styles program output
        // (`text`, `console`, `output`, `log`) apart, without a script.
        put(context, " data-language=\"");
        put_escaped(context, language);
        put(context, "\"");
    }
    if let Some(message) = error {
        put(context, " data-error=\"");
        put_escaped(context, &message);
        put(context, "\"");
    }
    put(context, "><code");
    if let Some(language) = language {
        put(context, " class=\"language-");
        put_escaped(context, language);
        put(context, "\"");
    }
    put(context, ">");
    let pieces = context
        .user
        .mapper
        .code_block(node, &code.literal, code.fenced);
    write_mapped(context, pieces, &line_feeds(&code.literal));
    context.user.close_leaf();
    put(context, "</code></pre>\n");
}

/// Writes ` data-k="n"` and ` data-diagram="…"`, and starts the block's
/// table.
fn write_block_attributes(
    context: &mut Writer<'_, '_, '_>,
    block: Option<usize>,
    diagram: Option<DiagramLanguage>,
) {
    if let Some(block) = block {
        let _ = write!(context, " data-k=\"{block}\"");
        context.user.open_leaf(block);
    }
    match diagram {
        Some(DiagramLanguage::Mermaid) => put(context, " data-diagram=\"mermaid\""),
        Some(DiagramLanguage::D2) => put(context, " data-diagram=\"d2\""),
        None => {}
    }
}

fn table_row(
    context: &mut Writer<'_, '_, '_>,
    node: Node<'_>,
    header: bool,
    entering: bool,
) -> ChildRendering {
    if entering {
        if header {
            put(context, "<thead>\n");
        } else if !context.user.table.body_open {
            put(context, "<tbody>\n");
            context.user.table.body_open = true;
        }
        context.user.table.header = header;
        context.user.table.cell = 0;
        let block = context.user.block_at(node);
        if header
            && let Some(BlockKind::TableRow { cells, .. }) =
                block.map(|block| &context.user.document.blocks[block].kind)
        {
            context.user.table.labels.clone_from(cells);
        }
        open_element(context, "tr", block);
    } else {
        context.user.close_leaf();
        put(context, "</tr>\n");
        if header {
            put(context, "</thead>\n");
        }
    }
    ChildRendering::HTML
}

fn table_cell(context: &mut Writer<'_, '_, '_>, entering: bool) -> ChildRendering {
    let tag = if context.user.table.header {
        "th"
    } else {
        "td"
    };
    if entering {
        let alignment = context
            .user
            .table
            .alignments
            .get(context.user.table.cell)
            .copied()
            .unwrap_or(TableAlignment::None);
        let align = match alignment {
            TableAlignment::Left => " align=\"left\"",
            TableAlignment::Center => " align=\"center\"",
            TableAlignment::Right => " align=\"right\"",
            TableAlignment::None => "",
        };
        let _ = write!(context, "<{tag}{align}");
        let label = context.user.table.labels.get(context.user.table.cell);
        if let Some(label) = label.filter(|_| !context.user.table.header).cloned() {
            put(context, " data-label=\"");
            put_escaped(context, &label);
            put(context, "\"");
        }
        put(context, ">");
    } else {
        close_inline(context);
        let _ = write!(context, "</{tag}>");
        context.user.table.cell += 1;
    }
    ChildRendering::HTML
}

fn alert_block(
    context: &mut Writer<'_, '_, '_>,
    node: Node<'_>,
    alert: &NodeAlert,
    entering: bool,
) -> ChildRendering {
    if entering {
        new_line(context);
        let kind = AlertKind::from(alert.alert_type)
            .name()
            .to_ascii_lowercase();
        let label = AlertType::default_title(&alert.alert_type);
        let _ = write!(
            context,
            "<aside class=\"alert\" data-alert=\"{kind}\" aria-label=\"{label}\">\n<p class=\"alert-title\" data-gen>"
        );
        put_escaped(context, alert.title.as_deref().unwrap_or(label));
        put(context, "</p>\n");
    } else {
        leave_container(context, node);
        put(context, "</aside>\n");
    }
    ChildRendering::HTML
}

/// Writes raw HTML shown as text, in `<code class="html">`.
fn write_escaped_html(context: &mut Writer<'_, '_, '_>, source: &str) {
    put(context, "<code class=\"html\">");
    put_escaped(context, &line_feeds(source));
    put(context, "</code>");
}

/// Writes an allowed opening tag with its allowed attributes.
fn write_open(
    context: &mut Writer<'_, '_, '_>,
    element: &str,
    attributes: &[(String, Option<&str>)],
) {
    let _ = write!(context, "<{element}");
    let kind = if element == "img" {
        LinkKind::Image
    } else {
        LinkKind::Link
    };
    for (name, value) in attributes {
        match raw::attribute(element, name) {
            Some(Value::Flag) => {
                let _ = write!(context, " {name}");
            }
            Some(Value::Text) => {
                let mut text = String::new();
                raw::push_text(&mut text, value.unwrap_or_default());
                let _ = write!(context, " {name}=\"{text}\"");
            }
            Some(Value::Address) => {
                let address = value
                    .and_then(decode_attribute)
                    .and_then(|address| context.user.resolve(&address, kind));
                if let Some(address) = address {
                    let _ = write!(context, " {name}=\"");
                    put_href(context, &address);
                    put(context, "\"");
                }
            }
            None => {}
        }
    }
    put(context, ">");
}

/// Writes an HTML block, tag by tag.
fn flow_html(context: &mut Writer<'_, '_, '_>, node: Node<'_>, literal: &str) {
    let depth = container_depth(node);
    for token in raw::tokens(literal) {
        match token {
            Token::Text(text) => {
                let mut out = String::new();
                raw::push_text(&mut out, text);
                put(context, &out);
            }
            Token::Hidden => {}
            Token::Open {
                name,
                attributes,
                source,
            } => match context.user.allowed.element(&name, Place::Flow) {
                Some(element) => {
                    write_open(context, element, &attributes);
                    if !raw::is_void(element) {
                        context.user.flow_open.push((element, depth));
                    }
                }
                None => write_escaped_html(context, source),
            },
            Token::Close { name, source } => match context.user.allowed.element(&name, Place::Flow)
            {
                Some(element) => close_flow_element(context, element, depth),
                None => write_escaped_html(context, source),
            },
        }
    }
}

/// Closes `element` when an HTML block of the current container opened
/// it, with the elements opened after it. A closing tag with nothing to
/// close is dropped, so raw HTML can never close the page's own elements.
fn close_flow_element(context: &mut Writer<'_, '_, '_>, element: &str, depth: usize) {
    // Depths never fall towards the top of the stack, so the topmost
    // element of this name is the only one this container can close.
    let open = &context.user.flow_open;
    let found = open
        .iter()
        .rposition(|&(name, _)| name == element)
        .filter(|&index| open[index] == (element, depth));
    if let Some(index) = found {
        while context.user.flow_open.len() > index {
            let (name, _) = context.user.flow_open.pop().unwrap_or_default();
            let _ = write!(context, "</{name}>");
        }
    }
}

/// Closes the elements HTML blocks opened at `depth` or deeper.
fn close_flow(context: &mut Writer<'_, '_, '_>, depth: usize) {
    while let Some(&(name, at)) = context.user.flow_open.last()
        && at >= depth
    {
        context.user.flow_open.pop();
        let _ = write!(context, "</{name}>");
    }
}

/// Writes one piece of inline HTML: an allowed tag, or the tag shown as
/// text with its place in the position table.
fn inline_html(context: &mut Writer<'_, '_, '_>, node: Node<'_>, literal: &str) {
    let tokens = raw::tokens(literal);
    let allowed = |name: &str| context.user.allowed.element(name, Place::Phrasing);
    match tokens.as_slice() {
        [Token::Hidden] => {}
        [
            Token::Open {
                name, attributes, ..
            },
        ] if allowed(name).is_some() => {
            let element = allowed(name).unwrap_or_default();
            write_open(context, element, attributes);
            if element == "img" {
                if let Some(piece) = context.user.mapper.whole(node, Kind::Object, "") {
                    context.user.note(&piece, 0);
                }
            } else if !raw::is_void(element) {
                context.user.inline_open.push(element);
            }
        }
        [Token::Close { name, .. }] if allowed(name).is_some() => {
            let found = context
                .user
                .inline_open
                .iter()
                .rposition(|open| open == name);
            if let Some(index) = found {
                while context.user.inline_open.len() > index {
                    let name = context.user.inline_open.pop().unwrap_or_default();
                    let _ = write!(context, "</{name}>");
                }
            }
        }
        _ => {
            let text = line_feeds(literal);
            let range = context.user.mapper.range(node);
            let source = context.user.document.source.as_str();
            let pieces = range.map(|range| {
                let kind = if source[range.clone()] == text {
                    Kind::Verbatim
                } else {
                    Kind::Atomic
                };
                vec![Piece {
                    kind,
                    text: text.clone(),
                    source: range,
                }]
            });
            put(context, "<code class=\"html\">");
            write_mapped(context, pieces, &text);
            put(context, "</code>");
        }
    }
}

/// Closes the inline HTML elements the ending paragraph, heading or cell
/// left open.
fn close_inline(context: &mut Writer<'_, '_, '_>) {
    while let Some(name) = context.user.inline_open.pop() {
        let _ = write!(context, "</{name}>");
    }
}

#[cfg(test)]
mod tests;
