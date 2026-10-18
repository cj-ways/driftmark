---
status: target
date: 2026-10-18
---

# Presets

> Short version: a preset is a ready-made rules file for a popular folder layout, built into driftmark. driftmark has five: OpenSpec, GitHub Spec Kit, ADR folders, Kiro with cc-sdd, and driftmark's own starter layout. A repository with no `driftmark.toml` uses every preset whose marker files it has; the starter preset has none, so it is used only when named. A repository with a `driftmark.toml` uses the presets it names in `presets = [...]`, after its own types. `driftmark presets` lists them and says which ones apply, and `driftmark presets show <name>` prints one to copy and change. driftmark only reads these folders; it never moves, renames or rewrites a document (decision 0006).

## What a preset gives

A preset is a rules file in the format of `docs/features/engine/rules-file.md`. It says:

- which paths hold which document type;
- where each type writes its status and its date;
- what each status means: current, superseded or archived;
- which headings open decision sections;
- for some layouts, extra masthead keys to read (decision 0009).

With a preset, the change list ranks a change under a decision heading as "decision" (`docs/features/engine/change-list.md`). The reading pages, the MCP connector and the coming status page show each document's type, status, lifecycle and date.

A preset gives only its `[[type]]` entries and its `[masthead]` table. The writing check's `[lint]` settings and the `[compare]` table come from the repository's own `driftmark.toml`, never from a preset. The starter preset's text also holds a `[lint]` table, because `driftmark init` writes that text as the repository's `driftmark.toml`. The table writes out the defaults, and a test keeps them equal, so naming the preset gives the same settings.

## Choosing presets

### With no rules file

A repository with no `driftmark.toml` uses every preset whose layout it has. driftmark looks for each preset's markers, in this order, and the first one found shows the layout:

| Preset | Markers |
|---|---|
| `openspec` | the file `openspec/config.yaml`; the file `openspec/project.md`; a Markdown file one of its types matches |
| `spec-kit` | the file `.specify/memory/constitution.md`; the file `memory/constitution.md`; a Markdown file at `specs/NNN-name/spec.md`, with three digits |
| `adr` | the file `.adr-dir`; the file `.log4brains.yml`; a Markdown file one of its types matches |
| `kiro` | a Markdown file one of its types matches |
| `starter` | none: it is used only when a rules file names it |

- More than one preset may apply. Their types are added in the order of this table.
- A folder named `specs/` alone does not count as Spec Kit, because many projects have one. The feature folders need Spec Kit's three-digit numbers, or the constitution must be there.
- Markers are looked for at the side a page or a comparison reads: at the commit, or in the working tree on disk.

### With a rules file

A `driftmark.toml` turns finding off. It names the presets it wants:

```toml
presets = ["openspec", "adr"]

# A type of this repository's own. It comes before the presets' types, so it
# wins for the files it matches.
[[type]]
name = "runbook"
paths = ["ops/runbooks/*.md"]
lifecycle = "current"

[lint]
exclude = ["vendor/**"]
```

- The file's own types come first, then each named preset's types, in the order named. The first type whose paths match a file wins, so an own type can take over files a preset would type.
- A preset named twice counts once.
- A name driftmark does not know is an error, with its line: ``driftmark has no preset "backlog"; the presets are "openspec", "spec-kit", "adr", "kiro", "starter"``.
- Masthead keys from the file and from every named preset are all known.
- A file with no `presets` key uses its own types only, even when a layout's markers are there. `driftmark presets` then says which presets it found but does not use.

### Copying a preset

`driftmark presets show <name>` prints a preset's rules file. Write it into `driftmark.toml` to change it:

```console
$ driftmark presets show adr > driftmark.toml
```

The copy works without `presets = [...]`, because it holds the types themselves. Inside a repository, the ADR preset's printed text already holds the extra folders that `.adr-dir` and `.log4brains.yml` name there.

## `driftmark presets`

```text
usage: driftmark presets [--repo <folder>]
       driftmark presets show <name> [--repo <folder>]
```

