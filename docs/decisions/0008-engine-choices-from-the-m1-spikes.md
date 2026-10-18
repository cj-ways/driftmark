---
status: accepted
date: 2026-10-18
---

# 0008 Engine choices from the M1 spikes

> Short version: driftmark reads git with gix and keeps the `git` command as a check and a fallback. Notes use one position table per block. Mermaid is drawn on the server by merman, measuring text with a font driftmark ships. The minimum Rust version becomes 1.95, and the licence allow-list grows to what these libraries need.

## Situation

Three spikes tested the riskiest assumptions of the plan before the engine is built. Their findings are in `docs/working/version-1/spikes/`: `speed.md`, `anchoring.md` and `mermaid.md`. The maintainer accepted all nine recommendations on 2026-10-18.

## Decision

1. **Reading git: gix, behind a small interface.** gix was faster at every git step and starts no processes. The `git` command stays in the code for two jobs. It is the test check in CI, where both readers must give identical results. It is also the automatic fallback for repos gix cannot read yet: partial clones with missing files, and SHA-256 or reftable repos. gix's pack cache is capped by bytes, which avoids an MPL-2.0 dependency of its default cache.
2. **Note positions: one offset table per block (design B).** The table is sent beside the page HTML. It passed all 113 fixture cases and 409,920 random selections on a 3,416-file corpus with no error. It still works after other scripts split the page text. It makes a page about 25% bigger after gzip.
3. **Tight ranges.** A selected bold word maps to `bold`, not to `**bold**`.
4. **M1 time limit.** On the maintainer's server, 30 days of changes (about 2,600 changed files) diff in under 1 second with all cores and under 5 seconds on one core. A typical branch diffs in under 0.1 seconds. The spike measured 0.13 seconds, 0.8 seconds and 0.02 seconds before pairing, moves and ranking were added.
5. **Mermaid on the server.** merman draws every Mermaid block to SVG, measuring text with a real font. When merman returns an error, the page falls back to mermaid.js in the browser. With its built-in width guess instead of a font, text was cut off in 375 of 754 diagrams; with the font, in 5.
6. **Diagram font: Liberation Sans.** It has Arial's text widths and the OFL-1.1 licence. driftmark ships the font files (about 1.65 MB), and pages use the same font for diagrams.
7. **Minimum Rust version: 1.95,** because merman 0.9 needs it.
8. **Licences.** `deny.toml` allows MIT, Apache-2.0, BSD-2-Clause, BSD-3-Clause, Zlib, Unicode-3.0, Unicode-DFS-2016, MPL-2.0 and EPL-2.0. The font is OFL-1.1, which cargo-deny cannot see. MPL-2.0 and EPL-2.0 require publishing changes to their own files; driftmark uses them unchanged and ships their licence texts. The release step in M6 generates a notices file.
9. **Upstream reports.** The two comrak bugs (escaped pipes in table cells shift inline positions; a code span across a CRLF keeps the carriage return) and the merman failure on very large ER diagrams are reported upstream, each with a made-up example.

## What this costs

- The program grows by about 25 MB with merman, so M6 publishes one npm package per system instead of one package for all.
- Two git readers to keep in step, checked against each other in CI.
- Rust older than 1.95 cannot build driftmark from source.
