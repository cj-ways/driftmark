---
status: target
date: 2026-10-18
---

# The MCP connector

> Short version: `driftmark mcp` is the same program, started by an agent such as Claude Code, Codex or Cursor. It answers over MCP (the Model Context Protocol) on standard input and output. Five tools let the agent see what changed in the docs between two git points, read one document's changes, read a document or one section of it, search every document, and run the writing check. Every answer is JSON, holds at most 16,000 characters unless the call asks for up to 40,000, and says what it cut and how to ask for the rest. The connector reads only through git and the repository's files, never writes a document, and runs no program but git. `driftmark mcp install` adds it to a repository's `.mcp.json`.

## Running it

```text
driftmark mcp [--repo <path>] [--base <branch>]
driftmark mcp install [--write]
```

| Option | Meaning |
|---|---|
| `--repo <path>` | A folder inside the repository. The default is the current folder, which is the project folder when Claude Code starts the connector. A bare repository works too; it has no working tree. |
| `--base <branch>` | The branch the range `auto` compares with. It wins over the rules file's `[compare] default-branch`. A call's own `base` wins over both. |
| `--help` | Print the usage text. |

An agent starts the connector itself; a person runs only `driftmark mcp install`. The connector reads requests on standard input and writes answers on standard output, one JSON-RPC message per line. It writes nothing else there. When the agent closes standard input, the connector stops with exit code 0.

Between calls the connector keeps parsed documents and counted ranges in a cache of 32 MB. It gives memory back as `driftmark serve` does (`docs/features/commands/serve.md`, "Memory"; decision 0011). It happens after each range count and each search index build, and once no call ran for one second. The connector drops its spare git readers, and on Linux with glibc it hands freed memory back to the system.

| Exit code | When |
|---|---|
| 0 | The agent ended the session, or `install` did its work. |
| 1 | The connector could not start: the folder is not inside a git repository. Or `install` refused, for a reason it names on standard error. |
| 2 | The command line is wrong. The reason and the usage text go to standard error. |

The connector speaks MCP through the `rmcp` crate, version 3.5.1, pinned. Only `crates/driftmark/src/mcp/protocol.rs` names it, so rmcp's next breaking release changes that one file (decision 0003). It answers both handshakes clients use today: the `initialize` request of MCP 2025-11-25 and earlier, and the per-request metadata of MCP 2026-07-28.

## The example repository

Every example below was made on this made-up repository, and `crates/driftmark/tests/mcp.rs` checks each answer against it. The rules file makes `docs/decisions/*.md` decision records:

```toml
[[type]]
name = "decision"
paths = ["docs/decisions/*.md"]
status = { front-matter = "status" }
current = ["accepted"]
other-statuses = ["proposed"]
decision-headings = ["Decision"]
```

On `main`, the repository holds the decision record `docs/decisions/0002-cache-rendered-pages.md` as `docs/features/engine/change-list.md` shows it before its change, with the status `proposed`. It also holds this guide, `docs/guide.md`:

```markdown
# Cache guide

> Short version: how to turn the page cache on and size it.

## Install

Run `cache install` once on each server.

## Configure

Set `cache.size` in megabytes. The default keeps 500 pages.
```

The branch `feature` accepts the decision, as change-list.md shows it after its change, and adds the note `docs/notes/cache-sizes.md`. `origin/main` points at `main`, so `main` is where `auto` compares from. The checkout is on `feature`, and one edit is not committed yet: a code block without a language at the end of the guide.

## Tools

Each tool takes its arguments as one JSON object. A call with an argument the tool does not take, a value of the wrong kind or a missing required argument is refused with a sentence that names the fix (see "Errors"). Every tool also takes `max_chars` (see "Size limits").

| Tool | What it answers |
|---|---|
| `changes_summary` | Which documents changed between two git points, each with its highest level and its counts. |
| `changes_document` | How one document changed: every change, its level, its signals and its words. |
| `document_read` | One document at a revision: metadata, outline and text, whole or for one section. |
| `search` | The documents that hold every word of a query, best first, with snippets. |
| `lint` | The writing check on given files and folders, or on the files changed since the last commit. |

