//! Drawings are safe before they reach a page.
//!
//! `fixtures/diagrams/hostile-svg/` holds SVG that a drawer could return if
//! it were tricked or broken: scripts, event attributes, outside addresses,
//! animations that change a link, style blocks, HTML labels with images and
//! frames, ids that copy a page's own, entities and two roots. Each is
//! cleaned, and the result must hold only what a drawing needs.
//!
//! `fixtures/diagrams/hostile-mermaid/` holds Mermaid sources that try the
//! same through merman: `click` to `javascript:`, HTML in labels, a class
//! definition that tries to break out of its CSS. Each is drawn and
//! cleaned, and checked the same way.

use std::fs;
use std::path::{Path, PathBuf};

use driftmark::diagram::{Drawer, clean};
use driftmark_core::document::DiagramLanguage;

const PREFIX: &str = "dmtest";

fn fixtures(folder: &str) -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/diagrams")
        .join(folder);
    let mut found: Vec<PathBuf> = fs::read_dir(dir)
        .expect("the folder exists")
        .map(|entry| entry.expect("an entry").path())
        .collect();
    found.sort();
    found
}

/// Elements a clean drawing may hold.
const ELEMENTS: &[&str] = &[
    "a",
    "b",
    "br",
    "circle",
    "clipPath",
    "code",
    "defs",
    "del",
    "desc",
    "div",
    "ellipse",
    "em",
    "feBlend",
    "feColorMatrix",
    "feComposite",
    "feDropShadow",
    "feFlood",
    "feGaussianBlur",
    "feMerge",
    "feMergeNode",
    "feOffset",
    "filter",
    "foreignObject",
    "g",
    "i",
    "ins",
    "li",
    "line",
    "linearGradient",
    "marker",
    "mask",
    "ol",
    "p",
    "path",
    "pattern",
    "polygon",
    "polyline",
    "radialGradient",
    "rect",
    "s",
    "small",
    "span",
    "stop",
    "strong",
    "sub",
    "sup",
    "svg",
    "switch",
    "symbol",
    "text",
    "textPath",
    "title",
    "tspan",
    "u",
    "ul",
    "use",
];

/// One tag of the output: its name and its attributes, decoded.
struct Tag {
    name: String,
    attributes: Vec<(String, String)>,
}

/// Reads the tags of a clean drawing. The cleaner escapes `<` in text and
/// `"` in values, so a plain scan finds every tag.
fn tags(svg: &str) -> Vec<Tag> {
    let mut found = Vec::new();
    let mut rest = svg;
    while let Some(start) = rest.find('<') {
        let end = rest[start..].find('>').expect("a tag ends") + start;
        let inside = rest[start + 1..end].trim_end_matches('/');
        rest = &rest[end + 1..];
        if inside.starts_with('/') {
            continue;
        }
        let name_end = inside.find(' ').unwrap_or(inside.len());
        let name = inside[..name_end].to_owned();
        let mut attributes = Vec::new();
        let mut text = inside[name_end..].trim_start();
        while !text.is_empty() {
            let eq = text.find("=\"").expect("every attribute has a value");
            let key = text[..eq].trim().to_owned();
            let value_end = text[eq + 2..].find('"').expect("a value ends") + eq + 2;
            let value = text[eq + 2..value_end]
                .replace("&quot;", "\"")
                .replace("&lt;", "<")
                .replace("&gt;", ">")
                .replace("&amp;", "&");
            attributes.push((key, value));
            text = text[value_end + 1..].trim_start();
        }
        found.push(Tag { name, attributes });
    }
    found
}

fn assert_safe(case: &str, svg: &str) {
    assert!(svg.starts_with("<svg"), "{case}: {svg}");
    for tag in tags(svg) {
        assert!(
            ELEMENTS.contains(&tag.name.as_str()),
            "{case}: element <{}>",
            tag.name
        );
        for (key, value) in &tag.attributes {
            let lower_key = key.to_ascii_lowercase();
            let lower = value.to_ascii_lowercase();
            assert!(!lower_key.starts_with("on"), "{case}: {key}");
            assert!(
                !lower_key.starts_with("data-") && lower_key != "style",
                "{case}: {key}"
            );
            assert!(
                !lower.contains("javascript") && !lower.contains("data:"),
                "{case}: {key}={value}"
            );
            assert!(!lower.contains("expression("), "{case}: {key}={value}");
            let mut search = lower.as_str();
            while let Some(at) = search.find("url(") {
                assert!(
                    search[at..].starts_with(&format!("url(#{PREFIX}")),
                    "{case}: {key}={value}"
                );
                search = &search[at + 4..];
            }
            if key == "id" {
                assert!(value.starts_with(PREFIX), "{case}: id {value}");
            }
            if key == "href" || key == "xlink:href" {
                let inside = value.starts_with(&format!("#{PREFIX}"));
                let web = tag.name == "a"
                    && (lower.starts_with("https://")
                        || lower.starts_with("http://")
                        || lower.starts_with("mailto:"));
                assert!(inside || web, "{case}: {key}={value}");
            }
        }
    }
}

