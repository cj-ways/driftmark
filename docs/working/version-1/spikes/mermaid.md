---
status: done
date: 2026-10-17
---

# Spike: Mermaid on the server with merman

> Short version: merman can be driftmark's default Mermaid renderer, with mermaid.js in the browser as the fallback, but only with one change from its defaults. Out of the box, merman guesses text widths, and in a real browser 375 of 754 diagrams show cut-off text, including 193 of the 194 private ones. When merman measures text with the same font file the page draws with, cut-off text drops to 5 diagrams, and 51 of 60 graded diagrams look the same as mermaid.js. The costs: 25 MB more binary, a minimum Rust version of 1.95 instead of 1.89, one font file shipped with the program, and six licences beyond MIT and Apache-2.0, one of them EPL-2.0.

## Facts in the brief that turned out wrong, and two that need care

1. **"At least 3 per diagram type, at most 60 images" cannot both hold.** The four corpora contain 33 diagram types, so 3 each would need 99 images. The sample gives 3 or more to the 4 types in the private corpus and to the 8 types the brief names. The other 25 types appear only in Mermaid's own docs and get 1 or 2 each. Every block still gets an automatic score.
2. **merman's minimum Rust version is 1.95.** The workspace says `rust-version = "1.89"`. Adding merman raises it to 1.95.
3. **Needs care: mermaid.js 12.1.0 alone cannot draw ZenUML.** ZenUML needs the separate plugin `@mermaid-js/mermaid-zenuml`. The reference run loads it (version 1.0.1). merman draws ZenUML by itself.
4. **Needs care: the theme of the mermaid.js reference.** Mermaid 12 gives flowchart, ER, sequence, class, state, requirement, use-case, swimlane and agent-flow diagrams their own theme (`redux-color`, `neo` look, font "Recursive Variable") when no theme is set. Calling `mermaid.initialize({ theme: 'default' })` overrides that and gives the older look. The first reference run made this mistake; the results here use Mermaid's true defaults, which merman matches.

## What was tested

| Corpus | Markdown files | Files with Mermaid | Mermaid blocks | Pinned at |
|---|---|---|---|---|
| The private corpus (docs folder) | 3,416 | 189 | 195 | `origin/dev` on 2026-10-17 |
| Mermaid syntax docs (`docs/syntax/`) | 34 | 34 | 545 | tag `mermaid@12.1.0`, commit `21f72f07` |
| OpenSpec | 729 | 3 | 4 | commit `c794978f` |
| GitHub Spec Kit | 170 | 6 | 11 | commit `c6a4c18f` |
| **All** | | | **755** | |

- Blocks were found with comrak 0.56 (the parser decision 0003 chose), counting fenced blocks whose info string starts with `mermaid`. A plain text search finds the same 195 private blocks. The 161 Markdown files outside the docs folder hold none.
- The private corpus is 94% ER diagrams: 183 ER, 9 flowcharts, 2 state diagrams, 1 sequence diagram. The median private block has 30 lines; the largest has 2,592 lines and 70,283 characters.
- merman 0.9.0, the latest crate (published 2026-10-12), says it follows Mermaid 12.1.0.
- The reference is mermaid.js 12.1.0 in headless Chromium build 1247, through Playwright 1.64.0-alpha-2026-10-02, the release that matches that cached browser.
- Both renderers' SVGs were placed inline in a page, sized to their own viewBox, and captured as PNG by the same Chromium.
- The automatic score is SSIM (structural similarity; 1.0 means identical) after scaling merman's picture to the reference size. The PNG step also counts HTML labels whose text runs outside its box, which clips the text.
- Grades: **good** means a reader gets the same picture; **acceptable** means a visible defect but every word is there and readable; **broken** means a word is missing, text is unreadable, or nothing renders.

## Results by diagram type

merman runs with real-font measurement (see the next section) unless the column says "estimate". "Text cut off" counts diagrams with at least one clipped label. The mermaid.js column counts ZenUML with its plugin. Its one failure is the 70,283-character block, which mermaid.js replaces with "Maximum text size in diagram exceeded" (its limit is 50,000 characters).

