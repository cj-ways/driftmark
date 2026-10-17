---
status: done
date: 2026-10-17
---

# Spike: how fast driftmark can read and diff a large docs folder

> Short version: on this server, driftmark can find, read, parse and block-diff 30 days of changes in the private corpus in 0.13 seconds with all 12 cores. That range changes 2,643 Markdown files. On one core it takes 0.8 seconds. A typical 19-commit branch takes 19 ms, and one commit takes 7 ms. gix is faster than the `git` command at every git step and starts no processes. The recommendation is gix, behind a small interface, with the `git` command kept as the test oracle. No cache is needed to open a change page in under 300 ms on this server. A cache of block lists keyed by blob hash costs about 5 MB for the whole corpus, and it protects slower machines. Proposed M1 limit: the 30-day range diffs in under 1 second with all cores, and in under 5 seconds on one core.

## Facts that turned out different from the brief

1. **A cold file cache could not be measured.** The server is an LXC container on ZFS. ZFS keeps file data in its own cache on the host (the ARC, 32 GB), not in the Linux page cache. Telling the kernel to drop a file (`posix_fadvise`) had no effect: a 277 MB pack file read in 0.05 seconds both before and after. Emptying the ARC needs the host. The section "Cold cache: an estimate" replaces the measurement.
2. **The load was lower than at briefing time.** The main runs ran at a load of 5.9 to 7.7 on 12 cores, against about 35 at briefing time. So the numbers are not pessimistic. A first pass at a load of 8.4 to 9.2 gave totals within about 10% of the main pass.
3. **Branches of about 20 commits are rare in the corpus.** One of the last 200 merges has 15 to 25 commits, and 14 of the last 1,500 do. The chosen branch has 19 commits and changes 20 Markdown files. The median among those 14 branches is 12 files.
4. **The 30-day range changes 2,643 Markdown files with rename detection off.** git's default diff pairs renamed files and reports 2,184.
5. **comrak's licence is BSD-2-Clause, and `deny.toml` allows only MIT and Apache-2.0.** M1 must add it, whichever git reader it uses. comrak's own dependencies also need Unicode-3.0 and Unicode-DFS-2016.

## What was measured

**Corpus.** The private corpus at the tip of its main branch: 3,416 Markdown files, 54.5 MB, the largest 479 KB.

**Ranges.**

| Range | How it was chosen | Commits | Changed `.md` files (edited, added, deleted) | Blobs read | MB read | Blocks |
|---|---|---|---|---|---|---|
| One commit | A commit that changes 3 files, the median for commits that touched docs in the last 30 days (1,405 such commits) | 1 | 3 (3, 0, 0) | 6 | 0.2 | 1,381 |
| Branch | A merged branch against its merge base | 19 | 20 (11, 9, 0) | 31 | 2.0 | 5,758 |
| 30 days | First-parent history, 30 days back from the tip | 3,179 | 2,643 (896, 1,093, 654) | 3,539 | 66.7 | 318,261 |
| Every file | All `.md` files at the tip, for search and status pages | - | 3,416 | 3,416 | 54.5 | 279,273 |

**Steps, each through both readers.**

- **List:** `git diff-tree -r -z --no-renames`, or gix's tree diff over the two `docs` trees. For every file at one commit: `git ls-tree -r`, or a gix tree walk.
- **Read both ends:** `git cat-file --batch`, one long-lived process per worker thread. Or gix `find_object`, one handle per worker thread.
- **Parse:** comrak 0.56.0 with tables, strikethrough, autolinks, task lists, footnotes, alerts and front matter. Every node carries its source position.
- **Split:** each top-level node, each list item and each table row is one block. A nested list splits into its own items. Each block's byte range comes from its line and column positions.
- **Hash:** Rust's standard hasher (SipHash) over each block's source bytes.
- **Block diff:** a longest-common-subsequence table over the two hash lists of each file, after trimming the common head and tail, with the edit list walked back.

**How it was run.**

- One thread, and all 12 threads (rayon).
- Each figure is the median of 3 runs, after one untimed warm-up run. git and gix runs alternate, so a change in load hits both.
- Wall and CPU time per run come from the driver. Per-step times come from inside the program. Peak memory comes from `/usr/bin/time -v`, plus the peak of each git child process.
- The load average is recorded before every run.
- Tools: Rust 1.99.1, git 2.43.0, gix 0.89.0, comrak 0.56.0, rayon 1.12.0. Release build with LTO, as in the workspace.

**Checks.**

- Both readers found the same files, the same bytes and the same block diff in all four cases.
- No block had an invalid source range.
- Blocks cover 98.7% of the source bytes. The rest is blank lines between blocks.

