---
status: target
date: 2026-10-18
---

# Change list

> Short version: `driftmark-core` compares two versions of one document block by block. It pairs the old blocks with the new ones, says what happened to each (formatting, edited, added, removed, moved, or moved and edited), where it happened in both versions, how an edit should be drawn, and how much it matters. The importance comes from rules, never from AI: each change gets signals, and its level is the highest level among them, from decision down to formatting. A change list gathers the documents of a range and sorts them, highest level first. It is plain data with a JSON form, and that JSON is a public contract with a schema version.

## How it is called

`driftmark_core::diff::compare(path, old, new)` compares the two versions of the file at `path`.

- `old` and `new` are each a `Version`: the parsed `Document` and its `Classification` from the rules file (`docs/features/engine/rules-file.md`). Without a rules file, use `Rules::default().classify(path, document)`.
- Give `None` for a side where the file does not exist. The file is then added or removed, and every block is an added or removed change.
- It returns `DocumentChanges`, or `None` when both sides are `None`.
- It reads nothing and keeps no state, so documents can be compared on many threads at once.

`ChangeList::new(range, documents)` gathers the documents of one range, sorts them for a reader and adds up the totals. `RangeInfo::of(&resolved_range)` turns a resolved git range (`docs/features/engine/git-ranges.md`) into the text the list shows.

## How blocks pair

Blocks come from the document model (`docs/features/engine/document-model.md`). Pairing runs in six steps. Each step pairs only blocks that no earlier step paired.

1. **Skeleton.** Blocks whose content hash and nesting (their containers) are equal are lined up in order, as a longest common subsequence. These pairs did not move. The stretches between two skeleton pairs are the gaps.
2. **Equal content elsewhere.** A block pairs with an unpaired block on the other side whose content hash is equal. A candidate in the same gap is preferred; otherwise the first one in order is taken. Out of order, this is a move.
3. **Front matter** pairs with front matter, whatever its words. A document has at most one.
4. **Table rows by their first cell.** Within one gap, a row pairs with the row whose first cell is equal. The first cell must not be empty and must be written once among the gap's unpaired rows on each side. Header rows pair only with header rows.
5. **The lone pair.** A gap whose only unpaired blocks are one old and one new block of the same kind pairs them. It is the same block, edited, however little of it was kept. This is how `## Status` renamed to `## State` shows as a renamed heading.
6. **Shared words.** The rest pair by the distinct words they share, counted in any order. Reordered sentences still pair. The score is twice the shared words over both blocks' word counts. Words are runs of letters and digits, compared in lower case.

Only blocks of the same kind pair: a paragraph with a paragraph, a list item with a list item, a table row with a table row (header with header, body with body). Code pairs with code and diagrams with diagrams.

After pairing, the pairs inside each gap that run in the same order on both sides stay in place. Among crossing pairs, the longest run in order stays. Every other pair, and every pair whose blocks sit in different gaps, moved.

The values are named constants in `driftmark_core::diff`:

| Constant | Value | Meaning |
|---|---|---|
| `MIN_SHARED` | 0.4 | The score two blocks in the same gap need to pair. `Pages expire after one hour.` and `Pages expire after two hours.` score 0.6 and pair. `The cache keeps pages.` and `The server logs requests.` score 0.25 and do not. |
| `MIN_SHARED_MOVED` | 0.6 | The score two blocks in different gaps need. Such a pair claims a move, so it needs more evidence. |
| `MAX_POSTING` | 64 | Pairing looks up candidates in an index of words. A word held by more than 64 unpaired new blocks, such as `the` in a long document, is not used to find candidates, so a large rewrite does not compare every block with every block. Candidates are still scored on all their words. A document with at most 64 unpaired new blocks is compared in full. |
| `MAX_CELLS` | 4,000,000 | The largest table the in-order alignment fills, for blocks, words, lines or sentences. A larger stretch is first split at items that occur once on each side, as patience diff does. A stretch with no such item is left unaligned: all removed, all added. |

## Change kinds

| Kind | JSON | When |
|---|---|---|
| Unchanged | (not listed) | A pair in place with equal source bytes, content and nesting. Its indexes are in `unchanged`. |
| Formatting only | `formatting` | A pair in place with equal content but other source bytes or other nesting: wrapping, emphasis style, list markers, numbering, an item indented under another. |
| Edited | `edited` | A pair in place whose content changed. |
| Added | `added` | A new block with no pair. |
| Removed | `removed` | An old block with no pair. |
| Moved | `moved` | A pair out of place with equal content. |
| Moved and edited | `moved_edited` | A pair out of place whose content changed. |

