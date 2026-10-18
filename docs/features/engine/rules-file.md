---
status: target
date: 2026-10-18
---

# Rules file

> Short version: `driftmark.toml` tells driftmark which documents are which type. For each type it says where the status, the date and "superseded by" are written, what each status value means, and which headings mark decisions. A value can also come from a JSON or YAML file beside the document, such as cc-sdd's `spec.json`. The first type whose paths match a file wins. A file that no type matches is a `document` with no status. A preset is a rules file in this same format, built into driftmark: `presets = [...]` names the ones a repository uses, and a repository with no rules file uses the ones whose layout it has (`docs/features/presets.md`). An optional `[lint]` table holds the writing check's settings, an optional `[masthead]` table adds masthead keys to the known ones, and an optional `[compare]` table names the branch `auto` and the home page compare with.

## Who reads it

- The program finds the file and reads its text. It is `driftmark.toml` at the repository root. `driftmark lint` reads it from the disk. `driftmark diff` reads it at the newer side of the range: at the head commit, or from the disk for `auto` (`docs/features/commands/diff.md`). `driftmark serve` reads it at the side a page shows: at the commit, or from the disk for the working tree (`docs/features/commands/serve.md`).
- `driftmark_core::rules::Rules::parse(text)` parses the text, so the core never touches a file. It keeps the names in `presets = [...]` without their types.
- `driftmark_core::presets::Setup::build(own, &files)` adds the presets: those the parsed file `own` names, after its own types, or, when `own` is `None`, every preset whose markers `files` holds. `files` is a `RepoFiles`, which the program answers for one side: a commit, or the working tree on disk. Every command reads its rules this way.
- `rules.classify(path, &document)` applies the rules to one parsed document. `path` is relative to the repository root, with `/` between folders, as git writes it. `rules.classify_with(path, &document, &files)` does the same and also reads a location's file beside the document.
- With no rules file and no preset found, the rules have no types, so every file is a `document`.
- `rules.masthead_keys()` gives the masthead keys to parse documents with: `Document::parse_with(bytes, rules.masthead_keys())`. See "Masthead keys" below.

## An example

This made-up file is also the test fixture in `fixtures/rules-file/`, so the example is tested.

```toml
# Rules for a made-up project that uses driftmark's starter layout, plus an
# older folder of adr-tools records.

[[type]]
name = "decision"
paths = ["docs/decisions/*.md"]
status = { front-matter = "status", masthead = "Status" }
current = ["accepted"]
superseded = ["superseded"]
archived = ["rejected", "withdrawn"]
date = { front-matter = "date", masthead = "Date" }
superseded-by = { front-matter = "superseded-by", masthead = "Superseded by" }
decision-headings = ["Decision"]

[[type]]
name = "feature"
paths = ["docs/features/**/*.md"]
status = { front-matter = "status" }
current = ["target", "shipped"]
archived = ["dropped"]
lifecycle = "current"

[[type]]
name = "standard"
paths = ["docs/standards/*.md"]
lifecycle = "current"

[[type]]
name = "archive"
paths = ["docs/archive/**"]
lifecycle = "archived"

[[type]]
name = "adr"
paths = ["doc/adr/*.md"]
status = { section = "Status" }
current = ["accepted"]
superseded = ["superseded", "superceded"]
decision-headings = ["Decision"]
```

What it gives for a few files of that project:

| File | Type | Status | Lifecycle | Why |
|---|---|---|---|---|
| `docs/decisions/0001-use-markdown.md` | decision | `accepted` | current | Front matter `status: accepted`; `accepted` is in `current`. |
| `docs/decisions/0002-one-big-file.md` | decision | `Superseded` | superseded | Masthead `> Status: Superseded \| Superseded by: [0003 Many small files](0003-many-small-files.md)`. "Superseded by" is `0003 Many small files`, with the link `0003-many-small-files.md`. |
| `docs/decisions/0003-many-small-files.md` | decision | `proposed` | none | `proposed` is in no list, and the type has no default lifecycle. |
| `docs/features/export.md` | feature | none | current | No status written, so the type's default applies. |
| `docs/archive/2026-01-old-plan.md` | archive | none | archived | The type names no status location, so its `> Status: Done` line is not read. |
| `doc/adr/0001-record-architecture-decisions.md` | adr | `Superceded by 2. Record decisions in the wiki` | superseded | The first line of text under `## Status`. |
| `notes/ideas.md` | document | none | none | No type matches. |

## The format

The file is TOML. It holds an optional `presets` list, a list of `[[type]]` tables and, optionally, one `[lint]` table, which `docs/features/lint.md` describes, one `[masthead]` table and one `[compare]` table. Nothing else may be in it. An unknown key anywhere is an error, so a typo such as `stauts` is caught, not ignored.

