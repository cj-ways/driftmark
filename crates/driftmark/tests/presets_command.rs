//! Presets in real git repositories: each made-up layout of
//! `/fixtures/presets/<name>/` is committed to a test repository and read by
//! the program, at the commit and in the working tree, through the same
//! rules that `driftmark diff`, `serve` and `mcp` use. Then a change under a
//! preset's decision heading must rank as "decision" in `driftmark diff`, and
//! `driftmark presets` must say which presets the repository uses.

mod support;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use driftmark::git::Git;
use driftmark::presets::{files_at, setup_at};
use driftmark_core::document::{BlockKind, Document};
use driftmark_core::git::{GitRead, RepoPath, Snapshot};
use driftmark_core::presets::find;
use driftmark_core::rules::Classification;
use support::{DAY, T0, TempDir, TestRepo};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/presets")
        .join(name)
}

/// Every file of a fixture tree, with its path from the tree's root.
fn tree(name: &str) -> Vec<(String, Vec<u8>)> {
    let mut found = Vec::new();
    let mut pending = vec![(fixture(name), String::new())];
    while let Some((folder, prefix)) = pending.pop() {
        for entry in std::fs::read_dir(&folder).expect("the folder lists") {
            let entry = entry.expect("an entry");
            let name = entry.file_name().into_string().expect("a UTF-8 name");
            let path = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            if entry.file_type().expect("a type").is_dir() {
                pending.push((entry.path(), path));
            } else {
                found.push((path, std::fs::read(entry.path()).expect("the file reads")));
            }
        }
    }
    found.sort();
    found
}

/// A repository holding one fixture tree, committed on `main`.
fn repo_of(name: &str) -> (TestRepo, String) {
    let repo = TestRepo::new(&format!("presets-{name}"));
    for (path, bytes) in tree(name) {
        repo.write(&path, bytes);
    }
    let commit = repo.commit("the layout", T0);
    (repo, commit)
}

/// Runs the program in `dir`: its exit code, standard output and standard
/// error.
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

/// One file's classification on one line, as the core's fixture test writes
/// it in `expected.txt`.
fn line(document: &Document, class: &Classification) -> String {
    let headings: Vec<&str> = class
        .decision_blocks
        .iter()
        .map(|&index| &document.blocks[index])
        .filter(|block| matches!(block.kind, BlockKind::Heading { .. }))
        .map(|block| block.text.as_str())
        .collect();
    let or_none = |value: &Option<String>| value.clone().unwrap_or_else(|| "-".to_owned());
    format!(
        "type {} | status {} | lifecycle {} | date {} | decisions {}",
        class.doc_type,
        or_none(&class.status),
        class.lifecycle.map_or_else(
            || "-".to_owned(),
            |lifecycle| format!("{lifecycle:?}").to_lowercase()
        ),
        or_none(&class.date),
        if headings.is_empty() {
            "-".to_owned()
        } else {
            format!(
                "{} blocks under {}",
                class.decision_blocks.len(),
                headings.join(", ")
            )
        }
    )
}

/// What the program reads at one side, written out as `expected.txt` is.
fn dump(git: &Git, side: &Snapshot) -> String {
    let setup = setup_at(git, side).expect("the rules load");
    let mut out = String::new();
    for in_use in &setup.presets {
        let folders = if in_use.folders.is_empty() {
            String::new()
        } else {
            format!(", also reading {}", in_use.folders.join(", "))
        };
        let found = in_use.found.as_deref().unwrap_or("its name");
        let _ = writeln!(
            out,
            "preset {} found by {found}{folders}",
            in_use.preset.name
        );
    }
    out.push('\n');
    let files = files_at(git, side);
    let roots = driftmark_core::git::Roots::all();
    for file in git.markdown_files(side, &roots).expect("the files list") {
        let bytes = match side {
            Snapshot::Commit(_) => git.read_blob(&file.id),
            Snapshot::Worktree => git.read_worktree_file(&file.path),
        }
        .expect("the file reads");
        let document = Document::parse_with(bytes, setup.rules.masthead_keys()).expect("parses");
        let path = file.path.to_lossy();
        let class = setup.rules.classify_with(&path, &document, &files);
        let _ = writeln!(out, "{path}\n    {}", line(&document, &class));
    }
    out
}

