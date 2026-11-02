---
status: target
date: 2026-11-02
---

# The export command

> Short version: `driftmark export` writes one HTML file that someone outside the repository can open from disk, in any browser, with no server and no network: a manager, a compliance reader, an email, a ticket, or a merge request. It exports one document, every document under a folder with an index, or a range's changes with the full text of the documents it changed. Everything the pages need is inside the file, and its Content-Security-Policy lets nothing else run or load. Links between documents in the file work; links to anything else show as text that says it was not exported. The file has no notes, no "Seen" and no live updates. A file with more than one document carries a small search index. A folder or a range too big for the size limit, 10 MB unless `--max-size` says otherwise, is cut down, and the file says what it left out. The same commits give the same bytes.

## Running it

```text
driftmark export doc <revision> <path> [<options>]
driftmark export folder <revision> <folder> [<options>]
driftmark export changes <range> [<options>] [-- <folder>...]
driftmark export changes --since <date> [<branch>] [<options>] [-- <folder>...]
```

| Command | What the file holds |
|---|---|
| `driftmark export doc main docs/guide.md` | The document at `main`. |
| `driftmark export doc worktree docs/guide.md` | The document as it is on disk now, committed or not. |
| `driftmark export folder v1.0 docs/decisions` | Every Markdown document under `docs/decisions` at the tag `v1.0`, with an index. `.` is every document of the repository. |
| `driftmark export changes main...feature` | What a merge request of `feature` into `main` changes: every changed document's change cards, and the full text of each one that still exists. |
| `driftmark export changes --since 2026-10-01 main` | Everything that reached `main` from 1 October on. |
| `driftmark export changes auto` | The files on disk against where this branch left the default branch. |
| `driftmark export changes main...feature -- docs` | Only the documents under `docs`. |

- **A revision** is anything git resolves to a commit: a branch, a tag, a commit id or an expression such as `main~2`. The word `worktree` means the files on disk, as in the server's addresses (`docs/features/commands/serve.md`).
- **A range** takes every form of `driftmark diff` (`docs/features/commands/diff.md`): `a..b`, `a...b`, `auto`, and `--since <date> [<branch>]`. Folders after `--` keep only the documents under them; the range is still counted from the whole repository.
- **Paths and folders** run from the repository root, wherever the command runs. A leading `./` is allowed.

### Options

| Option | Meaning |
|---|---|
| `-o <file>`, `--output <file>` | Write the file here. Without it, the file goes to standard output. |
| `--repo <path>` | A folder inside the repository. The default is the current folder. |
| `--max-size <size>` | Keep the file under this size: bytes, or with `K` or `M` for thousands or millions of bytes, such as `500K` or `25M`. The default is `10M`. See "Too big for one file". `doc` ignores it: one document is never cut. |
| `--base <branch>` | The branch `auto` compares with, as for `driftmark diff`. Only `changes auto` takes it. |
| `--help` | Print the usage text. |

Options may come before or after the other words, and take their value as the next word or after `=`, as in `--max-size=25M`.

### What it prints

- **Standard output** gets the file, unless `-o` names one. Nothing else goes there.
- **Standard error** gets one line when the file was written, and one more when the file was cut down. A program can read them:

```text
driftmark export: wrote `review.html`: 14 documents, 1203554 bytes
driftmark export: This file holds every change of this range, and the full text of 9 of the 12 documents it changed, highest ranked first. The others did not fit in one file of at most 10 MB.
```

  With standard output, the first line says `wrote standard output`. "Documents" counts the documents the file shows: their cards, their full text, or both.

| Exit code | When |
|---|---|
| 0 | The file was written. |
| 1 | It could not be made: the folder is not inside a git repository; a revision, a range or a document does not exist; git could not be read; the rules file is broken; or the file could not be written. The reason goes to standard error, with the words of `driftmark diff` and the server, such as ``driftmark export: `nope` does not name a commit``. |
| 2 | The command line is wrong. The reason and the usage text go to standard error. |