A reference link takes its target from a definition elsewhere in the file. Editing the definition changes the content of every block that uses it, while their source bytes stay the same. Such a block is edited, and its link signal shows the new target.

A file whose bytes changed only between blocks, such as blank lines, has no changes. All its blocks are unchanged.

## Where each change is

Every change has an `old` side and a `new` side.

- `at` is the block's index in that version when the version has the block.
- When it does not, `at` is the place the block would be: the index of the block it would come before, or the number of blocks when it would come last.
- `block` is there only when the version has the block. It holds the block's kind, its source bytes (`start`, `end`), the line it starts on, its heading path and its plain text.

The changes are listed in reading order: by `new.at`, never decreasing.

- An edited, formatting or moved block is listed at its place in the new version.
- A removed block is listed where it used to be. Its `new.at` is the place in the new version just after the last block that stayed in place before it.
- Within one gap, removed blocks come first, then the new side's blocks in their order.

So a block inserted between two edited blocks is listed between them. A moved block is listed at its new place, and `old.at` names where it came from.

## Views for an edited block

An edited block that is not code is drawn by the share of its words kept in the same order, measured on the longer version. Words are the word tokens below, code spans, links and images; white space and punctuation do not count. Two versions without any word count as all kept.

| Kept in order | View | JSON | What the reader sees |
|---|---|---|---|
| 0.85 or more (`INLINE_MIN_KEPT`) | Inline | `inline` | One text with deleted and inserted words marked. |
| Below 0.35 (`REWRITTEN_BELOW`) | Rewritten | `rewritten` | The old and the new version, each whole, without marks. |
| In between, both sides plain prose with at least 4 sentences (`SENTENCE_VIEW_MIN`) | Sentence by sentence | `sentences` | One row per sentence, each with its word marks. |
| In between, otherwise | Side by side | `side_by_side` | The old and the new version next to each other, words marked. |
| Code and diagram blocks, whatever the share | Lines | `lines` | One row per line: kept, deleted or inserted. |

The three values come from the private prototype. The view is chosen on the exact share; `kept_in_order` in the JSON is rounded to three decimals.

Plain prose means a paragraph, a list item, an alert or a footnote definition.

The sentence view lines up the sentences of both versions by their text. Between two lined-up sentences, the remaining old and new sentences pair in order. Each pair gets word marks; a sentence left over is one deleted or one inserted row.

A sentence ends after `.`, `!`, `?` or `…`, and any closing quotes or brackets, when white space follows and the next word does not start with a lower-case letter. So `e.g. this` stays one sentence. The full-width stops `。`, `！` and `？` end a sentence without white space. The empty line between two paragraphs of one list item always ends a sentence. Each sentence keeps the white space after it, so the rows together spell the whole text.

An edit can change a block's kind details while its words stay equal: a heading's level, a task's checkbox, a bullet list turned numbered, an alert's kind. Its marks then show no change. The `kind` on each side says what changed.

## Word marks

The plain text is cut into tokens. Joined again, the tokens give the text back exactly.

- **Words:** runs of letters, digits and `_`, with the combining marks that sit on letters. A `.`, `,`, `:`, `/`, `-`, `'`, `’` or `@` between two word characters joins them, so `0.85`, `2026-10-18`, `read-only`, `don't` and `docs/a.md` are one word each.
- **Kept whole:** each link, image and code span is one token, whatever words it holds. A link whose target changed but whose text did not is one deleted and one inserted token with the same text. The document model's `spans` say where they are.
- **White space:** a run of white space is one token. Runs of spaces match each other whatever their width; a tab (between table cells) and a line break match only their own kind.
- **Other characters:** each punctuation mark or symbol is one token, with the characters that join it: variation selectors, skin tones, a zero-width joiner and the character after it, the second half of a flag.

The tokens of both versions are lined up in order. Then white space that is kept between two changes becomes part of both, so `the quick brown` replaced by `a slow red` reads as one change, not three. Each run of changes is one deleted mark followed by one inserted mark.

**The guard.** The kept and inserted marks, joined, must equal the new block's plain text exactly. If they do not, the marks are dropped, the view becomes side by side, and `fallback` is true. Because the tokens always join back into the text, the guard only fails if driftmark has a bug. On the private corpus it never failed.

Code and diagram blocks are compared line by line. A table row's `cells` lists the positions of the cells whose text changed, from 0; a cell only one row has counts as changed.

## Signals and levels

Ranking follows rules only (decision 0005). Every change gets one or more signals. Its level is the highest level any of its signals gives.