The first form lists the presets and says, for each, whether the repository uses it. It reads the working tree, or `HEAD` in a bare repository. In a made-up repository with adr-tools records, a `.adr-dir` and no rules file:

```text
openspec   OpenSpec: living specs, open changes and the archive
           not found here
spec-kit   GitHub Spec Kit: one folder per feature under specs/
           not found here
adr        ADR folders: adr-tools, MADR, log4brains and Nygard-style records
           in use: found .adr-dir
           also reads the records in architecture/records, named by .adr-dir or .log4brains.yml
kiro       Kiro and cc-sdd: specs and steering under .kiro/
           not found here
starter    driftmark's starter layout, as driftmark init sets it up
           used only when driftmark.toml names it

This repository has no driftmark.toml, so driftmark uses every preset it finds.
To keep this choice, or to add types of your own, write this line in driftmark.toml:

  presets = ["adr"]

To see a preset's rules, ready to copy, run `driftmark presets show <name>`.
```

Each preset's second line is one of:

- `in use: found <marker>`: no rules file, and this marker showed the layout;
- `in use: driftmark.toml names it`;
- `found <marker>, but not in use: driftmark.toml does not name it`;
- `not found here`;
- `used only when driftmark.toml names it`, for a preset with no markers, such as `starter`.

The command exits with 0 when it ran, and with 2 for an unknown preset, a broken rules file, or a folder outside any git repository. `show` works outside a repository too, and then prints the preset as it is built in.

## OpenSpec

OpenSpec keeps living specs in `openspec/specs/`, each open change in a folder under `openspec/changes/`, and finished changes in `openspec/changes/archive/`.

| Type | Paths | Lifecycle | Date | Decision headings |
|---|---|---|---|---|
| `archived-change` | `openspec/changes/archive/**` | archived | `created` in `.openspec.yaml` | none |
| `delta-spec` | `openspec/changes/*/specs/**/*.md` | none | `created` in `.openspec.yaml` | `ADDED Requirements`, `MODIFIED Requirements`, `REMOVED Requirements`, `RENAMED Requirements` |
| `proposal` | `openspec/changes/*/proposal.md` | none | `created` in `.openspec.yaml` | `What Changes`, `Capabilities` |
| `design` | `openspec/changes/*/design.md` | none | `created` in `.openspec.yaml` | `Decisions` |
| `tasks` | `openspec/changes/*/tasks.md` | none | `created` in `.openspec.yaml` | none |
| `spec` | `openspec/specs/**/*.md` | current | none | `Requirements` |
| `project` | `openspec/project.md` | current | none | none |

- **Status.** OpenSpec writes no status. Where a document sits says what it is: an open change is neither current nor archived yet, so it has no lifecycle.
- **Date.** Each change folder's `.openspec.yaml` holds the day the change was made. driftmark reads it through a file location (`docs/features/engine/rules-file.md`).
- **The archive comes first**, because the first matching type wins and `openspec/changes/*/…` would also match an archived change.
- Other files under `openspec/`, such as `openspec/changes/IMPLEMENTATION_ORDER.md`, stay `document`.

A made-up delta spec, `openspec/changes/add-tags/specs/notes/spec.md`, beside an `.openspec.yaml` that says `created: 2026-03-02`:

```markdown
## ADDED Requirements

### Requirement: Tag a note

The app SHALL let the person add tags to a note.
```

It is a `delta-spec`, dated `2026-03-02`, with no lifecycle. Changing "add tags" to "add one or more tags" ranks as "decision", because the sentence sits under `ADDED Requirements`.

## GitHub Spec Kit

Spec Kit keeps one folder per feature, `specs/NNN-name/`, and the project's principles in `.specify/memory/constitution.md` (`memory/constitution.md` in early versions).

