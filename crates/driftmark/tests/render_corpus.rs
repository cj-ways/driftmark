//! Reading pages for every document of a real repo, for the checks the M2
//! render brief asks for. It is ignored in normal runs because it needs a
//! repo that is not in this one.
//!
//! It reads the repo with `git ls-tree` and `git cat-file` only, so it never
//! changes it, and it prints counts, sizes and timings only, never a path, a
//! name or any text from a file. With `DRIFTMARK_PAGES` set it also writes
//! every page, named by number only, with the blob ids in `blobs.txt` in the
//! same order, for `tests/browser/fuzz.mjs`. That folder must stay private.
//!
//! ```text
//! DRIFTMARK_CORPUS=/path/to/repo DRIFTMARK_CORPUS_REF=origin/main \
//! DRIFTMARK_CORPUS_ROOTS=docs DRIFTMARK_PAGES=/private/folder \
//! cargo test --release -p driftmark --test render_corpus -- --ignored --nocapture
//! ```

#[path = "../../driftmark-core/tests/support/html_text.rs"]
mod html_text;

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use driftmark_core::document::Document;
use driftmark_core::render::{
    ANCHOR_SCRIPT, LinkKind, LinkResolver, NoDiagrams, Resolved, Setup, reading,
};

/// Links to Markdown files lead to a document page; anything else is text.
struct DocumentLinks;

impl LinkResolver for DocumentLinks {
    fn resolve(&self, address: &str, kind: LinkKind) -> Resolved {
        let path = address.split('#').next().unwrap_or_default();
        let markdown = path
            .rsplit_once('.')
            .is_some_and(|(_, extension)| extension.eq_ignore_ascii_case("md"));
        if kind == LinkKind::Link && markdown {
            Resolved::Url(format!("/doc/{address}"))
        } else {
            Resolved::Text
        }
    }
}

/// Writes one page for the browser harness, named by number.
fn write_page(pages: &Path, number: usize, body: &str, table: &str) {
    let html = format!(
        "<!doctype html>\n<html><head><meta charset=\"utf-8\"></head><body><article>\n{body}</article>\n<script type=\"application/json\" id=\"Tables\">{}</script>\n<script src=\"anchor.js\"></script>\n<script>driftmarkAnchor.setTable(JSON.parse(document.getElementById('Tables').textContent), document.querySelector('article'));</script>\n</body></html>\n",
        table.replace("</", "<\\/")
    );
    std::fs::write(pages.join(format!("{number:04}.html")), html).expect("write a page");
}

/// The blob ids and bytes of the Markdown files under `roots` at `reference`.
fn read_blobs(repo: &PathBuf, reference: &str, roots: &[String]) -> Vec<(String, Vec<u8>)> {
    let listing = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["ls-tree", "-r", "-z", reference, "--"])
        .args(roots)
        .output()
        .expect("git ls-tree runs");
    assert!(listing.status.success(), "git ls-tree failed");
    let mut ids = Vec::new();
    for entry in listing.stdout.split(|&b| b == 0).filter(|e| !e.is_empty()) {
        let text = std::str::from_utf8(entry).expect("a UTF-8 listing");
        let (meta, path) = text.split_once('\t').expect("mode type id, tab, path");
        let fields: Vec<&str> = meta.split(' ').collect();
        let markdown = path.to_ascii_lowercase().ends_with(".md");
        if fields[1] == "blob" && fields[0] != "120000" && markdown {
            ids.push(fields[2].to_owned());
        }
    }
    let mut cat = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["cat-file", "--batch"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("git cat-file runs");
    let mut input = cat.stdin.take().expect("stdin");
    let wanted = ids.clone();
    let writer = std::thread::spawn(move || {
        for id in wanted {
            writeln!(input, "{id}").expect("an id is written");
        }
    });
    let mut output = BufReader::new(cat.stdout.take().expect("stdout"));
    let mut blobs = Vec::new();
    for id in ids {
        let mut header = String::new();
        output.read_line(&mut header).expect("a header");
        let size: usize = header
            .split_whitespace()
            .nth(2)
            .and_then(|size| size.parse().ok())
            .expect("a size");
        let mut bytes = vec![0; size + 1];
        output.read_exact(&mut bytes).expect("the blob");
        bytes.pop();
        blobs.push((id, bytes));
    }
    writer.join().expect("the writer finishes");
    assert!(cat.wait().expect("git cat-file exits").success());
    blobs
}

fn median_and_largest<T: Ord + Copy>(values: &mut [T]) -> (T, T) {
    values.sort_unstable();
    (values[values.len() / 2], values[values.len() - 1])
}

