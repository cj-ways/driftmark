//! Tests of the reading writer: the paths the fixture documents do not
//! reach, and the table format itself.

use comrak::nodes::LineColumn;

use super::*;
use crate::render::{AsWritten, NoDiagrams};

fn allowed() -> Vec<String> {
    crate::lint::DEFAULT_ALLOWED_HTML
        .map(str::to_owned)
        .to_vec()
}

fn render_with(source: &str, allowed: &[String]) -> Reading {
    let document = Document::parse(source.as_bytes().to_vec()).expect("parses");
    let setup = Setup {
        links: &AsWritten,
        diagrams: &NoDiagrams,
        allowed_html: allowed,
    };
    reading(&document, &setup)
}

fn render_text(source: &str) -> Reading {
    render_with(source, &allowed())
}

fn html(source: &str) -> String {
    render_text(source).html
}

/// Renders a tree that the test may have changed, with `document`'s blocks.
fn render_tree(root: Node<'_>, document: &Document) -> (String, PositionTables) {
    let allowed = allowed();
    let setup = Setup {
        links: &AsWritten,
        diagrams: &NoDiagrams,
        allowed_html: &allowed,
    };
    let mut out = String::new();
    let ids = Anchors::by_block(document);
    let tables = render(root, &mut out, document, &setup, ids, "");
    (out, tables)
}

#[test]
fn a_table_lists_each_piece_with_its_gaps() {
    let page = render_text("# T\n\nab *c* &amp; d![i](x.png)\n");
    // Block 1 starts at byte 5. "ab " is verbatim (3 units, 0 source gap);
    // "c" follows 1 byte of `*`; then " " after `*`; "&" maps to "&amp;"
    // whole; " d" is verbatim; the image is an object of 11 bytes.
    assert_eq!(
        page.positions.blocks,
        [
            vec![0, 0, 2, 2],
            vec![
                5, 0, 6, 0, 0, 2, 1, 0, 2, 1, 0, 3, 0, 5, 0, 4, 0, 0, 1, 0, 11
            ],
        ]
    );
    assert_eq!(page.outline.len(), 1);
}

#[test]
fn the_outline_holds_only_headings_outside_containers() {
    let page = render_text("# Title\n\n- ## In a list\n\n> ## In a quote\n\n## Section\n");
    let outline: Vec<(u8, &str, &str, usize)> = page
        .outline
        .iter()
        .map(|section| {
            (
                section.level,
                section.id.as_str(),
                section.text.as_str(),
                section.block,
            )
        })
        .collect();
    assert_eq!(
        outline,
        [(1, "title", "Title", 0), (2, "section", "Section", 4)]
    );
}

#[test]
fn a_prefix_starts_every_id_and_every_link_on_the_page() {
    let source = "# Guide\n\n## Install\n\nSee [install](#install), <a href=\" #install\">this</a>, [a](a.md#x), [web](https://example.com/#y) and a note[^n] twice[^n].\n\n[^n]: The note.\n";
    let document = Document::parse(source.as_bytes().to_vec()).expect("parses");
    let allowed = vec!["a".to_owned()];
    let setup = Setup {
        links: &AsWritten,
        diagrams: &NoDiagrams,
        allowed_html: &allowed,
    };
    let page = reading_prefixed(&document, &setup, "R7-");
    let ids: Vec<&str> = page
        .outline
        .iter()
        .map(|section| section.id.as_str())
        .collect();
    assert_eq!(ids, ["R7-guide", "R7-install"]);
    assert_eq!(
        page.html,
        "<h1 id=\"R7-guide\" data-k=\"0\">Guide<a class=\"anchor\" href=\"#R7-guide\" aria-label=\"Link to this section\"></a></h1>\n\
         <h2 id=\"R7-install\" data-k=\"1\">Install<a class=\"anchor\" href=\"#R7-install\" aria-label=\"Link to this section\"></a></h2>\n\
         <p data-k=\"2\">See <a href=\"#R7-install\">install</a>, <a href=\"%20#R7-install\">this</a>, <a href=\"a.md#x\">a</a>, <a href=\"https://example.com/#y\">web</a> and a note<sup class=\"footnote-ref\"><a href=\"#R7-fn:n\" id=\"R7-fnref:n\">1</a></sup> twice<sup class=\"footnote-ref\"><a href=\"#R7-fn:n\" id=\"R7-fnref:n:2\">1</a></sup>.</p>\n\
         <section class=\"footnotes\">\n<ol>\n<li id=\"R7-fn:n\">\n<p data-k=\"3\">The note.<span class=\"footnote-back\" data-gen> <a href=\"#R7-fnref:n\" aria-label=\"Back to reference 1\">↩</a> <a href=\"#R7-fnref:n:2\" aria-label=\"Back to reference 1\">↩<sup>2</sup></a></span></p>\n</li>\n</ol>\n</section>\n"
    );
    // The same document without a prefix is the plain reading page, whose
    // position tables the prefix never changes.
    let plain = reading(&document, &setup);
    assert_eq!(plain, reading_prefixed(&document, &setup, ""));
    assert_eq!(page.positions, plain.positions);
    assert_eq!(page.html.replace("R7-", ""), plain.html);
}

