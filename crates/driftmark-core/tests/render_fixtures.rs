//! Golden tests of the reading HTML and its position tables on the made-up
//! documents in `/fixtures/render`.
//!
//! - `reading/<name>.md` and `hostile/<name>.md` each have `<name>.html`, the
//!   page HTML, and `<name>.positions.json`, the position tables, beside
//!   them. Every run writes fresh output to `CARGO_TARGET_TMPDIR/render/`,
//!   so a deliberate change can be reviewed and copied over.
//! - Every fixture's page text must match its document model, block by block.
//! - Every tag on every page must come from the allow-lists, and no address
//!   may have a refused scheme.
//! - `anchoring/<name>.md` are the 30 documents of the 113 anchoring cases.
//!   Each run writes them as test pages to
//!   `CARGO_TARGET_TMPDIR/anchoring-pages/`, with the page script, for the
//!   browser harness in `tests/browser/` at the repository root.

#[path = "support/html_text.rs"]
mod html_text;

use std::fs;
use std::path::{Path, PathBuf};

use driftmark_core::document::{DiagramLanguage, Document};
use driftmark_core::render::{
    ANCHOR_SCRIPT, AsWritten, Diagram, DiagramRender, LinkKind, LinkResolver, NoDiagrams, Reading,
    Resolved, Setup, reading,
};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/render")
}

fn out_dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    fs::create_dir_all(&dir).expect("create the output folder");
    dir
}

