---
status: target
date: 2026-10-18
---

# The serve command

> Short version: `driftmark serve` is a small web server on the reader's own machine. It reads git when a page is opened, so a branch shows its latest commit without a rebuild. The home page lists the branches whose documents differ from the default branch. Other pages show a document at any revision, a range's change list, and one document's changes, each also as JSON. `/api/search` searches every document at a revision. It keeps parsed documents and finished pages in a cache keyed by blob id, capped at 128 MB. It watches the working tree, so open pages reload when a file changes. It listens on `127.0.0.1` by default; any other address needs a token, which a browser presents once and then keeps in a cookie.

## Running it

```text
driftmark serve [--repo <path>] [--addr <ip:port>] [--token <value>] [--base <branch>]
```

| Option | Meaning |
|---|---|
| `--repo <path>` | A folder inside the repository. The default is the current folder. A bare repository works too; it has no working tree. |
| `--addr <ip:port>` | Where to listen. The default is `127.0.0.1:4170`, this machine only. Port `0` picks a free port. |
| `--base <branch>` | The default branch: the one the home page and `auto` compare with. It wins over the rules file's `[compare] default-branch`. |
| `--token <value>` | The token every request must bring. It needs at least 16 characters, and only letters, digits, `-`, `_`, `.` and `~`, so it fits in an address and a cookie as it is. |
| `--help` | Print the usage text. |

When it starts, the server prints where it reads and the address to open:

```text
$ driftmark serve
driftmark serve: reading /home/ana/notes
driftmark serve: open http://127.0.0.1:4170/
driftmark serve: press Ctrl-C to stop
```

On another address, the line to open carries the token:

```text
$ driftmark serve --addr 0.0.0.0:4170
driftmark serve: reading /home/ana/notes
driftmark serve: open http://0.0.0.0:4170/?token=3f9c0d2a7b6e41c58a0f2d9e6b1c7a44
driftmark serve: write this machine's name or address in place of 0.0.0.0
driftmark serve: every request needs the token, because the address reaches beyond this machine
driftmark serve: press Ctrl-C to stop
```

Ctrl-C stops it. Open event streams end, work already started gets up to 2 seconds to finish, and the server prints `driftmark serve: stopped`.

| Exit code | When |
|---|---|
| 0 | It stopped on Ctrl-C. |
| 1 | It could not start: the folder is not inside a git repository, or the address cannot be listened on, for example because another program uses the port. The reason goes to standard error. |
| 2 | The command line is wrong. The reason and the usage text go to standard error. |

## Addresses

Every page has a short address that stays the same. It names a revision or a range, then, after the separator `/-/`, a document's path:

| Address | Page |
|---|---|
| `/` | The home page: branches with document changes. |
| `/doc/<revision>/-/<path>` | A document at a revision. |
| `/changes/<range>` | A range's change list. |
| `/changes/<range>/-/<path>` | One document's changes in a range. |

The separator is `/-/` because branch names hold `/`, as in `feature/login`. The path comes last, so a relative link in a document leads to the other document at the same revision without being rewritten: on `/doc/feature/login/-/docs/guide.md`, the link `setup.md` opens `/doc/feature/login/-/docs/setup.md`.

**A revision** is anything git resolves to a commit: a branch, a tag, a commit id or part of one, or an expression such as `main~2`. The word `worktree` means the files on disk, committed or not. Like `auto` for ranges, it always means the working tree, even when a branch has that name; write `refs/heads/worktree` for such a branch.

**A range** takes the forms of `driftmark diff` (`docs/features/engine/git-ranges.md`):

| Range | What it compares |
|---|---|
| `main..feature` | The tip of `main` with the tip of `feature`. |
| `main...feature` | The tip of `feature` with the commit where it left `main`. |
| `auto` | The working tree with the commit where this branch left the default branch. |
| `since/2026-10-01` | Everything that reached the current branch from 1 October on. |
| `since/2026-10-01/main` | The same on `main`. |

**A path** runs from the repository root, as git stores it. Only Markdown files are pages: names ending in `.md` or `.markdown`, in any case.

**Escaping.** Addresses are read before percent-decoding, so `%2F` inside a branch name never splits it, and `%2D` hides a `-` part from the separator: the branch `team/-/x` is written `team/%2D/x`. A path may hold any byte as `%XX`, so a file named in Latin-1 still has an address. The pages escape every character outside letters, digits and `-._~/@:!$'()*+,;=`.

