---
status: target
date: 2026-10-18
---

# Notes model

> Short version: a note is a reader's point on an exact stretch of a document's Markdown source, at the commit where it was made. `driftmark-core` holds the note, the rules of its loop and the anchoring: `notes::Note`, `notes::next_status`, `notes::anchor`, `notes::packet`, `notes::caused` and `notes::history`. A note keeps the history of every action done to it, each with where its request came from, and a reader's action that did not come from the reader's page carries a warning. A note keeps its quote with up to 32 characters of context on each side, its byte offsets, its headings and its block. When the document changes, `reanchor` finds the quote again and gives the note one of five states: exact, moved, fuzzy, outdated or orphaned. An orphaned note is never deleted. The loop moves a note from open through sent, answered and discussing to decided and applied; only the reader decides. A review's notes go to an agent as a prompt or a review file, in two rounds: discuss, then apply. The program keeps notes in one SQLite file (`docs/features/notes.md`, decision 0004).

## The note

`notes::Note` serializes to this JSON, the shape the notes file, the server and the MCP connector share. A made-up example:

```json
{
  "id": 12,
  "review": 3,
  "target": {
    "path": "docs/cache.md",
    "commit": "9af59f93c6614e7cdef58884b0b1255cc757f9c9",
    "blob": "6ea906a72ac9c8bb2b3fae9f2c0ad46fd9a6975d",
    "side": "reading",
    "start": 28,
    "end": 55,
    "quote": {
      "exact": "Keep each page for one hour",
      "prefix": "# Cache pages\n\n## Decision\n\n",
      "suffix": ", then drop it from the cache.\n\n"
    },
    "heading_path": ["Cache pages", "Decision"],
    "block": 2,
    "block_hash": "11c537eca5608914"
  },
  "stance": "disagree",
  "ask": "change",
  "blocking": true,
  "body": "Phones keep pages longer than an hour.",
  "status": "answered",
  "thread": [
    {
      "role": "agent",
      "by": "agent-session-1",
      "at": 1792353000,
      "stance": "counter",
      "body": "An hour matches the old server."
    }
  ],
  "proposal": {"outcome": "modify", "summary": "Ten minutes.", "by": "agent-session-1", "at": 1792353010},
  "decision": null,
  "applied": [],
  "author": "Ana",
  "created_at": 1792352574,
  "updated_at": 1792353010,
  "history": [
    {"action": "made", "role": "reader", "by": "Ana", "at": 1792352574, "source": "page",
     "user_agent": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) Safari/605.1.15"},
    {"action": "send", "role": "reader", "by": "Ana", "at": 1792352900, "source": "api",
     "user_agent": "curl/8.5.0", "warning": "Sent to an agent through the API, not on the page"},
    {"action": "reply", "role": "agent", "by": "agent-session-1", "at": 1792353000, "source": "mcp",
     "user_agent": "agent-session-1 1.0"},
    {"action": "propose", "role": "agent", "by": "agent-session-1", "at": 1792353010, "source": "mcp",
     "user_agent": "agent-session-1 1.0"}
  ]
}
```

| Field | What it holds |
|---|---|
| `id` | The note's number in its notes file. Pages and prompts show it as `n12`. |
| `review` | The review it belongs to, shown as `r3` (see "Reviews"). |
| `target` | What the note was made on (next table). |
| `stance` | `agree`, `disagree`, `question` or `suggestion`. |
| `ask` | What the reader wants: `change` the text, `explain` it, or `keep` it. |
| `blocking` | Whether the reader holds the work until the note is settled. |
| `body` | The reader's point, as plain text. |
| `status` | Where the loop is (see "Statuses"). |
| `thread` | The replies, oldest first: `role` (`reader` or `agent`), `by`, `at`, an optional `stance` (`agree`, `counter`, `question`, `proposal`) and `body`. |
| `proposal` | A decision an agent proposed, or `null`: `outcome`, `summary`, `by`, `at`. |
| `decision` | The reader's decision, or `null`: the same fields. |
| `applied` | The commits that applied the decision: `commit`, `by`, `at`. |
| `author` | The reader's name, from `driftmark serve --reader`. |
| `created_at`, `updated_at` | Seconds since 1970 UTC. The core reads no clock; the program passes the time in. |
| `history` | Every action done to the note, oldest first, with where it came from (see "History"). A note read from JSON without it has an empty one. |

