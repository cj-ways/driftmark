---
status: accepted
date: 2026-10-18
---

# 0011 Decisions from M2

> Short version: two "unmaintained" font libraries are allowed, notes live in the repo's git folder, the server returns memory to the system with one documented `unsafe` call on Linux, and the Linux release is built against glibc, not musl.

## Decision

1. **Two advisories are allowed.**
   - `rustybuzz` and `ttf-parser` were marked unmaintained in 2026 (RUSTSEC-2026-0192, RUSTSEC-2026-0206).
   - Neither has a known vulnerability. merman needs both to measure diagram text with a real font, and they only ever read the font driftmark ships.
   - `deny.toml` ignores those two ids, with the reason. The ignores come out when merman moves to their successors, harfrust and skrifa.
   - Without real-font measurement, text is cut off in 193 of 194 diagrams of the maintainer's corpus.
2. **The notes file lives in `<git common dir>/driftmark/notes.db`.**
   - Every worktree of a clone, and every agent working in one, sees the same notes, and git never commits them.
   - Deleting the clone deletes the notes; a written review file is the copy a reader can keep.
   - This settles open item 4 of the version 1 plan.
3. **Memory: drop spare git readers, and trim on Linux.**
   - The memory spike (`docs/working/version-1/spikes/memory.md`) found three causes of the server holding 632 MB after heavy use on a 3,400-file corpus:
     - spare git readers kept for every thread, about 130 MB still in use;
     - glibc keeping memory that bulk jobs freed;
     - page requests leaving freed memory behind.
   - **The fix:**
     - drop the spare readers after each bulk job and when the server is idle;
     - on Linux with glibc, call `malloc_trim(0)` after each bulk job and after one idle second.
   - The spike measured about 390 MB with this combination. A helper process for bulk counts would save about 100 MB more for about 550 lines and a process protocol, so it is not built.
4. **One exception to "no `unsafe`".**
   - The `malloc_trim` declaration is one `safe fn` inside an `unsafe extern "C"` block, in one file.
   - It is sound: it matches glibc's prototype, takes no pointers, has no preconditions, and is thread-safe. A `cfg` limits it to Linux with glibc.
   - The workspace lint moves from `forbid` to `deny`, with a single `#[allow]` on that block. No other `unsafe` is allowed.
5. **The Linux release is built against glibc, not musl.**
   - cj-ways/acctswap ships a static musl build, but for driftmark musl's allocator made counting 15 to 17 times slower (4.2 to 4.4 s instead of 0.25 to 0.29 s), and musl has no `malloc_trim`.
   - M6 builds Linux against an old glibc, so the program runs on most distributions.

## What this costs

- One `unsafe` block to keep sound across Rust and glibc releases.
- Linux users on musl-only systems (Alpine) need to build from source or use a container with glibc.
- The process stays at about 390 MB after the heaviest use measured, well above the cache's 128 MB cap: the cap counts the cache, not the program.
