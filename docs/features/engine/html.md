---
status: target
date: 2026-10-18
---

# Page HTML

> Short version: `driftmark-core` turns a parsed document into reading HTML, and one document's change list into change HTML. It decides the structure only: the elements, classes and attributes the page script and the style sheet may rely on. Every block that holds text carries `data-k="<block index>"`. Beside the HTML, each reading page has position tables, one per block, which the page script `anchor.js` uses to turn a selection into source bytes and back. Two traits let the program decide what the core cannot: `LinkResolver` says where a relative link leads, and `DiagramRender` draws Mermaid and D2. Raw HTML passes only through an allow-list, every other tag shows as text, and no `javascript:` or `data:` address reaches a page. Files over 2 MiB, or with a line that opens more than 100 quotes, are refused before the parser sees them.

## How it is called

`driftmark_core::render` holds both writers.

| Call | What it returns |
|---|---|
| `reading(&document, &setup)` | A `Reading`: the page HTML, the position tables and the outline. |
| `reading_prefixed(&document, &setup, prefix)` | The same, for a page that holds several documents, such as a one-file export: every id starts with `prefix` (see "Ids"). |
| `changes(&document_changes, id_prefix)` | One document's change list as an `<ol class="changes">`. |
| `ANCHOR_SCRIPT` | The page script, `crates/driftmark-core/web/anchor.js`, as text. |
| `words` | The words a reader sees for a level (`Decision`), a kind (`Moved and edited`, `New document`), a view (`Sentence by sentence`) and a signal (`in a decision section`). The change HTML and the program's own pages use the same words. |

Neither call fails and neither reads anything. A node whose source position cannot be read is still shown; it only gets no place in the position table.

`Setup` holds what the program decides for a reading page:

| Field | What it holds |
|---|---|
| `links` | A `LinkResolver`. `AsWritten` keeps every address as written. |
| `diagrams` | A `DiagramRender`. `NoDiagrams` draws nothing. |
| `allowed_html` | The raw HTML elements the rules file allows: `LintSettings::allowed_html`. |

`Reading` has three fields:

| Field | What it holds |
|---|---|
| `html` | The document as HTML, without `<html>`, `<head>` or `<body>`. The page puts it inside its own element, such as `<article>`. |
| `positions` | A `PositionTables`. It serializes to JSON as one array, so a page can load it only when the reader starts a note. |
| `outline` | The headings outside any container, in order: `level`, `id`, `text` and `block`, for an "on this page" list. |

## The traits

### `LinkResolver`

`resolve(address, kind)` returns `Resolved::Url(url)` or `Resolved::Text`. `kind` is `LinkKind::Link` or `LinkKind::Image`.

Only relative addresses reach it:

| Address | What happens |
|---|---|
| `setup.md#install`, `../img/cat.png`, `/docs/a.md`, an empty address | The resolver decides. |
| `#install` | Kept as written: a place on the same page. |
| `https:`, `http:`, `mailto:`, `//host/…` | Kept as written. |
| Any other scheme: `javascript:`, `data:`, `vbscript:`, `file:` and the rest | Refused: shown as text. |

The address is read the way a browser reads it: spaces and control characters at both ends are dropped, and tabs and line breaks anywhere. So `java&#9;script:` is refused too.

`Resolved::Text` shows the link's text, or the image's alt text, without a link. An address the resolver returns is checked again, so a resolver bug cannot put `javascript:` on a page.

### `DiagramRender`

`draw(&diagram)` gets a `Diagram`: its `language` (`Mermaid` or `D2`), its `code` (line feeds, no last one) and its `block` index, which a drawing can use to keep its ids unique on a page with many diagrams.

| It returns | The page shows |
|---|---|
| `Ok(Some(svg))` | The SVG in a `<figure class="diagram">`. |
| `Ok(None)` | The code, marked as an undrawn diagram, so the page can draw it in the browser. |
| `Err(message)` | The same, with the message in `data-error`. |

The SVG is placed in the page as returned. The program must make it safe: no scripts, no event attributes, and no `javascript:` or `data:` addresses. merman's own guide says its output needs that cleaning (`docs/working/version-1/spikes/mermaid.md`). The program's drawer, `driftmark::diagram::Drawer`, draws Mermaid with merman and D2 with the `d2` program, and cleans every drawing from an allow-list; `docs/features/pages.md` ("Diagrams") says how.

