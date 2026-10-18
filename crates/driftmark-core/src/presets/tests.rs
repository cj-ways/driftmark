use std::sync::Mutex;

use super::*;
use crate::document::Document;
use crate::lint::LintSettings;
use crate::rules::Lifecycle;

/// A made-up repository in memory, which remembers the folders it was asked
/// to list.
struct Repo {
    files: Vec<(String, Vec<u8>)>,
    asked: Mutex<Vec<Vec<String>>>,
}

impl Repo {
    fn new(files: &[(&str, &str)]) -> Repo {
        Repo {
            files: files
                .iter()
                .map(|(path, text)| ((*path).to_owned(), text.as_bytes().to_vec()))
                .collect(),
            asked: Mutex::new(Vec::new()),
        }
    }

    fn with_bytes(path: &str, bytes: &[u8]) -> Repo {
        Repo {
            files: vec![(path.to_owned(), bytes.to_vec())],
            asked: Mutex::new(Vec::new()),
        }
    }

    fn asked(&self) -> Vec<Vec<String>> {
        self.asked.lock().expect("the lock").clone()
    }
}

impl RepoFiles for Repo {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.files
            .iter()
            .find(|(name, _)| name == path)
            .map(|(_, bytes)| bytes.clone())
    }

    fn markdown_in(&self, folders: &[&str]) -> Vec<String> {
        self.asked
            .lock()
            .expect("the lock")
            .push(folders.iter().map(|folder| (*folder).to_owned()).collect());
        self.files
            .iter()
            .map(|(path, _)| path)
            .filter(|path| {
                crate::git::RepoPath::new(path.as_str()).is_some_and(|path| path.is_markdown())
            })
            .filter(|path| {
                folders
                    .iter()
                    .any(|folder| folder.is_empty() || path.starts_with(&format!("{folder}/")))
            })
            .rev()
            .cloned()
            .collect()
    }
}

fn names(found: &[Found]) -> Vec<(&str, &str)> {
    found
        .iter()
        .map(|found| (found.preset.name, found.marker.as_str()))
        .collect()
}

fn type_of(rules: &Rules, path: &str) -> Option<String> {
    rules.type_for(path).map(|rule| rule.name.clone())
}

#[test]
fn every_built_in_preset_is_a_valid_rules_file_of_types_only() {
    let mut seen = Vec::new();
    for preset in PRESETS {
        assert!(!seen.contains(&preset.name), "{} twice", preset.name);
        seen.push(preset.name);
        assert_ne!(preset.title, "");
        assert!(preset.text.ends_with('\n'), "{}", preset.name);
        let rules = preset.rules(&[]).expect("the preset parses");
        assert!(!rules.types().is_empty(), "{}", preset.name);
        // These tables are not applied when a preset is used, so a preset
        // that set them would mislead its reader.
        assert_eq!(rules.lint(), &LintSettings::default(), "{}", preset.name);
        assert_eq!(rules.default_branch(), None, "{}", preset.name);
        assert_eq!(rules.presets(), [] as [String; 0], "{}", preset.name);
        assert_eq!(find(preset.name), Some(preset));
    }
    assert_eq!(seen, ["openspec", "spec-kit", "adr", "kiro", "starter"]);
    assert_eq!(find("nope"), None);
    assert_ne!(PRESETS[0], PRESETS[1]);
    assert!(format!("{:?}", PRESETS[0]).contains("openspec"));
}