The levels, highest first: **decision**, **rule**, **fact**, **wording**, **formatting**.

| Signal | JSON | Level | When it applies |
|---|---|---|---|
| Status | `status` | decision | The document's status changed, and this block holds it. |
| Decision section | `decision_section` | decision | The block sits, on either side, in a section the rules file marks as a decision. Not for formatting-only changes. |
| Normative word | `normative` | rule | A word or phrase from the normative list came or went. An added or removed block gets it when it holds one. |
| Heading | `heading` | fact | A heading was added, removed, renamed or moved to another level. |
| Number | `number` | fact | A number, date, version or identifier came or went: a word holding a digit, or a code span. |
| Link | `link` | fact | A link or image target came, went or changed. |
| Table cell | `table_cell` | fact | A table row was added, removed or edited. |
| Code line | `code_line` | fact | A code or diagram block was added, removed or edited, its language included. |
| Moved | `moved` | wording | The block moved. |
| Wording | `wording` | wording | Words changed, and none of status, normative, heading, number, link, table cell or code line applies. |
| Formatting | `formatting` | formatting | Only formatting changed. It is the only signal of a formatting change. |

How the signals compare two versions of a block:

- **Normative words** are counted per word or phrase on each side; a different count is the signal. Moving `must` within a sentence is not.
- **Numbers and identifiers** are counted per token text on each side; a different count is the signal. `30 days, then 7` turned into `7 days, then 30` has the same counts, so it is not.
- **Link targets** are counted per target on each side, links without text included.
- Code and diagram blocks get only the code-line signal from their content; their words are not searched for numbers or normative words.

**The status signal.** For a modified document, the status comes from the rules file on each side. When it changed (ignoring case and the white space around it), the document's `status` holds both values. The signal goes to the first change whose old block holds the old status or whose new block holds the new one, compared without regard to case. It replaces that change's wording signal.

**The normative list** lives in one place, `NORMATIVE_WORDS`, and is compared without regard to case:

`must not`, `should not`, `shall not`, `must`, `never`, `always`, `only`, `required`, `forbidden`, `should`, `shall`.

The brief named the nine without `should not` and `shall not`. Those two are added so that `should` turned into `should not` counts, like `must` turned into `must not`. Words inside code spans and links do not count.

## A document's summary, and the order of documents

Each document has a `summary`:

- `changes`: how many changes it has;
- `highest`: the highest level among them, or `null` when it has none;
- `levels`: how many changes have each level;
- `kinds`: how many changes are of each kind.

`file` says whether the file was `added`, `removed` or `modified`.

A change list sorts its documents for a reader:

1. the highest level first, a document without changes last;
2. then the most changes first;
3. then by path, in byte order.

`totals` adds up every document's summary. Its `highest` is the highest level in the whole list.

## The JSON contract

The change list serializes with `serde`. The shape below is a public contract: the MCP connector (M3), the change pages (M2) and the merge-request diff (M5) read it.

- `schema_version` is 1. It goes up by one when a field changes meaning or is removed. Adding a field keeps it.
- Names are in snake case: fields, kinds, levels, signals, views and mark operations.
- A block kind is an object whose `type` names the kind, with its details beside it, as the document model states them: `{"type": "heading", "level": 2}`, `{"type": "list_item", "depth": 0, "ordered": false, "number": null, "task": "done"}`, `{"type": "table_row", "header": false, "cells": ["Home", "10 min"]}`.
- Left out when empty: `range`, a document's `status`, a side's `block`, a change's `edit`, and an edit's `marks`, `sentences`, `lines`, `cells` and `fallback`.
- `unchanged` lists `[old index, new index]` for each unchanged block. Pages use it to line up the two versions, and notes to follow a sentence to the next version.
- Byte offsets (`start`, `end`) index the version's source text, in UTF-8 bytes.

| Object | Fields |
|---|---|
| change list | `schema_version`, `range` (`base`, `head`, `notes`), `totals`, `documents` |
| document | `path`, `file`, `status` (`old`, `new`), `summary`, `changes`, `unchanged` |
| summary | `changes`, `highest`, `levels` (`decision`, `rule`, `fact`, `wording`, `formatting`), `kinds` (`formatting`, `edited`, `added`, `removed`, `moved`, `moved_edited`) |
| change | `kind`, `level`, `signals`, `old`, `new`, `edit` |
| side | `at`, `block` (`kind`, `start`, `end`, `line`, `heading_path`, `text`) |
| edit | `view`, `kept_in_order`, `marks`, `sentences`, `lines`, `cells`, `fallback` |
| mark | `op` (`kept`, `deleted`, `inserted`), `text` |

