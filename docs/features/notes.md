---
status: target
date: 2026-10-18
---

# Notes

> Short version: on any reading page or change page, the reader selects a sentence and adds a note: what they think of it, what they want done, whether it blocks the work, and their point. The note shows under its sentence, which is highlighted, and it stays on that sentence while agents edit the document. Each page with notes has a review list, with an "Unanchored" tab for notes whose sentence is gone. An agent reads and answers the notes through `driftmark mcp`; "Copy as prompt" and "Write review file" hand a review to an agent without it. Either way the round has two halves: discuss, then apply what the reader decided. Once an agent records the commits that applied a note, the change page marks the changes they made: "Changed because of note n12". Notes live in one file in the clone's shared git folder, which git never commits.

## Making a note

1. Select words in a document on a reading page, or in a change card on a change page. A button, "Add a note", appears under the selection.
2. Press it. A form opens with the selected words on top. It asks for:
   - **your stance**: ✓ Agree, ✗ Disagree, ? Question or ✎ Suggestion;
   - **what should happen**: Change it, Explain it, or Keep it;
   - **blocking**: tick it when the work should wait until the note is settled;
   - **your point**, in your own words.
3. Press "Save the note". The note appears under its paragraph, list item, table or code block, and the selected words are highlighted.

On a change page, every card's "Add a note" button works too. With words selected in the card, it notes them; with nothing selected, it notes the whole changed block. On the card of a new or deleted document it opens the document's reading page, where its text can be selected. On a card that shows both versions, words selected in struck-through text belong to the old version; everything else belongs to the new one.

The note keeps the exact Markdown of what was selected, its place and its headings, at the commit the page showed, or the working tree as it was on disk.

## Seeing notes

- **In place.** A note shows as a card under its block: its number (`n12`), stance, ask, whether it blocks, and its status; the quoted words; the reader's point; the thread of replies; any proposal and decision; and the commits that applied it. Its words are highlighted in the text, with a solid underline, or a wavy one when the text changed since.
- **After edits.** Every time a page opens, driftmark looks for each note's words again in the version the page shows. The card says what it found:

  | The card says | What happened |
  |---|---|
  | nothing | The words are unchanged, in the same place. |
  | "Moved since the note" | The words are unchanged, in another section or a moved paragraph. |
  | "Found with small differences" | A few characters changed. The card shows the words as they read now. |
  | "The text changed after this note" | The paragraph is still there, but the noted words were rewritten. The note sits on the paragraph, and the card shows what it says now. |

- **Unanchored.** A note whose words and paragraph are both gone moves to the review list's "Unanchored" tab. It is never deleted. It still shows where it was made.
- **On a change page,** a note shows inside the card of its block, on the side it belongs to. A note on words this range deleted shows on the old side. Notes on paragraphs the range did not change are listed above the document's cards.

## The review list

A page with notes shows "Notes" above the document, or above the change cards. It has two tabs:

- **On this page**: the notes, grouped by review, each with a link to its card. A document's reading pages share one review; a range's change pages share another.
- **Unanchored**: the notes whose text is gone from this version.

Under the list, a line says where the notes file is.

## Handing notes to an agent

Each review in the list has three buttons:

| Button | What it does |
|---|---|
| Copy as prompt | Round 1. Copies the review's notes under discussion as a prompt: the agent answers each one and edits nothing. Open notes become "sent". |
| Copy decided notes | Round 2. Copies the notes the reader decided, with each decision: the agent applies them and names the commits. |
| Write review file | Round 1 as a Markdown file, `r<number>.md` in the `reviews` folder next to the notes file. Tell the agent to read it. |

A browser on another device, over plain HTTP, may not let the page copy. The prompt then opens in a box to copy by hand.

The prompt's format is in `docs/features/engine/notes-model.md` ("The prompt packet").

### Through the MCP connector

An agent with `driftmark mcp` (`docs/features/commands/mcp.md`) reads and answers the notes itself, in the same notes file, while `driftmark serve` runs. A whole round:

1. The reader writes notes on a branch's change page or a reading page.
2. The agent calls `notes_list` and sees them. `note_get` gives one note with where its words are now and the section around them; `review_get` gives a review as the same prompt "Copy as prompt" copies.
3. The agent answers each note with `note_reply`, with a stance: agree, counter, question or proposal. It may suggest what to do with `note_propose_decision`. The note becomes "answered", and the reply shows on its card when the page reloads.
4. The reader replies or decides on the page. A proposal fills in the decision form.
5. The agent reads round 2 (`review_get` with round 2), edits the documents as decided, commits, and records the commits with `note_mark_applied`. The note becomes "applied".
6. The change page shows which changes came from which note (below).