#[test]
fn each_preset_types_its_layout_by_path() {
    let preset = |name: &str| find(name).expect("a preset").rules(&[]).expect("valid");
    let openspec = preset("openspec");
    for (path, wanted) in [
        (
            "openspec/changes/archive/2026-01-02-add-x/proposal.md",
            "archived-change",
        ),
        (
            "openspec/changes/archive/2026-01-02-add-x/specs/auth/spec.md",
            "archived-change",
        ),
        (
            "openspec/changes/add-x/specs/auth/login/spec.md",
            "delta-spec",
        ),
        ("openspec/changes/add-x/proposal.md", "proposal"),
        ("openspec/changes/add-x/design.md", "design"),
        ("openspec/changes/add-x/tasks.md", "tasks"),
        ("openspec/specs/auth/spec.md", "spec"),
        ("openspec/project.md", "project"),
    ] {
        assert_eq!(type_of(&openspec, path).as_deref(), Some(wanted), "{path}");
    }
    assert_eq!(type_of(&openspec, "openspec/changes/README.md"), None);
    let spec_kit = preset("spec-kit");
    for (path, wanted) in [
        (".specify/memory/constitution.md", "constitution"),
        ("memory/constitution.md", "constitution"),
        ("specs/001-login/spec.md", "feature-spec"),
        ("specs/001-login/plan.md", "plan"),
        ("specs/001-login/tasks.md", "tasks"),
        ("specs/001-login/research.md", "research"),
        ("specs/001-login/data-model.md", "data-model"),
        ("specs/001-login/quickstart.md", "quickstart"),
        ("specs/001-login/contracts/api.md", "contract"),
        ("specs/001-login/checklists/requirements.md", "checklist"),
    ] {
        assert_eq!(type_of(&spec_kit, path).as_deref(), Some(wanted), "{path}");
    }
    let adr = preset("adr");
    for path in [
        "docs/adr/0001-use-x.md",
        "doc/adr/0002-use-y.md",
        "docs/decisions/0003-z.md",
        "docs/adr/20240926-use-git-flow.md",
        "adr/ADR-012-cache.md",
        "docs/architecture/decisions/adr-7-queue.md",
    ] {
        assert_eq!(type_of(&adr, path).as_deref(), Some("adr"), "{path}");
    }
    for path in [
        "docs/adr/README.md",
        "docs/adr/template.md",
        "notes/0001-x.md",
    ] {
        assert_eq!(type_of(&adr, path), None, "{path}");
    }
    let kiro = preset("kiro");
    for (path, wanted) in [
        (".kiro/specs/albums/requirements.md", "requirements"),
        (".kiro/specs/crash/bugfix.md", "bugfix"),
        (".kiro/specs/albums/design.md", "design"),
        (".kiro/specs/albums/tasks.md", "tasks"),
        (".kiro/specs/albums/research.md", "research"),
        (".kiro/specs/albums/brief.md", "brief"),
        (".kiro/steering/product.md", "steering"),
    ] {
        assert_eq!(type_of(&kiro, path).as_deref(), Some(wanted), "{path}");
    }
}

#[test]
fn layouts_are_found_by_their_markers_in_preset_order() {
    assert_eq!(names(&detect(&Repo::new(&[("README.md", "# R\n")]))), []);
    let repo = Repo::new(&[
        (".kiro/steering/product.md", "# P\n"),
        ("docs/adr/0001-use-x.md", "# 1. Use X\n"),
        ("docs/adr/README.md", "# ADRs\n"),
        ("openspec/changes/add-x/proposal.md", "# P\n"),
        ("openspec/config.yaml", "schema: spec-driven\n"),
        (".specify/memory/constitution.md", "# C\n"),
    ]);
    assert_eq!(
        names(&detect(&repo)),
        [
            ("openspec", "openspec/config.yaml"),
            ("spec-kit", ".specify/memory/constitution.md"),
            ("adr", "docs/adr/0001-use-x.md"),
            ("kiro", ".kiro/steering/product.md"),
        ]
    );
    // One listing of every marker folder serves all presets.
    let asked = repo.asked();
    assert_eq!(asked.len(), 1);
    assert!(
        asked[0].contains(&"openspec/changes/archive".to_owned()),
        "{asked:?}"
    );
    assert!(asked[0].contains(&".kiro/steering".to_owned()), "{asked:?}");
    assert!(
        asked[0].windows(2).all(|pair| pair[0] < pair[1]),
        "{asked:?}"
    );
}

