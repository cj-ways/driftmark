# Standards

> Short version: rules that stay true over time, one topic per file. When a rule changes, its standard is changed in place, and git keeps the old versions.

## What belongs here

- Rules that people and agents follow every day, such as how to write documents: [the writing rules](writing.md).
- Each rule written as something to do, in the present tense.
- A short reason for each rule, so a reader can judge a case the rules do not cover.

## What does not belong here

- A hard choice between options, with its reasons. That is a decision, in [the decisions folder](../decisions/). The standard links to it.
- How one feature should work. That is a feature target, in [the features folder](../features/).
- Plans and progress. Those go in [the working folder](../working/).

## Adding a standard

Run `driftmark new standard "Error messages"`. It fills in the template and prints the new file's path, here `docs/standards/error-messages.md`.

## Statuses

| Status | Meaning |
|---|---|
| `draft` | Written down, not followed yet. |
| `active` | Followed now. |
| `retired` | No longer followed. It stays so that old links still work. |

## Changing a standard

Edit the rules in place, and set `date` to the day of the change. When a rule's reason was a hard choice, record that choice as a decision first.
