# driftmark-core

> Short version: the engine of [driftmark](https://github.com/cj-ways/driftmark), a tool that shows what AI agents changed in the Markdown documents of a git repository, ranked by meaning. To use driftmark, install the program instead: `cargo install driftmark --locked`.

## What it holds

- **The document model:** a Markdown file as a flat list of blocks, each with its kind, its source bytes, its headings and its plain text.
- **The change list:** two versions of a document compared block by block, with moves, formatting-only changes, and a level for each change from decision down to formatting.
- **The rules file and the presets:** which documents are decisions, specs or plans, and what their statuses mean.
- **The status view:** which documents are current, superseded or archived, and how long ago each changed.
- **Notes:** anchoring a reader's note to its words, finding them again after edits, and the rules of the note loop.
- **Search, the writing check's rules and the page HTML.**

It reads no files, no git, no network and no clock. The `driftmark` program reads them and hands the bytes in, through traits this crate defines.

## Its API

This crate is published so that `cargo install driftmark` can build. Its API serves the `driftmark` program, and may change in any release.

[The feature documents](https://github.com/cj-ways/driftmark/tree/main/docs/features/engine) describe each part exactly.

## Licence

Licensed under either of [MIT](https://github.com/cj-ways/driftmark/blob/main/LICENSE-MIT) or [Apache-2.0](https://github.com/cj-ways/driftmark/blob/main/LICENSE-APACHE), at your option.
