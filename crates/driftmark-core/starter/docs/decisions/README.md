# Decisions

> Short version: one record per decision, numbered in the order the decisions were made. A record is written once. When a decision changes, a new record replaces the old one, and the old one stays as it was.

## What belongs here

- A choice that is hard to undo, or that later work will follow, such as "Keep each service's data in its own database".
- One decision per file, named with its number and title, such as `0004-keep-each-services-data-in-its-own-database.md`.
- The situation, the options that were weighed, what was decided, and what it costs.

## What does not belong here

- Rules that people keep up to date. Those are standards, in [the standards folder](../standards/).
- How a feature should work. That is a feature target, in [the features folder](../features/).
- Work in progress. That goes in [the working folder](../working/).

## Adding a record

Run `driftmark new decision "Keep each service's data in its own database"`. It takes the next free number, fills in the template, and prints the new file's path.

## Statuses

| Status | Meaning |
|---|---|
| `proposed` | Written down, not decided yet. |
| `accepted` | Decided. It holds until a later record replaces it. |
| `superseded` | A later record replaced it. |
| `rejected` | Weighed and turned down. It stays as a record of why. |

## Replacing a record

1. Write the new record with `driftmark new decision`. Say in its text which record it replaces.
2. In the old record, change only the front matter: set `status: superseded`, and add `superseded-by: 0007-the-new-title.md`.
3. Leave the rest of the old record as it is. It shows what was true then, and why.
