---
status: done
date: 2026-10-18
---

# Spike: anchoring notes to the Markdown source

> Short version: yes, both directions work. A text selection on a rendered page maps to an exact byte range in the Markdown source, and a source byte range maps back to a highlight on the page. Both designs passed all 113 test cases in headless Chromium. On the 3,416-file private corpus, 409,920 random selections mapped there and back with no error, and the two designs gave the same answer every time. The recommendation is design B: one offset table per block, sent beside the HTML. Before compression it adds 28% to a page's HTML and design A adds 52%; after gzip both add about a quarter. Its table can also load only when the reader starts a note. comrak needs two small workarounds, and its licence needs a line in `deny.toml`.

## Facts in the brief that turned out different

1. **No Playwright release matches the cached Chromium.** The cache holds Chromium revision 1247. Playwright 1.63.0 expects 1243, and 1.64.0 and 1.64.1 expect 1248. The harness uses `playwright` 1.64.1 and points it at the cached `chromium_headless_shell-1247` with `executablePath`. That worked for every run.
2. **Image alt text cannot be selected.** The alt text is an attribute of the `<img>` element, not text on the page. The image cases select the whole image element instead. It maps to the whole image source, such as `![alt](cat.png)`.
3. **comrak's licence is BSD-2-Clause.** `deny.toml` allows only MIT and Apache-2.0. The first build lane that adds comrak to the workspace needs a licence line there. See "Needs the maintainer's decision".
4. **The corpus count.** The brief's example said 3,414 Markdown files. The corpus had 3,416 on 2026-10-17.

## What was built

All code is in `spikes/anchoring/`, a Cargo package outside the workspace.

- **The map** (`src/lib.rs`, `map_document`). It parses with comrak 0.56.0, the latest release, with tables, strikethrough, autolinks, task lists, footnotes, alerts and front matter turned on. For every text-bearing node it builds a list of pieces. A piece ties a stretch of rendered text to a source byte range. There are three kinds:
  - verbatim: the rendered text is the source bytes, character for character, so any offset inside maps exactly;
  - atomic: the rendered text came from source of another shape (an entity such as `&amp;`, an escape such as `\*`, a footnote number), so it maps only as a whole;
  - object: an element with no text (an image), which maps as a whole.
- **The renderer** (`render`). It uses comrak's own HTML writer and replaces only the parts that carry text. With no map turned on, its output equals comrak's HTML byte for byte on all 3,416 corpus files. It writes:
  - design A: every text-bearing inline wrapped in `<span data-s="start">`, with `data-e="end"` added for atomic pieces, and the same two attributes on images and footnote links;
  - design B: plain comrak HTML, a `data-k="n"` marker on each block that holds text (paragraph, heading, table cell, code block, tight list item), and one table per block in a `<script type="application/json">` element.
- **The page script** (`web/anchor.js`, 14.4 KB, 4.4 KB gzipped). It serves both designs. `toSource(range)` turns a DOM Range into a byte range. `fromSource(start, end)` turns a byte range into a DOM Range and paints it with the CSS Custom Highlight API.
- **The fixtures** (`fixtures/`). There are 30 made-up Markdown files and `cases.json` with 113 cases. `.gitattributes` there keeps the CRLF and tab bytes exact.
- **The harness** (`harness/`). `run.mjs` runs the cases in headless Chromium. `fuzz.mjs` runs random selections on the corpus pages. `sizes.mjs` and `timing.mjs` measure.

### What "exact" means

The source range of a selection runs from the source start of the first selected character to the source end of the last selected character. Markup inside the selection is inside the range. Markup at the edges stays outside it. Selecting the bold word in `some **bold** text` gives `bold`, not `**bold**`. Whitespace at the edges of a selection is dropped first.

### How each case is checked