| Type | Blocks (private / public) | merman parses | merman renders | mermaid.js renders | Text cut off: estimate / real font | Similarity median | Graded: good / acceptable / broken |
|---|---|---|---|---|---|---|---|
| er | 208 (183 / 25) | 100% | 99.5% (207) | 99.5% (207) | 206 / 1 | 0.90 | 3 / 4 / 1 |
| flowchart | 139 (9 / 130) | 100% | 100% (139) | 100% (139) | 36 / 0 | 1.00 | 6 / 0 / 0 |
| sequence | 44 (1 / 43) | 100% | 100% (44) | 100% (44) | 0 / 0 | 0.72 | 3 / 0 / 0 |
| class | 38 (0 / 38) | 100% | 100% (38) | 100% (38) | 38 / 0 | 0.95 | 3 / 0 / 0 |
| gitgraph | 36 (0 / 36) | 100% | 100% (36) | 100% (36) | 0 / 0 | 0.94 | 1 / 0 / 0 |
| block | 30 (0 / 30) | 100% | 100% (30) | 100% (30) | 22 / 0 | 0.86 | 2 / 0 / 0 |
| state | 24 (2 / 22) | 100% | 100% (24) | 100% (24) | 8 / 0 | 1.00 | 3 / 0 / 0 |
| usecase | 24 (0 / 24) | 100% | 100% (24) | 100% (24) | 23 / 1 | 0.94 | 0 / 0 / 1 |
| wardley | 19 (0 / 19) | 100% | 100% (19) | 100% (19) | 0 / 0 | 1.00 | 1 / 0 / 0 |
| zenuml | 16 (0 / 16) | 100% | 100% (16) | 100% (16) | 0 / 0 | 0.94 | 1 / 0 / 0 |
| railroad | 14 (0 / 14) | 100% | 100% (14) | 100% (14) | 0 / 0 | 0.97 | 1 / 0 / 0 |
| timeline | 14 (0 / 14) | 100% | 100% (14) | 100% (14) | 0 / 0 | 0.97 | 1 / 0 / 0 |
| eventmodeling | 13 (0 / 13) | 100% | 100% (13) | 100% (13) | 0 / 0 | 1.00 | 1 / 0 / 0 |
| mindmap | 13 (0 / 13) | 100% | 100% (13) | 100% (13) | 13 / 1 | 1.00 | 1 / 0 / 0 |
| swimlane | 13 (0 / 13) | 100% | 100% (13) | 100% (13) | 8 / 2 | 0.75 | 2 / 0 / 0 |
| treeview | 13 (0 / 13) | 100% | 100% (13) | 100% (13) | 0 / 0 | 0.79 | 0 / 1 / 0 |
| agentflow | 12 (0 / 12) | 100% | 100% (12) | 100% (12) | 9 / 0 | 1.00 | 1 / 0 / 0 |
| gantt | 11 (0 / 11) | 100% | 100% (11) | 100% (11) | 0 / 0 | 0.82 | 3 / 0 / 0 |
| requirement | 9 (0 / 9) | 100% | 100% (9) | 100% (9) | 9 / 0 | 1.00 | 1 / 0 / 0 |
| sankey | 8 (0 / 8) | 100% | 100% (8) | 100% (8) | 0 / 0 | 1.00 | 1 / 0 / 0 |
| treemap | 8 (0 / 8) | 100% | 100% (8) | 100% (8) | 0 / 0 | 1.00 | 1 / 0 / 0 |
| venn | 8 (0 / 8) | 100% | 100% (8) | 100% (8) | 0 / 0 | 0.86 | 1 / 0 / 0 |
| xychart | 8 (0 / 8) | 100% | 100% (8) | 100% (8) | 0 / 0 | 0.98 | 3 / 0 / 0 |
| architecture | 6 (0 / 6) | 100% | 100% (6) | 100% (6) | 0 / 0 | 1.00 | 1 / 0 / 0 |
| c4 | 6 (0 / 6) | 100% | 100% (6) | 100% (6) | 0 / 0 | 0.71 | 0 / 2 / 0 |
| pie | 4 (0 / 4) | 100% | 100% (4) | 100% (4) | 0 / 0 | 1.00 | 3 / 0 / 0 |
| cynefin | 3 (0 / 3) | 100% | 100% (3) | 100% (3) | 0 / 0 | 0.97 | 1 / 0 / 0 |
| kanban | 3 (0 / 3) | 100% | 100% (3) | 100% (3) | 3 / 0 | 1.00 | 1 / 0 / 0 |
| packet | 3 (0 / 3) | 100% | 100% (3) | 100% (3) | 0 / 0 | 1.00 | 1 / 0 / 0 |
| quadrant | 3 (0 / 3) | 100% | 100% (3) | 100% (3) | 0 / 0 | 1.00 | 1 / 0 / 0 |
| radar | 3 (0 / 3) | 100% | 100% (3) | 100% (3) | 0 / 0 | 1.00 | 1 / 0 / 0 |
| ishikawa | 1 (0 / 1) | 100% | 100% (1) | 100% (1) | 0 / 0 | 0.97 | 1 / 0 / 0 |
| journey | 1 (0 / 1) | 100% | 100% (1) | 100% (1) | 0 / 0 | 1.00 | 1 / 0 / 0 |
| **All** | 755 (195 / 560) | 100% | 99.9% (754) | 99.9% (754) | 375 / 5 | 0.98 | 51 / 7 / 2 |