## The reading HTML

### Blocks and `data-k`

Every element that holds a block's text carries `data-k`, the block's index in the document model (`docs/features/engine/document-model.md`). A change list names blocks by the same index, so a page can go from a change to its place.

| Block | Element |
|---|---|
| Heading | `<h1 id="…" data-k="n">` to `<h6>`, with `<a class="anchor" href="#…" aria-label="Link to this section"></a>` after its text. |
| Paragraph | `<p data-k="n">`. In a tight list, `<span data-k="n">`. |
| List item | Its leading paragraphs: `<p data-k="n">`, or `<span data-k="n">` in a tight list. Two paragraphs of one item are two elements with the same `data-k`. |
| Task item | `<li class="task" data-task="open">` or `data-task="done"`, with `<input type="checkbox" disabled aria-label="Not done">` (or `checked aria-label="Done"`) before the text. The box has no label of its own, so it carries its state as a name for screen readers. |
| Table row | `<tr data-k="n">`, its cells `<th>` or `<td>`, with `align="left"`, `"center"` or `"right"` when the column says so. Each `<td>` carries `data-label`, the text of its column's header cell, so a phone can show a row as a card whose values say which column they are. |
| Code | `<pre data-k="n" data-language="x"><code class="language-x">`. Both name the first word of the info string; a block without one has neither. `data-language` lets the page label the block and style program output (`text`, `console`, `output`, `log`) without a script. |
| Diagram, drawn | `<figure class="diagram" data-k="n" data-diagram="mermaid">` holding the SVG. |
| Diagram, not drawn | `<pre data-k="n" data-diagram="mermaid" data-undrawn><code class="language-mermaid">`, with `data-error="…"` when drawing failed. `data-diagram` is `mermaid` or `d2`. |
| Alert | `<aside class="alert" data-alert="warning" aria-label="Warning">`, then `<p class="alert-title" data-gen>` with the title (the written one, or Note, Tip, Important, Warning, Caution), then the leading paragraphs as `<p data-k="n">`. |
| Footnote definition | `<li id="fn:name">` in `<section class="footnotes"><ol>` at the end of the page, its leading paragraphs as `<p data-k="n">`. |
| Block quote | `<blockquote>`. It is not a block; its paragraphs are. |
| Thematic break | `<hr />`. |
| HTML block | Its raw HTML, through the allow-list. No `data-k` (see "Limits"). |
| Front matter, unrendered text | Nothing. The page shows metadata from `Document.meta`. |

Elements with `data-k` never nest, and the elements of one block always sit next to each other in the page.

### Inline elements

| Markdown | HTML |
|---|---|
| Emphasis, strong, strikethrough | `<em>`, `<strong>`, `<del>` |
| Code span | `<code>` |
| Hard line break | `<br>` and a line feed |
| Soft line break | A line feed |
| Link | `<a href="…" title="…">`; shown as text, `<span class="link-off">` |
| Image | `<img src="…" alt="…" title="…">`; shown as text, `<span class="image-off">` with the alt text |
| Footnote reference | `<sup class="footnote-ref"><a href="#fn:name" id="fnref:name">1</a></sup>`; the second reference to the same note gets `id="fnref:name:2"` |
| Footnote back links | At the end of the definition's last paragraph: `<span class="footnote-back" data-gen>`, holding one `<a href="#fnref:name" aria-label="Back to reference 1">↩</a>` per reference |
| Raw HTML that is not allowed | `<code class="html">` with the tag as text |

### Wrappers

- A table is wrapped in `<div class="table" role="region" tabindex="0" aria-label="Table">`, so it scrolls sideways on its own and a keyboard can reach it.

### Ids