1. The harness finds the case's text on the page and builds a Range with the Range API. It puts the Range into the page's selection and reads it back.
2. Seven cases use the mouse instead: double-click on a word, triple-click on a paragraph, a list item and a table cell, and drags across blocks and across inline nodes.
3. `toSource` runs. Its byte range must equal the expected range. The expected range is found by searching the fixture file's bytes for the expected source text, so it does not depend on the map.
4. `toSource` also reports whether the result is exact. It must say "not exact" exactly for the 8 cases that are meant to snap.
5. The reverse direction then highlights the expected source range. The highlighted text must equal the selected text, ignoring whitespace. The number of selected images must match too.
6. Six more cases test only the reverse direction, starting from a source range.
7. Every case runs four times: design A, design B, and each of them again after a script wrapped every word on the page in its own `<mark>` element. The text stays the same; only the nodes change.

## Results per case

All 113 cases pass in design A and in design B. After every word is wrapped in its own element, design B still passes all 113 and design A passes 25. Before that result, four things were fixed:

- three expected ranges in `cases.json` included a closing `*`, which the rule above leaves out;
- inline positions inside table cells with an escaped pipe were wrong in comrak (see "comrak problems found"); with the repair turned off, 8 table cases fail in both designs;
- the code-span aligner did not yet skip the list indent on a wrapped line (found on the corpus, then added as fixture 29);
- a selection from a paragraph into the footnote list, whose source runs backwards, gave a different range once the text was split; the script now covers every selected piece (fixture 30).

To check that the harness can fail, three deliberate breakages were run. Counting every character as one byte failed 11 cases. Turning off grapheme snapping failed 3. Treating atomic pieces as verbatim failed 6.

"Selection" is the rendered text the case selects. "Maps to" is the source text the selection must map to; a reverse-only case lists the source range and the text the highlight must show. "(snaps)" marks a case that must report a snap. In the table, `\n`, `\r`, `\t` and `\u00a0` stand for a line feed, a carriage return, a tab and a no-break space.