#[test]
fn each_kind_of_marker_finds_its_layout() {
    let found = |files: &[(&str, &str)]| {
        names(&detect(&Repo::new(files)))
            .first()
            .map(|(name, marker)| ((*name).to_owned(), (*marker).to_owned()))
    };
    let pair = |name: &str, marker: &str| Some((name.to_owned(), marker.to_owned()));
    assert_eq!(
        found(&[("openspec/changes/add-x/proposal.md", "# P\n")]),
        pair("openspec", "openspec/changes/add-x/proposal.md")
    );
    assert_eq!(
        found(&[("openspec/project.md", "# P\n")]),
        pair("openspec", "openspec/project.md")
    );
    assert_eq!(
        found(&[("memory/constitution.md", "# C\n")]),
        pair("spec-kit", "memory/constitution.md")
    );
    assert_eq!(
        found(&[("specs/001-login/spec.md", "# S\n")]),
        pair("spec-kit", "specs/001-login/spec.md")
    );
    assert_eq!(found(&[("specs/api/spec.md", "# S\n")]), None);
    assert_eq!(
        found(&[(".adr-dir", "decisions\n")]),
        pair("adr", ".adr-dir")
    );
    assert_eq!(
        found(&[(".log4brains.yml", "project:\n  adrFolder: ./docs/adr\n")]),
        pair("adr", ".log4brains.yml")
    );
    assert_eq!(found(&[("docs/adr/README.md", "# A\n")]), None);
    assert_eq!(
        found(&[
            (".kiro/specs/b/tasks.md", "# T\n"),
            (".kiro/specs/a/design.md", "# D\n"),
        ]),
        pair("kiro", ".kiro/specs/a/design.md")
    );
}

/// A list of made-up presets with file markers only, and one with a text
/// that is not a valid rules file.
const ODD: &[Preset] = &[
    Preset {
        name: "files-only",
        title: "a preset found by a file",
        text: "[[type]]\nname = \"x\"\npaths = [\"x/*.md\"]\n",
        markers: &[Marker::File("x/marker")],
        folders: Folders::None,
    },
    Preset {
        name: "broken",
        title: "a preset whose text does not parse",
        text: "[[type]]\nname = \"y\"\n",
        markers: &[Marker::File("y/marker"), Marker::Types],
        folders: Folders::None,
    },
    Preset {
        name: "line-last",
        title: "a preset whose folder line ends the text",
        text: "# driftmark adds the folders that .adr-dir and .log4brains.yml name here.",
        markers: &[],
        folders: Folders::Adr,
    },
];

#[test]
fn a_list_without_markdown_markers_lists_no_folder() {
    let repo = Repo::new(&[("x/marker", "")]);
    let found = detect_in(&ODD[..1], &repo);
    assert_eq!(names(&found), [("files-only", "x/marker")]);
    assert_eq!(repo.asked(), Vec::<Vec<String>>::new());
}

#[test]
fn a_broken_preset_is_refused_with_its_name() {
    let error = ODD[1].rules(&[]).expect_err("broken");
    assert!(
        error
            .to_string()
            .starts_with("the rules file could not be read: driftmark's built-in preset \"broken\" is broken, which is a bug in driftmark: "),
        "{error}"
    );
    let repo = Repo::new(&[("y/marker", "")]);
    assert!(Setup::build_from(ODD, None, &repo).is_err());
    assert_eq!(
        ODD[2].text_with(&["d".to_owned()]),
        format!("{FOLDERS_LINE}  \"d/[0-9]*.md\",\n  \"d/[Aa][Dd][Rr]-[0-9]*.md\",\n")
    );
}

