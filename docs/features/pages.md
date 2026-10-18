---
status: target
date: 2026-10-18
---

# Pages

> Short version: `driftmark serve` shows three kinds of page in the review-first design of decision 0010. The inbox lists the branches whose documents changed and says where to start. A change page has a rail with every change and one card per change, with its level, what happened, how it is drawn and why it ranks there. A reading page shows a document with its status, its sections and the changes waiting on open branches. Every page keeps the same readability rules on a computer and a phone, in light and dark. Diagrams are drawn on the server, code is coloured in the browser, and pages that show the working tree update themselves. Reading and change pages carry the reader's notes and "Seen" marks (`docs/features/notes.md`).

## Every page

The bar at the top stays in view. It holds:

- `driftmark`, which leads back to the inbox;
- the tabs "Inbox" and "This checkout" (the working tree's changes, when the repository has one);
- the search box;
- the "Theme" button, which switches between light and dark. The choice is kept in the browser. Without a choice, the page follows the device.

The search box and the theme button need the page script, so they stay hidden in a browser that runs no scripts. A "Skip to the content" link is the first thing a keyboard reaches.

### Readability rules

| Rule | How the pages keep it |
|---|---|
| Body text 17 px | On every page and every screen. Code is 14 px. |
| About 60 to 80 characters a line on a computer | Text sits in a column of 38 rem. A phone holds about 35 to 45. |
| WCAG AA contrast in light and dark | Every colour is set for both themes, checked with axe-core on every page state. |
| Marks never by colour alone | Deleted words are struck through, inserted words underlined. Levels carry a sign and a word: ◆ Decision, ▲ Rule, ● Fact, ✎ Wording, ¶ Formatting. Sentence rows carry `=`, `~`, `+` or `−`. |
| Table headers stay in view | A table that fits its column keeps its header under the bar while it scrolls. Only a table too wide for the column scrolls sideways, inside its own box, and then its header stays at its top. |
| Tables on a phone | A table of three or more columns becomes one card per row. Each value carries its column's name, which the server writes into every cell as `data-label`. |

### Fonts

- Text uses the device's own fonts: San Francisco on a Mac or an iPhone, Segoe UI on Windows. No font file is loaded for text.
- Diagrams use Liberation Sans, the font the server measures their labels with (decisions 0008 and 0010). The page loads the font file (147 KB) only when it shows a drawn diagram, once per device.
- Nothing is loaded from the internet. Every file comes from the program.

## The inbox

Address: `/`.

1. **This checkout.** The working tree compared with where its branch left the default branch: its highest level, its counts and when they were made. While the server counts, the card says so. It links to `/changes/auto`.
2. **One group per highest level**, decision first: "Decisions changed: review these first", "Rules changed", "Facts changed", "Only wording changed", "Only formatting changed". Inside a group, the newest branch comes first.
3. **One card per branch.** It shows:
   - the branch name and how long ago its last commit was made (the exact time is its tooltip);
   - how many documents and changes it has;
   - a bar with one stretch per level, as long as that level's share;
   - the counts per level, in words;
   - "Start with": the document that ranks first in the branch's change list, with its count at its highest level;
   - "Review", which opens the branch's change page.
4. **What was left out**: branches with no document changes, and branches that share no history with the default branch.

The inbox updates itself when the server has counted the working tree again.

## Change pages

Addresses: `/changes/<range>` for a range, `/changes/<range>/-/<path>` for one document.

### The rail

On a computer the rail sits on the left and scrolls on its own. It lists:

- the range's name and its counts;
- one chip per level with its count;
- every document the page shows in full, highest level first, with its level, and under it every change: a sign for its level, what happened and its first words. A click goes to the change's card;
- "N more documents", which goes to the list of the others.

On a phone the rail folds into one line under the bar, "Every change", which opens the list.

### The page

1. The range, where it was compared from, and a link to its JSON.
2. **What needs your eyes**: the highest level, how many changes of it the first document has, and a link to it ("Start there"), then the counts of the other levels.
3. **One section per document**: its title (a link to its reading page at the newer side), its path, its level and counts, and a status change when there is one. Then its cards.
4. A range page shows documents in full, highest ranked first, up to 200 KB of cards (about 40 KB on the network), 300 changes and 40 documents; a document that would pass a limit is skipped for the next, among the first 80. The others are listed with their counts, each linking to its own page: at most 300 of them, the rest in the range's JSON. A branch's review fits; a month of a large repository still opens at once.

### A card

Every change is a card. Its first line says:

- the level, as a badge with a sign and a word;
- what happened: Edited, Added, Removed, Moved, Moved and edited, Formatting only, New document or Deleted document;
- how it is drawn: Word marks, Side by side, Sentence by sentence, Rewritten or Line by line;
- why it ranks there, in words, such as "in a decision section · a number or a name changed";
- "Seen" and "Add a note", which work when the notes file can be read (`docs/features/notes.md`). "Seen" turns the card pale and dashed and counts it in the rail; "Add a note" notes the words selected in the card, or the whole changed block.

Under it: the headings above the changed block ("in Decision › Not in version 1"), then the change itself:

| Change | What the card shows |
|---|---|
| Word marks | The text once, with deleted words struck through and inserted words underlined. |
| Side by side | "Before" and "After", each with its own marks. On a phone they stack. |
| Sentence by sentence | One row per sentence, marked `=` kept, `~` edited, `−` deleted or `+` inserted. An edited row shows its old and its new sentence. |
| Rewritten | "Before" and "After", whole, without marks: too little was kept for marks to help. |
| Line by line | Code and diagrams: each line kept, deleted (`−`) or inserted (`+`). |
| Added or removed block | The block whole, with a green or red line beside it. |
| Moved | "Moved here from old line 12", linking to a dashed place-holder where the block was, which says "Moved to new line 40" and links back. |
| Formatting only | Runs of them fold into one line, "3 formatting-only changes", which opens to show each. |
| New or deleted document | One card with the document's title, summary and length. Its blocks are not listed: open the reading page for them. |

A table row in a card is plain text with its cells apart, as the change list gives it.

## Reading pages

Address: `/doc/<revision>/-/<path>`; `worktree` is the files on disk.

1. **Changes waiting.** When open branches change this document, a box on top lists up to six of them, newest first: the branch, its highest level for this document, its count and its time. Each links to the document's changes on that branch. A branch is left out when the page shows that branch itself.
2. **The path and the revision**, with a link to the JSON.
3. **"On this page"**: the document's sections (levels 2 and 3), each with a count of the changes waiting in it. On a large screen it sits beside the text and stays in view; on a smaller one it is a folded list above the text.
4. **The document**:
   - its title, then its **pills**: its type from the rules file, its status, its lifecycle (current, superseded, archived), its date and what superseded it. When the rules file names no place for the status or the date, the document's own `status` and `date` fields are used;
   - its summary, the quote under the title, in larger text;
   - its sections. A heading with changes waiting carries a flag, "3 changes wait", that links to the first branch's changes.

What the document holds:

| Markdown | The page |
|---|---|
| Code with a language | A card with the language on top, coloured by Prism. |
| `text`, `console`, `output` or `log` | Program output: its long lines wrap instead of scrolling. |
| Mermaid | Drawn on the server. When the server cannot draw it, the browser draws it with mermaid.js; the page loads mermaid.js only then. |
| D2 | Drawn on the server when the `d2` program is installed. Otherwise the code stays, marked "Diagram, not drawn yet". |
| Alerts | A box with a sign and a word: Note, Tip, Important, Warning or Caution. |
| Task lists | Ticked and unticked boxes, which cannot be changed here. |
| Links to Markdown files | Links to the same revision. A link from the repository root, such as `/docs/a.md`, works too. |
| Links to other files, and images | Shown as their text, with a dotted underline: the server shows only Markdown documents. |

A page with `#k.<n>` at the end of its address scrolls to block `n` and marks it. Search results use this.

## Diagrams

`driftmark::diagram::Drawer` is the program's `DiagramRender` (`docs/features/engine/html.md`).

- **Mermaid** is drawn by merman, which measures text with the four Liberation Sans faces the program carries. They are written once to the user's cache folder (`~/.cache/driftmark/fonts-<hash>/` on Linux, `~/Library/Caches/…` on a Mac, `%LOCALAPPDATA%` on Windows), because merman reads fonts only from files. When they cannot be written, merman is not used and the browser draws every diagram.
- One drawing may make up to 1,048,576 text measurements, against merman's default of 4,096, so large ER diagrams draw (merman issue 207). One drawing may take 10 seconds; a slower one is left to the browser.
- **D2** goes to the `d2` program on standard input, with a 10-second limit. A drawing that fails or takes longer leaves the block undrawn.
- **Every drawing is cleaned** before it goes into the page. It is read as XML and written out again from an allow-list: shapes, text, groups, markers, gradients, filters and labels pass; scripts, style blocks, animations, images, frames and event attributes do not. Every reference must point inside the drawing, and every id starts with the drawing's own prefix. `style` attributes, which the page's security policy blocks, become presentation attributes on shapes and a few fixed classes on labels. A drawing that is not one well-formed SVG is refused, and the browser draws the diagram instead.
- **One drawing serves both themes.** The drawing's own style block is removed, and the page's style sheet colours the classes merman uses, in light and in dark. Labels are reset to the size the server measured them at (14 px, line height 1.5), so the page's text styles cannot make them overflow.
- **Drawings are kept** by a hash of the diagram's language and text, up to 32 MB. The same diagram in another document, or after the page was reloaded, costs nothing.
- **In the browser**, mermaid.js draws with the same font. Its style block goes into a style sheet the script builds, which the page's security policy allows. A drawing made in the browser is drawn again when the theme changes.

## Code colours

Prism 1.30.0 colours code in the browser. The program carries Prism's core and 59 language files. A page loads the core and the languages its code uses, with the languages they build on, and nothing on a page without code. Common names are understood: `sh`, `yml`, `ts`, `py`, `rs`, `golang`, `tf` and others. Program output (`text`, `console`, `output`, `log`) and languages Prism does not know stay plain. Code inside change cards is not coloured, so its marks stay readable.

## Live updates

A page that shows the working tree (a document at `worktree`, the `auto` range) listens to `/events`. When a file changes, the page fetches itself again and swaps its content in place, keeping the scroll position. A reading page reacts only to its own file. The inbox swaps its content when the working tree was counted again.

## The search box

The search box asks `/api/search` (the `search` lane; its JSON is in `docs/features/commands/serve.md`) as the reader types, from the second letter on. It searches the revision the page shows, or the working tree on other pages. Up to 10 results drop down under the box: the title, the path and the headings above the best block, and a snippet with the matched words marked. A result opens the reading page at the block that matched. Enter opens the first result, the arrow keys move through them, and Escape closes the list. A server without search says so in the box.

## Page weight

Measured on 2026-10-18 with the release build. "First visit" is a fresh browser; "repeat visit" opens the page again in the same browser, where the style sheet, the script, Prism and the font come from the browser's cache. The bytes are what crossed the network, compressed.

On this repository's own documents and history, at 1440 px, in KB:

| Page | First visit | Of it: style sheet and script | Of it: diagram font | Repeat visit |
|---|---|---|---|---|
| Inbox | 20.6 | 17.8 | 0 | 2.8 |
| Reading page with tables and code | 31.3 | 17.8 | 0 | 8.8 |
| Reading page with a drawn diagram | 168.7 | 17.8 | 143.3 | 7.6 |
| A branch's changes, 126 changes in 14 documents | 39.1 | 17.8 | 0 | 21.4 |
| One document's changes, 45 changes | 27.5 | 17.8 | 0 | 9.8 |

- Notes add to reading and change pages, measured after gzip on 2026-10-18: `notes.js`, 9.0 KB, once per device, and 1.3 KB more style sheet and script. `anchor.js` (4.8 KB) and the position tables (a median of 1.9 KB on the private corpus, `docs/features/engine/html.md`) load only when the reader starts a note or the page has notes to show. The largest page above stays near 50 KB.
- A page with code also loads Prism: 4.7 KB for TOML and Rust, 6.5 KB with Bash added.
- A page with a diagram the server could not draw also loads mermaid.js, 1,531 KB, once per device.
- A drawn diagram travels inside the page's HTML: the private corpus's drawings take a median of 7 KB each after compression (`docs/working/version-1/spikes/mermaid.md`).

### The limit

**A first visit stays under 100 KB on the network, not counting drawn diagrams, the diagram font and mermaid.js.** The style sheet and the script take 17.8 KB of it, once per device. A repeat visit loads only the page's HTML.

- Every page of this repository meets it, the largest at 39.1 KB.
- On the private corpus (below), every inbox, range page and document change page meets it, and every reading page but 6 of 2,945. Those 6 show the corpus's largest documents, whose HTML alone is 85 to 192 KB after compression: the page cannot be smaller than the document.
- A range's page shows its documents in full only up to 200 KB of cards, 300 changes and 40 documents, lists at most 300 more, and leaves the rest to its JSON, so a month of a large repository still fits.

### On the private corpus

Measured on 2026-10-18 with the release build serving the maintainer's private corpus (a company repository of about 3,400 documents) on `127.0.0.1`, at a load of 30 to 50 on 12 cores, 6 requests at a time.

| What | Result |
|---|---|
| The 30-day range | 2,945 changed documents, 15,260 changes |
| Their change pages and reading pages | 5,890 of 5,890 answered 200, in 28 seconds |
| One document's change page | median 1.6 KB, 95th percentile 5.8 KB, largest 75 KB; median 2.2 ms, slowest 206 ms |
| A reading page | median 6.6 KB, 95th percentile 23 KB; median 27 ms, 95th percentile 162 ms, slowest 1.7 s (the first drawing of the largest ER diagram) |
| A reading page without its drawn diagrams | median 6.3 KB, 95th percentile 21.9 KB, 99th percentile 41.5 KB, largest 192 KB |
| The inbox, 148 branches | 14.5 KB; 0.54 to 0.72 s for the first request, 16 to 160 ms after |
| A branch's change page, all 148 | median 6.3 KB, largest 40.7 KB; median 10 ms, slowest 202 ms |
| The 30-day range's page | 33.3 KB: 2 documents in full (220 cards), 300 more listed; 0.57 s for the first request, 7 to 11 ms after |
| Mermaid blocks | 195 of 195 drawn on the server, none left to the browser, the 70,000-character ER diagram included |
| Drawn SVG on a page, after cleaning | median 54 KB, largest 4.8 MB, before compression |

Every number is about pages, not documents: no path, title or text of the corpus is recorded here.

## Not covered yet

- **A search page.** Results show only in the box.
- **Images and other files** in a document show as text; the server serves only Markdown.
- **Safari** was not tested; every check ran in Chromium on Linux. The layout uses `:has()` (Safari 15.4 and later) and constructed style sheets for the browser's diagrams (Safari 16.4 and later).