| Type | Paths | Status | Lifecycle | Date | Decision headings |
|---|---|---|---|---|---|
| `constitution` | `.specify/memory/constitution.md`, `memory/constitution.md` | none | current | none | none |
| `feature-spec` | `specs/*/spec.md` | masthead `Status` | by status | masthead `Created` | `Requirements`, `Requirements (mandatory)`, `Clarifications` |
| `plan` | `specs/*/plan.md` | none | none | masthead `Date` | none |
| `tasks`, `research`, `data-model`, `quickstart` | `specs/*/<name>.md` | none | none | none | none |
| `contract` | `specs/*/contracts/**/*.md` | none | none | none | none |
| `checklist` | `specs/*/checklists/**/*.md` | none | none | none | none |

A spec's status values, from Spec Kit's own `Draft` and the values its users write most:

| Lifecycle | Values |
|---|---|
| current | `implemented`, `complete`, `completed`, `done`, `approved`, `accepted`, `shipped` |
| superseded | `superseded`, `subsumed`, `migrated` |
| archived | `deprecated`, `abandoned`, `cancelled`, `canceled`, `rejected`, `withdrawn`, `obsolete` |
| none | `draft`, `in progress`, `in review`, `review`, `planned`, `proposed`, `ready` |

- **Status.** Spec Kit writes bold lines under the title, such as `**Status**: Draft`. driftmark reads them as a masthead, whether they share one paragraph or each line is its own paragraph.
- **Masthead keys.** The preset adds `Feature Branch`, `Created` and `Input`. Those lines come before `Status` in a spec, so without them driftmark would stop reading before the status.
- Other values, such as `Landed`, give no lifecycle, and the writing check warns about them (`status-value`).

A made-up spec, `specs/003-slideshow/spec.md`:

```markdown
# Feature Specification: Slideshow

**Feature Branch**: `003-slideshow`
**Created**: 2026-01-20
**Status**: Superseded by [004-player](../004-player/spec.md)
```

It is a `feature-spec` with the status `Superseded by 004-player`, superseded, dated `2026-01-20`.

## ADR folders

One preset covers architecture decision records written with adr-tools, MADR, log4brains, or by hand in Michael Nygard's style. All of them keep one record per file in a folder.

| Type | Paths | Status | Date | Superseded by | Decision headings |
|---|---|---|---|---|---|
| `adr` | the records in the ADR folders | front matter `status`, then masthead `Status`, then the first line under `## Status` | front matter `date`, then masthead `Date` | front matter `superseded-by`, then masthead `Superseded by` | `Decision`, `Decision Outcome` |

