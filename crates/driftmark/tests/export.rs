//! `driftmark export` run as a program on a small made-up repository
//! (`fixtures/export/`): what each kind of file holds, that it reaches
//! nothing outside itself, that its security policy names exactly its own
//! blocks, how it is cut down to its size limit, and its exit codes.
//!
//! The files it writes also go to `<target>/tmp/export/`, where the browser
//! test (`tests/browser/export.mjs`) opens them from disk.

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use sha2::{Digest, Sha256};
use support::{DAY, T0, TempDir, TestRepo};

/// Runs the program in `dir` and returns its exit code, standard output and
/// standard error.
fn driftmark(dir: &Path, args: &[&str]) -> (i32, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_driftmark"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("the program runs");
    (
        output.status.code().expect("an exit code"),
        String::from_utf8(output.stdout).expect("UTF-8 output"),
        String::from_utf8(output.stderr).expect("UTF-8 errors"),
    )
}

/// The fixture folder of one version.
fn fixture(version: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/export")
        .join(version)
}

/// Writes every file under `from` into the repo, at the same path.
fn copy_into(repo: &TestRepo, from: &Path, prefix: &str) {
    for entry in std::fs::read_dir(from).expect("read the fixture") {
        let entry = entry.expect("an entry");
        let name = entry.file_name().into_string().expect("a name");
        let path = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        if entry.file_type().expect("a type").is_dir() {
            copy_into(repo, &entry.path(), &path);
        } else {
            repo.write(&path, std::fs::read(entry.path()).expect("read a file"));
        }
    }
}

/// The fixture repository: `main` holds `base/`; `feature`, a day later,
/// holds `head/`. `main` is checked out.
fn repo() -> TestRepo {
    let repo = TestRepo::new("export");
    copy_into(&repo, &fixture("base"), "");
    repo.commit("base", T0);
    repo.git(&["checkout", "-q", "-b", "feature"]);
    repo.remove("docs/decisions/0002-old.md");
    copy_into(&repo, &fixture("head"), "");
    repo.commit("head", T0 + DAY);
    repo.git(&["checkout", "-q", "main"]);
    repo
}

/// Where the browser test finds the files.
fn pages_dir() -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("export");
    std::fs::create_dir_all(&dir).expect("make the pages folder");
    dir
}

/// Every value of `name="…"` in the file's tags.
fn attributes(html: &str, name: &str) -> Vec<String> {
    let marker = format!(" {name}=\"");
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(at) = rest.find(&marker) {
        rest = &rest[at + marker.len()..];
        let end = rest.find('"').expect("a closed attribute");
        out.push(rest[..end].to_owned());
        rest = &rest[end..];
    }
    out
}

/// The text between `<tag…>` and `</tag>`, for each element with that tag
/// written exactly as `open`.
fn blocks<'h>(html: &'h str, open: &str, close: &str) -> Vec<&'h str> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(at) = rest.find(open) {
        rest = &rest[at + open.len()..];
        let end = rest.find(close).expect("a closed element");
        out.push(&rest[..end]);
        rest = &rest[end..];
    }
    out
}

/// Whether an id is a page's or a section's: the letter, then digits.
fn page_id(id: &str, letter: char) -> bool {
    id.strip_prefix(letter)
        .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
}

fn hash(text: &str) -> String {
    format!(
        "'sha256-{}'",
        STANDARD.encode(Sha256::digest(text.as_bytes()))
    )
}