## Timings, warm cache

All times are in milliseconds. "List" for gix includes opening the repository (0.8 to 1.0 ms). On 12 threads, parse, split and hash run as one pass, so they share one figure. Peak memory is driftmark's process plus the sum of its git child processes, in MB.

| Range | Reader | Threads | List | Read | Parse | Split | Hash | Block diff | Total wall | CPU | Peak memory |
|---|---|---|---|---|---|---|---|---|---|---|---|
| One commit | git | 1 | 4.7 | 3.5 | 2.0 | 0.1 | 0.1 | 0.2 | 13 | 13 | 8 + 5 |
| One commit | git | 12 | 4.8 | 3.7 | 1.1 (all three) | | | 0.3 | 13 | 33 | 8 + 24 |
| One commit | gix | 1 | 1.4 | 0.7 | 1.9 | 0.1 | 0.1 | 0.2 | 7 | 7 | 4 |
| One commit | gix | 12 | 1.6 | 0.5 | 1.1 (all three) | | | 0.3 | 7 | 14 | 4 |
| Branch | git | 1 | 5.9 | 11.6 | 15.1 | 0.9 | 0.4 | 2.2 | 40 | 42 | 13 + 14 |
| Branch | git | 12 | 6.3 | 8.7 | 7.2 (all three) | | | 2.3 | 28 | 88 | 16 + 57 |
| Branch | gix | 1 | 1.9 | 5.7 | 15.1 | 0.9 | 0.4 | 2.3 | 31 | 31 | 20 |
| Branch | gix | 12 | 2.2 | 2.3 | 7.4 (all three) | | | 2.2 | 19 | 50 | 25 |
| 30 days | git | 1 | 13.5 | 285 | 492 | 33 | 17 | 11.9 | 906 | 966 | 99 + 100 |
| 30 days | git | 12 | 13.0 | 56 | 63 (all three) | | | 4.8 | 149 | 1,094 | 103 + 211 |
| 30 days | gix | 1 | 6.7 | 195 | 483 | 33 | 16 | 14.7 | 809 | 807 | 157 |
| 30 days | gix | 12 | 6.5 | 35 | 61 (all three) | | | 4.0 | 127 | 879 | 181 |
| Every file | git | 1 | 7.7 | 268 | 413 | 28 | 14 | - | 774 | 819 | 61 + 62 |
| Every file | git | 12 | 7.5 | 60 | 50 (all three) | | | - | 134 | 916 | 79 + 167 |
| Every file | gix | 1 | 5.5 | 161 | 410 | 28 | 14 | - | 667 | 664 | 122 |
| Every file | gix | 12 | 5.1 | 31 | 53 (all three) | | | - | 112 | 706 | 140 |

What the numbers say:

- **Parsing is the largest cost on wide ranges.** It is 60% of the one-thread time for 30 days. comrak parses about 135 MB per second on one core.
- **Splitting and hashing cost less than a tenth of parsing.**
- **The block diff is small.** The 30-day range fills 9.9 million table cells in 12 to 15 ms. The largest single table has 0.78 million cells.
- **Reading is the second cost.** gix reads 1.5 times faster than `git cat-file` on one thread (195 against 285 ms) and 1.6 times faster on 12 threads (35 against 56 ms).
- **Listing is 1.4 to 3.4 times faster with gix.** One `git` launch costs 1.0 ms here (`git --version`), and 1.5 ms inside the repository (`git rev-parse`). A merge base for the 30-day range takes 9 ms with the `git` command.
- **12 threads cut the 30-day total by 6 times**, from 0.81 to 0.13 seconds with gix.
- **Memory:** gix's process peaks higher (181 against 103 MB for 30 days), because the pack files it maps count against it. The `git` path adds 12 child processes whose peaks sum to 211 MB.
- **Load matters on 12 threads.** At a load of 13 to 14, the 30-day range with gix took 138 to 176 ms instead of 127 ms. One-thread times did not change.

**Rendering HTML**, measured in a separate run:

| Range | gix, 12 threads | gix, 1 thread | git, 12 threads | HTML produced |
|---|---|---|---|---|
| Branch | 19 ms | 37 ms | 29 ms | 3.4 MB from 2.0 MB |
| 30 days | 139 ms | 1,019 ms | 158 ms | 114 MB from 66.7 MB |
| Every file | 113 ms | 836 ms | 144 ms | 93 MB from 54.5 MB |

Rendering adds about 40% to parsing time on one core.

