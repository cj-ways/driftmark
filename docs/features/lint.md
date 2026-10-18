---
status: target
date: 2026-10-18
---

# Writing check

> Short version: `driftmark lint` checks that Markdown files keep the shape driftmark renders well (decision 0002). It reports each problem with its file, line, column, rule and a message that names the fix. A `[lint]` table in `driftmark.toml` changes how serious a rule is, turns rules off, sets four size limits and names files to leave out. `driftmark hook install` adds the check to Claude Code, so an agent sees the findings right after each edit.

## Running it

| Command | What it checks |
|---|---|
| `driftmark lint` | Every Markdown file in the repository. |
| `driftmark lint docs specs` | Every Markdown file in those folders. |
| `driftmark lint docs/cache.md` | That one file. |
| `driftmark lint --json docs` | The same, written as JSON. |
| `driftmark lint --hook claude-code` | The file in a Claude Code event read from standard input. See "The Claude Code hook". |

- **Which files.** A Markdown file is one whose name ends in `.md` or `.markdown`, in any case. "Every Markdown file in the repository" means the files git tracks and the new files it would offer to track, as `docs/features/engine/git-ranges.md` says for the working tree. Ignored files are left out, and so are the files the `[lint]` table's `exclude` names.
- **Paths** are read from the current folder. A folder means every Markdown file inside it, except the excluded ones. A file named on the command line is checked even when git ignores it, `exclude` names it, or its name does not end in `.md`.
- **The rules file** is `driftmark.toml` at the repository root. Without one, every rule runs with its default settings.
- **Where it runs.** It must run inside a git repository with a working tree.
- **What it reads.** The files are read from disk as they are now, saved or not committed.
- **Speed.** All files are checked at once, one per processor core.

### Text output

One line per finding, then a summary line:

```text
docs/cache.md:1:1: error title: this comes before the `#` title, and only front matter may; move it below the title
docs/cache.md:5:1: warning summary: the document has no summary before its first section; add a paragraph or a `>` quote under the title that says in a sentence or two what the document is for
docs/cache.md:9:1: warning code-language: this code block names no language; write it after the opening fence, such as ```sh, or ```text for plain text
docs/cache.md:13:5: error anchor: `#install` is not a heading in `docs/setup.md`; link to a heading that exists; its anchor is its text in lower case, with spaces as `-` and punctuation left out
driftmark lint: 2 files checked, 2 errors and 2 warnings in 1 file
```

- The form is `path:line:column: severity rule: message`.
- The path runs from the repository root, with `/` between folders, whatever the current folder is.
- Lines and columns count from 1. A column counts characters, not bytes, so an editor jumps to the right place.
- Findings come in order of path, then line, then column, then the rule's place in the table below.
- With no findings the summary reads `driftmark lint: 12 files checked, no problems found`.

### JSON output

`--json` prints one object:

```json
{
  "files": 1,
  "errors": 2,
  "warnings": 2,
  "findings": [
    {
      "path": "docs/cache.md",
      "line": 1,
      "column": 1,
      "severity": "error",
      "rule": "title",
      "message": "this comes before the `#` title, and only front matter may; move it below the title"
    }
  ]
}
```

`files` counts the files checked. `findings` is one flat list in the order of the text output.

### Exit codes

| Code | When |
|---|---|
| 0 | No finding is an error. Warnings alone exit 0. |
| 1 | At least one finding is an error. |
| 2 | The check could not run: an unknown option, a path that does not exist or lies outside the repository, no git repository, or a rules file that is not valid. The reason goes to standard error. |

## The rules

A rule's severity is its default; the `[lint.rules]` table can change it. Errors break how a page renders or what driftmark knows about a document. Warnings are about shape.

| Rule | Severity | Checks |
|---|---|---|
| `parse` | error | The file can be read as Markdown at all. |
| `title` | error | Exactly one `#` title, first in the file. |
| `summary` | warning | A summary paragraph or `>` quote before the first section. |
| `heading-levels` | warning | No heading skips a level. |
| `heading-length` | warning | No heading is longer than `max-heading-length` characters. |
| `code-closed` | error | Every code block is closed. |
| `code-language` | warning | Every code block names its language. |
| `code-line-length` | warning | No code line is longer than `max-code-line-length` characters. |
| `diagram` | error | Every Mermaid diagram parses, and diagram languages are in lower case. |
| `html` | warning | Raw HTML uses only the elements in `allowed-html`. |
| `front-matter` | error | The front matter is valid YAML. |
| `required-field` | error | The front matter holds every key the document's type requires. |
| `status-value` | warning | The status is one the document's type lists. |
| `link` | error | Relative links lead to files or folders that exist. |
| `anchor` | error | `#anchors` lead to headings that exist. |
| `table-columns` | warning | No table has more than `max-table-columns` columns. |
| `file-size` | warning | The file is at most `max-file-size` bytes. |