The 60 graded blocks: 12 private (5 ER, 3 flowcharts, 2 state, 1 sequence, and the 70,283-character ER block), 2 OpenSpec and Spec Kit flowcharts, and 46 from Mermaid's docs. Within each type they were drawn at random with a fixed seed.

The similarity score is a weak judge on its own. Sequence diagrams score a median of 0.72, yet all 3 graded ones are good: merman spaces messages slightly tighter, which shifts every line below. ER diagrams with one long relationship label score as low as 0.14 while being acceptable, because the label's height moves the lower table. Use the cut-off counts and the grades; use the score only to find outliers.

## What goes wrong, and why

### 1. merman's default text widths do not match the browser

- By default merman does not measure text with a font. It adds up a fixed width per character (0.43 em for most lower-case letters, 0.60 em for capitals). The browser then draws the labels in a real font inside boxes of that guessed size, and clips what does not fit.
- Mermaid 12's default font list starts with "Recursive Variable", a font almost no computer has. The browser falls back to Arial, or on Linux to Liberation Sans, which has the same widths. Across the 19,576 type and column names in the private ER diagrams, Arial is 10% wider than merman's guess at the median, and wider for 79% of them.
- Result with the guess: 375 of 754 diagrams and 11,933 of 18,595 HTML labels (64%) are cut off. In the private corpus it is 193 of 194 diagrams. Every ER table cell loses its last letters.
- merman can measure with real fonts (its `native-fonts` feature). With the four Liberation Sans faces (regular, bold, italic, bold italic) loaded, cut-off text drops to 5 diagrams and 5 labels. mermaid.js itself has 1 cut label.
- This only works if the browser draws with the same font. Liberation Sans has the same widths as Arial, which Macs, Windows and iPhones have, so the test matches those. Android has no Arial. driftmark should therefore ship one font, serve it to the page as a web font, set it as Mermaid's `fontFamily`, and give the same file to merman.
- Gaps in merman's API that driftmark has to work around:
  - Fonts load only from files or folders, not from bytes inside the program. driftmark would write its embedded font to a cache folder first.
  - A font context has one budget of 4,096 text measurements for its whole life. A server must call `for_operation()` for each render, or every render after the first few fails.

### 2. ER relationship labels are not wrapped

mermaid.js wraps long relationship labels to about 200 pixels. merman keeps them on one line. The private corpus has 403 relationship labels, mostly full foreign-key clauses, with a median of 72 characters and a longest of 129. merman draws each as one line. The edge line then crosses the label, and two neighbouring labels can touch. Every word stays readable, so these diagrams are graded acceptable. This is the main remaining difference for the private corpus and is worth reporting upstream.

### 3. Very large diagrams