## Routes

| Route | Answer |
|---|---|
| `GET /` | The home page, HTML. |
| `GET /api/home` | The home page, JSON. |
| `GET /doc/<revision>/-/<path>` | A document, HTML. |
| `GET /api/doc/<revision>/-/<path>` | A document, JSON. |
| `GET /changes/<range>` | A range's change list, HTML. |
| `GET /api/changes/<range>` | A range's change list, JSON: the summary form of `driftmark diff`. |
| `GET /changes/<range>/-/<path>` | One document's changes, HTML. |
| `GET /api/changes/<range>/-/<path>` | One document's changes, JSON: a change list with that one document. |
| `GET /api/search?q=<query>` | Full-text search over every document at a revision, JSON. |
| `GET /events` | The working tree's changes, as server-sent events. |
| `GET /static/<hash>/<name>` | The files the pages load, built into the program: the page script, the style sheet, the diagram font, Prism and mermaid.js (see "Static files"). |
| `GET /api/status` | The cache's counts and the watcher's state, JSON. |

Every route also answers `HEAD`. A route that exists answers another method with `405`, and any other address answers `404`.

The pages follow the review-first design of decision 0010. `docs/features/pages.md` says what a reader sees on each; this document covers the routes, the JSON and how the server makes them.

### The home page

The home page is an inbox of branches (decision 0010):

1. **The default branch** is the one `auto` uses: the branch `--base` names, else the one the rules file's `[compare] default-branch` names (`docs/features/engine/rules-file.md`), else the first that exists of `origin/main`, `main`, `origin/master`, `master`, `origin/dev`, `dev`. The rules file is the one in the working tree, or at `HEAD` in a bare repository, read on each request. Without a default branch, the page says so and compares nothing. A named branch that does not exist answers `404` with ``the default branch `dev`, named by `--base` or the rules file, does not name a commit``.
2. **Which branches** come from `driftmark_core::git::inbox`: every local and remote-tracking branch, except those at the default branch's tip, and only one of several branches at one tip (`docs/features/engine/git-ranges.md`).
3. **Each branch is compared** with the default branch from where it left it, as `<default>...<branch>`: what a merge request shows. The rules file is read at the branch's tip.
4. **Only branches whose documents differ are listed**: at least one Markdown file changed. A branch merged into the default branch changes nothing from where it left, so it drops out. The page counts the branches left out: those without document changes, and those that share no history with the default branch.
5. **Newest first**, by the committer date of the branch's tip.
6. **The working tree** comes first, as one line: this checkout's `auto` range, with its counts. The watcher counts it in the background (see "Live reload"), so the page never waits for the working tree to be read. It shows the last count and when it was made, and says when a new count is running. Until the first count ends, it says it is counting.

Each branch shows its highest level, its number of changes and documents, when its tip was made, and the document to start with: the first of its change list, which sorts the highest level first. The counts are those of `driftmark diff --json=summary`. A branch's count keeps the paths of its changed documents too, so a reading page can say which open branches change its document.

The JSON, for a made-up repository with one branch, `feature`, that edits a decision record and adds a document, while `README.md` has an edit not committed yet:

```json
{
  "schema_version": 1,
  "default_branch": {"name": "origin/main", "tip": "6a4b0aaa6279cbc66f326430580836762ac76ea5"},
  "current_branch": "main",
  "branches": [
    {
      "name": "feature",
      "remote": false,
      "tip": "67e144003ae9156fa9a2f4e55fa51b512a938c53",
      "committed_at": 1767312000,
      "range": "origin/main...feature",
      "base": "6a4b0aaa6279cbc66f326430580836762ac76ea5",
      "notes": [],
      "documents": 2,
      "skipped": 0,
      "totals": {
        "changes": 4,
        "highest": "decision",
        "levels": {"decision": 3, "rule": 0, "fact": 1, "wording": 0, "formatting": 0},
        "kinds": {"formatting": 0, "edited": 2, "added": 2, "removed": 0, "moved": 0, "moved_edited": 0}
      }
    }
  ],
  "left_out": {"unchanged": 0, "unrelated": 0},
  "worktree": {
    "range": "auto",
    "state": "counted",
    "updating": false,
    "counted_at": 1792332186,
    "base": "6a4b0aaa6279cbc66f326430580836762ac76ea5",
    "notes": ["compared with `origin/main`"],
    "documents": 1,
    "skipped": 0,
    "totals": {
      "changes": 1,
      "highest": "wording",
      "levels": {"decision": 0, "rule": 0, "fact": 0, "wording": 1, "formatting": 0},
      "kinds": {"formatting": 0, "edited": 1, "added": 0, "removed": 0, "moved": 0, "moved_edited": 0}
    }
  }
}
```

