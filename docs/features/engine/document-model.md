---
status: target
date: 2026-10-18
---

# Document model

> Short version: `driftmark-core` turns one Markdown file into a `Document`: its text, a flat list of blocks in source order, and its metadata. A block is one paragraph, heading, list item, table row, code block, diagram, alert, footnote, HTML block or rule. Each block carries its kind, its exact source bytes, the headings and containers it sits in, its plain text, and two hashes. Equal content hashes with different source hashes mean the change was formatting only. The change view, the pages, notes and the status view all build on this model.

## How a file is parsed

`Document::parse_with(bytes, keys)` in `driftmark_core::document` takes the file's bytes and the known masthead keys, and returns a `Document` or a `ParseError`. The core reads no files; the program hands it the bytes. With a rules file, the keys are `rules.masthead_keys()` (`docs/features/engine/rules-file.md`). `Document::parse(bytes)` uses the built-in keys alone.

- **Parser.** comrak 0.56, with the options in `document::markdown::options()`. Every part of driftmark must parse with these options, so blocks, pages and note positions come from the same tree.
- **Extensions on:** tables, strikethrough, autolinks, task lists, footnotes, GitHub alerts, and YAML front matter between `---` lines at the top of the file.
- **Line endings.** A line feed, a Windows line ending (`\r\n`) and a bare carriage return each end a line, as in comrak.
- **A byte order mark** at the start of the file is kept in the text and belongs to no block.

These cases give an error value, never a panic:

| Error | When |
|---|---|
| `TooLarge { size, limit }` | The file has more than 2 MiB (`MAX_FILE_SIZE`). It is checked first, before the parser sees the file. `docs/features/engine/html.md` says why the limit is where it is. |
| `NotUtf8 { offset, line }` | The bytes are not UTF-8. `offset` is the first bad byte; `line` is its 1-based line. |
| `TooDeep { line, limit }` | A block would sit inside more than 100 block quotes, list items, alerts and footnotes (`MAX_DEPTH`). Real documents nest a handful deep. The limit keeps a hostile file from making every block carry thousands of containers. A line that starts with more than 100 `>` quote markers is refused before the parser sees the file, because the parser's time grows with the square of them. |
| `BadPosition { line, column }` | comrak reported a position outside the file or inside a character. That would be a comrak bug, not a fault of the file. The parse refuses it rather than return a range that does not slice. |
| `Internal { message }` | comrak panicked. The panic is caught, so one bad file cannot stop a server. |

Front matter that is not valid YAML is not an error. The document still parses, and `meta.front_matter_error` says what is wrong and on which line of the file.

## The document

| Field | What it holds |
|---|---|
| `source` | The file's text. Every block's range indexes into it. |
| `blocks` | The blocks, in source order. Their ranges never overlap. |
| `meta` | The title, the summary and the key-value fields. |

`document.source_of(block)` returns a block's source text.

## How a file splits into blocks

1. **Leaves are blocks.** Front matter, each heading, each paragraph, each code block, each diagram, each HTML block and each thematic break (`---`, `***`) is one block.
2. **A table is one block per row.** The header row is a block too. Its range runs on to the end of the delimiter row under it (`|---|---|`), so a change of column alignment still lands in a block.
3. **A list item is one block.** It holds the item's marker, its checkbox and its leading paragraphs. Everything after them is a block of its own inside the item: a nested list's items, a code block, a table, a later paragraph.
4. **Alerts and footnote definitions work like list items.** One block holds the marker (`> [!NOTE]`, `[^1]:`) and the leading paragraphs. Later lists or code inside them are their own blocks.
5. **A block quote is not a block.** Its paragraphs, list items and code blocks are, with the quote in their container path.
6. **Text that renders nothing still lands in a block.** Link reference definitions (`[guide]: https://…`) and footnote definitions that nothing refers to make no node in comrak's tree. driftmark gives each stretch of them an `Unrendered` block, so an edit there still shows as a change.
7. **Source order.** The page shows footnote definitions at the end, and comrak moves them there in its tree. The block list keeps them where they are written. So blocks are sorted by their start byte.
8. **Nothing is lost.** Every character that is in no block is white space, a `>` quote marker, or a byte order mark.

An item with no leading paragraph, such as one that starts with a code block, still gets its block. That block holds only the marker, such as `-` or `1.`.

### A made-up example

```markdown
---
status: accepted
---

# Cache pages

> Short version: the server keeps each rendered page for one hour, so a second visit costs nothing.

## Decision

Keep each page for **one hour**.

- Pages expire after
  one hour.
  - A page with notes expires sooner.

> [!WARNING]
> Clear the cache after a deploy.

| Page | Time |
|---|---|
| Home | 10 min |

[guide]: https://example.com/guide
```