- The largest private block (2,592 lines, 164 tables, 97 relationships) exceeds merman's default layout budget (14.2 million work units against a ceiling of 14.1 million). The "trusted" profile's ceiling of 15.0 million is also too low.
- With no limits and the built-in guess, merman draws it in 86 ms (6.1 MB of SVG).
- With no limits and real fonts, merman stops with an internal error: "ER entity measured size mismatch". A made-up diagram of the same size reproduces it (`spikes/mermaid/make_big_er.py`, seeds 0 to 2 fail, 3 to 5 pass). Diagrams of 150 small tables pass, so it is not a plain table-count limit. This is a merman bug to report.
- mermaid.js refuses this block by default too: it is above the 50,000-character limit. With the limit raised, mermaid.js draws it in 3.2 seconds.

### 4. Smaller defects, all outside the private corpus

- **Use-case diagrams:** a bold word inside a Markdown edge label is dropped from the visible label ("opens **form**" shows as "opens"). It survives only in the hidden accessibility text. Graded broken.
- **Tree-view diagrams:** the SVG sets no root font, so file names fall back to the browser's serif font.
- **C4 diagrams:** merman places boxes differently and packs them narrower; some labels sit on box text. mermaid.js's own C4 output also has overlaps.
- **Gantt charts:** mermaid.js stretches a Gantt chart to the page width; merman uses a fixed width. Content is the same.
- **Packet diagrams in the dark theme:** see the dark theme section.

## Speed

Every timing is the median of 3 runs on the shared server, with a load average of about 30 on 12 cores. The 70,283-character block is left out.

| Renderer | Per block, median | Per block, 95th percentile | Slowest block | All 754 blocks |
|---|---|---|---|---|
| merman, built-in guess | 0.43 ms | 2.5 ms | 30 ms | 0.6 s |
| merman, real fonts | 1.6 ms | 22 ms | 163 ms | 3.8 s |
| mermaid.js in a Chromium page | 19 ms | 89 ms | 778 ms | 23.8 s |

- Private blocks alone: merman with real fonts takes a median of 9.3 ms and a 95th percentile of 45 ms. mermaid.js takes a median of 44 ms on the same blocks, on a server CPU; a phone is slower.
- **Cold against warm.** In a fresh process the first render takes about 1 ms more than later ones (for example 1.7 ms against 0.9 ms for a 31-line ER diagram with the guess, and 10.5 ms against 9.5 ms with real fonts). Loading the four font files takes 3.2 ms once. A whole process that starts, renders one diagram and exits takes 8 to 9 ms. After the first block, warm and first-pass times are within 7% of each other, so caching rendered SVGs matters more than warming up.
- Memory: rendering all 755 blocks in one process peaks at 29 to 33 MB.

## Page weight

- The browser fallback costs 5.2 MB of JavaScript (1.5 MB compressed) for mermaid.js, plus 3.1 MB (0.8 MB compressed) for the ZenUML plugin.
- A server-drawn private diagram is 71 KB at the median (7.3 KB compressed); the 95th percentile is 386 KB (32 KB compressed).
- So drawing on the server keeps a phone page light only if the browser fallback loads on demand, for the blocks that need it.

## Dark theme

- Yes. merman draws Mermaid's `dark` theme for every type. It also knows `neo-dark`, `redux-dark` and `redux-dark-color`.
- With `theme: dark`, 754 of 755 blocks render, the median similarity to mermaid.js's dark output is 0.97, and 5 diagrams have a cut label (the same 5 as in the light theme).
- One defect: packet diagrams keep light boxes, and their bit numbers and title stay black on the dark background, so those cannot be read. There are 3 packet blocks, all in Mermaid's docs.
- Colours are baked into the SVG. A page that follows the system's light or dark setting needs both SVGs, or a fresh render when the setting changes. Rendering twice costs little (see Speed); sending both doubles the diagram weight unless one loads on demand.

## Binary size

A probe program that reads Mermaid on stdin and prints SVG was built with driftmark's release settings (`lto = true`, `codegen-units = 1`, `strip = true`). Without merman it is 0.30 MB. Sizes are in millions of bytes; "added" is the difference from that baseline.