#[test]
fn adr_folders_come_from_adr_dir_and_log4brains() {
    let adr = find("adr").expect("adr");
    let folders = |files: &[(&str, &str)]| adr.folders(&Repo::new(files));
    assert_eq!(folders(&[]), Vec::<String>::new());
    assert_eq!(
        folders(&[(".adr-dir", "\n  ./docs/architecture/records/ \n")]),
        ["docs/architecture/records"]
    );
    assert_eq!(folders(&[(".adr-dir", ".\n")]), [""]);
    assert_eq!(folders(&[(".adr-dir", "\n\n")]), Vec::<String>::new());
    assert_eq!(folders(&[(".adr-dir", "my\\adr")]), ["my/adr"]);
    // A folder the preset names already adds nothing.
    assert_eq!(folders(&[(".adr-dir", "doc/adr")]), Vec::<String>::new());
    for outside in ["../adr", "/adr", "a//b", "a/./b", "a\u{7}b"] {
        assert_eq!(
            folders(&[(".adr-dir", outside)]),
            Vec::<String>::new(),
            "{outside}"
        );
    }
    assert_eq!(
        adr.folders(&Repo::with_bytes(".adr-dir", &[0xff])),
        Vec::<String>::new()
    );
    let log4brains = "project:\n  name: Demo\n  adrFolder: ./docs/adr\n  packages:\n    - name: core\n      adrFolder: ./packages/core/docs/adr\n    - plain\n    - name: web\n      adrFolder: [x]\n    - name: api\n      adrFolder: ./docs/adr\n";
    assert_eq!(
        folders(&[(".adr-dir", "decisions"), (".log4brains.yml", log4brains)]),
        ["decisions", "packages/core/docs/adr"]
    );
    for odd in [
        "project: [a]\n",
        "a: [\n",
        "project:\n  packages: none\n",
        "project:\n  adrFolder: x\n  packages: no\n",
    ] {
        let found = folders(&[(".log4brains.yml", odd)]);
        assert!(found.len() <= 1, "{odd}: {found:?}");
    }
    assert_eq!(
        folders(&[(
            ".log4brains.yml",
            "project:\n  adrFolder: x\n  packages: no\n"
        )]),
        ["x"]
    );
    assert_eq!(
        adr.folders(&Repo::with_bytes(".log4brains.yml", &[0xff])),
        Vec::<String>::new()
    );
    let openspec = find("openspec").expect("openspec");
    assert_eq!(
        openspec.folders(&Repo::new(&[(".adr-dir", "x")])),
        Vec::<String>::new()
    );
}

#[test]
fn extra_folders_are_written_into_the_adr_text() {
    let adr = find("adr").expect("adr");
    assert_eq!(adr.text_with(&[]), adr.text);
    assert_eq!(adr.text_with(&["doc/adr".to_owned()]), adr.text);
    let text = adr.text_with(&[
        "docs/arch".to_owned(),
        String::new(),
        "odd[1]\"q".to_owned(),
    ]);
    let after = &text[text.find(FOLDERS_LINE).expect("the line") + FOLDERS_LINE.len()..];
    assert!(
        after.starts_with("\n  \"docs/arch/[0-9]*.md\",\n  \"docs/arch/[Aa][Dd][Rr]-[0-9]*.md\",\n  \"[0-9]*.md\",\n  \"[Aa][Dd][Rr]-[0-9]*.md\",\n  \"odd[[]1[]]\\\"q/[0-9]*.md\","),
        "{after}"
    );
    let rules = adr
        .rules(&["docs/arch".to_owned(), "odd[1]\"q".to_owned()])
        .expect("valid");
    assert_eq!(
        type_of(&rules, "docs/arch/0001-x.md").as_deref(),
        Some("adr")
    );
    assert_eq!(
        type_of(&rules, "odd[1]\"q/0001-x.md").as_deref(),
        Some("adr")
    );
    assert_eq!(type_of(&rules, "docs/arch/README.md"), None);
    let kiro = find("kiro").expect("kiro");
    assert_eq!(kiro.text_with(&["x".to_owned()]), kiro.text);
}