/// What every exported file must hold, whatever was exported:
///
/// - a security policy, first in `<head>`, that names exactly the hash of
///   each inline style sheet and script, and allows nothing else to load;
/// - every id once;
/// - every link to a place in the file, which exists, or to the web;
/// - no address that would load anything: no `src`, no `/static/`, no
///   `url(` other than the font's `data:` address;
/// - no notes and no "Seen".
fn check_self_contained(html: &str) {
    let policy = attributes(html, "content")
        .into_iter()
        .find(|value| value.starts_with("default-src"))
        .expect("a security policy");
    let csp_at = html
        .find("http-equiv=\"Content-Security-Policy\"")
        .expect("the policy is in the file");
    assert!(csp_at < html.find("<style>").expect("a style sheet"));
    assert!(csp_at < html.find("<script>").expect("a script"));
    let mut expected: Vec<String> = vec!["default-src 'none'".to_owned()];
    let styles = blocks(html, "<style>", "</style>");
    assert_eq!(styles.len(), 1);
    expected.push(format!("style-src {}", hash(styles[0])));
    let scripts: Vec<String> = blocks(html, "<script>", "</script>")
        .into_iter()
        .map(hash)
        .collect();
    expected.push(format!("script-src {}", scripts.join(" ")));
    if html.contains("data:font/woff2;base64,") {
        expected.push("font-src data:".to_owned());
    }
    expected.push("base-uri 'none'".to_owned());
    expected.push("form-action 'none'".to_owned());
    assert_eq!(policy, expected.join("; "));
    // Data blocks never run, so they need no hash.
    assert_eq!(
        html.matches("<script").count(),
        scripts.len() + html.matches("<script type=\"application/json\"").count()
    );
    let ids = attributes(html, "id");
    let unique: BTreeSet<&String> = ids.iter().collect();
    assert_eq!(unique.len(), ids.len(), "every id once");
    for href in attributes(html, "href") {
        if let Some(target) = href.strip_prefix('#') {
            assert!(
                unique.contains(&target.to_owned()),
                "`{href}` leads to no id in the file"
            );
        } else {
            assert!(
                href.starts_with("https://")
                    || href.starts_with("http://")
                    || href.starts_with("mailto:"),
                "`{href}` leads outside the file"
            );
        }
    }
    assert!(attributes(html, "src").is_empty(), "nothing is loaded");
    assert!(!html.contains("/static/"));
    assert!(!html.contains("url(\"/") && !html.contains("url(/"));
    assert!(
        !html.contains("class=\"acts\""),
        "no Seen and no Add a note"
    );
    assert!(!html.contains("data-notes"));
    assert!(!html.contains("EventSource"));
    assert!(!html.contains(" style=\""), "no style attribute");
}

#[test]
fn a_document_is_one_page_with_everything_inside() {
    let repo = repo();
    let (code, html, err) = driftmark(&repo.dir, &["export", "doc", "main", "docs/guide.md"]);
    assert_eq!(code, 0, "{err}");
    assert!(
        err.starts_with("driftmark export: wrote standard output: 1 document, "),
        "{err}"
    );
    check_self_contained(&html);
    std::fs::write(pages_dir().join("doc.html"), &html).unwrap();
    assert!(html.contains("<title>Field guide · driftmark</title>"));
    assert_eq!(html.matches("class=\"xpage").count(), 1);
    // Its own headings and footnotes carry its page's prefix; its links to
    // documents outside the file, its source link and its image are text.
    assert!(html.contains("<h2 id=\"R1-install\" data-k="));
    assert!(html.contains("href=\"#R1-fn:tides\""));
    assert!(html.contains("<span class=\"link-off\">cache decision</span>"));
    assert!(html.contains("<span class=\"link-off\">source</span>"));
    assert!(html.contains("<span class=\"link-off\">missing page</span>"));
    assert!(html.contains("<span class=\"image-off\">harbour map</span>"));
    // The page's code languages, and only those, come with Prism.
    assert!(html.contains(
        "{\"kind\":\"doc\",\"first\":\"R1\",\"prism\":{\"rust\":\"rust\",\"sh\":\"bash\"}}"
    ));
    assert_eq!(
        html.matches("<script>").count(),
        4,
        "the script, Prism, bash, rust"
    );
    assert!(
        !html.contains("id=\"SearchIndex\""),
        "one document needs no search"
    );
    // A drawn diagram brings the diagram font, inside the style sheet.
    if html.contains("<figure class=\"diagram\"") {
        assert!(html.contains("font-src data:"));
        assert!(html.contains("@font-face"));
    } else {
        assert!(!html.contains("@font-face"));
    }
    // The file says what it is.
    assert!(html.contains("<p class=\"xabout\">Exported by driftmark: this file opens offline, holds no notes, and stays as it is when the documents change.</p>"));
    assert!(html.contains(
        "<p class=\"kicker\"><code>docs/guide.md</code> at <code>main</code>, commit <code>"
    ));
}

