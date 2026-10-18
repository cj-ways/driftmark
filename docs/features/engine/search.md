---
status: target
date: 2026-10-18
---

# Search

> Short version: `driftmark-core` keeps an index in memory of every word in every document of a revision, and where each word stands. A query is words, `"phrases"` and a last word that is still being typed; a document must hold all of them. A match in the title ranks above one in a heading, and a heading above the body, and a rarer word weighs more. Each result names the block that matches best, the headings above it, and a snippet of about 160 characters with the matched words marked. Words split at Unicode word boundaries and compare without regard to case, so Georgian, Greek and other scripts work like English. The program builds the index on all cores and serves it at `/api/search` (`docs/features/commands/serve.md`).

## How it is called

The core reads no files. The program reads and parses every document, then:

1. `Shard::new()`, then `shard.add(Entry { path, doc_type, document })` for each document in order. `doc_type` comes from the rules file (`rules.type_for(path)`, or `document`). `shard.remove(path)` stands for a path that has no document: deleted, or not parseable.
2. `Index::from_shards(shards)` joins the shards. Documents are numbered by their order across the shards, from 0. The program builds one shard per run of files, on several threads at once.
3. `Query::parse(text)` reads the query.
4. `search(&[&index], &query, &filter, limit)` returns a `Found`: how many documents were searched, how many matched, and the best `limit` of them as `Hit`s, best first. A hit has the document's id, path, title and type, its score, and `block`, the index of the block that matches best.
5. For each hit, the program parses the document again and calls `query.snippet(&document.blocks[hit.block])` and `place(&document, hit.block)`.

`terms(text)` gives the words of a text as the index keeps them, for tests and for tools that want to show it.

## What is indexed

Every block's plain text (`docs/features/engine/document-model.md`): headings, paragraphs, list items, table rows, code, diagrams' code, alerts, footnotes, HTML blocks with their tags removed, and front matter. Link reference definitions and thematic breaks are left out, because a reader does not see them as text.

The words of a document are numbered in reading order. Each block starts one place after the previous block's last word, so a phrase never spans two blocks. Each block is in one of three fields:

| Field | Which block |
|---|---|
| Title | The document's first level-1 heading outside any container: the block its title comes from. |
| Heading | Any other heading, in a container too. |
| Body | Every other block. |

## Words

**Splitting.** A text splits at Unicode's word boundaries (UAX #29). A word is a piece between two boundaries that holds at least one letter or digit.

| Text | Words |
|---|---|
| `Don't cache read-only pages` | `don't`, `cache`, `read`, `only`, `pages` |
| `snake_case`, `v0.89`, `3.14`, `a.md`, `e.g.` | each one word: `snake_case`, `v0.89`, `3.14`, `a.md`, `e.g` |
| `docs/guide.md` | `docs`, `guide.md` |
| `ქართული ტექსტი` | `ქართული`, `ტექსტი` |
| `👩‍💻 developer ✅ done` | `developer`, `done`: emoji and punctuation are not words |
| `中文文章` | `中`, `文`, `文`, `章`: each Chinese character is a word |
| `ひらがな` | `ひ`, `ら`, `が`, `な`: each Hiragana character is a word too |
| `カタカナ` | one word: Katakana runs stay whole |

The change view has its own word rule, which keeps `read-only` whole so an edit inside it shows as one change. Search splits it, so that `read` finds it.

**Folding.** Words compare in a folded form:

- every letter takes its lower-case form, so `Cache`, `CACHE` and `cache` are one word, and Georgian capitals (Mtavruli, `ᲥᲐᲠᲗᲣᲚᲘ`) match the ordinary letters (`ქართული`);
- `ß` and `ẞ` become `ss`, and the Greek final sigma `ς` becomes `σ`, as Unicode case folding does;
- the typographic apostrophe `’` becomes `'`, so `don’t` and `don't` match.

**Long words.** A word longer than 100 bytes after folding (`MAX_WORD_BYTES`) is not kept: such a run is almost always a hash, a key or an inline image. It still takes its place, so the words on either side of it are not next to each other.

## Queries