#[test]
fn a_block_written_as_two_elements_keeps_one_table() {
    let page = render_text("- one\n\n  two\n- three\n");
    // The item's two paragraphs continue one table; the next item starts
    // its own.
    assert_eq!(
        page.positions.blocks,
        [vec![0, 0, 6, 2, 0, 6, 4], vec![13, 0, 10, 2]]
    );
}

#[test]
fn line_breaks_count_as_rendered_text_without_a_source() {
    let page = render_text("a\nb  \nc\n");
    // "a"; a soft break (1 unit, no source); "b" one byte on; a hard break;
    // "c" after the two spaces and the line feed.
    assert_eq!(page.positions.blocks, [vec![0, 0, 2, 0, 1, 2, 1, 1, 2, 3]]);
}

#[test]
fn without_blocks_the_page_still_shows_every_node() {
    let source = "# Title\n\nText with `code` and ![alt](a.png) and a[^1],\nover two lines.\n\n- item\n\n| a |\n|---|\n| b |\n\n```mermaid\ngraph\n```\n\n<details>\nx\n</details>\n\n[^1]: Note.\n";
    let mut document = Document::parse(source.as_bytes().to_vec()).expect("parses");
    document.blocks.clear();
    let arena = Arena::new();
    let root = parse_document(&arena, source, &options());
    let (out, tables) = render_tree(root, &document);
    assert!(!out.contains("data-k"), "{out}");
    assert!(out.contains("<h1>Title</h1>"), "{out}");
    assert!(out.contains("<li><span>item</span></li>"), "{out}");
    assert!(
        out.contains("<pre data-diagram=\"mermaid\" data-undrawn>"),
        "{out}"
    );
    assert!(out.contains("<tr><td>b</td></tr>"), "{out}");
    assert!(out.contains("Text with <code>code</code> and <img src=\"a.png\" alt=\"alt\">"));
    assert!(out.contains(",\nover two lines."), "{out}");
    assert_eq!(tables.blocks, Vec::<Vec<i64>>::new());
}

#[test]
fn nodes_with_bad_positions_are_shown_without_a_table_place() {
    let source = "Some text, `code`, ![alt](a.png) and <img src=\"b.png\"> here.\n\n```\nx\n```\n";
    let document = Document::parse(source.as_bytes().to_vec()).expect("parses");
    let arena = Arena::new();
    let root = parse_document(&arena, source, &options());
    let paragraph = root.first_child().expect("a paragraph");
    for node in paragraph.children() {
        node.data_mut().sourcepos.start = LineColumn { line: 9, column: 1 };
    }
    let block = root.last_child().expect("a code block");
    block.data_mut().sourcepos = (3, 1, 3, 3).into();
    // A second document whose paragraph itself has no readable position.
    let lost = parse_document(&arena, "Lost.\n", &options());
    let lost_paragraph = lost.first_child().expect("a paragraph");
    lost_paragraph.data_mut().sourcepos.start = LineColumn { line: 9, column: 1 };
    let lost_document = Document::parse(b"Lost.\n".to_vec()).expect("parses");
    assert_eq!(render_tree(lost, &lost_document).0, "<p>Lost.</p>\n");
    let mut allowed = allowed();
    allowed.push("img".to_owned());
    let setup = Setup {
        links: &AsWritten,
        diagrams: &NoDiagrams,
        allowed_html: &allowed,
    };
    let mut out = String::new();
    let tables = render(
        root,
        &mut out,
        &document,
        &setup,
        Anchors::by_block(&document),
        "",
    );
    assert!(out.contains("<p data-k=\"0\">Some text, <code>code</code>, <img src=\"a.png\" alt=\"alt\"> and <img src=\"b.png\"> here.</p>"), "{out}");
    // Only rendered gaps: no piece maps, the images included.
    assert_eq!(tables.blocks[0], [0]);
    assert!(
        out.contains("<pre data-k=\"1\"><code>x\n</code></pre>"),
        "{out}"
    );
}

#[test]
fn raw_html_shown_as_text_maps_whole_when_its_source_differs() {
    let page = render_text("> a <span\n> title=\"x\"> b\n");
    assert!(
        page.html
            .contains("<code class=\"html\">&lt;span\ntitle=&quot;x&quot;&gt;</code>"),
        "{}",
        page.html
    );
    // "a " verbatim, then the tag as one atomic piece over its 18 source
    // bytes, the `> ` of the second line included.
    assert_eq!(page.positions.blocks[0][..8], [2, 0, 4, 0, 0, 33, 0, 18]);
}

