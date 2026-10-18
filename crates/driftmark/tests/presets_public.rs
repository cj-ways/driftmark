//! Each built-in preset on real public repositories, fetched from GitHub at
//! pinned commits. driftmark finds the presets with no rules file, reads every
//! Markdown file at the commit, and its counts of types, lifecycles, statuses
//! and dates must equal counts made by hand with `git ls-tree` and `grep`.
//!
//! The tests use the network, so normal runs leave them out. The
//! `presets-public` job in `.github/workflows/ci.yml` runs them:
//!
//! ```text
//! cargo test -p driftmark --test presets_public -- --ignored
//! ```

mod support;

use std::collections::BTreeMap;
use std::fmt::Write as _;

use driftmark::git::Git;
use driftmark::presets::{files_at, setup_at};
use driftmark_core::document::Document;
use driftmark_core::git::{GitRead, ObjectId, Roots, Snapshot};
use support::TestRepo;

/// What driftmark makes of a repository at `commit` of `url`, written out:
/// the presets found, then counts per type, lifecycle, status and date.
fn counts(url: &str, commit: &str) -> String {
    let repo = TestRepo::new("presets-public");
    repo.git(&["fetch", "-q", "--depth", "1", url, commit]);
    let git = Git::open(&repo.dir).expect("the repo opens");
    let side = Snapshot::Commit(ObjectId::from_hex(commit).expect("an id"));
    let setup = setup_at(&git, &side).expect("the rules load");
    let files = files_at(&git, &side);
    let mut types: BTreeMap<String, usize> = BTreeMap::new();
    let mut lifecycles: BTreeMap<String, usize> = BTreeMap::new();
    let mut statuses: BTreeMap<String, usize> = BTreeMap::new();
    let mut dated = 0;
    let mut unread = 0;
    for file in git
        .markdown_files(&side, &Roots::all())
        .expect("the files list")
    {
        let bytes = git.read_blob(&file.id).expect("the file reads");
        let Ok(document) = Document::parse_with(bytes, setup.rules.masthead_keys()) else {
            unread += 1;
            continue;
        };
        let class = setup
            .rules
            .classify_with(&file.path.to_lossy(), &document, &files);
        if let Some(lifecycle) = class.lifecycle {
            *lifecycles
                .entry(format!("{lifecycle:?}").to_lowercase())
                .or_default() += 1;
        }
        if let Some(status) = &class.status {
            *statuses
                .entry(format!("{} {status}", class.doc_type))
                .or_default() += 1;
        }
        dated += usize::from(class.date.is_some());
        *types.entry(class.doc_type).or_default() += 1;
    }
    let listed = |counts: &BTreeMap<String, usize>| {
        if counts.is_empty() {
            "-".to_owned()
        } else {
            let items: Vec<String> = counts
                .iter()
                .map(|(name, count)| format!("{name} {count}"))
                .collect();
            items.join(", ")
        }
    };
    let mut out = String::new();
    for in_use in &setup.presets {
        let _ = write!(
            out,
            "presets: {} ({})",
            in_use.preset.name,
            in_use.found.as_deref().unwrap_or("named")
        );
        if !in_use.folders.is_empty() {
            let _ = write!(out, ", also reading {}", in_use.folders.join(", "));
        }
        out.push('\n');
    }
    let _ = writeln!(out, "types: {}", listed(&types));
    let _ = writeln!(out, "lifecycles: {}", listed(&lifecycles));
    let _ = writeln!(out, "statuses: {}", listed(&statuses));
    let _ = writeln!(out, "dated: {dated}");
    let _ = writeln!(out, "unread: {unread}");
    out
}

#[test]
#[ignore = "fetches a public repository from GitHub; the presets-public CI job runs it"]
fn openspec_on_its_own_repository() {
    assert_eq!(
        counts(
            "https://github.com/Fission-AI/OpenSpec",
            "c794978f4f8845ce59693373c9dd882ee776588c"
        ),
        "\
presets: openspec (openspec/config.yaml)
types: archived-change 336, delta-spec 63, design 17, document 219, proposal 30, spec 36, tasks 28
lifecycles: archived 336, current 36
statuses: -
dated: 212
unread: 0
"
    );
}

#[test]
#[ignore = "fetches a public repository from GitHub; the presets-public CI job runs it"]
fn spec_kit_with_one_header_paragraph() {
    assert_eq!(
        counts(
            "https://github.com/Antigenic-OSS/pAInt",
            "2dda05b48cc31511d268967d6a22292db24a8e30"
        ),
        "\
presets: spec-kit (.specify/memory/constitution.md)
types: checklist 8, constitution 1, contract 7, data-model 8, document 35, feature-spec 8, plan 8, quickstart 8, research 8, tasks 8
lifecycles: current 1
statuses: feature-spec Draft 8
dated: 16
unread: 0
"
    );
}

#[test]
#[ignore = "fetches a public repository from GitHub; the presets-public CI job runs it"]
fn spec_kit_with_a_paragraph_per_header_line() {
    assert_eq!(
        counts(
            "https://github.com/cleder/gpc-init",
            "9ec0c197db357ae0740050682c45f85407282de1"
        ),
        "\
presets: spec-kit (.specify/memory/constitution.md)
types: checklist 1, constitution 1, contract 1, data-model 1, document 35, feature-spec 1, plan 1, quickstart 1, research 1, tasks 1
lifecycles: current 1
statuses: feature-spec Draft 1
dated: 2
unread: 0
"
    );
}

#[test]
#[ignore = "fetches a public repository from GitHub; the presets-public CI job runs it"]
fn adr_tools_on_its_own_repository() {
    assert_eq!(
        counts(
            "https://github.com/npryce/adr-tools",
            "b3279baf9be2207d1a4f4bbd608fd0b591c72aee"
        ),
        "\
presets: adr (doc/adr/0001-record-architecture-decisions.md)
types: adr 9, document 4
lifecycles: current 9
statuses: adr Accepted 9
dated: 9
unread: 0
"
    );
}

#[test]
#[ignore = "fetches a public repository from GitHub; the presets-public CI job runs it"]
fn log4brains_on_its_own_repository() {
    assert_eq!(
        counts(
            "https://github.com/thomvaill/log4brains",
            "17e32021a8c5130386f17e921d4efa6da7709a66"
        ),
        "\
presets: adr (.log4brains.yml), also reading packages/core/docs/adr, packages/web/docs/adr
types: adr 19, document 51
lifecycles: current 17, superseded 2
statuses: adr accepted 17, adr superseded by 20201016-use-the-adr-slug-as-its-unique-id 1, adr superseded by 20241217-switch-back-to-github-flow 1
dated: 19
unread: 0
"
    );
}

#[test]
#[ignore = "fetches a public repository from GitHub; the presets-public CI job runs it"]
fn cc_sdd_on_its_own_repository() {
    assert_eq!(
        counts(
            "https://github.com/gotalab/cc-sdd",
            "e2a0c671aef37a482404905b409ab5c27092b25b"
        ),
        "\
presets: kiro (.kiro/specs/customer-support-rag-backend-en/design.md)
types: design 4, document 393, requirements 4, research 2, tasks 4
lifecycles: current 14
statuses: design tasks-generated 4, requirements tasks-generated 4, research tasks-generated 2, tasks tasks-generated 4
dated: 14
unread: 0
"
    );
}