`range.head` is a commit id, or `worktree` for the files on disk. `range.base` is `null` before the first commit.

### A made-up example

A decision record changes in an agent's working tree. Before:

```markdown
---
status: proposed
---

# Cache rendered pages

## Decision

Keep each rendered page in the cache for 60 minutes.

## Notes

The cache lives in memory.
```

After:

```markdown
---
status: accepted
---

# Cache rendered pages

## Decision

Keep each rendered page in the cache for 90 minutes.

Never cache a page with open notes.
```

The rules file says that files under `docs/decisions/` keep their status in front matter, and that `## Decision` opens a decision section. The change list, as `tests/diff_json.rs` checks it against this text:

```json
{
  "schema_version": 1,
  "range": {
    "base": "a1b2c3d4e5f60718293a4b5c6d7e8f9012345678",
    "head": "worktree",
    "notes": [
      "compared with `origin/main`"
    ]
  },
  "totals": {
    "changes": 5,
    "highest": "decision",
    "levels": {
      "decision": 3,
      "rule": 0,
      "fact": 1,
      "wording": 1,
      "formatting": 0
    },
    "kinds": {
      "formatting": 0,
      "edited": 2,
      "added": 1,
      "removed": 2,
      "moved": 0,
      "moved_edited": 0
    }
  },
  "documents": [
    {
      "path": "docs/decisions/0002-cache-rendered-pages.md",
      "file": "modified",
      "status": {
        "old": "proposed",
        "new": "accepted"
      },
      "summary": {
        "changes": 5,
        "highest": "decision",
        "levels": {
          "decision": 3,
          "rule": 0,
          "fact": 1,
          "wording": 1,
          "formatting": 0
        },
        "kinds": {
          "formatting": 0,
          "edited": 2,
          "added": 1,
          "removed": 2,
          "moved": 0,
          "moved_edited": 0
        }
      },
      "changes": [
        {
          "kind": "edited",
          "level": "decision",
          "signals": [
            "status"
          ],
          "old": {
            "at": 0,
            "block": {
              "kind": {
                "type": "front_matter"
              },
              "start": 0,
              "end": 24,
              "line": 1,
              "heading_path": [],
              "text": "status: proposed"
            }
          },
          "new": {
            "at": 0,
            "block": {
              "kind": {
                "type": "front_matter"
              },
              "start": 0,
              "end": 24,
              "line": 1,
              "heading_path": [],
              "text": "status: accepted"
            }
          },
          "edit": {
            "view": "side_by_side",
            "kept_in_order": 0.5,
            "marks": [
              {
                "op": "kept",
                "text": "status: "
              },
              {
                "op": "deleted",
                "text": "proposed"
              },
              {
                "op": "inserted",
                "text": "accepted"
              }
            ]
          }
        },
        {
          "kind": "edited",
          "level": "decision",
          "signals": [
            "decision_section",
            "number"
          ],
          "old": {
            "at": 3,
            "block": {
              "kind": {
                "type": "paragraph"
              },
              "start": 63,
              "end": 115,
              "line": 9,
              "heading_path": [
                "Cache rendered pages",
                "Decision"
              ],
              "text": "Keep each rendered page in the cache for 60 minutes."
            }
          },
          "new": {
            "at": 3,
            "block": {
              "kind": {
                "type": "paragraph"
              },
              "start": 63,
              "end": 115,
              "line": 9,
              "heading_path": [
                "Cache rendered pages",
                "Decision"
              ],
              "text": "Keep each rendered page in the cache for 90 minutes."
            }
          },
          "edit": {
            "view": "inline",
            "kept_in_order": 0.9,
            "marks": [
              {
                "op": "kept",
                "text": "Keep each rendered page in the cache for "
              },
              {
                "op": "deleted",
                "text": "60"
              },
              {
                "op": "inserted",
                "text": "90"
              },
              {
                "op": "kept",
                "text": " minutes."
              }
            ]
          }
        },
        {
          "kind": "removed",
          "level": "fact",
          "signals": [
            "heading"
          ],
          "old": {
            "at": 4,
            "block": {
              "kind": {
                "type": "heading",
                "level": 2
              },
              "start": 117,
              "end": 125,
              "line": 11,
              "heading_path": [
                "Cache rendered pages"
              ],
              "text": "Notes"
            }
          },
          "new": {
            "at": 4
          }
        },
        {
          "kind": "removed",
          "level": "wording",
          "signals": [
            "wording"
          ],
          "old": {
            "at": 5,
            "block": {
              "kind": {
                "type": "paragraph"
              },
              "start": 127,
              "end": 153,
              "line": 13,
              "heading_path": [
                "Cache rendered pages",
                "Notes"
              ],
              "text": "The cache lives in memory."
            }
          },
          "new": {
            "at": 4
          }
        },
        {
          "kind": "added",
          "level": "decision",
          "signals": [
            "decision_section",
            "normative"
          ],
          "old": {
            "at": 6
          },
          "new": {
            "at": 4,
            "block": {
              "kind": {
                "type": "paragraph"
              },
              "start": 117,
              "end": 152,
              "line": 11,
              "heading_path": [
                "Cache rendered pages",
                "Decision"
              ],
              "text": "Never cache a page with open notes."
            }
          }
        }
      ],
      "unchanged": [
        [
          1,
          1
        ],
        [
          2,
          2
        ]
      ]
    }
  ]
}
```

