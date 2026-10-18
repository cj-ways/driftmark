# Fixtures

> Test documents for driftmark. Every file here is made up for a test.

- No document from a real company, team or person goes here, not even with names changed. The repo will be public, and git keeps every old version.
- A fixture tests one behaviour. Its folder name says which one, for example `moved-and-edited/`.
- Public repos used in tests (OpenSpec, GitHub Spec Kit, an ADR repo) are fetched at a pinned commit when the test runs. They are never copied in here.
- A document fixture `<name>.md` may have `<name>.expected.txt` beside it: the document model the test expects, written out. Each test run also writes its fresh output under the build folder's `tmp/document-dumps/`, so a deliberate change can be reviewed and copied over.
- A pair of files `<name>.a.md` and `<name>.b.md` tests one difference between two versions of a document.
- A folder under `lint/` is named after one writing rule and is a small repository of its own: `pass.md`, which no rule faults; `fail.md`, which only that rule faults; `fail.expected.txt`, its findings; and, when the rule needs them, a `driftmark.toml` and the files its links point to.
- A folder in `diff/` holds `a.md` (the old version), `b.md` (the new one), an optional `driftmark.toml`, and `expected.txt`, the change list written out. A missing `a.md` or `b.md` means the document was added or removed.
- `.gitattributes` here keeps every fixture's bytes exactly as written, line endings included, on every system.
- `render/reading/` and `render/hostile/` hold documents `<name>.md` with the page HTML `<name>.html` and the position tables `<name>.positions.json` the renderer must give. `render/anchoring/` holds the 30 documents and the 113 cases (`cases.json`) of the notes anchoring check, which runs in a browser.
- `render/changes/<case>.html` is the change HTML of the pair in `diff/<case>/` or `render/changes/<case>/`.
- `diagrams/hostile-svg/` holds drawings a diagram program could return if it were tricked or broken: scripts, event attributes, outside addresses, animations that change a link, style blocks, HTML labels with images and frames, copied ids, entities, two roots. `diagrams/hostile-mermaid/` holds Mermaid sources that try the same through merman. `crates/driftmark/tests/diagrams.rs` cleans or draws each and checks what is left.
- A folder in `search/` is a small made-up repository for one search behaviour: its Markdown files, an optional `driftmark.toml`, `queries.toml` with the queries to run, and `expected.txt`, the results written out with matches in brackets. `search/words/terms.expected.txt` shows how each block of `words.md` splits into words.
- A folder in `notes/` holds `a.md`, the version notes were made on, `b.md`, a later version, `notes.toml`, the notes (each a `quote`, which occurrence of it with `nth`, and an `origin`: `known`, `same` or `lost`), and `expected.txt`, where each note lands in `b.md` with its anchoring state. `crates/driftmark-core/tests/notes_fixtures.rs` checks them.
