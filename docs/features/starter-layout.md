---
status: target
date: 2026-10-18
---

# Starter layout

> Short version: `driftmark init` sets up a docs layout for a team that has none. It makes five folders with a README each, a writing standard and a rules file, and it prints a block of instructions for the team's agents. `driftmark new <kind> <title>` then starts each document from a template. `init` writes nothing without `--write`, and it never changes a file that exists.

## The layout

Five folders, one per kind of document (decision 0006):

| Folder | What it holds | How it changes |
|---|---|---|
| `docs/decisions/` | Numbered decision records | Written once. A later record replaces an earlier one. |
| `docs/standards/` | Rules that stay true | Changed in place. |
| `docs/features/` | The decided target of each feature | Changed when the target changes. |
| `docs/working/<topic>/` | Active work: a plan, a status file, notes | Changed while the work goes on. |
| `docs/archive/` | Finished working folders | Never. A folder moves here whole. |

`init` writes these files:

```text
docs/decisions/README.md
docs/standards/README.md
docs/standards/writing.md
docs/features/README.md
docs/working/README.md
docs/archive/README.md
driftmark.toml
```

- **Each README** has a title, a short version, and sections on what belongs in the folder, what does not and where that goes instead, how to add a document with `driftmark new`, and what each status means. The decisions README also says how a record is replaced, the working README how work ends, and the archive README how a folder is moved there.
- **`docs/standards/writing.md`** is the writing standard. See "The writing standard" below.
- **`driftmark.toml`** is the `starter` preset. See "The rules file" below.
- **The texts** are the files in [driftmark's `crates/driftmark-core/starter/` folder](../../crates/driftmark-core/starter/), laid out as `init` writes them. The program builds them in, so `init` needs no network and no other file.
- **The living example** is driftmark's own `docs/` folder. It uses this layout, though it has no standards or archive yet.

## `driftmark init`

| Command | What it does |
|---|---|
| `driftmark init` | Prints what it would create, and the agent block. It writes nothing. |
| `driftmark init --write` | Creates the folders and files that are missing. |
| `driftmark init --agent-file AGENTS.md` | Also says whether it would add the agent block to `AGENTS.md`. |
| `driftmark init --write --agent-file AGENTS.md` | Also adds the agent block at the end of `AGENTS.md`, once. |

- **Where.** It runs inside a git repository with a working tree, from any folder in it. The layout goes at the repository root.
- **Outside a repository** it refuses, with exit code 1, and writes nothing.

### What it prints

For a made-up repository at `/home/sam/shop` that already has `docs/decisions/` and a `driftmark.toml`, `driftmark init` prints:

```text
driftmark init: the starter layout in /home/sam/shop. Nothing is written without --write.

  exists        docs/decisions/
  would create  docs/decisions/README.md
  would create  docs/standards/
  would create  docs/standards/README.md
  would create  docs/standards/writing.md
  would create  docs/features/
  would create  docs/features/README.md
  would create  docs/working/
  would create  docs/working/README.md
  would create  docs/archive/
  would create  docs/archive/README.md
  exists        driftmark.toml (it stays as it is)

Run `driftmark init --write` to create what is missing. It never changes a file that exists.

Add this block to AGENTS.md or CLAUDE.md, so agents know where each kind of document goes. `--agent-file AGENTS.md` adds it for you.
```

The agent block follows, as "The agent block" below shows it.

- There is one row per part of the layout, folders first, in the order above. The agent file, when one is named, comes last.
- The path is the canonical path of the repository root. Folders end in `/`.
- When nothing is missing, the line after the rows reads "Everything is in place, so there is nothing to create."

The word in each row says what happened:

| Word | Means |
|---|---|
| `would create` | Missing. `--write` would create it. |
| `exists` | It is there, and nothing happens to it. |
| `created` | This run created it. |
| `skipped` | With `--write`: it exists, so it was left as it is. |
| `would append`, `appended` | The agent block goes, or went, at the end of an existing agent file. |

### With `--write`

```text
driftmark init: the starter layout in /home/sam/shop.

  exists   docs/decisions/
  created  docs/decisions/README.md
  created  docs/standards/
  created  docs/standards/README.md
  created  docs/standards/writing.md
  created  docs/features/
  created  docs/features/README.md
  created  docs/working/
  created  docs/working/README.md
  created  docs/archive/
  created  docs/archive/README.md
  skipped  driftmark.toml (it exists, so it was left as it is)

Next, start a document with `driftmark new <kind> "<title>"`, and check the documents with `driftmark lint`.
```

The agent block follows here too, unless `--agent-file` added it.

- **Only what is missing.** Each missing folder is made, then each missing file, in the order of the rows.
- **Never an existing file.** A file counts as existing whatever it holds, even when it is empty or a link. It is never opened for writing. Each file is created only if nothing has that name at that moment, so a file that appears while `init` runs is safe too.
- **Folders stay as they are.** Files in them that `init` does not write are not read.
- **An existing rules file is not merged.** When `driftmark.toml` exists, its types may not cover the layout. Copy the `[[type]]` tables you want from "The rules file" below.
- **A failure stops the run.** When a folder or file cannot be made, `init` prints the rows of what it did so far, then the reason on standard error, and exits with 1.

### The agent file

`--agent-file <file>` names the file that gets the agent block, such as `AGENTS.md` or `CLAUDE.md`.

- **Which file.** The path is read from the current folder. The file must be inside the repository. A link is followed to its file, which must be inside the repository too, and the link stays a link.
- **Where the block goes.** At the end of the file, after a blank line. A last line without a line break gets one first. A file with Windows line breaks gets them in the block too.
- **A new file.** A file that does not exist yet, or is empty, starts with a title and a short version, so it passes the writing check:

  ```markdown
  # Agent instructions

  > Short version: how AI agents work in this repository.

  ```

- **Once.** A file that already holds the block's first line, `<!-- driftmark: starter layout -->`, gets nothing, even after someone edited the text below it.
- **Nothing else.** The file's text so far is never rewritten, and no other file is touched.
- **Without `--write`** the row says what would happen, and the block is printed.
- **Without `--agent-file`** the block is printed for the user to paste.

### Exit codes

| Code | When |
|---|---|
| 0 | The layout was set up, or would be. |
| 1 | It refused: outside a repository; an agent file outside the repository, a link to nothing, or not UTF-8 text; a folder or file that could not be made. |
| 2 | An unknown argument, or `--agent-file` without a file. |

## `driftmark new`

`driftmark new <kind> <title>` starts a document from its kind's template, and prints the new file's path.

```console
$ driftmark new decision "Keep one database per service"
docs/decisions/0004-keep-one-database-per-service.md
$ driftmark new plan "Faster search"
docs/working/faster-search/plan.md
$ driftmark new status "Faster search"
docs/working/faster-search/status.md
```

| Kind | Path | Title line | First status |
|---|---|---|---|
| `decision` | `docs/decisions/<number>-<name>.md` | `# <number> <title>` | `proposed` |
| `standard` | `docs/standards/<name>.md` | `# <title>` | `draft` |
| `feature` | `docs/features/<name>.md` | `# <title>` | `target` |
| `plan` | `docs/working/<name>/plan.md` | `# <title> plan` | `active` |
| `status` | `docs/working/<name>/status.md` | `# <title> status` | `active` |

- **The title** is every word after the kind, joined by spaces, with spaces at either end dropped. Quotes are only for the shell. A word that starts with `--` is refused as an unknown option. For a title that starts with `--`, put `--` after the kind.
- **The name** comes from the title. Letters and digits are kept in lower case, and each run of other characters becomes one `-`. Apostrophes are dropped, so "Don't cache" gives `dont-cache`. Letters of any script are kept, so "Straße" gives `straße`. A name is at most 60 characters, cut after the last whole word that fits.
- **A plan and a status file** with the same title share one folder, `docs/working/<name>/`.
- **The number** of a decision is one more than the highest number in `docs/decisions/`, or 1 when there is none. Only names of the form `<digits>-<anything>.md` count. It has at least four digits, such as `0004` or `12345`.
- **Numbers can collide across branches.** Two branches that each add a record can take the same number. The one that merges second renames its record.
- **The date** is today's date in the local time zone, such as `2026-03-02`. When the time zone cannot be found, it is the date in UTC.
- **The output** is the path alone on standard output, from the repository root with `/` between folders. A script can use it, as in `$(driftmark new standard "Error messages")`.
- **Never an existing file.** When the file exists, `new` writes nothing and exits with 1.
- **Folders** are made when missing, so `new` also works without `init`. Its paths are always the starter layout's, whatever `driftmark.toml` says.
- **Where.** It runs from any folder inside a git repository.

| Problem | Exit code | Message after `driftmark new: ` |
|---|---|---|
| No kind | 2 | `name a kind and a title` |
| Unknown kind | 2 | ``there is no kind `memo`; use decision, standard, feature, plan or status`` |
| A word starting with `--` | 2 | ``unknown option `--force`; put `--` before a title that starts with `--` `` |
| No title | 2 | ``the title is empty; write it after the kind, such as `driftmark new decision "Keep one database per service"` `` |
| A line break or a tab | 2 | `the title holds a line break or a tab; write it on one line, with spaces between the words` |
| No letter or digit | 2 | `the title has no letter or digit, so no file name can be made from it; use words in the title` |
| The file exists | 1 | `docs/standards/error-messages.md already exists, so nothing was written; choose another title, or edit that file` |
| Outside a repository | 1 | `run it inside a git repository (…)` |

Every message with exit code 2 is followed by the usage text.

## The templates

Every template has the shape of decision 0002: front matter with `status` and `date`, one `#` title, a `> Short version:` line, then three or four `##` sections.

- **Guidance comments.** Each section, and the short version, holds one HTML comment of one line that says what to write there. The writer, or the agent, replaces each comment with text.
- **Comments render nothing.** A document with its comments left in shows empty sections, and it still passes the writing check.

The examples below are what `new` writes on 2026-03-02, with `0004` as the next decision number.

### Decision

`driftmark new decision "Keep one database per service"` writes `docs/decisions/0004-keep-one-database-per-service.md`:

```markdown
---
status: proposed
date: 2026-03-02
---

# 0004 Keep one database per service

> Short version: <!-- The decision and its main reason, in one or two sentences. -->

## Situation

<!-- What makes a decision necessary now. Facts only, with numbers where you have them. -->

## Options

<!-- Two to four options. For each: what it is, what it costs, and why it was or was not chosen. -->

## Decision

<!-- What was decided, as numbered points of one sentence each. -->

## What this costs

<!-- What gets harder, slower or more expensive because of this decision. -->
```

### Standard

`driftmark new standard "Error messages"` writes `docs/standards/error-messages.md`:

```markdown
---
status: draft
date: 2026-03-02
---

# Error messages

> Short version: <!-- What this standard asks for, in one or two sentences. -->

## Rules

<!-- Numbered rules. Each rule is one sentence that says what to do. -->

## Why

<!-- The reasons behind the rules, so a reader can judge a case the rules do not cover. -->

## Examples

<!-- One made-up example that follows the rules, and one that breaks them. -->
```

### Feature

`driftmark new feature "Export to PDF"` writes `docs/features/export-to-pdf.md`:

```markdown
---
status: target
date: 2026-03-02
---

# Export to PDF

> Short version: <!-- What the feature does for the person who uses it, in one or two sentences. -->

## What it does

<!-- What a user sees, step by step, with made-up examples. -->

## Contract

<!-- What other work builds on: inputs, outputs, errors and limits, stated exactly. -->

## Not covered

<!-- What the feature leaves out on purpose, and where that work waits. -->
```

### Plan

`driftmark new plan "Faster search"` writes `docs/working/faster-search/plan.md`:

```markdown
---
status: active
date: 2026-03-02
---

# Faster search plan

> Short version: <!-- The goal and the order of the work, in one or two sentences. -->

## Goal

<!-- What is true when the work is done, and how anyone can check it. -->

## Steps

<!-- Numbered steps. Each says what it delivers and how to tell that it is done. -->

## Open questions

<!-- What is not decided yet, and who decides it. -->
```

### Status file

`driftmark new status "Faster search"` writes `docs/working/faster-search/status.md`:

```markdown
---
status: active
date: 2026-03-02
---

# Faster search status

> Short version: <!-- Where the work stands today, in one sentence. -->

## Now

<!-- What is in progress, and what is blocked and by what. -->

## Next

<!-- The next steps, in order. -->

## Log

<!-- One line per working day, newest first: the date, then what changed. -->
```

## The rules file

`init` writes the `starter` preset as `driftmark.toml`. It is an ordinary rules file in the format of `docs/features/engine/rules-file.md`:

```toml
# driftmark's rules for the starter layout that `driftmark init` sets up.
#
# Each [[type]] says which files hold one kind of document, where its status
# is written, and what each status means: current, superseded or archived.
# The first type whose paths match a file wins. The format is described in
# driftmark's documentation, in docs/features/engine/rules-file.md.

# A folder's README says what belongs in the folder. It needs no status.
[[type]]
name = "readme"
paths = ["docs/README.md", "docs/*/README.md"]
lifecycle = "current"

# Decision records: written once, numbered, and replaced by a later record
# instead of being edited.
[[type]]
name = "decision"
paths = ["docs/decisions/*.md"]
status = { front-matter = "status" }
other-statuses = ["proposed"]
current = ["accepted"]
superseded = ["superseded"]
archived = ["rejected"]
date = { front-matter = "date" }
superseded-by = { front-matter = "superseded-by" }
decision-headings = ["Decision"]
required = ["status", "date"]

# Standards: rules that stay true over time and change in place.
[[type]]
name = "standard"
paths = ["docs/standards/**/*.md"]
status = { front-matter = "status" }
other-statuses = ["draft"]
current = ["active"]
archived = ["retired"]
date = { front-matter = "date" }
required = ["status", "date"]

# Features: the decided target of each feature.
[[type]]
name = "feature"
paths = ["docs/features/**/*.md"]
status = { front-matter = "status" }
current = ["target", "shipped"]
archived = ["dropped"]
date = { front-matter = "date" }
required = ["status", "date"]

# Active work: one folder per topic, with a plan and a status file.
[[type]]
name = "plan"
paths = ["docs/working/*/plan.md"]
status = { front-matter = "status" }
other-statuses = ["paused"]
current = ["active"]
archived = ["done"]
date = { front-matter = "date" }
required = ["status", "date"]

[[type]]
name = "status"
paths = ["docs/working/*/status.md"]
status = { front-matter = "status" }
other-statuses = ["paused"]
current = ["active"]
archived = ["done"]
date = { front-matter = "date" }
required = ["status", "date"]

# Anything else a working folder holds: notes, findings, drafts.
[[type]]
name = "working"
paths = ["docs/working/**"]
status = { front-matter = "status" }
other-statuses = ["paused"]
current = ["active"]
archived = ["done"]
date = { front-matter = "date" }
lifecycle = "current"

# Finished working folders, kept as a record. Their status is not read.
[[type]]
name = "archive"
paths = ["docs/archive/**"]
lifecycle = "archived"

# The writing check, `driftmark lint`. These are its defaults, written out so
# they can be changed here; docs/standards/writing.md names the same values.
[lint]
allowed-html = ["details", "summary", "br", "sub", "sup", "kbd"]
max-table-columns = 8
max-code-line-length = 120
max-file-size = 100000
max-heading-length = 100
```

What it gives:

| Type | Files | Current | Superseded | Archived | Other | Required |
|---|---|---|---|---|---|---|
| `readme` | a folder's `README.md` | always | | | | nothing |
| `decision` | `docs/decisions/*.md` | `accepted` | `superseded` | `rejected` | `proposed` | `status`, `date` |
| `standard` | `docs/standards/**/*.md` | `active` | | `retired` | `draft` | `status`, `date` |
| `feature` | `docs/features/**/*.md` | `target`, `shipped` | | `dropped` | | `status`, `date` |
| `plan`, `status` | `docs/working/*/plan.md`, `status.md` | `active` | | `done` | `paused` | `status`, `date` |
| `working` | the rest of `docs/working/` | `active`, or no status | | `done` | `paused` | nothing |
| `archive` | `docs/archive/**` | | | always | | nothing |

- **READMEs come first.** Their type matches before the folder's own, so a README needs no front matter.
- **Plans and status files come before `working`,** so they get their own type, and must have a status and a date.
- **A decision record's `## Decision` section** holds its decisions. The change view ranks a change there above wording changes. No other type has decision headings.
- **A superseded record names its successor** in `superseded-by`, such as `superseded-by: 0007-use-two-queues.md`.
- **An archived document's status is not read.** A plan moved to the archive keeps `status: done`, and it is archived because of where it is.
- **The `[lint]` table** holds the writing check's defaults, written out so a team sees them and can change them. A test keeps them equal to the defaults.

### Joining the presets

- The preset's name is `starter`. Its text is `driftmark_core::starter::preset::RULES`, which `init` writes as it is.
- It is a rules file in the usual format, so it can join the built-in presets unchanged.

## The writing standard

`init` writes `docs/standards/writing.md`, which a team can adopt as it is. It holds the conventions of decision 0002 and the plain-words rule of driftmark's `AGENTS.md`, with made-up examples only. Its sections:

| Section | What it says |
|---|---|
| Why these rules | Who reads the documents, why Markdown costs agents few tokens, and that the lint makes the rules hold. |
| The shape of a document | Front matter with `status` and `date`, one `#` title, a `>` short version, `##` sections with no skipped level. |
| Sentences | The answer first, one idea per sentence, short sentences, everyday words. |
| Diagrams | Mermaid by default, in lower case, with one drawn example; D2 allowed for architecture pictures. |
| Code blocks | Fenced, closed, with a language; `text` for plain text and program output; lines of at most 120 characters. |
| Raw HTML | Only `<details>`, `<summary>`, `<br>`, `<sub>`, `<sup>` and `<kbd>`; comments allowed; alerts for notes and warnings. |
| Links | Relative Markdown links, anchors for sections, no wiki links, full addresses only outside the repository. |
| Size | At most 8 table columns, 100-character headings and 100,000-byte files. |
| Checking | What `driftmark lint` checks, the Claude Code hook, and where the limits live. |

- Its front matter says `status: active`, with the date driftmark last changed it. A team that changes a rule sets the date to the day of its change.
- Its limits and HTML list are the ones the preset's `[lint]` table writes out.
- It passes the writing check under the preset, its Mermaid diagram included.

## The agent block

`init` prints this block, and `--agent-file` adds it to a file:

```markdown
<!-- driftmark: starter layout -->

## Documents

This repository keeps its documents in `docs/`, one folder per kind:

- `docs/decisions/`: numbered decision records. A record is written once. A later record
  replaces it instead of an edit.
- `docs/standards/`: rules that stay true. Change them in place.
- `docs/features/`: the decided target of each feature.
- `docs/working/<topic>/`: active work, with a `plan.md` and a `status.md`.
- `docs/archive/`: finished working folders, moved here whole.

Start every new document with `driftmark new <kind> "<title>"`. The kind is `decision`,
`standard`, `feature`, `plan` or `status`. The command prints the new file's path. Replace each
`<!-- … -->` comment in the file with your text.

Write as `docs/standards/writing.md` says. After you edit a document, run `driftmark lint` and
fix what it reports.

<!-- driftmark: end of starter layout -->
```

- **The comments at both ends** render nothing. The first one marks a file that holds the block.
- **A `##` section,** because the block goes into a file that has its own `#` title.
- **Lines of at most 100 characters,** so the block also fits a code block like this one without a lint warning.
- **No links.** The writing standard is named as code, because the agent file may sit in any folder, where a relative link would break.

## Checked by tests

- **The layout and the templates.** Every file `init` writes, and every kind's template as `new` fills it, passes the writing check under the preset with no finding.
- **An empty repository.** `init --write --agent-file AGENTS.md`, then `new` for each kind, then `driftmark lint`, finds no problem.
- **This document.** Its rules file, its agent block, its five templates and its `init` output are compared with what the program writes.

## Not covered

- **A team's own templates.** `new` uses the built-in templates only.
- **Paths from the rules file.** `new` always uses the starter layout's paths.
- **Merging into an existing rules file.** `init` leaves an existing `driftmark.toml` as it is.
- **Guidance comments left in a document.** The writing check does not report them.
- **Detecting the layout** in a repository without a rules file, as the other presets are detected.
