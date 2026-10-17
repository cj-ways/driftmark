---
status: accepted
date: 2026-10-17
---

# 0007 Name, licence and home

> Short version: the tool is called driftmark, uses MIT or Apache-2.0 at the user's choice, and lives at github.com/cj-ways/driftmark, private until version 1 works.

## Decision

1. **Name: driftmark.** "Drift" is documents slowly moving away from what the reader approved. "Mark" is the tool marking those changes and the reader's notes. On 2026-10-17 the name was free on npm, crates.io and PyPI. It is not reserved yet, and no trademark search has been done. The GitHub account name `driftmark` belongs to an unrelated user, so the repo sits in the `cj-ways` organisation.
2. **Licence: MIT OR Apache-2.0.** The usual Rust choice; each user picks one. Comparable tools use MIT (Spec Kit, OpenSpec) or Apache-2.0 (Plannotator). cj-ways/acctswap uses the same pair.
3. **Home: github.com/cj-ways/driftmark.** cj-ways already holds the maintainer's other developer tools, and acctswap's release setup (crates.io, PyPI and npm through trusted publishing, plus a Homebrew tap) can be copied.
4. **Visibility.** Private from 2026-10-17. Public when the first usable version works on the maintainer's 3,400-file corpus. Making a repo public exposes its whole history, so nothing goes into git that may not become public. Private research stays in `private/`, which git ignores.

## Open

- Ownership of the code is the maintainer's to confirm before the repo goes public.
- Reserving the name on npm, crates.io and PyPI needs his approval, because it publishes packages.