| # | Case | Fixture | Selection | Maps to | A | B | B, words split |
|---|---|---|---|---|---|---|---|
| 1 | plain-words | 01-plain | `quick brown fox` | `quick brown fox` | pass | pass | pass |
| 2 | plain-sentence | 01-plain | `A second sentence follows it.` | `A second sentence follows it.` | pass | pass | pass |
| 3 | italic | 02-emphasis | `italic` | `italic` | pass | pass | pass |
| 4 | bold-around-nested-italic | 02-emphasis | `phrase with nested italic inside` | `phrase with *nested italic* inside` | pass | pass | pass |
| 5 | inside-nested-italic | 02-emphasis | `ted ital` | `ted ital` | pass | pass | pass |
| 6 | strikethrough | 02-emphasis | `struck` | `struck` | pass | pass | pass |
| 7 | bold-italic-together | 02-emphasis | `both at once` | `both at once` | pass | pass | pass |
| 8 | underscore-emphasis | 02-emphasis | `under score` | `under_ __score` | pass | pass | pass |
| 9 | code-span | 03-code-spans | `cargo build` | `cargo build` | pass | pass | pass |
| 10 | code-span-with-backticks | 03-code-spans | `` a `tick` here `` | `` a `tick` here `` | pass | pass | pass |
| 11 | code-span-padded | 03-code-spans | `` `x` `` | `` `x` `` | pass | pass | pass |
| 12 | into-code-span | 03-code-spans | `Run cargo` | `` Run `cargo `` | pass | pass | pass |
| 13 | inline-link | 04-links | `setup guide` | `setup guide` | pass | pass | pass |
| 14 | reference-link | 04-links | `reference page` | `reference page` | pass | pass | pass |
| 15 | collapsed-reference-link | 04-links | `second one` | `second one` | pass | pass | pass |
| 16 | angle-autolink | 04-links | `https://example.com/a?b=c&d=e` | `https://example.com/a?b=c&d=e` | pass | pass | pass |
| 17 | email-autolink | 04-links | `team@example.com` | `team@example.com` | pass | pass | pass |
| 18 | bare-autolink | 04-links | `https://example.com/path` | `https://example.com/path` | pass | pass | pass |
| 19 | www-autolink | 04-links | `www.example.org` | `www.example.org` | pass | pass | pass |
| 20 | link-across-soft-break | 04-links | `details.\nRead the reference` | `details.\nRead the [reference` | pass | pass | pass |
| 21 | image-alone | 05-images | the img element (#1) | `![a small *cat* picture](cat.png "Cat")` | pass | pass | pass |
| 22 | image-inside-text | 05-images | from `Before` to `after` | `Before ![a small *cat* picture](cat.png "Cat") after` | pass | pass | pass |
| 23 | image-own-paragraph | 05-images | the img element (#2) | `![Diagram of the flow](flow.svg)` | pass | pass | pass |
| 24 | entity-amp | 06-entities | `Tom & Jerry` | `Tom &amp; Jerry` | pass | pass | pass |
| 25 | entity-nbsp | 06-entities | `a\u00a0b` | `a&nbsp;b` | pass | pass | pass |
| 26 | entity-alone | 06-entities | `&` | `&amp;` | pass | pass | pass |
| 27 | entity-numeric-emoji | 06-entities | `😀 smile` | `&#x1F600; smile` | pass | pass | pass |
| 28 | entity-two-chars-half | 06-entities | `≂` | `&NotEqualTilde;` (snaps) | pass | pass | pass |
| 29 | escaped-asterisks | 07-escapes | `*emphasis*` | `\*emphasis\*` | pass | pass | pass |
| 30 | between-escapes | 07-escapes | `emphasis` | `emphasis` | pass | pass | pass |
| 31 | escaped-backslash | 07-escapes | `a \ backslash` | `a \\ backslash` | pass | pass | pass |
| 32 | soft-break | 08-breaks | `here\nsecond` | `here\nsecond` | pass | pass | pass |
| 33 | hard-break-spaces | 08-breaks | from `red` to `violets` | `red  \nviolets` | pass | pass | pass |
| 34 | hard-break-backslash | 08-breaks | from `One` to `two` | `One\\ntwo` | pass | pass | pass |
| 35 | georgian-word | 09-georgian | `ტექსტი` | `ტექსტი` | pass | pass | pass |
| 36 | georgian-across-bold | 09-georgian | `ქართული ტექსტი` | `ქართული** ტექსტი` | pass | pass | pass |
| 37 | emoji-joiner-sequence | 10-emoji | `👩‍💻 developer` | `👩‍💻 developer` | pass | pass | pass |
| 38 | emoji-half-of-sequence | 10-emoji | `👩` | `👩‍💻` (snaps) | pass | pass | pass |
| 39 | emoji-split-surrogate | 10-emoji | `🇬🇪` minus its last UTF-16 unit | `🇬🇪` (snaps) | pass | pass | pass |
| 40 | cjk-japanese | 11-cjk | `文章` | `文章` | pass | pass | pass |
| 41 | cjk-mixed | 11-cjk | `です。这是中文` | `です。这是中文` | pass | pass | pass |
| 42 | tight-list-item | 12-list-tight | `banana split` | `banana *split` | pass | pass | pass |
| 43 | tight-list-two-items | 12-list-tight | from `apple` to `banana` | `apple\n- banana` | pass | pass | pass |
| 44 | loose-list-paragraph | 13-list-loose | `with a second paragraph.` | `with a second paragraph.` | pass | pass | pass |
| 45 | loose-list-two-paragraphs | 13-list-loose | from `Second item` to `with` | `Second item\n\n   with` | pass | pass | pass |
| 46 | nested-list-deep | 14-list-nested | `deepest three` | `deepest three` | pass | pass | pass |
| 47 | nested-list-across-levels | 14-list-nested | from `inner` to `deepest` | `inner two\n    - deepest` | pass | pass | pass |
| 48 | task-item | 15-tasks | `write the plan` | `write the plan` | pass | pass | pass |
| 49 | task-item-emphasis | 15-tasks | `check the result` | `check *the* result` | pass | pass | pass |
| 50 | table-cell | 16-table | `plain` | `plain` | pass | pass | pass |
| 51 | table-escaped-pipe | 16-table | `a \| b` | a &#92;\| b | pass | pass | pass |
| 52 | table-after-escaped-pipe | 16-table | `bold y` | `bold** y` | pass | pass | pass |
| 53 | table-pipe-in-code | 16-table | `p \| q` | p &#92;\| q | pass | pass | pass |
| 54 | table-two-cells | 16-table | from `alpha` to `a \| b` | alpha \| a &#92;\| b | pass | pass | pass |
| 55 | table-header-pipe | 16-table | `Value \| unit` | Value &#92;\| unit | pass | pass | pass |
| 56 | table-multibyte-cell | 16-table | `日本語` | `日本語` | pass | pass | pass |
| 57 | blockquote-two-lines | 17-blockquote | `line\ncontinues` | `line\n> continues` | pass | pass | pass |
| 58 | blockquote-nested | 17-blockquote | `A nested quote.` | `A nested quote.` | pass | pass | pass |
| 59 | alert-body | 18-alerts | `Remember to save often.` | `Remember to *save* often.` | pass | pass | pass |
| 60 | alert-from-title | 18-alerts | from `Note` to `Remember` | `Remember` (snaps) | pass | pass | pass |
| 61 | alert-warning | 18-alerts | `Do not delete` | `Do not delete` | pass | pass | pass |
| 62 | atx-heading | 19-headings | `Main title` | `Main title` | pass | pass | pass |
| 63 | atx-heading-closing-hashes | 19-headings | `Section two` | `Section *two` | pass | pass | pass |
| 64 | setext-heading | 19-headings | `Setext heading` | `Setext heading` | pass | pass | pass |
| 65 | footnote-ref-with-word | 20-footnotes | `note1` | `note[^1]` | pass | pass | pass |
| 66 | footnote-ref-alone | 20-footnotes | contents of `sup.footnote-ref a` (#1) | `[^1]` | pass | pass | pass |
| 67 | footnote-body | 20-footnotes | `note text itself` | `note *text* itself` | pass | pass | pass |
| 68 | footnote-body-two-lines | 20-footnotes | `longer note\nwith` | `longer note\n    with` | pass | pass | pass |
| 69 | footnote-into-back-link | 20-footnotes | `itself. ↩` | `itself.` (snaps) | pass | pass | pass |
| 70 | across-bold-and-code | 21-cross-inline | `part and code` | `` part** and `code `` | pass | pass | pass |
| 71 | across-code-and-link | 21-cross-inline | `code part and link` | `` code part` and [link `` | pass | pass | pass |
| 72 | across-two-paragraphs | 22-cross-block | from `ends here.` to `The second` | `ends here.\n\nThe second` | pass | pass | pass |
| 73 | across-paragraph-and-list | 22-cross-block | from `starts` to `a list` | `starts here.\n\n- and a list` | pass | pass | pass |
| 74 | front-matter-heading | 23-front-matter | `Title after front matter` | `Title after front matter` | pass | pass | pass |
| 75 | front-matter-body | 23-front-matter | `Body text here.` | `Body *text* here.` | pass | pass | pass |
| 76 | crlf-soft-break | 24-crlf | `soft\nbreak` | `soft\r\nbreak` | pass | pass | pass |
| 77 | crlf-hard-break | 24-crlf | from `hard` to `break here` | `hard  \r\nbreak here` | pass | pass | pass |
| 78 | crlf-list-item | 24-crlf | `item one` | `item *one` | pass | pass | pass |
| 79 | crlf-code-span | 24-crlf | `span with` | `span\r\nwith` | pass | pass | pass |
| 80 | crlf-table-pipe | 24-crlf | `c \| d` | c &#92;\| d | pass | pass | pass |
| 81 | tab-after-list-marker | 25-tabs | `Tab after the marker` | `Tab after the marker` | pass | pass | pass |
| 82 | tab-continuation-line | 25-tabs | `marker\nstill the` | `marker\n\tstill the` | pass | pass | pass |
| 83 | tab-inside-text | 25-tabs | `Text\twith` | `Text\twith` | pass | pass | pass |
| 84 | tab-after-quote-marker | 25-tabs | `Quote after a tab.` | `Quote after a tab.` | pass | pass | pass |
| 85 | tab-indented-code | 25-tabs | `fn main` | `fn main` | pass | pass | pass |
| 86 | fenced-code-line | 26-code-blocks | `println!("hi");` | `println!("hi");` | pass | pass | pass |
| 87 | fenced-code-in-list | 26-code-blocks | `cargo build\ncargo test` | `cargo build\n   cargo test` | pass | pass | pass |
| 88 | fenced-code-in-quote | 26-code-blocks | `quoted code` | `quoted code` | pass | pass | pass |
| 89 | raw-html-inline | 27-html-inline | `Press Ctrl and S` | `Press <kbd>Ctrl</kbd> and <kbd>S` | pass | pass | pass |
| 90 | whole-paragraph-by-element | 02-emphasis | contents of `p` (#2) | `A **bold phrase with *nested italic* inside** it, then ***both at once*** and _under_ __score__.` | pass | pass | pass |
| 91 | selection-with-edge-spaces | 01-plain | ` quick brown fox ` | `quick brown fox` | pass | pass | pass |
| 92 | only-generated-text | 18-alerts | `Note` | no range | pass | pass | pass |
| 93 | alert-into-next-title | 18-alerts | from `often.` to `Warning` | `often.` (snaps) | pass | pass | pass |
| 94 | table-emoji-cell | 16-table | `👩‍💻` | `👩‍💻` | pass | pass | pass |
| 95 | combining-mark-split | 28-escaping-and-marks | `cafe` | `café` (snaps) | pass | pass | pass |
| 96 | html-special-chars-in-code | 28-escaping-and-marks | `a < b && c > d` | `a < b && c > d` | pass | pass | pass |
| 97 | html-special-chars-in-text | 28-escaping-and-marks | `3 < 4 > 2 & more "quotes"` | `3 < 4 > 2 & more "quotes"` | pass | pass | pass |
| 98 | mouse-double-click-word | 01-plain | mouse double-click: `brown` | `brown` | pass | pass | pass |
| 99 | mouse-double-click-georgian | 09-georgian | mouse double-click: `ტექსტი` | `ტექსტი` | pass | pass | pass |
| 100 | mouse-triple-click-paragraph | 22-cross-block | mouse triple-click: `second paragraph` | `The second paragraph starts here.` | pass | pass | pass |
| 101 | mouse-triple-click-list-item | 12-list-tight | mouse triple-click: `banana` | `banana *split` | pass | pass | pass |
| 102 | mouse-triple-click-table-cell | 16-table | mouse triple-click: `a \| b` | a &#92;\| b | pass | pass | pass |
| 103 | mouse-drag-across-blocks | 22-cross-block | mouse drag: from `first` to `second` | `first paragraph ends here.\n\nThe second` | pass | pass | pass |
| 104 | mouse-drag-across-inlines | 21-cross-inline | mouse drag: from `bold` to `link` | `` bold part** and `code part` and [link `` | pass | pass | pass |
| 105 | code-span-wraps-in-list | 29-code-span-wrap | `make_thing(first, second)` | `make_thing(first,\n  second)` | pass | pass | pass |
| 106 | code-span-wraps-in-quote | 29-code-span-wrap | `alpha beta` | `alpha\n> beta` | pass | pass | pass |
| 107 | into-footnote-list-backwards | 30-footnote-order | from `last paragraph` to `note body` | `The note body.\n\nA last paragraph here.` (snaps) | pass | pass | pass |
| 108 | reverse-markup-around | 02-emphasis | source `**bold**` | shows `bold` | pass | pass | pass |
| 109 | reverse-half-entity | 06-entities | source `amp` | shows `&` | pass | pass | pass |
| 110 | reverse-link-url-only | 04-links | source `https://example.com/setup` | shows (nothing) | pass | pass | pass |
| 111 | reverse-code-indent | 26-code-blocks | source `   cargo test` | shows `cargo test` | pass | pass | pass |
| 112 | reverse-two-blocks | 22-cross-block | source `here.\n\nThe second` | shows `here.The second` | pass | pass | pass |
| 113 | reverse-escaped-pipe | 16-table | source &#92;\| | shows `\|` | pass | pass | pass |

## Results on the private corpus

The corpus is 3,416 Markdown files, 63.9 MB of rendered HTML.

**The map covers everything.**

| Node type | Count | Mapped exactly |
|---|---|---|
| Text | 846,229 | 846,229 (134 of them needed escapes or entities lined up) |
| Code span | 341,076 | 341,076 |
| Code block lines | 111,156 in 7,114 blocks | 111,156 |
| Table cells needing the escaped-pipe repair | 45 | 45 |
| Images, footnote references | 0, 0 | only the fixtures test these |

A self-check ran on every file. Every verbatim piece equals its source bytes, and every node's pieces spell the node's text. It found 0 errors. Before the code-span fix, 100 code spans that wrap onto a second line inside a list item or quote fell back to whole-span snapping. After it, 0 did.

**Random selections in the browser.** Every page was opened three times: design A, design B, and design B with every word split into its own element. Each opening got the same 40 random selections. A selection starts at a random character and is 1 to 1,000 characters long, so most cross several inline nodes and many cross blocks.

| Check | Result |
|---|---|
| Selections, per variant (design A, design B, design B with every word split into its own element) | 136,640 each, 409,920 in all |
| Mapped to a source range | 408,330 (99.6%) |
| Of those, exact | 408,321 |
| Snapped, because the edge split a character made of several code points | 9 |
| No range, because the selection held only whitespace | 1,590 |
| No range although the selection held text | 0 |
| Highlighting the mapped range did not show the selected text | 0 |
| The range did not start or end with the selected characters in the source file | 0 |
| Design A and design B gave different answers | 0 |
| Splitting the text changed design B's answer | 0 |

The source check does not use the map. For each exact result whose first or last piece is verbatim, it reads the source file with git and checks that the range starts with the first selected character and ends with the last one.

## Size cost per page

Measured on all 3,416 corpus pages. "Added" is compared with comrak's plain HTML for the same file.

| | Median page | 90th percentile page | All pages together |
|---|---|---|---|
| Plain HTML | 11.9 KB | 38.1 KB | 63.9 MB |
| Design A adds | 5.8 KB (51%) | 21.1 KB (68%) | 33.1 MB (52%) |
| Design B adds, markers and table | 3.2 KB (27%) | 10.8 KB (47%) | 17.8 MB (28%) |
| of which the block markers | 9% | 23% | 10% |
| Design B with a compact table (not browser-tested) | 19% | 36% | 20% |
| comrak's own `data-sourcepos`, for comparison | 5.2 KB | 18.0 KB | 29.6 MB (46%) |

After gzip, which is what a server sends:

| | Median page | 90th percentile page | All pages together |
|---|---|---|---|
| Plain HTML, gzipped | 4.4 KB | 12.3 KB | 21.2 MB |
| Design A adds | 1.1 KB (26%) | 3.8 KB (35%) | 28% |
| Design B adds | 1.1 KB (24%) | 3.1 KB (46%) | 25% |
| Design B with a compact table adds | 23% | 44% | 24% |

- The compact table writes each number in base64 digits of 5 bits, as JavaScript source maps do. It halves the table, from 11.5 MB to 6.3 MB raw. The page script does not read it yet.
- The worst pages for design B are long tables with short cells. One 75 KB page with 3,744 cells gzips to 5.5 KB, and design B adds 24 KB to that.
- The page script is 14.4 KB, 4.4 KB gzipped, before any minifying. It is the same for every page, so a browser loads it once.
- comrak's own `data-sourcepos` attributes give only line and column of elements, and nothing for text inside an element. They cannot do this job alone.
- On the 30 small fixture pages together, design A adds 98% and design B 52%. Short pages carry more markup per character, so the corpus numbers are the ones to plan with.

## Speed

Timings ran 3 times; the numbers are the medians. The server was shared, with a load average between 16 and 36.

| Step | Whole corpus | Median file | Slowest file |
|---|---|---|---|
| comrak parse | 470 ms | 0.1 ms | 6.4 ms |
| Build the map | 496 ms | 0.1 ms | 6.4 ms |
| Render HTML and table, design B | 366 ms | 0.1 ms | 9.5 ms |

In Chromium:

| Page | Pieces on the page | Build the pieces, on first selection | `toSource` per selection | `fromSource` per range |
|---|---|---|---|---|
| Median corpus page | 217 | 0.4 ms (A), 0.4 ms (B) | 0.02 ms | 0.01 ms |
| Largest corpus page, 454,000 characters | 10,413 | 17.5 ms (A), 11.9 ms (B) | 0.05 ms (A), 0.04 ms (B) | 0.04 ms |
| Slowest build among the 5 largest pages | 8,968 | 8.2 ms (A), 19.3 ms (B) | 0.03 ms | 0.03 ms |

Each number is the median of 3 page loads, with 200 random selections per load. The first version of the edge trimming walked every node under the selection and took up to 13 ms on the largest pages. It now walks only the nodes at the two edges.

## Recommended design

Design B: plain HTML, a marker on each block that holds text, and one offset table per block beside the HTML.

Both designs passed the same tests and agreed on every random selection. B wins on the other points:

1. It adds about half the bytes before compression: 28% of the HTML against 52%. After gzip the two cost about the same, 25% against 28%.
2. The table can travel as its own file, so a page could load it only when the reader selects text. Until then the cost would be the block markers alone, 10% of the HTML. The spike always sends the table inside the page; loading it later is not built.
3. It survives other code splitting a block's text into more nodes, as syntax highlighting, search hits or word-level change marks do. In a test pass that wrapped every word on the page in its own `<mark>` element, design B still passed all 113 cases and design A passed 25. On the corpus, the same split changed none of design B's answers (see above).

### Data the core crate produces

For each document, a list of blocks that hold text, in rendered order. Each block has:

- the source byte where the block starts;
- its pieces, in rendered order. Each piece has its kind (verbatim, atomic or object), its rendered text, and its source byte range.

The core also holds the two comrak workarounds below, and the rules that line up escapes, entities, code spans and code blocks with their source. It needs no file, git or network access, so it fits `driftmark-core`.

### What the program renders

- comrak's HTML writer, with the writers for paragraph, heading, table cell and code block replaced so the block gets `data-k="n"`. A tight list item's text gets a `<span data-k="n">`.
- The writers for text, code span, image and footnote reference replaced so they record where their text lands. Rendered offsets count UTF-16 units, as the browser does.
- Per block, a flat list of whole numbers. The first is the block's source start. Then each piece adds:
  - the rendered gap since the previous piece;
  - the rendered length, times 2, plus 1 when the piece is atomic;
  - the source gap since the previous piece;
  - the source length, for atomic pieces only. A verbatim piece needs none, because the script counts the UTF-8 bytes of its rendered text.
- Text the program adds inside a block, such as a footnote back link, must come after the block's last piece. Otherwise it needs a marker that the script skips; the spike does not build that marker. A change page that shows deleted words inside a block needs the same marker.

### What the page script does

1. On the first selection it reads the table. For each `[data-k]` element it walks the text nodes and images in order and lays the table's pieces over them. A piece that covers several text nodes is cut into one part per node.
2. `toSource(range)`:
   1. drops whitespace at both edges of the Range;
   2. finds the first and last pieces the Range touches, by binary search over the pieces in page order;
   3. for a verbatim piece, adds the UTF-8 length of the rendered text before the edge to the piece's source start;
   4. for an atomic piece or an image, takes the piece's whole source range;
   5. returns the start, the end, and whether the result is exact, with the reasons when it is not.
3. `fromSource(start, end)` finds the pieces that touch the byte range, turns byte offsets back into UTF-16 offsets inside verbatim pieces, and builds one DOM Range from the first to the last. It paints the Range with the CSS Custom Highlight API and outlines any image inside it.

### How a selection that cannot map exactly snaps

The rule: grow to the whole piece, never cut one in half. Shrink past text that has no source.

| The selection edge falls | It snaps to | Example |
|---|---|---|
| inside an entity, an escape or a footnote number | the whole piece | half of `≂̸` from `&NotEqualTilde;` gives the whole entity |
| inside a character made of several code points | the whole character | half of `👩‍💻` gives all of it; half of a flag's surrogate pair gives the flag |
| between a letter and its combining accent | the letter with its accent | `cafe` out of `café` gives `cafe` plus the accent |
| on an image | the whole image source | `![alt](cat.png "Cat")` |
| on text the program generated | the nearest mapped character inside the selection | from the alert title "Note" into its body gives the body only |
| on whitespace | the nearest non-space character inside | ` quick brown fox ` gives `quick brown fox` |

A selection made only of generated text (an alert title alone) maps to nothing. The result then says "no range", and the UI can refuse the note.

## comrak problems found

Both are worked around in the map. Both are worth reporting upstream.

1. **Escaped pipes shift positions in table cells.** comrak removes the backslash of `\|` before it reads the cell, then reports positions as if no byte was removed. Every inline node after an escaped pipe ends up one byte early per pipe. The workaround counts the escaped pipes in the cell's source and shifts each inline position back. 45 cells in the corpus needed it.
2. **A code span over a Windows line ending keeps the carriage return.** comrak gives `span\r with` for the source `` `span\r\nwith` ``. CommonMark turns the line ending into one space. The workaround renders one space, mapped to both bytes.

Everything else lined up with comrak's positions: front matter, CRLF files, tabs after list and quote markers, footnotes, alerts, autolinks, setext headings, nested lists and quotes.

## Limits that remain

1. **Only Chromium was tested.** The cached WebKit build needs system libraries that are not installed, and installing them is not allowed here. Safari has the three things the script uses: the Range API, `Intl.Segmenter` (Safari 14.1 and later) and the Custom Highlight API (Safari 17.2 and later). It is untested.
2. **Diagrams and raw HTML have no map.** A Mermaid diagram drawn as SVG replaces its code block, so text inside a diagram cannot carry a note. A note on a diagram needs a separate rule, such as anchoring to the whole block. Text inside a raw HTML block, such as a `<summary>`, is not mapped either. Markdown between HTML tags is mapped as usual.
3. **A source range that covers only markup shows nothing.** A link target alone, such as `(https://example.com/setup)`, has no rendered text, so `fromSource` returns nothing. Re-anchoring (decision 0004) must fall back to the block in that case.
4. **Footnotes render at the end of the page.** A selection from a paragraph into the footnote list can cover two separate stretches of source. The script returns one range that covers every selected character, and marks it "not exact" with the reason "reordered". Fixture 30 tests one such case. A note on that range quotes text the reader did not select, between the two stretches.
5. **Change pages are not covered.** Deleted words shown inside a block are not in the head version's source. They must be marked so the script skips them, or mapped to the base version with a second table.
6. **The corpus has no images and no footnotes.** Only the fixtures test those.
7. **Code blocks rely on one rule.** Each code line must be the end of its source line. A tab that the list indent splits in half would break that rule. The line then maps as a whole. It did not happen in the corpus.
8. **The compact table was measured, not tested.** The page script reads only the JSON table.
9. **Building the pieces takes up to 19 ms on the largest pages.** That happens once per page, on the first selection. It is not tuned.

## Needs the maintainer's decision

1. **comrak's licence.** Add `BSD-2-Clause` to the allowed licences in `deny.toml`, with comrak as the reason, when comrak enters the workspace. Decision 0003 already chose comrak. Recommended: yes.
2. **Reporting the two comrak problems upstream.** That is work outside this repo. Recommended: yes, with the made-up fixtures 16 and 24 as examples.
3. **Whether a selection that covers a whole bold or linked phrase widens to its markup.** Today `bold` maps to `bold`, not `**bold**`. Recommended: keep it tight. The note's quote is the rendered text, and a tight range finds the sentence again more precisely.

## How to run it again

The code is on branch `spike/anchoring`, in `spikes/anchoring/`.

```sh
export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=/root/cj-ways/target/spike-anchoring
cd spikes/anchoring
nice -n 10 cargo build --release -j 4
OUT=/root/cj-ways/driftmark/private/spikes/anchoring/fixtures-out
$CARGO_TARGET_DIR/release/anchoring render fixtures $OUT
cp web/anchor.js web/page.css $OUT/
cd harness
PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1 npm install
node run.mjs $OUT ../fixtures $OUT/results.json
node table.mjs ../fixtures/cases.json $OUT/results.json
```

The corpus run feeds `git cat-file --batch` output to `anchoring corpus <out> --pages N`, then runs `harness/fuzz.mjs`, `harness/sizes.mjs` and `harness/timing.mjs` on the result. It names every output file by number only, and writes only to the private folder.