| merman features | Binary | Added | Added, gzip | Added, xz |
|---|---|---|---|---|
| Default: every diagram type, SVG, ELK and Cytoscape layouts, math labels | 23.74 MB | 23.44 MB | 9.94 MB | 7.09 MB |
| Default plus real-font measurement (`native-fonts`) | 25.28 MB | 24.99 MB | 10.64 MB | 7.58 MB |
| Default without ELK | 21.20 MB | 20.90 MB | 8.84 MB | 6.33 MB |
| Every type, SVG only (no ELK, no Cytoscape, no math) | 16.32 MB | 16.02 MB | 6.88 MB | 4.93 MB |
| The 8 common types only (flowchart, ER, sequence, class, state, Gantt, pie, xychart), SVG and ELK | 13.87 MB | 13.57 MB | 5.81 MB | 4.20 MB |

- The recommended set (default plus real-font measurement) adds 25.0 MB, plus the font files: the four Liberation Sans faces are 1.65 MB (0.86 MB compressed). Regular and bold alone are 0.83 MB.
- ELK costs 2.5 MB. Math labels and the Cytoscape layouts together cost 4.9 MB. Real-font measurement costs 1.6 MB.
- Dropping the 25 rarer diagram types saves about 10 MB. Most of the size sits in code every type shares.
- For M6, this decides the npm package shape: one package holding every platform's binary would carry about 25 MB per platform.

## Licences

cargo-deny 0.20.2 was run on a crate that depends only on merman, with driftmark's own `deny.toml` rules (only MIT and Apache-2.0 allowed). Every feature set fails. These licences would have to be added:

| Licence | Crates | In which feature sets |
|---|---|---|
| Unicode-3.0 | 20 (the `icu_*` family, `unicode-ident`, `zerovec` and others) | all |
| MPL-2.0 | 4: `cssparser`, `cssparser-macros`, `dtoa-short`, `selectors` | all |
| BSD-3-Clause | 2: `encoding_rs`, `lol_html`; with real fonts also `tiny-skia-path` | all |
| Zlib | 1 or 2: `foldhash`, and `slotmap` with layouts | all |
| EPL-2.0 | 1: `merman-elk-layered` (Eclipse ELK translated to Rust) | with ELK, which is in merman's defaults |
| BSD-2-Clause | 1: `arrayref` | with real-font measurement |

Notes:

- **Unicode-3.0 is not specific to merman.** `unicode-ident` comes with `proc-macro2`, which almost every Rust project with `serde` derives already pulls in.
- **MPL-2.0 is a file-level copyleft licence.** Using the crates unchanged needs nothing beyond the notice and pointing to their source.
- **EPL-2.0 is the one that needs a decision.** It is also file-level copyleft. Distributing a binary needs the EPL notice and a way to get the source of those files.
  - Dropping it means building merman without ELK. Mermaid 12 uses ELK as its default layout. Without ELK, merman does not refuse: it falls back to the older dagre layout (the Mermaid 11 look).
  - Tested on the whole corpus: still 754 of 755 render and 5 diagrams have a cut label, but 445 SVGs change. The median similarity to mermaid.js drops from 0.98 to 0.90 on Mermaid's docs and from 0.68 to 0.59 on the private corpus.
  - ER tables and flowcharts stay readable (flowcharts get curved edges). State diagrams get worse: labels overlap around loop edges.
- **cargo-deny does not see everything.** merman's own `THIRD_PARTY_NOTICES.md` lists more licences inside its crates:
  - the KaTeX font files embedded by the math feature, under OFL-1.1;
  - code translated from fmin, under BSD-3-Clause;
  - code translated from DOMPurify, under Apache-2.0 or MPL-2.0;
  - many MIT projects (dagre, cytoscape, roughjs and others).
  driftmark's release needs a third-party notice file anyway.
- **The diagram font adds one more.** Liberation Sans is under OFL-1.1. Any other font driftmark picks will also come with a font licence.

## Recommendation

**Draw Mermaid on the server by default, with real-font measurement and a font driftmark ships. Fall back to mermaid.js in the browser for each block where merman returns an error.** Do not split by diagram type.

Reasons:

1. With real fonts, merman matches mermaid.js on 51 of 60 graded diagrams and keeps every word readable on 58. The 2 broken ones are a 70,283-character diagram that mermaid.js also refuses by default, and a use-case label from Mermaid's docs.
2. For the maintainer's own corpus, the remaining difference is unwrapped ER relationship labels. That is cosmetic, and it is a merman issue that can be reported upstream.
3. It is 12 times faster than mermaid.js per block at the median, and it spares the phone 1.5 MB of compressed JavaScript for pages that do not need the fallback.
4. A split by type gains nothing yet: the types with known defects (use-case, tree-view, C4, packet in dark mode) have no blocks in the private corpus, OpenSpec or Spec Kit.

What makes this the default only if the maintainer accepts:

- 25 MB more binary, plus about 1.7 MB of font files;
- EPL-2.0 in `deny.toml`, or a build without ELK that draws the older layout;
- raising the minimum Rust version from 1.89 to 1.95;
- shipping one font file, its OFL-1.1 licence, and serving it to pages as a web font.

Without real-font measurement, the server default is wrong for this corpus: 193 of 194 private diagrams would show cut-off text.

## For the M2 brief

- Load the fonts once, then call `for_operation()` per render.
- Raise merman's layout budget for trusted local files, or send over-budget blocks to the browser fallback.
- Give every SVG a unique `diagram_id`; one page holds many diagrams.
- Do not set `theme: 'default'` unless the older look is wanted; Mermaid 12's defaults differ per diagram type.
- When the page follows light and dark mode, render both themes or render on demand.
- Load mermaid.js only on pages that have a fallback block.
- Report upstream: unwrapped ER relationship labels, the size-mismatch error on large ER diagrams, the dropped bold word in use-case labels, the missing root font in tree-view, the packet dark theme, the lifetime measurement budget, and font loading from bytes.

## Not finished

- **Most types got 1 or 2 graded pictures.** Only the 4 private types and the 8 named types got 3 or more. The automatic counts cover every block.
- **Fonts other than Arial's widths were not tested.** Liberation Sans matches Arial, which Macs, Windows and iPhones have. An Android phone, or a font driftmark ships as a web font, was not tried.
- **merman's other SVG modes were not tried:** `htmlLabels: false` (labels as SVG text) and the "resvg-safe" pipeline. Either might avoid clipped HTML labels without real fonts.
- **The size-mismatch bug was not narrowed down further** than "164 tables and 97 relationships fail, 150 small tables pass".
- **No issue was filed upstream.** That happens outside this repo and needs the maintainer's OK.
- **What fills the 23 MB was not broken down** (for example with `cargo bloat`).
- **The safety of merman's SVG output was not reviewed.** Labels can carry HTML. merman's guide says the host must clean the SVG before putting it in a page. M2 must decide how.
- **Phone rendering speed of the mermaid.js fallback was not measured.** The browser timings come from a server CPU.

## How to rerun

The spike's code is in `spikes/mermaid/`. It is its own Cargo package, outside the workspace.

1. `cargo build --release` builds `extract` (finds blocks with comrak), `render` (renders a manifest with merman) and `once` (cold start probe). `--features native` adds real-font measurement through `SPIKE_FONT=<file>[,<file>...]`. `--no-default-features --features native` builds without ELK.
2. `extract <corpus> <root> <subfolder> <out-dir>` writes the blocks and `manifest.jsonl`.
3. `render <manifest> <blocks> <svg-dir> <results> <theme> <passes>` renders with merman. `SPIKE_RESOURCES=trusted|unbounded` changes the limits, and `SPIKE_CONFIG='{"layout":"dagre"}'` adds Mermaid settings.
4. In `js/`, `npm ci`, then `node render-mermaidjs.mjs ...` renders the reference and `node to-png.mjs ...` makes PNGs and counts clipped labels.
5. `python3 -I compare.py score <out-dir>` scores; `side` builds side-by-side pictures.
6. `size-probe/` measures binary size per feature set and is the crate cargo-deny checks.
7. `python3 -I make_big_er.py <dir>` writes the made-up large ER diagrams.

Raw outputs, including everything from the private corpus, stay in `private/spikes/mermaid/`, which git ignores.