| Field | Meaning |
|---|---|
| `schema_version` | 1, the version of the server's own JSON: the home page, a document and the status. It goes up by one when a field changes meaning or is removed. |
| `default_branch` | Its name and tip, or `null` when there is none. |
| `current_branch` | The local branch `HEAD` points at, or `null` when `HEAD` is detached. |
| `branches` | The listed branches, newest first. `range` is the range to open, `/changes/<range>`. `base` is where the branch left the default branch, and `notes` say how it was chosen, as for a range. `documents` counts the compared documents and `skipped` the files that could not be compared. `totals` is the change list's totals. `committed_at` is in seconds since 1970 UTC. |
| `left_out` | How many branches were compared and not listed: `unchanged` hold no document changes, `unrelated` share no history with the default branch. |
| `worktree` | The `auto` range's last count, or `null` in a bare repository. `state` is `counting` before the first count ends, `counted`, or `failed` with an `error`, such as a repository without a default branch. `updating` is `true` while a newer count runs. |

### A document

The page shows the document as HTML from `driftmark_core::render::reading` (`docs/features/engine/html.md`), with its diagrams drawn, and links to its JSON. A document from the working tree is read from disk; a document at a commit, from git.

The page also lists the open branches that change the document: the home page's branches whose changed paths hold it, newest first, up to six, each with the document's changes from where the branch left the default branch. Their headings mark the page's sections. A branch whose tip is the commit the page shows is left out. When the branches cannot be read, the page shows the document without them.

```json
{
  "schema_version": 1,
  "path": "docs/a.md",
  "revision": "feature",
  "commit": "67e144003ae9156fa9a2f4e55fa51b512a938c53",
  "blob": "21a092c5469e41bcd43d5e7df8bf67918fb7c167",
  "type": "decision",
  "status": "accepted",
  "title": "Cache pages",
  "summary": null,
  "bytes": 122,
  "lines": 11,
  "source": "---\nstatus: accepted\n---\n\n# Cache pages\n\n## Decision\n\nKeep each page for 90 minutes.\n…"
}
```

The `…` cuts the source here; the answer holds the whole text.

- `revision` is as the address wrote it. `commit` is the commit it resolved to, or `worktree`. `blob` is left out for the working tree.
- `type` and `status` come from the rules file at that revision (`docs/features/engine/rules-file.md`). `title` and `summary` come from the document's metadata (`docs/features/engine/document-model.md`).
- A document that cannot be parsed, such as a file that is not UTF-8, has `null` for `type`, `status`, `title`, `summary`, `bytes`, `lines` and `source`, and an `error` that says why. The page says the same.

### A range's change list

The page shows the range's change cards, document by document in the order of the change list, up to 200 KB of change HTML, 300 changes and 40 documents; a document that would pass a limit is skipped for the next, among the first 80 tried. The others are listed with a link to their own pages, at most 300 of them; the JSON lists every one. Each document's change HTML comes from `driftmark_core::render::changes`, with ids that start with `C` and a hash of its path, so the same HTML serves the range's page and the document's own page. The JSON is exactly `driftmark diff --json=summary` for the same range (`docs/features/commands/diff.md`), with one more field, `skipped`: the files that could not be compared, each with its `path` and the `reason`, in the words of `driftmark diff`.

```json
{
  "schema_version": 2,
  "range": {
    "base": "6a4b0aaa6279cbc66f326430580836762ac76ea5",
    "head": "67e144003ae9156fa9a2f4e55fa51b512a938c53",
    "notes": []
  },
  "totals": {"changes": 4, "highest": "decision", "levels": {"…": 0}, "kinds": {"…": 0}},
  "documents": [
    {"path": "docs/a.md", "file": "modified", "summary": {"changes": 3, "highest": "decision", "…": 0}},
    {"path": "docs/new.md", "file": "added", "summary": {"changes": 1, "highest": "fact", "…": 0}}
  ],
  "skipped": []
}
```

