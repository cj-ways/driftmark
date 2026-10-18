---
status: done
date: 2026-10-18
---

# Spike: three visual directions for driftmark's pages

> Short version: three directions are built as static pages in `spikes/design/`, each with a branch list, a reading page and a change page, at desktop and phone width, light and dark. A, "Calm reading", is a serif long-form article. B, "Dense reference", is compact API-docs style. C, "Review first", starts from what changed and gives every change a card. All three meet every readability rule in the brief on desktop: body text 16 to 19 px, lines of 61 to 78 characters, contrast of at least 5.09 in both themes, change marks that show a line as well as a colour, and table headers that stay in view. On a phone, lines hold 31 to 46 characters, because 60 does not fit in 390 px at 17 px. I would pick C for this maintainer, because driftmark exists for change review and C is the only direction built around it. System fonts cost 0 bytes; the bundled body font costs 42,280 bytes per page; the diagram font costs 147,036 bytes as shipped, or 20,648 as a renamed subset.

## Facts in the brief that turned out wrong, or need care

1. **60 to 80 characters per line cannot hold on a phone.** At 390 px wide with text of at least 17 px, a line holds 31 to 46 characters (median 36 to 39). The desktop pages meet the rule.
2. **No real pair in this repo's history shows every view.** The engine found inline, side-by-side, rewritten, line and formatting changes, but no sentence rows and no moves. So the change page uses the fallback the brief allows: a made-up edit of `0005-version-1-scope.md`.
3. **The made-up edit needed a made-up "before" side too.** The sentence view needs prose with at least 4 sentences on both sides. No block of the real 0005 has 4 sentences. The "before" side is the real 0005 plus one made-up paragraph of 5 sentences. The "after" side is a made-up edit.
4. **The HTML is generated, the CSS is hand-written.** A short script fills each direction's page frame with the same content, converted once from the real Markdown and the real engine output. This keeps the three directions' content identical. No framework is used, and no build step is needed to view or serve the pages.
5. **The engine on `main` predates decision 0009.** It lists a new document block by block and counts rule words in new blocks. The mockups apply 0009's first two rules themselves: a new document is one item, and a rule word counts only when it changed inside an edited block.
6. **No sample document has every element the reading page needs.** The reading page is `docs/features/engine/rules-file.md`. It has the title, status, summary, wide and narrow tables, code and program output. For the mockup it gained one note alert, one warning alert, one Mermaid diagram, three links where document paths were written as code, and one outside link. The changed file is `spikes/design/tools/sample/rules-file-reading.md`.
7. **Liberation Sans reserves its name.** Its licence (OFL 1.1 with the Reserved Font Name "Liberation") lets a modified copy, such as a subset, ship only under another name. Shipping the whole font, as decision 0008 says, is fine.

## What was built

| Path on `spike/design` | What it is |
|---|---|
| `spikes/design/index.html` | The comparison page: the three directions side by side as screenshots, light or dark, with links to every live page and three live phone-width frames. It opens from disk. |
| `spikes/design/calm/`, `dense/`, `review/` | One folder per direction: `index.html` (branch list), `read.html`, `change.html` and one `style.css`. |
| `spikes/design/fonts/` | Literata (3 Latin files, 63,428 bytes), Liberation Sans as WOFF2 (147,036 bytes), and both licences. |
| `spikes/design/shots/` | 36 first-screen captures as WebP, 1,002,276 bytes in all. |
| `spikes/design/tools/` | The scripts that drew the diagram, prepared the data, built the pages and measured them, with the sample files and the prepared data. Not needed to view the pages. |
| `spikes/design/compare/` | A 45-line Rust program, outside the workspace, that runs `driftmark_core::diff::compare` on file pairs. It exists because sub-agents in this session cannot commit, so the made-up edit could not go through a git range. |