**Rename detection is expensive in git.** `git diff-tree -M` on the 30-day range takes 396 ms instead of 12 ms, and pairs 478 renamed files. If driftmark pairs renamed files, it should compare the block hashes it already has, not ask git.

## Cold cache: an estimate

These figures are counted, not timed. For each range they count the objects that would come off the disk: the blobs at both ends, plus every delta base below them.

| Range | Objects with delta bases | Bytes on disk | 128 KiB pack records | Loose object files | Longest delta chain |
|---|---|---|---|---|---|
| One commit | 6 | 0.09 MB | 0 | 6 | 0 |
| Branch | 78 | 0.5 MB | 15 (2.0 MB) | 0 | 8 |
| 30 days | 4,649 | 17.8 MB | 232 (30.4 MB) | 204 | 18 |
| Every file | 4,846 | 21.6 MB | 211 (27.7 MB) | 204 | 10 |

The five pack index files add at most 6.9 MB, which is 53 records.

What this means:

- NVMe drives like this server's typically answer a random read in about 0.1 ms. This was not measured here. About 500 such reads one after another would take about 50 ms.
- So on this server, a cold cache adds tens of milliseconds to the 30-day range, not seconds.
- On a spinning disk, at about 10 ms per seek, the same reads would take about 5 seconds.
- The 128 KiB record size is ZFS's default. The container cannot read the dataset's real setting.

## The `git` command or gix

| Point | `git` command | gix |
|---|---|---|
| Speed | 13, 28 and 149 ms for one commit, a branch and 30 days on 12 threads | 7, 19 and 127 ms. Listing is 1.4 to 3.4 times faster, reading 1.5 to 1.6 times |
| Windows | Needs Git for Windows installed. A change page needs 2 to 4 launches (resolve names, merge base, list, and read unless one process stays open). Launches are slower on Windows than on Linux (process creation, the MSYS layer, antivirus scans). Not measured here | No launches. gitoxide runs its tests on Windows in CI (`windows-latest`, `windows-2025`, `windows-11-arm`). Not measured here |
| Licence | git is GPL-2.0, but driftmark only runs it, so driftmark's licence does not change | gix is MIT OR Apache-2.0. Its dependencies need Zlib added to `deny.toml`. Its default pack cache pulls in MPL-2.0 (`uluru`); the byte-capped cache avoids it |
| Binary size | No change | Adds 1.15 to 1.23 MB, and 83 to 102 crates |

**Binary size**, measured on x86_64 Linux. All builds use LTO, one codegen unit and stripping, as in the workspace. Clean build times are at 4 jobs and a load of 6 to 11, so they are rough.

| Build | Binary | Crates | Clean build |
|---|---|---|---|
| `git` command only (comrak, rayon, spike) | 1.24 MiB | 25 | 15 s |
| gix, default pack cache | 2.35 MiB | 110 | 30 s |
| gix, byte-capped pack cache (no MPL-2.0) | 2.33 MiB | 108 | 24 s |
| gix, plus revision parsing and merge base | 2.36 MiB | 114 | 33 s |
| gix, plus revisions and rename tracking | 2.41 MiB | 127 | 37 s |

**The byte-capped pack cache costs no speed.** With 16 MB per handle, reading took 189 ms on one thread and 42 ms on 12, against 194 and 42 ms with the default cache. That run was at a load of 13 to 14.

**What gix cannot do yet**, from gitoxide's own status file:

- **Partial clones.** gix cannot fetch a missing blob from the remote. `git cat-file` can. A CI job that clones with `--filter=blob:none` would fail with gix. This matters for M5's merge-request job.
- **SHA-256 repositories and reftable.** Git 3.0 plans both as defaults for new repositories. It has no release date yet.
- **A stable API.** gix is at version 0.89. Under Rust's version rules, each 0.x release may break the API, so driftmark must pin it.

**Recommendation: gix.** The reasons:

1. It is faster at every git step: 6 to 22 ms less per page on Linux, and 20% less CPU on the 30-day range.
2. It starts no processes. That matters most on Windows and in a server that answers many page requests.
3. Its licences fit after adding Zlib. MPL-2.0 can be avoided at no cost. BSD-3-Clause comes only with rename tracking.
4. 1.15 MB more binary is acceptable.

The conditions:

- Keep git reading behind a small interface: list changes, read a blob, resolve a name, find a merge base, list commits since a date.
- Keep the `git` command version as the test oracle in CI. In this spike the two readers agreed exactly in all four cases.
- Whether the `git` command also ships as a fallback, for partial clones and SHA-256 or reftable repositories, is the maintainer's decision.

The speed gap alone is small in absolute terms. Parsing dominates every wide range, whichever reader is used.