### The target

| Field | What it holds |
|---|---|
| `path` | The document's path from the repository root. |
| `commit` | The commit the page showed, or `null` for the working tree. |
| `blob` | The blob id git gives the exact bytes the offsets count in. A version with the same `blob` needs no re-anchoring. For the working tree it is computed from the bytes on disk, and the program keeps those bytes in the notes file, because git does not hold them. |
| `side` | `reading` for a reading page; `old` or `new` for the two sides of a change page. |
| `start`, `end` | The quote's source bytes, UTF-8, end exclusive. |
| `quote.exact` | The source text from `start` to `end`, markup included. A reading page's selection maps tight: a selected bold word is `bold`, not `**bold**` (decision 0008). |
| `quote.prefix`, `quote.suffix` | Up to 32 characters of source just before and after the quote (`anchor::CONTEXT`). Fewer at the start or the end of the file. |
| `heading_path` | The headings above the quote's first block, outermost first, as in the document model. |
| `block`, `block_hash` | The quote's first block, by index, and its source hash as 16 hex digits. |

`Anchor::capture(document, range)` builds `start` to `block_hash` from a byte range. It refuses an empty range, one outside the source, one that splits a character, one that holds only white space, and one after the last block.

The quote, the context, the position and the version follow the W3C Web Annotation model's quote selector, position selector and state (decision 0004). Unlike a page annotator, driftmark anchors to the Markdown source, the text agents edit and git keeps every version of.

## Statuses

| Status | Meaning |
|---|---|
| `open` | Written, not handed to an agent yet. |
| `sent` | Handed to an agent in a round-1 prompt or review file. |
| `answered` | An agent replied or proposed a decision. |
| `discussing` | The reader replied to an answer. |
| `decided` | The reader decided to change the text, as asked (`change`) or another way (`modify`). |
| `applied` | Someone recorded the commits that applied the decision. |
| `closed` | The reader decided to keep the text (`keep`): closed without a change. |
| `withdrawn` | The reader took the note back. |

`applied`, `closed` and `withdrawn` are final: nothing changes the note after them.

`notes::next_status(current, who, action)` is the only place that moves a note. The rules:

| Action | Who | From | To |
|---|---|---|---|
| Edit the text, stance, ask or blocking flag | reader | any but decided | the same status |
| Send (prompt or review file) | reader | open | sent; any other status stays |
| Reply | agent | any but decided | answered; a decided note stays decided |
| Reply | reader | answered | discussing; any other status stays |
| Propose a decision | agent | any but decided | answered |
| Decide `change` or `modify` | reader | any | decided |
| Decide `keep` | reader | any | closed |
| Record the commits | reader or agent | decided | applied |
| Withdraw | reader | any | withdrawn |

Every other combination is refused with a `Refusal`: the note is final, only the reader may do it, only an agent proposes, the note is decided already, or only a decided note can be applied. `src/notes/loop.expected.txt` lists every status, action and role with its answer.

## History

`notes::history` holds a note's history (decision 0012). Each entry, `notes::Event`:

| Field | What it holds |
|---|---|
| `action` | `made`, `edit`, `send`, `reply`, `propose`, `decide`, `applied` or `withdraw`. Every decision is `decide`, whatever its outcome; `EventAction::of` maps a loop action to its entry. |
| `role` | `reader` or `agent`. |
| `by` | The reader's name, or the agent's client. |
| `at` | Seconds since 1970 UTC. |
| `source` | Where the request came from: `page` (the reader's own page), `mcp` (the connector), `api` (any other request with the token) or `unknown` (recorded before driftmark kept sources). The program decides it; `docs/features/commands/serve.md` ("Where an action came from") says how. |
| `user_agent` | What the sending program calls itself, cut to 160 characters: a browser's or a tool's `User-Agent`, or the MCP client's name and version. Left out when it gave none. |
| `warning` | A sentence when the entry should be looked at; left out otherwise. |

`notes::warning(action, role, source)` is the only rule that makes a warning, and `Event::new` applies it, so the page and the connector say the same thing:

- an agent's action never warns: replying, proposing and recording commits through the connector is the loop;
- a reader's action from the page never warns;
- any other reader's action warns, with the action and the source in plain words.

| Action | Through the API | Through the connector | Before sources were kept |
|---|---|---|---|
| `made` | Made through the API, not on the page | Made through the MCP connector, not on the page | Made before driftmark recorded where actions come from, so it may not have come from the page |
| `edit` | Edited … | Edited … | Edited … |
| `send` | Sent to an agent … | Sent to an agent … | Sent to an agent … |
| `reply` | Replied … | Replied … | Replied … |
| `propose` | Proposed … | Proposed … | Proposed … |
| `decide` | Decided through the API, not on the page | Decided through the MCP connector, not on the page | Decided before driftmark recorded where actions come from, so it may not have come from the page |
| `applied` | Marked applied … | Marked applied … | Marked applied … |
| `withdraw` | Withdrawn … | Withdrawn … | Withdrawn … |

The `…` stands for the rest of the sentence in the same column. `src/notes/history.expected.txt` lists every action, role and source with its warning. The connector has no tool for a reader's action, so the middle column shows only if the program is changed to allow one.

The warning is worked out each time a note is read, from the action, the role and the source; the notes file keeps only those three. In the notes file, a version 1 file brought up to version 2 gets the actions it can show with the source `unknown`: the making of each note, each reply, the last proposal, the decision, each set of recorded commits (as the reader's when the name is the note's author, otherwise an agent's), and a withdrawal. Edits and sends left no trace before, so an old note's history has none.

## Anchoring

`reanchor(anchor, origin, document)` finds a note in a version of its document. `origin` says what is known of the version the note was made on:

| `Origin` | When |
|---|---|
| `Same` | The version has the note's `blob`: the note stays where it is. |
| `Known(old)` | The old version can be read. |
| `Paired(old, map)` | The same, with the old version's blocks already paired with the new one's by `diff::block_map`. A page with many notes on one version pairs once. |
| `Lost` | The old version cannot be read: only the quote, its context and its headings are left. |

It tries, in order, and stops at the first match it accepts:

1. **The same bytes.** With `Same` and the quote still at its offsets, the note is exact.
2. **The block diff.** The note's blocks are followed to the blocks the change list pairs them with (`diff::block_map`, the pairing of `docs/features/engine/change-list.md`). The quote is looked for there, expected at the same offset into the block.
3. **The section.** The blocks under the note's own headings, from the first to the last.
4. **The whole document.**

Each search looks for the quote exactly first. Without an exact match, it searches with Myers' bit-vector algorithm for the stretches with the fewest edits, at most half the quote's characters and at most 256 (`anchor::MAX_ERRORS`). A fuzzy match that starts or ends inside a word takes the whole word, so "two days" found in "three days" shows as "three days", not "ree days".

### Scoring

Each match gets a score from 0 to 1, after the Hypothesis client's quote matcher (`match-quote.ts`, BSD-2-Clause; driftmark's code is its own):

| Part | Weight | Similarity |
|---|---|---|
| The quote | 50 | 1 minus the edits over the quote's length |
| The text before it | 20 | 1 minus the fewest edits between the prefix and the text before the match, over the prefix's length |
| The text after it | 20 | the same for the suffix |
| The place | 2 | 1 minus the distance from the expected byte over the document's length |

The best-scored match of a search wins; a tie keeps the earlier one. A match is accepted when the quote's similarity is at least 0.75 (`ACCEPT_QUOTE`), and, for a quote shorter than 32 characters (`LONG_QUOTE`), when the whole score is at least 0.75 too (`ACCEPT`). A short quote such as one word occurs in many places; its context decides. Hypothesis accepts any match; driftmark refuses weak ones, because a note on the wrong sentence is worse than an unanchored one.

### States

| State | When | Where the note sits |
|---|---|---|
| `exact` | The quote is unchanged, in its own block that stayed in place, or elsewhere in its section. | On the quote. |
| `moved` | The quote is unchanged, in a block that moved, or in another section. | On the quote. |
| `fuzzy` | An accepted match with edits: the text changed a little. | On the match. |
| `outdated` | No match was accepted, but the note's block still has a partner in the new version: the text changed after the note. | On what is left of a long quote when its context agrees, otherwise on the whole block. |
| `orphaned` | Neither the quote nor its block was found. | Nowhere; the note is kept and shown apart. |

`Placement` holds the state, the source `range` (absent when orphaned), the `block` it starts in, and a `score`: the quote's similarity, rounded to three decimals, 1 for an unchanged quote and 0 when nothing of it was found.

The golden fixtures in `fixtures/notes/` show each state on made-up edits: a sentence reworded, a paragraph moved, split and merged, deleted text, text rewritten after the note, formatting only, a renamed heading, a lost version, and single words.

### Text selected on a change page

A change card shows a block's plain text, not its rendered Markdown, so a selection there has no position table. `locate_text(document, block, text, hint)` finds the selected text in the block's source instead: exactly when it can, otherwise with the fewest edits, at most half its characters, so markup such as `*` counts as an edit. A card joins a table row's cells with a tab; the search reads a tab as ` | `. `hint`, the selection's start in characters of the card's text, picks between equal matches.

### Notes on the old side of a change

A note made on the old side of a change page, on text the newer version deleted or changed, stays on that old version. It is placed only where that version is shown: on the old side of change pages. A reading page lists it without a place. Any other note on a change page is looked for in the newer version first, and in the older one when its text is gone from the newer.

## Reviews

A review is the group of notes handed to agents together. The program makes one per place notes are made: a document's reading pages share one review (its `kind` is `document`, its `scope` the path), a range's change pages another (`range`, the range as written, such as `origin/main...feature`).

## The prompt packet

`packet::prompt(review, items, round)` writes one round of a review as text to paste into an agent's session. `packet::review_file` writes the same as a Markdown file under a title. Each item carries the note, where it sits in the version the agent will edit (the range's newer side, or the working tree for a document's review), its lines, and the text there when it differs from the quote.

**Round 1, discuss** holds the notes that are open, sent, answered or discussing:

```text
Review r3 · origin/main...feature · 1 note · at 6a4b0aa
Round 1: discuss only. For each note, say whether you agree, disagree or have a question, and why.
Do not edit the documents yet. Answer each note with the note_reply tool, or write your answer on its "Agent:" line.

## 1 · n12 · disagree · blocking · ask: change
File: docs/cache.md, line 5, under Cache pages › Decision
> Keep each page for one hour
The text changed after the note. It now reads:
> Keep each page for ten minutes
Reader: Phones keep pages longer than an hour.
Agent:
```

**Round 2, apply** holds the decided notes, each with the decision:

```text
Review r3 · origin/main...feature · 1 decided note · at 6a4b0aa
Round 2: apply the decided notes. Change each document as its decision says, and nothing else.
Then record each commit with the note_mark_applied tool, or write it on the note's "Commit:" line.

## 1 · n12 · decided: modify
File: docs/cache.md, line 5, under Cache pages › Decision
> Keep each page for one hour
Reader: Phones keep pages longer than an hour.
Agent: An hour matches the old server.
Decision: Ten minutes.
Commit:
```

- The head line names the review, its scope, how many notes and the version the lines count in.
- A moved note says "The quoted text has moved since the note."; a fuzzy or outdated one says "The text changed after the note." with the new text; an orphaned one says "The quoted text is no longer in the document." and gives no lines.
- A note on the old side of a change says so, with the commit it was made at.
- The thread follows the reader's point, one `Reader:` or `Agent:` line per reply, later lines of a reply indented. An agent's open proposal shows as `Proposed:`.
- The tool names, `note_reply` and `note_mark_applied`, are the MCP connector's (decision 0004); an agent without it answers on the `Agent:` and `Commit:` lines.

## Changes a note caused

`notes::caused(changes, around, ends)` says which changes of a range a commit made, so a change page can mark them "Changed because of note n12" for the commits recorded on a note. It reads no git: the program gives it four versions of one document, each parsed the way the range's change list parsed its ends.

| Argument | What it holds |
|---|---|
| `changes` | The document's change list for the range. |
| `around.before`, `around.after` | The document at the commit's first parent and at the commit; `None` where it does not exist. |
| `ends.base`, `ends.head` | The document at the range's two ends; `None` where it does not exist. |

It works in three steps:

1. **What the commit did.** The blocks of `before` and `after` are paired as the change list pairs them (`diff::block_map`). A block of `after` the commit wrote: one without a partner, one whose source differs from its partner's, or one that moved. A block of `before` it deleted: one without a partner.
2. **Where that is in the range.** A written block is followed forward, from `after` to `head`, through the pairing of those two versions. A deleted block is followed back to `base`, through the pairing of `base` and `before`. So a later commit that moves or edits the block again does not hide the first commit's part in it.
3. **Which changes.** A change is the commit's when its new block is one the commit wrote, or its old block is one the commit deleted. A side without a block, such as the new side of a removed block, never counts, whatever place it names. A new or deleted document is one change, and it is the commit's when the commit wrote or deleted any block of it.

A commit that changed only the blank lines between blocks made no change. The answer is the changes' indices, in list order. The unit tests of `src/notes/caused.rs` cover each case with made-up documents: two commits each with its own edit, an edit edited again later, a deletion, a move, a commit that left a block alone, a new and a deleted document, and the two places where a gap's index equals a block the commit touched.

## Seen marks

`notes::change_key(path, change)` gives a change of a change page a key that stays the same while the change does: 16 hex digits of an XXH3 hash of the path, the kind of change and the text on each side. The program keeps each reader's "Seen" marks under these keys. A change seen once stays seen after an agent adds commits that leave it as it is; a change whose text moves on is new again. The notes file keeps each mark with the reader's name, its source (as in "History") and its `User-Agent`; a mark shows no warning.

## Speed

Measured on 2026-10-18 on the maintainer's server with the release build, at a load of 13 to 37 on 12 cores. Each run makes a note on the first sentence of every paragraph, list item and table row at an older commit, and one on a word in it, then finds every note again at a newer one:

| History | Documents | Notes | Re-anchoring, all | Slowest document |
|---|---|---|---|---|
| This repository's `docs/`, 12 commits back | 17, 5 changed | 2,892 | 7 ms | 2.2 ms for 263 notes |
| A private corpus's `docs/`, 7 days back | 3,161, 340 changed | 368,580 | 0.69 s | 99 ms for 568 notes |
| The same, 30 days back | 2,333, 901 changed | 269,530 | 2.9 s | 180 ms for 114 notes |

| History | Exact | Moved | Fuzzy | Outdated | Orphaned |
|---|---|---|---|---|---|
| This repository, 12 commits | 2,858 | 1 | 17 | 12 | 4 |
| Private corpus, 7 days | 366,367 | 114 | 554 | 400 | 1,145 |
| Private corpus, 30 days | 262,458 | 396 | 1,868 | 993 | 3,815 |

- The slowest documents are large ones that agents rewrote: most of their notes fail in their own block and section and are searched for in the whole document.
- Three choices keep it fast. A version's blocks are paired once for all its notes (`Origin::Paired`); without it the slowest document of this repository took 111 ms instead of 2.2. The search reads the document in place and reports bytes, instead of copying it into characters for every note. Characters below 256 find their bit masks by index, not by a hash lookup. The last two cut the private corpus's 30 days from 10.2 s to 2.9 s.
- `driftmark serve` also keeps each note's placement per version, so a page opened again re-anchors nothing.
- The test is `crates/driftmark/tests/notes_history.rs` (ignored; run it with `--ignored --nocapture`). The corpus numbers are counts only.
