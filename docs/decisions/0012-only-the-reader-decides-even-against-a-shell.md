---
status: accepted
date: 2026-10-18
---

# 0012 Only the reader decides, even against an agent with a shell

> Short version: every request that changes notes needs the reader's token, on every address, 127.0.0.1 included. Every note action records where it came from: the page, the MCP connector, or a bare API call. A reader action that did not come from the page is shown as a warning.

## Situation

Decision 0004 says only the reader sets "decided". The MCP connector has no tool to decide, edit or withdraw a note. But an agent usually also has a shell on the same machine. On 127.0.0.1 the server asked for no token, and accepted a body without an `Origin` header. So `curl -X POST http://127.0.0.1:4170/api/notes/1/decide` decided a note as the reader. The M3 wave 2 lane found this, and the real Claude Code session it ran pointed it out on its own.

On one machine under one user account, nothing fully stops a determined agent: it can read every file the server can read. Only separate operating-system users would. The aim is narrower: a bypass must be a deliberate act, and it must be visible.

## Decision

1. **Writes need the token everywhere.**
   - Every request that changes notes or seen marks needs the reader's token cookie, whatever address the server listens on.
   - Reading pages on 127.0.0.1 stays free of a token.
   - When the server starts, it prints the token link once. The page tells a reader without the cookie how to get it.
   - The token can come from `--token`, from a file named by `--token-file`, or from the `DRIFTMARK_TOKEN` environment variable. The last two keep it out of the process list.
2. **Every action records its source.** Each note action and seen mark stores where it came from:
   - `page`: a same-origin request from driftmark's own page script, carrying the page's per-load marker;
   - `mcp`: the connector;
   - `api`: any other request with the token.
3. **The page warns.** A reader action whose source is not `page` shows in the note's history as a warning, for example "decided through the API, not on the page". The warning is always visible, and cannot be dismissed.

## What this costs

- On the reader's own machine, the first visit that writes a note needs the token link.
- A determined agent can still read the token file or the page's marker. It then has to do so on purpose, and the action still shows as coming from outside the page unless it also fakes browser headers.