The `…` stand for the counts written out in full above. The whole change list of a large range, with every change, is not served here: 30 days of the private corpus make 47 MB. A page asks for one document's changes when it needs them.

### One document's changes

The JSON is a change list, in the shape of `driftmark diff --json` (`docs/features/engine/change-list.md`, `schema_version` 2), holding this one document with all its changes, plus `skipped`. When the document cannot be compared, `documents` is empty and `skipped` says why.

```json
{
  "schema_version": 2,
  "range": {
    "base": "6a4b0aaa6279cbc66f326430580836762ac76ea5",
    "head": "67e144003ae9156fa9a2f4e55fa51b512a938c53",
    "notes": []
  },
  "totals": {"changes": 1, "highest": "fact", "…": 0},
  "documents": [
    {
      "path": "docs/new.md",
      "file": "added",
      "summary": {"changes": 1, "highest": "fact", "…": 0},
      "changes": [
        {
          "kind": "added",
          "level": "fact",
          "signals": ["heading", "number"],
          "document": {
            "title": "New & old",
            "summary": "Short version: a new page.",
            "bytes": 70,
            "lines": 5,
            "source": "# New & old\n\n> Short version: a new page.\n\nIt holds 500 <b>pages</b>.\n"
          }
        }
      ],
      "unchanged": []
    }
  ],
  "skipped": []
}
```

A path that did not change in the range answers `404`.

### Search

`/api/search` finds the documents that hold every word of a query, best first, each with the block that matches best and a short snippet of it. There is no search page; pages draw the box and the results from this answer. `docs/features/engine/search.md` states the query syntax, the ranking and the snippet rules.

```text
GET /api/search?q=cache+%22one+hour%22&rev=main&path=docs/&type=decision&limit=20
```

| Parameter | Meaning |
|---|---|
| `q` | The query as typed. A missing or empty `q` finds nothing. |
| `rev` | A revision, as in `/doc/<revision>/-/<path>`: a branch, a tag, a commit or an expression such as `main~2`, or `worktree` for the files on disk. The default is `worktree`, or `HEAD` in a bare repository. |
| `path` | Keep only documents whose path starts with this text, as written: `docs/decisions/` keeps that folder, and `docs/d` also keeps `docs/drafts/`. |
| `type` | Keep only documents of this type from the rules file, such as `decision`. It may be given several times, to keep several types. A file that no type matches has the type `document`. |
| `limit` | The most results, from 1 to 100. The default is 20. |

The parameters are form-encoded, as a browser's `URLSearchParams` writes them: `+` and `%20` are spaces. Other parameters are ignored.

The answer, for a made-up repository with three documents at `main`. Its rules file makes `docs/decisions/*.md` decisions, and its decision record `docs/decisions/0001-cache-pages.md` has front matter, the title `# Cache pages`, the summary `> Short version: pages are kept in a cache.`, and "Keep each page for one hour." under `## Decision`. The test `crates/driftmark/tests/serve_search.rs` checks this answer:

```json
{
  "schema_version": 1,
  "query": "cache \"one hour\"",
  "revision": "main",
  "commit": "67e144003ae9156fa9a2f4e55fa51b512a938c53",
  "documents": 3,
  "total": 1,
  "results": [
    {
      "path": "docs/decisions/0001-cache-pages.md",
      "url": "/doc/main/-/docs/decisions/0001-cache-pages.md#decision",
      "title": "Cache pages",
      "type": "decision",
      "block": 4,
      "headings": ["Cache pages", "Decision"],
      "anchor": "decision",
      "snippet": [
        {"text": "Keep each page for ", "hit": false},
        {"text": "one hour", "hit": true},
        {"text": ".", "hit": false}
      ]
    }
  ],
  "skipped": []
}
```

| Field | Meaning |
|---|---|
| `schema_version` | 1, the version of the server's own JSON. |
| `query` | The query as typed. |
| `revision` | The revision as asked, or the default that was used. |
| `commit` | The commit it resolved to, or `worktree`. |
| `documents` | How many documents were searched: every Markdown file at the revision that could be parsed. |
| `total` | How many documents matched, before `limit`. |
| `results` | The matches, best first, at most `limit`. |
| `skipped` | The files that could not be searched, each with its `path` and the `reason`, such as a file that is not UTF-8. |

Each result:

