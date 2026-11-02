---
status: target
date: 2026-11-02
---

# The status page

> Short version: the status page answers "which documents are still current?". `/lifecycle/<revision>` shows every document of a revision by type: a table counts each type's documents as current, superseded, archived or with no lifecycle, and a list shows each document with its status, its date and how many days ago a commit last changed it. A superseded document is folded under the one that replaced it, and the archive is folded. A document no commit changed for 180 days, or the rules file's `untouched-days`, is called untouched: a fact shown beside it, never ranked as a problem. The same view comes as JSON from `/api/lifecycle/<revision>` and to agents from the MCP tool `documents_status`. When each document last changed comes from one walk back along the branch's first parents, not one git call per file.

## What the page answers

Decision 0001 makes the status view driftmark's second job, after "what changed": users of GitHub Spec Kit (discussion 152) and OpenSpec (issue 1525) asked to see which spec is current. For one revision, the page shows, by document type:

- what is current;
- what was superseded, and by what;
- what is archived;
- what has not been touched for a long time.

A revision is anything `/doc/<revision>/-/<path>` takes (`docs/features/commands/serve.md`): a branch, a tag, a commit, an expression such as `main~2`, or `worktree` for the files on disk.

## A document's lifecycle

Each Markdown file is read and classified with the rules of its revision: the repository's `driftmark.toml`, or the presets found there (`docs/features/engine/rules-file.md`, `docs/features/presets.md`). Its type, status, date and "superseded by" come from the rules. Its lifecycle is one of four:

| Lifecycle | When |
|---|---|
| current | Its status is in the type's `current` list, or its type's default `lifecycle` is current. |
| superseded | Its status is in the type's `superseded` list, or its type's default is superseded. |
| archived | Its status is in the type's `archived` list, or its type's default is archived. |
| none | Neither its status nor its type says. A file no type matches, of the type `document`, always has none. |

## When a document last changed

Each document carries the last commit that changed it and that commit's committer date. driftmark finds them in one walk back along the revision's history, through the git reader's question `last_changes` (`docs/features/engine/git-ranges.md`). The rule:

- **Only first parents are followed.** A merge counts as changing what it brought in. So a file edited on a branch counts as changed when the branch was merged: when the change reached this branch.
- **A commit changes a file when the file's bytes differ from those in its first parent.** A new mode alone, such as a file made executable, is no change.
- **The first commit changes every file it holds.** So does the oldest commit of a shallow clone, the furthest back git can see there.
- **The walk stops as soon as every file of the revision is found.** On a repository where every file changed last month, it reads last month only.

The working tree has no commit of its own. A working-tree file whose bytes git calls unchanged since `HEAD` has `HEAD`'s last change. Any other file, new or edited, is "not committed yet": someone is working on it now, so it is never untouched.

The view reads no clock and no git itself. The program gives it the time now and the commit times, and the view counts whole days: `(now − commit time) / 86,400`, rounded down, and never below 0.

## Untouched for long

A document whose last commit is at least `untouched-days` old is untouched. The day count comes from the rules file:

```toml
[status-view]
untouched-days = 90
```

- Without it, the count is 180: long enough that a document changed with the work of one season is not called untouched.
- It is a whole number from 1 to 36,500. Any other value is a `Syntax` error with its line: ``the rules file could not be read: … `untouched-days` is 0; write a whole number of days from 1 to 36500, such as 180``.
- Like `[lint]` and `[compare]`, the table belongs to the repository's own file. A preset never sets it.

Untouched is a fact, not a fault. A decision record that nobody changed for a year may be exactly right. So the page shows it as plain words beside the date, in the text colour, and nothing is sorted, counted first or marked as a problem because of it. The counts table has a column for it, and the filters can show only untouched documents.

## What superseded a document

A document names its successor in its "superseded by" value, which the rules read from front matter or a masthead. A superseded document with no such value may name it in its status instead, after the word "by", as adr-tools writes `Superceded by [3. Store files on disk](0003-store-files-on-disk.md)` under `## Status`. The value is tried four ways, and the first that leads to exactly one document at the same revision wins:

1. **Its links**, from a masthead value, resolved from the document's folder, or from the repository root when they start with `/`. `#` places, `?` queries and `%XX` escapes are handled as a browser would; links to other sites are skipped.
2. **The text as a file name**, when it ends in `.md` or `.markdown` and holds no space, such as `superseded-by: 0005-split-the-database.md`: from the document's folder, then from the repository root.
3. **A title**: a document whose title is the text, ignoring case and runs of spaces. adr-tools' link text is the newer record's title.
4. **A number**: the first number in the text, as in `Superseded by 0009` or `ADR-12`, and the one file in the same folder whose name starts with that number. Leading zeros and an `adr-` prefix, in any case, do not count, so `9`, `0009` and `ADR-9` all find `0009-new.md`.

What it gives:

| Result | JSON `state` | The page says |
|---|---|---|
| A document | `found` | "Superseded by" and a link to it. |
| A document, but following "superseded by" from it leads back here | `circle` | The link, and "but following “superseded by” from there leads back here". |
| A link or a file name that names a path no document has | `missing` | "Superseded by “…”: `docs/decisions/0009-gone.md` is not a document at this revision". |
| A text that leads nowhere | `unmatched` | "Superseded by “…”: no document at this revision has that path, title or number". |
| No text at all, only a superseded status | `unnamed` | "Superseded, but the document does not say by what". |

A link or a file name that leads nowhere is reported as missing only when the title and the number lead nowhere either.

**Folding.** A superseded document whose successor was found is folded under it on the page, in "Replaced 2 documents". So a chain shows once: A by B and B by C puts B under C and A under B. A document that is not superseded but still names a successor, such as a current one with a `superseded-by` field, gets the link and is not folded. Two or more documents that name each other in a circle are not folded under each other; each says so.

## The page

Address: `/lifecycle/<revision>`, with the filters as query parameters. The address does not use "status" because `/api/status` is already the server's own state (`docs/features/commands/serve.md`).

**How a reader gets there.**

- The inbox has a link under its title: "Which documents are current on `origin/main` →", to the default branch's page. Without a default branch it leads to the working tree, or to `HEAD` in a bare repository.
- On every reading page, the type, status and lifecycle pills are links. The type pill shows that type on the same revision, `/lifecycle/<revision>?type=decision`; the status and lifecycle pills add the document's lifecycle, `&lifecycle=current`.

**What it shows, top to bottom.**

1. The revision, its commit and the number of documents, with a link to the JSON. Then the title, "Which documents are current", and one sentence saying what "untouched" means here.
2. **The counts**: one row per type that has documents, in the rules file's order, then other types by name, then "No type" for the documents no type matches, and a last row, "Every type". The columns are Current, Superseded, Archived, No lifecycle, "Untouched 180+ days" and All. Every count above 0 is a link that shows those documents.
3. **The filters**, a form that needs no script: Type, Lifecycle, Folder (a path prefix, such as `docs/decisions/`), "Untouched 180+ days", "Show" and "Clear".
4. **How many are shown**: "All of the 412 documents these filters list, folded ones included.", or "Showing 1000 of the 3724 …" when the list is longer than one page.
5. **One section per type**, with its name and its number of documents, or, under a filter, how many of them it keeps: "2 of 19 documents". Its documents are listed by path. Archived ones come last, folded into "Archived · 21". When typed documents are listed too, the "No type" section is folded whole into "Documents no type of the rules file matches", because a repository with presets may hold thousands of plain notes.
6. Links to the next documents and back to the first, when the list is longer than one page.
7. The files that could not be read, folded, each with its reason.

**One row** shows:

- the title, linking to its reading page at the same revision, or the file name when it has no title;
- its lifecycle as a word on a framed badge: Current, Superseded or Archived. A document with no lifecycle shows no word, because on a repository without lifecycles every row would repeat it;
- its path, whose folder part is a link that keeps the filters and adds that folder;
- its status and date as written, and "last changed 40 days ago", with the exact time as its tooltip, or "not committed yet". An untouched document adds the word "untouched", in italics;
- what superseded it, as above;
- "Replaced 2 documents", folded, with each document it replaced as a row of its own, and their predecessors folded inside them.