#[test]
fn each_layout_reads_the_same_at_a_commit_and_in_the_working_tree() {
    for name in ["openspec", "spec-kit", "adr", "kiro"] {
        let (repo, commit) = repo_of(name);
        let expected =
            std::fs::read_to_string(fixture(name).join("expected.txt")).expect("expected.txt");
        let git = Git::open(&repo.dir).expect("the repo opens");
        let commit = driftmark_core::git::ObjectId::from_hex(&commit).expect("an id");
        assert_eq!(
            dump(&git, &Snapshot::Commit(commit)),
            expected,
            "{name} at the commit"
        );
        assert_eq!(
            dump(&git, &Snapshot::Worktree),
            expected,
            "{name} in the working tree"
        );
    }
}

/// The level of each change `driftmark diff` lists for `path`, in reading
/// order.
fn levels(dir: &Path, range: &str, path: &str) -> Vec<String> {
    let (code, out, err) = driftmark(dir, &["diff", "--json", range]);
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    let list: serde_json::Value = serde_json::from_str(&out).expect("JSON");
    let document = list["documents"]
        .as_array()
        .expect("documents")
        .iter()
        .find(|document| document["path"] == path)
        .unwrap_or_else(|| panic!("{path} changed"));
    document["changes"]
        .as_array()
        .expect("changes")
        .iter()
        .map(|change| change["level"].as_str().expect("a level").to_owned())
        .collect()
}

/// Commits one edit of `path` on a branch from `main`, replacing each
/// `(old, new)` pair once.
fn edit(repo: &TestRepo, path: &str, edits: &[(&str, &str)]) {
    repo.git(&["checkout", "-q", "-b", "edit"]);
    let mut text = String::from_utf8(std::fs::read(repo.dir.join(path)).expect("the file reads"))
        .expect("UTF-8");
    for (old, new) in edits {
        assert!(text.contains(old), "{path} holds {old:?}");
        text = text.replacen(old, new, 1);
    }
    repo.write(path, text);
    repo.commit("an edit", T0 + DAY);
}

/// An edit of one document of a layout, and the levels its changes must get.
struct Ranked {
    preset: &'static str,
    path: &'static str,
    edits: &'static [(&'static str, &'static str)],
    levels: &'static [&'static str],
}

#[test]
fn a_change_under_a_decision_heading_ranks_as_a_decision() {
    let cases = [
        Ranked {
            preset: "adr",
            path: "doc/adr/0001-record-architecture-decisions.md",
            edits: &[
                (
                    "keep decision records in this folder",
                    "keep decision records in the doc folder",
                ),
                (
                    "Each decision has a short record.",
                    "Each decision has its own short record.",
                ),
            ],
            levels: &["decision", "wording"],
        },
        Ranked {
            preset: "openspec",
            path: "openspec/changes/add-tags/specs/notes/spec.md",
            edits: &[("add tags to a note", "add one or more tags to a note")],
            levels: &["decision"],
        },
        Ranked {
            preset: "spec-kit",
            path: "specs/001-albums/spec.md",
            edits: &[
                ("picks photos and puts them", "picks photos and drops them"),
                (
                    "let a person make an album",
                    "let a person make an album from the grid",
                ),
            ],
            levels: &["wording", "decision"],
        },
        Ranked {
            preset: "kiro",
            path: ".kiro/specs/meal-plan/requirements.md",
            edits: &[(
                "SHALL show seven days",
                "SHALL show seven days and their meals",
            )],
            levels: &["decision"],
        },
    ];
    for case in cases {
        let (repo, _) = repo_of(case.preset);
        edit(&repo, case.path, case.edits);
        assert_eq!(
            levels(&repo.dir, "main..edit", case.path),
            case.levels,
            "{}",
            case.preset
        );
    }
}