Every tool only reads. The tool list says so to the client: each tool carries `readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true` and `openWorldHint: false`.

### `changes_summary`

The documents that changed in a range, ranked as `driftmark diff` ranks them: the highest level first, then the most changes.

| Argument | Meaning |
|---|---|
| `range` | The two git points, in the forms of `docs/features/engine/git-ranges.md`, written as the server's addresses write them: `auto` (the default), `<base>..<head>`, `<base>...<head>`, `since/<date>` or `since/<date>/<branch>`. |
| `base` | Only with `auto`: the branch to compare with, such as `origin/dev`. |
| `offset` | How many documents to skip. The default is 0. |
| `limit` | The most documents to list, from 1 to 500. The default is 25: each document costs about 140 tokens. |

The answer is the JSON of `driftmark diff --json=summary` (`docs/features/commands/diff.md`), with `schema_version` 2. Three fields follow:

- `skipped`: the files that could not be compared, each with its `path` and the `reason`, at most 20 of them;
- `page`: the first document listed (`offset`), how many are listed (`shown`) and how many there are in all (`total`);
- `cut`, when the answer leaves documents out (see "Size limits").

`totals` always counts every document of the range, whatever the page shows.

Call `changes_summary` with:

```json
{"range": "main...feature"}
```

It answers:

```json
{
  "schema_version": 2,
  "range": {
    "base": "562c4b3e7a8338f5087597effad34dd9d81371c0",
    "head": "59f1a26ed292405ee4ef67182b681d68723d2726",
    "notes": []
  },
  "totals": {
    "changes": 6,
    "highest": "decision",
    "levels": {"decision": 3, "rule": 0, "fact": 2, "wording": 1, "formatting": 0},
    "kinds": {"formatting": 0, "edited": 2, "added": 2, "removed": 2, "moved": 0, "moved_edited": 0}
  },
  "documents": [
    {
      "path": "docs/decisions/0002-cache-rendered-pages.md",
      "file": "modified",
      "summary": {
        "changes": 5,
        "highest": "decision",
        "levels": {"decision": 3, "rule": 0, "fact": 1, "wording": 1, "formatting": 0},
        "kinds": {
          "formatting": 0,
          "edited": 2,
          "added": 1,
          "removed": 2,
          "moved": 0,
          "moved_edited": 0
        }
      }
    },
    {
      "path": "docs/notes/cache-sizes.md",
      "file": "added",
      "summary": {
        "changes": 1,
        "highest": "fact",
        "levels": {"decision": 0, "rule": 0, "fact": 1, "wording": 0, "formatting": 0},
        "kinds": {
          "formatting": 0,
          "edited": 0,
          "added": 1,
          "removed": 0,
          "moved": 0,
          "moved_edited": 0
        }
      }
    }
  ],
  "skipped": [],
  "page": {"offset": 0, "shown": 2, "total": 2}
}
```

### `changes_document`

How one document changed in a range.

| Argument | Meaning |
|---|---|
| `path` | The document, from the repository root. Required. |
| `range`, `base` | As for `changes_summary`. |
| `full_text` | Whether a new or deleted document carries its full text as `source`. The default is `true`. With `false`, its title, summary and size stay. |
| `offset` | How many changes to skip. The default is 0. |
| `limit` | The most changes to list, from 1 to 500. The default is 50. |

The answer is the change list of `driftmark diff --json` with this one document (`docs/features/engine/change-list.md`, `schema_version` 2), with three differences:

1. `unchanged` is left out. It lines up the two versions on a page; an agent reads the changes, and on a long document the pairs alone could fill an answer.
2. `skipped` and `page` are added, as for `changes_summary`. When the document could not be compared, `documents` is empty and `skipped` says why.
3. `cut` is added when the answer leaves changes out.

A path that did not change in the range is refused.

Call `changes_document` with:

```json
{"range": "main...feature", "path": "docs/decisions/0002-cache-rendered-pages.md", "limit": 2}
```