/// What the run measured, in numbers only.
#[derive(Default)]
struct Tally {
    refused: BTreeMap<String, usize>,
    wrong_blocks: usize,
    wrong_kinds: BTreeMap<String, usize>,
    checked_blocks: usize,
    parse_times: Vec<Duration>,
    render_times: Vec<Duration>,
    html_sizes: Vec<usize>,
    table_sizes: Vec<usize>,
}

impl Tally {
    /// Checks one rendered page's text against its document, block by block.
    fn check_text(&mut self, document: &Document, html: &str) {
        self.checked_blocks += html_text::shown(html).len();
        for (block, _, _) in html_text::compare(document, html) {
            self.wrong_blocks += 1;
            let kind = format!("{:?}", document.blocks[block].kind);
            let kind = kind.split([' ', '(', '{']).next().unwrap_or_default();
            *self.wrong_kinds.entry(kind.to_owned()).or_insert(0) += 1;
        }
    }

    fn print(&mut self, documents: usize, total: Duration) {
        let ms = |duration: Duration| duration.as_secs_f64() * 1000.0;
        let (parse_median, parse_largest) = median_and_largest(&mut self.parse_times);
        let (render_median, render_largest) = median_and_largest(&mut self.render_times);
        let (html_median, html_largest) = median_and_largest(&mut self.html_sizes);
        let (table_median, table_largest) = median_and_largest(&mut self.table_sizes);
        let html_total: usize = self.html_sizes.iter().sum();
        let table_total: usize = self.table_sizes.iter().sum();
        println!("documents: {documents}");
        println!("refused by the parser: {:?}", self.refused);
        println!("rendered: {}", self.render_times.len());
        println!(
            "blocks with an element checked: {}, text differs: {} {:?}",
            self.checked_blocks, self.wrong_blocks, self.wrong_kinds
        );
        println!(
            "parse ms per page: median {:.3}, largest {:.1}; render ms per page: median {:.3}, largest {:.1}; whole run {:.0} ms",
            ms(parse_median),
            ms(parse_largest),
            ms(render_median),
            ms(render_largest),
            ms(total)
        );
        println!(
            "html bytes per page: median {html_median}, largest {html_largest}, total {html_total}"
        );
        println!(
            "table bytes per page: median {table_median}, largest {table_largest}, total {table_total}"
        );
    }
}

#[test]
#[ignore = "needs a real repo: set DRIFTMARK_CORPUS"]
fn every_corpus_document_renders_with_its_text_and_tables() {
    let repo = PathBuf::from(std::env::var("DRIFTMARK_CORPUS").expect("DRIFTMARK_CORPUS"));
    let reference = std::env::var("DRIFTMARK_CORPUS_REF").unwrap_or_else(|_| "HEAD".to_owned());
    let roots: Vec<String> = std::env::var("DRIFTMARK_CORPUS_ROOTS")
        .unwrap_or_else(|_| "docs".to_owned())
        .split(',')
        .map(str::to_owned)
        .collect();
    let pages = std::env::var("DRIFTMARK_PAGES").ok().map(PathBuf::from);
    let blobs = read_blobs(&repo, &reference, &roots);
    let allowed = driftmark_core::lint::DEFAULT_ALLOWED_HTML
        .map(str::to_owned)
        .to_vec();
    let setup = Setup {
        links: &DocumentLinks,
        diagrams: &NoDiagrams,
        allowed_html: &allowed,
    };
    if let Some(pages) = &pages {
        std::fs::create_dir_all(pages).expect("create the pages folder");
        std::fs::write(pages.join("anchor.js"), ANCHOR_SCRIPT).expect("write the script");
    }
    let mut tally = Tally::default();
    let mut written = String::new();
    let started = Instant::now();
    for (id, bytes) in &blobs {
        let parse_start = Instant::now();
        let document = match Document::parse(bytes.clone()) {
            Ok(document) => document,
            Err(error) => {
                let name = format!("{error:?}");
                let name = name.split(' ').next().unwrap_or_default().to_owned();
                *tally.refused.entry(name).or_insert(0) += 1;
                continue;
            }
        };
        tally.parse_times.push(parse_start.elapsed());
        let render_start = Instant::now();
        let page = reading(&document, &setup);
        tally.render_times.push(render_start.elapsed());
        let table = serde_json::to_string(&page.positions).expect("tables serialize");
        tally.html_sizes.push(page.html.len());
        tally.table_sizes.push(table.len());
        tally.check_text(&document, &page.html);
        if let Some(pages) = &pages {
            write_page(pages, written.lines().count(), &page.html, &table);
            written.push_str(id);
            written.push('\n');
        }
    }
    let total = started.elapsed();
    if let Some(pages) = &pages {
        std::fs::write(pages.join("blobs.txt"), written).expect("write the blob list");
    }
    tally.print(blobs.len(), total);
    assert_eq!(
        tally.wrong_blocks, 0,
        "every block's page text matches its model"
    );
}