#[test]
fn hostile_drawings_come_out_clean_or_not_at_all() {
    let cases = fixtures("hostile-svg");
    assert!(cases.len() >= 12, "{} cases", cases.len());
    let mut refused = Vec::new();
    for path in cases {
        let case = path.file_name().unwrap().to_string_lossy().into_owned();
        let svg = fs::read_to_string(&path).expect("UTF-8");
        match clean(&svg, PREFIX) {
            Ok(out) => assert_safe(&case, &out),
            Err(_) => refused.push(case),
        }
    }
    refused.sort();
    assert_eq!(
        refused,
        [
            "doctype.svg",
            "not-svg.svg",
            "two-roots.svg",
            "unclosed.svg"
        ],
        "only drawings that are not one well-formed SVG are refused"
    );
}

#[test]
fn cleaning_keeps_what_the_drawing_needs() {
    let read = |name: &str| {
        let svg = fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/diagrams/hostile-svg")
                .join(name),
        )
        .unwrap();
        clean(&svg, PREFIX).unwrap()
    };
    let script = read("script.svg");
    assert!(
        script.contains("<text x=\"1\" y=\"20\">Kept text</text>"),
        "{script}"
    );
    let links = read("links.svg");
    assert!(
        links.contains("<a href=\"https://example.com/ok\"><text>web</text></a>"),
        "{links}"
    );
    assert!(links.contains("<a><text>one</text></a>"), "{links}");
    assert!(
        links.contains(&format!("<use xlink:href=\"#{PREFIX}-local\"/>")),
        "{links}"
    );
    let style = read("style.svg");
    assert!(
        style.contains("<text font-size=\"16px\" font-weight=\"400\">Words</text>"),
        "style becomes presentation attributes: {style}"
    );
    let foreign = read("foreign.svg");
    assert!(
        foreign.contains("<div class=\"dm-wrap dm-start\"><span class=\"nodeLabel\"><p>Label <b>bold</b><br>next </p></span></div>"),
        "{foreign}"
    );
    let ids = read("ids.svg");
    assert!(
        ids.contains(&format!("id=\"{PREFIX}\"")) || ids.contains(&format!("id=\"{PREFIX}-main\"")),
        "{ids}"
    );
    assert!(
        ids.contains(&format!("<g id=\"{PREFIX}-decision\">")),
        "{ids}"
    );
    assert!(
        ids.contains(&format!("<use href=\"#{PREFIX}-decision\"/>")),
        "{ids}"
    );
    assert!(ids.contains("class=\"ok good\""), "{ids}");
    let breaking = read("break-out.svg");
    assert!(
        breaking.contains("<text>&lt;/svg&gt;&lt;script&gt;alert(1)&lt;/script&gt;</text>"),
        "{breaking}"
    );
}

#[test]
fn hostile_mermaid_is_drawn_clean() {
    let drawer = Drawer::new();
    assert!(drawer.draws_mermaid(), "merman loads the shipped fonts");
    let mut drawn = 0;
    for path in fixtures("hostile-mermaid") {
        let case = path.file_name().unwrap().to_string_lossy().into_owned();
        let code = fs::read_to_string(&path).expect("UTF-8");
        let code = code.trim_end_matches('\n');
        if let Ok(Some(svg)) = drawer.drawing(DiagramLanguage::Mermaid, code) {
            drawn += 1;
            // The drawer uses its own prefix; check against it.
            let prefix_at = svg.find("id=\"").map(|at| &svg[at + 4..at + 18]);
            let own = prefix_at.unwrap_or_default();
            let svg = svg.replace(own, PREFIX);
            assert_safe(&case, &svg);
        }
    }
    assert!(drawn >= 4, "most hostile diagrams still draw: {drawn}");
}