- **Change page.** A made-up branch, `agent/scope-after-m1`, with 4 documents and 37 changes. The made-up 0005 edit gives 14 changes, with every view: 5 edits (2 inline, 1 side by side, 1 rewritten, 1 sentence rows), 1 moved, 1 moved and edited (drawn side by side), 1 added and 6 formatting-only, folded in 3 groups. The real `rules-file.md` edit from commit `6d15e47` gives 21 changes, with a line-by-line view of a program output block. Two real new documents, `lint.md` and `git-ranges.md`, each show as one item. Every kind, level, signal, view and word mark comes from the real engine.
- **Branch list.** This repo's 8 branches with document changes, counted by the real engine against where each branch left `main`, plus the made-up branch.
- **Diagram.** Drawn once with mermaid.js 12.1.0 in headless Chromium, with Liberation Sans, and inlined as SVG.
- **Screenshots.** Playwright 1.64 with the cached Chromium 1247, at 1440 px and 390 px, light and dark: 36 full pages in `private/spikes/design/screens/` (44 MB, out of git) and the 36 first screens in `spikes/design/shots/`.

## The three directions

**A. Calm reading.** A long-form article. One narrow column with nothing beside it, a serif at 19 px with wide line spacing, warm paper colours in light and warm charcoal in dark. "On this page" is a quiet box at the top. Wide tables and two-version views step out of the column on a large screen. Changes read like tracked changes in a book: a small level word, then the text with its marks. It suits reading whole documents for a long time, on the phone too.

**B. Dense reference.** Good API docs. A sticky bar with a search box, a document tree on the left, "on this page" and "changed on" on the right, the system font at 16 px, striped compact tables, cool greys and one blue. The change list is a table with a column per level. Each change is a row: level and kind on the left, the place, every ranking signal and the view on the right. It suits looking things up and jumping between documents on a big screen.

**C. Review first.** It starts from what changed. The home page is an inbox grouped by highest level, with a bar per branch. The change page has a rail with every change, a "what needs your eyes" box, and one card per change with a solid level badge, the reason it ranks there, "Seen" and "Add a note". "Seen" dims the card and counts it. The reading page says which open branches change the document and which sections have changes waiting. The system font at 17 px. It suits a daily check of what agents changed: decide, note, move on.

All three draw the same change views. Deleted words are struck through on a red tint; inserted words are underlined on a green tint. Sentence rows carry `=`, `~`, `+` or `−` in a gutter. Side-by-side and rewritten views label "Before" and "After". A moved block says "Moved here from" and its old place says "Moved from here to", each linking to the other. Formatting-only changes fold into one line that opens to show the Markdown before and after.

## Readability rules: the measurements

Measured with headless Chromium on Linux for every page, at 1440 px and 390 px, light and dark: 36 page states. Line length counts the characters on each line of running text, except a paragraph's last line. Three numbers are the 10th percentile, the median and the 90th percentile.

| Rule | A. Calm reading | B. Dense reference | C. Review first |
|---|---|---|---|
| Body text, desktop / phone | 19 px / 18 px | 16 px / 17 px | 17 px / 17 px |
| Table text, desktop / phone | 16 px / 17 px | 16 px / 17 px | 16 px / 17 px |
| Characters per line, reading page, desktop | 61 / 67 / 72 | 63 / 71 / 78 | 64 / 72 / 78 |
| Characters per line, change page, desktop | 42 / 67 / 74 | 64 / 70 / 75 | 44 / 67 / 75 |
| Characters per line, reading page, phone | 31 / 36 / 40 | 31 / 39 / 45 | 33 / 39 / 46 |
| Lowest text contrast, light / dark | 5.09 / 6.07 | 5.41 / 6.89 | 5.18 / 6.80 |
| Lowest contrast of text inside a mark, light / dark | 11.57 / 9.69 | 13.56 / 10.72 | 14.88 / 10.74 |
| Mark lines against the mark's tint, lowest | 5.22 | 4.29 | 4.95 |
| Diagram lines against the page, light / dark | 5.59 / 6.73 | 6.39 / 7.37 | 6.69 / 7.32 |
| Diagram box borders against the page, light / dark | 3.50 / 4.47 | 3.04 / 4.16 | 3.07 / 3.99 |
| Table headers in view while scrolling, desktop | 6 of 6 | 6 of 6 | 6 of 6 |
| Tables on the phone | 3 stay tables with a sticky header; 3 become cards | the same | the same |
| Page scrolls sideways at 390 px | never | never | never |
| axe-core 4.14, WCAG 2.1 A and AA | 0 problems | 0 problems | 0 problems |