**Which rows a filter shows.** A document that matches every filter is a row of its own type, unless a newer document it is folded under matches too: then it is shown folded under that one. So with no filter every document shows once; with "superseded" a chain shows from its newest superseded document down; with "current" each current document still carries its folded history.

**Pages.** One page shows at most 1,000 documents, folded ones included. A row and everything folded under it never splits across pages. `offset=1000` shows the next ones. A row takes about 500 bytes of HTML and 30 after compression, so a full page stays far under the 100 KB first-visit limit (`docs/features/pages.md`).

**The working tree's page** reloads itself when a file changes, as the other working-tree pages do.

The page follows the review-first design of decision 0010 and the readability rules of `docs/features/pages.md`: 17 px body text, words as well as colours for every lifecycle, a header row that stays in view, and on a phone the counts table becomes one card per type, each count carrying its column's name. axe-core finds no problem on it in light or dark, at 1440 px or 390 px.

### Filters

| Parameter | Meaning |
|---|---|
| `type` | Keep only documents of this type. It may be given several times, to keep several types. `document` is the files no type matches. |
| `lifecycle` | `current`, `superseded`, `archived` or `none`, in any case. Empty means every lifecycle. |
| `folder` | Keep only documents whose path starts with this text, as written: `docs/decisions/` keeps that folder, and `docs/d` also keeps `docs/drafts/`. |
| `untouched` | `yes`, `1`, `true` or `on` keeps only untouched documents; `no`, `0`, `false` or empty keeps all. |
| `offset` | The page only: how many rows to skip. |

The parameters are form-encoded, as a browser writes them. Other parameters are ignored. A `lifecycle`, `untouched` or `offset` that means none of these, or a parameter that is not valid text, answers `400`, with a sentence that names the fix, such as ``` `lifecycle` is "draft"; write current, superseded, archived or none ```. A revision that names no commit answers `404`, and so does `worktree` in a bare repository.

## The JSON

`GET /api/lifecycle/<revision>` takes the same filters, except `offset`, and lists every document they keep, by path, with no folding: `replaces` says what a page would fold. For a made-up repository whose rules file makes `docs/decisions/*.md` decision records, at `main`, with the filter `?type=decision&lifecycle=superseded`:

```json
{
  "schema_version": 1,
  "revision": "main",
  "commit": "67e144003ae9156fa9a2f4e55fa51b512a938c53",
  "as_of": 1793630000,
  "untouched_days": 180,
  "filter": {"types": ["decision"], "lifecycle": "superseded", "folder": null, "untouched": false},
  "totals": {"documents": 4, "current": 1, "superseded": 1, "archived": 1, "none": 1, "untouched": 0},
  "types": [
    {"type": "decision", "documents": 3, "current": 1, "superseded": 1, "archived": 1, "none": 0, "untouched": 0},
    {"type": "document", "documents": 1, "current": 0, "superseded": 0, "archived": 0, "none": 1, "untouched": 0}
  ],
  "documents": [
    {
      "path": "docs/decisions/0001-keep-one-database.md",
      "title": "Keep one database",
      "type": "decision",
      "status": "superseded",
      "lifecycle": "superseded",
      "date": "2026-02-10",
      "superseded_by": {
        "text": "0002-split-the-database.md",
        "path": "docs/decisions/0002-split-the-database.md",
        "state": "found"
      },
      "last_change": {"commit": "6a4b0aaa6279cbc66f326430580836762ac76ea5", "committed_at": 1790000000, "days": 42},
      "untouched": false
    }
  ],
  "skipped": []
}
```

| Field | Meaning |
|---|---|
| `schema_version` | 1, the version of the server's own JSON. |
| `revision`, `commit` | The revision as asked, and the commit it resolved to, or `worktree`. |
| `as_of` | The time the ages were counted at, in seconds since 1970 UTC. |
| `untouched_days` | The day count in use. |
| `filter` | The filter that chose `documents`. |
| `totals`, `types` | Every document, and each type's, counted by lifecycle, whatever the filter. Types come in the page's order. |
| `documents` | The documents the filter keeps, sorted by path. |
| `skipped` | The files that could not be read, each with its `path` and the `reason`, such as a file that is not UTF-8. |

