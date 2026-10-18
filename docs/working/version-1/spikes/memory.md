---
status: done
date: 2026-10-18
---

# Spike: bringing `driftmark serve` near its memory cap

> Short version: neither fix alone brings the server near its 128 MB cache cap. After the heavy-use run on the private corpus, today's main holds 632 MB, and 690 MB after one search. The helper process (fix A) ends at 417 and 569 MB. `malloc_trim(0)` after each bulk job (fix B) ends at 602 and 560 MB. Both together end at 410 and 420 MB. The brief's cause, memory that glibc keeps after the counting threads free it, is one of three. The second is live memory: the git reader keeps a spare gix handle for every thread that ever read at once, and after heavy use 12 of them hold 126 to 137 MB. The third is memory freed by ordinary page requests, which a trim after bulk jobs never reaches. Two small additions fix most of the rest: drop the spare handles, and trim once the server has been idle for a second. With both, B ends at 393 and 394 MB, and A with B at 290 and 326 MB. The live heap alone is then 185 to 188 MB, so about 250 MB of resident memory is the floor with today's caches. Recommendation: drop the spare handles (safe code, every system), and on Linux with glibc trim after bulk jobs and when idle, which needs one `unsafe extern` declaration. Keep the helper process for later: it saves about 100 MB more, but adds about 550 lines and a process protocol. Separately, a static musl build, which acctswap ships for Linux, made the 30-day and branch counts 15 to 17 times slower.

## Facts that turned out different from the brief

1. **The documents phase now includes the branch count.** Since the pages lane, every reading page lists the open branches that change its document. So the first reading page does the home page's work: it counts every branch. The run records memory after the first reading page for this reason. Opening the home page afterwards adds nothing.
2. **Today's main holds more than the server lane measured.** After every changed document it holds 625 to 650 MB, against 375 to 455 MB in `docs/features/commands/serve.md`. After one search it holds 677 to 731 MB. Since that measurement, reading pages also count the branches and draw diagrams on the server, and search was added.
3. **glibc's retention is one cause of three.** glibc's own counters (`mallinfo2`) split the heap into memory in use and memory free. After every document, the server without fixes has 334 to 345 MB of heap in use, while its cache counts 128 MB. 126 to 137 MB of the difference is the git reader's 12 spare gix handles, which are live, so no trim can return them. `serve.md` says a smaller gix pack cache "changed nothing measurable"; the in-use counter shows the handles do hold that memory.
4. **Fix B as briefed misses most of the memory.** A trim after each bulk job returns 35 MB after the 30-day count, 49 MB after the branch count and 154 MB after a search index build. The documents phase then leaves 252 MB free in the heap, and no bulk job follows it.
5. **acctswap ships Linux as a static musl build.** The plan copies acctswap's release setup (M6). musl has no `malloc_trim`, so fix B would do nothing in that build. musl's own allocator gives memory back by itself, but it made the 30-day and branch counts 15 to 17 times slower here.
6. **The repository moved during the runs.** Another session fetched new branches. The 30-day list stayed the same in every run: 2,945 changed documents and 5,070 page requests. The home page's branches changed between runs, so each correctness check compared variants run at the same time.

## What was measured

**Corpus and server.** The private corpus, served on `127.0.0.1` with a free port, with `--base` set to the corpus's working branch. It has about 409 branch refs. The trial servers on ports 8791 and 8792 were not touched.

**Variants.** One spike build carries every fix behind environment switches, so the variants share one binary. "main" is today's `main` (258d174), built separately and unchanged.