**The same commits give the same bytes.** A file names its commits, not the time it was made, and every list in it has a fixed order. So exporting a range twice gives the same file, and a tool can tell whether anything changed by comparing files. Only the working tree, which changes on disk, gives different files.

## What the file holds

Every file has the bar on top (driftmark, the search box when the file has one, and the theme button), one line saying what the file is ("Exported by driftmark: this file opens offline, holds no notes, and stays as it is when the documents change."), then its pages. The pages are the server's pages of decision 0010, with the same classes and the same style sheet (`docs/features/pages.md`), so they look the same and keep the same readability rules: 17 px text, WCAG AA contrast in light and dark, marks never by colour alone, tables as cards on a phone.

### `doc`: one document

One reading page: the path, the revision and its commit, "On this page", the pills (type, status, lifecycle, date), and the document. There is no search box: one document needs none.

### `folder`: a folder with its index

1. **The index**, which opens first: the folder, the revision and its commit, and how many documents. Then the documents, grouped by the folder each one sits in, in path order. Each shows its title as a link to its page, its path, its pills and its summary.
2. **One reading page per document**, in the same order. Each says where it was read and has a link back to the index.

### `changes`: a range's changes

1. **The change list**, which opens first. It is the server's change page (`docs/features/pages.md`, "Change pages"):
   - the rail, with every document and its changes;
   - "What needs your eyes";
   - one section per document, highest ranked first, with its level, its counts, its status change, and one card per change.
   
   The server's page shows at most 40 documents and leaves the rest to their own pages; the file has no other pages, so it shows every document. The rail lists the first 300 changes one by one and counts the rest, so a month of changes does not double the file.
2. **The full text of each changed document**, at the newer side, as a reading page. A document's title on the change list leads to it, and the reading page links back to "its changes". A deleted document has no newer side, so its title is plain text, and its card shows its title, summary and length, as on the server.

## One file, with nothing fetched

Everything is inside the file:

| Part | How it is inside |
|---|---|
| The style sheet | The server's `page.css`, then `export.css`, in one `<style>` element. |
| The page script | `export.js`, in one `<script>` element in `<head>`. |
| Code colours | Prism 1.30.0: its core, and the language files the file's code uses, with the languages they build on, each in its own `<script>` element. A file without code carries no Prism. |
| Diagrams | Drawn on the export's machine by merman (or `d2`), cleaned exactly as for the server's pages (`docs/features/pages.md`, "Diagrams"), and written into the page as SVG. |
| The diagram font | Liberation Sans, 147,036 bytes, as a base64 `data:` address in the style sheet's `@font-face`: 196,048 bytes. Only a file with a drawn diagram carries it. |
| The search index and the settings | JSON in `<script type="application/json">` elements, which the browser never runs. |

**The Content-Security-Policy** is a `<meta http-equiv>` element, the first thing in `<head>` after the character set:

```text
default-src 'none'; style-src 'sha256-…'; script-src 'sha256-…' 'sha256-…'; font-src data:; base-uri 'none'; form-action 'none'
```

- `style-src` names the SHA-256 hash of the one style sheet, and `script-src` the hash of each script, in base64. Nothing allows `'unsafe-inline'`. A script or a style sheet added to the file afterwards, or changed, does not run.
- `font-src data:` appears only when the file carries the font.
- Everything else falls under `default-src 'none'`: no image, no frame, and no request (`fetch`, a form, a link prefetch). A `style` attribute is blocked too, because `style-src` names hashes only; the pages have none.
- A policy in a `<meta>` element cannot hold `frame-ancestors`, so the file does not refuse to be framed; a browser that opens it from disk shows it alone.

**What the pages leave out** to stay inside the file:

- **No notes and no "Seen"** (decision 0004: notes live in the notes file, not in what is shared). Change cards lose their two buttons; nothing reads or writes the notes file.
- **No live reload.** The file is a snapshot.
- **No diagram drawn in the browser.** mermaid.js weighs 5.5 MB, so a diagram the export could not draw stays code, marked "Diagram, not drawn". In the private corpus's 30-day export, all 68 diagrams were drawn.
- **No images.** An image shows as its alt text, after "Image:", with "(not exported)".

## Links

- **Inside a document**, a link to a section (`#install`) and a footnote's links lead to that document's own places.
- **To another document in the file**, a link leads to that document's page, and to its section when it names one: `decisions/0001-cache.md#decision` leads to the page `R1` at its `decision` heading. Links read the way the server reads them: relative to the document's folder, or from the repository root when they start with `/`, with `%20` and the like decoded.
- **To anything else** (a document the file does not hold, any other file, a folder) the link shows as its text, dotted underlined, followed by "(not exported)" in smaller text.
- **To the web** (`https:`, `http:`, `mailto:`), the link stays.
- No `javascript:` or `data:` link reaches the file: the same address rules as the server's pages apply (`docs/features/engine/html.md`, "Raw HTML").

### Places in the file

The file's places have fixed names, so another tool, such as the merge-request job, can link into it as `review.html#D3`:

| Id | What |
|---|---|
| `Changes` | The change list of a `changes` file. |
| `Index` | The index of a `folder` file. |
| `D<n>` | Document `n`'s section on the change list, counted from 1 in the list's order. |
| `C<n>.<i>` | Change `i` of document `n`, counted from 0, as the change HTML names them (`docs/features/engine/html.md`, "The change HTML"). A moved block's old place is `C<n>.<i>.from`. |
| `R<n>` | Document `n`'s reading page: in a `changes` file, the same `n` as its section; in a `folder` file, its place in the index; `R1` in a `doc` file. |
| `R<n>-<anchor>` | A heading of that page: its GitHub anchor after the page's id, such as `R3-install`. Footnotes are `R<n>-fn:<name>`. |
| `R<n>/k.<b>` | Block `b` of that page, as search results name it; the script scrolls to it and marks it. |

Two documents with a section of the same name keep their own ids, because every id inside a document starts with its page's id (`render::reading_prefixed`, `docs/features/engine/html.md`).

## The page script

`export.js` sets the reader's theme before the first paint, then:

- shows **one page at a time**: the page a link points at, or the first page. A link to a place in another page shows that page and scrolls to the place; the browser's back button works, because pages change with the address's `#` part. The brand leads to the first page; the skip link keeps the page shown;
- opens "On this page" and the rail on wide screens, marks the tables that fit their column so their headers stick, and colours each page's code with Prism the first time it shows;
- runs the search box;
- switches the theme with the button, and keeps the choice in the browser when it may (a file opened from disk may have no storage; the theme still changes for the page).

**Without the script**, such as in a mail program's preview, every page shows, one after another, and every link still leads to its place. A printed file shows every page too.

## Search

A file with more than one document has a search box: "Search this file". It searches every page in the file: the reading pages, and in a `changes` file each document's cards too. It takes the server's queries (`docs/features/engine/search.md`): words, `"phrases"`, a last word that is a prefix while it is typed, and a piece such as `read-only` as a phrase. Every clause must match. Up to 10 results drop down, each with the document's title (or "Changes: " and the title for cards), its path and the section, and a snippet with the matched words marked. A result shows its page at its block and marks it.

**The index** is built when the file is written. For every word, as `driftmark_core::search::terms` splits and folds it, it lists the pages that hold it. It is JSON in a `<script type="application/json" id="SearchIndex">` element, written without spaces; here with line breaks:

```json
{
  "v": 1,
  "pages": [
    ["R1", "Cache lantern schedules", "docs/decisions/0001-cache.md", "read"],
    ["D1", "Cache lantern schedules", "docs/decisions/0001-cache.md", "changes"]
  ],
  "words": ["cache", "hour"],
  "in": ["0.1", "0"]
}
```

