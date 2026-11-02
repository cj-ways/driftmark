//! Documents and change lists as HTML that a page can show and the notes
//! script can read.
//!
//! [`reading`] turns a parsed document into reading HTML, the position tables
//! notes need, and the outline of its sections; [`reading_prefixed`] does the
//! same for a page that holds several documents. [`changes`] turns one
//! document's change list into HTML. Both decide the structure only: the
//! elements, classes and attributes that the page script and the style sheet
//! rely on. `docs/features/engine/html.md` states that contract in full.
//!
//! The core reads no files. Two traits let the program decide what the core
//! cannot: [`LinkResolver`] says where a relative link leads, and
//! [`DiagramRender`] draws Mermaid and D2.

mod changes;
mod html;
mod pieces;
mod reading;
mod url;
pub mod words;

use serde::Serialize;

pub use changes::changes;
pub use reading::{reading, reading_prefixed};

use crate::document::DiagramLanguage;

/// The page script that maps a selection to source bytes and back, from
/// `web/anchor.js` in this crate. It reads the [`PositionTables`] of a
/// reading page. Pages serve it as it is.
pub const ANCHOR_SCRIPT: &str = include_str!("../../web/anchor.js");

/// What a page shows for one document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reading {
    /// The document's blocks as HTML, without `<html>`, `<head>` or
    /// `<body>`. The page puts it inside its own element.
    pub html: String,
    /// The position table of each block, for notes. A page loads it only
    /// when the reader starts a note.
    pub positions: PositionTables,
    /// The headings outside any container, in order, for an "on this page"
    /// list.
    pub outline: Vec<Section>,
}

/// One heading of a document's outline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// 1 to 6.
    pub level: u8,
    /// The heading's `id` on the page, without `#`.
    pub id: String,
    /// The heading's plain text.
    pub text: String,
    /// The heading's block index.
    pub block: usize,
}

/// The position tables of one reading page: one list of whole numbers per
/// block, indexed by block. A block with no element on the page has an
/// empty list.
///
/// A list starts with the block's first source byte. Then each piece of the
/// block's rendered text adds three or four numbers: the rendered gap since
/// the previous piece, the rendered length times 2 plus 1 when the piece maps
/// only as a whole, the source gap since the previous piece, and, for a piece
/// that maps only as a whole, its source length. Rendered lengths count
/// UTF-16 units, as the browser does; source offsets count UTF-8 bytes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct PositionTables {
    /// The tables, by block index.
    pub blocks: Vec<Vec<i64>>,
}

/// Whether an address belongs to a link or an image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkKind {
    /// A link: `[text](address)`, or `<a href>` in raw HTML.
    Link,
    /// An image: `![alt](address)`, or `<img src>` in raw HTML.
    Image,
}

/// Where a relative link or image leads, as the program decides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    /// The page links here. An address with a scheme other than `http`,
    /// `https` or `mailto` is still refused.
    Url(String),
    /// The page shows the link's text, or the image's alt text, without a
    /// link: the target is outside what the program can show.
    Text,
}

/// Turns the relative links and image addresses of a document into the
/// addresses the program serves.
///
/// Only relative addresses reach it: a path such as `setup.md#install`,
/// `../img/cat.png` or `/docs/a.md`, or an empty one. Addresses on the same
/// page (`#install`) and on the web (`https:`, `mailto:`) stay as written,
/// and every other scheme is refused before it gets here.
pub trait LinkResolver {
    /// Where `address`, as written in the document, leads.
    fn resolve(&self, address: &str, kind: LinkKind) -> Resolved;
}

/// A [`LinkResolver`] that keeps every address as written. Tests use it,
/// and so does a page that sits next to the files it links to.
#[derive(Debug, Clone, Copy, Default)]
pub struct AsWritten;

impl LinkResolver for AsWritten {
    fn resolve(&self, address: &str, _kind: LinkKind) -> Resolved {
        Resolved::Url(address.to_owned())
    }
}

/// One diagram to draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Diagram<'a> {
    /// Mermaid or D2.
    pub language: DiagramLanguage,
    /// The diagram's code, with line feeds and without the last one.
    pub code: &'a str,
    /// The diagram's block index. A page holds many diagrams, so a drawing
    /// can use it to keep its ids unique.
    pub block: usize,
}

/// Draws diagrams on the server.
///
/// The core does not draw; the program does, with merman for Mermaid and
/// the `d2` program for D2. A drawing is placed in the page as it is
/// returned, so it must be safe to show: no scripts, no event attributes, and
/// no `javascript:` or `data:` addresses.
pub trait DiagramRender {
    /// The diagram as SVG markup.
    ///
    /// `Ok(None)` means this program does not draw the language. Either
    /// way, the block then stays code marked as an undrawn diagram, so the
    /// page can draw it in the browser.
    ///
    /// # Errors
    ///
    /// Why the diagram could not be drawn. The page shows the message beside
    /// the code.
    fn draw(&self, diagram: &Diagram<'_>) -> Result<Option<String>, String>;
}

/// A [`DiagramRender`] that draws nothing, so every diagram stays code for
/// the browser to draw.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoDiagrams;

impl DiagramRender for NoDiagrams {
    fn draw(&self, _diagram: &Diagram<'_>) -> Result<Option<String>, String> {
        Ok(None)
    }
}

/// What the program decides for a reading page.
#[derive(Clone, Copy)]
pub struct Setup<'a> {
    /// Where relative links and images lead.
    pub links: &'a dyn LinkResolver,
    /// Who draws diagrams.
    pub diagrams: &'a dyn DiagramRender,
    /// The raw HTML elements the rules file allows, such as
    /// `LintSettings::allowed_html`. Only elements that cannot run code or
    /// load a frame are ever shown; any other is shown as text.
    pub allowed_html: &'a [String],
}

impl std::fmt::Debug for Setup<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Setup")
            .field("allowed_html", &self.allowed_html)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_keep_addresses_and_draw_nothing() {
        assert_eq!(
            AsWritten.resolve("a.md", LinkKind::Link),
            Resolved::Url("a.md".to_owned())
        );
        let diagram = Diagram {
            language: DiagramLanguage::D2,
            code: "a -> b",
            block: 0,
        };
        assert_eq!(NoDiagrams.draw(&diagram), Ok(None));
        let setup = Setup {
            links: &AsWritten,
            diagrams: &NoDiagrams,
            allowed_html: &[],
        };
        assert_eq!(format!("{setup:?}"), "Setup { allowed_html: [], .. }");
    }

    #[test]
    fn the_page_script_is_built_in() {
        assert!(ANCHOR_SCRIPT.contains("driftmarkAnchor"));
    }
}