| Variant | What it does |
|---|---|
| main | Today's `main`. |
| A | The 30-day count, the branch count and the working-tree count run in a child process, `driftmark __count`. |
| B | `malloc_trim(0)` after each bulk job: a range count, the branch count, the working-tree count and a search index build. |
| AB | A and B. |
| BI | B, plus a trim once no request came for one second. |
| BDI | BI, plus dropping the git reader's spare handles after each bulk job. |
| ABDI | A and BDI. |
| musl | The spike build for `x86_64-unknown-linux-musl`, static, no switch on. |
| muslA | The musl build with A. |
| off | Second set: the spike build with every switch off, to check it against main. |
| D | Second set: spare handles dropped after each bulk job, no trim. |
| AD | Second set: A and D, the best that needs no `unsafe`. |
| BDI2 | Second set: BDI, and the idle trim drops the spare handles first. |
| ABDI2 | Second set: A and BDI2. |

**Steps of one run**, on a fresh server, after the server lane's memory run:

1. Wait for the startup count of the working tree.
2. Open the 30-day change list page once cold, then 7 times warm; fetch its JSON.
3. For each of its 2,945 documents, open its changes page, and its reading page unless it was removed: 5,070 requests.
4. Open the home page once cold, then 7 times warm.
5. Run one search at the documents' revision, a two-word query, once cold, then 7 times warm.

Before each memory reading the server rests 2 seconds, the same for every variant, so a server that trims when idle shows it. The home page's cold time comes from a second, short run per variant: a fresh server, the home page cold, then 7 warm.

**Rounds.** Two sets of three rounds. Each round runs every variant of its set once, in turn, so a change in load hits all of them. Tables show the median of 3 runs. The second set ran 40 minutes after the first; its "off" variant matched main within 2 to 8 MB in every phase, so the sets compare. The load average (1 minute, 12 cores) was 11 to 51 during the runs; other sessions ran Go test suites.

**Correctness.** Every variant answered the 30-day list JSON, the home page JSON and the `auto` range JSON byte for byte like main, run at the same time. No request in any run failed.

## Memory

Resident memory (RSS) in MB, median of 3 runs.

| Variant | After start | 30-day list | First reading page | Every document | First search | Highest (peak) |
|---|---|---|---|---|---|---|
| main | 33 | 260 | 390 | 632 | 690 | 690 |
| A | 19 | 88 | 103 | 417 | 569 | 569 |
| B | 33 | 232 | 350 | 602 | 560 | 698 |
| AB | 17 | 87 | 101 | 410 | 420 | 564 |
| BI | 33 | 226 | 351 | 506 | 536 | 636 |
| BDI | 32 | 175 | 265 | 421 | 410 | 558 |
| ABDI | 17 | 86 | 93 | 301 | 328 | 468 |
| musl | 31 | 168 | 264 | 430 | 467 | 523 |
| muslA | 16 | 83 | 96 | 315 | 368 | 423 |
| off | 33 | 269 | 398 | 630 | 682 | 682 |
| D | 33 | 271 | 354 | 557 | 600 | 600 |
| AD | 19 | 90 | 101 | 442 | 588 | 588 |
| BDI2 | 32 | 177 | 262 | 393 | 394 | 531 |
| ABDI2 | 17 | 79 | 91 | 290 | 326 | 455 |

- The spread between runs was small: main 625 to 650 MB after every document, A 385 to 431, B 593 to 607, BDI2 393 to 397, ABDI2 286 to 294.
- The cache reached its 128 MB in every variant after every document.
- A's numbers leave out the child: it lives 0.3 to 0.6 seconds per count and peaks at 203 to 212 MB for the 30-day count and 240 to 242 MB for the branch count.

**The home page alone**, on a fresh server, RSS after it: main 266 MB, A 32, B 223, AB 28, BDI 220, BDI2 206, ABDI 28, musl 165, muslA 25.

## Speed

Milliseconds. "Cold" is the first request on a fresh server; "warm" is the median of the 7 after it. Median of 3 runs.