| Field | Meaning |
|---|---|
| `path` | The document's path from the repository root. |
| `url` | The document's reading page at the same revision, with `#` and the section's anchor when there is one. |
| `title` | The document's title, or `null`. |
| `type` | Its type from the rules file at that revision. |
| `block` | The block that matches best, by its index in the document model. On the reading page, the elements with `data-k` of this value hold it (`docs/features/engine/html.md`). |
| `headings` | The titles of the headings above that block, outermost first. When the block is itself a heading outside any container, its own title comes last. Empty for a block before the first heading. |
| `anchor` | The page id of the nearest heading at or above the block, without `#`, or `null` when no heading comes before it. |
| `snippet` | Up to about 160 characters of the block's plain text, as pieces. Joined, the pieces give the snippet; each piece with `hit` set is a match, to be marked. A cut end shows `…`. |

A page writes each piece as text, never as HTML: a snippet holds the document's own words, which may contain `<` or `&`.

- **A revision's index** is built the first time it is searched, from every Markdown file of the commit, on all cores, and kept in the cache (see "The cache"). Requests that arrive while it is built wait for that one build.
- **The working tree's index** is built on the first search of `worktree`, then kept up to date from the change signal that reloads open pages (see "Live reload"): the files that changed are read again, and the next search finds their new text. When more than 256 files changed since the last full build, or `driftmark.toml` changed, the whole working tree is indexed again in the background. Searches meanwhile use the last index.
- A query with more than 32 words, a `limit` outside 1 to 100, or a parameter that is not valid text answers `400`.

### Status

`/api/status` shows what the cache holds and what the watcher does, for a person checking memory or a test:

```json
{
  "schema_version": 1,
  "threads": 12,
  "cache": {
    "cap_bytes": 134217728,
    "bytes": 23205,
    "kinds": {
      "rules": {"entries": 1, "bytes": 16544, "hits": 3, "misses": 1, "evicted": 0},
      "documents": {"entries": 4, "bytes": 5345, "hits": 5, "misses": 4, "evicted": 0},
      "ranges": {"entries": 1, "bytes": 644, "hits": 0, "misses": 1, "evicted": 0},
      "branches": {"entries": 1, "bytes": 672, "hits": 0, "misses": 1, "evicted": 0},
      "search": {"entries": 1, "bytes": 5120, "hits": 2, "misses": 1, "evicted": 0}
    }
  },
  "worktree": {
    "watching": true,
    "files": 3,
    "changes": 0,
    "counting": false,
    "listeners": 0,
    "counted_at": 1792332186,
    "count_ms": 10
  },
  "search": {
    "worktree": {"documents": 3, "changed": 1, "bytes": 4096}
  }
}
```

A kind of entry shows once it was asked for: `rules`, `documents`, `pages`, `ranges`, `branches` or `search`. `worktree` is `null` in a bare repository. Its `files` are the Markdown files watched, `changes` counts the changes announced, `listeners` the open event streams, and `count_ms` how long the last count took. `search.worktree` is the working tree's search index: the documents it searches, the files read again since its last full build, and its estimated memory. It is `null` before the first search of the working tree, and in a bare repository.

## Errors

| Status | When | Example message |
|---|---|---|
| 400 | A search's parameters are wrong: a query with more than 32 words, a `limit` outside 1 to 100, or a parameter that is not valid text. | `` `limit` must be a number from 1 to 100 `` |
| 404 | The address names nothing: an unknown revision, text that is not a range, a date in neither form, two points without a common history, a path that is not a Markdown file in the repository, a document missing at that revision, a document that did not change in the range, a named default branch that does not exist, or a working tree in a bare repository. | `` `nope` does not name a commit `` |
| 500 | Reading git failed, or the rules file is broken. The message names the cause. | ``git could not be read: object 3f2a… is not in the repo`` |

A page route answers with an HTML page that says so; an `/api/` route answers with JSON: `{"error": "<the message>"}`.

## Live reload

The page script opens `/events` on a page whose content can change on disk: the home page, a document at `worktree`, and the `auto` range. The stream sends:

| Event | Data | When |
|---|---|---|
| `ready` | `{}` | At once, so the page knows the stream is open. |
| `change` | `{"paths": ["docs/a.md"]}` | Working-tree files changed, came or went. The paths run from the repository root. An empty list means events were missed. |
| `counted` | `{}` | The watcher counted the working tree again for the home page. |

A page that shows the working tree reloads on `change`. The home page reloads on `counted`, so it shows the new counts. Every 15 seconds the stream sends a comment, so nothing between the browser and the server closes it as idle.

