@AGENTS.md

# Claude Code — extra notes

The shared rules are in `AGENTS.md` (imported above). This part is only for Claude Code.

- **Cargo is not on the default PATH on the server.** Rust was installed without changing shell startup files. Start commands with `export PATH="$HOME/.cargo/bin:$PATH"`.
- **Commit author on the server.** The global git email there is not the one to use. Commit with `GIT_AUTHOR_EMAIL` and `GIT_COMMITTER_EMAIL` set to the address in `AGENTS.md`, unless `git config user.email` in this repo already shows it.
- **`../_shared/` exists only in the Projects folder on the Mac and the PC.** On the server the clone is `/root/cj-ways/driftmark`, and the shared rules load from `~/.claude/rules/` instead.
- **Releases publish only through `release.yml`.** crates.io (both crates), PyPI and npm trust only that workflow file in this repo, through the GitHub environments `crates-io`, `pypi` and `npm`, which accept only `v*` tags. Both crates refuse token uploads.
- **Long checks** (`cargo mutants`, full test runs, CI): run them in the background and wait for the notification.
