# Features

> Short version: the decided target of each feature: what it does, and the exact contract that other work builds on. A feature document says how things should be, even before they are built.

## What belongs here

- One file per feature, or a folder of files for a large feature.
- What a user sees, with made-up examples.
- The contract: inputs, outputs, errors and limits, stated exactly.

## What does not belong here

- The order of the work and how far it got. Those go in [the working folder](../working/).
- A hard choice between designs, with its reasons. That is a decision, in [the decisions folder](../decisions/).
- Rules that every feature follows. Those are standards, in [the standards folder](../standards/).

## Adding a feature

Run `driftmark new feature "Export to PDF"`. It fills in the template and prints the new file's path, here `docs/features/export-to-pdf.md`.

## Statuses

| Status | Meaning |
|---|---|
| `target` | Decided, and not fully built yet. |
| `shipped` | Built and released as the document says. |
| `dropped` | Will not be built. It stays as a record. |

## Keeping it true

When the target changes, change the feature document first, then the code. A feature document that no longer matches what is built is a bug.
