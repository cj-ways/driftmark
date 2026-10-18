# driftmark — Agent Instructions

A Rust tool for a git folder of Markdown that AI agents keep editing. It shows what changed between two git points, ranked by meaning, shows which documents are current, and runs a notes loop between the reader and the agents. The repo is private until version 1 works (`cj-ways/driftmark`); then it goes public with its whole history.

**This file is shared by every AI agent.** Claude Code reads it through `CLAUDE.md`. Codex reads it directly. Keep tool-specific notes out of here.

**Shared rules for all projects:** `../_shared/README.md`. Before social media, Codex, video or browser work, read the matching file in `_shared/playbooks/`. 🔒 Locks there can't be overridden here.

---

## Start of every session
1. Read `STATUS.md` (kept out of git).
2. Run `git status`. It should be clean, or match what `STATUS.md` says.
3. Read `docs/working/version-1/plan.md` and the decisions it names in `docs/decisions/`.

## End of every session
1. Update `STATUS.md`: log, open problems, next steps.
2. Commit with the commit rules below.

## Where things live
| Path | What it is |
|---|---|
| `crates/driftmark-core/` | The rules: blocks, rules files, change detection and ranking, note anchoring, lint rules. No file, git or network access |
| `crates/driftmark/` | The `driftmark` program: git reader, server, notes file, MCP connector, rendering, commands |
| `fixtures/` | Made-up test documents only (see its README) |
| `docs/decisions/` | Decision records, written once. A later decision supersedes an earlier one; it never edits it |
| `docs/working/<topic>/` | Active work: a plan and its status |
| `.github/workflows/ci.yml` | Format, Clippy, tests on Linux, macOS and Windows, the minimum Rust version, core coverage, cargo-deny |
| `STATUS.md` | Current state and log; out of git, on the maintainer's machines only |
| `private/` | Notes that must never be public, out of git: the research behind the decisions |

## Run and check
- `cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --all --check`: run them before saying a code task is done.
- Core coverage stays at 100%: `cargo llvm-cov -p driftmark-core --fail-under-lines 100 --fail-under-functions 100 --fail-under-regions 100`.
- Mutation testing on the core misses nothing: `cargo mutants -p driftmark-core` (scope with `-f <file>` while iterating).

## Rules
- **Commits:** one line `type(scope): summary`, no body, no trailers. Types: `feat`, `fix`, `perf`, `refactor`, `test`, `docs`, `ci`, `chore`.
- **Commit author:** `Saba Janelidze <139575863+000Janela000@users.noreply.github.com>`. Never a work email.
- **What may be committed without asking:** work from an agent brief the maintainer signed off may be committed and pushed to `main`, because `main` deploys nothing. Going public, publishing a package, changing repo settings, and anything outside this repo need his OK first.
- **Agents:** an implementation agent works in its own git worktree and branch, from a brief the maintainer signed off. The coordinating session checks the result and merges it.
- **Nothing private in git.** No document, name, path, address or email from the maintainer's workplace goes into a commit, not even in a test or an example. Making the repo public will expose every old commit. Material that must stay private goes in `private/`.
- **Fresh code.** The design follows a private prototype, described in `docs/decisions/0003-rust.md`. No code is copied from it.
- No `unsafe`, with one exception (decision 0011, item 4): the `malloc_trim` declaration in `crates/driftmark/src/memory.rs`, which gives memory back to the system on Linux with glibc. The workspace lint `unsafe_code` is `deny`, not `forbid`, so that one block can carry `#[allow(unsafe_code)]`; no other code may. A new dependency needs a reason; `deny.toml` checks licences and sources, and a new licence is added there only with that reason.
- The licence files live at the root and, as copies, in each crate folder (crates.io packages each crate alone). Keep the copies equal.
- **Plain, simple words** in all docs and notes: one idea per sentence, everyday words, the answer first.
- **Docs layout.** A decision record has YAML frontmatter with `status` and `date`, a `#` title, a `>` short version, then `##` sections.