It answers:

```json
{
  "schema_version": 2,
  "range": {
    "base": "562c4b3e7a8338f5087597effad34dd9d81371c0",
    "head": "59f1a26ed292405ee4ef67182b681d68723d2726",
    "notes": []
  },
  "totals": {
    "changes": 5,
    "highest": "decision",
    "levels": {"decision": 3, "rule": 0, "fact": 1, "wording": 1, "formatting": 0},
    "kinds": {"formatting": 0, "edited": 2, "added": 1, "removed": 2, "moved": 0, "moved_edited": 0}
  },
  "documents": [
    {
      "path": "docs/decisions/0002-cache-rendered-pages.md",
      "file": "modified",
      "status": {"old": "proposed", "new": "accepted"},
      "summary": {
        "changes": 5,
        "highest": "decision",
        "levels": {"decision": 3, "rule": 0, "fact": 1, "wording": 1, "formatting": 0},
        "kinds": {
          "formatting": 0,
          "edited": 2,
          "added": 1,
          "removed": 2,
          "moved": 0,
          "moved_edited": 0
        }
      },
      "changes": [
        {
          "kind": "edited",
          "level": "decision",
          "signals": ["status"],
          "old": {
            "at": 0,
            "block": {
              "kind": {"type": "front_matter"},
              "start": 0,
              "end": 24,
              "line": 1,
              "heading_path": [],
              "text": "status: proposed"
            }
          },
          "new": {
            "at": 0,
            "block": {
              "kind": {"type": "front_matter"},
              "start": 0,
              "end": 24,
              "line": 1,
              "heading_path": [],
              "text": "status: accepted"
            }
          },
          "edit": {
            "view": "side_by_side",
            "kept_in_order": 0.5,
            "marks": [
              {"op": "kept", "text": "status: "},
              {"op": "deleted", "text": "proposed"},
              {"op": "inserted", "text": "accepted"}
            ]
          }
        },
        {
          "kind": "edited",
          "level": "decision",
          "signals": ["decision_section", "number"],
          "old": {
            "at": 3,
            "block": {
              "kind": {"type": "paragraph"},
              "start": 63,
              "end": 115,
              "line": 9,
              "heading_path": ["Cache rendered pages", "Decision"],
              "text": "Keep each rendered page in the cache for 60 minutes."
            }
          },
          "new": {
            "at": 3,
            "block": {
              "kind": {"type": "paragraph"},
              "start": 63,
              "end": 115,
              "line": 9,
              "heading_path": ["Cache rendered pages", "Decision"],
              "text": "Keep each rendered page in the cache for 90 minutes."
            }
          },
          "edit": {
            "view": "inline",
            "kept_in_order": 0.9,
            "marks": [
              {"op": "kept", "text": "Keep each rendered page in the cache for "},
              {"op": "deleted", "text": "60"},
              {"op": "inserted", "text": "90"},
              {"op": "kept", "text": " minutes."}
            ]
          }
        }
      ]
    }
  ],
  "skipped": [],
  "page": {"offset": 0, "shown": 2, "total": 5},
  "cut": {
    "reason": "2 of 5 changes are listed, from number 0; ask with `offset` 2 for the rest",
    "next": {
      "range": "main...feature",
      "path": "docs/decisions/0002-cache-rendered-pages.md",
      "limit": 2,
      "offset": 2
    }
  }
}
```

The new note, without its text:

Call `changes_document` with:

```json
{"range": "main...feature", "path": "docs/notes/cache-sizes.md", "full_text": false}
```

It answers:

```json
{
  "schema_version": 2,
  "range": {
    "base": "562c4b3e7a8338f5087597effad34dd9d81371c0",
    "head": "59f1a26ed292405ee4ef67182b681d68723d2726",
    "notes": []
  },
  "totals": {
    "changes": 1,
    "highest": "fact",
    "levels": {"decision": 0, "rule": 0, "fact": 1, "wording": 0, "formatting": 0},
    "kinds": {"formatting": 0, "edited": 0, "added": 1, "removed": 0, "moved": 0, "moved_edited": 0}
  },
  "documents": [
    {
      "path": "docs/notes/cache-sizes.md",
      "file": "added",
      "summary": {
        "changes": 1,
        "highest": "fact",
        "levels": {"decision": 0, "rule": 0, "fact": 1, "wording": 0, "formatting": 0},
        "kinds": {
          "formatting": 0,
          "edited": 0,
          "added": 1,
          "removed": 0,
          "moved": 0,
          "moved_edited": 0
        }
      },
      "changes": [
        {
          "kind": "added",
          "level": "fact",
          "signals": ["heading", "number"],
          "document": {
            "title": "Cache sizes",
            "summary": "Short version: the size limit of the page cache.",
            "bytes": 87,
            "lines": 5
          }
        }
      ]
    }
  ],
  "skipped": [],
  "page": {"offset": 0, "shown": 1, "total": 1}
}
```

### `document_read`

One document at a revision.

| Argument | Meaning |
|---|---|
| `path` | The document, from the repository root. Required. |
| `revision` | A branch, a tag, a commit or an expression such as `main~2`, or `worktree` for the file on disk now, committed or not. The default is `worktree`, or `HEAD` in a bare repository. |
| `section` | A heading's anchor, such as `decision`. Only that section is read: the heading and everything after it up to the next heading of its level or a higher one. A leading `#` is allowed, and an anchor in another case finds its heading. The anchor of a heading inside a quote or a list reads the section around it. |
| `offset` | The byte of the document to read on from, as a cut answer's `cut.next` gives it. |
| `outline` | Whether to list the outline. The default is `true`, and `false` when `offset` is given, because the first part already listed it. |

The answer, with `schema_version` 1:

| Field | Meaning |
|---|---|
| `path`, `revision` | As asked. |
| `commit` | The commit the revision resolved to, or `worktree`. |
| `blob` | The file's blob id; left out for the working tree. |
| `type` | The document's type from the rules file at that revision, or `document`. |
| `status`, `lifecycle`, `date`, `superseded_by` | What the rules file reads from the document: its status as written, what that status means (`current`, `superseded` or `archived`), its date and what replaced it. Each is left out when the document has none. |
| `title`, `summary` | From the document's metadata (`docs/features/engine/document-model.md`); left out when it has none. |
| `bytes`, `lines` | The whole document's size. |
| `section` | The section asked for: its heading path, anchor, block index and size in bytes. |
| `outline` | The sections in the text read: each one's heading path, anchor, the block index of its heading, the line it starts on and its size in bytes. A heading inside a quote, a list, an alert or a footnote opens no section. |
| `text_range` | Where the text sits in the document: its first byte (`start`), the byte after it (`end`), and its first and last line. |
| `text` | The text, exactly as written. |
| `cut` | When the text or the outline was cut. |

A file that cannot be parsed, such as one that is not UTF-8, is refused, and so is a revision that names no commit or a document missing there.

Call `document_read` with:

```json
{"path": "docs/guide.md"}
```

It answers:

```json
{
  "schema_version": 1,
  "path": "docs/guide.md",
  "revision": "worktree",
  "commit": "worktree",
  "type": "document",
  "title": "Cache guide",
  "summary": "Short version: how to turn the page cache on and size it.",
  "bytes": 226,
  "lines": 15,
  "outline": [
    {"path": ["Cache guide"], "anchor": "cache-guide", "block": 0, "line": 1, "bytes": 225},
    {"path": ["Cache guide", "Install"], "anchor": "install", "block": 2, "line": 5, "bytes": 52},
    {
      "path": ["Cache guide", "Configure"],
      "anchor": "configure",
      "block": 4,
      "line": 9,
      "bytes": 95
    }
  ],
  "text_range": {"start": 0, "end": 226, "first_line": 1, "last_line": 15},
  "text": "# Cache guide\n\n> Short version: how to turn the page cache on and size it.\n\n## Install\n\nRun `cache install` once on each server.\n\n## Configure\n\nSet `cache.size` in megabytes. The default keeps 500 pages.\n\n```\ncache status\n```\n"
}
```