| Written | Means |
|---|---|
| `cache pages` | Documents that hold both words, anywhere. The last word is a prefix: see below. |
| `cache pages ` | Both words exactly: a space after the last word ends it. |
| `"one hour"` | The words next to each other, in this order, in one block. `“one hour”` and `„one hour“` work too, as phones type them. |
| `"one ho` | A quote that is never closed runs to the end, and its last word is a prefix. |
| `read-only`, `2026-10-18`, `driftmark-core` | A piece without spaces that holds several words is a phrase: `read` then `only`. |
| `cach` | The last word, with nothing after it, is a prefix: it finds `cache`, `caches` and `caching`, so results show while the reader types. |

- Case does not matter; queries fold like documents.
- Every clause must match. There is no "or" and no "not".
- A clause written twice counts once.
- Punctuation and emoji alone are not words. A query of only those, or an empty query, finds nothing.
- A query holds at most 32 words (`MAX_QUERY_WORDS`), counting the words inside phrases. Each word costs one pass over its part of the index, so the limit keeps one request from holding a server up. `Query::parse` returns `QueryError::TooManyWords` past it.

**Filters.** `Filter { path, types }` keeps only some documents:

- `path` keeps documents whose path starts with this text, as written. `docs/decisions/` keeps that folder; `docs/d` also keeps `docs/drafts/`.
- `types` keeps documents of any of these types from the rules file. Empty keeps every type. A file no type matches has the type `document`.

The filter does not change the scores: the weights below count every document searched.

## Ranking

A document's score adds, for each clause of the query:

```text
weight × (field + more)

weight = ln(1 + N / n)       N documents searched, n of them hold the clause
field  = 4 for the title, 2 for a heading, 1 for the body   where its best match stands
more   = 0.5 − 0.5 / k       k matches of the clause in the document
```

- A rarer clause weighs more. In 3,500 documents, a word in 10 of them weighs 5.9, and a word in 2,000 weighs 1.0.
- A clause's best match counts once, by its field. Further matches add `more`, which stays below 0.5 however many there are. So a title match always ranks above a heading match of the same clause, and a heading match above a body match.
- Equal scores rank by path, in byte order.

For example, with 4 documents, 3 of which hold "cache", the weight of `cache` is ln(1 + 4/3) = 0.85:

| Document | Query `cache` | Score |
|---|---|---|
| `# Cache` as its title | title, once | 0.85 × 4 = 3.39 |
| `## Cache rules` as a section | heading, once | 0.85 × 2 = 1.69 |
| "cache" three times in its text | body, three times | 0.85 × (1 + 0.33) = 1.13 |
| no "cache" | no match | not listed |

## Results

Each hit names the block that matches best. That is the block holding the largest weight of distinct clauses:

- on equal weight, a heading wins over the body, then the earlier block;
- the title is chosen only when no other block matches, because a result shows the title anyway.

`place(document, block)` says where the block sits:

- `headings`: the titles of the headings above it, outermost first. A heading outside any container opens a section, so when the block is such a heading, its own title comes last.
- `anchor`: the page id of the heading that opens the block's section, as the reading page writes it (`docs/features/engine/html.md`): the block's own id when it is a heading, otherwise the nearest heading before it outside any container. `None` before the first heading.

A result links to its block through `block`, the same index as `data-k` on the reading page, and to its section through `anchor`.

## Snippets

`query.snippet(block)` returns pieces: stretches of text, each marked as a match or not. Joined, they give the snippet.

- **What is marked.** A word clause is marked wherever it stands in the block. A phrase is marked only where its words stand together, as one piece from its first word to its last. A prefix marks the whole word it matched. Overlapping marks join.
- **Length.** A block of at most 160 characters (`SNIPPET_CHARS`) shows whole. Characters are counted, not bytes, so a Georgian snippet holds as many letters as an English one.
- **Where a long block is cut.** The snippet shows the run of matches that fits in 160 characters and holds the most of them, the first such run on a tie. It starts up to 40 characters (`LEAD_CHARS`) before the run's first match, then runs on to 160 characters, or to the end of the run when the run alone is longer. Near the end of the block, it starts earlier so it still holds 160 characters. A block without a match shows its start.
- **Cuts fall between words.** Each cut moves out to the nearest Unicode word boundary, so a snippet can run past 160 characters by up to one word at each end. White space at a cut is dropped, and `…` marks each cut end.
- **White space.** Each run of white space is one space. In a table row, the space between two cells is ` · `.