#[test]
fn a_pattern_lists_the_folder_before_its_first_wildcard() {
    for (pattern, folder) in [
        ("openspec/changes/archive/**", "openspec/changes/archive"),
        ("openspec/project.md", "openspec"),
        ("docs/adr/[0-9]*.md", "docs/adr"),
        ("specs/*/spec.md", "specs"),
        ("a/b?/c/d.md", "a"),
        ("[0-9]*.md", ""),
        ("README.md", ""),
    ] {
        assert_eq!(folder_of(pattern), folder, "{pattern}");
    }
}

#[test]
fn a_repository_without_a_rules_file_uses_the_presets_it_has() {
    let empty = Setup::build(None, &Repo::new(&[])).expect("valid");
    assert_eq!(empty.rules, Rules::default());
    assert_eq!(empty.presets, []);
    let repo = Repo::new(&[
        (
            "doc/adr/0001-record-architecture-decisions.md",
            "# 1. Record decisions\n\nDate: 2026-01-05\n\n## Status\n\nAccepted\n\n## Decision\n\nWe record decisions.\n",
        ),
        ("openspec/specs/auth/spec.md", "# Auth\n"),
        (".adr-dir", "docs/records\n"),
    ]);
    let setup = Setup::build(None, &repo).expect("valid");
    let used: Vec<(&str, Option<&str>, &[String])> = setup
        .presets
        .iter()
        .map(|in_use| {
            (
                in_use.preset.name,
                in_use.found.as_deref(),
                in_use.folders.as_slice(),
            )
        })
        .collect();
    assert_eq!(
        used,
        [
            ("openspec", Some("openspec/specs/auth/spec.md"), &[][..]),
            ("adr", Some(".adr-dir"), &["docs/records".to_owned()][..]),
        ]
    );
    assert_eq!(
        type_of(&setup.rules, "openspec/specs/auth/spec.md").as_deref(),
        Some("spec")
    );
    assert_eq!(
        type_of(&setup.rules, "docs/records/0002-x.md").as_deref(),
        Some("adr")
    );
    let path = "doc/adr/0001-record-architecture-decisions.md";
    let bytes = repo.read(path).expect("the record");
    let document = Document::parse_with(bytes, setup.rules.masthead_keys()).expect("parses");
    let class = setup.rules.classify_with(path, &document, &repo);
    assert_eq!(class.doc_type, "adr");
    assert_eq!(class.status.as_deref(), Some("Accepted"));
    assert_eq!(class.lifecycle, Some(Lifecycle::Current));
    assert_eq!(class.date.as_deref(), Some("2026-01-05"));
    assert_eq!(class.decision_blocks, [4, 5]);
}

#[test]
fn a_rules_file_uses_only_the_presets_it_names_after_its_own_types() {
    let repo = Repo::new(&[
        ("openspec/config.yaml", "schema: spec-driven\n"),
        (".kiro/steering/product.md", "# P\n"),
    ]);
    let none = Rules::parse("[lint]\nexclude = [\"x/**\"]\n").expect("valid");
    let setup = Setup::build(Some(none.clone()), &repo).expect("valid");
    assert_eq!(setup.rules, none);
    assert_eq!(setup.presets, []);
    let own = Rules::parse(
        "presets = [\"kiro\", \"adr\"]\n\n[[type]]\nname = \"steering-draft\"\npaths = [\".kiro/steering/draft-*.md\"]\n",
    )
    .expect("valid");
    let setup = Setup::build(Some(own), &repo).expect("valid");
    let used: Vec<(&str, Option<&str>)> = setup
        .presets
        .iter()
        .map(|in_use| (in_use.preset.name, in_use.found.as_deref()))
        .collect();
    assert_eq!(used, [("kiro", None), ("adr", None)]);
    assert_eq!(
        type_of(&setup.rules, ".kiro/steering/draft-x.md").as_deref(),
        Some("steering-draft")
    );
    assert_eq!(
        type_of(&setup.rules, ".kiro/steering/product.md").as_deref(),
        Some("steering")
    );
    assert_eq!(
        type_of(&setup.rules, "docs/adr/0001-x.md").as_deref(),
        Some("adr")
    );
    assert_eq!(type_of(&setup.rules, "openspec/specs/a/spec.md"), None);
}

