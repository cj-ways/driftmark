# Security policy

> Short version: report a vulnerability privately, through GitHub's "Report a vulnerability" form on this repository, never in a public issue. The server, its token, notes and MCP writes, the cleaning of HTML and SVG, and the installers that write agent settings are in scope. driftmark guards the reader against other sites and other machines; it cannot fully guard the reader against an agent that runs under the reader's own user account.

## Reporting a vulnerability

Use GitHub's private vulnerability reporting:

1. Open [the form for a new report](https://github.com/cj-ways/driftmark/security/advisories/new), or the "Security" tab of this repository, then "Report a vulnerability".
2. Say which version you ran (`driftmark --version`) and on which system.
3. Give the steps that show the problem, and what someone gains by using it.

Only the maintainer sees the report. Please do not open a public issue, pull request or discussion about it until a fix is released. You can follow the fix in the report. When the fix is released, the report becomes a public security advisory, which can name you.

Fixes go into the latest release only.

## What is in scope

**The server, `driftmark serve`** ([the serve command](docs/features/commands/serve.md)):

- reading a file that git does not list, or a file outside the repository, through any address;
- a request from a page on another site that reads an answer or writes a note;
- a request that names this machine falsely in its `Host` header to read pages (DNS rebinding);
- a page that runs a script or loads anything the Content-Security-Policy should block.

**The token** ([the serve command, "The token"](docs/features/commands/serve.md#the-token)):

- a write accepted without the reader's token, on any address;
- a read accepted without the token where the server asks for it;
- a token that can be guessed, or that leaks into an address, a log, a page or the browser's history;
- a token file that other users of the machine can read, but that the server accepts anyway.

**Notes and MCP writes** ([notes](docs/features/notes.md), [the MCP connector](docs/features/commands/mcp.md)):

- an agent that decides, edits or withdraws a note through `driftmark mcp`, which only the reader may do;
- a note action recorded as coming from the reader's page when it did not, other than through the known limit below;
- the MCP connector reading outside the repository, writing any file other than the notes file, or starting a program other than `git`.

**HTML and SVG cleaning** ([page HTML](docs/features/engine/html.md), [the pages, "Diagrams"](docs/features/pages.md#diagrams), [the export command](docs/features/commands/export.md)):

- raw HTML in a document that gets past the allow-list;
- a `javascript:` or `data:` link that reaches a page;
- a Mermaid or D2 drawing that keeps a script, an event attribute or a reference outside itself after cleaning;
- an exported HTML file that runs or loads anything beyond its own scripts and styles.

**The installers that write agent settings** ([the writing check, "The Claude Code hook"](docs/features/lint.md#the-claude-code-hook), [the MCP connector, "Installing it"](docs/features/commands/mcp.md#installing-it)):

- `driftmark hook install`, `driftmark mcp install` or `driftmark init --agent-file` writing outside the repository, following a link out of it, or changing settings they should leave alone;
- a settings file left half written.

**The merge-request job** ([merge-request review in CI](docs/features/ci.md)): a branch name, a folder or a document that runs as code in the GitHub Action or the GitLab template, or that leaks their token.

## Known limits

These are known, and are not vulnerabilities on their own. A way around them that the text below does not describe is in scope.

- **One machine, one user account** ([decision 0012](docs/decisions/0012-only-the-reader-decides-even-against-a-shell.md)). An agent that runs under the reader's own account can read the token file and a page's marker, and send the browser's headers too. Its action then shows as the reader's, from the page. driftmark makes such an action deliberate, and shows any reader action that did not come from the page as a warning that cannot be dismissed. Only a separate system user for the agent stops it.
- **Pages on `127.0.0.1` open without the token** when the server made its own token. Another user or program on the same machine can read the documents through it. On a shared machine, give a token with `--token-file` or `DRIFTMARK_TOKEN`: then every request needs it.
- **Plain HTTP.** The server has no HTTPS. Across machines, the token and the pages travel unencrypted, so use a private network such as a VPN or Tailscale.
- **The token is the reader.** Anyone who holds the token, or the browser cookie it sets, can write notes and decide them. A token given with `--token` shows in the system's process list; `--token-file` and `DRIFTMARK_TOKEN` keep it out.
- **The notes file is not encrypted.** It sits in the clone's git folder, as `driftmark/notes.db`. Anyone who can read the clone can read the notes.
- **D2 diagrams** are drawn by the separate `d2` program, when it is installed. driftmark cleans what it returns, but `d2` itself is outside this policy.
- **Two unmaintained font libraries.** `rustybuzz` and `ttf-parser` are marked unmaintained (RUSTSEC-2026-0192, RUSTSEC-2026-0206), with no known vulnerability. They only read the font driftmark ships ([decision 0011](docs/decisions/0011-decisions-from-m2.md)).

## How the code is checked

- CI checks every dependency's licence, source and known advisories with `cargo deny`, on every push to `main` and every pull request.
- The program has one `unsafe` block, the `malloc_trim` declaration that gives memory back on Linux ([decision 0011](docs/decisions/0011-decisions-from-m2.md)). The workspace refuses any other.