**How the watcher works.** It polls; it asks the system for no file events. A file-event library would add a licence that `deny.toml` does not allow yet (CC0-1.0, with ISC below it), and polling looks only at what git lists:

1. Once a second it checks the size and modification time of every Markdown file git lists in the working tree, of the folders that hold them, and of `driftmark.toml`.
2. A file that changed is announced at once with `change`.
3. A folder that changed may hold a new or removed file. So the working tree is listed again through the git reader, which follows the ignore rules, and the files that came or went are announced.
4. Each listing also counts the `auto` range for the home page, then sends `counted`.
5. The `.git` folder is never looked at; the git reader keeps it to itself. Build folders and ignored files are never looked at either, because git does not list them.

A new Markdown file in a folder that held none before is found by the next listing: after another change, when the home page finds the last count older than 60 seconds, or within 60 seconds while a page listens.

## Static files

These files are built into the program:

| File | What it is | Loaded by |
|---|---|---|
| `page.css` | The look of every page. | every page |
| `page.js` | The theme button, the search box, live updates, tables, code colours and the browser's diagrams. | every page |
| `liberation-sans-regular.woff2` | The diagram font, Liberation Sans 2.1.5, unmodified (147,036 bytes). | pages with a drawn diagram |
| `prism-core.js`, `prism-<language>.js` | Prism 1.30.0 and 59 of its languages. | pages with code, for their languages |
| `mermaid.js` | mermaid.js 12.1.0, for diagrams the server could not draw. | pages with such a diagram |

Each is served at `/static/<hash>/<name>`, where the hash is 16 hex digits of a hash of its bytes, so a new build gives a new address. At the right hash, the answer says `Cache-Control: public, max-age=31536000, immutable`: the browser keeps it for a year. At an old hash, the same file comes back with `no-cache`, so a page loaded before an upgrade still works.

- The style sheet and the script name the other files by their hashed addresses. The server writes those addresses into them when it starts, so the files carry no placeholder when they are served.
- `mermaid.js` is stored compressed (1.6 MB instead of 5.5 MB). It goes as it is, with `Content-Encoding: gzip` and `Vary: Accept-Encoding`, to a browser that accepts gzip, which every browser does. Any other client gets it unpacked.
- Their licences sit beside them in `crates/driftmark/assets/`: OFL 1.1 for the font, MIT for Prism and mermaid.js. mermaid.js bundles other projects (d3 under ISC among them); the release's notices file (M6) lists them.

## Compression

Every answer is compressed with gzip when the browser sends `Accept-Encoding: gzip` and the answer is longer than 32 bytes. The event stream is not, because each event must reach the page as it is written.

## The cache

The server keeps what it read between requests, keyed by what fixes its content:

| Kind | Key | Value |
|---|---|---|
| `rules` | The rules file's blob id. | The parsed rules. |
| `documents` | The blob id, and the version of the rules file. | The parsed document, or why it cannot be parsed. |
| `pages` | The blob id, the rules version and the path; or the two blob ids, the rules version and the path. | A document's HTML and its headings; or one document's changes, their HTML and its title. Without the page around them, which changes with the branches. |
| `ranges` | The two commit ids. | Each document's counts. |
| `branches` | The default branch's tip and the branch's tip. | The branch's counts. |
| `search` | The commit's tree id. | The search index of every Markdown file in it. |

The rules version is part of a document's key because the rules file's masthead keys change how a title and a summary are read. The path is part of a page's key because the page shows it.