- **Contrast.** The check walks every visible text element, with every fold opened, and compares its colour with what is really behind it, tints and stripes included. Every text needs 4.5 (3 for large text). No element failed. Lines and borders need 3 against their background, as graphics do.
- **Change marks.** Colour is never the only sign. Deleted words are struck through and inserted words are underlined, in the mark's own darker colour. Each level has a word and a shape: ◆ decision, ▲ rule, ● fact, ○ or ✎ wording, ¶ formatting.
- **Change-page lines.** The low 10th percentile in A and C comes from side-by-side views, where each half holds about 40 to 50 characters by design.
- **Sticky headers.** The check scrolls each table up until its top is above the screen, then reads where the header row sits. It sits at 0 px in A and right under the bar in B (48 px) and C (52 px).
- **Phone tables.** A table with 3 or more columns becomes one card per row, and every value carries its column name. Two-column tables stay tables.
- **What does not meet a 16 px floor.** Code and program output use 14 or 15 px monospace. The smallest labels, such as level badges and "Before", are 11.8 to 12 px.
- **On a Mac.** These pages were measured with Linux fonts. With stand-ins for the Apple fonts, lines stay in range: Charis SIL (built from Charter) in A gives 62 / 69 / 75, and Inter (close to SF Pro) gives 61 / 67 / 74 in B and 62 / 68 / 73 in C.
- **The first pass failed three axe rules, now fixed.** The theme button's spoken name did not contain its visible word. Scroll boxes for code and diagrams could not be reached by keyboard. B's links were told apart by colour only.

## Fonts

| Font | Bytes added | Notes |
|---|---|---|
| System font stack | 0 | B and C use it for everything, A for its labels. On the maintainer's Mac and iPhone it is SF Pro; A's serif stack is Charter. |
| Literata, bundled (OFL 1.1, Latin subset, WOFF2) | 20,420 regular, 21,148 italic, 21,860 semibold: 63,428 in all | A uses it by default. Its pages load 2 files, 42,280 bytes, because no sample text is italic. A's font button switches to the system serif. |
| Liberation Sans for diagrams, whole font as WOFF2 | 147,036 (the TTF is 410,820) | The mockups load it first for diagrams, as decision 0008 says. Every page with a diagram downloads it once per device. |
| Liberation Sans, Latin subset as WOFF2 | 20,648 | Measured only. Its licence requires a new name inside the file. |

- **Body text: the bundled font buys nothing on the maintainer's own devices.** Charter, there for free, gives the same line length as Literata (69 and 67 median characters). A bundled font gives the same look on every machine, and in a one-file export (M5). It costs 42 to 63 KB on the first page.
- **No font comes from the internet.** Every file is next to the pages.
- **The diagram font can cost nothing.** With the order `"Liberation Sans", Arial, <bundled>`, the browser downloaded no font here, where Liberation Sans is installed. Macs, iPhones and Windows have Arial, which was the model for Liberation Sans's letter widths. Kerning may differ slightly from what merman measured; that was not tested.

## Page weight

HTML and CSS of each page, raw and gzipped at level 6. Fonts are counted above. The reading page's HTML holds the 14,176-byte diagram (2,393 gzipped).

| Page | A. Calm reading | B. Dense reference | C. Review first |
|---|---|---|---|
| Branch list | 25,662 (6,531) | 32,334 (7,185) | 32,652 (7,434) |
| Reading page | 62,487 (14,975) | 67,421 (15,874) | 68,480 (15,884) |
| Change page | 49,420 (11,182) | 56,477 (11,972) | 69,512 (13,089) |
| The CSS alone | 20,900 (5,190) | 23,908 (5,692) | 25,695 (5,962) |

Inline scripts add 575 to 994 bytes per page: the theme button, the font button, opening "on this page" on large screens, and C's "seen" count. The column names that phone cards need add 1,970 bytes to the reading page, 145 gzipped.

## For the render and pages lanes