| # | Kind | Source | Containers | Parent | Plain text |
|---|---|---|---|---|---|
| 0 | front matter | the three lines from `---` to `---` | | | `status: accepted` |
| 1 | heading, level 1 | `# Cache pages` | | | `Cache pages` |
| 2 | paragraph | `Short version: …` (after the `> `) | block quote | | `Short version: the server …` |
| 3 | heading, level 2 | `## Decision` | | | `Decision` |
| 4 | paragraph | `Keep each page for **one hour**.` | | | `Keep each page for one hour.` |
| 5 | list item, depth 0 | `- Pages expire after` and `  one hour.` | | | `Pages expire after one hour.` |
| 6 | list item, depth 1 | `- A page with notes expires sooner.` | list item | 5 | `A page with notes expires sooner.` |
| 7 | alert, warning | `> [!WARNING]` and `> Clear the cache after a deploy.` | | | `Clear the cache after a deploy.` |
| 8 | table row, header | `\| Page \| Time \|` and `\|---\|---\|` | | | `Page` tab `Time` |
| 9 | table row | `\| Home \| 10 min \|` | | | `Home` tab `10 min` |
| 10 | unrendered | `[guide]: https://example.com/guide` | | | the same text |

Blocks 4 to 10 have the heading path `["Cache pages", "Decision"]`. The title is `Cache pages`, the summary is block 2's text, and the front matter gives one field, `status`.

## What a block holds

| Field | Type | What it holds |
|---|---|---|
| `kind` | `BlockKind` | What the block is, with the details later parts need. See the next section. |
| `range` | `Range<usize>` | The block's source bytes in `Document.source`, end exclusive. |
| `heading_path` | `Arc<[String]>` | The titles of the headings above the block, outermost first. |
| `containers` | `Arc<[Container]>` | The containers the block sits inside, outermost first. |
| `parent` | `Option<usize>` | The index of the block of the innermost list item, alert or footnote definition the block sits inside. |
| `text` | `String` | The plain text: the rendered text without markup, for comparing words. |
| `spans` | `Vec<TextSpan>` | The stretches of `text` that came from links, images and code spans. |
| `source_hash` | `u64` | A hash of the exact bytes in `range`. |
| `content_hash` | `u64` | A hash of the content with formatting removed. |

### `range`

- The range starts at the block's first byte: a list item's marker, an alert's `>`, a footnote's `[^`, a header row's first `|`.
- It ends after the block's last character that is not blank. It never ends with a space, a tab or a line ending.
- It always falls on character boundaries, so `&source[range]` never fails. This was checked on all 3,416 files of a private 54 MB corpus.
- A block that spans lines includes the container markers of its later lines. A paragraph in a quote that wraps onto a second line includes that line's `> `.
- An indented code block starts at its code, after the four spaces of indentation.

### `heading_path`

- It holds the plain text of each heading, as in `["Cache pages", "Decision"]`.
- Only headings outside every container count. A heading inside a quote, a list item, an alert or a footnote is a block, but it opens no section.
- A heading closes the open headings of its own level and deeper ones. Its own path holds only the headings above it. `## B` after `# A` and `## Old` has the path `["A"]`.
- Levels may skip. `### C` straight after `# A` has the path `["A"]`, and blocks under it have `["A", "C"]`.
- Blocks in the same section share one allocation, so the path costs little memory.

### `containers` and `parent`

`Container` has four values: `BlockQuote`, `ListItem`, `Alert` and `FootnoteDefinition`. There is one `ListItem` entry per level of list nesting. A list item's depth is the number of `ListItem` entries above it.

Block quotes have no block of their own, so `parent` never points at one. `parent` always points at an earlier block. `Unrendered` blocks have no containers and no parent, even when they sit inside a quote.

### `kind`

