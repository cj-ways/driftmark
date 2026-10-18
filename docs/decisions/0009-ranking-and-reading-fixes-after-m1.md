---
status: accepted
date: 2026-10-18
---

# 0009 Ranking and reading fixes after M1

> Short version: rule words rank a change only when they themselves changed, a new or deleted document is one item, only known keys count as metadata, the lint gets an exclude list and leaves program output alone, a ticked checkbox is a fact, and `--json=summary` gives a small list.

## Situation

M1 ran the engine on a real corpus of 3,416 documents and showed six places where the result misleads the reader. The maintainer accepted all six fixes on 2026-10-18.

## Decision

1. **Rule words count only when they change.** A normative word (must, never, only and the rest of the list in `change-list.md`) raises a change to "rule" only when that word was added to or removed from an edited block. A block that is new or deleted gets no rule signal from words it merely contains. Before this, "only" alone tagged 13,206 changes over 30 days, and 1,897 of 2,652 changed documents ranked as "rule".
2. **A new or deleted document is one item.** The change list reports an added or removed document as a single change at document level, with its title, summary and size. Its blocks are not listed one by one. The full text stays reachable through the reading page and the JSON's per-document detail.
3. **Only known keys are metadata.** A `> Key: Value` line under the title counts as metadata only when the key is a known metadata key (Status, Date, Supersedes, Superseded by, Owner, Type, Tags, Version) or a key the rules file lists. Any other line stays the summary, whatever its length. This replaces the 12-word guess, which turned short `> Short version: …` lines into metadata.
4. **The lint has an exclude list.** `[lint] exclude` in `driftmark.toml` takes path globs that `driftmark lint` skips when it walks the repo. A file named explicitly on the command line is still checked.
5. **Program output is exempt from line length.** Code blocks marked `text`, `console`, `output` or `log` are not checked against the line-length limit. Pages wrap their long lines instead.
6. **A ticked checkbox is a fact.** A task item that changes from open to done, or back, gets a "task state changed" signal at fact level.
7. **A small JSON list.** `driftmark diff --json=summary` prints one entry per document: path, change kind, highest level and counts. `--json` alone keeps the full change list.

## What this costs

- The JSON contract changes for added and removed documents, so its schema version goes up.
- A reader who wants every block of a new document opens the document instead of the change list.