#[test]
fn a_folder_has_an_index_and_links_between_its_pages() {
    let repo = repo();
    let out = TempDir::new("export-out");
    let file = out.path().join("folder.html");
    let file_arg = file.to_str().unwrap();
    let (code, stdout, err) = driftmark(
        &repo.dir,
        &["export", "folder", "feature", "docs", "-o", file_arg],
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(stdout, "");
    let html = std::fs::read_to_string(&file).unwrap();
    assert_eq!(
        err,
        format!(
            "driftmark export: wrote `{file_arg}`: 3 documents, {} bytes\n",
            html.len()
        )
    );
    check_self_contained(&html);
    std::fs::write(pages_dir().join("folder.html"), &html).unwrap();
    // The index, then one page per document in path order.
    let pages: Vec<String> = attributes(&html, "id")
        .into_iter()
        .filter(|id| id == "Index" || page_id(id, 'R'))
        .collect();
    assert_eq!(pages, ["Index", "R1", "R2", "R3"]);
    assert!(html.contains("<a class=\"x-title\" href=\"#R1\">0001 Cache lantern schedules</a>"));
    // Two documents have a section called Install; each keeps its own id,
    // and a link to the other document's section leads there.
    assert!(html.contains("<h2 id=\"R1-install\""));
    assert!(html.contains("<h2 id=\"R2-install\""));
    assert!(
        html.contains("<a href=\"#R2-install\">guide</a>"),
        "0001 links to the guide's section"
    );
    assert!(
        html.contains("<a href=\"#R1-decision\">cache decision</a>"),
        "the guide links to 0001's decision"
    );
    assert!(html.contains("<a href=\"#R2-night-watch\">field guide</a>"));
    // The old rule is not at `feature`, so its link is text.
    assert!(html.contains("<span class=\"link-off\">old rule</span>"));
    // Search covers the three reading pages.
    let index = blocks(
        &html,
        "<script type=\"application/json\" id=\"SearchIndex\">",
        "</script>",
    );
    let index: serde_json::Value = serde_json::from_str(index[0]).unwrap();
    assert_eq!(
        index["pages"],
        serde_json::json!([
            [
                "R1",
                "0001 Cache lantern schedules",
                "docs/decisions/0001-cache.md",
                "read"
            ],
            ["R2", "Field guide", "docs/guide.md", "read"],
            ["R3", "Tide tables", "docs/tides.md", "read"]
        ])
    );
    let words: Vec<&str> = index["words"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w.as_str().unwrap())
        .collect();
    let at = words.iter().position(|w| *w == "lantern").unwrap();
    assert_eq!(index["in"][at], "0.1.1", "in every page");
    let at = words.iter().position(|w| *w == "monday").unwrap();
    assert_eq!(index["in"][at], "2", "in the tide tables only");
}

#[test]
fn changes_hold_every_card_and_the_changed_documents_whole() {
    let repo = repo();
    let (code, html, err) = driftmark(&repo.dir, &["export", "changes", "main...feature"]);
    assert_eq!(code, 0, "{err}");
    check_self_contained(&html);
    std::fs::write(pages_dir().join("changes.html"), &html).unwrap();
    assert!(html.contains("<title>feature · driftmark</title>"));
    // One section per changed document, highest ranked first: the decision
    // record, then the deleted and the new documents, then the guide.
    let sections: Vec<String> = attributes(&html, "id")
        .into_iter()
        .filter(|id| page_id(id, 'D'))
        .collect();
    assert_eq!(sections, ["D1", "D2", "D3", "D4"]);
    let order: Vec<&str> = blocks(&html, "<p class=\"docchg-path\"><code>", "</code>");
    assert_eq!(
        order,
        [
            "docs/decisions/0001-cache.md",
            "docs/decisions/0002-old.md",
            "docs/tides.md",
            "docs/guide.md"
        ]
    );
    // Every document that still exists is in the file whole; the deleted one
    // is not, so its heading is plain text.
    let reading: Vec<String> = attributes(&html, "id")
        .into_iter()
        .filter(|id| page_id(id, 'R'))
        .collect();
    assert_eq!(reading, ["R1", "R3", "R4"]);
    assert!(html.contains("<h2 id=\"D1-t\"><a href=\"#R1\">0001 Cache lantern schedules</a></h2>"));
    assert!(html.contains("<h2 id=\"D2-t\">0002 Light lanterns by hand</h2>"));
    assert!(html.contains("<a href=\"#D1\">its changes</a>"));
    // Cards keep their ids and lose their buttons.
    assert!(html.contains("<li id=\"C1.0\" class=\"change\""));
    assert!(!html.contains("Arrives with notes"));
    assert!(!html.contains("class=\"xcut\""), "nothing was left out");
    assert!(html.contains("id=\"SearchIndex\""));
}

#[test]
fn folders_after_dashes_keep_only_their_documents() {
    let repo = repo();
    let (code, html, err) = driftmark(
        &repo.dir,
        &["export", "changes", "main..feature", "--", "docs/decisions"],
    );
    assert_eq!(code, 0, "{err}");
    check_self_contained(&html);
    let order: Vec<&str> = blocks(&html, "<p class=\"docchg-path\"><code>", "</code>");
    assert_eq!(
        order,
        ["docs/decisions/0001-cache.md", "docs/decisions/0002-old.md"]
    );
    assert!(html.contains(" · only in <code>docs/decisions</code>"));
}

#[test]
fn a_range_too_big_is_cut_down_and_says_so() {
    let repo = repo();
    let (_, whole, _) = driftmark(&repo.dir, &["export", "changes", "main...feature"]);
    // Room for every card but not every document's text: the full texts
    // go first, lowest ranked first.
    let (_, cards_only, _) = driftmark(
        &repo.dir,
        &["export", "changes", "main...feature", "--max-size", "1"],
    );
    let limit = usize::midpoint(whole.len(), cards_only.len());
    let limit_arg = limit.to_string();
    let (code, html, err) = driftmark(
        &repo.dir,
        &[
            "export",
            "changes",
            "main...feature",
            "--max-size",
            &limit_arg,
        ],
    );
    assert_eq!(code, 0, "{err}");
    check_self_contained(&html);
    assert!(html.len() <= limit, "{} > {limit}", html.len());
    assert!(html.contains("<div class=\"xcut\" role=\"note\"><p>This file holds every change of this range, and the full text of "));
    assert!(
        err.contains("\ndriftmark export: This file holds every change of this range"),
        "{err}"
    );
    std::fs::write(pages_dir().join("changes-cut.html"), &html).unwrap();
    // With no room for a single card, the file still lists every document.
    assert!(
        cards_only
            .contains("The file holds the changes of the 0 highest-ranked of its 4 documents")
    );
    assert!(cards_only.contains("4 more documents not in this file</h2>"));
    check_self_contained(&cards_only);
    // Room for some cards: the highest ranked come first, the others are
    // listed with their counts, and the file says so.
    let sections = |html: &str| html.matches("<section class=\"docchg\"").count();
    let mut limit = cards_only.len();
    let (html, err) = loop {
        limit += 500;
        let limit_arg = limit.to_string();
        let (code, html, err) = driftmark(
            &repo.dir,
            &[
                "export",
                "changes",
                "main...feature",
                "--max-size",
                &limit_arg,
            ],
        );
        assert_eq!(code, 0, "{err}");
        if sections(&html) > 0 {
            break (html, err);
        }
    };
    assert!(sections(&html) < 4);
    assert!(html.len() <= limit, "{} > {limit}", html.len());
    check_self_contained(&html);
    let kept = sections(&html);
    assert!(html.contains(&format!(
        "This range is too big for one file of at most {limit} bytes. The file holds the changes of the {kept} highest-ranked of its 4 documents"
    )));
    assert!(
        html.contains("<section id=\"D1\"") || html.contains("<section class=\"docchg\" id=\"D1\"")
    );
    assert!(err.contains("This range is too big"), "{err}");
}

#[test]
fn a_folder_too_big_keeps_its_whole_index() {
    let repo = repo();
    let (_, whole, _) = driftmark(&repo.dir, &["export", "folder", "feature", "."]);
    let limit = (whole.len() - 2_000).to_string();
    let (code, html, err) = driftmark(
        &repo.dir,
        &["export", "folder", "feature", ".", "--max-size", &limit],
    );
    assert_eq!(code, 0, "{err}");
    check_self_contained(&html);
    assert!(html.contains("This folder is too big for one file of at most "));
    assert!(html.contains("· not in this file</span>"));
    assert!(html.len() <= whole.len() - 2_000);
    // The index lists every document; only those the file holds are links.
    let pages = html.matches("class=\"xpage xread\"").count();
    let linked = html.matches("<a class=\"x-title\"").count();
    let listed = html.matches("<span class=\"x-title\">").count();
    assert!(pages > 0 && listed > 0);
    assert_eq!((pages, linked + listed), (linked, 3));
}

#[test]
fn empty_ranges_and_folders_still_make_a_file() {
    let repo = repo();
    let (code, html, err) = driftmark(&repo.dir, &["export", "changes", "main..main"]);
    assert_eq!(code, 0, "{err}");
    check_self_contained(&html);
    assert!(html.contains("<p class=\"note\">No document changed in this range.</p>"));
    assert!(!html.contains("class=\"attention\""));
    assert!(err.contains(": 0 documents, "), "{err}");
    let (code, html, err) = driftmark(&repo.dir, &["export", "folder", "main", "src"]);
    assert_eq!(code, 0, "{err}");
    check_self_contained(&html);
    assert!(html.contains("<p class=\"note\">This folder holds no Markdown document.</p>"));
}

#[test]
fn the_same_commits_give_the_same_bytes() {
    let repo = repo();
    let first = driftmark(&repo.dir, &["export", "changes", "main...feature"]);
    let second = driftmark(&repo.dir, &["export", "changes", "main...feature"]);
    assert_eq!(first.1, second.1);
}

#[test]
fn the_working_tree_can_be_exported() {
    let repo = repo();
    repo.write("docs/draft.md", "# Draft\n\nNot committed yet.\n");
    let (code, html, err) = driftmark(&repo.dir, &["export", "doc", "worktree", "docs/draft.md"]);
    assert_eq!(code, 0, "{err}");
    assert!(html.contains("in the working tree, as it was on disk"));
    let (code, html, err) = driftmark(&repo.dir, &["export", "changes", "auto", "--base", "main"]);
    assert_eq!(code, 0, "{err}");
    assert!(html.contains("<code>docs/draft.md</code>"));
    check_self_contained(&html);
}

#[test]
fn mistakes_have_exit_codes() {
    let repo = repo();
    let (code, _, err) = driftmark(&repo.dir, &["export", "doc", "nope", "docs/guide.md"]);
    assert_eq!(
        (code, err.as_str()),
        (1, "driftmark export: `nope` does not name a commit\n")
    );
    let (code, _, err) = driftmark(&repo.dir, &["export", "doc", "main", "docs/tides.md"]);
    assert_eq!(code, 1);
    assert_eq!(
        err,
        "driftmark export: `main` has no Markdown document `docs/tides.md`\n"
    );
    let (code, _, err) = driftmark(&repo.dir, &["export", "changes", "main...nope"]);
    assert_eq!(code, 1, "{err}");
    let (code, _, err) = driftmark(&repo.dir, &["export"]);
    assert_eq!(code, 2);
    assert!(
        err.starts_with("driftmark export: say what to export"),
        "{err}"
    );
    let (code, out, _) = driftmark(&repo.dir, &["export", "--help"]);
    assert_eq!(code, 0);
    assert!(out.starts_with("usage: driftmark export doc"));
    let outside = TempDir::new("export-nowhere");
    let (code, _, err) = driftmark(outside.path(), &["export", "doc", "main", "a.md"]);
    assert_eq!(code, 1);
    assert!(
        err.contains("is not inside a git repository that can be read"),
        "{err}"
    );
    let (code, _, err) = driftmark(
        &repo.dir,
        &[
            "export",
            "doc",
            "main",
            "docs/guide.md",
            "-o",
            "/nonexistent-folder/x.html",
        ],
    );
    assert_eq!(code, 1);
    assert!(
        err.starts_with("driftmark export: could not write `/nonexistent-folder/x.html`: "),
        "{err}"
    );
}

/// The sizes of the files, for the export document: printed with
/// `--nocapture`.
#[test]
fn sizes_of_the_fixture_files() {
    let repo = repo();
    let mut sizes = BTreeMap::new();
    for (name, args) in [
        ("doc", &["export", "doc", "main", "docs/guide.md"][..]),
        ("folder", &["export", "folder", "feature", "docs"][..]),
        ("changes", &["export", "changes", "main...feature"][..]),
    ] {
        let (_, html, _) = driftmark(&repo.dir, args);
        sizes.insert(name, html.len());
    }
    eprintln!("fixture export sizes: {sizes:?}");
    assert!(sizes.values().all(|size| *size < 1_000_000));
}