What it says, change by change:

1. The front matter's status went from `proposed` to `accepted`, so it is a decision change, and the document's `status` holds both values. Half its words were kept, so it is shown side by side.
2. The decision paragraph's number changed inside the decision section: a decision change, shown inline.
3. The `## Notes` heading was removed: a fact change. Its `new.at` of 4 puts it after block 3 of the new version, the paragraph it followed.
4. The paragraph under it was removed: a wording change, at the same place.
5. The new sentence under `## Decision` holds `never`: a decision change, because it sits in a decision section. It is listed after the removed blocks of the same gap, and `old.at` of 6 puts it at the end of the old version.

The title and the `## Decision` heading did not change; they are the two `unchanged` pairs.

## Tests

- `fixtures/diff/<behaviour>/` holds one made-up pair per behaviour: `a.md`, `b.md`, an optional `driftmark.toml`, and `expected.txt`, the change list written out for reading. A missing `a.md` or `b.md` means the file was added or removed. There is a folder for each change kind, each view, each signal, a table with one changed cell, rows paired by their first cell, a move inside a decision section, and the three cases the private prototype got wrong: a new block between two edited blocks (`inserted-between-edits`), a block that moved and was edited (`moved-and-edited`), and an edited list item that marks only itself (`edited-list-item`).
- `tests/diff_fixtures.rs` also checks on every pair that each block of each version is listed exactly once, as a change or as unchanged, that changes come in reading order, and that the marks spell the new text.
- `tests/diff_json.rs` checks the JSON of every type and variant, and the example above.

## Speed on the private corpus

Measured on the server on 2026-10-18 with `crates/driftmark/tests/diff_corpus.rs`, which reads both sides with gix, parses, classifies with no rules file, compares and gathers the list. Each figure is the median of 3 runs. The two final runs ran at a load of about 10 and of about 20 on 12 cores, and the table gives both.

| Range | Changed documents | Blocks (old, new) | 12 threads | 1 thread |
|---|---|---|---|---|
| A 19-commit branch against its merge base | 20 | 2,341, 3,459 | 13 to 33 ms | 33 to 39 ms |
| 30 days of the main branch | 2,652 | 143,811, 178,205 | 180 to 251 ms | 1,754 to 1,790 ms |

The limits in decision 0008 are under 1 second on all cores and under 5 seconds on one core for 30 days, and under 0.1 seconds for a typical branch. On one thread, the 30 days split into about 160 ms reading, 920 ms parsing and 650 ms classifying and comparing. Serializing the 30-day change list gives 95.5 MB of JSON in about 170 to 190 ms, most of it the text of added and removed files.

| Over 30 days | Count |
|---|---|
| Changes | 161,799: 96,119 added, 61,725 removed, 3,836 edited, 70 formatting, 29 moved, 20 moved and edited |
| Levels | 24,504 rule, 107,202 fact, 30,023 wording, 70 formatting, 0 decision (the corpus has no rules file) |
| Edits by view | 1,867 inline, 215 sentences, 1,109 side by side, 427 rewritten, 238 lines |
| Guard fallbacks | 0 |

## Not covered

- Moves between two documents. A block that moves to another file is removed from one and added to the other. This is not decided yet (decision 0005).
- AI labels for importance.
- The `driftmark diff` command, which the `cli` lane builds on this.
- Drawing the views as HTML (M2).
- A signal for a task checkbox, a list type or an alert kind that changed while the words stayed the same. These rank as wording.
- Rename detection between files. A renamed file is one removed and one added document, as in `git-ranges.md`.