#[test]
fn raw_html_with_nothing_allowed_is_all_text() {
    let page = render_with("<details>\n\nx <kbd>K</kbd>\n\n</details>\n", &[]);
    assert_eq!(
        page.html,
        "<code class=\"html\">&lt;details&gt;</code>\n<p data-k=\"1\">x <code class=\"html\">&lt;kbd&gt;</code>K<code class=\"html\">&lt;/kbd&gt;</code></p>\n<code class=\"html\">&lt;/details&gt;</code>\n"
    );
}

#[test]
fn open_raw_html_closes_where_its_container_ends() {
    // An unclosed element at the end of the document.
    assert_eq!(
        html("<details>\n<summary>S</summary>\n"),
        "<details>\n<summary>S</summary>\n</details>"
    );
    // An element opened in a list item closes with the item. One opened
    // outside it cannot be closed from inside, so that `</details>` is
    // dropped and the last one closes it.
    let nested =
        html("<details>\n\n- item\n\n  </details>\n\n  <details>\n\n- next\n\n</details>\n");
    assert_eq!(
        nested,
        "<details>\n<ul>\n<li>\n<p data-k=\"1\">item</p>\n\n<details>\n</details></li>\n<li>\n<p data-k=\"4\">next</p>\n</li>\n</ul>\n</details>\n"
    );
}

#[test]
fn closing_an_outer_raw_element_closes_the_inner_ones() {
    let mut allowed = allowed();
    allowed.push("div".to_owned());
    let page = render_with("<details><div>\nx\n</details>\n", &allowed);
    assert_eq!(page.html, "<details><div>\nx\n</div></details>\n");
}

#[test]
fn inline_raw_html_closes_inside_its_paragraph() {
    assert_eq!(
        html("a <sub><kbd>b</sub> c <sup>d\n\ne </kbd> f\n"),
        "<p data-k=\"0\">a <sub><kbd>b</kbd></sub> c <sup>d</sup></p>\n<p data-k=\"1\">e  f</p>\n"
    );
    // Closing an inner element leaves the outer one open.
    assert_eq!(
        html("<sub><kbd>x</kbd>y</sub>\n"),
        "<p data-k=\"0\"><sub><kbd>x</kbd>y</sub></p>\n"
    );
}

#[test]
fn a_footnote_ending_in_code_gets_its_back_link_after_it() {
    let out = html("a[^n]\n\n[^n]:\n    ```\n    x\n    ```\n");
    assert!(
        out.ends_with("</code></pre>\n<span class=\"footnote-back\" data-gen> <a href=\"#fnref:n\" aria-label=\"Back to reference 1\">↩</a></span></li>\n</ol>\n</section>\n"),
        "{out}"
    );
}

/// The Markdown files of a fixture folder.
fn fixture_documents(folder: &str) -> Vec<Document> {
    let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/render")
        .join(folder);
    let mut paths: Vec<_> = std::fs::read_dir(folder)
        .expect("the folder exists")
        .map(|entry| entry.expect("an entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "md"))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|path| Document::parse(std::fs::read(path).expect("reads")).expect("parses"))
        .collect()
}

#[test]
fn every_fixture_renders_in_the_unit_build_too() {
    // The golden tests run against the library built for integration tests.
    // Rendering the same documents here keeps every writer covered within
    // this build as well, which the coverage gate measures on its own.
    let mut allowed = allowed();
    allowed.extend(["img", "a", "div", "p", "span"].map(str::to_owned));
    let setup = Setup {
        links: &AsWritten,
        diagrams: &Drawn,
        allowed_html: &allowed,
    };
    let mut pages = 0;
    for folder in ["reading", "hostile", "anchoring"] {
        for document in fixture_documents(folder) {
            let page = reading(&document, &setup);
            assert!(!page.html.contains("<script"));
            pages += 1;
        }
    }
    assert_eq!(pages, 47);
}

/// Draws every Mermaid diagram that holds "graph", fails on "fail", and
/// draws no D2.
struct Drawn;

impl crate::render::DiagramRender for Drawn {
    fn draw(&self, diagram: &Diagram<'_>) -> Result<Option<String>, String> {
        match diagram.language {
            DiagramLanguage::Mermaid if diagram.code.contains("fail") => Err("no".to_owned()),
            DiagramLanguage::Mermaid if diagram.code.contains("graph") => {
                Ok(Some("<svg></svg>".to_owned()))
            }
            _ => Ok(None),
        }
    }
}

#[test]
fn the_page_helpers_count_like_the_browser() {
    assert_eq!(units("a😀é"), 4);
    assert_eq!(line_feeds("a\r\nb\rc\n"), "a\nb\nc\n");
    assert_eq!(number(7), 7);
}