- A heading's id is its anchor by GitHub's rules, the same function the lint's `anchor` rule uses (`Anchors::by_block`): lower case, punctuation dropped, spaces as `-`, and `-1`, `-2` for repeats. So a link the lint accepts always finds its heading.
- Every id the page itself adds contains a character no heading anchor has: `fn:name`, `fnref:name`, and the change list's `prefix.n`. A page that adds its own element ids must do the same, for example with a capital letter such as `id="Tables"`. Otherwise a heading named "Tables" could take the id first.
- `reading_prefixed` puts its prefix in front of every id the document's HTML carries: each heading's, which the outline repeats, and each footnote's (`R3-fn:name`, `R3-fnref:name`). It puts it after the `#` of every link to a place on the same page too, from Markdown (`[x](#install)`) and from raw HTML (`<a href="#install">`), so the link still finds this document's own heading. With the prefix `R3-`, two documents with an `## Install` section get `R3-install` and `R4-install`. A drawn diagram keeps the ids its drawer gave it. The position tables do not change. `reading` is `reading_prefixed` with an empty prefix.

### `data-gen`

An element marked `data-gen` holds text the page added, not text of a block: alert titles, footnote back links, and the change list's labels. The page script skips it, and so does the text check.

### Text on the page

A block's text on the page, read by these rules, equals the document model's `text`. The corpus check below proves it for every block.

1. Text inside `data-gen` is skipped.
2. Raw HTML shown as text (`code.html`) and footnote numbers (`sup.footnote-ref`) are markup and are skipped.
3. An image counts as its alt text.
4. The parts of one block are joined: table cells by a tab, paragraphs by an empty line.
5. Runs of white space count as one space, and the ends are trimmed. Code is compared exactly, without its last line feed.

## Raw HTML

Raw HTML in a document passes only through an allow-list.