## Proposed time limit for M1's "done" check

> On this server, `driftmark diff` over the 30-day range of the private corpus (about 2,600 changed files) finishes in under 1 second with all cores, and in under 5 seconds on one core (`RAYON_NUM_THREADS=1`). The typical branch finishes in under 100 ms. Each is the median of 3 runs at a load below 12.

The reasons:

- The rough pipeline here takes 0.13 seconds on all cores and 0.81 seconds on one.
- M1 adds pairing by shared words, move detection, ranking and JSON output. The limit leaves 7 times the measured time on all cores, and 6 times on one core, for that work.
- Pairing needs care. In the 30-day range, the unmatched parts of the edited files hold 65,041 removed blocks and 98,820 added ones. Comparing every removed block with every added block in the same file means up to 9.9 million pairs. An index of words per file keeps pairing inside the limit.

## Does the server need a cache keyed by blob hash?

**Not to open a change page in under 300 ms on this server.** With gix on 12 cores, the whole change-page work takes 7 ms for one commit, 19 ms for a branch, and 139 ms for 30 days. That includes listing, reading, parsing, splitting, hashing, block diff, and HTML for every changed file.

**It becomes needed in four situations:**

1. One core: 0.8 to 1.0 seconds for 30 days.
2. A machine with fewer cores than this server.
3. A busy server: 176 ms for 30 days at a load of 14, before rendering.
4. M1's pairing and ranking, which add work these numbers do not include.

The 30-day page also produces 114 MB of HTML. No page should send that at once. The page has to render files when they are opened, cache or not.

**Recommendation:** build the cache in M2, as the plan already says. It should hold each blob's block list. It should not hold parsed trees. With a warm cache the 30-day page needs only listing and block diff: about 10 ms on 12 threads and 21 ms on one.

**What the cache costs in memory:**

| What is kept per blob | Whole corpus at one commit (3,416 blobs) | Blobs of the 30-day range (3,539) |
|---|---|---|
| Block hashes and byte ranges, 16 bytes per block | 4.5 MB (279,273 blocks) | 5.1 MB (318,261 blocks) |
| Plus the source text | 59 MB | 72 MB |
| Plus the rendered HTML | 152 MB | 186 MB |
| Parsed trees instead of block lists | about 440 MB (8 bytes per source byte) | about 540 MB (same ratio) |

The source text does not need caching: gix reads one file in under 1 ms.

## What was not done

- A real cold-cache measurement. The estimate above replaces it.
- Any measurement on Windows or macOS. No machine was available.
- The speed of gix's rename tracking.
- Other hash functions. Only Rust's standard SipHash was timed. Hashing is 2% of the one-thread time, so the choice matters little.

## Decisions for the maintainer

1. gix or the `git` command. The recommendation is gix.
2. Whether the `git` command also ships as a fallback for partial clones, SHA-256 and reftable repositories.
3. Licences to add to `deny.toml`:
   - BSD-2-Clause, Unicode-3.0 and Unicode-DFS-2016, needed by comrak in any case;
   - Zlib, for gix;
   - BSD-3-Clause, only if gix's rename tracking is used.
   MPL-2.0 can be avoided with gix's byte-capped pack cache.
4. The proposed M1 time limit.

## How to run it again

The spike code is in `spikes/speed/`. It is its own Cargo package, outside the workspace.

1. Build: `cargo build --release` in `spikes/speed/`.
2. Write a config file that names the repository, the folder and the revisions, and keep it out of git.
3. Time everything: `python3 run.py CONFIG.json OUT_DIR`. Results go to `OUT_DIR/runs.jsonl`, and medians to `OUT_DIR/summary.md`.
4. Count the cold-cache bytes: `python3 cold_estimate.py CONFIG.json OUT.json`.
5. Measure size without gix: `cargo build --release --no-default-features`.
6. Measure gix without MPL-2.0: `cargo build --release --no-default-features --features gix-dynamic-cache`.

## Sources

- [gitoxide crate status](https://github.com/GitoxideLabs/gitoxide/blob/main/crate-status.md): partial clones, SHA-256 and reftable not yet supported.
- [gitoxide CI workflow](https://github.com/GitoxideLabs/gitoxide/blob/main/.github/workflows/ci.yml): Windows test jobs.
- [Git's planned breaking changes for 3.0](https://code.googlesource.com/git/+/6a64ac7b014fa2cfa7a69af3c253bcd53a94b428/Documentation/BreakingChanges.adoc): no release date set.
- [Diagnosing performance issues in Git for Windows](https://gitforwindows.org/diagnosing-performance-issues): why it runs slower than on Linux.