One section, at the commit of `main`:

Call `document_read` with:

```json
{"path": "docs/guide.md", "section": "configure", "revision": "main"}
```

It answers:

```json
{
  "schema_version": 1,
  "path": "docs/guide.md",
  "revision": "main",
  "commit": "562c4b3e7a8338f5087597effad34dd9d81371c0",
  "blob": "2773231d91e63e6b4e78b2a7386befb879d8d56a",
  "type": "document",
  "title": "Cache guide",
  "summary": "Short version: how to turn the page cache on and size it.",
  "bytes": 204,
  "lines": 11,
  "section": {"path": ["Cache guide", "Configure"], "anchor": "configure", "block": 4, "bytes": 73},
  "outline": [
    {
      "path": ["Cache guide", "Configure"],
      "anchor": "configure",
      "block": 4,
      "line": 9,
      "bytes": 73
    }
  ],
  "text_range": {"start": 130, "end": 203, "first_line": 9, "last_line": 11},
  "text": "## Configure\n\nSet `cache.size` in megabytes. The default keeps 500 pages."
}
```

### `search`

The search of `/api/search` (`docs/features/commands/serve.md` and `docs/features/engine/search.md`), with the same query syntax, ranking and filters.

| Argument | Meaning |
|---|---|
| `query` | Words and `"phrases"`; a document must hold all of them. The last word also finds longer words it starts. At most 32 words. Required. |
| `revision` | As for `document_read`. The default is `worktree`. |
| `path` | Keep only documents whose path starts with this text, such as `docs/decisions/`. |
| `types` | Keep only documents of these types from the rules file, such as `["decision"]`. |
| `offset` | How many results to skip. The default is 0. |
| `limit` | The most results to list, from 1 to 50. The default is 10. |

The answer, with `schema_version` 1, holds `query`, `revision`, `commit`, the number of documents searched (`documents`), the number that matched (`total`), the `results`, the files that could not be searched (`skipped`, at most 20), `page` and, when cut, `cut`. Each result has the document's `path`, `title` and `type`, the index of the block that matches best (`block`), the `headings` above it, the `anchor` of its section, for `document_read`'s `section`, and a `snippet`.

Two things differ from `/api/search`. The snippet is one text with each match between `**` marks, which costs an agent fewer tokens than a list of pieces. And there is no `url`, because an agent reads a document with `document_read`, not a page.

The working tree's index is built on the first search of `worktree`. Each later search lists the working tree with the git reader, which hashes every Markdown file, and builds the index again only when a file or the rules file changed. A commit's index is the server's: built on first use and kept in the cache under the commit's tree.

Call `search` with:

```json
{"query": "500 pages"}
```

It answers:

```json
{
  "schema_version": 1,
  "query": "500 pages",
  "revision": "worktree",
  "commit": "worktree",
  "documents": 3,
  "total": 2,
  "results": [
    {
      "path": "docs/guide.md",
      "title": "Cache guide",
      "type": "document",
      "block": 5,
      "headings": ["Cache guide", "Configure"],
      "anchor": "configure",
      "snippet": "Set cache.size in megabytes. The default keeps **500** **pages**."
    },
    {
      "path": "docs/notes/cache-sizes.md",
      "title": "Cache sizes",
      "type": "document",
      "block": 2,
      "headings": ["Cache sizes"],
      "anchor": "cache-sizes",
      "snippet": "It holds **500** **pages**."
    }
  ],
  "skipped": [],
  "page": {"offset": 0, "shown": 2, "total": 2}
}
```

### `lint`

The writing check of `driftmark lint` (`docs/features/lint.md`), with the rules file's `[lint]` table.

| Argument | Meaning |
|---|---|
| `paths` | Markdown files and folders, from the repository root, such as `["docs/guide.md", "docs/decisions"]`. A folder means every Markdown file git lists in it, except those `exclude` names; a file named on its own is always checked. `.` means the whole repository. |
| `offset` | How many findings to skip. The default is 0. |
| `limit` | The most findings to list, from 1 to 500. The default is 50, as many as the Claude Code hook reports. |