| Variant | 30-day list cold | warm | Home page cold | warm | First search cold | warm |
|---|---|---|---|---|---|---|
| main | 536 | 2.1 | 620 | 5.8 | 601 | 2.4 |
| A | 690 | 2.4 | 489 | 4.0 | 445 | 2.1 |
| B | 544 | 2.4 | 454 | 4.0 | 738 | 1.9 |
| AB | 774 | 2.6 | 520 | 3.9 | 482 | 2.4 |
| BI | 719 | 2.2 | 402 | 4.3 | 861 | 7.1 |
| BDI | 649 | 2.5 | 363 | 4.3 | 638 | 4.1 |
| ABDI | 886 | 2.4 | 689 | 3.7 | 642 | 2.5 |
| musl | 5,050 | 5.9 | 4,569 | 5.2 | 4,372 | 4.8 |
| muslA | 5,015 | 5.4 | 4,237 | 6.0 | 4,468 | 4.4 |
| off | 1,190 | 2.2 | 461 | 3.9 | 576 | 3.0 |
| D | 642 | 3.0 | 716 | 3.8 | 1,294 | 2.9 |
| AD | 1,320 | 2.4 | 546 | 4.2 | 904 | 3.8 |
| BDI2 | 831 | 2.4 | 651 | 4.2 | 676 | 3.5 |
| ABDI2 | 762 | 2.6 | 859 | 4.5 | 671 | 3.8 |

At loads of 11 to 51, the cold times of one variant varied by up to a factor of 5 between rounds, and "off", which does what main does, took 1,190 ms against main's 536. So cold differences under about 500 ms are noise here. Two results stand out of the noise:

- **Warm pages do not change** with any fix: 2 to 7 ms.
- **musl is 7 to 9 times slower cold.** Timed inside the server, the 30-day count took 4.4 seconds on musl against 254 ms on glibc, the branch count 4.2 seconds against 286 ms, and an index build 4.3 seconds against 680 ms. The working-tree count, which mostly reads files, took 527 ms against 377 ms.

The bulk jobs, cold, timed inside the server (median of 3 runs):

| Job | In the server (B) | In the helper (A) | In the helper (AB) |
|---|---|---|---|
| 30-day count | 254 ms | 394 ms | 336 ms |
| Branch count | 286 ms | 317 ms | 296 ms |
| Working-tree count | 377 ms | 390 ms | 391 ms |

The helper adds about 10 to 150 ms to a count. The child opens the repository from nothing, and the answer travels as JSON: 865 KB for the 30-day count, 234 KB for the branches, 394 KB for the working tree.

## Where the memory goes

glibc's counters, summed over every pool (`mallinfo2`), median of 3 runs, in MB. "Anonymous" is the process's memory not backed by a file. The rest of RSS is mapped files: the program and git's pack files, which the system can drop and read again.

| Variant | Phase | RSS | Anonymous | Mapped files | Cache's own count | Heap in use | Heap free | Spare handles |
|---|---|---|---|---|---|---|---|---|
| B | 30-day list | 232 | 177 | 55 | 49 | 119 | 98 | - |
| BDI | 30-day list | 175 | 120 | 55 | 50 | 60 | 158 | - |
| A | 30-day list | 88 | 62 | 27 | 50 | 58 | 7 | - |
| off | Every document | 630 | 559 | 71 | 128 | 334 | 249 | 12 |
| D | Every document | 557 | 486 | 71 | 128 | 208 | 308 | 1 |
| B | Every document | 602 | 530 | 71 | 128 | 338 | 252 | - |
| BDI2 | Every document | 393 | 322 | 71 | 128 | 188 | 345 | 0 |
| A | Every document | 417 | 353 | 64 | 128 | 205 | 184 | - |
| ABDI2 | Every document | 290 | 227 | 64 | 128 | 185 | 186 | 0 |

What the numbers say:

1. **The counts' working memory.** A 30-day count peaks at about 205 MB in the helper's child. In the server, glibc keeps that memory in the pools of the 12 counting threads. A trim returns only part of it: 35 MB after the 30-day count, 80 to 86 MB when the spare handles are dropped first. The rest stays resident although it is free: after every document, BDI2's anonymous memory is about 134 MB above its heap in use, ABDI2's about 42 MB. Long-lived memory, such as cache entries, is placed between the freed pieces, so whole pages rarely come free. Only A keeps this memory out of the server.
2. **The spare gix handles.** The git reader lends each thread a gix handle and keeps it afterwards for reuse, with up to 16 MB of pack cache and 4 MB of object cache. After the 30-day count, 8 spare handles held 66 to 69 MB. After the branch count, 12 held 100 to 103 MB. After every document, 12 held about 126 to 137 MB (off against D). This memory is live, so no allocator setting returns it. Dropping the spare handles is safe Rust and works on every system. Without a trim, glibc keeps what they freed, so D alone saved only 73 to 82 MB of RSS.
3. **Free memory from page requests.** The 5,070 page requests parse, render and draw, then the cache drops older entries. This leaves 184 to 345 MB of free heap. A trim after bulk jobs never sees it. A trim after one idle second returns most of it: BI ends 96 MB below B after every document.
4. **The floor.** After every document with no spare handles (BDI2, ABDI2), the heap in use is 185 to 188 MB. The cache counts 128 MB of it, and the diagram cache, which has its own 32 MB cap outside the 128 MB, counts 17 MB. The other 40 MB was not broken down; it includes the allocator's overhead per entry, which the cache's estimate leaves out. With 64 to 71 MB of mapped files, about 250 MB of RSS is the floor for this run with today's caches. Getting nearer 128 MB needs the cache's count to cover what it really holds; that is outside this spike.

## Fix A: the helper process

**How it works in the spike.** The server starts the same program as `driftmark __count <repo> [--base <branch>]`. It writes one job as JSON on the child's standard input: a range between two commits, the working tree, or the branches with the default branch's tip. The child counts on all cores, writes the answer as JSON, writes its peak memory on standard error, and exits. The server reads the answer into the same values the in-process count makes, and caches them the same way. `driftmark diff --json=summary` was not used as it is: it leaves out each document's status change, which the counts keep, and the home page needs a mode for many branches. One internal command with three jobs was simpler.

**Code size.**

- `serve/helper.rs`: 449 lines, of which about 390 are code. About 220 of them turn the counts into JSON and back, because the core's types are only `Serialize`. Deriving `Deserialize` in the core would remove most of those, at the cost of the core's 100% coverage rule for the new code.
- `serve/repo.rs`: about 70 lines to route the three counts through the helper and fall back. The branch count was split out of the home page code so the child can run it; that split changes no behaviour.
- `main.rs` and `serve/mod.rs`: 23 lines.
- Total: about 550 lines, with no tests yet. A real version needs tests of each job, of the fallback, and of the JSON both ways.

**Process-start cost per job.** Starting the program, opening the repository and answering an empty job took 2.8 to 3.5 ms (median of 31 starts, measured three times, at a load of 42). `driftmark --version` alone took 1.5 ms. Inside a real count the child also opens the repository from nothing, and the answer travels as JSON. Together that added about 10 to 150 ms to a 250 to 400 ms count (table above).

**Error handling** in the spike, and what a real version needs:

| Case | The spike | A real version also needs |
|---|---|---|
| The program cannot start the child | Counts in the server and prints one line | The same |
| The child fails: git error, crash, killed for memory | The page answers 500 with the child's last line; nothing is cached, so the next request tries again | The same |
| The answer cannot be read | 500, "a bug in driftmark" | The same |
| The child hangs | Waits forever | A time limit, then kill |
| The program file was replaced by an upgrade | The new version answers; its JSON may differ | A version check in the job, or starting `/proc/self/exe` on Linux |
| The child writes a warning, such as gix falling back to the `git` command | Lost | Pass the child's other lines on to the server's standard error |
| Two counts at once | Two children, each on all cores | A limit of one child at a time, or accept it |

