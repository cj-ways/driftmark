@AGENTS.md

# Claude Code — extra notes

The shared rules are in `AGENTS.md` (imported above). This part is only for Claude Code.

- **Cargo is not on the default PATH on the server.** Rust was installed without changing shell startup files. Start commands with `export PATH="$HOME/.cargo/bin:$PATH"`.
- **Commit author on the server.** The global git email there is a work address. Commit with `GIT_AUTHOR_EMAIL` and `GIT_COMMITTER_EMAIL` set to the address in `AGENTS.md`, unless `git config user.email` in this repo already shows it.
- **Long checks** (`cargo mutants`, full test runs, CI): run them in the background and wait for the notification.
