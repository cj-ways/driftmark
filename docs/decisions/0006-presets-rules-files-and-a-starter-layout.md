---
status: accepted
date: 2026-10-17
---

# 0006 Presets, rules files and a starter layout

> Short version: driftmark reads other tools' folder layouts and never changes them. A rules file tells it which files are which type and where status is written. Version 1 ships rules for four popular layouts, and its own starter layout for teams that have none.

## Situation

Without knowing a layout, driftmark sees only Markdown files. It cannot say which document is a decision, which is an open change, or which is archived. The change ranking and the status view need that knowledge.

## Decision

1. **Rules files.** A small file states: files at this path are this type; the status is written here (frontmatter field or `> Status:` line); these headings mean a decision. driftmark only reads documents. It never moves, renames or rewrites them.
2. **Built-in presets in version 1.** Ready-made rules files for:
   - OpenSpec: current specs, open changes, the archive, and ADDED / MODIFIED / REMOVED headings.
   - GitHub Spec Kit: `specs/NNN-feature/` with spec, plan and tasks.
   - ADR folders: adr-tools, MADR and log4brains, with Accepted and Superseded status.
   - Kiro and cc-sdd: `.kiro/specs/` with requirements, design and tasks.
3. **Company layouts stay in their own repo.** A team with its own layout keeps its rules file in its repo. The package names no company.
4. **Starter layout in version 1.** `driftmark init` creates a layout for teams with no system, drawn from a long-running private corpus of about 3,400 documents:
   - `docs/decisions/`: decision records, written once and superseded rather than edited.
   - `docs/standards/`: rules that stay true over time and change in place.
   - `docs/features/`: the decided target for each feature.
   - `docs/working/`: active work, with a plan and a status file.
   - `docs/archive/`: finished working folders.
   It comes with templates, writing rules, and an agent instruction snippet. Status lives in frontmatter.

This repo's own `docs/` folder uses the starter layout.

## Evidence

- Mature OpenSpec repos often hold 800 to 3,000 Markdown files, most of it archive. Spec Kit repos are small (about 9 files typically). ADR folders hold 100 to 360 records in active repos.
- Detection rules and fields for each layout are recorded in the private research notes for this decision (`private/research/2026-10-17-r2-other-users.md`, not in git).

## What this costs

- About a day per preset, including a test against a real public repo.
- The starter layout is a second product inside the first: templates, rules and their own documentation. It adds roughly a week to version 1 (a guess).