| `BlockKind` | Details |
|---|---|
| `FrontMatter` | None. |
| `Heading { level }` | 1 to 6. An underlined heading is level 1 (`===`) or 2 (`---`). |
| `Paragraph` | None. Only paragraphs that are not the leading text of a list item, alert or footnote are paragraph blocks. |
| `ListItem(ListItem { depth, ordered, number, task })` | `depth`: 0 for a top-level item. `ordered`: the list is numbered. `number`: the number the page shows, which is the list's first number plus the item's position; `None` in a bullet list. `task`: `None`, `Some(Open)` for `[ ]`, `Some(Done)` for `[x]` or `[X]`. |
| `TableRow { header, cells }` | `header`: whether it is the header row. `cells`: each cell's plain text, without markup or padding. |
| `Code { language, closed }` | `language`: the first word of the fence's info string, such as `rust`; `None` for an indented block or a bare fence. `closed`: whether a closing fence ends it. Indented blocks always count as closed. |
| `Diagram { language, closed }` | A fenced block whose info string's first word is exactly `mermaid` or `d2`, in lower case. GitHub draws only lower-case `mermaid`, so driftmark matches it the same way. `language` is `Mermaid` or `D2`. |
| `Alert { kind, title }` | `kind`: `Note`, `Tip`, `Important`, `Warning` or `Caution`. `title`: the text after the marker, as in `> [!NOTE] Heads up`. |
| `FootnoteDefinition { name }` | The label between `[^` and `]`. |
| `Html` | None. An HTML comment is an HTML block too. |
| `ThematicBreak` | None. |
| `Unrendered` | None. |

### `text`

The plain text is what a reader sees, written without markup.

- Text, code spans and the alt text of images count. An image counts as its alt text, so a paragraph holding only an image still has words to compare.
- Emphasis markers, link targets, raw HTML tags, footnote numbers, task checkboxes and list markers do not count.
- A soft line break becomes a space. A hard line break becomes a line feed.
- The leading paragraphs of a list item, an alert or a footnote are joined by an empty line (`"\n\n"`).
- A table row's cells are joined by a tab, as a browser copies a table row.
- Code and diagrams: the code as written, with line feeds for every line ending and without the final one.
- HTML: the block with its tags and comments removed. Entities such as `&amp;` stay as written.
- Front matter: the YAML between the two `---` lines.
- Unrendered: the source text.
- A thematic break has no text.
- Text has no white space at either end.
- Repair of a comrak 0.56 bug: comrak keeps the carriage return of a Windows line ending inside a code span (`` `a\r\nb` `` comes out as `a\r b`). driftmark turns it into one space, as Markdown requires.

### `spans`

The plain text drops link targets and code-span backticks. The change view needs both: it keeps a link or a code span whole when it marks changed words, and it compares link targets. So each block lists them.

- A `TextSpan` has a `range`, the bytes of the block's `text` it covers, and a `kind`: `Code`, `Link { target }` or `Image { target }`.
- A reference link's target is the one its definition gives. An autolink is a link whose text is its address. An image's text is its alt text.
- Spans come in the order they end, so a code span inside a link comes before the link.
- A link without text has an empty range; it still counts for the target.
- Table cells and the paragraphs of a list item count from the start of the block's whole `text`, tabs and empty lines included.
- Only paragraphs, headings, list items, alerts, footnote definitions and table rows have spans. Most blocks have none.

In ``Use [the guide](a.md) or `cargo doc`.``, the text is `Use the guide or cargo doc.`, and the spans are `4..13` for the link to `a.md` and `17..26` for the code span.

### `source_hash` and `content_hash`

Both are 64-bit XXH3 hashes with seed 0. XXH3 has a published specification, so the values are the same on every system. A cache that keeps hashes across driftmark versions must still key on the version, because a later version may change the content rules below.

The **source hash** covers the exact bytes in `range`.

The **content hash** covers the block's kind and its content after formatting is removed. Neither hash covers the heading path, the containers, the list depth or the item number. The change view compares those on their own.

Two blocks with the same source bytes and the same kind have the same content hash, with one exception. A reference link such as `[guide][g]` takes its target from a definition elsewhere in the file. Editing that definition changes the content hash of every block that uses it, while their source hashes stay the same. The change view can then show the link change where the link is.

