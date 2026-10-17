---
status: accepted
date: 2026-10-17
---

# 0005 What version 1 contains

> Short version: version 1 is large on purpose. It ships reading, the ranked change view, live branches, the notes loop, the MCP connector, the status view, the writing check with an agent hook, a one-file export and a merge-request diff. The plan builds it in milestones so the change view and notes are usable first.

## Decision

**In version 1:**

| Part | What the user gets |
|---|---|
| Reading pages | Search, Mermaid and D2, styled tables, callouts, dark mode, phone layout |
| Change view | Branch against its base, everything since a date, or any two commits; decision changes ranked above wording changes; formatting-only changes folded |
| Live branches | The server reads git when a page opens; no rebuild timer |
| Notes loop | Notes on sentences; discuss, decide and apply with agents (decision 0004) |
| MCP connector | `driftmark mcp`: what changed, the lint, reading and answering notes |
| Status view | Which documents are current, superseded, archived or untouched for long |
| Writing check | `driftmark lint`, a Claude Code hook installer, and a CI mode; off by default for new users |
| One-file export | A document, folder or change page as one HTML file for people outside the repo |
| Merge-request diff | A CI job for GitHub and GitLab that renders a merge request's document changes and links them in a comment |
| Presets | OpenSpec, GitHub Spec Kit, ADR folders, Kiro and cc-sdd (decision 0006) |
| Starter layout | `driftmark init` creates a docs layout with templates and writing rules (decision 0006) |

The maintainer chose to include the export and the merge-request diff in version 1. The recommendation had been to wait with those two.

**Not decided yet:**

- Detecting a section that moved from one document to another.
- Several repos behind one server.
- AI labels for change importance. Rules come first, because measured accuracy of such labels is low, and a label may only sort changes, never hide them.

**Not in version 1:**

- Sharing notes in a team.
- Editor plugins (VS Code, Obsidian).
- A merged "current state" view of an OpenSpec spec with its open changes.