Without `paths`, it checks the Markdown files changed in the working tree since the last commit: edited, staged and new files that git does not ignore, except those `exclude` names. The answer then says `"changed": true`.

The answer, with `schema_version` 1, holds the number of files checked (`files`), the counts of `errors` and `warnings`, the `findings` and `page`, and `cut` when cut. Each finding has its `path`, `line`, `column`, `severity`, `rule` and `message`, as `driftmark lint --json` writes them.

Only files git lists in the working tree are checked, so a symbolic link is never read. That differs from `driftmark lint`, which checks a file named on the command line even when git ignores it.

Call `lint` with:

```json
{}
```

It answers:

```json
{
  "schema_version": 1,
  "files": 1,
  "errors": 0,
  "warnings": 1,
  "findings": [
    {
      "path": "docs/guide.md",
      "line": 13,
      "column": 1,
      "severity": "warning",
      "rule": "code-language",
      "message": "this code block names no language; write it after the opening fence, such as ```sh, or ```text for plain text"
    }
  ],
  "page": {"offset": 0, "shown": 1, "total": 1},
  "changed": true
}
```

## Size limits

Claude Code shows a warning when one tool answer passes 10,000 tokens. It saves a text answer longer than 50,000 characters to a file, and the agent sees only the file's name (code.claude.com/docs/en/mcp, "MCP output limits and warnings", read on 2026-10-18). So every answer has a limit.

- **The limit** is 16,000 characters. The answers measured 2.0 to 2.7 characters a token with Claude's tokenizer (see below), so that is about 6,000 to 8,000 tokens, under the warning. A call may ask for another limit with `max_chars`, from 2,000 to 40,000. Characters are counted as Claude Code counts them, in UTF-16 units, so an emoji outside the basic plane counts two.
- **Lists are cut.** `changes_summary`, `changes_document`, `search` and `lint` list as many items as fit, from `offset` on, and at most `limit`. Counts such as `totals`, `total` and `errors` always cover the whole list.
- **Text is cut after a whole line.** `document_read` keeps the outline to at most 40% of the limit, leaving out the deepest headings first, and gives the rest to the text. The text ends after the last whole line that fits. Only a single line longer than the room left is cut inside, at a character.
- **One change too large to show** is shown without its texts: its blocks' `text`, its `edit` and a whole document's `source` are left out, and its kind, level, signals and places stay. The agent reads the lines with `document_read`.
- **An answer that does not fit even so**, such as one whose range notes alone are longer than a small `max_chars`, is refused, with the size it needs.

A cut answer has a `cut` object. `reason` says what was left out, in a sentence. `next`, when the rest can be asked for, holds the arguments of the call that continues: the same arguments, with `offset` moved on. For `document_read` it also sets `outline` to `false`. A cut `changes_summary`:

Call `changes_summary` with:

```json
{"range": "main...feature", "limit": 1}
```

It answers:

```json
{
  "schema_version": 2,
  "range": {
    "base": "562c4b3e7a8338f5087597effad34dd9d81371c0",
    "head": "59f1a26ed292405ee4ef67182b681d68723d2726",
    "notes": []
  },
  "totals": {
    "changes": 6,
    "highest": "decision",
    "levels": {"decision": 3, "rule": 0, "fact": 2, "wording": 1, "formatting": 0},
    "kinds": {"formatting": 0, "edited": 2, "added": 2, "removed": 2, "moved": 0, "moved_edited": 0}
  },
  "documents": [
    {
      "path": "docs/decisions/0002-cache-rendered-pages.md",
      "file": "modified",
      "summary": {
        "changes": 5,
        "highest": "decision",
        "levels": {"decision": 3, "rule": 0, "fact": 1, "wording": 1, "formatting": 0},
        "kinds": {
          "formatting": 0,
          "edited": 2,
          "added": 1,
          "removed": 2,
          "moved": 0,
          "moved_edited": 0
        }
      }
    }
  ],
  "skipped": [],
  "page": {"offset": 0, "shown": 1, "total": 2},
  "cut": {
    "reason": "1 of 2 documents are listed, from number 0; ask with `offset` 1 for the rest",
    "next": {"range": "main...feature", "limit": 1, "offset": 1}
  }
}
```

