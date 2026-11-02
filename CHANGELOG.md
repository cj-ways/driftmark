# Changelog

> Short version: what each release of driftmark contains, newest first, by what a user can do with it. Version 0.1.0 is the first release.

## 0.1.0 (not released yet)

The first release holds everything version 1 set out to do ([decision 0005](docs/decisions/0005-version-1-scope.md)).

### See what changed: `driftmark diff`

- Compares the Markdown files of a git repository between two points: `main..feature`, `main...feature` (what a merge request shows), `auto` (your work, committed or not, against the default branch), or `--since <date>` on a branch. Folders after `--` narrow it.
- Ranks every change by written rules, never by AI: decision, rule, fact, wording, then formatting. A changed status or a change in a decision section is a decision; a `must`, `never` or `only` that came or went is a rule.
- Finds blocks that moved, also when they were edited too, and folds changes that are formatting only. A new or deleted document is one change.
- Prints a summary, the whole change list as JSON (`--json`), one entry per document (`--json=summary`), or a comment for a merge request (`--format markdown-summary`).
- [The diff command](docs/features/commands/diff.md), [the change list](docs/features/engine/change-list.md).

### Read and review in the browser: `driftmark serve`

- A small web server on your own machine. It reads git when a page opens, so a branch shows its latest commit without a rebuild.
- **The inbox** lists the branches whose documents changed, grouped by their highest level, and names the document to start with.
- **A change page** has a rail with every change, and one card per change: its level, what happened, why it ranks there, and the change itself, word by word, side by side, sentence by sentence or line by line.
- **A reading page** shows a document with its type, status and lifecycle, the changes waiting on open branches, and its sections.
- Mermaid diagrams are drawn on the server, and D2 diagrams when the `d2` program is installed. Code is coloured. Pages have search, dark mode and a phone layout. A first visit stays under 100 KB, not counting drawn diagrams and their font.
- Pages that show the files on disk update themselves when a file changes.
- A token, printed at start, guards every write; on any address beyond your machine it guards every request.
- [Pages](docs/features/pages.md), [the serve command](docs/features/commands/serve.md).

### Which documents are current: the status page

- Shows every document of a revision by type: current, superseded, archived, or with no lifecycle, and how many days ago a commit last changed it.
- Folds a superseded document under the one that replaced it, and finds that document by link, file name, title or number.
- Calls a document untouched after 180 days without a commit, or the number of days the rules file sets. Untouched is shown as a fact, never as a fault.
- [The status page](docs/features/status-page.md).

### Notes to agents

- Select words on a reading page or a change card and write a note: your stance, what should happen, whether it blocks the work, and your point.
- A note stays on its words while agents edit the document. A note whose words are gone moves to an "Unanchored" tab and is never deleted.
- Hand a review to an agent with "Copy as prompt" or "Write review file": round 1 to discuss, round 2 to apply what you decided.
- Only you decide. An agent replies, proposes a decision and records the commits that applied it. The change page then marks each change a note caused: "Changed because of note n12".
- Every note action records where it came from: your page, the MCP connector, or another program with the token. A reader action that did not come from the page shows as a warning that cannot be dismissed ([decision 0012](docs/decisions/0012-only-the-reader-decides-even-against-a-shell.md)).
- "Seen" marks a change as reviewed.
- Notes live in one file in the clone's git folder, which git never commits.
- [Notes](docs/features/notes.md).

### Agents: `driftmark mcp`

- An MCP connector that agents such as Claude Code, Codex and Cursor start themselves. Twelve tools: what changed, one document's changes, reading and searching documents, which documents are current, the writing check, and six tools for notes.
- Every answer fits a size limit and says how to ask for the rest.
- `driftmark mcp install` adds the connector to the repository's `.mcp.json` for Claude Code, and `--notes-line` adds a line to `AGENTS.md` that tells agents to look for notes.
- [The MCP connector](docs/features/commands/mcp.md).

### The writing check: `driftmark lint`

- Seventeen rules for documents that render well: one title, a summary, closed code blocks with a language, Mermaid diagrams that draw, links and anchors that exist, valid front matter, required fields and known status values.
- A `[lint]` table in `driftmark.toml` changes a rule's severity, turns rules off, sets size limits and leaves files out.
- `--changed <range>` checks only the files a branch changed, and `--format github` or `--format gitlab` prints findings the way the CI host shows them.
- `driftmark hook install` runs the check after each edit Claude Code makes, and shows the agent what to fix.
- [The writing check](docs/features/lint.md).

### Layouts: presets, the rules file and `driftmark init`

- Reads OpenSpec, GitHub Spec Kit, ADR folders (adr-tools, MADR, log4brains), and Kiro with cc-sdd, as they are, with built-in presets. `driftmark presets` says which ones a repository uses.
- `driftmark.toml` describes any other layout: which files are which type, where the status is written, and which headings mark decisions.
- `driftmark init` sets up a starter layout for a repository that has none: five folders, a writing standard, a rules file and instructions for agents. `driftmark new` starts a document from a template.
- [Presets](docs/features/presets.md), [the rules file](docs/features/engine/rules-file.md), [the starter layout](docs/features/starter-layout.md).

### Sharing: `driftmark export`

- Writes one document, a folder, or a range's changes as one HTML file that opens offline in any browser, with its diagrams and code colours. A file with more than one document can be searched.
- The file loads nothing and runs nothing beyond its own scripts. A file that would pass 10 MB, or the size `--max-size` sets, is cut down, and says what it left out.
- [The export command](docs/features/commands/export.md).

### Merge requests

- A GitHub Action and a GitLab CI template post one comment on each merge request with the ranked changes and a link to the rendered file, and update it on every push.
- The same job shows the writing check's findings next to the changed lines.
- [Merge-request review in CI](docs/features/ci.md).

### Install

- Packages on crates.io, PyPI and npm, and a Homebrew tap, for macOS, Linux and Windows.

### Known limits

- Notes are for one reader on one machine; sharing them in a team is not in this release.
- The server speaks plain HTTP. Across machines, use a private network.
- A renamed file shows as one deleted and one new document, and a section that moved to another document as one removal and one addition.
- Images and other files in a document show as text.
- Every browser check ran in Chromium; Safari and Firefox were not tested.
- On one machine and one user account, an agent with a shell can act as the reader; see [the security policy](SECURITY.md#known-limits).
