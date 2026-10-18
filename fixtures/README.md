# Fixtures

> Test documents for driftmark. Every file here is made up for a test.

- No document from a real company, team or person goes here, not even with names changed. The repo will be public, and git keeps every old version.
- A fixture tests one behaviour. Its folder name says which one, for example `moved-and-edited/`.
- Public repos used in tests (OpenSpec, GitHub Spec Kit, an ADR repo) are fetched at a pinned commit when the test runs. They are never copied in here.
- A document fixture `<name>.md` may have `<name>.expected.txt` beside it: the document model the test expects, written out. Each test run also writes its fresh output under the build folder's `tmp/document-dumps/`, so a deliberate change can be reviewed and copied over.
- A pair of files `<name>.a.md` and `<name>.b.md` tests one difference between two versions of a document.
- `.gitattributes` here keeps every fixture's bytes exactly as written, line endings included, on every system.