| Changes that keep the content hash (formatting) | Example |
|---|---|
| Line wrapping and runs of spaces | a paragraph on three lines and on one |
| Emphasis and strong emphasis, in any style | `*word*`, `_word_`, `**word**` and `word` |
| Code-span backticks | `` `cargo` `` and `cargo` |
| Hard line breaks | `line  ` + line feed, and a soft break |
| Escapes and entities that show the same character | `Tom &amp; Jerry` and `Tom & Jerry`; `\*` and `&#42;` |
| Autolink style | `<https://example.com>` and `https://example.com` |
| Table padding, outer pipes and column alignment | `\| a \|` and `a` |
| List marker style and numbering | `-`, `*`, `+`; `1.` and `1)`; `1. 1. 1.` and `1. 2. 3.` |
| Task checkbox letter | `[x]` and `[X]` |
| Heading style | `# Title` and `Title` underlined with `===`; closing `#` signs |
| Code fence style and indentation | ```` ``` ````, `~~~~` or four spaces |
| Line endings, and spaces at line ends, in code, HTML and front matter | `\r\n` and `\n` |
| Blank lines at the start or end of a code block | |
| Quote continuation | `> a` + `b` and `> a` + `> b` |
| Thematic break style | `---`, `***` and `* * *` |

| Changes that alter the content hash (edits) | Example |
|---|---|
| Any word | `blocks` and `pages` |
| Space between words | `Some text` and `Sometext` |
| A link's target, its title, or which words it covers | `[guide](a.md)` and `[guide](b.md)` |
| Adding or removing a link or an image | |
| An image's alt text | |
| Strikethrough | `~~now~~` |
| Inline HTML | `<kbd>Ctrl</kbd>` |
| A footnote reference's label | `[^a]` and `[^b]` |
| A heading's level | `##` and `###` |
| A task's state | `[ ]` and `[x]` |
| Bullet list and numbered list | `- one` and `1. one` |
| A table cell, or the number of cells | |
| Code, including indentation inside it | |
| A code fence's whole info string | `sh` and `bash` |
| An alert's kind or title | |
| HTML blocks, front matter and unrendered text | |

The fixtures in `fixtures/formatting-only/` and `fixtures/edited/` hold one made-up pair for each row.

## Metadata

| Field | What it holds |
|---|---|
| `title` | The plain text of the first level-1 heading (`#` or `===`) outside any container. `None` when there is none. |
| `summary` | The plain text of the quote right under the title, when it is plain prose. |
| `fields` | The front matter's top-level keys, then the masthead's known keys, in written order. Every front matter key is kept. A masthead key is kept only when it is known (see below). |
| `front_matter_error` | Why the front matter could not be read, as a sentence. |

### Front matter