1. **A sticky table header needs a table that is not inside a scroll box.** The render brief wraps tables "for horizontal scrolling". If that wrapper scrolls, the header can only stick inside it, not to the page. The mockups never scroll a table: cells wrap, paths break only after `/`, wide tables step out of the column in A, and phones get cards. Only a table too wide for all of that should scroll, and it then loses the sticky header.
2. **Phone cards need each cell to carry its column name** (`data-label` here). The renderer can write it, or the page script can add it.
3. **One diagram drawing can serve both themes.** The mockups remove Mermaid's `<style>` block and colour the diagram with page CSS, through 6 classes: `label-container`, `flowchart-link`, `arrowMarkerPath`, `nodeLabel`, `edgeLabel`, `labelBkg`. A merman flowchart from the Mermaid spike carries the same 6 classes and one `<style>` block.
4. **Page CSS leaks into Mermaid's labels.** Labels are HTML inside the SVG. Without a reset to 16 px, line height 1.5, no margins and normal letter spacing, A's 19 px text would overflow every label box.
5. **Word marks lose code and links.** The engine marks edited blocks on plain text, so code spans and links lose their look there, and lines grew to 86 characters before the views were narrowed. Added, removed and moved blocks were drawn from their Markdown source instead, using the byte offsets in the change list. Mapping marks onto the rendered markup would fix edited blocks.
6. **The heading path alone cannot place a change inside a section.** Every change in 0005 sits under `## Decision`, so a move read "from Decision to Decision". The mockups add the bold label above a list, such as "Not decided yet". The render lane or the engine needs something similar, or the neighbouring block's text.
7. **Scroll boxes need `tabindex="0"`** so a keyboard can scroll them: code, program output and wide diagrams.
8. **A new document's level is not set by decision 0009.** The mockups give it the highest level among its blocks: "fact" for both new documents here. The fixes lane decides.
9. **Decision 0009 changes the home page a lot.** Without a rules file, all 8 real branches ranked "rule" before 0009 and all rank "fact" after it. `m1/cli` goes from 111 changes to 8, because its new document counts once.

## Which direction I would pick

**C, Review first.**

1. Decision 0001 puts change review first and reading second. C is the only direction whose first screen answers "what changed, and where do I look first?" Its inbox puts the one branch with decision changes on top. Its change page opens with "7 decision changes in 0005. Start there."
2. C already has the place for the notes loop of M3: every change card has "Add a note". "Seen" gives a review routine with a count, which suits checking agent work every day.
3. It meets every rule with room to spare: 17 px everywhere, lines of 64 to 78 characters (62 to 73 with an SF Pro stand-in), contrast of at least 5.18.
4. On the phone, the rail folds into one sticky line with the "seen" count, cards stack, and tables become cards.
5. It uses system fonts: 0 bytes for body text, and SF Pro on his devices.

The cost: C is the busiest of the three. For long reading sessions A is calmer. All three share the same HTML, so A's reading page could later become a reading style inside C with CSS alone. If he spends more time reading than reviewing, A is the better pick.

## What I could not do

- No test in Safari, on an iPhone or on a Mac. Everything ran in headless Chromium on Linux. The Apple fonts were measured with open stand-ins.
- The maintainer has not looked at the pages yet. The coordinator serves `spikes/design/` to him.
- Code highlighting is not designed: code is one colour. The render lane is measuring highlighters.
- Search, notes and the status view are placeholders only.
- No screen reader test beyond axe. Phone cards turn tables into blocks, which some screen readers then stop reading as tables.

## Decisions for the maintainer

1. **The direction:** A, B or C. I recommend C.
2. **The body font:** the system fonts (0 bytes, recommended) or the bundled Literata (42 to 63 KB, the same look on every machine and in exports).
3. **The diagram font**, three options:
   - always the shipped Liberation Sans: 147,036 bytes once per device, exactly decision 0008;
   - the installed Liberation Sans or Arial first: 0 bytes on his devices, with a small, untested risk of labels a pixel too wide;
   - a renamed Latin subset of Liberation Sans: 20,648 bytes, exact widths, and a renaming step at release. This ships a modified copy of the font, so it needs his OK. I recommend this one.