Each document:

| Field | Meaning |
|---|---|
| `path`, `title`, `type`, `status`, `date` | As the rules read them; `title`, `status` and `date` are left out when the document has none. |
| `lifecycle` | `current`, `superseded`, `archived` or `none`. |
| `superseded_by` | Left out when the document names no successor and is not superseded. `text` is the value as written, or `null`. `path` is the document it leads to for `found` and `circle`, the path no document has for `missing`, and `null` otherwise. `state` is one of the five results above, and `problem`, for every state but `found`, says why in words. |
| `replaces` | The superseded documents whose successor this is, by path. Left out when empty. |
| `last_change` | The last commit that changed it, its committer date and the whole days since; `null` when it is not committed yet. |
| `untouched` | Whether `days` is at least `untouched_days`. |

On the private corpus the JSON of every document is 1.1 MB. An agent asks the MCP tool instead, which pages and cuts its answer.

## The MCP tool

`documents_status` answers from the same view with the same filters: `revision`, `types`, `lifecycle`, `path` (the folder prefix), `untouched`, then `offset` and `limit` for paging, within the connector's size limits. `docs/features/commands/mcp.md` has its arguments and an example.

## The model in the core

The rules live in `driftmark_core::status`; the core reads no files, no git and no clock.

- `Entry::new(path, title, classification)` keeps what the view needs of one document.
- `Touched` is `Committed { commit, at }` or `Uncommitted`.
- `View::build(entries, &rules, now)` sorts the documents by path, finds each successor, breaks circles, records what each replaced, counts the types and lifecycles, and works out each age. `rules.untouched_days()` gives the day count.
- `view.matches(index, &filter)`, `view.listed(&filter)` and `view.groups(&filter)` choose documents and rows as above. `Stage` names a lifecycle as a filter asks for it, `none` included.
- `status::needed(source, rule)` gives the part of a document the view needs: its head, up to the first `## ` heading after its title, when its type reads nothing from a later section. It returns the whole source when the type reads a section, such as adr-tools' `## Status`; when the file is too large or not UTF-8, so the parse fails as it does on a reading page; when no `# ` title line and blank line come before a `## ` line; when front matter is not closed or a code fence is open at the cut; when a line of the head starts with `<`, which may open an HTML block that runs past the cut; and when a line after the cut starts with `[` and holds `]:`, which may define a link or footnote the head uses. On the private corpus, 3,314 of 3,724 documents are cut to their head, which leaves 6.7 MB of 60.5 MB to parse, and each reads exactly as it does whole.
- `driftmark_core::git::LastChanges` holds the walk's rule, and `walk_last_changes` walks with the reader's other questions.

## How the program reads and keeps it

- `driftmark::status::entries` reads, cuts, parses and classifies every Markdown file of a revision on all cores, in runs of 16 files that the threads take one after another, so a run of large documents does not hold one thread up.
- The git reader answers `last_changes`. gix reads a window of 256 commits along first parents, then compares each with its parent on up to 16 threads. A comparison looks only in the folders that still hold a file being looked for, and opens a folder only when its tree changed. The `git` command answers with one `git log --first-parent -m --root --raw` that it stops once every file is found (`docs/features/engine/git-ranges.md`).
- For a commit, the server reads the documents and walks the history at the same time, and keeps both in its cache under the commit's id, the documents with the rules' version too (kind `status` in `/api/status`). Only the first request reads them; each later request works out the ages again from the time now.
- The working tree's files that git calls unchanged since `HEAD` take `HEAD`'s entries, when both sides read the same rules; the others are read from the disk. The server keeps the working tree's documents while its watcher has announced no change, `HEAD` stays and the rules stay the same. Listing the working tree hashes every file, which costs more than the rest of the view. Rules that read a value from a file beside a document, which the watcher does not watch, are read again each time. The MCP connector, which has no watcher, reads the working tree each time.
- After a request that read many documents, the server gives memory back as after any bulk job (decision 0011).

## Speed on the private corpus