/// The Markdown files of one fixture folder, by name.
fn documents(folder: &str) -> Vec<(String, Document)> {
    let mut found: Vec<(String, Document)> = fs::read_dir(fixtures().join(folder))
        .expect("the folder exists")
        .map(|entry| entry.expect("an entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "md"))
        .map(|path| {
            let name = path
                .file_stem()
                .expect("a name")
                .to_string_lossy()
                .into_owned();
            let bytes = fs::read(&path).expect("the fixture reads");
            (name, Document::parse(bytes).expect("the fixture parses"))
        })
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

fn default_allowed() -> Vec<String> {
    driftmark_core::lint::DEFAULT_ALLOWED_HTML
        .map(str::to_owned)
        .to_vec()
}

/// A test page around reading HTML: the tables in a JSON script element
/// whose id no heading can take, and the page script beside it.
fn test_page(name: &str, page: &Reading) -> String {
    let table = serde_json::to_string(&page.positions).expect("tables serialize");
    format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><title>{name}</title></head>\n<body><article>\n{}</article>\n<script type=\"application/json\" id=\"Tables\">{}</script>\n<script src=\"anchor.js\"></script>\n<script>driftmarkAnchor.setTable(JSON.parse(document.getElementById('Tables').textContent), document.querySelector('article'));</script>\n</body></html>\n",
        page.html,
        table.replace("</", "<\\/")
    )
}

/// Where the fixtures' links lead: documents to `/doc/`, images to `/raw/`,
/// anything outside the docs to plain text, and one document to an address
/// the page must still refuse.
struct TestLinks;

impl LinkResolver for TestLinks {
    fn resolve(&self, address: &str, kind: LinkKind) -> Resolved {
        if address.starts_with("../outside/") {
            Resolved::Text
        } else if address == "evil.md" {
            Resolved::Url("javascript:alert(0)".to_owned())
        } else if kind == LinkKind::Image {
            Resolved::Url(format!("/raw/{address}"))
        } else {
            Resolved::Url(format!("/doc/{address}"))
        }
    }
}

/// Draws Mermaid code that holds "graph" as a small SVG, fails on code that
/// holds "fail", and draws no D2.
struct TestDiagrams;

impl DiagramRender for TestDiagrams {
    fn draw(&self, diagram: &Diagram<'_>) -> Result<Option<String>, String> {
        match diagram.language {
            DiagramLanguage::Mermaid if diagram.code.contains("fail") => {
                Err("could not draw \"fail\" <here> & there".to_owned())
            }
            DiagramLanguage::Mermaid if diagram.code.contains("graph") => Ok(Some(format!(
                "<svg viewBox=\"0 0 10 10\" aria-label=\"diagram {}\"></svg>",
                diagram.block
            ))),
            DiagramLanguage::Mermaid | DiagramLanguage::D2 => Ok(None),
        }
    }
}

/// Every element name a page may hold: driftmark's own, and the raw HTML
/// elements a rules file may allow.
const PAGE_ELEMENTS: &[&str] = &[
    "a",
    "abbr",
    "aside",
    "b",
    "bdi",
    "blockquote",
    "br",
    "caption",
    "cite",
    "code",
    "dd",
    "del",
    "details",
    "dfn",
    "div",
    "dl",
    "dt",
    "em",
    "figcaption",
    "figure",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "hr",
    "i",
    "img",
    "input",
    "ins",
    "kbd",
    "li",
    "mark",
    "ol",
    "p",
    "pre",
    "q",
    "rp",
    "rt",
    "ruby",
    "s",
    "samp",
    "section",
    "small",
    "span",
    "strong",
    "sub",
    "summary",
    "sup",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "tr",
    "u",
    "ul",
    "var",
    "wbr",
];

/// Every attribute name a page may hold.
const PAGE_ATTRIBUTES: &[&str] = &[
    "align",
    "alt",
    "aria-label",
    "checked",
    "class",
    "colspan",
    "data-alert",
    "data-diagram",
    "data-error",
    "data-gen",
    "data-k",
    "data-label",
    "data-language",
    "data-task",
    "data-undrawn",
    "disabled",
    "height",
    "href",
    "id",
    "open",
    "role",
    "rowspan",
    "src",
    "start",
    "tabindex",
    "title",
    "type",
    "width",
];

/// Checks every tag of a page: only allowed elements and attributes, and no
/// address with a scheme other than http, https or mailto. The test SVG of
/// a drawn diagram is the drawer's and is skipped.
fn assert_safe(name: &str, html: &str) {
    let html = html
        .replace("<svg viewBox=\"0 0 10 10\"", "<svgtest")
        .replace("</svg>", "");
    let mut rest = html.as_str();
    while let Some(start) = rest.find('<') {
        let end = rest[start..].find('>').expect("a tag ends") + start;
        let tag = &rest[start + 1..end];
        rest = &rest[end + 1..];
        let tag = tag.trim_start_matches('/').trim_end_matches('/');
        let mut words = tag.split(' ');
        let element = words.next().unwrap_or_default();
        if element == "svgtest" {
            continue;
        }
        assert!(
            PAGE_ELEMENTS.contains(&element),
            "{name}: element <{element}>"
        );
        if element == "input" {
            assert!(
                tag.starts_with("input type=\"checkbox\" disabled"),
                "{name}: {tag}"
            );
        }
        let mut attributes = tag[element.len()..].trim();
        while !attributes.is_empty() {
            let name_end = attributes.find(['=', ' ']).unwrap_or(attributes.len());
            let attribute = &attributes[..name_end];
            assert!(
                PAGE_ATTRIBUTES.contains(&attribute),
                "{name}: attribute {attribute} on <{element}>"
            );
            attributes = &attributes[name_end..];
            if let Some(value) = attributes.strip_prefix("=\"") {
                let value_end = value.find('"').expect("a value ends");
                let value_text = &value[..value_end];
                if matches!(attribute, "href" | "src") {
                    let lower = value_text.to_ascii_lowercase();
                    let scheme = lower.split_once(':').map(|(scheme, _)| scheme);
                    let safe = match scheme {
                        Some(scheme) if scheme.chars().all(|c| c.is_ascii_alphanumeric()) => {
                            matches!(scheme, "http" | "https" | "mailto")
                        }
                        _ => true,
                    };
                    assert!(safe, "{name}: {attribute}=\"{value_text}\"");
                    assert!(!lower.contains("javascript"), "{name}: {value_text}");
                }
                attributes = &value[value_end + 1..];
            }
            attributes = attributes.trim_start();
        }
    }
}

/// Renders every fixture of a folder and compares the HTML and the tables
/// with the golden files beside it. Fresh output goes to the build folder.
fn check_golden(folder: &str, setup: &Setup<'_>, text_check: bool) {
    let out = out_dir(&format!("render/{folder}"));
    let mut failures = Vec::new();
    for (name, document) in documents(folder) {
        let page = reading(&document, setup);
        let tables = format!(
            "{}\n",
            serde_json::to_string(&page.positions).expect("tables serialize")
        );
        fs::write(out.join(format!("{name}.html")), &page.html).expect("write html");
        fs::write(out.join(format!("{name}.positions.json")), &tables).expect("write tables");
        if text_check {
            let wrong = html_text::compare(&document, &page.html);
            assert_eq!(
                wrong,
                [],
                "{folder}/{name}: page text differs from the model"
            );
        }
        assert_safe(&format!("{folder}/{name}"), &page.html);
        let golden = |extension: &str| {
            fs::read_to_string(fixtures().join(folder).join(format!("{name}.{extension}")))
                .unwrap_or_default()
        };
        if golden("html") != page.html || golden("positions.json") != tables {
            failures.push(name);
        }
    }
    assert!(
        failures.is_empty(),
        "golden files differ in {folder}: {failures:?}; fresh output is in {}",
        out.display()
    );
}

#[test]
fn reading_pages_match_their_golden_files() {
    let allowed = default_allowed();
    let setup = Setup {
        links: &TestLinks,
        diagrams: &TestDiagrams,
        allowed_html: &allowed,
    };
    check_golden("reading", &setup, true);
}

#[test]
fn hostile_documents_render_safe_pages() {
    // A rules file that names dangerous elements still cannot show them.
    let mut allowed = default_allowed();
    allowed.extend(
        [
            "script",
            "style",
            "iframe",
            "object",
            "embed",
            "svg",
            "math",
            "template",
            "textarea",
            "title",
            "noscript",
            "xmp",
            "form",
            "input",
            "button",
            "base",
            "meta",
            "link",
            "plaintext",
            "img",
            "a",
            "div",
            "p",
            "span",
            "table",
            "mglyph",
            "mtext",
        ]
        .map(str::to_owned),
    );
    let setup = Setup {
        links: &TestLinks,
        diagrams: &NoDiagrams,
        allowed_html: &allowed,
    };
    // The text check is left out: this rules file allows `<img>`, whose alt
    // text the page shows but the document model, which drops raw HTML, does
    // not.
    check_golden("hostile", &setup, false);
}

#[test]
fn the_anchoring_fixtures_become_test_pages() {
    let allowed = default_allowed();
    let setup = Setup {
        links: &AsWritten,
        diagrams: &NoDiagrams,
        allowed_html: &allowed,
    };
    let out = out_dir("anchoring-pages");
    fs::write(out.join("anchor.js"), ANCHOR_SCRIPT).expect("write the script");
    let found = documents("anchoring");
    assert_eq!(found.len(), 30);
    for (name, document) in &found {
        let page = reading(document, &setup);
        assert_eq!(html_text::compare(document, &page.html), [], "{name}");
        fs::write(out.join(format!("{name}.html")), test_page(name, &page)).expect("write a page");
    }
}