#[test]
fn the_presets_command_says_which_presets_a_repository_uses() {
    let (repo, _) = repo_of("adr");
    let (code, out, err) = driftmark(&repo.dir, &["presets"]);
    assert_eq!((code, err.as_str()), (0, ""));
    assert_eq!(
        out,
        "\
openspec   OpenSpec: living specs, open changes and the archive
           not found here
spec-kit   GitHub Spec Kit: one folder per feature under specs/
           not found here
adr        ADR folders: adr-tools, MADR, log4brains and Nygard-style records
           in use: found .adr-dir
           also reads the records in architecture/records, packages/checkout/docs/adr, named by .adr-dir or .log4brains.yml
kiro       Kiro and cc-sdd: specs and steering under .kiro/
           not found here
starter    driftmark's starter layout, as driftmark init sets it up
           used only when driftmark.toml names it

This repository has no driftmark.toml, so driftmark uses every preset it finds.
To keep this choice, or to add types of your own, write this line in driftmark.toml:

  presets = [\"adr\"]

To see a preset's rules, ready to copy, run `driftmark presets show <name>`.
"
    );

    // A rules file turns finding off: only the presets it names are used.
    repo.write("driftmark.toml", "[lint]\nexclude = [\"packages/**\"]\n");
    let (_, out, _) = driftmark(&repo.dir, &["presets"]);
    assert!(
        out.contains("found .adr-dir, but not in use: driftmark.toml does not name it\n"),
        "{out}"
    );
    assert!(
        out.contains("driftmark.toml names no preset, so driftmark uses its own types only."),
        "{out}"
    );
    repo.write("driftmark.toml", "presets = [\"adr\", \"kiro\"]\n");
    let (_, out, _) = driftmark(&repo.dir, &["presets"]);
    assert!(out.contains("adr        ADR folders"), "{out}");
    assert!(
        out.contains("           in use: driftmark.toml names it\n           also reads"),
        "{out}"
    );
    assert!(
        out.contains(
            "driftmark.toml names the presets \"adr\", \"kiro\". Its own types come first."
        ),
        "{out}"
    );

    let empty = TestRepo::new("presets-empty");
    empty.write("README.md", "# Readme\n");
    empty.commit("first", T0);
    let (_, out, _) = driftmark(&empty.dir, &["presets"]);
    assert!(
        out.contains("No preset's layout is in this repository."),
        "{out}"
    );

    let outside = TempDir::new("presets-outside");
    let (code, _, err) = driftmark(outside.path(), &["presets"]);
    assert_eq!(code, 2);
    assert!(
        err.starts_with("driftmark presets: run it inside a git repository"),
        "{err}"
    );
}

#[test]
fn show_prints_a_preset_ready_to_copy() {
    let adr = find("adr").expect("adr");
    let outside = TempDir::new("presets-show");
    let (code, out, _) = driftmark(outside.path(), &["presets", "show", "adr"]);
    assert_eq!((code, out.as_str()), (0, adr.text));
    // In a repository, the folders named in its marker files are written in.
    let (repo, _) = repo_of("adr");
    let (code, out, _) = driftmark(&repo.dir, &["presets", "show", "adr"]);
    assert_eq!(code, 0);
    assert!(
        out.contains("  \"architecture/records/[0-9]*.md\",\n"),
        "{out}"
    );
    assert!(
        out.contains("  \"packages/checkout/docs/adr/[0-9]*.md\",\n"),
        "{out}"
    );
    // What it prints is a valid rules file that types the repository's
    // records as the preset does.
    repo.write("driftmark.toml", &out);
    let git = Git::open(&repo.dir).expect("opens");
    let setup = setup_at(&git, &Snapshot::Worktree).expect("the copy is valid");
    assert_eq!(setup.presets, []);
    let path = "architecture/records/0001-split-the-shop-into-two-services.md";
    assert_eq!(
        setup.rules.type_for(path).map(|rule| rule.name.as_str()),
        Some("adr")
    );
    let (code, _, err) = driftmark(&repo.dir, &["presets", "show", "nope"]);
    assert_eq!(code, 2);
    assert_eq!(
        err,
        "driftmark presets: driftmark has no preset \"nope\"; the presets are openspec, spec-kit, adr, kiro, starter\n"
    );
    let (code, out, _) = driftmark(&repo.dir, &["presets", "--help"]);
    assert_eq!(code, 0);
    assert!(out.starts_with("usage: driftmark presets"), "{out}");
    let (code, _, err) = driftmark(&repo.dir, &["presets", "show"]);
    assert_eq!(code, 2);
    assert!(
        err.starts_with("driftmark presets: `driftmark presets show` needs a preset's name"),
        "{err}"
    );
}

#[test]
fn a_rules_file_adds_its_own_types_before_the_presets_it_names() {
    let (repo, _) = repo_of("kiro");
    repo.write(
        "driftmark.toml",
        "presets = [\"kiro\"]\n\n[[type]]\nname = \"template\"\npaths = [\".kiro/settings/templates/**\"]\nlifecycle = \"current\"\n",
    );
    let git = Git::open(&repo.dir).expect("opens");
    let setup = setup_at(&git, &Snapshot::Worktree).expect("valid");
    let used: Vec<(&str, Option<&str>)> = setup
        .presets
        .iter()
        .map(|in_use| (in_use.preset.name, in_use.found.as_deref()))
        .collect();
    assert_eq!(used, [("kiro", None)]);
    let type_of = |path: &str| setup.rules.type_for(path).map(|rule| rule.name.clone());
    assert_eq!(
        type_of(".kiro/settings/templates/specs/requirements.md").as_deref(),
        Some("template")
    );
    assert_eq!(
        type_of(".kiro/specs/meal-plan/requirements.md").as_deref(),
        Some("requirements")
    );

    // A preset driftmark does not know is an error that names its line.
    repo.write("driftmark.toml", "presets = [\"kiro\", \"backlog\"]\n");
    let (code, _, err) = driftmark(&repo.dir, &["diff", "auto"]);
    assert_eq!(code, 1);
    assert!(
        err.starts_with("driftmark diff: driftmark.toml in the working tree: the rules file could not be read: TOML parse error at line 1"),
        "{err}"
    );
    assert!(
        err.contains("driftmark has no preset \"backlog\"; the presets are \"openspec\", \"spec-kit\", \"adr\", \"kiro\", \"starter\""),
        "{err}"
    );
}

#[test]
fn the_writing_check_uses_the_presets_found() {
    let (repo, _) = repo_of("spec-kit");
    repo.write(
        "specs/004-print/spec.md",
        "# Feature Specification: Print\n\n**Feature Branch**: `004-print`  \n**Created**: 2026-03-01  \n**Status**: Wobbly\n",
    );
    let (code, out, _) = driftmark(&repo.dir, &["lint", "--json", "specs/004-print/spec.md"]);
    assert!(code == 0 || code == 1, "{out}");
    let report: serde_json::Value = serde_json::from_str(&out).expect("JSON");
    let rules: Vec<&str> = report["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|finding| finding["rule"].as_str().expect("a rule"))
        .collect();
    assert!(rules.contains(&"status-value"), "{out}");
}

#[test]
fn a_file_beside_a_document_is_read_at_each_commit() {
    // cc-sdd's phase lives in spec.json. Each commit's own spec.json counts.
    let (repo, first) = repo_of("kiro");
    let spec = ".kiro/specs/shopping-list/spec.json";
    let text = String::from_utf8(std::fs::read(repo.dir.join(spec)).expect("reads"))
        .expect("UTF-8")
        .replace("design-generated", "tasks-generated");
    repo.write(spec, text);
    let second = repo.commit("the tasks are written", T0 + DAY);
    let git = Git::open(&repo.dir).expect("opens");
    let path = ".kiro/specs/shopping-list/design.md";
    let status = |commit: &str| {
        let side =
            Snapshot::Commit(driftmark_core::git::ObjectId::from_hex(commit).expect("an id"));
        let setup = setup_at(&git, &side).expect("valid");
        let Snapshot::Commit(id) = &side else {
            unreachable!()
        };
        let blob = git
            .blob_at(id, &RepoPath::new(path).expect("a path"))
            .expect("looks up")
            .expect("the file is there");
        let document = Document::parse_with(
            git.read_blob(&blob).expect("reads"),
            setup.rules.masthead_keys(),
        )
        .expect("parses");
        setup
            .rules
            .classify_with(path, &document, &files_at(&git, &side))
            .status
    };
    assert_eq!(status(&first).as_deref(), Some("design-generated"));
    assert_eq!(status(&second).as_deref(), Some("tasks-generated"));
}

/// One GET request to a local `driftmark serve`, and the answer's body.
fn get(port: u16, target: &str) -> String {
    use std::io::{Read, Write};
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
    let request =
        format!("GET {target} HTTP/1.1\r\nHost: localhost:{port}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).expect("send");
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).expect("read");
    let raw = String::from_utf8(raw).expect("UTF-8");
    let (head, body) = raw.split_once("\r\n\r\n").expect("a head and a body");
    assert!(head.starts_with("HTTP/1.1 200"), "{target}: {head}");
    if !head
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        return body.to_owned();
    }
    let mut rest = body;
    let mut out = String::new();
    while let Some((size, after)) = rest.split_once("\r\n") {
        let size = usize::from_str_radix(size.trim(), 16).expect("a chunk size");
        if size == 0 {
            break;
        }
        out.push_str(&after[..size]);
        rest = &after[size + 2..];
    }
    out
}

#[test]
fn reading_pages_show_a_status_from_spec_json() {
    use std::io::BufRead;
    let (repo, _) = repo_of("kiro");
    let mut server = Command::new(env!("CARGO_BIN_EXE_driftmark"))
        .args(["serve", "--addr", "127.0.0.1:0", "--repo"])
        .arg(&repo.dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("the server starts");
    let mut lines = std::io::BufReader::new(server.stdout.take().expect("standard output"));
    let mut port = None;
    let mut line = String::new();
    while lines.read_line(&mut line).expect("a line") > 0 {
        if let Some(rest) = line.strip_prefix("driftmark serve: open http://127.0.0.1:") {
            port = rest
                .split('/')
                .next()
                .and_then(|port| port.parse::<u16>().ok());
        }
        if line.contains("press Ctrl-C to stop") {
            break;
        }
        line.clear();
    }
    std::thread::spawn(move || std::io::copy(&mut lines, &mut std::io::sink()));
    let port = port.expect("the server says its port");
    let path = ".kiro/specs/shopping-list/design.md";
    let json: serde_json::Value =
        serde_json::from_str(&get(port, &format!("/api/doc/main/-/{path}"))).expect("JSON");
    assert_eq!(json["type"], "design");
    assert_eq!(json["status"], "design-generated");
    let page = get(port, &format!("/doc/main/-/{path}"));
    assert!(
        page.contains("data-pill=\"status\">Status: design-generated<"),
        "{page}"
    );
    assert!(
        page.contains("data-pill=\"date\">Dated 2026-02-03T16:30:00Z<"),
        "{page}"
    );
    let _ = server.kill();
    let _ = server.wait();
}

/// Each Markdown file of the working tree with what the rules say about it.
fn classes(git: &Git) -> Vec<(String, Classification)> {
    let side = Snapshot::Worktree;
    let setup = setup_at(git, &side).expect("the rules load");
    let files = files_at(git, &side);
    git.markdown_files(&side, &driftmark_core::git::Roots::all())
        .expect("the files list")
        .into_iter()
        .map(|file| {
            let bytes = git.read_worktree_file(&file.path).expect("the file reads");
            let document =
                Document::parse_with(bytes, setup.rules.masthead_keys()).expect("parses");
            let path = file.path.to_lossy().into_owned();
            let class = setup.rules.classify_with(&path, &document, &files);
            (path, class)
        })
        .collect()
}

#[test]
fn the_starter_preset_reads_an_init_layout_as_its_own_rules_file_does() {
    let repo = TestRepo::new("presets-starter");
    repo.write("README.md", "# Readme\n");
    repo.commit("first", T0);
    let (code, _, err) = driftmark(&repo.dir, &["init", "--write"]);
    assert_eq!((code, err.as_str()), (0, ""));
    for (kind, title) in [
        ("decision", "Keep one database"),
        ("standard", "Name things plainly"),
        ("feature", "Export to PDF"),
        ("plan", "Move the queue"),
        ("status", "Move the queue"),
    ] {
        let (code, _, err) = driftmark(&repo.dir, &["new", kind, title]);
        assert_eq!((code, err.as_str()), (0, ""), "{kind}");
    }
    let git = Git::open(&repo.dir).expect("opens");

    // The rules file `init` wrote is the starter preset's own text.
    let written = std::fs::read_to_string(repo.dir.join("driftmark.toml")).expect("reads");
    assert_eq!(written, find("starter").expect("starter").text);
    let own = setup_at(&git, &Snapshot::Worktree).expect("valid");
    assert_eq!(own.presets, []);
    let own_classes = classes(&git);
    let (_, own_lint, _) = driftmark(&repo.dir, &["lint"]);

    repo.write("driftmark.toml", "presets = [\"starter\"]\n");
    let named = setup_at(&git, &Snapshot::Worktree).expect("valid");
    let used: Vec<(&str, Option<&str>)> = named
        .presets
        .iter()
        .map(|in_use| (in_use.preset.name, in_use.found.as_deref()))
        .collect();
    assert_eq!(used, [("starter", None)]);
    assert_eq!(named.rules.types(), own.rules.types());
    assert_eq!(named.rules.masthead_keys(), own.rules.masthead_keys());
    // The starter text's `[lint]` table is not applied, but it writes out the
    // defaults, so the writing check's settings are the same.
    assert_eq!(named.rules.lint(), own.rules.lint());
    assert_eq!(classes(&git), own_classes);
    let (code, named_lint, _) = driftmark(&repo.dir, &["lint"]);
    assert_eq!(named_lint, own_lint);
    assert_eq!(code, 0, "{named_lint}");

    // The comparison is not empty: every kind of the layout is typed.
    let mut types: Vec<&str> = own_classes
        .iter()
        .map(|(_, class)| class.doc_type.as_str())
        .collect();
    types.sort_unstable();
    types.dedup();
    for wanted in [
        "decision", "feature", "plan", "readme", "standard", "status",
    ] {
        assert!(types.contains(&wanted), "{wanted} in {types:?}");
    }
    let (_, record) = own_classes
        .iter()
        .find(|(path, _)| path.starts_with("docs/decisions/0001-"))
        .expect("the new decision record");
    assert_eq!(record.doc_type, "decision");
    assert!(
        record.status.is_some() && record.date.is_some(),
        "{record:?}"
    );
    assert!(!record.decision_blocks.is_empty(), "{record:?}");
}