- `pages`: each page's id, title, path, and `read` or `changes`.
- `words`: every word, sorted.
- `in`: for each word, the numbers of the pages that hold it, written as the gaps between them in base 36, joined by `.`. Pages 0, 3 and 40 are `0.3.11`.

The script looks up the pages that hold every clause in the index, then reads only those pages' text to check phrases, choose the best block and cut the snippet. It ranks as the server does, `weight × (field + more)` per clause, with one difference: `more` counts the blocks that match, not the matches.

**What it costs**, measured below: on this repository's fixture folder, 1.5 KB for 3 documents (0.6 KB after gzip). On larger files, see "Sizes".

## Too big for one file

The file stays under `--max-size`, 10 MB by default. An email client sends an attachment as base64, a third larger, so a 10 MB file still fits the 20 to 25 MB most mail services accept.

**A range** fills the file in this order:

1. **Every document's change cards**, highest ranked first: the changes are what the file is for. When they do not all fit, the file holds the cards of the highest-ranked documents that fit, lists the others at the end with their level and counts under "N more documents not in this file", and holds no full text.
2. **Then the full text of each changed document**, highest ranked first, each one that still fits. A document whose text is left out keeps its cards; its title is plain text.

**A folder** keeps its whole index, and holds the text of each document in the index's order while it fits. A document left out stays in the index, without a link, marked "not in this file".

**The file says what it left out**, in a box at the top of its first page and on standard error:

- "This range is too big for one file of at most 10 MB. The file holds the changes of the 120 highest-ranked of its 900 documents, without their full text; the other 780 are listed at the end with their counts."
- "This file holds every change of this range, and the full text of 9 of the 12 documents it changed, highest ranked first. The others did not fit in one file of at most 10 MB."
- "This folder is too big for one file of at most 10 MB. The file holds the text of 40 of its 51 documents; the index lists the others without their text."

The numbers in these examples are made up. Links in the file always lead to a page the file holds: a document left out is written as text everywhere.

## Safety

- **The same HTML as the server's pages.** Document HTML comes from `driftmark_core::render` with the same raw-HTML allow-list and address rules, and every drawing goes through the same SVG cleaning (`docs/features/engine/html.md`, `docs/features/pages.md`). Every text from the repository (titles, paths, branch names) is escaped.
- **The policy.** Only the file's own blocks run, by hash; nothing loads from anywhere (see "One file, with nothing fetched").
- **No secrets.** The file holds the documents, their changes and the commit ids, nothing from the reader's machine: no notes, no token, no path outside the repository.
- **JSON inside the file** writes every `<` as `\u003c`, so no title or word can close its element.

## Sizes

Measured on 2026-11-02 with the release build. "gzip" is `gzip -6` of the whole file, which is about what a zip attachment or a server's compression gives.

**What every file holds.** The style sheet is 51.4 KB and the script 19.3 KB: about 71 KB before gzip and 17 KB after, in every file. The diagram font adds 196.0 KB (147 KB after gzip, which cannot shrink a font), only with a drawn diagram. Prism adds 7.5 KB for its core and 0.1 to 14 KB per language.

**On this repository's fixtures** (`fixtures/export/`, each with one drawn diagram, and Rust and shell code):

| File | Size | gzip | Of it: pages | Of it: search index |
|---|---|---|---|---|
| `doc main docs/guide.md` | 292.8 KB | 177.0 KB | 8.3 KB | none |
| `folder feature docs`, 3 documents | 298.6 KB | 178.2 KB | 12.5 KB | 1.5 KB (0.6 KB gzip) |
| `changes main...feature`, 4 documents, 5 changes | 305.6 KB | 179.5 KB | 19.0 KB | 1.9 KB (0.7 KB gzip) |