- **Nothing goes stale.** A branch name never keys an entry, because a branch moves. Each request resolves the name, then looks up what that commit holds. A branch with a new commit gets new entries; the old ones stay correct for the old commit.
- **The working tree is never cached.** Its bytes can change while its name stays. A document at `worktree`, the `auto` range and its documents are read and compared on every request.
- **Drawings have a cache of their own**, 32 MB, keyed by a hash of the diagram's language and text (`docs/features/pages.md`, "Diagrams"). A document read again from the working tree, or another document with the same diagram, draws nothing again.
- **The cap is 128 MB**, counted as the estimated heap memory of each entry: a document's text, each block's plain text, spans and details, and the heading paths and containers that blocks share, counted once. When the entries add up to more, the least recently used go first. A value larger than the whole cap is used once and not kept.
- **Search indexes share the cap.** A revision's index is one entry, about a fifth of the cap for a corpus of 3,500 documents (see "Speed"). It stays while the reader keeps searching, because each search uses it again; pages read since push it out only when they are used more recently. A dropped index is built again on the next search, in well under a second. The working tree's index is not in the cache, like every working-tree read; `/api/status` shows its size.
- **Why 128 MB.** The speed spike measured about 5 MB of block lists for the whole private corpus, and 152 to 186 MB with the text and the rendered HTML. Parsed documents are larger than block lists, so the whole corpus does not fit. A day's reading does, and a dropped document costs one parse, a few milliseconds, when it is opened again.
- **One load per key.** Two requests that need the same missing entry share one load: the second waits for the first. An error is not kept, so the next request tries again.
- **Shared work.** A blob that many branches change, or that a page opened before, is parsed once. The home page compares the branches it has not counted yet together, on all cores.

## Who may ask

**Bound to this machine only** (an address in `127.0.0.0/8` or `::1`, and no `--token`): a request must name this machine in its `Host` header: `localhost`, a name under `.localhost`, an address in `127.0.0.0/8`, or `[::1]`, with any port. Any other host gets `403`. This stops a web page elsewhere that points its own name at `127.0.0.1` (DNS rebinding): its requests carry its own name. The port is not checked, so a tunnel from another local port still works.

**Bound to any other address, or started with `--token`**: every request needs the token, the static files and the event stream included. Without `--token`, a random one is made: 128 bits from the system, written as 32 hex digits.

1. The reader opens the address the server printed, which ends in `?token=<token>`.
2. The server checks the token. When it matches, it answers `303 See Other`, sends the browser to the same address without the `token` part (other query parts stay), and sets the cookie `driftmark-<port>=<token>; Path=/; HttpOnly; SameSite=Strict; Max-Age=2592000`.
3. The browser then sends the cookie with every request, for 30 days. The token is no longer in the address, so it does not stay in the history or in a link the reader shares.
4. A request without the token or the cookie, or with a wrong one, gets `401` and a page that says to open the printed address. The token is accepted in the address only for `GET` and `HEAD`.

- The cookie's name holds the port, because a browser sends a host's cookies to every port, and two servers must not share one.
- `HttpOnly` keeps the cookie from scripts. `SameSite=Strict` keeps another site from sending requests with it.
- Tokens are compared in time that does not depend on where they differ.
- A tool without a cookie jar uses one: `curl -c jar -b jar -L 'http://host:4170/api/home?token=…'`.

**Requests with a body** (notes, from M3 on) must be JSON and come from the server's own pages. Before any route sees it, a `POST`, `PUT`, `PATCH` or `DELETE`:

- without `Content-Type: application/json` gets `415`;
- with an `Origin` other than `http://<the Host it asked for>` gets `403`, and so does `Origin: null`;
- with `Sec-Fetch-Site` other than `same-origin` or `none` gets `403`.

A request without `Origin`, such as one from a command-line tool, passes; it still needs the token where one is required. M2 has no route that takes a body, so an allowed request then gets `405`.

## Safety

