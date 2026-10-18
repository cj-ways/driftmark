//! Golden tests of the change list HTML.
//!
//! Every pair in `/fixtures/diff/<case>` (one per change kind, view and
//! signal) and in `/fixtures/render/changes/<case>` is compared, and its HTML
//! must equal `/fixtures/render/changes/<case>.html`. A missing `a.md` or
//! `b.md` means the document was added or removed, and an optional
//! `driftmark.toml` is the rules file. Every run also writes the HTML to
//! `CARGO_TARGET_TMPDIR/render/changes/`, so a deliberate change can be
//! reviewed and copied over.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use driftmark_core::diff::{DocumentChanges, Version, compare};
use driftmark_core::document::Document;
use driftmark_core::render::changes;
use driftmark_core::rules::Rules;

const PATH: &str = "doc.md";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

/// Every case, with the folder that holds its pair.
fn cases() -> BTreeMap<String, PathBuf> {
    let mut found = BTreeMap::new();
    for parent in ["diff", "render/changes"] {
        for entry in fs::read_dir(fixtures().join(parent)).expect("the folder exists") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                let name = path
                    .file_name()
                    .expect("a name")
                    .to_string_lossy()
                    .into_owned();
                found.insert(name, path);
            }
        }
    }
    found
}

fn run(folder: &Path) -> DocumentChanges {
    let read = |name: &str| fs::read(folder.join(name)).ok();
    let rules = read("driftmark.toml").map_or_else(Rules::default, |text| {
        Rules::parse(&String::from_utf8(text).expect("UTF-8")).expect("the rules parse")
    });
    let old = read("a.md").map(|bytes| Document::parse(bytes).expect("parses"));
    let new = read("b.md").map(|bytes| Document::parse(bytes).expect("parses"));
    let old_class = old.as_ref().map(|document| rules.classify(PATH, document));
    let new_class = new.as_ref().map(|document| rules.classify(PATH, document));
    let old_version = old
        .as_ref()
        .zip(old_class.as_ref())
        .map(|(document, classification)| Version {
            document,
            classification,
        });
    let new_version = new
        .as_ref()
        .zip(new_class.as_ref())
        .map(|(document, classification)| Version {
            document,
            classification,
        });
    compare(PATH, old_version, new_version).expect("at least one side exists")
}

/// The elements and attributes change HTML may hold.
const ELEMENTS: &[&str] = &[
    "a", "code", "del", "details", "div", "ins", "li", "ol", "p", "pre", "summary",
];
const ATTRIBUTES: &[&str] = &[
    "class",
    "data-alert",
    "data-bytes",
    "data-block",
    "data-cells",
    "data-count",
    "data-depth",
    "data-diagram",
    "data-fallback",
    "data-file",
    "data-footnote",
    "data-gen",
    "data-header",
    "data-heading",
    "data-kind",
    "data-language",
    "data-level",
    "data-lines",
    "data-new",
    "data-new-line",
    "data-number",
    "data-old",
    "data-old-line",
    "data-path",
    "data-row",
    "data-side",
    "data-signals",
    "data-task",
    "data-view",
    "href",
    "id",
];

/// Checks that every tag is one change HTML writes, so no text from a
/// document became markup.
fn assert_only_own_tags(case: &str, html: &str) {
    let mut rest = html;
    while let Some(start) = rest.find('<') {
        let end = rest[start..].find('>').expect("a tag ends") + start;
        let tag = rest[start + 1..end].trim_start_matches('/');
        rest = &rest[end + 1..];
        let element = tag.split(' ').next().unwrap_or_default();
        assert!(ELEMENTS.contains(&element), "{case}: <{element}>");
        for attribute in tag.split(' ').skip(1) {
            let name = attribute.split('=').next().unwrap_or_default();
            let old_detail = name
                .strip_prefix("data-old-")
                .map(|detail| format!("data-{detail}"));
            let known = ATTRIBUTES.contains(&name)
                || old_detail.is_some_and(|detail| ATTRIBUTES.contains(&detail.as_str()))
                || !attribute.contains('=') && !name.starts_with("data-") && !name.is_empty();
            assert!(known, "{case}: attribute {name} in <{tag}>");
        }
    }
}

/// The value of `name="…"` in the opening tag that holds `id="<id>"`.
fn attribute_of(html: &str, id: &str, name: &str) -> Option<String> {
    let at = html.find(&format!(" id=\"{id}\""))?;
    let tag_end = html[at..].find('>')? + at;
    let tag = &html[at..tag_end];
    let start = tag.find(&format!(" {name}=\""))? + name.len() + 3;
    let end = tag[start..].find('"')? + start;
    Some(tag[start..end].to_owned())
}

#[test]
fn every_change_carries_the_json_names_of_its_kind_level_signals_and_view() {
    for (case, folder) in cases() {
        let document = run(&folder);
        if document.file != driftmark_core::diff::FileState::Modified {
            continue;
        }
        let html = changes(&document, "c");
        let json = serde_json::to_value(&document).expect("serializes");
        for (index, change) in json["changes"]
            .as_array()
            .expect("changes")
            .iter()
            .enumerate()
        {
            let id = format!("c.{index}");
            let get = |name: &str| attribute_of(&html, &id, name);
            assert_eq!(
                get("data-kind").as_deref(),
                change["kind"].as_str(),
                "{case} {id}"
            );
            assert_eq!(
                get("data-level").as_deref(),
                change["level"].as_str(),
                "{case} {id}"
            );
            let signals: Vec<&str> = change["signals"]
                .as_array()
                .expect("signals")
                .iter()
                .map(|signal| signal.as_str().expect("a name"))
                .collect();
            assert_eq!(get("data-signals"), Some(signals.join(" ")), "{case} {id}");
            assert_eq!(
                get("data-view").as_deref(),
                change["edit"]["view"].as_str(),
                "{case} {id}"
            );
            let side = if change["new"]["block"].is_null() {
                "old"
            } else {
                "new"
            };
            assert_eq!(
                get("data-block").as_deref(),
                change[side]["block"]["kind"]["type"].as_str(),
                "{case} {id}"
            );
        }
    }
}

#[test]
fn change_lists_match_their_golden_html() {
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("render/changes");
    fs::create_dir_all(&out).expect("create the output folder");
    let mut failures = Vec::new();
    let found = cases();
    assert!(found.len() >= 30, "{} cases", found.len());
    for (case, folder) in found {
        let document = run(&folder);
        let html = changes(&document, "c");
        fs::write(out.join(format!("{case}.html")), &html).expect("write the html");
        assert_only_own_tags(&case, &html);
        for index in 0..document.changes.len() {
            let modified = document.file == driftmark_core::diff::FileState::Modified;
            if modified {
                assert_eq!(
                    html.matches(&format!(" id=\"c.{index}\"")).count(),
                    1,
                    "{case}: change {index} has one element"
                );
            }
        }
        let golden = fs::read_to_string(fixtures().join(format!("render/changes/{case}.html")))
            .unwrap_or_default();
        if golden != html {
            failures.push(case);
        }
    }
    assert!(
        failures.is_empty(),
        "golden files differ: {failures:?}; fresh output is in {}",
        out.display()
    );
}