1. **Which elements.** The rules file's `allowed-html` list (`details`, `summary`, `br`, `sub`, `sup` and `kbd` by default), kept to elements that cannot run code, load a frame, or change how the browser parses what follows. A rules file that names `script`, `style`, `iframe`, `svg`, `math`, `form`, `textarea` or `template` still gets them shown as text. The full list of elements a rules file can allow is `RENDERABLE` in `render/html.rs`.
2. **Inside a paragraph**, only elements that may sit there pass, such as `kbd` or `sub`. A `<details>` there would end the paragraph in the browser, so it is shown as text.
3. **Which attributes.** Only these, and on these elements:

   | Attribute | Elements |
   |---|---|
   | `title` | any allowed element |
   | `alt`, `width`, `height` | `img` |
   | `src` | `img`, through the address rules and the resolver |
   | `href` | `a`, through the address rules and the resolver |
   | `colspan`, `rowspan` | `td`, `th` |
   | `start` | `ol` |
   | `open` | `details` |

   Every other attribute is dropped: `style`, `class`, `id`, `name` (which could replace the page's own elements), and every event handler.
4. **Everything else is escaped.** A tag that is not allowed is shown as text in `<code class="html">`. Text between tags is escaped, keeping character references such as `&amp;`. A NUL byte shows as `�`.
5. **Comments, declarations, processing instructions and CDATA** render nothing.
6. **Raw HTML closes only what it opened.** A closing tag closes an element only when raw HTML in the same paragraph (inline) or the same container (blocks) opened it. Any other closing tag is dropped, so `</li></ul></article>` cannot break the page. Elements still open are closed where their paragraph or container ends.

## The position tables

`PositionTables` holds one list of whole numbers per block, by block index. A block with no element on the page has an empty list. In JSON it is one array: `[[0,0,2,2],[5,0,6,0,0,2,1]]`.

A list starts with the block's first source byte. Then each piece of the block's rendered text adds three or four numbers:

1. the rendered gap since the previous piece;
2. the rendered length times 2, plus 1 when the piece maps only as a whole;
3. the source gap since the previous piece;
4. for a piece that maps only as a whole, its source length.

Rendered lengths count UTF-16 units, as the browser does. Source offsets count UTF-8 bytes. A piece is one of three kinds:

| Kind | What it is | How it maps |
|---|---|---|
| Verbatim | Text that is its source bytes, such as a word | Any offset inside, exactly |
| Atomic | Text from source of another shape: `&amp;`, `\*`, a footnote number, a Windows line ending in code, raw HTML shown as text across lines | Only as a whole |
| Object | An image | As a whole; its rendered length is 0 |

Rendered text that maps to no source, such as a line break, is a rendered gap.

A made-up example. In `# T\n\nab *c* &amp; d![i](x.png)\n`, block 1 starts at byte 5, and its table is `[5, 0,6,0, 0,2,1, 0,2,1, 0,3,0,5, 0,4,0, 0,1,0,11]`: `ab ` (3 units, verbatim), `c` one byte later, a space after the `*`, `&` mapping to all of `&amp;`, ` d`, and the image, an object of 11 bytes.

The design and its tests come from the anchoring spike (`docs/working/version-1/spikes/anchoring.md`, design B). Ranges are tight: a selected bold word maps to `bold`, not to `**bold**` (decision 0008).

`driftmark serve` does not put the tables in the page. It serves them at `/api/positions/<revision>/-/<path>`, cached with the page's HTML, and the notes script loads them only when the reader starts a note, or when the page has notes to highlight (`docs/features/notes.md`).

## The page script

`anchor.js` defines `window.driftmarkAnchor`:

| Call | What it does |
|---|---|
| `setTable(table, root)` | Gives it the page's position tables, parsed from JSON, and the element that holds the page HTML. |
| `toSource(range)` | A DOM `Range` → `{ start, end, exact, why }`, or `null` when the selection holds no text with a source. `why` lists `grapheme`, `atomic`, `unmapped` or `reordered` when the range snapped. |
| `fromSource(start, end)` | A source byte range → `{ range, text, exact }`, or `null`. |
| `highlight(start, end)` | `fromSource`, painted with the CSS Custom Highlight API as `dm-note`; an image inside gets the class `dm-note`. |
| `reset()` | Drops what it worked out, after other code changed the page's text nodes. |

It reads the pieces of each block from every element with that `data-k`, in page order, and skips text inside `data-gen`. A piece that other code split into several text nodes, as highlighting, search marks or word marks do, is cut per node, so the answers stay the same.

## The change HTML

`changes(document, id_prefix)` writes one `<ol class="changes" data-path="…" data-file="added|removed|modified">`. The text comes from the change list (`docs/features/engine/change-list.md`), so it is plain text: the CSS must keep its line feeds and tabs (`white-space: pre-wrap`).

### One element per change

Each change about one block is `<li id="prefix.n" class="change">`, where `n` is its index in the change list. Its attributes:

| Attribute | What it holds |
|---|---|
| `data-kind` | `formatting`, `edited`, `added`, `removed`, `moved` or `moved_edited` |
| `data-level` | `decision`, `rule`, `fact`, `wording` or `formatting` |
| `data-signals` | The signals' JSON names, separated by spaces, so CSS can select `[data-signals~=number]` |
| `data-old`, `data-new` | The block index on each side (`old.at`, `new.at`) |
| `data-old-line`, `data-new-line` | The line the block starts on, on each side that has it |
| `data-view` | For an edit: `inline`, `side_by_side`, `sentences`, `rewritten` or `lines` |
| `data-fallback` | The marks failed the guard, so both versions are shown whole |
| `data-cells` | For a table row, the changed cells' positions, separated by spaces |
| `data-block` | The block kind's JSON `type`, such as `paragraph` or `list_item` |
| `data-heading`, `data-depth`, `data-number`, `data-task`, `data-header`, `data-language`, `data-diagram`, `data-alert`, `data-footnote` | The block kind's details, from the new side (the old one for a removed block) |
| `data-old-…` | A detail the old side had differently, such as `data-old-heading="2"` or `data-old-task="open"`; empty when the old side did not have it |

Inside it come, in order:

1. The head, `<div class="head" data-gen>`, which says in words what the attributes say:
   - `<span class="level" data-level="decision">Decision</span>`;
   - `<span class="kind">Edited</span>`: what happened (`words::kind`);
   - for an edit, `<span class="view">Side by side</span>`, or `Both versions whole` when the marks failed the guard;
   - `<span class="why">in a decision section · a number or a name changed</span>`: every signal, in words, joined by ` · `;
   - `<span class="acts">` with two inactive buttons, `<button type="button" class="seen" disabled title="Arrives with notes">Seen</button>` and `class="note"` "Add a note". Notes and the seen mark arrive together in M3 (decision 0004); a page that does not offer them hides `.acts`.
2. The block's heading path, as `<ol class="path"><li>Cache rendered pages</li><li>Decision</li></ol>`.
3. The content.

### The content of each view

Word marks are `<del>` for text only the old version has and `<ins>` for text only the new one has. Kept text is plain.

| Change | Content |
|---|---|
| Inline | `<div class="side" data-side="both">` with every mark |
| Side by side | `<div class="side" data-side="old">` with kept and deleted marks, then `data-side="new"` with kept and inserted marks |
| Sentence rows | `<div class="rows">`, then one `<div class="row" data-row="kept|edited|deleted|inserted">` per sentence: a kept row has one `data-side="both"`, an edited row an old and a new side, a deleted row only the old side, an inserted row only the new side |
| Rewritten, or a fallback | The old side whole and the new side whole, without marks |
| Lines (code and diagrams) | `<pre class="lines"><code class="language-x">`, one line per row: kept lines plain, `<del>` and `<ins>` lines marked, each followed by a line feed |
| Added block | `data-side="new"` with the whole text in `<ins>`; code inside `<pre><code>` |
| Removed block | `data-side="old"` with the whole text in `<del>` |
| Moved, formatting only | `data-side="new"` with the text |

### Moves

A moved block is listed where it arrived. It also gets a place-holder where it left, so both places link to each other:

- the moved change holds `<p class="moved"><a href="#prefix.n.from" data-gen>Moved here from old line 12</a></p>`;
- the place-holder is `<li id="prefix.n.from" class="moved-from" data-new="4"><a href="#prefix.n" data-gen>Moved to new line 40</a></li>`. It stands where the block used to be, in the new version's reading order: just after the last block before it that stayed in place, as a removed block is listed.

### Formatting-only changes

A run of formatting-only changes is folded in one `<li class="fold" data-count="3"><details><summary data-gen>3 formatting-only changes</summary><ol class="changes">…</ol></details></li>`. `<details>` folds without a script.

### A new or deleted document

A new or deleted document is one item (decision 0009), from the change's `document` (`docs/features/engine/change-list.md`, schema version 2): `<li id="prefix.n" class="change document" data-kind="added|removed" data-level="…" data-signals="…" data-bytes="2340" data-lines="61">`. Inside it come the same head as a block change (its kind reads `New document` or `Deleted document`), then `<p class="title">` when the document has a title, `<p class="summary">` when it has a summary, and `<p class="size" data-gen>61 lines</p>`. Its blocks are not listed, and its text is not shown: a reader opens the reading page for that.

## Size limits

Two checks run in `Document::parse` before comrak sees a file, so the diff, the lint and the pages are all protected.

| Limit | Value | Error | Why |
|---|---|---|---|
| File size | 2 MiB (`MAX_FILE_SIZE`, 2,097,152 bytes) | `TooLarge { size, limit }` | The parser's time and memory grow with the file. A hostile 2 MiB file of nested list markers takes about half a second and 280 MB; 4 MiB takes twice that. The largest file in a 3,585-file private corpus has 479,045 bytes, so the limit leaves more than four times that. |
| Quote markers at the start of one line | 100 (`MAX_DEPTH`) | `TooDeep { line, limit }` | The parser's time grows with the square of the `>` markers one line opens: 20,000 take 0.2 s and 50,000 take 2 s. A line with more than 100 would nest deeper than the block list allows anyway. The deepest line in the corpus has 7. |

Measured on the server on 2026-10-18 with comrak 0.56, single runs at a load of 20 to 37. A line inside a code block that starts with more than 100 `>` signs is refused too, because the check runs before parsing.

## Code highlighting

The core does not highlight code; it marks each block with its language class. Highlighting in the browser is the recommendation, measured on 2026-10-18:

| | comrak's syntect adapter, on the server | Prism 1.30.0, in the browser |
|---|---|---|
| Size | The program grows by 1.1 MB (Oniguruma regex) or 2.3 MB (pure-Rust regex) | 15.7 KB gzipped for its core and the 15 languages the private corpus uses, loaded once and cached |
| Time on the private corpus (3,424 pages) | 3.5 s (Oniguruma) or 7.2 s (pure-Rust), against 0.63 s without; the slowest page 0.3 to 1 s | Only on pages that hold code, in the reader's browser |
| HTML | 38% more (88.5 MB against 64.0 MB for the corpus) | No change |
| Languages | No TypeScript, TSX, GraphQL, PromQL, LogQL, HCL or JSONC in its built-in set: about a fifth of the corpus's labelled blocks | TypeScript, TSX, GraphQL, PromQL and HCL; not LogQL |
| Licences | Within `deny.toml`; Oniguruma needs a C compiler to build | MIT |

Highlighting splits text into more nodes. The position tables survive that: the 113 anchoring cases also run after every word was wrapped in its own element.

## Tests

- `fixtures/render/reading/`: one made-up document per subject (block kinds, alerts, diagrams with and without a drawing, links, headings, raw HTML, footnotes, lists, tables, code, Windows line endings), each with `<name>.html` and `<name>.positions.json`. `crates/driftmark-core/tests/render_fixtures.rs` checks them, and that each page's text matches its document model.
- `fixtures/render/hostile/`: documents that try scripts, event attributes, refused addresses, attribute break-outs, element clobbering and parser tricks, rendered with a rules file that allows dangerous elements. The test checks every tag of every page against the allowed elements and attributes, and every address against the address rules.
- `fixtures/render/changes/<case>.html`: the change HTML of every pair in `fixtures/diff/` (one per change kind, view and signal) and of the pairs in `fixtures/render/changes/`. `crates/driftmark-core/tests/render_changes.rs` also checks every change's attributes against the change list's JSON.
- `fixtures/render/anchoring/`: the 30 documents and 113 cases of the anchoring spike. The Rust test writes them as pages; `tests/browser/run.mjs` at the repository root runs the cases in headless Chromium with the shipped `anchor.js`, on each page as written and after every word was wrapped in its own element. Run `cargo test -p driftmark-core --test render_fixtures`, then, in `tests/browser/`, `npm ci` and `node run.mjs <target>/tmp/anchoring-pages ../../fixtures/render/anchoring <results.json>`, where `<target>` is the cargo build folder.
- `crates/driftmark/tests/render_corpus.rs` (ignored) renders every document of a real repo and checks its text; with `DRIFTMARK_PAGES` it writes the pages for `tests/browser/fuzz.mjs`, which maps random selections there and back.

## On the private corpus

Measured on 2026-10-18 on the 3,424 Markdown files under `docs/` of the private corpus, with the release build. The branch gained a file during the day; a last run on its 3,425 files gave the same results.

| Check | Result |
|---|---|
| Documents that render | 3,424 of 3,424 (later 3,425 of 3,425); none refused by the size limits |
| Blocks whose page text matches the model | 275,182 of 275,182 (later 275,429 of 275,429) |
| Random selections in Chromium, 5 per page, on each page as written and with every word split | 34,240 |
| Of those: mapped exactly, snapped, whitespace only | 34,096, 2, 140 |
| No range although the selection held text | 2: one selection, twice, inside the text of a raw HTML block |
| Round trip failed, source edge wrong, or splitting the text changed the answer | 0, 0, 0 |

| Per page | Median | Largest |
|---|---|---|
| Parse, at a load of 10 on 12 cores | 0.18 ms | 8.6 ms |
| Render HTML and tables, at the same load | 0.23 ms | 11.6 ms |
| HTML | 14.3 KB | 671 KB |
| Position tables, as JSON | 1.9 KB | 95 KB |
| Building the pieces in the browser, on the first selection | 0.8 ms | 23 ms |

## Limits

- **Raw HTML text is not mapped.** An HTML block has no `data-k`, because its tags can open in one block and close in another, as `<details>` does around Markdown. A selection inside its text gets no source range. On the corpus that was 2 of 34,240 selections.
- **Change pages have no position tables.** Deleted words inside a block are not in the new version's source. A note on a change card sends the selected text instead, without the `<del>` text on the new side and the `<ins>` text on the old side, and the server finds it in the block's source (`docs/features/engine/notes-model.md`, "Text selected on a change page").
- **A drawn diagram's text cannot carry a note.** The SVG replaces the code; a note on a diagram anchors to its whole block.
- **Footnotes render at the end.** A selection from a paragraph into the footnote list covers two stretches of source and reports `reordered`.
- **Tables in the change HTML are plain text**, cells separated by tabs.
- **Only Chromium was tested**, as in the spike.