**On Windows.** Rust's `Command` starts the child with `CreateProcessW`, and `current_exe()` finds the program; the spike's code has nothing Unix-only. Not measured here: starting a process on Windows is much slower than on Linux, over 20 times in one benchmark ([bitsnbites](https://www.bitsnbites.eu/benchmarking-os-primitives/)), and antivirus software scans each start. A few tens of milliseconds per count would still be small next to the count. The working-tree count runs again on every file change, so it would pay that each time. A child of a console program shares the console, and Ctrl-C reaches both. If the server ever runs without a console, the child needs the `CREATE_NO_WINDOW` flag, or a window flashes. Windows locks a running program file, so the upgrade case above cannot happen there.

**Search indexes.** A revision's index stays in the cache, so a child would have to send the whole index back: about 26 MB of estimated memory, through a format the index does not have yet. The build's temporary memory is what grows the process. A trim right after the build returned 154 MB (B), 122 to 132 MB with the handles dropped (BDI, BDI2). So a trim fits the index build, and a helper does not.

## Fix B: `malloc_trim(0)`

**The code**, as a real version would carry it (the spike's `src/memory.rs` adds diagnostics around it):

```rust
#[cfg(all(target_os = "linux", target_env = "gnu"))]
#[allow(unsafe_code)]
mod glibc {
    // SAFETY: glibc's own prototype from <malloc.h>, `int malloc_trim(size_t pad)`.
    // `usize` is `size_t` and `c_int` is `int` on every target Rust builds for glibc.
    // The function takes no pointer and has no precondition; the manual lists it as
    // MT-Safe, and it locks each arena while it trims it.
    unsafe extern "C" {
        pub(super) safe fn malloc_trim(pad: usize) -> std::ffi::c_int;
    }
}

/// Gives memory that bulk work freed back to the system. Does nothing without glibc.
pub fn give_back() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    glibc::malloc_trim(0);
}
```

Declaring it `safe fn` means the call sites need no `unsafe` block; the declaration carries the one promise. The workspace's lint is `unsafe_code = "forbid"`, and `forbid` refuses even this declaration under an `#[allow]`. The spike lowers it to `deny` in the `driftmark` crate only.

**Why it is sound.**

1. The declaration matches glibc's prototype exactly, so the call passes and returns the right types.
2. The function takes a number and no pointers, so it cannot read or write memory it was not meant to.
3. It has no precondition and is thread-safe: glibc locks each arena while it trims it, so other threads may allocate meanwhile.
4. The `cfg` limits it to Linux with glibc, where the symbol always exists. Rust needs glibc 2.17 or later, and since glibc 2.8 the function trims every arena ([malloc_trim(3)](https://man7.org/linux/man-pages/man3/malloc_trim.3.html)).
5. Rust's allocator on that target is glibc's malloc. If driftmark ever set another `#[global_allocator]`, the call would stay safe and simply return little.

**What it costs.** A trim took 6 to 12 ms after the 30-day count, 9 to 11 ms after the branch count, and 16 to 21 ms after an index build. An idle trim took a median of 1.3 to 6 ms. Of 102 idle trims, the slowest took 359 ms, at a load of 32; the next slowest took 35 ms. A trim holds each pool's lock while it works on it, so a request that allocates in that pool waits. The idle trim runs only when no request came for a second.

**Wrapper crates.** No crate on crates.io wraps `malloc_trim` safely by itself; the names `malloc_trim`, `malloc-trim` and `trim-malloc` do not exist. The `libc` crate (MIT OR Apache-2.0, already in `Cargo.lock` through tokio and getrandom) declares `malloc_trim` and `mallinfo2`, but as `unsafe` functions, so using it needs an `unsafe` block in driftmark anyway; it saves only the 3-line declaration. `tikv-jemalloc-ctl` offers a safe purge call, but only for jemalloc, which `serve.md` measured at 366 to 478 MB, and which builds C code on every target.

## macOS, Windows and static Linux builds

None of these could be run here. The mechanism measured here, glibc's pool per thread, exists on none of them. Whether their allocators keep a similar amount of freed memory resident cannot be told from their documentation.

- **macOS.** The system allocator is Apple's libmalloc, not glibc, so fix B's code does nothing there. Whether the counts' freed memory stays resident there is not known without a measurement. libmalloc has its own call to give memory back, `malloc_zone_pressure_relief(NULL, 0)`, which makes "its best effort towards releasing (i.e. munmap()-ing)" memory from all zones ([malloc.h](https://github.com/apple-oss-distributions/libmalloc/blob/main/include/malloc/malloc.h)). It would need the same kind of `unsafe` declaration.
- **Windows.** Rust's allocator there is `HeapAlloc` on the process's one default heap ([std's windows.rs](https://github.com/rust-lang/rust/blob/master/library/std/src/sys/alloc/windows.rs)), so glibc's per-thread pools do not exist. Microsoft says the heap "cannot guarantee" that freed memory is decommitted, and that `HeapCompact` is only a hint ([KB 3073576](https://support.microsoft.com/en-us/help/3073576)). So some retention is possible there too, of an unknown size.
- **Static musl builds** (acctswap's Linux release). musl's allocator returns freed memory to the system by itself ([musl list, 2022](https://openwall.com/lists/musl/2022/09/19/12)), which the run shows: 430 and 467 MB without any fix, 315 and 368 MB with A. But it keeps one global lock, and the 30-day and branch counts took 4.2 to 4.4 seconds instead of 254 to 286 ms.
- **Everywhere.** The spare gix handles are live memory on every system, so dropping them helps everywhere. Fix A works everywhere, at the start costs above.

## Recommendation

1. **Drop the git reader's spare handles after each bulk job and when the server goes idle.** It is about 15 lines of safe code and helps on every system. After heavy use it takes 126 to 137 MB of live heap off the server. On glibc it pays off fully only with item 2: alone it saved 73 to 82 MB of RSS.
2. **On Linux with glibc, trim after each bulk job and after one idle second.** It is about 40 lines, with the idle thread, and one `unsafe extern` declaration, which needs the maintainer's exception to "no `unsafe`" and the lint at `deny` instead of `forbid`. With item 1 (BDI2) the server ends at 393 MB after every document and 394 MB after a search, against 632 and 690 MB today, with no measurable cost to warm pages.
3. **Do not build the helper process now.** With items 1 and 2 it reaches 290 and 326 MB (ABDI2), about 100 MB lower, and its peak is 455 MB instead of 531. It costs about 550 lines plus tests, a JSON protocol that must survive upgrades, a time limit, up to 150 ms per cold count, and a 200 to 240 MB child at count time. Revisit it if the server must stay near 300 MB, or if "no `unsafe`" stays: without item 2 it is the only fix here that returns the counts' memory (AD: 442 and 588 MB). It is also the only fix that works the same on every system.
4. **Decide the Linux release build before M6.** A static musl build with musl's allocator made the 30-day and branch counts 15 to 17 times slower. The choices are a glibc build against an old glibc, which keeps item 2 working, or musl with another allocator, measured first.
5. **Count the cache's real memory.** The 128 MB cap leaves out the diagram cache and the allocator's overhead per entry. If "near the cap" means resident memory, the cap's count must cover what the cache really holds.

## Not finished

- Nothing was measured on macOS or Windows.
- The helper's start cost on Windows is an estimate from a published benchmark, not a measurement.
- The fixes were not tried with a search of the working tree, whose index lives outside the cache.
- `MALLOC_ARENA_MAX=2`, which `serve.md` measured on an older main, was not measured again.
- The spike code has no tests; it stays on the `spike/memory` branch.