- **Every path comes from git.** No route opens a file by a path taken from the address. The path is decoded to bytes and must be a path git could store: no empty, `.` or `..` part, no NUL byte, no `/` at either end. It is then looked up in git: in the commit's tree, or, for the working tree, among the files git tracks at `HEAD` or lists as not ignored. The git reader reads a working-tree file only through real folders, so a symbolic link cannot lead outside the repository.
- **Content Security Policy** on every answer, errors included: `default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; font-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'`. A page runs only the program's own script and style sheet, loads images and fonts only from the server, and cannot be framed. Inline scripts, inline styles and images from the web are blocked. The pages need nothing more: a drawing's own style block is removed and the style sheet colours it; a bar's widths are SVG attributes; and mermaid.js, in the browser, gets its styles through a style sheet the page script builds and through the style object, which the policy allows.
- **Other headers** on every answer: `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, `X-Frame-Options: DENY`, `Cross-Origin-Resource-Policy: same-origin` and `Cross-Origin-Opener-Policy: same-origin`.
- **No CORS headers**, ever, so a page on another site cannot read an answer.
- **Text from the repository is escaped** in every page: file names, branch names, titles and document text.
- **A slow request does not stall others.** Git work runs on separate threads.

## Speed

Measured on the maintainer's server, which has 12 cores, on 2026-10-18, with the release build serving the private corpus on `127.0.0.1`. The corpus has 401 branch refs; 341 are compared after the rules of the inbox, 144 are listed and 197 hold no document changes. The 30-day range is 30 days of the corpus's working branch, written `since/<date>/<branch>`: 2,945 changed documents in the whole repository. The large document is the corpus's largest, 479 KB, a 486 KB page.

"Cold" is the first request after the server started, with an empty cache: once right after the greeting, and once after the startup count of the working tree (380 ms) ended. "Warm" is the median of 7 requests after it, with the slowest in brackets. The load average was about 10.

| Page | Cold, at start | Cold, after the startup count | Warm |
|---|---|---|---|
| Home page | 289 ms | 250 ms | 4.2 ms (6.4) |
| Home page, JSON | 266 ms | 249 ms | 3.8 ms (6.7) |
| 30-day change list | 236 ms | 215 ms | 4.7 ms (10.7) |
| 30-day change list, JSON (845 KB) | 223 ms | 204 ms | 1.8 ms (6.4) |
| Large document | 16 ms | 13 ms | 0.4 ms (0.9) |

- **The home page's work**, cold: listing the branches 5 ms; the merge-bases and the changed files of 341 branches about 110 ms on 12 threads; comparing 1,763 distinct file changes about 185 ms. A merged branch is found by its merge-base alone, and a change that several branches share is compared once (2,114 changes, 1,763 distinct).
- **`driftmark diff`** takes 225 to 248 ms for the same 30-day range at the same load, so the cold change list costs what the command costs.
- **Search** builds a revision's index in about 0.43 seconds on 12 cores and answers a query in about 5 ms with its snippets; `docs/features/engine/search.md` has the measurements.
- **At a load of 65**, earlier the same day, the cold home page took about 2 seconds and the cold 30-day list 1.2 to 1.7 seconds; warm pages stayed under 25 ms.

**Memory.** Opening every changed document of the 30-day range, its changes page and its reading page (5,070 requests, all answered with 200, median 0.9 ms, slowest 35 ms), left the cache at 127.7 MB of its 128 MB: 104.8 MB in 1,488 documents and 22.9 MB in 2,136 pages, after dropping 2,285 documents and 2,934 pages. The process as a whole held more:

| Moment | Process memory (RSS) | With `MALLOC_ARENA_MAX=2` |
|---|---|---|
| After start | 37 MB | 35 MB |
| After the 30-day list, cache nearly empty | 211 MB | 161 MB |
| After every changed document | 387 to 455 MB | 308 MB |
| After the home page as well | 579 MB | 410 MB |

About 52 MB of it is pack files mapped from disk. The rest above the cache is memory the system allocator keeps after the counting threads free it: each of the 12 threads keeps its own pool, and memory one thread frees is not reused by another. With two pools instead of 96 (`MALLOC_ARENA_MAX=2`), the same run ends 147 MB lower. A smaller gix pack cache (1 MB instead of 16 MB per handle) changed nothing measurable.

Other allocators did not help, measured the same way later that day at a load of 27 to 63:

| Allocator | After every changed document | After the home page as well |
|---|---|---|
| The system's (glibc) | 375 to 406 MB | 528 to 550 MB |
| glibc with `MALLOC_ARENA_MAX=2` | 290 MB | 419 MB |
| mimalloc 0.1.52 | 455 to 490 MB | 888 MB |
| mimalloc, `MIMALLOC_PURGE_DELAY=0` | 446 MB | 679 MB |
| jemalloc 0.7.0 | 366 to 376 MB | 465 to 478 MB |
| glibc on 4 cores instead of 12 | 353 MB | 447 MB |

What stays above the cache is the working memory of the counts on many threads: during the 30-day list the process peaks at 230 to 270 MB with an empty cache, and no allocator gives most of it back. The program keeps the system's allocator.

## Not covered yet

- **Images and other files.** The server serves Markdown documents only. A reading page shows a link to any other file as its text, and an image as its alt text.
- **A size limit before parsing.** A file with very deep nesting is refused by the parser after it reads it; the `render` lane adds a check before the parser sees the file.
- **Notes** (M3).
- **HTTPS.** The server speaks plain HTTP; across machines, use a private network such as Tailscale.
- **Paths that are not UTF-8 in change lists.** A change list names files as text, so the link to such a file's changes may not lead back to it.