A long paragraph with "lantern" in its middle gives:

```text
…and roads for a while, then it names the [lantern] in the middle of its sentences, and then it goes on about mountains, valleys, bridges and towns until it is much…
```

The brackets stand for the piece marked as a match.

## Several indexes as one

`search` takes a list of indexes. A later index replaces every document of an earlier one that has the same path, and a path the later index marks as removed hides the earlier document. The working tree uses this: one index of every file as it was, and a small one of the files changed since, so a change costs a few files, not the whole tree. `N`, the number of documents searched, counts each path once.

## Size and speed

The index keeps, for each word, the documents that hold it and the places where it stands, written compactly as differences in variable-length integers. It does not keep the text: a snippet comes from the document, parsed again by the program for the hits it shows. `index.bytes()` estimates its memory.

Measured on 2026-10-18 on the private corpus at the tip of its working branch, 3,585 Markdown files and 56 MB, on the maintainer's 12-core server, from the release build, while other work kept the load average between 28 and 54. `crates/driftmark/tests/search_corpus.rs` runs it and prints numbers only.

| Measure | Result |
|---|---|
| Building the index, 12 threads | 431 ms (median of 3; 410 to 420 ms at a load of 6) |
| Building the index, 1 thread | 2.0 s: reading 0.14 s, parsing 0.99 s, indexing the words 0.84 s, joining 0.09 s |
| Index size | 25.9 MB, about a fifth of the server's cache |
| Process memory after the first build | 153 MB more than before it, most of it memory the system allocator keeps after the threads free it. This test calls the build directly; the server gives most of it back right after the build (`docs/features/commands/serve.md`, "Memory"), and its first search added 7 MB instead of 72 MB on 2026-10-18. |
| 100 queries, the index alone | median 0.9 ms, slowest 8.4 ms |
| 100 queries, with snippets from documents read and parsed again | median 5.1 ms, slowest 18.1 ms |

- The 100 queries are made from words of the corpus with a fixed seed: 20 single words, 20 pairs of words, 20 phrases of three words, 20 prefixes of three letters, and 20 words followed by two letters of the next. 98 of them find something.
- Most of a query's time goes to parsing the hits' documents again for their snippets, so the program parses them on all cores. The slowest queries hold common words or short prefixes, which match thousands of places.
- At a load average of 42 to 48 earlier the same day, before the ASCII fast path below, the build took 0.87 s and the slowest query 140 ms, with snippets parsed one after another.
- **The ASCII fast path.** Splitting at Unicode word boundaries was 75% of indexing the words. Text is cut at ASCII white space that an ASCII character follows, where Unicode always puts a boundary, and each chunk that is all ASCII is split by the few rules ASCII can meet. A test checks the two give the same words on 20,000 random texts of ASCII, Latin, Georgian, Chinese, emoji and combining marks. It cut indexing the words from 1.22 s to 0.84 s on one thread.
- **No search library.** The plain index stays well inside the limits set for it: under 1 s to build, and under 50 ms at the median and 200 ms at the slowest per query. So driftmark uses none. A library such as tantivy was not measured; it would bring its own index format, tokenizers and dependencies.

## Fixtures

`fixtures/search/` holds one folder per behaviour, each a small made-up repository with `queries.toml` and `expected.txt`: `words`, `phrases`, `prefixes`, `filters`, `ranking` and `snippets`. `crates/driftmark-core/tests/search_fixtures.rs` runs them. `words/terms.expected.txt` shows how each block of `words/words.md` splits into words.

## Limits

- **Scripts without spaces.** Chinese and Japanese characters are each a word, so a word written in several characters is found as a phrase of them. Thai, Lao, Khmer and Burmese runs stay whole, because splitting them needs a dictionary; a word inside such a run is found only by a prefix of the run.
- **Normalization.** Text compares as written. A letter written as a base letter plus a combining accent does not match the same letter written as one character. Agents and editors write the single character almost always.
- **No stemming.** `page` does not find `pages` unless it is the last word, typed as a prefix. Stemming is different in every language, and a wrong stem hides results.
- **No synonyms, no typo tolerance, no "or", no "not".**
- **File names are not searched**, only text. The `path` filter narrows by folder.
- **An alert's written title**, as in `> [!NOTE] Heads up`, is not searched; its text is.