#[test]
fn spec_kit_reads_its_bold_lines_and_cc_sdd_its_spec_json() {
    let spec_kit = Setup::build(
        Some(Rules::parse("presets = [\"spec-kit\"]\n").expect("valid")),
        &Repo::new(&[]),
    )
    .expect("valid");
    let read = |rules: &Rules, path: &str, source: &str, files: &dyn RepoFiles| {
        let document = Document::parse_with(source.as_bytes().to_vec(), rules.masthead_keys())
            .expect("parses");
        rules.classify_with(path, &document, files)
    };
    let apart = read(
        &spec_kit.rules,
        "specs/001-login/spec.md",
        "# Feature Specification: Login\n\n**Feature Branch**: `001-login`\n\n**Created**: 2026-01-05\n\n**Status**: Implemented\n\n**Input**: User description: \"log in\"\n",
        &Repo::new(&[]),
    );
    assert_eq!(apart.status.as_deref(), Some("Implemented"));
    assert_eq!(apart.date.as_deref(), Some("2026-01-05"));
    assert_eq!(apart.lifecycle, Some(Lifecycle::Current));
    let together = read(
        &spec_kit.rules,
        "specs/002-export/spec.md",
        "# Feature Specification: Export\n\n**Feature Branch**: `002-export`  \n**Created**: 2026-02-01  \n**Status**: Draft  \n**Input**: User description: \"export\"\n\n## Requirements *(mandatory)*\n\n- **FR-001**: The system MUST export.\n",
        &Repo::new(&[]),
    );
    assert_eq!(together.status.as_deref(), Some("Draft"));
    assert_eq!(together.lifecycle, None);
    assert_eq!(together.decision_blocks, [2, 3]);
    let kiro = Setup::build(
        Some(Rules::parse("presets = [\"kiro\"]\n").expect("valid")),
        &Repo::new(&[]),
    )
    .expect("valid");
    let files = Repo::new(&[(
        ".kiro/specs/albums/spec.json",
        "{\"phase\": \"design-generated\", \"updated_at\": \"2026-03-01T10:00:00Z\"}",
    )]);
    let class = read(
        &kiro.rules,
        ".kiro/specs/albums/design.md",
        "# Design\n",
        &files,
    );
    assert_eq!(class.status.as_deref(), Some("design-generated"));
    assert_eq!(class.date.as_deref(), Some("2026-03-01T10:00:00Z"));
    assert_eq!(class.lifecycle, Some(Lifecycle::Current));
    let plain_kiro = read(
        &kiro.rules,
        ".kiro/specs/albums/design.md",
        "# Design\n",
        &Repo::new(&[]),
    );
    assert_eq!(plain_kiro.status, None);
    assert_eq!(plain_kiro.lifecycle, Some(Lifecycle::Current));
}

#[test]
fn the_starter_preset_is_used_only_when_named() {
    let starter = find("starter").expect("starter");
    assert!(starter.markers.is_empty());
    assert_eq!(starter.text, crate::starter::preset::RULES);
    let layout = Repo::new(&[
        ("docs/README.md", "# Docs\n"),
        ("docs/standards/writing.md", "# Writing\n"),
        ("docs/features/export.md", "# Export\n"),
        ("docs/working/move-queue/plan.md", "# Plan\n"),
        ("docs/archive/old/plan.md", "# Old\n"),
    ]);
    assert_eq!(names(&detect(&layout)), []);
    let named = Setup::build(
        Some(Rules::parse("presets = [\"starter\"]\n").expect("valid")),
        &layout,
    )
    .expect("valid");
    assert_eq!(
        type_of(&named.rules, "docs/working/move-queue/plan.md").as_deref(),
        Some("plan")
    );
    assert_eq!(
        named.rules.types(),
        Rules::parse(crate::starter::preset::RULES)
            .expect("valid")
            .types()
    );
}