### Typical answers on this repository

Measured on 2026-10-18 on driftmark's own repository: commit `258d174` with this document added and not committed yet, so the range `auto` holds this one document. Every call used the default limits. Tokens were counted with Claude's own tokenizer (Opus 5.5): the input tokens `claude -p` reports with the answer in the prompt, minus those without it.

| Call | What the answer holds | Characters | Tokens | Characters a token |
|---|---|---|---|---|
| `changes_summary`, `auto` | 1 document | 650 | 323 | 2.0 |
| `changes_summary`, `since/2026-10-01/main` | 25 of the 325 documents of the whole history; cut | 7,470 | 3,642 | 2.1 |
| `changes_document`, `serve.md` over its last 3 commits | 16 of 79 changes; cut at the limit | 15,899 | 6,038 | 2.6 |
| `changes_document`, this new document | 1 change, its 38 KB text left out; cut | 1,726 | 667 | 2.6 |
| `document_read`, decision 0009 | the whole document, 2.6 KB | 3,732 | 1,363 | 2.7 |
| `document_read`, `serve.md` | the outline and lines 1 to 195 of 537; cut | 15,804 | 6,223 | 2.5 |
| `document_read`, `serve.md`, section `search` | one section | 6,263 | 2,353 | 2.7 |
| `search`, `cache` | 10 of 36 results; cut | 2,911 | 1,232 | 2.4 |
| `lint`, changed files | 2 findings in 1 file | 552 | 217 | 2.5 |
| `lint`, `docs` | 4 findings in 26 files | 987 | 376 | 2.6 |

- The JSON runs at 2.0 to 2.7 characters a token, fewer than English prose. Counts and keys cost the most: a summary entry is about 140 tokens.
- No answer passed 6,300 tokens, so none reached Claude Code's warning at 10,000.
- The raw answers and counts are kept outside git, in `private/m3/mcp/answers/`.

## Installing it

`driftmark mcp install` prints the entry it would add to the repository's `.mcp.json`, and the `claude mcp add` command that adds the same entry. It changes nothing:

```text
driftmark mcp install would add this server to .mcp.json:

{
  "mcpServers": {
    "driftmark": {
      "type": "stdio",
      "command": "driftmark",
      "args": [
        "mcp"
      ]
    }
  }
}

Run `driftmark mcp install --write` to add it, or add the same with Claude Code:

  claude mcp add --transport stdio --scope project driftmark -- driftmark mcp

Claude Code asks once whether to start the servers in .mcp.json.
```

`driftmark mcp install --write` adds it.

- **The file** is `.mcp.json` at the repository root, the file Claude Code reads a project's servers from. It works from any folder inside the repository. The file is made when missing.
- **Other servers and keys stay**, in their order. The file is written back with its own indentation and line endings, and with a final line ending only if it had one, as `driftmark hook install` writes `.claude/settings.json` (`docs/features/lint.md`). The new file is written beside the old one and then moved over it.
- **Running it twice adds nothing.** The second run says ``.mcp.json already starts `driftmark mcp` as the server `driftmark`; nothing to add``. Any server whose command is `driftmark`, by any path, with `mcp` as its first argument counts.
- **It refuses**, with exit code 1, leaving the file as it was, when the file is not valid JSON, its top level or its `mcpServers` is not an object, or another server is already named `driftmark`. A dry run refuses the same files.

Claude Code asks once, in an interactive session, before it starts the servers of a project's `.mcp.json`. `claude -p` runs start them without asking. `driftmark` must be on the `PATH` that Claude Code sees. On Windows, an npm install puts a `driftmark.cmd` there, which a client may need to start as `cmd /c driftmark mcp`.

### Other agents

Read on 2026-10-18 from each tool's documentation:

- **Codex** keeps MCP servers in `~/.codex/config.toml`, or in a trusted project's `.codex/config.toml`, as `[mcp_servers.driftmark]` with `command = "driftmark"` and `args = ["mcp"]`. `codex mcp add driftmark -- driftmark mcp` writes the same. A `cwd` key sets the folder the server starts in; without it, give `--repo`. Codex waits 10 seconds for a server to start and 60 seconds for a tool, and lets a tool's answer be limited with `output_token_limit`.
- **Cursor** reads `.cursor/mcp.json` in the project, or `~/.cursor/mcp.json`, with the same `mcpServers` shape and `"type": "stdio"`. Its documentation does not say which folder a server starts in, so the entry should pass `"args": ["mcp", "--repo", "${workspaceFolder}"]`. Cursor asks before it uses an MCP tool.

`driftmark mcp install` writes only Claude Code's file.

## Safety

- **It reads only through git and the repository's files.** Every document is read through the same git reader as `driftmark serve`: from the commit's tree, or, for the working tree, among the files git tracks or lists as not ignored, reached through real folders only. A symbolic link is never followed, so a link inside the repository cannot make the connector read a file outside it.
- **No argument is a raw path outside the repository.** A path runs from the repository root, and must be a path git could store: no empty, `.` or `..` part. A path written from the root of the file system is accepted only when it lies inside the working tree, because agents often hold such paths; it is then read as the path from the repository root. Any other path is refused.
- **The writing check reads no heading through a link.** Its link check sees file names in real folders. It counts a link that leads through a linked folder as there, without looking, and reads the headings of a linked file only when it is a regular file reached through real folders.
- **It never writes a document**, and writes nothing at all to the repository. `driftmark mcp install --write` writes `.mcp.json`, and only when a person runs it.
- **It runs no program but git.** gix reads the repository in the process; the `git` command is the fallback of `docs/features/engine/git-ranges.md`. The connector draws no diagram, so it never starts `d2`.
- **It reaches nothing beyond the repository**: no network, and no file outside it.
- **Standard output carries only protocol messages.** Messages for people, such as the git reader saying it switched to the `git` command, go to standard error, which Claude Code keeps in its log.

## Errors

A tool that cannot answer gives back a sentence with `isError: true`, so the agent sees why and can try again. The sentence names the fix where there is one. Examples:

| Cause | Sentence |
|---|---|
| An unknown argument | `` `search` takes no argument `q`; its arguments are `query`, `revision`, … `` |
| A missing argument | `` `changes_document` needs the argument `path` `` |
| A wrong value | `` `limit` must be a whole number from 1 to 500`` |
| A range that is not one | `` `main` is not a range; write `<base>..<head>`, `<base>...<head>`, `auto` or `since/<date>/<branch>` `` |
| A name that is no commit | `` `nope` does not name a commit`` |
| A path outside the repository | `` `/etc/hosts.md` is outside the repository; give a path from the repository root, such as docs/guide.md`` |
| A document that did not change | `` `docs/guide.md` did not change in this range`` |

Call `document_read` with:

```json
{"path": "docs/guide.md", "section": "usage"}
```

It refuses:

```text
`docs/guide.md` has no heading with the anchor `usage`; its sections' anchors are: cache-guide, install, configure
```

A tool name that does not exist is a protocol error, JSON-RPC code `-32602`: ``driftmark has no tool `nope` ``.

## Notes, in wave 2

The note tools come once the notes store lands: `notes_list`, `note_get`, `review_get`, `note_reply`, `note_propose_decision` and `note_mark_applied`. Each is one more entry in the tool list of `crates/driftmark/src/mcp/tools.rs`, and the notes store joins the tools' shared context. The tools that write, such as `note_reply`, say so to the client: they are listed without `readOnlyHint`.

## Not covered

- **The notes loop** (wave 2, above).
- **Other transports.** Only standard input and output; no HTTP.
- **Resources and prompts.** The connector offers tools only.
- **Paths that are not UTF-8.** A change list and the writing check name files as text, so such a file can be read but not always named back.