Measured on 2026-11-02 with the release build serving the maintainer's private corpus (a company repository of 3,724 Markdown files and 3,254 first-parent commits) on `127.0.0.1`, with `--base` set to its working branch and the ADR preset found by its markers. "Cold" is the first request to a fresh server: once right after the greeting, while the watcher's own startup count runs, and once after it ended. "Warm" is the median of 7 requests after it. Each figure is the median of 3 servers, with the range in brackets; the load average was 4 to 5 on 12 cores.

| Request | Cold, at start | Cold, after the startup count | Warm |
|---|---|---|---|
| `/lifecycle/<working branch>`, every document | 223 ms (198 to 231) | 168 ms (163 to 181) | 12 ms (10 to 12) |
| `/api/lifecycle/<working branch>`, 1.1 MB | 196 ms (181 to 199) | | 9 ms (7 to 10) |
| `/lifecycle/worktree`, after the page above | 401 ms (395 to 405) | 399 ms (392 to 411) | 7 ms (7 to 7) |

- **The history walk** finds the last change of all 3,724 files in 104 to 136 ms with gix on 12 threads, using 300 to 350 ms of CPU. It has to walk 3,251 of the 3,254 commits, because the oldest files never changed again. One `git log` takes 284 to 357 ms. Before the folders were pruned and the comparisons spread over threads, gix took 275 to 327 ms.
- **Reading and classifying** every document takes 36 to 38 ms on 12 threads, 310 to 350 ms of CPU. Parsing whole documents took about 1,300 ms of CPU.
- **The working tree's first request** costs more because the git reader hashes every working-tree file to list them, as the watcher's own count does. Later requests skip that until a file changes.
- **The page** for every document is 591 KB of HTML and 54.6 KB after gzip: 1,000 rows on the first page.
- Earlier the same day, at a load of 25 to 27, the cold page took 193 to 231 ms and the warm one 9 to 11 ms.

The counts, with the ADR preset found by its markers and no rules file:

| Type | Documents | Current | Superseded | Archived | No lifecycle | Untouched 180+ days |
|---|---|---|---|---|---|---|
| adr | 295 | 226 | 18 | 21 | 30 | 0 |
| document (no type) | 3,429 | 0 | 0 | 0 | 3,429 | 57 |

All 18 superseded records name a successor driftmark finds, so all 18 fold under it. No file was skipped. Every number here is about documents, not their content: no path, title or text of the corpus is recorded.

## Where it is tested

- `fixtures/status/<behaviour>/` holds a made-up repository per behaviour: `lifecycles/` (each lifecycle, read from front matter, a masthead and a section, and a type's default), `superseded-chain/` (A by B by file name, B by C by title, and D by C by number), `broken-superseded-by/` (a missing file, a missing link target, a text that names nothing, a superseded status with no successor, and two records that name each other) and `untouched/` (89 and 90 days with `untouched-days = 90`, 400 and 500 days, and a file not committed yet). Its `touched.toml` gives each file's age in days, and `expected.txt` the view written out.
- `crates/driftmark-core/tests/status_fixtures.rs` builds the view of each tree and compares it with `expected.txt`. `crates/driftmark/tests/status.rs` commits each tree to git, every file on the day its `touched.toml` gives, and checks the server's JSON for the working tree and for `main` against the same file. It also checks the page, the filters, paging, the links from the inbox and the pills, a working-tree edit, a rules file changed on disk, a repository without commits and the refusals.
- `crates/driftmark/tests/readers.rs` checks that gix, the `git` command, the chooser and the core's own walk find the same last changes on a history with branches, a merge, a mode-only change and a moved file, and in a shallow clone.
- `crates/driftmark/tests/status_corpus.rs`, ignored in normal runs, times the walk and the reading on a real repository and checks that every head reads as its whole document.

## Not covered

- **A status kept outside the Markdown file** is shown, but the watcher does not watch it, so the working tree's page reads it again on every request.
- **A file broken only past its head**, such as one nested more than 100 levels deep further down, is listed with the status its head gives, while its reading page refuses it.
- **Renamed files** count as new: the walk does not follow renames, as `driftmark diff` does not.
- **Several repositories**, and status across branches at once: the page shows one revision.
- **The working tree's first request** hashes every file to list it, so on a 3,700-file repository it takes about 0.4 seconds.