### Presets

`presets` names built-in presets whose types the repository uses:

```toml
presets = ["openspec", "adr"]
```

- The file's own types come first, then each preset's, in the order named. A file can therefore type some of a preset's paths its own way.
- The names are `openspec`, `spec-kit`, `adr` and `kiro`. A name driftmark does not know is a `Syntax` error with its line: ``driftmark has no preset "backlog"; the presets are "openspec", "spec-kit", "adr", "kiro"``. A name written twice counts once.
- The masthead keys of every named preset are known too.
- A preset gives types and masthead keys only. The `[lint]` and `[compare]` tables come from this file.
- A rules file with no `presets` key uses no preset, even where a preset's markers are. With no rules file at all, driftmark uses every preset whose markers it finds.

`docs/features/presets.md` describes each preset, its markers and how to copy one into this file.

### Keys of a type

| Key | Required | Value | Meaning |
|---|---|---|---|
| `name` | yes | text | The type's name, such as `decision`. Two types may share a name, for example when one type lives in two layouts. |
| `paths` | yes | list of patterns | The files of this type. See "Paths" below. |
| `status` | no | a location | Where the status is written. |
| `current` | no | list of text | Status values that mean the document says what is true now. |
| `superseded` | no | list of text | Status values that mean another document replaced this one. |
| `archived` | no | list of text | Status values that mean the document is kept only as a record. |
| `lifecycle` | no | `"current"`, `"superseded"` or `"archived"` | The lifecycle of a document of this type whose status does not say otherwise. |
| `date` | no | a location | Where the date is written. |
| `superseded-by` | no | a location | Where the document names the one that replaced it. |
| `decision-headings` | no | list of text | Heading titles whose sections hold decisions, such as `Decision`. |
| `other-statuses` | no | list of text | Status values that are allowed but mean none of the three lifecycles, such as `proposed` or `draft`. |
| `required` | no | list of text | Front matter keys every document of this type must have, with a value. The writing check reports the ones a document lacks (`required-field`). |

### Paths

- The types are tried in the order the file lists them. The first type with a matching pattern wins.
- A pattern is matched against the whole path, relative to the repository root.
- `*` matches any run of characters within one folder or file name. `?` matches one character. Neither crosses a `/`.
- `[abc]` matches one of the listed characters, `[a-z]` one in the range, and `[!abc]` one not listed.
- `**` matches any number of folders, none included. It must be a whole part of the path, as in `docs/**/*.md` or `docs/archive/**`.
- Case matters. `*` also matches names that start with a dot.
- There is no `{a,b}` choice. List several patterns instead.

| Pattern | Matches | Does not match |
|---|---|---|
| `docs/decisions/*.md` | `docs/decisions/0001-a.md` | `docs/decisions/old/0001-a.md` |
| `docs/features/**/*.md` | `docs/features/a.md`, `docs/features/x/y/a.md` | `docs/feature.md` |
| `docs/archive/**` | every file under `docs/archive/` | `docs/archive.md` |
| `specs/*/spec.md` | `specs/001-login/spec.md` | `specs/a/b/spec.md` |
| `adr/[0-9]*.md` | `adr/0001-x.md` | `adr/template.md` |

### Locations

A location says where to find one value. It is a table with one or more of four places:

| Key | Reads |
|---|---|
| `front-matter = "key"` | That key of the YAML front matter. |
| `masthead = "Key"` | That key of the masthead: the `Key: Value` lines right under the title, as a quote, a paragraph or a list. |
| `section = "Title"` | The first line of text under the first heading with that title, outside any container. Deeper headings inside the section are skipped. The section ends at the next heading of the same or a higher level. This is how adr-tools writes status. |
| `file = "name"` with `key = "a.b"` | The value at `key` in the nearest JSON or YAML file called `name`: in the document's folder, or else in the closest folder above it. This is how cc-sdd keeps a spec's phase, in `spec.json`. |

- When a location names more than one place, they are tried in this order: front matter, masthead, section, file. The first value found wins. So `{ front-matter = "status", masthead = "Status" }` suits a folder that is moving from mastheads to front matter.
- Keys match without regard to case, and `-`, `_` and spaces count as the same. `superseded-by` finds `superseded_by:` in front matter and `Superseded by:` in a masthead. Section titles match the same way.
- A value is text. A front matter list or map is not a value, and neither is an empty or blank value. The next place is tried instead.
- Masthead values are their rendered text, so `**Accepted**` reads as `Accepted`. The targets of links in a masthead "superseded by" value come back too.

#### A value in a file beside the document

```toml
[[type]]
name = "requirements"
paths = [".kiro/specs/*/requirements.md"]
status = { file = "spec.json", key = "phase" }
date = { file = "spec.json", key = "updated_at" }
```

