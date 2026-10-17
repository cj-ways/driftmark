---
status: accepted
date: 2026-10-17
---

# 0004 Notes live in one file kept by the tool

> Short version: notes are saved in one SQLite file on the machine that runs driftmark. It is never committed. Agents reach notes through driftmark's MCP connector, or through a Markdown review file driftmark writes.

## Situation

The maintainer's habit today: while reading a spec he copies sentences into a notes app, writes his point under each one, then pastes the list into an agent session. They discuss each point, he decides, and the agent corrects the document. driftmark must support this loop on reading pages and on change pages, on a computer and a phone.

## Options

| Option | Good | Bad |
|---|---|---|
| One file kept by the tool | Git stays clean; every device and agent sees the same notes | Not in git; team sharing needs a later option |
| Review files committed to the repo | Any agent anywhere can read them | Notes appear in history and in the change view; the file has to live on some branch, often an agent's working folder; conflicts; opinions become public in a public repo |
| Markers inside the document | Agents see notes next to the text | Every note shows up in the document's own change view; agents break markers when they rewrite a paragraph |

## Decision

1. **Storage.** One SQLite file (SQLite is a database stored in a single file and built into the program). Its exact folder is still open: inside the clone's `.git/driftmark/` or in the user's data folder.
2. **How it runs.** `driftmark serve` is a program on the user's own machine. The browser on that machine, or on a phone through a private network, sends notes to it, and it writes them to the file. `driftmark mcp` is the same program started by an agent, and it reads and writes the same file. No cloud and no account.
3. **Review file.** For agents without MCP, driftmark also writes one Markdown file per review: each quote, its place, and the reader's point.
4. **Anchoring.** A note stores the quoted text with about 32 characters before and after it, the heading path, the source offsets and the git commit, following the W3C Web Annotation model. After the document changes, driftmark finds the sentence again through the git history, then the block diff, then a fuzzy quote match (the method of the Hypothesis client). A note whose sentence is gone moves to an "unanchored" tab. It is never deleted.
5. **The loop.** A note moves through open, sent, answered, discussing, decided and applied. Only the reader sets "decided". An agent can reply and propose a decision, and it records the commit that applied the note.

## Evidence

- No tool found keeps the agent's reply and the reader's decision for each note. Kiro keeps comments in memory only. Traycer lets agents change only a status line. Obsidian review plugins delete the thread on resolve.
- W3C model: [w3.org/TR/annotation-model](https://www.w3.org/TR/annotation-model/). Hypothesis quote matching: [match-quote.ts](https://github.com/hypothesis/client/blob/main/src/annotator/anchoring/match-quote.ts).
- One study found 19% of Hypothesis highlights orphaned after edits ([arXiv 1512.06195](https://arxiv.org/pdf/1512.06195)), so the unanchored tab is needed.

## What this costs

- Notes are not backed up by git. Deleting the clone deletes them if the file sits in `.git/`.
- Team sharing of notes is a later option (committing review files).
- Remote use (a phone) needs a login on the server. An open port with no login is not allowed.
