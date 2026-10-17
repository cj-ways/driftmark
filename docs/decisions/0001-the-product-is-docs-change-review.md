---
status: accepted
date: 2026-10-17
---

# 0001 The product is docs change review

> Short version: driftmark is for a git folder of Markdown that AI agents keep editing. It lets one person read it, see what changed and whether it matters, see which documents are still current, and answer the agents with notes. Reading pages come with it, but they are not the reason to choose it.

## Situation

An engineer who manages AI coding agents reads specs, decisions and plans instead of code. Agents write those documents in Markdown and keep changing them while they implement. The maintainer's own work repo holds about 3,400 such files.

Two problems follow:

1. Raw Markdown is uncomfortable to read without an editor, and diagrams do not draw.
2. After agents refactor a spec, nobody can see what changed. Re-reading a whole folder takes hours, and a line diff shows syntax, not meaning.

## Options

| Option | What it is | Why not |
|---|---|---|
| Docs change review | Reading, a ranked change view across the whole folder, a status view, and notes back to agents | Chosen |
| Full docs reader | Compete with MkDocs, Zensical and Starlight on themes and plugins, with change review as one feature | At least 8 free viewers exist; reading alone gives nobody a reason to switch |
| Agent plan reviewer | Review each agent's plan before it codes, with notes back to the agent | Plannotator (about 9,500 stars) already does this, and it ignores long-lived docs |
| A tool for one company only | Keep the private prototype and extend it | Serves nobody else |

## Decision

driftmark is a docs change review tool. Its main jobs, in order:

1. Show what changed between two git points across the whole docs folder, rendered, with decision changes above wording changes.
2. Show which documents are current, superseded, archived or untouched for a long time.
3. Let the reader attach notes to sentences and run a discuss, decide and apply loop with agents.
4. Render every document as a comfortable page, because the change view needs a renderer anyway.

## Evidence

- No open-source tool renders a whole folder's changes between two git points. GitHub renders one file at a time. GitLab renders none: [issue 17000](https://gitlab.com/gitlab-org/gitlab/-/issues/17000), open since 2017.
- Spec tools either overwrite specs in place (GitHub Spec Kit, Kiro) or ask the agent to write its own change list (OpenSpec). An agent that edits a spec directly leaves no trace there. A git-based diff catches it.
- Users ask for this: OpenSpec [discussion 1543](https://github.com/Fission-AI/OpenSpec/discussions/1543) wants "changes in meaning", Spec Kit [discussion 152](https://github.com/github/spec-kit/discussions/152) (72 upvotes) and OpenSpec [issue 1525](https://github.com/Fission-AI/OpenSpec/issues/1525) want to know which spec is current.
- Thoughtworks Technology Radar vol. 34 (April 2026) reports "overly verbose markdown outputs that increased cognitive load".

## What this costs

- Reading features stay modest. driftmark will not match MkDocs on themes or plugins.
- The closest tools could add git ranges later (Plannotator, markdown-diff-viewer). driftmark has to stay ahead on the whole-folder view, the status view and the per-note decisions.