The agent replies under the name its client gives, such as `claude-code`. It never decides, edits or withdraws a note: the connector has no tool for those, and the notes file refuses them from an agent with a sentence that says why. `driftmark mcp install --notes-line` adds a line to `AGENTS.md` or `CLAUDE.md` that tells agents to look for notes before they edit.

## Deciding

Under each note that is not settled:

- **Reply** adds the reader's answer to the thread.
- **Decide** records the decision: change it as the note asks, change it another way, or keep it as it is, with a sentence that says exactly what. Keeping it closes the note. An agent's proposal fills the form in; only the reader decides.
- **Edit** changes the note's stance, ask, blocking flag or text, until it is decided.
- **Withdraw** takes the note back. It stays in the notes file, marked withdrawn.

The loop: open → sent → answered ⇄ discussing → decided → applied, or closed without a change, or withdrawn. An agent replies, proposes and records the commits that applied a decision; it never decides.

## Changed because of a note

When the notes of a range's change page have applied commits inside that range, the page marks what those commits did:

- **On a change card:** a line under its head, "↳ Changed because of note n12", for each card whose block a note's commit wrote, edited, moved or deleted. The note's number is a link to its card.
- **On the note's card:** "Changes it caused on this page", a list of those changes by kind and headings, each a link to its card. A change in a document the page does not show in full links to that document's own change page.

Only the note's own document counts, and only commits the range holds: the newer side has them and the older side does not. A later commit that edits the same block again keeps the mark, because the block is followed from the commit to the range's newer side; a block a commit deleted is followed back to the older side. A new or deleted document is one card, marked when the commit changed any block of it. The rule is `notes::caused` in the core (`docs/features/engine/notes-model.md`).

## "Seen"

Every change card has a "Seen" button. Pressing it marks the change seen for the reader: the card turns pale and dashed, the button shows ✓, and the rail counts "3 of 37 seen". The mark belongs to the change, not to the commit: it stays when an agent adds commits that leave the change as it is, and goes when the change's text moves on. The reader's name comes from `driftmark serve --reader`.

## Where notes live

- In one SQLite file, `driftmark/notes.db`, in the clone's shared git folder: the folder `git rev-parse --git-common-dir` names, usually `.git` at the top of the clone.
- Every worktree of the clone, and every agent working in one, sees the same notes. `driftmark serve` and `driftmark mcp` can use the file at the same time.
- Git never commits the file, so notes never show in history or in the change view.
- **Deleting the clone deletes the notes.** A review file, written with "Write review file", is a copy you can keep elsewhere. `driftmark serve --review-dir <path>` writes review files to another folder.
- Nothing is written before the first note or seen mark: reading pages never makes the file.

## Tests

- `fixtures/notes/`: made-up edits and where each note lands, checked by `crates/driftmark-core/tests/notes_fixtures.rs`.
- `crates/driftmark/tests/serve_notes.rs`: every notes route on small repos, the same-origin rule, and the notes file shared by a linked worktree.
- `crates/driftmark/tests/mcp_notes.rs`: a whole round between the routes and the MCP connector on one notes file, every action an agent may not take, and the size limits of the note tools.
- `tests/browser/notes.mjs`: the loop in Chromium. It selects a sentence on a reading page, saves a note, reloads and finds it highlighted in place; edits the file and finds it exact, then fuzzy, then unanchored; on a change page notes a card's words and marks the card seen; and after an agent applies that note through `driftmark mcp`, finds "Changed because of note" on the card and the change listed on the note. Run `npm ci` in `tests/browser/`, then `node notes.mjs <target>/debug/driftmark [<screenshot folder>]`.
- `crates/driftmark/tests/notes_history.rs` (ignored): notes made at an old commit of a real repository, found again at a newer one, with the counts per state.

## Not covered yet

- **Notes on diagrams and raw HTML.** A drawn diagram's text and the text inside a raw HTML block cannot be selected for a note (`docs/features/engine/html.md`, "Limits"). A note on a whole change card works for them.
- **Keyboard selection.** A selection made with the keyboard shows the same button, but it comes last in the tab order.
- **Sharing notes in a team** (decision 0005).
- **Safari.** Every check ran in Chromium. The highlight uses the CSS Custom Highlight API (Safari 17.2 and later); without it, notes still show under their blocks.