**On the private corpus** (a private repository of 3,725 Markdown documents), read-only at its working branch, on the maintainer's server with 12 cores at a load of 5.6 to 7.6. Only numbers are recorded here.

| Export | Size | gzip | Search index (gzip) | Time | Memory |
|---|---|---|---|---|---|
| A typical document: the median source, 10.0 KB | 87.8 KB | 23.5 KB | none | 0.01 s | 23 MB |
| The largest document: 806 KB of source | 1.09 MB | 252 KB | none | 0.04 s | 32 MB |
| A folder of 51 documents | 1.28 MB | 337 KB | 161 KB (46 KB) | 0.05 s | 36 MB |
| The last merge on the branch: 2 documents | 138 KB | 38.6 KB | 12.9 KB (3.8 KB) | 0.02 s | 26 MB |
| 30 days, 1,255 changed documents, at the default 10 MB | 8.33 MB | 1.52 MB | 578 KB (174 KB) | 0.74 s | 375 MB |
| The same at `--max-size 25M` | 21.9 MB | 4.70 MB | 1.51 MB (476 KB) | 2.70 s | 579 MB |
| The same with no limit (`--max-size 2000M`) | 69.5 MB | 14.6 MB | 3.90 MB (1.24 MB) | 2.95 s | 660 MB |

- **Cut down at 10 MB**, the 30-day file holds the cards of the 261 highest-ranked documents and lists the other 994. It stops at 8.3 MB because the next document's cards did not fit, and the file keeps a whole run of the highest ranked rather than skipping one.
- **At 25 MB** it holds every change of the 1,255 documents and the full text of 150 of the 1,152 that still exist.
- **The search index** is 6 to 13 percent of a file, about the same share after gzip. A folder of 51 documents pays 46 KB on the network.
- **No document on this corpus needed the size limit to stay whole:** the largest makes a 1.09 MB file.
- **In the browser**, offline, in headless Chromium: the 10 MB file opens in 1.9 s; its first search takes 0.44 s, because it reads the text of every page that matches, and later ones 8 to 63 ms. The 25 MB file opens in 6.6 s and its first search takes 1.3 s. The 51-document folder opens in 0.36 s and searches in under 100 ms.
- **Every file checked**: no id twice (35,217 ids in the largest), every link to a place in the file finds it, no `src` or `style` attribute, the policy's hashes match every block, and no console error in Chromium.

## Tests

- `crates/driftmark/tests/export.rs` builds the made-up repository of `fixtures/export/` (two commits on two branches) and runs the program: each kind of file, links between documents with the same section names, folders after `--`, the working tree, both ways a range is cut down and a folder too, the same bytes twice, and the exit codes. For every file it checks that the policy names exactly the hash of each inline block, that every id is unique, that every link leads to an id in the file or to the web, and that nothing would load from outside. It writes the files to `<target>/tmp/export/`.
- `tests/browser/export.mjs` opens those files from disk in headless Chromium, with the browser offline and every request refused. It checks that the browser asked for nothing, that no console error, page error or policy violation happened, that links, the back button, the drawn diagram in its font, Prism, the theme button and the search work, and that every page shows without the script. axe-core 4.14 checks each page shown, the search results and a cut-down file, in light and dark, at 1440 and 390 px: 70 checks, all passing. Run `cargo test -p driftmark --test export`, then, in `tests/browser/`, `npm ci` and `node export.mjs <target>/tmp/export [<screenshots folder>]`.

## Not covered yet

- **Images.** They show as their alt text. Putting them in the file as `data:` addresses would need an image allow-list and a size rule.
- **Diagrams the export cannot draw** stay code; there is no browser fallback in a file.
- **Safari and Firefox** were not tested; the file uses `:has()` from the server's style sheet (Safari 15.4 and later) and `Intl.Segmenter` for search words (Firefox 125 and later; older browsers split words with a simpler rule).
- **A notes review file in the export.** Notes stay in the notes file (decision 0004); a later version may add a reviewed note list on purpose.