- **Records.** A record is a file whose name starts with a digit, or with `ADR-` or `adr-` and a digit: `0001-use-markdown.md`, `20240926-use-git-flow.md`, `ADR-012-cache.md`. A folder's `README.md`, `index.md` and `template.md` are not records.
- **Folders.** `docs/adr`, `doc/adr`, `adr`, `docs/decisions`, `docs/adrs`, `docs/architecture/decisions` and `doc/architecture/decisions`. These are the defaults of adr-tools (`doc/adr`), log4brains (`docs/adr`) and MADR (`docs/decisions`), and the folders most often seen on GitHub.
- **Folders named in marker files.** adr-tools' `.adr-dir` names the folder its records live in, and log4brains' `.log4brains.yml` names one `adrFolder` for the project and one per package. driftmark reads the records in those folders too, so a project that moved its records keeps working. A folder outside the repository, or one the preset names already, is left out.
- **Where each tool writes the status.** MADR 3 and later: `status:` in front matter. MADR 2 and log4brains: a `- Status: accepted` list item under the title. adr-tools and Nygard: the first line under `## Status`, or a `Status: Accepted` paragraph under the title.
- **Lifecycle by status.** Current: `accepted`, `approved`. Superseded: `superseded`, `superceded` (adr-tools' old spelling), `replaced`. Archived: `deprecated`, `rejected`, `withdrawn`, `obsolete`. No lifecycle: `proposed`, `draft`. A status may go on after its word, so `Superceded by [3. Store files on disk](0003-store-files-on-disk.md)` is superseded.
- **A record without a status has no lifecycle.** MADR makes the status optional. driftmark cannot tell whether such a record is in force, so it does not guess.
- Records in other folders, such as a monorepo's `services/api/doc/adr/`, need `.adr-dir`, `.log4brains.yml`, or a copied preset with that folder added.

A made-up adr-tools record, `doc/adr/0002-store-files-in-sqlite.md`:

```markdown
# 2. Store files in SQLite

Date: 2026-01-09

## Status

Superceded by [3. Store files on disk](0003-store-files-on-disk.md)

## Decision

We will store each file in an SQLite table.
```

It is an `adr`, superseded, dated `2026-01-09`. A change to its `## Decision` section ranks as "decision".

## Kiro and cc-sdd

Kiro and cc-sdd share one layout: a folder per feature or bug under `.kiro/specs/`, and steering files under `.kiro/steering/`.

| Type | Paths | Status | Lifecycle | Date | Decision headings |
|---|---|---|---|---|---|
| `requirements` | `.kiro/specs/*/requirements.md` | `phase` in `spec.json` | current | `updated_at` in `spec.json` | `Requirements` |
| `bugfix` | `.kiro/specs/*/bugfix.md` | `phase` in `spec.json` | current | `updated_at` in `spec.json` | `Bug Analysis` |
| `design` | `.kiro/specs/*/design.md` | `phase` in `spec.json` | current | `updated_at` in `spec.json` | none |
| `tasks` | `.kiro/specs/*/tasks.md` | `phase` in `spec.json` | current | `updated_at` in `spec.json` | none |
| `research` | `.kiro/specs/*/research.md` | `phase` in `spec.json` | current | `updated_at` in `spec.json` | `Design Decisions` |
| `brief` | `.kiro/specs/*/brief.md` | `phase` in `spec.json` | current | `updated_at` in `spec.json` | none |
| `steering` | `.kiro/steering/**/*.md` | none | current | none | none |

- **Kiro writes no status.** Its specs are living documents, changed in place, so they count as current. Kiro's `.config.kiro` file in each spec folder holds the spec's id, its workflow and its kind (`feature` or `bugfix`), not a status.
- **cc-sdd keeps a phase in `spec.json`** beside the spec's files, such as `requirements-generated`, `design-generated` or `tasks-generated`. driftmark shows that phase as the status, and `updated_at` as the date. The phase does not change the lifecycle: every value is allowed.
- cc-sdd's templates under `.kiro/settings/` stay `document`.

A made-up cc-sdd spec folder `.kiro/specs/shopping-list/` with this `spec.json`:

```json
{
  "feature_name": "shopping-list",
  "updated_at": "2026-02-03T16:30:00Z",
  "phase": "design-generated",
  "approvals": { "requirements": { "generated": true, "approved": true } }
}
```

Its `requirements.md` is a `requirements` document with the status `design-generated`, current, dated `2026-02-03T16:30:00Z`. A copied preset can read the approval instead, with `status = { file = "spec.json", key = "approvals.requirements.approved" }`; the status is then `true` or `false`.

## Starter layout

The `starter` preset is the rules file of the layout `driftmark init` sets up: decisions, standards, features, working folders and an archive. `docs/features/starter-layout.md` describes the layout, its templates and its rules file in full. `init` writes the preset's text as the repository's `driftmark.toml`, so a repository made by `init` names no preset; it holds the types themselves. A `driftmark.toml` with `presets = ["starter"]` gives the same types instead.

| Type | Paths | Status | Lifecycle by status | Date | Decision headings |
|---|---|---|---|---|---|
| `readme` | `docs/README.md`, `docs/*/README.md` | none | current | none | none |
| `decision` | `docs/decisions/*.md` | front matter `status` | current: `accepted`; superseded: `superseded`; archived: `rejected`; none: `proposed` | front matter `date` | `Decision` |
| `standard` | `docs/standards/**/*.md` | front matter `status` | current: `active`; archived: `retired`; none: `draft` | front matter `date` | none |
| `feature` | `docs/features/**/*.md` | front matter `status` | current: `target`, `shipped`; archived: `dropped` | front matter `date` | none |
| `plan`, `status` | `docs/working/*/plan.md`, `docs/working/*/status.md` | front matter `status` | current: `active`; archived: `done`; none: `paused` | front matter `date` | none |
| `working` | `docs/working/**` | front matter `status` | as `plan`; current when no status says otherwise | front matter `date` | none |
| `archive` | `docs/archive/**` | not read | archived | none | none |

- **No markers.** The layout's folders are common names, and a numbered `docs/decisions/` is also MADR's default folder. A repository with no rules file is therefore never taken for the starter layout. Its numbered decision records are found by the `adr` preset instead, which reads their front matter status too.
- A decision, standard, feature, plan or status file must have `status` and `date` in its front matter (`required`).

A made-up record, `docs/decisions/0002-keep-one-database.md`:

```markdown
---
status: superseded
date: 2026-02-10
superseded-by: 0005-split-the-database.md
---

# Keep one database
```

It is a `decision`, superseded, dated `2026-02-10`, superseded by `0005-split-the-database.md`.

## Where it is tested

- `fixtures/presets/<name>/` holds a made-up tree for each preset, and its `expected.txt` lists every Markdown file's type, status, lifecycle, date and decision sections. The core test reads the trees from disk. The program test commits each tree to a git repository and reads it at the commit and in the working tree.
- The program test also edits a sentence under a decision heading of each layout and checks that `driftmark diff` ranks it as "decision".
- For the starter preset, the program test runs `driftmark init --write` and `driftmark new` for each kind in a git repository. Every Markdown file must read the same with the rules file `init` wrote and with `presets = ["starter"]`: the same types, masthead keys, lint settings, classifications and `driftmark lint` output.
- `crates/driftmark/tests/presets_public.rs` fetches public repositories at pinned commits, and compares driftmark's counts of types, lifecycles, statuses and dates with counts made by hand. The `presets-public` CI job runs it; it is the only job that uses the network.

| Preset | Repository | Commit |
|---|---|---|
| `openspec` | [Fission-AI/OpenSpec](https://github.com/Fission-AI/OpenSpec) | `c794978f4f8845ce59693373c9dd882ee776588c` |
| `spec-kit` | [Antigenic-OSS/pAInt](https://github.com/Antigenic-OSS/pAInt), header lines in one paragraph | `2dda05b48cc31511d268967d6a22292db24a8e30` |
| `spec-kit` | [cleder/gpc-init](https://github.com/cleder/gpc-init), one paragraph per header line | `9ec0c197db357ae0740050682c45f85407282de1` |
| `adr` | [npryce/adr-tools](https://github.com/npryce/adr-tools) | `b3279baf9be2207d1a4f4bbd608fd0b591c72aee` |
| `adr` | [thomvaill/log4brains](https://github.com/thomvaill/log4brains), with package folders in `.log4brains.yml` | `17e32021a8c5130386f17e921d4efa6da7709a66` |
| `kiro` | [gotalab/cc-sdd](https://github.com/gotalab/cc-sdd) | `e2a0c671aef37a482404905b409ab5c27092b25b` |

## Not covered

- **A status kept outside the Markdown does not rank changes.** The change list compares Markdown files only. A commit that changes only cc-sdd's `spec.json` lists no change, and a phase read from it takes no part in the "status changed" signal. Reading pages, the MCP connector and the status page show it.
- **Markers are read when the rules are read.** `driftmark serve` notices a new `driftmark.toml` in the working tree at once. A marker file added later, such as a new `.adr-dir`, counts from the next page that reads the rules.
- **Records in nested ADR folders** that no marker file names, such as a monorepo's `services/api/doc/adr/`.
- **A merged view of an OpenSpec spec with its open changes** (decision 0005).
- **Other layouts** from the research, such as BMAD, Agent OS, Conductor and Backlog.md. A team writes its own rules file for them.
- **The company layout** of the maintainer lives in that company's repository, with its own approval (decision 0006).