Every rule has a folder in `fixtures/lint/<rule>/` with a `pass.md` that no rule faults, a `fail.md` that only that rule faults, and the exact findings in `fail.expected.txt`. The examples below are shorter versions of the same.

### `parse`

The file must be UTF-8 text, and its block quotes, lists, alerts and footnotes must nest at most 100 levels deep. When it cannot be read, this is the file's only finding, because no other rule can run.

```text
fail.md:5:1: error parse: the file is not UTF-8 text: byte 150, on line 5, is not valid UTF-8; save the file as UTF-8
```

### `title`

The file starts with one `#` title. Only front matter may come before it, and HTML comments, which render nothing. A level-1 heading underlined with `===` counts as a `#` title. Headings inside quotes, lists, alerts and footnotes do not count, as in the document model.

Three findings are possible: no title at all, something before the title, and each extra title.

Fails:

```markdown
Notes first.

# Cache pages

Each page is kept for one hour.

# Appendix
```

```text
fail.md:1:1: error title: this comes before the `#` title, and only front matter may; move it below the title
fail.md:7:1: error title: this is a second `#` title, and a file has exactly one; make it a `##` section, or move it to a file of its own
```

Passes:

```markdown
---
status: accepted
---
<!-- made by the release tool -->
# Cache pages

Each page is kept for one hour.

## Appendix
```

### `summary`

Between the title and the first heading after it, there must be a paragraph or a `>` quote that is not a masthead. A list, a table, a code block or an alert does not count. Without a title, the search starts at the top of the file.

The masthead rules come from `docs/features/engine/document-model.md`. A quote is a masthead only when it holds a known masthead key, such as `Status` or `Owner`, or one the rules file's `[masthead]` table adds (decision 0009). So `> Short version: pages expire after an hour.` is a summary, however short.

Fails, because the quote is a masthead:

```markdown
# Cache pages

> Status: Accepted | Date: 2026-01-05

## Decision
```

```text
fail.md:5:1: warning summary: the document has no summary before its first section; add a paragraph or a `>` quote under the title that says in a sentence or two what the document is for
```

Passes, because `Short version` is not a masthead key:

```markdown
# Cache pages

> Short version: pages expire after an hour.

## Decision
```

Passes too, with a masthead and then a summary paragraph:

```markdown
# Cache pages

> Status: Accepted | Date: 2026-01-05

The server keeps each rendered page for one hour, so a second visit costs nothing.

## Decision
```

### `heading-levels`

A heading may be at most one level deeper than the heading before it. Going back up any number of levels is fine. Only headings outside quotes, lists, alerts and footnotes count, because only they make the document's outline.

Fails:

```markdown
# Cache pages

Each page is kept for one hour.

### Expiry
```

```text
fail.md:5:1: warning heading-levels: this `###` heading skips a level after the `#` heading above it; make it `##`
```

Passes: the same with `## Expiry`.

### `heading-length`

A heading's text, without markup, has at most `max-heading-length` characters, 100 by default. A longer heading is a sentence, which belongs in the text under it. Every heading counts, inside containers too.

```text
fail.md:5:1: warning heading-length: this heading has 110 characters, more than the limit of 100; shorten it and say the rest in the text below it
```

### `code-closed`

Every fenced code block and diagram ends with a closing fence. An unclosed fence turns the rest of the file into code. Indented code blocks are always closed. The message names the block's own fence, such as ```` ``` ```` or `~~~~`.

Fails:

````markdown
## Build