- YAML between `---` lines at the very top of the file.
- Each value is a `MetaValue`: `Text`, `List` or `Map`. Every scalar stays text as written: `007`, `1.10` and `2026-10-18` keep their spelling, and `true` stays `"true"`. An empty value is empty text.
- Only the first YAML document counts.
- Aliases are copied, but the whole front matter may hold at most 10,000 values with copies counted. This stops a "billion laughs" file from using all memory.
- These make `front_matter_error` say what is wrong, and give no front matter fields: YAML that does not parse (with the file's line), a top level that is not keys and values, a key that is not text, too many values.

### The masthead and the summary

The masthead is a block of `Key: Value` lines right under the title. driftmark reads it in three shapes:

```markdown
> Status: Accepted | Date: 2026-01-05 · Superseded by: [0009](0009-new.md)

**Status**: Draft
**Created**: 2026-03-01

* Status: accepted
* Deciders: Ana, Ben
```

The quote is the brief's own form. The paragraph is how GitHub Spec Kit writes it, and the list is how MADR 2 writes it, so presets for those layouts can name masthead keys. In the paragraph, `Created` is not a known key, so only `Status` is read unless a rules file adds `Created`. In the list, the same holds for `Deciders`.

The rules:

1. Up to three blocks right under the title are read. Reading stops at the first block that is neither a masthead nor the summary.
2. A line splits into fields at ` | ` and ` · `. Each piece is `Key: Value`, split at the first colon. A piece that is not `Key: Value` belongs to the value before it, so `Affects: a | b` keeps `a | b`. The first piece of a line must be `Key: Value`.
3. A key starts with a letter, has one to four words and at most 40 characters, and holds only letters, digits, spaces and `-_/.'()`. Bold markers do not matter, because the rendered text is read. A value that starts with `//` means the colon was part of a web address, so the piece is not a field.
4. Every line of the block must be `Key: Value`, or the block is not a masthead.
5. **Only known keys are metadata** (decision 0009). The known keys are `Status`, `Date`, `Supersedes`, `Superseded by`, `Owner`, `Type`, `Tags` and `Version`, in `MASTHEAD_KEYS`, plus the keys the rules file adds. They match without regard to case, with `-`, `_` and spaces the same. A field with any other key is left out. A block with no known key is not a masthead, whatever the length of its values.
6. The summary is the first quote under the title that holds only paragraphs and is not a masthead. Its paragraphs are joined by an empty line.
7. A masthead value is its rendered text. The targets of links in it are kept in `field.links`, so "superseded by" can link to the newer document.

So a summary written as `Label: value` stays the summary, however short, when its label is not a known key:

```markdown
# Cache pages

> Short version: pages expire after an hour.

> Status: Accepted | Deciders: Ana
```

Here the summary is `Short version: pages expire after an hour.`, and the masthead gives one field, `Status`. The fixture `fixtures/masthead/short-summary.md` holds this example. `Deciders` is not a known key, so it is left out. A rules file that lists `Deciders` in its `[masthead]` table keeps it too. Before decision 0009, a single `Label: value` of 12 words or fewer counted as a masthead field, which turned short `> Short version: …` quotes into metadata.

`MastheadKeys` holds the known keys. `MastheadKeys::default()` is the built-in list, and `MastheadKeys::with(extra)` adds more; `keys.knows(key)` tests one.

### Looking up a field

`meta.field(origin, key)` finds the first field from the front matter or the masthead with that key. Keys match without regard to case, and `-`, `_` and spaces count as the same. So `superseded-by` finds `superseded_by` and `Superseded by`.

The status, the date and "superseded by" are not read here. Where they live depends on the document's type, which the rules file decides. `docs/features/engine/rules-file.md` says how.

## Sections and the outline

`Outline::of(&document)` in `driftmark_core::document::outline` lists the document's sections, in reading order. A tool that reads part of a long document asks for one section instead of the whole text: the MCP connector's `document_read` does (`docs/features/commands/mcp.md`).

- **A section** starts at a heading outside every container. It holds that heading and every block after it, up to the next heading outside every container whose level is the same or higher. So sections nest: a `##` section holds the `###` sections under it.
- **A heading inside a container** (a quote, a list item, an alert or a footnote) opens no section, as in `heading_path`.
- **Blocks before the first heading**, such as front matter, belong to no section.

Each `Section` holds:

| Field | What it holds |
|---|---|
| `block` | The heading's block index. |
| `level` | The heading's level, 1 to 6. |
| `title` | The heading's plain text. |
| `path` | The `heading_path` of the heading, then its own title. |
| `anchor` | The heading's anchor, as GitHub and the reading page make it, without `#`. |
| `blocks` | The block indexes it covers, end exclusive. |
| `bytes` | Its source bytes: from the heading's first byte up to the next heading that ends it, without the white space before that heading. |

`outline.containing(block)` gives the innermost section that holds a block. `outline.find(anchor)` gives the section a heading's anchor names: the heading's own section, or, for a heading inside a container, the section around it. An anchor in another case finds its heading when no heading has it exactly.

In the example above, the outline holds `Cache pages` (blocks 1 to 10, the whole document after the front matter) and `Decision` (blocks 3 to 10, anchor `decision`).

## The comrak layer, for pages and notes

`driftmark_core::document::markdown` holds what M2 needs to map rendered text back to source bytes:

- `options()`: the parser options.
- `LineIndex`: turns comrak's line and column positions into byte offsets, with the same line endings as comrak.
- `Positions::range(node)`: the source bytes of any comrak node. It repairs a comrak 0.56 bug. comrak removes the backslash of `\|` in a table cell before it reads the cell, then reports the cell's inline positions as if no byte was removed. Every inline after an escaped pipe would start one byte early per pipe. Block positions, cells included, are right and need no repair.
- `code_span_text(literal)`: a code span's text with the Windows line ending repaired.

Both repairs come from the anchoring spike. Tests check that comrak still has each bug, so the repair can go when comrak fixes it.

## Speed and size on the private corpus

All 3,416 Markdown files of the private corpus (54.5 MB) parse with no error. Parsing them one after another takes 0.75 seconds on one core of the server; comrak alone takes 0.43 of that. They hold 282,031 blocks:

| Kind | Blocks |
|---|---|
| List item | 110,370 |
| Paragraph | 64,920 |
| Table row | 48,763 |
| Heading | 43,545 |
| Code | 6,919 |
| Thematic break | 6,907 |
| HTML | 398 |
| Diagram | 195 |
| Front matter | 13 |
| Unrendered | 1 |

No range fails to slice, no two blocks overlap, and no character outside a block is anything but white space, a quote marker or a byte order mark.

## Not covered

- Rendering HTML and the per-block position tables for notes are in `docs/features/engine/html.md`.
- Comparing two documents comes in wave 2.
- Blocks carry byte ranges, not line numbers. `LineIndex::line_of` gives the line of a byte.
- Footnote numbers are not in the plain text, because renumbering would read as an edit.
- A block itself has no JSON form. Its kind has one, which the change list uses: an object whose `type` names the kind in snake case, with the kind's details beside it, as in `{"type": "heading", "level": 2}`. `docs/features/engine/change-list.md` shows it.