For `.kiro/specs/albums/requirements.md`, driftmark reads `.kiro/specs/albums/spec.json`, or else `.kiro/specs/spec.json`, `.kiro/spec.json` and `spec.json`, and takes the first that exists.

- `file` is a file name, without folders. `key` names the value; nested tables are joined with `.`, as in `approvals.requirements.approved`. Keys match as front matter keys do, so `updated-at` finds `updated_at`.
- The file is read as YAML, which also reads JSON. Tabs that indent a JSON line are allowed.
- A value must be text: a number or `true` reads as written. A list, a table, an empty value, a missing key, or a file that is too large (over 2 MiB), not UTF-8 or not valid, gives no value.
- `file` and `key` go together. Either one alone is an `Invalid` type, and so is a file name with a `/` or a `\`, `.` or `..`, or a key with an empty part between its dots.
- Only `classify_with` reads files. The change list compares Markdown files only, so a value from a file takes no part in its ranking (`docs/features/presets.md`, "Not covered"). The writing check reads the document alone, so it does not see such a value either.

`docs/features/engine/document-model.md` has the exact rules for reading front matter and mastheads.

### Masthead keys

A `Key: Value` line under the title is metadata only when its key is known (decision 0009). A quote with no known key stays the summary, however short, so `> Short version: …` is never a field. In a masthead that has known keys, a field with another key is left out. `docs/features/engine/document-model.md` has the exact rules. The known keys are:

- the built-in ones: `Status`, `Date`, `Supersedes`, `Superseded by`, `Owner`, `Type`, `Tags` and `Version`;
- every key a type's `masthead` location names, such as `Estado` in `status = { masthead = "Estado" }`;
- the keys the `[masthead]` table lists.

```toml
[masthead]
keys = ["Deciders", "Feature Branch", "Created"]
```

- Keys match without regard to case, with `-`, `_` and spaces the same.
- A key must have the shape every masthead key has: it starts with a letter, has one to four words and at most 40 characters, and holds only letters, digits, spaces and `-_/.'()`. Any other key could never match a line, so it is a `Syntax` error with its line: `the masthead key "a*b" cannot match a masthead line; …`.
- One list serves every type, because the masthead is read when a document is parsed, before its type is known.
- A preset for GitHub Spec Kit or MADR 2 lists the keys its layout writes, such as `Created` or `Deciders`, when it needs them as fields. Their `Status` line is read without that, because `Status` is known.

### The default branch

`auto` compares the working tree with the commit where its branch left the default branch, and the home page of `driftmark serve` compares every branch with it (`docs/features/engine/git-ranges.md`). Without a setting, the default branch is the first that exists of `origin/main`, `main`, `origin/master`, `master`, `origin/dev`, `dev`. A repository whose agents start their branches from another one names it:

```toml
[compare]
default-branch = "origin/dev"
```

- The name is used as written. `dev` is the local branch, `origin/dev` the branch as the remote has it; a local branch is often behind, so a shared rules file usually names the remote one.
- `--base <branch>` on `driftmark diff` and `driftmark serve` wins over the file.
- The file is read where `auto` reads it: from the working tree. A bare repository has no working tree, so `driftmark serve` reads it at `HEAD` there.
- A name that is empty, starts with `-`, holds `..`, white space or a control character cannot name a branch, so it is a `Syntax` error with its line: ``the default branch "a b" cannot name a branch; …``.
- A name that no branch has is not an error in the file. Comparing then fails with ``the default branch `dev`, named by `--base` or the rules file, does not name a commit``.
- `rules.default_branch()` returns the name, or `None` without one.

### Status values and lifecycles

A status value means current, superseded or archived when one of the three lists holds it.

- Case does not matter, and neither do symbols before the first letter or digit. `**Accepted**` and `✅ accepted` match `accepted`.
- The status may go on after the listed value, as long as the next character is not a letter or a digit. `Superseded by 0009` matches `superseded`; `acceptedly` does not match `accepted`.
- When several listed values match, the longest wins. With `superseded = ["superseded"]` and `archived = ["superseded in part"]`, the status `Superseded in part by 0010` is archived.
- A status that matches no list, or no status at all, gives the type's `lifecycle`. With no `lifecycle`, it gives none.
- A value in `other-statuses` takes part in the matching like the others, but means no lifecycle. So it also gives the type's `lifecycle`. With `superseded = ["superseded"]` and `other-statuses = ["superseded in part"]`, the status `Superseded in part by 0010` gives the type's `lifecycle`, because the longer value wins.
- The four lists together are every status the type allows. When a type lists at least one value, the writing check reports a status that matches none of them (`status-value`). A type that lists no values accepts any status.

For example, a type for decision records that may still be under discussion:

```toml
[[type]]
name = "decision"
paths = ["docs/decisions/*.md"]
status = { front-matter = "status" }
current = ["accepted"]
superseded = ["superseded"]
other-statuses = ["proposed"]
required = ["status", "date"]
```

Here `status: proposed` is allowed and has no lifecycle. `status: draft` gets a `status-value` warning. A decision without `date:` in its front matter gets a `required-field` error.

### Decision headings

`decision-headings` names headings whose sections hold decisions. The change view ranks changes there above wording changes.

- Titles match like keys: without regard to case, with `-`, `_` and spaces the same. They must match whole: `Decision` does not match `Decisions`.
- A decision heading's block counts, and so does every block in its section: the blocks whose heading path holds that title, sub-sections included.
- Only headings outside any container open sections, as everywhere in the document model.

### Errors

`Rules::parse` returns a `RulesError` instead of a value when the file is wrong. Its text reads as a sentence.

| Error | When |
|---|---|
| `Syntax` | The file is not valid TOML, a key is unknown or missing, a value has the wrong shape, a `[masthead]` key cannot match a line, `default-branch` cannot name a branch, or `presets` names a preset driftmark does not have. The TOML reader's message names the line and column and shows the line. |
| `Invalid` | A type is valid TOML but says something impossible. The message names the type by its position and name, as in `the rules file's type 2 ("feature"): it lists no paths`. |
| `Lint` | The `[lint]` table is valid TOML but names an HTML element that cannot exist, as in ``the rules file's [lint] table: `allowed-html` holds "<b>", …``. An unknown rule id or a limit of 0 is a `Syntax` error instead, because the TOML reader catches it with its line. |

A misspelled key gives this `Syntax` message:

```text
the rules file could not be read: TOML parse error at line 4, column 1
  |
4 | stauts = { masthead = "S" }
  | ^^^^^^
unknown field `stauts`, expected one of `name`, `paths`, `status`, `current`, `superseded`, `archived`, `lifecycle`, `date`, `superseded-by`, `decision-headings`, `required`, `other-statuses`
```

A type is invalid when:

- its name is empty;
- it lists no paths, or a pattern does not parse;
- a location names no place to look, or names an empty key or title;
- a location names a `file` without a `key`, or a `key` without a `file`, a file name with folders in it, or a key with an empty part;
- it lists status values, in any of the four lists, but has no `status` location;
- one status value is in two lists, or has no letters or digits;
- a decision heading is empty;
- a `required` key is empty.

## What `classify` returns

| Field | What it holds |
|---|---|
| `doc_type` | The matching type's name, or `document`. |
| `status` | The status as written, when the type has a `status` location and the document has a value there. |
| `lifecycle` | `Current`, `Superseded` or `Archived`, by the rules above. |
| `date` | The date as written. driftmark does not read it as a date. |
| `superseded_by` | The text naming the document that replaced this one. |
| `superseded_by_links` | The link targets in that text, when it came from the masthead. |
| `decision_blocks` | The indexes of the blocks in decision sections. |

`rules.classify_with(path, &document, &files)` returns the same fields, with the values of `file` places read from `files`. `rules.type_for(path)` returns the matching type itself. `rule.lifecycle_of(status)` and `rule.is_decision_heading(title)` apply one rule each.

Three more lookups serve the writing check:

- `rule.allows_status(status)` says whether any of the four lists holds the status, and `rule.statuses()` lists every value in the order the file writes them.
- `location.find(&document)` reads one location. It returns a `Located`: the value's text, the targets of its links, and the `Place` that held it (`FrontMatter`, `Masthead`, `Section` or `File`), so a finding can point at the right line. `location.find_in(path, &document, &files)` also reads a `file` place.
- `rules.lint()` returns the `[lint]` settings, or the defaults when the file has no `[lint]` table.
- `rules.masthead_keys()` returns the known masthead keys: the built-in ones, the `[masthead]` table's, and those the types' `masthead` locations name.

## Presets

A preset is a rules file in this same format, built into driftmark. Version 1 has four: `openspec`, `spec-kit`, `adr` and `kiro`. `docs/features/presets.md` lists each one's types, statuses, lifecycles and decision headings, and the markers driftmark finds it by. `driftmark presets show <name>` prints one, so it can be copied into `driftmark.toml` and changed.

How the presets meet a rules file is under "Presets" in "The format" above.

## Not covered

- Naming which folders hold documents at all, and leaving files out of the change view. The program decides which files it reads. Only the writing check has an exclude list, `[lint] exclude` (`docs/features/lint.md`).
- Choosing between `{a,b}` in a pattern.
- More than one rules file in a repository.
- Reading a date as a date.
- Checking that a masthead holds the `required` keys. `required` reads front matter only.