```sh
cache build --release

## Checks
````

```text
fail.md:3:1: error code-closed: this code block is never closed, so the rest of the file shows as code; add a closing ``` line where the code ends
```

Passes: the same with a closing ```` ``` ```` line after the command.

### `code-language`

Every code block names its language after the opening fence, such as ```` ```sh ````. Plain text names `text`. An indented code block cannot name a language, so it is reported too, with advice to fence it. Diagrams always name theirs.

Fails:

````markdown
```
cache.ttl = 3600
```

    make release
````

```text
fail.md:1:1: warning code-language: this code block names no language; write it after the opening fence, such as ```sh, or ```text for plain text
fail.md:5:5: warning code-language: this indented code block cannot name its language; write it as a fenced block instead, such as ```sh, or ```text for plain text
```

Passes: ```` ```toml ```` and ```` ```sh ```` fences around the same lines.

### `code-line-length`

A line inside a code block has at most `max-code-line-length` characters, 120 by default. Each long line is one finding. Its column is the first character past the limit. Indentation and quote markers before the code do not count. Diagram code is not checked, because it is drawn, not shown.

Program output is not checked either (decision 0009). A code block marked `text`, `console`, `output` or `log`, in any case, holds what a program printed, and pages wrap its long lines instead. The list is `OUTPUT_LANGUAGES` in `driftmark_core::lint`. The rule's `pass.md` fixture holds a `text` block and a `console` block whose lines are longer than 120 characters, and no finding.

```text
fail.md:8:121: warning code-line-length: this code line has 123 characters, more than the limit of 120; break it into shorter lines
```

### `diagram`

Two checks.

1. **Every Mermaid diagram parses.** The program checks it with merman's parser (decision 0008), through the `DiagramCheck` trait in the core. The finding points at the place merman names. When merman names no place, it points at the opening fence. An unclosed diagram is left to `code-closed`.
2. **Diagram languages are in lower case.** GitHub draws only ```` ```mermaid ````, so driftmark does the same. A block written ```` ```Mermaid ```` or ```` ```D2 ```` shows as code.

D2 diagrams are not parsed. driftmark has no D2 parser, and the `d2` program may not be installed.

Fails:

````markdown
```mermaid
flowchart TD
  Tag[Tag the commit] --> Build[Build the packages]
  Build -->
```

```Mermaid
flowchart TD
  Build --> Upload
```
````

```text
fail.md:4:12: error diagram: this Mermaid diagram will not draw: unexpected end of input; expected node identifier, edge label; fix the diagram code
fail.md:7:1: error diagram: this block is written ```Mermaid, so it shows as code instead of a diagram; write ```mermaid in lower case
```

A diagram whose first line names no diagram type gets "its first line names no diagram type; start it with one, such as `flowchart TD` or `sequenceDiagram`".

### `html`

Raw HTML may use only the elements in `allowed-html`. By default those are `details`, `summary`, `br`, `sub`, `sup` and `kbd`. They render the same on GitHub, GitLab and driftmark, and Markdown has no way to write them.

- HTML blocks and inline HTML both count.
- Only opening tags are checked, so one element gives one finding.
- Element names are compared in lower case, so `<DETAILS>` is allowed.
- HTML comments are allowed, and tags inside a comment are not checked.
- Text such as `a < b` is not a tag. Code spans and code blocks are not HTML.

Fails:

```markdown
<div align="center">
  <img src="logo.png" alt="Cache logo">
</div>

Keep the <span style="color: red">cache</span> small.
```

```text
fail.md:1:1: warning html: `<div>` is raw HTML outside the allowed list; write it in Markdown, or add "div" to `allowed-html` in the rules file's `[lint]` table
fail.md:2:3: warning html: `<img>` is raw HTML outside the allowed list; write it in Markdown, or add "img" to `allowed-html` in the rules file's `[lint]` table
fail.md:5:10: warning html: `<span>` is raw HTML outside the allowed list; write it in Markdown, or add "span" to `allowed-html` in the rules file's `[lint]` table
```

Passes:

```markdown
<details>
<summary>Old steps</summary>

Press <kbd>Ctrl</kbd> and <kbd>C</kbd> to stop the build.

</details>
```

### `front-matter`

YAML front matter between `---` lines must parse. The finding is on the line the YAML error names. The message is the document model's own sentence about the error.

Fails:

```markdown
---
status: [accepted
date: 2026-01-05
---
```

```text
fail.md:3:1: error front-matter: the front matter is not valid YAML: illegal placement of ':' indicator, on line 3 of the file; fix the YAML between the `---` lines
```

### `required-field`

A document type in the rules file can list front matter keys every document of that type needs, with `required = ["status", "date"]`. A key whose value is empty or blank counts as missing. A list or a map counts as a value.

- One finding lists every missing key. It is on line 1.
- Only front matter is read. A masthead line does not satisfy `required`.
- When the front matter is not valid YAML, this rule waits for `front-matter` to be fixed first.
- A document that no type matches has no required keys.

With this rules file:

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

this fails:

```markdown
---
status: accepted
---

# Cache pages
```

```text
docs/decisions/0004-cache-pages.md:1:1: error required-field: the front matter lacks `date`, which every `decision` document needs; add it with a value
```

A file with no front matter gets "the file has no front matter, and every `decision` document needs `status` and `date` there; add front matter between `---` lines at the top of the file".

### `status-value`

When a document type lists status values, its documents' status must be one of them. The allowed values are the type's `current`, `superseded`, `archived` and `other-statuses` lists together. They match as `docs/features/engine/rules-file.md` says: case and leading symbols do not matter, and the status may go on after the value, so `Superseded by 0009` matches `superseded`.

- The status is read from the type's `status` location, wherever that is: front matter, masthead or section.
- The finding points at the status where it is written.
- A type that lists no status values accepts any status.
- A document with no status is not this rule's business; `required` can ask for one.

With the rules file above, `status: draft` fails:

```text
docs/decisions/0004-cache-pages.md:2:9: warning status-value: the status "draft" is not one the `decision` type lists; use `accepted`, `superseded` or `proposed`
```

### `link`

Every relative link and image must lead to a file or folder that exists in the repository.

- **Which links.** Inline links, reference links (with the target their definition gives), autolinks and images all count. Links inside raw HTML are not read.
- **Links that are not checked:**
  - a target with a scheme, such as `https:`, `mailto:` or `ftp:`;
  - a target that starts with `//`;
  - an empty target.
- **Where a path starts.** A path that starts with `/` starts at the repository root, as on GitHub. Any other path starts at the linking file's folder. `.` and `..` work as usual.
- **What is ignored.** `?query` is ignored, and `%20`-style escapes are decoded.
- **Case.** A name must match exactly, case included, on every system. GitHub and Linux treat `Setup.md` and `setup.md` as two files, so a link that only works on a Mac or Windows disk is still broken.
- **Outside the repository.** A path that climbs above the repository root is reported, because it breaks on GitHub.

Fails:

```markdown
- [The setup guide](guides.md) has moved.
- [Notes outside the repository](../../notes.md).
- [The cache flow](images/flow.png).
```

```text
docs/cache.md:1:3: error link: the link `guides.md` leads to `docs/guides.md`, which does not exist; fix the path, or create the file
docs/cache.md:2:3: error link: the link `../../notes.md` leads outside the repository; link to a file inside it, or write a full web address
docs/cache.md:3:3: error link: the link `images/flow.png` leads to `docs/images/flow.png`, which does not exist; fix the path, or create the file
```

When the path is the same as the link, as for a file at the root, the message is shorter: "the link `guides.md` names a file that does not exist".

Passes, when `docs/setup.md` and the folder `docs/notes/` exist:

```markdown
[The setup guide](setup.md), [its install step](setup.md#install), [the notes](notes/), [the site](https://example.com/cache).
```

### `anchor`

A link's `#anchor` must name a heading of the file it leads to.

- **Which links.** A link to the same file, such as `#decision`, and a link to another Markdown file, such as `setup.md#install`, are checked. An anchor into any other file, such as `main.go#L10`, is not.
- **How anchors are made.** They follow GitHub. A heading's text is put in lower case. Every character is dropped that is not a letter, a digit, a mark, `-`, `_` or a space. Each space becomes `-`. A second heading with the same anchor gets `-1`, a third `-2`. So `## Cache: one hour` gets `#cache-one-hour`.
- **Which headings.** Every heading counts, inside containers too.
- **A link in the wrong case** is told the right spelling.
- **An unreadable target.** A link into a Markdown file that cannot be parsed is not checked.

Fails, when `setup.md` has the headings `# Setup guide`, `## Install` and `## Configure`:

```markdown
- [Install first](setup.md#Install), then [configure](setup.md#setup).
- [The steps above](#step).
```

```text
fail.md:1:3: error anchor: `#Install` is not a heading in `setup.md`; anchors are lower case, so write `#install`
fail.md:1:43: error anchor: `#setup` is not a heading in `setup.md`; link to a heading that exists; its anchor is its text in lower case, with spaces as `-` and punctuation left out
fail.md:2:3: error anchor: `#step` is not a heading in `fail.md`; link to a heading that exists; its anchor is its text in lower case, with spaces as `-` and punctuation left out
```

### `table-columns`

A table has at most `max-table-columns` columns, 8 by default, because wider tables do not fit a phone screen. The finding is on the header row, once per table.

```text
fail.md:7:1: warning table-columns: this table has 9 columns, more than the limit of 8; split it into narrower tables, or turn some columns into rows
```

### `file-size`

A file has at most `max-file-size` bytes, 100,000 by default. An agent reads a 100 KB file as about 25,000 tokens, and a reader no longer finds their way in it. The finding is on line 1.

```text
fail.md:1:1: warning file-size: the file has 456 bytes, more than the limit of 400; split it into smaller documents that link to each other
```

## Settings: the `[lint]` table

The rules file `driftmark.toml` may hold one `[lint]` table beside its `[[type]]` tables. Every key is optional. An unknown key is an error, as everywhere in the rules file.

```toml
[lint]
exclude = ["fixtures/**", "CLAUDE.md"]
allowed-html = ["details", "summary", "br", "sub", "sup", "kbd", "img"]
max-table-columns = 6
max-code-line-length = 100
max-file-size = 50000
max-heading-length = 80

[lint.rules]
code-language = "error"
heading-length = "off"
summary = "warning"
```

| Key | Default | Meaning |
|---|---|---|
| `exclude` | `[]` | Path patterns of files that `driftmark lint` skips when it walks the repository or a folder, and that the Claude Code hook leaves alone. A file named on the command line is still checked. |
| `allowed-html` | `["details", "summary", "br", "sub", "sup", "kbd"]` | The HTML elements raw HTML may use. The list replaces the default; it does not add to it. Names are compared in lower case. |
| `max-table-columns` | 8 | The most columns a table may have. |
| `max-code-line-length` | 120 | The most characters a code line may have. |
| `max-file-size` | 100000 | The most bytes a file may have. |
| `max-heading-length` | 100 | The most characters a heading may have. |
| `[lint.rules]` | every rule at its default severity | Rule ids as keys, each set to `"error"`, `"warning"` or `"off"`. |

- A rule that is off does not run at all. So turning `diagram` off also skips the Mermaid parser.
- `exclude` patterns are matched against the path from the repository root, as the rules file's `paths` are (`docs/features/engine/rules-file.md`): `*` stays within a folder, and `**` crosses folders. `fixtures/**` leaves out everything under `fixtures/`, and `CLAUDE.md` only the file at the root. Test files that break the rules on purpose belong here; this repository's own `driftmark.toml` excludes `fixtures/**` and `CLAUDE.md`.
- `required` and `other-statuses` belong to each `[[type]]`, not to `[lint]`, because they differ by document type. `docs/features/engine/rules-file.md` lists them.

These mistakes are refused, and `driftmark lint` exits with 2:

| Mistake | Message |
|---|---|
| An unknown rule id | The TOML reader's message, with the line and the list of real ids: ``unknown variant `titel`, expected one of `parse`, `title`, …`` |
| A severity other than the three | The TOML reader's message with the line. |
| A limit of 0 or below | The TOML reader's message with the line. |
| An `exclude` entry that is not a valid pattern | ``the rules file's [lint] table: `exclude` holds "a/**b", which is not a valid pattern: …`` |
| An `allowed-html` entry that is not an element name | ``the rules file's [lint] table: `allowed-html` holds "<b>", which is not an HTML element name; write names such as "details", without `<` or `>` `` |

### Why these defaults

The limits were set by looking at a private corpus of 3,423 agent-written documents:

| Limit | Default | Over the limit in the corpus |
|---|---|---|
| Table columns | 8 | 20 of 6,182 tables |
| Code line length | 120 | about 2% of 111,000 code lines |
| File size | 100,000 bytes | 29 of 3,423 files |
| Heading length | 100 characters | about 5% of 43,600 headings |

Each limit flags the far end, not the habit.

## The Claude Code hook

### Installing it

`driftmark hook install` prints the entry it would add to the repository's `.claude/settings.json`, and changes nothing:

```text
driftmark hook install would add this entry to `hooks.PostToolUse` in .claude/settings.json:

{
  "matcher": "Edit|Write|MultiEdit",
  "hooks": [
    {
      "type": "command",
      "command": "driftmark lint --hook claude-code",
      "timeout": 30
    }
  ]
}

Run `driftmark hook install --write` to add it.
```

`driftmark hook install --write` adds it.

- **The file.** It works from any folder inside the repository. The settings file is `.claude/settings.json` at the repository root. It is made, with its folder, when missing.
- **Where the entry goes.** The entry is added as a new matcher group at the end of `hooks.PostToolUse`. `hooks` and `PostToolUse` are made when missing.
- **Other settings.** Every other setting, hook and key stays, in its order. The file is written back with its own indentation and line endings, and with a final line ending only if it had one. Values never change, but JSON escapes such as `é` and number spellings such as `1e3` come back in serde_json's spelling.
- **Running it twice adds nothing.** The second run says `.claude/settings.json already runs `driftmark lint --hook claude-code` after edits; nothing to add`. The hook counts as installed when any `PostToolUse` handler's command is `driftmark lint --hook claude-code`, whatever its matcher or timeout, with the program named by any path, such as `/opt/bin/driftmark`.
- **What it refuses.** It refuses, with exit code 1, and leaves the file as it was, when:
  - the file is not valid JSON;
  - its top level is not an object;
  - `hooks` is not an object;
  - `hooks.PostToolUse` is not a list.
  A dry run refuses the same files.
- **Atomic write.** The new file is written beside the old one and then moved over it, so a crash never leaves half a file.

Exit codes: 0 when the entry was added, would be added, or is already there; 1 when refused or outside a repository; 2 for unknown arguments.

### What the entry means

| Field | Value | Why |
|---|---|---|
| `matcher` | `Edit\|Write\|MultiEdit` | The tools that change a file's text. `MultiEdit` is not in Claude Code's current tool list; it stays for older versions, and a name that matches no tool matches nothing. |
| `type` | `command` | A shell command. |
| `command` | `driftmark lint --hook claude-code` | Shell form, so `driftmark` is found on `PATH` the way the user's shell finds it. That includes npm's `.cmd` wrapper on Windows, which exec form cannot start. |
| `timeout` | 30 | Claude Code's default for a command hook is 600 seconds. One file takes milliseconds, so a hung check is cut off much sooner. |

`driftmark` must be on the `PATH` that Claude Code's hooks see.

### What the hook does

Claude Code runs the command after each matching tool call, with the event as JSON on standard input. `driftmark lint --hook claude-code`:

1. reads `tool_input.file_path`, an absolute path for the file tools. A relative path is read from the event's `cwd`;
2. stops, silent with exit code 0, when:
   - there is no path;
   - the file is not Markdown;
   - the file no longer exists;
   - the file is not inside a git repository;
3. reads that repository's `driftmark.toml`. When its `[lint]` table's `exclude` names the file, it stops, silent with exit code 0: the hook runs after every edit anywhere in the repository, as a walk does, and a fixture that breaks the rules on purpose must not send the agent off to fix it. Otherwise it lints the one file;
4. with no findings, exits 0 and prints nothing;
5. with findings, writes them to standard error and exits 2. The report starts with `driftmark lint found 2 problems in docs/cache.md. Fix them in the file:`, then has one finding per line, in the text format above. Warnings are reported too, because the agent can fix them as cheaply as errors.

It lists at most 50 findings. A longer report ends with `…and 10 more; run `driftmark lint docs/many.md` to see them all.`, so one bad file cannot fill the agent's context.

A rules file that is not valid also gets exit code 2, with the reason, so Claude can fix it. Input that is not JSON gets exit code 1, which Claude Code shows to the user as a hook error, not to Claude.

### Claude Code's hook contract

From Claude Code's hooks reference, [code.claude.com/docs/en/hooks](https://code.claude.com/docs/en/hooks), read on 2026-10-18:

- **"Hook locations":** `.claude/settings.json` is the project's file, "Yes, can be committed to the repo".
- **"Exit code 2 behavior per event":** for `PostToolUse`, exit 2 "Shows stderr to Claude; the tool already ran".
- **"Exit code 0":**
  - "Claude never sees stderr from a hook that exits 0."
  - "To surface a warning to Claude from a `PostToolUse` or `PostToolUseFailure` hook, exit 2 instead so Claude sees the stderr even though the tool already ran."
  - For `PostToolUse`, plain stdout goes to the debug log, not to Claude.
- **"Other exit codes":** any other code is "a non-blocking error". The user sees a notice with the first line of stderr.
- **"PostToolUse input":** the event carries `tool_name`, `tool_input` and `tool_response`. File-tool paths are "always absolute, with the platform's native separators". The `Write` and `Edit` input tables name the field `file_path`.
- **"Common fields":** `timeout` is "Seconds before canceling", with a default of 600 for `command` hooks.
- **"Exec form and shell form":** without `args`, the command runs in a shell: `sh -c` on macOS and Linux, Git Bash on Windows, or PowerShell when Git Bash isn't installed.
- **"Disable or remove hooks":** "Direct edits to hooks in settings files are normally picked up automatically by the file watcher."
- **"Hook handler fields":** "If you define the same handler in more than one settings file, it runs once."

The tools reference, [code.claude.com/docs/en/tools-reference](https://code.claude.com/docs/en/tools-reference), lists `Edit`, `Write` and `NotebookEdit` as the tools that change files, and no `MultiEdit`.

**Why exit code 2 and not JSON.** A `PostToolUse` hook could instead exit 0 and print `{"decision": "block", "reason": …}` or `hookSpecificOutput.additionalContext`. Exit code 2 with standard error is the form the reference names for this exact case. It needs no JSON on the hook's side, and it has worked the same way across Claude Code versions.

## Speed

The hook runs after every edit, so one document must be checked in under 50 milliseconds. Times are on the maintainer's server, release build, at a load average between 3 and 7.

| What | Time |
|---|---|
| The hook, start to exit, on a file with a Mermaid diagram, in a made-up repository of 3,400 files | 6.0 ms median, 6.6 ms slowest of 30 |
| The same on a file without a diagram | 2.9 ms median, 3.4 ms slowest |
| One file of a private 3,423-file corpus, with nothing cached, inside one process | 0.27 ms median, 4.6 ms at the 99th percentile, 15.5 ms for the slowest file |
| The whole corpus, 54.5 MB, on 12 threads | 0.2 seconds |
| The whole made-up repository with `driftmark lint` | 0.28 seconds |

- About 2.5 ms of the diagram case is making merman's parser. It is made only when a file has a Mermaid block.
- No file of the corpus took 50 ms or more.
- The 3 ms difference between the two hook runs is merman: making its parser and parsing the diagram. The rest is starting the program and opening the repository.

## Not covered

- **D2 diagrams** are not parsed.
- **Anchors from HTML.** An `id` or `name` set in raw HTML does not count as an anchor, and links inside raw HTML are not checked.
- **Ignored files.** A link to a file git ignores counts as existing, because the check looks at the disk.
- **Required keys in a masthead.** `required` reads front matter only.
- **Fixing findings** automatically, presets, and a CI mode come later in M4.
- **Hooks for other agents.**
  - Codex has a `PostToolUse` hook that also reports back through exit code 2 and standard error. Its edits arrive as a patch in `tool_input.command`, so a hook must read the file paths out of the patch. Project hooks need the user's trust review.
  - Cursor's `afterFileEdit` hook gets `file_path`, but its reference documents no output the agent sees. Its `postToolUse` hook can return `additional_context`, which reaches the conversation.
